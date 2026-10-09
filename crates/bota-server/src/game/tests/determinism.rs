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
            (1500, 0x765e_2bf8_9a20_ba5d, 0x03d1_4a5d_3a2b_9853),
            (3000, 0x4976_643b_fb5d_03e7, 0xa7cf_a96b_14d2_88be),
            (4500, 0x563a_98d8_6858_d498, 0xaf68_9c38_4991_039c),
            (6000, 0x9396_9475_d2e6_0ab1, 0x1314_f5b7_0bf7_ac71),
            (7500, 0xdb9b_013c_5a0b_c1a6, 0x6ead_002f_ea05_8140),
            (9000, 0x1a52_3ff2_7832_0943, 0xcdd8_bbae_7727_6c8a),
            (10500, 0xf0b2_5ab2_6adc_7d0f, 0xa131_c095_8332_fd67),
            (12000, 0x72ab_d6fb_f262_1c2e, 0xa30e_ad72_e1bc_c923),
            (13500, 0xe025_56d9_34d4_8726, 0x1a22_cc15_65a5_996d),
            (15000, 0x97ef_7d62_19d2_7f04, 0xda52_3923_5e9b_9985),
            (16500, 0x9a79_1dee_6608_b288, 0xdb5c_bf7e_605e_74d3),
            (18000, 0xa568_79a2_79eb_34f3, 0x2ac3_dc51_31e5_c81b),
        ],
        last: 0x27db_71c5_9845_08dd,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x8e72_48ad_d844_725b, 0x18e1_66ee_37a2_6248),
            (3000, 0x5818_1775_f94d_1657, 0xafc3_39f4_1569_da44),
            (4500, 0x3481_f64c_c8a0_5300, 0xe444_a7d0_1255_6449),
            (6000, 0x77a8_7f73_bd17_7748, 0xe3f5_b009_2628_c0b0),
            (7500, 0x7502_1dca_f4b3_9357, 0x534c_b45d_0b4f_b2ae),
            (9000, 0x4eca_a103_c2f3_b518, 0x6a1c_b045_e719_fede),
            (10500, 0x9c6f_75fa_f626_db2b, 0x8493_39f1_5fd9_a850),
            (12000, 0x1f7c_e34a_d3c2_6bc4, 0xe5cb_ae66_e223_8b71),
            (13500, 0xaa9a_7e84_9210_824b, 0x0218_7b32_5048_508e),
            (15000, 0xab35_64a6_08ad_c31d, 0x1c40_60fe_df72_f5c0),
            (16500, 0x09aa_7591_54ae_095b, 0x5295_6a9b_3392_af27),
            (18000, 0xef1e_cc40_e205_2528, 0x3ac3_c548_abbc_c71a),
        ],
        last: 0x1637_d0e8_e098_4725,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xbf23_30f0_4cd1_ad24, 0xee51_1bfc_25cc_ccaa),
            (3000, 0x4ae2_fd78_adb7_f956, 0x6e3f_c460_0b09_5aa3),
            (4500, 0x92b5_134b_f84f_d16e, 0xe28f_7ee0_6aee_cfe7),
            (6000, 0x766d_c653_3142_bc3f, 0x3efa_b7ce_c0f5_4cb7),
            (7500, 0xc034_6903_28e7_38dc, 0xf390_6b23_4e05_7945),
            (9000, 0xcf96_591f_92f7_862d, 0x980e_fcb6_5176_0cb1),
            (10500, 0xb2e1_e0c4_8c5f_1e2d, 0x8df6_76b6_0f54_f0bd),
            (12000, 0xcd62_5ff3_a71d_dde7, 0x8455_f55f_0031_4d3b),
            (13500, 0xa217_663f_dfbf_9b7a, 0xc075_53ae_4e0b_6f8d),
            (15000, 0x8711_cb3c_ec6c_c3ec, 0x518e_77d0_e016_23d2),
            (16500, 0x3b48_1d59_1166_6f7e, 0xd565_af01_1532_4eff),
            (18000, 0xae6d_36ea_1dc3_739b, 0xeafd_683f_bda1_198d),
        ],
        last: 0xcdd1_05ac_c9e5_4a15,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x679d_7626_d2e5_9291, 0x98ca_b80b_b21f_ed82),
            (3000, 0x251a_de0f_9f77_19f6, 0x7a84_a5fe_9e01_c4a6),
            (4500, 0xdde9_8792_114b_25f4, 0xad24_8829_6eac_9052),
            (6000, 0xd936_a7e7_032d_f9ef, 0xbb34_e43f_e971_174a),
            (7500, 0x4f90_4965_3373_1c1c, 0xea8f_e5f3_6408_7adf),
            (9000, 0x67de_921d_bf5a_dc3a, 0xa412_88c1_bd15_b58f),
            (10500, 0x7474_10c9_1b58_111d, 0x1dee_eb27_cf6d_d96b),
            (12000, 0x5c09_5235_4e7d_c954, 0x7a39_d2ee_2529_2e7c),
            (13500, 0xde96_8065_6479_0907, 0x82ee_c7e8_b1c2_f6cf),
            (15000, 0xfa5b_cbf9_2185_eea2, 0xcd0b_2bff_0da4_e1d9),
            (16500, 0xbf30_4816_62b7_cdc2, 0x651a_e632_7a6f_9418),
            (18000, 0x0bef_45cd_50ac_06a4, 0xef1e_d80e_a28a_6b77),
        ],
        last: 0x12ac_fc95_54be_435d,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xf707_ad9b_bc68_41e8, 0xa6b0_4924_1c2d_974d),
            (3000, 0x0c1f_6575_e1da_c9bc, 0xc073_1418_a577_ffbb),
            (4500, 0xba1b_60b3_7d53_d452, 0xea73_3a7c_7a6e_aea0),
            (6000, 0x60ef_f05b_9a8d_9e11, 0xcac4_db69_1a3e_1605),
            (7500, 0x85a2_7dfd_87e9_88aa, 0x3bf7_97b3_716f_2cf0),
            (9000, 0xbc20_f5b6_430e_8334, 0xd743_d756_8e18_aaf7),
        ],
        last: 0x4c4a_9bef_c6d3_ed15,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xb059_ccfb_1d64_8405, 0x5624_9551_0111_b5f9),
            (3000, 0x6f39_d6e5_7c8a_4370, 0x4dd2_d581_d1df_21ea),
            (4500, 0xc3ca_2bf9_bc87_2a3b, 0x5de1_82cf_8fe4_6a4a),
            (6000, 0x411c_5cfa_ba4a_2819, 0xca7b_8934_2e50_5935),
            (7500, 0x269b_3169_9ada_4466, 0xadbb_748e_77ad_0676),
        ],
        last: 0x5240_0395_2acb_d429,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xdab4_8a6a_0eda_19f6, 0x71a9_68ae_9b60_90da),
            (3000, 0x6fac_8383_93b8_2ebc, 0x0f22_1141_bc0d_eff4),
            (4500, 0x8a0c_adcc_50d8_fd29, 0x6583_ae9f_9751_4bc3),
            (6000, 0x04ca_64f0_b0e9_87ee, 0xefcc_6314_e8aa_b607),
        ],
        last: 0xdfa4_0ee0_1440_f2a0,
    },
];
