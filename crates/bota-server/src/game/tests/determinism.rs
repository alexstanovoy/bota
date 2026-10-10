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
            (1500, 0x6713_09c1_5d5f_f60d, 0x485d_dbfd_393d_38a7),
            (3000, 0xe6d4_1b0d_2889_441f, 0xdab8_11ad_9d29_028e),
            (4500, 0xfcec_84c3_033d_04c5, 0x6af4_ec0d_a44e_de62),
            (6000, 0x3c93_5615_2820_e68e, 0xa632_3ac2_4299_44f7),
            (7500, 0x1f7c_04fe_e44a_4b23, 0xbc0b_d44c_108b_6f56),
            (9000, 0xf229_8b25_953b_2866, 0x1d64_21c4_6f78_2984),
            (10500, 0xbee2_6f02_3a57_ef4b, 0x1373_70df_2c58_3ca4),
            (12000, 0x734f_8821_34b6_d52f, 0x361d_54ea_0682_ed1c),
            (13500, 0x81f8_df2b_d49e_2487, 0xbadb_6126_ff40_38d6),
            (15000, 0xefa3_e82b_91e5_2d4b, 0xbf31_a9df_b9c2_b2f0),
            (16500, 0xe3f4_aefe_ed13_f367, 0x155c_f9dd_8f54_c790),
            (18000, 0xe36e_90ce_93a1_4577, 0x2fd9_8dc0_2cf4_904a),
        ],
        last: 0x292f_8342_efa1_5ba1,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xf034_4e9c_85d1_9bbe, 0x28df_5d39_a620_1d00),
            (3000, 0xe6c5_ee2e_1694_6707, 0x0a69_a524_d812_8d16),
            (4500, 0xe96a_1940_8599_c800, 0x353f_c61c_fba6_f3f7),
            (6000, 0x2f9c_7a28_b0f5_d503, 0x6e81_ffd3_714a_a583),
            (7500, 0x62ee_118d_5d98_7338, 0xc569_5500_a56a_baee),
            (9000, 0x4811_32b7_5c04_0288, 0x7f18_2b82_76b0_67bd),
            (10500, 0xaa5b_2844_efe5_d906, 0x5050_6e54_e518_490c),
            (12000, 0x9051_4d33_a509_0737, 0x8401_d43d_64bb_5634),
            (13500, 0x3feb_8963_559a_39a4, 0x4a35_0bdf_a6f5_3fd9),
            (15000, 0xe503_2f5e_6af6_377e, 0x1861_c00e_98ba_36b0),
            (16500, 0x4d09_6d25_1c96_25cd, 0xde4f_a483_253d_a359),
            (18000, 0x3ffe_6c5e_f930_990c, 0xda81_dfa3_6b0a_0255),
        ],
        last: 0x321c_e43e_2a3e_80d4,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x49ca_aa6d_61da_803c, 0x2fdd_0347_302a_8010),
            (3000, 0xe4d0_761f_1c4d_a03f, 0x3f40_b0ec_34fe_bc42),
            (4500, 0xe0a7_55ae_a6d7_cf16, 0x1d30_14e4_11aa_b694),
            (6000, 0xec6d_96fc_2362_5fb5, 0x7328_3274_8ce9_bd8c),
            (7500, 0x2da0_b322_0a0f_1c12, 0x6b9b_4666_fddc_964a),
            (9000, 0xd603_5f45_a3ea_dcb5, 0xe828_4ee6_0e59_2498),
            (10500, 0x6d69_337a_b002_b469, 0x88b1_4251_c513_7d98),
            (12000, 0x3977_bf70_03a9_eca6, 0x6ac8_f697_b405_1b79),
            (13500, 0xda85_0d68_d2ab_c1bb, 0x197d_cbb5_80a3_de1a),
            (15000, 0x15c1_22c0_33e0_54d4, 0x93ac_a310_f225_c9f5),
            (16500, 0x1491_cb14_23b8_189d, 0x36b9_54e1_eae2_024a),
            (18000, 0xaf45_7bbb_0da6_a806, 0x17d0_fcf3_96a1_cec7),
        ],
        last: 0x1c9a_ef93_4eeb_e71f,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x8e61_01a2_dff2_a60b, 0xb464_f428_b7b7_f32d),
            (3000, 0x65f8_be02_aac4_cfad, 0x4a06_b9fd_4dfe_ea76),
            (4500, 0x11c9_2ca2_824d_8a3b, 0x39a9_d133_eced_cf14),
            (6000, 0xd130_f6f7_980f_09e4, 0x403f_c896_b4af_1c11),
            (7500, 0xd2af_9ab8_c626_a353, 0x6ba6_8973_6799_7863),
            (9000, 0x9d1e_4388_9f3b_840f, 0xba1c_d765_a390_e019),
            (10500, 0xd34c_6a96_9738_3ef7, 0xdb30_ebe0_679e_3fae),
            (12000, 0x6d82_944e_f90a_ef39, 0x6775_5a15_5328_b909),
            (13500, 0xad36_58a9_0baf_f8de, 0x48f8_961f_370d_93d1),
            (15000, 0x0faa_602c_99f9_02fd, 0xa1c8_1b95_3e94_9d40),
            (16500, 0x6596_40c5_756c_1ead, 0x76e7_f5cf_cdd4_4cda),
            (18000, 0xdada_e14e_96af_6212, 0xbcdf_386b_b713_7d3d),
        ],
        last: 0xca74_164d_f9cd_22af,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x272f_b107_73cc_e75a, 0x0c30_d6ab_f069_6e70),
            (3000, 0x499b_9a96_9fdd_beac, 0xdfe5_8957_44e1_caeb),
            (4500, 0xce59_3073_2755_70bd, 0x5f6f_e988_5ebe_7972),
            (6000, 0x1694_d363_c694_d0f5, 0x4a27_3d33_4064_5cd1),
        ],
        last: 0xd39e_20b0_930c_3e4d,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x04fb_3877_b9fd_2911, 0xc8ff_24d6_087c_c221),
            (3000, 0xa20c_09fa_44ba_490f, 0x35d0_6238_2572_983d),
            (4500, 0x1fec_b4f9_e34b_94e8, 0x9ab0_1b39_02bc_eff4),
            (6000, 0xd9d5_37c2_a7f1_b400, 0xa204_357f_c2e2_ad9f),
            (7500, 0xea78_a67d_fa9e_f906, 0xce1a_04e4_5cfa_5500),
            (9000, 0x8313_d3c3_c37a_5bdb, 0x25ed_7759_3633_1915),
        ],
        last: 0x1966_e431_242f_f946,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x8d31_6a0e_a03d_049e, 0xeb01_3179_ff33_70d2),
            (3000, 0xfc72_1d38_7370_a5e9, 0xb3ea_90ea_db88_add3),
            (4500, 0x636b_7bd5_f5bf_19e0, 0xbb4e_8be3_a422_6a1d),
            (6000, 0x3890_d1c8_72bc_475e, 0x12dd_bfc3_e48c_b275),
            (7500, 0x3b2c_d4b8_75fb_c980, 0x514c_cc1b_bdd2_6213),
        ],
        last: 0x8771_e4d8_02d2_5261,
    },
];
