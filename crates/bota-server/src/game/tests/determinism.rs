//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, EventVisibility, Fnv, ITEMS, MatchConfig, World, map_of, rules};

/// Ticks between two pinned checkpoints.
const CHECKPOINT_TICKS: u32 = 1_500;
/// Ticks a match on a map without a cap is played for.
const UNCAPPED_TICKS: u32 = 18_000;
/// Ticks a debug build plays: the pinned prefix it can afford.
const DEBUG_TICKS: u32 = CHECKPOINT_TICKS;
/// Ticks between two decisions of a seat.
const DECISION_TICKS: u32 = 3;
/// Bag slots an order may name, stash included.
const ANY_SLOT: u64 = (crate::game::BAG_SLOTS + rules::STASH_SLOTS) as u64;

/// One pinned match: its map, seed and heroes, and what it must replay to.
struct Pinned {
    /// The map it is played on.
    map: MapId,
    /// The seed of the match and of its order streams.
    seed: u64,
    /// The hero of each seat, slot order, Radiant on even slots.
    heroes: &'static [u16],
    /// `(tick, world hash, stream digest)` at every checkpoint.
    checkpoints: &'static [(u32, u64, u64)],
    /// The digest of the final tick, winner and match statistics.
    last: u64,
}

/// One seat's order stream.
struct Driver {
    /// The xorshift state the stream draws from.
    state: u64,
}

impl Driver {
    /// The next draw of the stream.
    fn draw(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// A draw below a bound, zero for an empty range.
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.draw() % bound }
    }

    /// A spot within a square about a point.
    fn near(&mut self, at: Vec2, half: i32) -> Vec2 {
        let span = u64::from(half.unsigned_abs()) * 2 + 1;
        let dx = self.below(span) as i32 - half;
        let dy = self.below(span) as i32 - half;
        Vec2::from_ints(at.x.to_int() + dx, at.y.to_int() + dy)
    }

    /// A unit of the view picked at random, if it shows any.
    fn unit(&mut self, view: &WorldView) -> Option<Target> {
        let at = self.below(view.units.len() as u64) as usize;
        view.units.get(at).map(|unit| Target::Unit(unit.id))
    }

    /// What an aim is pointed at this time.
    fn aimed(&mut self, aim: Aim, me: &UnitView, view: &WorldView) -> Target {
        match aim {
            Aim::Own => Target::None,
            Aim::Unit => self.unit(view).unwrap_or(Target::None),
            Aim::Point | Aim::Tree | Aim::Building => Target::Pos(self.near(me.pos, 900)),
        }
    }

    /// The seat's next order for the unit it drives, from what it sees.
    fn order(&mut self, me: &UnitView, view: &WorldView, enemy_base: Vec2) -> Order {
        match self.below(16) {
            0..=3 => Order::Attack {
                target: Target::Pos(self.near(enemy_base, 600)),
            },
            4..=6 => Order::Attack {
                target: self.unit(view).unwrap_or(Target::None),
            },
            7 => Order::Move {
                target: Target::Pos(self.near(me.pos, 1_200)),
            },
            8 | 9 => {
                let slot = self.below(me.abilities.len() as u64) as usize;
                let aim = me.abilities.get(slot).map_or(Aim::Own, |held| held.aim);
                Order::Cast {
                    slot: AbilitySlot(slot as u8),
                    target: self.aimed(aim, me, view),
                }
            }
            10 if self.below(2) == 0 => Order::Learn {
                slot: AbilitySlot(self.below(me.abilities.len() as u64) as u8),
            },
            10 => Order::LearnTalent {
                talent: TalentSlot(self.below(u64::from(TalentSlot::COUNT)) as u8),
            },
            11 => Order::Buy {
                item: ItemId(self.below(ITEMS.len() as u64) as u16),
            },
            12 => {
                let slot = self.below(me.items.len() as u64) as usize;
                let aim = me
                    .items
                    .get(slot)
                    .copied()
                    .flatten()
                    .and_then(|item| item.aim)
                    .unwrap_or(Aim::Own);
                Order::Use {
                    slot: ItemSlot(slot as u8),
                    target: self.aimed(aim, me, view),
                }
            }
            13 => match view.loot.get(self.below(view.loot.len() as u64) as usize) {
                Some(loot) => Order::Take {
                    target: Target::Unit(loot.id),
                },
                None => Order::Sell {
                    slot: ItemSlot(self.below(ANY_SLOT) as u8),
                },
            },
            14 => Order::Swap {
                from: ItemSlot(self.below(ANY_SLOT) as u8),
                to: ItemSlot(self.below(ANY_SLOT) as u8),
            },
            _ => Order::Put {
                slot: ItemSlot(self.below(ANY_SLOT) as u8),
                target: Target::None,
            },
        }
    }
}

