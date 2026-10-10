//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, Target,
    Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
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
            10 => Order::Learn {
                slot: AbilitySlot(self.below(me.abilities.len() as u64) as u8),
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
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x6789_677c_81d3_fed4, 0xb06d_c2ef_4f7a_6e74),
            (3000, 0x85da_de12_2b05_8f62, 0x7ffd_7ea9_48b0_0698),
            (4500, 0x5dfa_6b33_f3e4_3003, 0xd2b4_d1a8_4667_8a2a),
            (6000, 0x2295_8703_829e_e703, 0xd407_ea5e_3a4a_6682),
            (7500, 0x8012_63c8_4589_4fc4, 0x9a5e_b1bb_2974_5ce2),
            (9000, 0x7eba_0747_d4ed_52af, 0x9715_9f53_dd8b_b00a),
            (10500, 0xf3d9_fba2_219c_6077, 0xb8f3_9cbc_55ba_7237),
            (12000, 0x7911_9aad_8ea8_18b3, 0xf37d_a7ea_458d_ad0f),
            (13500, 0xd362_7143_56b5_c835, 0x70a7_9222_84c8_b809),
            (15000, 0xd82c_5113_59ec_8480, 0x057e_d61d_5d88_62b6),
            (16500, 0xb478_5be7_e94a_3c02, 0x5ca1_4c02_49fb_e854),
            (18000, 0xae48_b7ce_d68a_b3d2, 0x0fa6_6fde_122e_1022),
        ],
        last: 0x49dd_5824_0eba_b246,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x3f4d_e3a7_296e_39f8, 0x9cbc_e8e8_e276_1207),
            (3000, 0x17ad_25d3_0177_425a, 0xec75_65a5_2af6_ebdc),
            (4500, 0x972e_72cd_9dff_192f, 0x498d_d18b_64cb_e2bf),
            (6000, 0xf559_0b9d_6d5d_e8d5, 0xa4de_549f_ae57_8b08),
            (7500, 0x8c72_c24f_0f3a_0874, 0x043e_5e89_b789_8c8c),
            (9000, 0xebb8_d96b_9b8a_7e30, 0xe69c_62d2_c27b_da3f),
            (10500, 0xeace_eade_9595_fe30, 0xf1fa_b511_6533_50a9),
            (12000, 0x4f90_a142_91cb_5c70, 0x020d_893a_f9a1_6070),
            (13500, 0xbc20_d263_c1f5_2684, 0xd9da_4fcd_5413_8eab),
            (15000, 0x1d70_0d8f_1068_e371, 0xf390_fe8c_d1b8_0ce7),
            (16500, 0xab27_d1ee_7521_1d7a, 0xc9ca_b930_60ef_ec10),
            (18000, 0x4c34_dc03_b96d_2e1d, 0x4ce3_76aa_e4d8_2e2b),
        ],
        last: 0x7505_e6b9_ae41_68cd,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x732a_bbf6_5645_9aec, 0x5bdd_912e_1c23_6a2f),
            (3000, 0x794d_f673_7048_24e4, 0x4e4f_4424_5889_c511),
            (4500, 0x6aec_426a_a891_058e, 0x69f1_3e42_78c3_f8c1),
            (6000, 0xcd55_e503_fe9f_7102, 0x93fb_e8e7_1ebc_2bcd),
            (7500, 0x8d2a_63f8_0256_401c, 0xa703_b631_154e_053e),
            (9000, 0xcec7_a0f0_06d0_a4c5, 0x8f56_a760_6c83_fc80),
            (10500, 0x3d40_7c9a_d5bf_cd9e, 0xcd3f_1154_ad50_d4d0),
            (12000, 0xe042_dbaf_6154_2823, 0xf4bf_f0a7_527d_31bb),
            (13500, 0xfd1d_39f8_4320_7234, 0xf911_dd38_dc16_6bce),
            (15000, 0x400c_b48d_30b6_e85e, 0x0517_8429_915a_d5ca),
            (16500, 0xe215_8707_d3ea_7c60, 0xdd42_9f95_7223_0bef),
            (18000, 0xd570_3c85_2ec8_0082, 0x829a_0cd2_88a8_aa92),
        ],
        last: 0xb293_3308_30b6_bb5e,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x2bc8_5d31_cd28_057f, 0xce0e_b6a4_2fa2_8b4b),
            (3000, 0x78d6_8127_3792_3b32, 0xde36_d3dd_66ab_6e9f),
            (4500, 0x9cc3_ac76_cf78_0c63, 0xdcdc_aa1c_02ac_0c5a),
            (6000, 0x3ce9_8d07_1f92_0922, 0x2291_bc0f_f0bd_c993),
            (7500, 0x74ac_dc03_61f7_ce19, 0x1dd9_8f4d_0ccd_0dc9),
            (9000, 0x066b_cebb_7c28_91b0, 0x647d_4ecc_7ad8_becf),
            (10500, 0xbc51_127b_88c7_cc11, 0x37c2_f613_ee27_38c6),
            (12000, 0xeabb_1f1d_02a4_414c, 0x18ba_06ed_4cb6_9327),
            (13500, 0xc934_8a1a_2d35_2217, 0x7570_f9f1_ab2a_8a45),
            (15000, 0x84d8_fcb7_5a89_9709, 0x2f76_e5f7_0ac5_2b6a),
            (16500, 0xd5b8_76e0_ab19_ad90, 0x7944_d76a_9617_0b88),
            (18000, 0x1383_bcd7_fedb_55d7, 0x1d6c_4b5e_0219_fbd3),
        ],
        last: 0x9024_4410_2c17_87df,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfafc_b3f8_b321_1af0, 0x00ee_607e_a100_f91c),
            (3000, 0xadc3_eab3_5cfa_3f27, 0x3b4b_c434_6bbc_997b),
            (4500, 0x0da8_f4d1_4066_125d, 0x33ef_eaef_4e74_32d3),
            (6000, 0xd2ed_1277_3ecd_323d, 0x4287_eaf0_a446_5d9f),
            (7500, 0xb73a_76e9_baa5_ae1d, 0x4658_ce80_2d4a_076c),
        ],
        last: 0x583f_07f1_2a85_704a,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfab4_1943_a6e6_d24b, 0x1931_73c9_1798_9ec5),
            (3000, 0xe6e4_2339_6ed1_7f88, 0x7375_6a94_cc47_5260),
            (4500, 0xd15c_8ab1_3409_6809, 0x6cd9_ceca_fb43_2dad),
        ],
        last: 0x85f9_b20d_23ce_0f3a,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x1d8f_1e56_0f2f_4132, 0x8902_4edc_a19b_21e4),
            (3000, 0xe24b_71d3_c56e_315d, 0xdd07_9066_da70_177c),
            (4500, 0x66b4_a8cb_76c9_d956, 0xea37_2611_e4c1_4009),
            (6000, 0x6a9d_7ec0_9519_a59c, 0x39c5_8ec2_0598_c04c),
        ],
        last: 0xbfdc_eb89_48d4_8d7b,
    },
];
