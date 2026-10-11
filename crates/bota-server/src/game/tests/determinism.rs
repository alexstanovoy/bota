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
            (1500, 0x99e8_a84a_9a73_f517, 0xefad_22d1_a272_e515),
            (3000, 0xb0fd_2bd6_d43d_dacd, 0x27ce_92fe_d322_46a3),
            (4500, 0x0e12_56e6_47ab_74e9, 0xf76d_abd0_c6e2_69b2),
            (6000, 0x7aea_6fd7_c380_8ce1, 0xc5f5_2c3d_1496_b687),
            (7500, 0xa5a9_ce1b_030f_dc72, 0x76d1_3aa6_2b8f_ddf5),
            (9000, 0x9b49_bd61_438d_7c3a, 0xf113_d3a4_a152_86a7),
            (10500, 0x0bdc_6d8d_a184_c8c3, 0x1e81_937f_eb0b_f3de),
            (12000, 0xc437_636c_8326_093b, 0xac0b_7351_d0a0_2595),
            (13500, 0x8332_c1d4_613c_c5e8, 0x161a_07ee_3525_db85),
            (15000, 0x4274_442b_60cd_ef01, 0xb079_b9a3_f1a3_b400),
            (16500, 0xb207_70d6_e7ed_7088, 0xebaa_643d_4af0_a33d),
            (18000, 0x5e50_7176_6f25_a2a6, 0xf2de_4739_57ed_b2e2),
        ],
        last: 0x4202_2bad_9051_922a,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x69fb_e57a_4ff6_353e, 0x8cd9_3271_2543_e0f8),
            (3000, 0x02d0_1fb6_5007_f950, 0xaaab_e0ed_5ba3_b7b1),
            (4500, 0x204c_daaf_5d9a_9a93, 0xf132_c1d2_259f_a766),
            (6000, 0x9c0c_b051_184d_25a2, 0xe94d_7a06_9ecc_8a34),
            (7500, 0x4fee_4740_4b32_9d84, 0x05bb_f302_1a09_fe71),
            (9000, 0xd3b3_6afb_107f_d47c, 0x66d4_b277_10f9_a8e7),
            (10500, 0x14a2_a4dd_ecd2_6526, 0x9952_f45b_a6cc_36cb),
            (12000, 0xa6ee_931c_3b17_34f8, 0x637d_ef63_0f56_152b),
            (13500, 0x57e9_4362_ddea_4c28, 0x473d_ba99_a33c_50b4),
            (15000, 0xad7f_dd13_25e4_ab95, 0xc1cb_37a8_eafe_cf90),
            (16500, 0xaaac_6b6c_5bbb_0b9b, 0xe4b8_bafc_a448_46e9),
            (18000, 0x4c52_ce2a_a652_f24f, 0x764f_f051_0ea4_5b09),
        ],
        last: 0xd9d4_9c3a_13bf_2695,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xcc80_347d_e135_07a8, 0xf578_b9f3_d081_a713),
            (3000, 0xee34_0c7e_fa08_a81d, 0x4f43_659a_37bc_e555),
            (4500, 0xb2c5_6d35_d0c1_da79, 0x469e_adae_821b_1017),
            (6000, 0x0f35_1adb_b742_82ee, 0x1d3f_0d08_11bf_7212),
            (7500, 0x8a67_bc2a_dbb1_1365, 0xb995_c706_9b83_fd70),
            (9000, 0x835c_4947_2114_ed22, 0x32ec_48fd_e058_e719),
            (10500, 0xaac9_172c_547f_d083, 0xe57f_8e53_c95b_9bb3),
            (12000, 0x5ca0_5bd3_db4d_5856, 0x0e15_c857_b89c_c235),
            (13500, 0x8e81_7ad2_9b7f_7062, 0x9982_fb20_a0e7_4c67),
            (15000, 0x4244_b13a_554a_b775, 0x1a56_c5fe_3202_a1a8),
            (16500, 0x8d5d_499e_bd51_e6f0, 0x1d99_6185_d0b7_0585),
            (18000, 0x3248_d171_14c0_f09b, 0x758b_4313_28d5_89d8),
        ],
        last: 0x8b43_1c61_17c7_71ff,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xb957_93df_6229_1014, 0x4dd4_a361_8c19_b1a7),
            (3000, 0x6e9e_6f7f_12fd_20a7, 0x1543_940a_05d6_5b14),
            (4500, 0xdc35_b8f6_f955_5596, 0xd729_50e2_2441_f658),
            (6000, 0x2770_9a70_d806_0487, 0x9489_5762_fdac_77b2),
            (7500, 0x0732_8932_e9e0_85d5, 0xf9e7_49ec_300a_b602),
            (9000, 0x11f0_202b_515d_f87d, 0xe1e3_9241_0127_5c1f),
            (10500, 0x7bc4_0b79_a8e1_d88a, 0x931d_01f3_ba4e_fb36),
            (12000, 0xfcbc_820e_040c_494a, 0xa5f2_28f8_4d07_efef),
            (13500, 0x54ab_56f3_185b_c0ec, 0xea01_022a_0714_760e),
            (15000, 0x472f_1567_b9cd_4c9a, 0xc1e2_4d3d_7604_ec80),
            (16500, 0x6813_1f2f_2da9_84a5, 0x9a86_13a1_6b21_69a6),
            (18000, 0xd8be_9ce0_56db_d7b4, 0xcfe1_0749_3f2a_5128),
        ],
        last: 0xa4a6_7953_7432_148a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7527_1669_5dba_3a8d, 0xf9ac_27dd_4659_9e71),
            (3000, 0x98d5_1402_50cb_3cff, 0x3098_c237_a1a3_5408),
            (4500, 0x5372_b959_c0b8_cd83, 0x9951_5f7e_9c03_3091),
            (6000, 0xc789_1071_15b7_5c92, 0x03c5_9606_3436_2c0e),
            (7500, 0x93d4_811b_661d_80d8, 0xccf1_da5e_8891_9919),
            (9000, 0x23c1_5888_82f1_3717, 0x30a5_159a_8686_53a8),
        ],
        last: 0x7ac7_5397_04c2_c61c,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xe024_81aa_a935_0846, 0x00a5_1504_f4e5_6953),
            (3000, 0x633c_92df_3a3d_49a5, 0x5373_7cd8_ff95_bf45),
            (4500, 0x4e59_8627_7b80_4ae7, 0xaf88_9c1c_11d7_43c1),
            (6000, 0xe8ea_ecf9_3eac_188b, 0x4817_7fc2_e716_34d6),
            (7500, 0x5091_3bcc_2262_f921, 0x983b_2014_bb6f_c1f4),
            (9000, 0xa8b9_8534_a984_99d0, 0xed22_13ee_a84c_9629),
        ],
        last: 0xf8ba_1aaf_498d_0dcb,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2f7f_4b1c_59b9_6e78, 0x5442_2847_238c_5eb6),
            (3000, 0xac41_d6bc_3b22_f337, 0x288f_ae95_31fd_db4f),
            (4500, 0x75cd_00b4_82a1_2439, 0xdd87_3b4f_4237_444b),
            (6000, 0x77e8_20eb_8924_18c6, 0xc32a_bf64_8aed_f9f4),
            (7500, 0x345c_35e5_0fcc_54a1, 0xa559_95a6_2087_1f8d),
            (9000, 0x9995_b707_5103_da84, 0xfc01_21b0_983a_585b),
        ],
        last: 0x99d7_e385_3a1f_ef5f,
    },
];
