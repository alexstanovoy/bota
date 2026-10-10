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
            (1500, 0x4525_5bc0_5572_58aa, 0x4e9f_bad6_60f8_1695),
            (3000, 0xd3aa_d973_7970_979e, 0x8a15_5a48_9cd6_422b),
            (4500, 0xe6ef_99c5_6736_8d79, 0x5285_53cc_293a_6306),
            (6000, 0x42cf_4dc4_c4a1_a8d5, 0x5a71_4f3a_9c90_1c8b),
            (7500, 0x6031_b2d1_07a1_5389, 0x6256_959a_f741_9640),
            (9000, 0xbf01_10e3_82d3_1a00, 0x7cd7_af3e_e442_7837),
            (10500, 0x4df2_3bde_2d23_980a, 0xb207_1b97_f1df_db2d),
            (12000, 0x0814_7e7c_6926_21ef, 0xa369_ce5a_1631_d6eb),
            (13500, 0x4771_d2e8_fbed_1e15, 0xdbae_9f56_4b44_8caa),
            (15000, 0x46f0_02a2_7dce_69e0, 0xa363_c1b8_164f_a9d2),
            (16500, 0x0629_7b26_2619_b149, 0x8dfd_751f_fd61_bd4d),
            (18000, 0x488d_e9f7_bb96_8660, 0xf16c_b2e4_1958_d1e2),
        ],
        last: 0x0da0_2f13_3222_c765,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7fee_0eb1_fae6_1c8e, 0xc139_2a97_e1ac_5c70),
            (3000, 0x6030_3e07_6b4e_c455, 0xa4eb_f952_ea32_d56b),
            (4500, 0x412e_069e_0149_1bcc, 0x19a8_2d50_4756_131d),
            (6000, 0xfe97_15c2_c615_2cae, 0xea4e_5901_8c5c_d562),
            (7500, 0xa9d1_3724_1f0c_6b21, 0xf96a_9803_92be_a1f3),
            (9000, 0x8afe_6740_49d9_e912, 0xe408_2759_116e_c932),
            (10500, 0x23c1_3540_7921_aab1, 0x18bf_c6f1_aa6e_3215),
            (12000, 0x1ac5_d1b8_1194_eeb2, 0x2820_cd51_5be9_acd6),
            (13500, 0x9182_f2d6_1d93_f84e, 0x4230_6623_ba9c_d9cb),
            (15000, 0x8432_6fc7_312b_3b90, 0x0101_534c_6dc4_4003),
            (16500, 0x9345_8ec2_3325_9e89, 0xccc4_ef1d_06de_edf0),
            (18000, 0x7d07_a8cd_121d_4caa, 0x726d_0126_2b67_9591),
        ],
        last: 0x7181_479f_6dda_d9d7,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xa27d_00cf_56eb_53c6, 0xb3fd_d4ae_74c7_327d),
            (3000, 0x0b16_0799_b611_4e0e, 0x39ea_766e_3ce1_c054),
            (4500, 0xfd2b_720c_a2ab_1b04, 0xdcf0_50c6_4e92_29f7),
            (6000, 0x5f91_1b98_999a_4e17, 0x9ac2_5105_fa2a_f1db),
            (7500, 0xb145_fc94_40f2_f219, 0x5984_a08d_8732_2bc9),
            (9000, 0xcff9_8fdf_1123_4691, 0xc0d4_39f6_2d1f_fa29),
            (10500, 0x7c9f_daaf_6710_2c88, 0x156b_79a7_7507_d77e),
            (12000, 0xfe2b_7876_b246_6178, 0x166c_deab_23e7_f257),
            (13500, 0x2afd_1f4d_cdd1_4991, 0xeda0_8085_b37c_1ef6),
            (15000, 0xbf48_0c5d_32c6_ef74, 0xb086_2dc5_f8d1_c78a),
            (16500, 0xdcee_91a8_08d7_d31d, 0x5349_be23_314f_7d5f),
            (18000, 0x063b_3008_850a_9497, 0xff3b_86b6_7290_0901),
        ],
        last: 0x506a_1fd3_e356_10b4,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x4084_9959_e82c_ea07, 0xbdeb_7a38_64c0_c531),
            (3000, 0x0238_5580_9fa1_7f55, 0x6a39_c9ae_94d0_498f),
            (4500, 0xe47a_8451_96eb_1eea, 0x3cd3_dede_46de_c53f),
            (6000, 0x5577_072a_e96c_6080, 0xd93d_fc5d_ed8a_40c3),
            (7500, 0xc21f_952f_767c_c3ce, 0x7958_6b5c_ff44_fdcd),
            (9000, 0xc094_3253_b519_6d8f, 0x7fc8_3c95_d9cf_a473),
            (10500, 0x0f79_d2d9_8f68_e5d5, 0x8e13_6e0e_1bdf_45b7),
            (12000, 0x26a2_8385_aa51_9f32, 0x8927_fae7_574a_ac0c),
            (13500, 0xf938_3008_af72_1a35, 0xd156_bb24_d883_2e9e),
            (15000, 0xa665_dff5_01c4_9d06, 0x5082_d848_98bf_47f7),
            (16500, 0x0a88_85cf_675c_ba50, 0x7bb4_570b_729a_af55),
            (18000, 0x7987_7b9b_2243_b12c, 0xf398_6999_705e_5a92),
        ],
        last: 0xf268_1be7_5cbc_6c61,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5b27_a926_66d7_d15a, 0xb4db_08ca_0999_6544),
            (3000, 0x145c_665e_238b_de1c, 0xf0b2_5654_2d28_7d8d),
            (4500, 0x3f40_6c1b_97d9_1c7d, 0x34b0_8be7_1c4e_6384),
            (6000, 0xe38b_3f1a_0799_51c9, 0x4dad_a763_847e_f0a3),
            (7500, 0xe813_4906_ccd2_2711, 0x2f60_ec8d_ac9c_0182),
        ],
        last: 0x7de2_d704_83a0_5c1e,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd46f_e640_f596_8a2f, 0xcf49_688f_dd1d_9a31),
            (3000, 0x0435_2049_7c75_6589, 0xc374_e631_d2ec_f178),
            (4500, 0x5652_b445_17ce_415d, 0xfdcf_45ab_0d91_a40f),
            (6000, 0xd978_308b_0659_1fd6, 0x18e8_ea10_e239_adc8),
            (7500, 0x72fa_0425_88e1_f048, 0xc970_896a_96a9_6b96),
            (9000, 0x8e2b_4da0_a335_6fcc, 0xd175_b97b_6e3b_40d7),
        ],
        last: 0xda4b_3217_2f91_4d7d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2e99_0a95_838c_7f61, 0xa0f3_8800_3591_24e6),
            (3000, 0x371c_3808_3832_ef74, 0xf910_04d1_a671_5bfa),
            (4500, 0x3fc1_fe99_2065_15dd, 0xe690_4971_4c7d_f0c1),
            (6000, 0x91b9_badd_25ce_9e9b, 0x5dd6_bf67_c077_1795),
        ],
        last: 0xfe63_c1da_bb83_b80c,
    },
];
