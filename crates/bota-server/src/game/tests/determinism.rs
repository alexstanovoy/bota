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
            (1500, 0x6789_677c_81d3_fed4, 0x3dd6_22f8_0272_59d1),
            (3000, 0x6b59_084a_f492_afb5, 0x7d58_f928_7b3b_319f),
            (4500, 0xd183_5166_e3bb_af4a, 0xe0aa_ba93_073b_bb8a),
            (6000, 0xda9b_1908_d8ac_b79e, 0xbdd3_f3d7_2d4e_9bf5),
            (7500, 0x0f1a_95f2_9551_f5cf, 0xf73e_fd42_64f9_1264),
            (9000, 0xbd63_5ef6_6aa2_9d3c, 0x4722_652e_7ede_c3b8),
            (10500, 0xb1e7_b4d7_f150_0b30, 0x3a32_c99b_896c_e91d),
            (12000, 0x5d62_ff70_312b_89bd, 0xf6e4_0a3b_f335_31bd),
            (13500, 0xdf27_5953_1baf_1a90, 0x00a1_7784_ffcb_902f),
            (15000, 0x75ad_fc9f_d996_38cf, 0xd325_7cc7_d85a_0d0f),
            (16500, 0xc53d_7a38_738f_8615, 0x28d7_0ad2_6e5c_0424),
            (18000, 0x66f7_5863_327a_afbb, 0xd152_c5c0_92dd_7bad),
        ],
        last: 0xd1a3_d16c_02f1_644f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x3f4d_e3a7_296e_39f8, 0x18e1_66ee_37a2_6248),
            (3000, 0x9d86_bb38_2a39_e3a3, 0xa56c_9ca1_6fa5_b48d),
            (4500, 0x4ac4_2679_da91_a9d4, 0xe7a9_e4c1_b0ce_1e3a),
            (6000, 0x4439_556c_5da9_ed51, 0xf60b_492d_c6ca_50ff),
            (7500, 0x210a_f1d0_91a7_cd18, 0x9142_1e0b_a7d4_b761),
            (9000, 0x9f67_5b26_f62b_22cd, 0x4a96_0f0a_2fee_a25a),
            (10500, 0x82ad_27b9_0d3e_7496, 0x55a7_0149_f313_e4da),
            (12000, 0x0753_a262_07ba_c5a4, 0x6d61_69a7_3b32_6e18),
            (13500, 0x0746_042d_4b79_fca1, 0x838e_d43e_4069_c736),
            (15000, 0xc3fa_674c_3440_4524, 0xd4ce_29e7_3492_e0e9),
            (16500, 0x3e86_16aa_f6f6_8a85, 0xa65e_5488_4c9f_774b),
            (18000, 0x2c60_5a20_4299_cb8d, 0xf51c_19cd_9ab2_b4b5),
        ],
        last: 0x6139_0dea_46b8_172b,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x63f8_238c_4353_46b8, 0x4cae_e0cb_f1ce_54ec),
            (3000, 0x79c7_94fb_6ff7_7176, 0xd780_a956_2476_26ba),
            (4500, 0xbe91_fac0_5b2d_260c, 0x2306_3a61_b0d8_efb1),
            (6000, 0xd2f7_f7a2_36de_faa5, 0xdb31_b384_9fe4_a03c),
            (7500, 0xa922_534e_465f_a068, 0x566e_a389_927c_3695),
            (9000, 0x5f6a_cac6_df53_ea63, 0x4924_4c09_83ae_e67b),
            (10500, 0x3d6d_5701_d435_9183, 0x8107_02a0_6955_1cb2),
            (12000, 0xdf4c_4db8_936a_f517, 0x930c_a764_8453_ff5d),
            (13500, 0x4413_dcc8_3cea_4f59, 0xfb58_1f66_9958_c541),
            (15000, 0xf4f7_47ed_a960_c54f, 0xae26_6632_72e9_dd31),
            (16500, 0xce1b_7b1f_f642_1c37, 0x30fd_881f_e7d2_e101),
            (18000, 0x67c6_386d_b1dd_f815, 0x77c8_319a_4eff_31b8),
        ],
        last: 0x5c28_1c9c_097d_40a4,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x4bcf_7e40_4a2a_3998, 0x8336_de4c_94be_48bb),
            (3000, 0xb692_9563_497f_32c4, 0x2e0e_6ac2_b179_5d63),
            (4500, 0xe611_0aad_c750_8e3b, 0xdc8f_7ef1_f4b4_d818),
            (6000, 0xf953_db37_d146_6a0b, 0x10b0_68d3_8a12_f63c),
            (7500, 0x287b_51ad_22c9_0437, 0x42ca_6acb_f78b_68b3),
            (9000, 0xa6b2_a7f4_b1c7_fa1f, 0x5bc7_298d_6c90_f408),
            (10500, 0xd6d7_c735_9746_2c7f, 0x62f4_3d25_8393_4221),
            (12000, 0x1d05_573a_62f9_6a58, 0x40f4_6944_0aac_f93d),
            (13500, 0xc0c4_1fd4_32e9_bced, 0x4b99_dff2_066e_29e4),
            (15000, 0xa5eb_90c5_5bf8_3f73, 0x7817_373e_16c4_dab2),
            (16500, 0x2220_1471_643a_4075, 0x4e71_9f7c_2ed1_93a1),
            (18000, 0xf7c1_7420_24f8_900c, 0x8a91_bce7_2465_400d),
        ],
        last: 0x4c57_987f_f18d_0803,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfafc_b3f8_b321_1af0, 0x5b5a_d961_c2fb_b268),
            (3000, 0x276e_15a0_224d_a9d5, 0xda2d_c03a_5cce_5b1d),
            (4500, 0x585b_ffa5_e5b9_61b7, 0x935d_4548_f428_cc5d),
            (6000, 0x6ffa_85fb_159f_9d8b, 0xde1d_9ed9_2a4e_be65),
        ],
        last: 0x5b2a_676d_b281_ea40,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfab4_1943_a6e6_d24b, 0x30e4_0574_bc4c_292d),
            (3000, 0x27e6_f4ba_9fbb_9a6e, 0x1902_f3eb_cfd8_a61d),
            (4500, 0x0561_a411_7db8_7612, 0xa211_f276_f814_c048),
        ],
        last: 0x9912_75c1_8c0d_925a,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x1d8f_1e56_0f2f_4132, 0x840b_d609_8db3_38a4),
            (3000, 0x7e98_11d3_fa64_8efa, 0x7dd2_ddc6_78b2_7eb2),
            (4500, 0x0bdc_a6f0_dd19_cd11, 0x6704_952f_b644_3d8c),
            (6000, 0x77d1_dae1_8e83_5a44, 0x7bc5_27b3_4886_1b92),
        ],
        last: 0x16e2_5000_4872_adb7,
    },
];