/// The configuration of a pinned match.
fn config(pinned: &Pinned) -> MatchConfig {
    let mut master_key = [0; 32];
    master_key[..8].copy_from_slice(&pinned.seed.to_le_bytes());
    MatchConfig {
        match_id: pinned.seed,
        master_key,
        picks: pinned
            .heroes
            .iter()
            .enumerate()
            .map(|(slot, &hero)| Pick {
                slot: SlotId(slot as u8),
                team: if slot % 2 == 0 {
                    Team::Radiant
                } else {
                    Team::Dire
                },
                hero: HeroId(hero),
            })
            .collect(),
        map: pinned.map,
        tick_rate: rules::TICKS_PER_SECOND as u16,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// Folds one encoded message into a digest.
fn fold(digest: &mut Fnv, msg: &ServerMsg) {
    for byte in encode_frame_to_vec(msg).expect("every message fits a frame") {
        digest.u8(byte);
    }
}

/// The orders every seat gives this tick, validated as a trainer does, with
/// each refusal folded into the digest.
fn decide(world: &World, drivers: &mut [Driver], digest: &mut Fnv) -> Vec<Command> {
    let mut commands = Vec::new();
    for (seat, driver) in world.seats.iter().zip(drivers.iter_mut()) {
        let view = world.view(seat.team);
        let owned: Vec<&UnitView> = view
            .units
            .iter()
            .filter(|unit| unit.owner == Some(seat.slot) && unit.kind != UnitKind::Hero)
            .collect();
        let hero = seat.unit.map(crate::game::wire_id);
        let named = if driver.below(8) == 0 && !owned.is_empty() {
            Some(owned[driver.below(owned.len() as u64) as usize].id)
        } else {
            hero
        };
        let Some(me) = view.units.iter().find(|unit| Some(unit.id) == named) else {
            continue;
        };
        let enemy = usize::from(seat.team == Team::Radiant);
        let order = driver.order(me, &view, world.map.fountains[enemy]);
        let unit = named.filter(|id| Some(*id) != hero);
        match world.validate_order(seat.slot, unit, &order) {
            Ok(()) => commands.push(Command {
                slot: seat.slot,
                unit,
                order,
            }),
            Err(reason) => fold(
                digest,
                &ServerMsg::OrderRejected {
                    seq: world.tick,
                    reason,
                },
            ),
        }
    }
    commands
}

/// Plays a pinned match to its end or horizon, and answers every
/// checkpoint and, for a whole match, the digest of its end.
fn replay(pinned: &Pinned, horizon: u32) -> (Vec<(u32, u64, u64)>, Option<u64>) {
    let cfg = config(pinned);
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut drivers: Vec<Driver> = (0..cfg.picks.len() as u64)
        .map(|slot| Driver {
            state: ((pinned.seed << 8) | (slot + 1)).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
        })
        .collect();
    let mut digest = Fnv::new();
    let mut checkpoints = Vec::new();
    while world.tick < horizon && world.victor().is_none() {
        let commands = if world.tick.is_multiple_of(DECISION_TICKS) {
            decide(&world, &mut drivers, &mut digest)
        } else {
            Vec::new()
        };
        let events = world.advance(&commands);
        for team in [Team::Radiant, Team::Dire] {
            fold(
                &mut digest,
                &ServerMsg::Snapshot {
                    view: world.view(team),
                },
            );
            fold(
                &mut digest,
                &ServerMsg::Events {
                    tick: world.tick,
                    events: events
                        .iter()
                        .filter(|event| match event.visible_to {
                            EventVisibility::Everyone => true,
                            EventVisibility::OneTeam(side) => side == team,
                        })
                        .map(|event| event.kind.clone())
                        .collect(),
                },
            );
        }
        if world.tick.is_multiple_of(CHECKPOINT_TICKS) {
            checkpoints.push((world.tick, world.hash(), digest.done()));
        }
    }
    let whole = world.victor().is_some() || horizon >= match_ticks(pinned.map);
    let last = whole.then(|| {
        let mut end = Fnv::new();
        end.u32(world.tick);
        end.u64(digest.done());
        end.u64(world.hash());
        fold(
            &mut end,
            &ServerMsg::MatchOver {
                winner: world.victor().unwrap_or(Team::Neutral),
                stats: world.match_stats(),
            },
        );
        end.done()
    });
    (checkpoints, last)
}

/// Ticks a whole match on a map runs for at most.
fn match_ticks(map: MapId) -> u32 {
    if map == crate::game::MAP2_ID {
        crate::game::MAP2_TICK_CAP
    } else {
        UNCAPPED_TICKS
    }
}

/// Replays the pinned matches on a map and checks them against their pins:
/// the whole match in release, the prefix it affords in debug.
fn check(map: MapId) {
    for pinned in PINS.iter().filter(|pinned| pinned.map == map) {
        assert!(map_of(pinned.map).id == pinned.map, "the map is registered");
        let whole = match_ticks(pinned.map);
        let horizon = if cfg!(debug_assertions) {
            DEBUG_TICKS
        } else {
            whole
        };
        let (checkpoints, last) = replay(pinned, horizon);
        let expected: Vec<_> = pinned
            .checkpoints
            .iter()
            .copied()
            .filter(|&(tick, _, _)| tick <= horizon)
            .collect();
        assert_eq!(
            checkpoints, expected,
            "map {} seed {}: checkpoints diverged",
            pinned.map.0, pinned.seed
        );
        if let Some(last) = last {
            assert_eq!(
                format!("{last:#018x}"),
                format!("{:#018x}", pinned.last),
                "map {} seed {}: the end diverged",
                pinned.map.0,
                pinned.seed
            );
        }
    }
}

#[test]
fn map0_matches_replay_their_pinned_fingerprints() {
    check(MapId(0));
}

#[test]
fn map1_matches_replay_their_pinned_fingerprints() {
    check(MapId(1));
}

#[test]
fn map2_matches_replay_their_pinned_fingerprints() {
    check(MapId(2));
}

/// Prints the fingerprints of every pinned match, to record pins afresh.
#[test]
#[ignore = "records pins"]
fn print_pins() {
    for pinned in &PINS {
        let (checkpoints, last) = replay(pinned, match_ticks(pinned.map));
        println!(
            "map {} seed {} last {:#018x}",
            pinned.map.0,
            pinned.seed,
            last.unwrap_or(0)
        );
        for (tick, world, stream) in checkpoints {
            println!("({tick}, {world:#018x}, {stream:#018x}),");
        }
    }
}

/// Every pinned match, recorded from the simulation as it stood when the
/// contract was laid.
const PINS: [Pinned; 7] = [
    Pinned {
        map: MapId(0),
        seed: 1,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xb298_2894_e137_143b, 0x4108_1da4_350e_7026),
            (3000, 0x2299_f9b9_b7f1_ed59, 0x3546_7a4d_5075_9cdd),
            (4500, 0x15ca_87e9_83e1_a1bb, 0x9683_e155_8329_b2c0),
            (6000, 0x9421_2b1b_efa2_9dc7, 0x1dc7_068d_8366_e2a2),
            (7500, 0x3064_9713_67af_b248, 0x30a2_c40c_d830_707a),
            (9000, 0xef92_48c6_80ed_8b70, 0xbdf7_634c_c284_dde3),
            (10500, 0x9aa2_1f9a_278d_da17, 0x7e65_6ce8_500e_b076),
            (12000, 0xa95e_8362_4ceb_665c, 0xce5d_d820_f6ec_7016),
            (13500, 0x65b8_cfdb_7a88_1e1f, 0x6cad_acbe_cbc4_1e20),
            (15000, 0x0434_1b33_4858_9ef0, 0x3794_34ce_b08f_748f),
            (16500, 0x00e7_bdbc_417e_f09c, 0xc6af_7e0a_617c_0329),
            (18000, 0xdc1a_a52b_f513_f8be, 0x5b3a_061b_b437_2f82),
        ],
        last: 0x5ec7_3cc8_308f_f936,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xf1ac_3c0e_5fb7_c028, 0x621a_24ca_e4d0_faad),
            (3000, 0xd34c_70d7_f2be_2394, 0x6abf_e363_621b_bbdf),
            (4500, 0xc7c3_792a_ee1d_f687, 0xe816_a0ca_ee36_09bc),
            (6000, 0x5046_5395_5708_dc30, 0x6356_ab82_6814_1259),
            (7500, 0x8dec_f00b_64e8_c41e, 0xf862_2f50_b216_a891),
            (9000, 0x7799_5e2e_939e_8812, 0xa065_ff4c_b7f7_ab6f),
            (10500, 0x31b8_d1a4_a0ab_8072, 0xc71c_7f93_56a9_dcc7),
            (12000, 0x22e1_4aac_aa1c_8df4, 0xbd46_898a_8141_1e51),
            (13500, 0xe7a4_cd3e_fb41_9f52, 0x64c5_5625_3ad8_42a2),
            (15000, 0xb555_5ac3_e000_286b, 0xdf38_2b2f_938c_79d2),
            (16500, 0x7847_d00c_7b18_9539, 0x43b6_0bb6_d040_d6f2),
            (18000, 0xb74e_d969_136d_f775, 0x17c4_87dc_7ff1_9235),
        ],
        last: 0xacf1_f02b_b345_4894,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xc020_9bc3_484c_c843, 0x314f_2532_261b_08fd),
            (3000, 0x0b22_828c_3515_945e, 0xde1c_b695_21af_ac7c),
            (4500, 0x0d42_deef_9fae_d8cc, 0xc553_ad39_a1c6_f48d),
            (6000, 0xb507_11de_7bb6_b7f7, 0x873c_caa8_2d34_a4f0),
            (7500, 0xb14e_9d72_fbbf_7e75, 0x614d_ce7c_21c1_d415),
            (9000, 0xc3c6_7a9e_2458_4a38, 0x5d1a_7f4e_728b_73d2),
            (10500, 0x7e92_b787_dc3a_dc3a, 0x641e_4a0e_17aa_5924),
            (12000, 0xf64f_2d94_6a36_44f3, 0x3172_671d_e968_9066),
            (13500, 0xeacb_148c_f277_ee58, 0x172b_db04_5171_1ff2),
            (15000, 0x97d9_5059_b0b2_db33, 0x5f6a_65a5_4788_39bd),
            (16500, 0x5007_0749_66d3_f40f, 0x323b_d703_d045_157b),
            (18000, 0xaf49_f305_9a0b_22aa, 0x58b0_2449_033a_3d8d),
        ],
        last: 0x70c6_16be_81a2_6c7f,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xa2ff_f0d2_d0a3_860b, 0xb2e1_641f_e795_df25),
            (3000, 0x5f17_5c75_a4f2_7bb0, 0x3939_597d_1eb0_e1ce),
            (4500, 0x3e51_84bb_d384_68ab, 0xb435_a2e3_766a_be85),
            (6000, 0x627a_2b4c_ee77_ad0c, 0x8999_5a89_e9f1_ffb7),
            (7500, 0xb999_c51c_45ea_2b0c, 0x1dfc_1f2d_8e56_5d9a),
            (9000, 0x1c55_295a_b675_9f0c, 0x2a48_7803_da4a_5043),
            (10500, 0xb2d0_0e2e_d28f_3437, 0xce99_5244_2653_7c76),
            (12000, 0xe1d8_2818_f893_91f3, 0x579d_b81e_a6d8_65a0),
            (13500, 0x814c_c107_a452_777b, 0x7ba3_d3ee_4d0d_f5de),
            (15000, 0xeadd_65fd_1972_d719, 0x9be2_c9e6_66e8_8073),
            (16500, 0xd2c5_3320_b788_35d0, 0x61e7_123e_ccfd_3791),
            (18000, 0x427b_bc2f_58eb_e1cf, 0x632b_be7a_35f9_df21),
        ],
        last: 0x3d85_bc39_69c5_e11d,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x58aa_3d31_c68d_333a, 0x2268_993a_e2c3_0d6d),
            (3000, 0xc49e_2c56_69d3_8ac2, 0x979e_9469_51b0_aba1),
            (4500, 0xf86d_7bd0_26a0_1098, 0xd7af_6eb3_3790_465c),
            (6000, 0x6371_7ca2_e09f_91b3, 0x1e7f_665e_1d73_ac65),
            (7500, 0x3237_8f77_849b_1095, 0x248d_8103_904e_a782),
            (9000, 0x49cf_7a22_2351_a93c, 0x4be2_be51_50c5_5449),
        ],
        last: 0x5124_a79e_f759_c2ce,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd800_e8c2_edac_d3d4, 0x63c4_9a5a_bba9_486d),
            (3000, 0xa731_5914_aff5_9001, 0x8a91_66ed_a124_d0b1),
            (4500, 0xfdb1_6466_7e64_a937, 0x8139_6678_c418_3c3d),
            (6000, 0x5a24_1d8f_8a66_607e, 0x109e_8a21_2e04_5956),
            (7500, 0xa1eb_6bf7_1862_2117, 0xab5e_a23a_8fe2_5aa6),
            (9000, 0x275c_627f_7ab3_a2ca, 0x6cba_26c1_5504_768c),
        ],
        last: 0xaa20_7166_b39e_7c0a,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd580_df39_b66b_54af, 0x2e5b_7a74_315d_a7ad),
            (3000, 0x9c5d_a810_ade8_2350, 0xc9c5_bec4_2c92_e866),
            (4500, 0x2ff2_d1f9_057a_9b94, 0x9677_6463_a8ee_345d),
            (6000, 0xa678_7095_7033_0a69, 0xf01f_7a83_5a45_4161),
            (7500, 0x5934_cc03_a37d_2f4c, 0x7dde_64df_a950_e205),
            (9000, 0x0a34_e45c_5894_b287, 0x6b77_e950_8f20_de46),
        ],
        last: 0xfc64_e1c2_5b5d_92fd,
    },
];
