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
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x01c6_3eca_2537_c847, 0x62f9_8c9c_abd9_712b),
            (3000, 0x75cb_8f57_cc17_540f, 0x44cd_ac03_43b4_40ec),
            (4500, 0x5a01_c576_9294_a3bf, 0xcb6c_40da_6ed3_a43b),
            (6000, 0x3668_1d3e_fd7b_acc3, 0xd0ef_14ee_281f_b0c0),
            (7500, 0x0a46_74d1_5f26_cc54, 0x8e6d_3686_2628_d82d),
            (9000, 0x6321_c6f6_1224_aa62, 0x6f6d_8969_b7d9_6401),
            (10500, 0xfe9a_9a5f_bfcc_5e44, 0x4159_2156_e125_5a46),
            (12000, 0xe2db_54b1_75f6_a4c3, 0x0fea_0895_612c_ed26),
            (13500, 0xaa99_c6a9_cb4f_93ba, 0x986b_c45d_9747_276d),
            (15000, 0x5f46_701d_6311_198a, 0x408c_00f1_0972_7cb6),
            (16500, 0x560a_6557_3035_5afe, 0xa099_3cd8_7c34_eda6),
            (18000, 0xceba_8a08_7be6_682c, 0xc35f_20af_32dd_680b),
        ],
        last: 0x0c9c_03d0_b747_f92e,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xc507_ed67_7efe_0fb1, 0x56ec_4e50_5723_1fb7),
            (3000, 0x6ab2_085d_4c93_ccb8, 0xcd8d_8a84_71f9_4a96),
            (4500, 0x4c28_fdfd_b939_f165, 0x7f7b_f92a_4adc_e84e),
            (6000, 0xb527_fa48_f874_d540, 0x60b9_71e1_4b7d_622b),
            (7500, 0x73dc_ef75_415f_4e29, 0x249c_9726_a7cb_78e2),
            (9000, 0x7d31_46e9_2d52_6940, 0x87ae_28b9_35a0_62e5),
            (10500, 0xf9c3_0be8_1cfe_e44d, 0xa561_836a_0698_f31b),
            (12000, 0xe97f_f249_f9e5_daa9, 0xdc29_6b37_5390_4d77),
            (13500, 0xed14_c117_3d70_607e, 0x6095_954c_600c_3432),
            (15000, 0x2aa4_a270_b4c2_138e, 0xb654_15be_a70e_2742),
            (16500, 0x157e_b510_4dd2_ef88, 0x9b77_11d6_434f_1696),
            (18000, 0x7762_c60c_c25d_88cc, 0x10bf_86da_1bc2_7351),
        ],
        last: 0xaa09_5165_86df_da6d,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x0d1f_6417_01df_6be7, 0xbfb4_e510_15e3_409f),
            (3000, 0x816b_2bbc_6db1_05b4, 0xda9e_6048_7007_724a),
            (4500, 0xb62c_a439_063e_03a4, 0x61af_7100_ca82_07f6),
            (6000, 0x423d_8910_2b4a_6343, 0x4aa9_5bb2_5b63_6e9f),
            (7500, 0x57a8_1885_acf6_bfc3, 0xec67_52f7_41c0_ecd8),
            (9000, 0xabea_d772_a335_1cd8, 0xb0f5_c3dc_35ea_3b4b),
            (10500, 0x7a2f_4d7e_d3d9_65e8, 0x512f_6685_07ba_6359),
            (12000, 0xd711_e9e7_aef9_00d5, 0xca17_030b_4df8_9b43),
            (13500, 0x393e_50b1_be57_159c, 0xed45_0fdb_5859_66ee),
            (15000, 0xb90a_0b5b_7b64_ec39, 0x7955_5fa4_3076_76ca),
            (16500, 0x7b09_6e8c_1cd0_e3d1, 0x16c9_12e4_2fea_e26f),
            (18000, 0x76e8_d268_724e_0b16, 0xd531_9355_9b5d_7e04),
        ],
        last: 0xc5f1_3193_52d3_4bf1,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xc0f9_845c_03cf_3ff1, 0x06a4_4d97_2b6a_0530),
            (3000, 0x939f_a753_e89d_219e, 0xc139_b039_da40_0b96),
            (4500, 0x6f95_1278_3fdb_f320, 0x1ca4_76ec_af91_35f9),
            (6000, 0xe815_31db_eed7_96d0, 0xafa9_6636_0f02_d42d),
            (7500, 0xebde_f148_b607_07f3, 0xfdd0_ace0_167e_e8db),
            (9000, 0xc2d0_06d0_367a_4f8d, 0xedc9_5d08_a8bd_fbe8),
            (10500, 0x0f09_7de4_f71b_17bb, 0x8f30_2148_fb8b_c0be),
            (12000, 0x69b3_5277_6edc_c058, 0x38e9_514a_9e1e_cb06),
            (13500, 0x82cf_f992_cf93_1dd4, 0x82f4_90a6_7e09_ad22),
            (15000, 0xb2f4_d7e4_5cb2_f1ae, 0xc350_0b71_75d8_32d6),
            (16500, 0x2654_f91c_ebf1_dbc7, 0xa74b_f88f_a706_9633),
            (18000, 0xb0b3_b685_e58b_f811, 0xc1a4_63ed_0771_7bbd),
        ],
        last: 0xb739_4dcc_6cc1_c197,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x4378_efde_ac60_936a, 0x0c1c_d0fa_8dfe_77f6),
            (3000, 0xf671_9c2d_2296_3762, 0xe2d3_662a_4a32_006c),
            (4500, 0x6695_6afa_bf91_e792, 0xab21_6c3b_621a_8e8a),
            (6000, 0x6cd6_b954_2f8a_2869, 0xf137_102e_dcd1_cd09),
            (7500, 0x49f5_b561_e268_4be7, 0x9896_5ba0_3dbe_0de3),
            (9000, 0x273f_7bac_cf2b_b368, 0xb195_6c77_0a81_ff00),
        ],
        last: 0x768e_9df6_2a9a_1be6,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xe49b_7e96_e630_1304, 0x4083_1f8e_a2d8_817b),
            (3000, 0x2e81_f0fa_54ce_3459, 0xeef2_4fac_2131_8ff8),
            (4500, 0xc51e_ebd8_b7cb_dbbf, 0x9838_aa92_13bd_4e98),
            (6000, 0x960a_b280_3f84_fb66, 0x2695_6bea_c404_0027),
            (7500, 0xb7ea_1046_2cfb_817b, 0x8bf5_77f6_b303_d075),
            (9000, 0x2ada_dd69_1188_3fb2, 0x0c2c_1415_434a_b2fd),
        ],
        last: 0xcdba_ad45_d364_0df0,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x331c_39b3_794f_bfbf, 0x0dec_f9d6_a6e2_419f),
            (3000, 0xd57c_1271_e122_74f2, 0xa430_7e16_4083_86b4),
            (4500, 0x24a2_f8e0_34fd_be14, 0x615e_abcf_7105_3199),
            (6000, 0x15cf_1793_1f24_ae2b, 0x26a9_73d3_90ae_dd9a),
            (7500, 0x1135_a939_d90e_2b70, 0xed6e_af5e_687b_869d),
            (9000, 0xc434_c6ff_6b36_386d, 0xc2ac_7cb9_ace9_7658),
        ],
        last: 0x6f57_1c77_b3f1_611e,
    },
];
