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
            (1500, 0x73cc_3c6e_af5c_4b9a, 0xd6dd_4836_1695_d456),
            (3000, 0xe519_05ba_481d_49dc, 0x78c0_a270_ceaf_402e),
            (4500, 0x79ae_c89a_c4e9_e827, 0x07eb_6e7b_8ab3_72d5),
            (6000, 0x2627_0080_22fe_0623, 0xb4f7_ae48_f21b_41b4),
            (7500, 0x4dbc_792f_7a08_c929, 0x5fe0_e47a_7032_93a7),
            (9000, 0xda79_771a_aaf6_8a00, 0xf433_ff99_0a9a_e844),
            (10500, 0x5bf9_6e97_5d8b_11fc, 0x9d69_8ffa_2425_cabf),
            (12000, 0xc244_9657_0c84_a927, 0xed2a_e0cd_9db7_db58),
            (13500, 0xf409_75c4_16c5_c0ea, 0x79b3_0f44_45d1_1b31),
            (15000, 0x00a0_1f2d_3dc4_08f5, 0x8446_1201_638b_78aa),
            (16500, 0x6ad9_e2e8_82f5_18dc, 0x35a9_6e87_38dc_e63e),
            (18000, 0x33f9_2f7f_08fc_1559, 0xf22f_3533_8b23_1a8d),
        ],
        last: 0x6201_092c_8c4f_61f6,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xac2c_8531_0a45_07d1, 0x0c95_340a_8c4d_f981),
            (3000, 0x595e_94b1_b18a_1083, 0x557e_432f_01f6_37a7),
            (4500, 0xf733_3c90_0ee7_aaa0, 0x3e41_03c1_85df_8686),
            (6000, 0x7fda_371e_3854_d996, 0xab6c_0502_ebdd_b8bb),
            (7500, 0x2918_c62e_79fb_036c, 0x4ecc_470e_5c03_cff2),
            (9000, 0x2651_4557_b164_f1fa, 0x23ac_8c0d_ad26_596b),
            (10500, 0x5ae3_aa57_9134_c3ba, 0x4d84_119f_b241_ce95),
            (12000, 0x7a26_0142_9d8b_daa1, 0x986f_70ba_87e7_aca3),
            (13500, 0x73bf_c7b5_a648_7c2a, 0x53c7_40d3_5a56_cea4),
            (15000, 0x6abc_7e1d_d2af_a108, 0x8871_27de_b13f_fd1e),
            (16500, 0x6ded_1785_daf8_f451, 0x63a3_a217_d31c_5c08),
            (18000, 0x3be2_b6de_5c6a_0a86, 0x5cf5_f24d_f6b1_205e),
        ],
        last: 0xfdbc_a0c0_79c2_d444,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xf2c3_0579_1881_f08b, 0x5911_5667_e974_4b39),
            (3000, 0x6524_3497_69f7_8af0, 0xbab3_dbc0_99b5_de29),
            (4500, 0xa644_7629_ee7a_97a2, 0xc0b4_81ad_75d6_a9b8),
            (6000, 0x9326_49f8_9a37_c7ac, 0xf840_113e_eb32_f666),
            (7500, 0xefe7_9b16_2f8b_7f24, 0x7efd_fa73_f336_8702),
            (9000, 0xb2bb_8a09_9292_73a9, 0xfa15_ae83_e641_1d21),
            (10500, 0xf498_19fc_daf3_0fe5, 0xa53d_9f70_d6b8_3ca1),
            (12000, 0xab08_7ede_d8f0_155c, 0x314b_e963_4ac7_05cf),
            (13500, 0x4846_e23c_72bb_f0ed, 0x4d82_a559_2c4b_2488),
            (15000, 0x9553_05ad_8e64_ce30, 0xefd0_b9a1_ca21_ed32),
            (16500, 0xe674_0fcc_5e05_4063, 0x2ebd_0991_0424_ca1f),
            (18000, 0x6d6d_7b71_6ffe_178c, 0x2fdd_8711_f55f_600e),
        ],
        last: 0x8013_f073_f7aa_1835,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3310_5d12_9cf6_004b, 0x9b26_eeb7_5981_ec8d),
            (3000, 0xacee_2194_8595_7e0f, 0x92b5_1059_edcf_e9f6),
            (4500, 0xe5f3_2058_0d70_8e30, 0xa1e4_47da_d4cc_dc57),
            (6000, 0xe08e_d528_347c_db7a, 0x2047_ac08_8ab3_6b27),
            (7500, 0xe209_d230_f1dd_ca71, 0xf9de_e6e6_00cf_0b42),
            (9000, 0x3d15_2ee9_1223_72b0, 0x712a_14bc_32a1_66f9),
            (10500, 0x4484_4985_e426_885e, 0xee19_4242_e972_b16f),
            (12000, 0x9fdc_08d9_9680_b664, 0xf367_e13d_f4bc_0708),
            (13500, 0xecab_460e_8744_c432, 0xc77c_1eef_235f_9ebe),
            (15000, 0x8dfc_dbf4_55de_7b57, 0x1737_813d_dfda_7180),
            (16500, 0xd456_80ef_8f31_73a6, 0xee5f_7182_aa12_8760),
            (18000, 0x9d79_a985_5ccc_0ddb, 0xdfa4_8f36_692a_a2c9),
        ],
        last: 0x41aa_53d7_4fc3_038c,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x2fae_68e3_728b_a7d9, 0xbd94_7959_80c7_bb8a),
            (3000, 0x5502_af73_6b5a_597c, 0xb63a_71d3_c27e_932a),
            (4500, 0x0979_d73a_376b_eeea, 0x5b20_92c6_2350_ab01),
            (6000, 0x407d_c454_bb58_2962, 0xabbb_dafc_f9ce_e1d2),
        ],
        last: 0x0065_9cf5_db43_1546,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x9941_d8dd_1733_f061, 0xb5a6_b764_7546_7637),
            (3000, 0x9cde_0cf0_fff0_e24a, 0xc32c_412c_ddc7_6b82),
            (4500, 0xc80a_eb4a_c047_87e3, 0x9343_c950_e42c_211a),
            (6000, 0xa263_9287_2ace_8e40, 0xe588_209c_a483_bed3),
            (7500, 0x5211_e068_c74a_977b, 0xc615_fc43_df14_83e0),
            (9000, 0x04a2_0a75_faa5_40ef, 0x3207_7395_2715_3237),
        ],
        last: 0xfc68_ae50_f32a_e85d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfb4a_d503_6818_7715, 0x4cd4_782f_1083_e7c1),
            (3000, 0x4c1c_9bf8_a513_5660, 0x1a1c_865d_fe66_9d85),
            (4500, 0x4def_d504_554c_ddba, 0x4eea_bded_266a_8c44),
            (6000, 0xce5b_8d3c_2f9e_9682, 0xb7fe_3acd_9730_6859),
        ],
        last: 0xe3a5_36c1_b52b_b0d4,
    },
];
