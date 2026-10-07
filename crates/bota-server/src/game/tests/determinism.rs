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
            (1500, 0x765e_2bf8_9a20_ba5d, 0xc10c_36f5_886b_6ae6),
            (3000, 0x4976_643b_fb5d_03e7, 0xf817_9fa9_a456_b1d0),
            (4500, 0x563a_98d8_6858_d498, 0xd0ec_3ccd_e24b_2590),
            (6000, 0x9396_9475_d2e6_0ab1, 0xc963_d366_de3e_0b32),
            (7500, 0xdb9b_013c_5a0b_c1a6, 0x1d0f_7994_8633_556d),
            (9000, 0x1a52_3ff2_7832_0943, 0xfa06_158c_8f00_5c26),
            (10500, 0xf0b2_5ab2_6adc_7d0f, 0xf6bd_d25e_5f8c_029d),
            (12000, 0x72ab_d6fb_f262_1c2e, 0x2831_7ac4_c50c_54b4),
            (13500, 0xe025_56d9_34d4_8726, 0x03b7_31a3_8e4a_a499),
            (15000, 0x97ef_7d62_19d2_7f04, 0x7590_4448_cd99_dc8d),
            (16500, 0x9a79_1dee_6608_b288, 0xce17_7d2d_5709_874d),
            (18000, 0xa568_79a2_79eb_34f3, 0x895b_040a_0a78_ac70),
        ],
        last: 0x24cb_e199_28a8_4d62,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x8e72_48ad_d844_725b, 0xf1d6_3fdb_5707_c664),
            (3000, 0x5818_1775_f94d_1657, 0x0647_303b_d330_e6f7),
            (4500, 0x3481_f64c_c8a0_5300, 0xb963_9f36_edaa_479f),
            (6000, 0x77a8_7f73_bd17_7748, 0x981f_7cb0_1944_e34d),
            (7500, 0x7502_1dca_f4b3_9357, 0xc4b5_0b7f_4ade_89c9),
            (9000, 0x4eca_a103_c2f3_b518, 0x4105_c18a_ad3c_372f),
            (10500, 0x9c6f_75fa_f626_db2b, 0x5519_bcec_15e6_17ff),
            (12000, 0x1f7c_e34a_d3c2_6bc4, 0x0d37_7c87_98d7_aa2a),
            (13500, 0xaa9a_7e84_9210_824b, 0x7751_c755_7176_09cd),
            (15000, 0xab35_64a6_08ad_c31d, 0x4cae_10d9_7468_9971),
            (16500, 0x09aa_7591_54ae_095b, 0x918c_62d7_8739_1240),
            (18000, 0xef1e_cc40_e205_2528, 0x4785_d59c_610f_b94f),
        ],
        last: 0xf6cb_d6f4_ab6c_dde6,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xbf23_30f0_4cd1_ad24, 0x6dfb_4ca0_12ee_df3c),
            (3000, 0x4ae2_fd78_adb7_f956, 0x9bba_5f41_5f9c_8a68),
            (4500, 0x92b5_134b_f84f_d16e, 0x4154_2b50_22d8_ffe0),
            (6000, 0x766d_c653_3142_bc3f, 0x3c8c_92b0_4308_5a64),
            (7500, 0xc034_6903_28e7_38dc, 0x46ec_8807_d37c_3dff),
            (9000, 0xcf96_591f_92f7_862d, 0x18b7_83eb_46f8_6fe1),
            (10500, 0xb2e1_e0c4_8c5f_1e2d, 0x942d_5cc1_8984_b221),
            (12000, 0xcd62_5ff3_a71d_dde7, 0x4c08_0484_0bec_186f),
            (13500, 0xa217_663f_dfbf_9b7a, 0x5672_fca9_73af_83a2),
            (15000, 0x8711_cb3c_ec6c_c3ec, 0x8d6c_12ce_3af0_7119),
            (16500, 0x3b48_1d59_1166_6f7e, 0xda0c_7d3d_1d0a_a5aa),
            (18000, 0xae6d_36ea_1dc3_739b, 0x9801_82a2_3f44_95d0),
        ],
        last: 0x8d08_9f1d_a557_a31e,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x679d_7626_d2e5_9291, 0xaacc_8646_56a2_f9a6),
            (3000, 0x251a_de0f_9f77_19f6, 0x1f16_9eef_aae0_24f1),
            (4500, 0xdde9_8792_114b_25f4, 0x190d_6649_c95c_3715),
            (6000, 0xd936_a7e7_032d_f9ef, 0x0dc9_2a42_7062_9a79),
            (7500, 0x4f90_4965_3373_1c1c, 0xe473_60d1_a253_656a),
            (9000, 0x67de_921d_bf5a_dc3a, 0x0b89_2614_aa3b_d2ea),
            (10500, 0x7474_10c9_1b58_111d, 0x1933_311c_6da0_904f),
            (12000, 0x5c09_5235_4e7d_c954, 0xdc93_e7d1_c62d_0ec5),
            (13500, 0xde96_8065_6479_0907, 0x55eb_952f_4865_b3c6),
            (15000, 0xfa5b_cbf9_2185_eea2, 0x730d_fbd9_8bf9_4085),
            (16500, 0xbf30_4816_62b7_cdc2, 0xb609_f5d1_872c_c2c4),
            (18000, 0x0bef_45cd_50ac_06a4, 0x6ab6_5f7b_82d9_9093),
        ],
        last: 0xe873_c135_e4f1_795a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xf707_ad9b_bc68_41e8, 0x79e2_4390_2315_3944),
            (3000, 0x0c1f_6575_e1da_c9bc, 0x4929_ea28_9fb0_fe08),
            (4500, 0xba1b_60b3_7d53_d452, 0x3c88_43de_87fd_3d42),
            (6000, 0x60ef_f05b_9a8d_9e11, 0xa8fa_7f79_9b2b_df96),
            (7500, 0x85a2_7dfd_87e9_88aa, 0xa9ff_697c_c21c_658a),
            (9000, 0xbc20_f5b6_430e_8334, 0x22e0_7f21_bae5_121b),
        ],
        last: 0x71f8_589a_6c47_2f44,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xb059_ccfb_1d64_8405, 0x6fef_3e17_c98a_70a0),
            (3000, 0x6f39_d6e5_7c8a_4370, 0x187b_354b_bc05_0e13),
            (4500, 0xc3ca_2bf9_bc87_2a3b, 0xc5fd_cd13_5ada_60ab),
            (6000, 0x411c_5cfa_ba4a_2819, 0xbf64_84d1_884b_ea16),
            (7500, 0x269b_3169_9ada_4466, 0x7982_214a_faff_85ab),
        ],
        last: 0x9905_e83c_b477_45b3,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xdab4_8a6a_0eda_19f6, 0x2101_dd71_b8ff_ec5b),
            (3000, 0x6fac_8383_93b8_2ebc, 0xbd8f_a680_7987_312d),
            (4500, 0x8a0c_adcc_50d8_fd29, 0x0cbb_0f4e_ef2c_c44a),
            (6000, 0x04ca_64f0_b0e9_87ee, 0xdb30_91a5_f85b_12a5),
        ],
        last: 0x3a38_1ddb_1329_e923,
    },
];
