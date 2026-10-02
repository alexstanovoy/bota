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
            (1500, 0x73cc_3c6e_af5c_4b9a, 0xb68c_b176_6840_4f21),
            (3000, 0xe519_05ba_481d_49dc, 0x34b9_6160_e558_7bc4),
            (4500, 0x79ae_c89a_c4e9_e827, 0xcdb3_7b80_63b7_bc14),
            (6000, 0x2627_0080_22fe_0623, 0xce25_e3fd_6c80_54ce),
            (7500, 0x4a15_ccf0_0d70_1a23, 0x225b_c871_872a_4174),
            (9000, 0xdcff_0832_b427_831b, 0x71f3_1969_efd2_cff9),
            (10500, 0xe7d7_466e_9c71_0063, 0x80f8_dd1a_7d8b_e727),
            (12000, 0x01f3_9ea8_ab81_e888, 0x026d_b467_e1ba_658a),
            (13500, 0xe339_5fb3_8faa_dbb3, 0x8257_8e86_32d9_cbd3),
            (15000, 0x0128_1459_16ce_0540, 0xb097_f757_de45_1a82),
            (16500, 0x3c83_0d69_0a04_b161, 0xa228_b23f_be79_3b42),
            (18000, 0xe949_e760_e2c0_360e, 0x8e55_53aa_ed3f_b206),
        ],
        last: 0x7916_e778_4fe8_15a9,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xac2c_8531_0a45_07d1, 0x533a_3ddb_abcb_8c48),
            (3000, 0x595e_94b1_b18a_1083, 0xa3c7_54a4_2355_0eb3),
            (4500, 0xf733_3c90_0ee7_aaa0, 0xc614_5d2f_f206_d142),
            (6000, 0x7fda_371e_3854_d996, 0xee9c_6258_0ce0_a251),
            (7500, 0x2918_c62e_79fb_036c, 0x0ca1_de78_54a3_919c),
            (9000, 0x2651_4557_b164_f1fa, 0xac5f_6f8d_f76b_510b),
            (10500, 0x8bf9_c37e_aa25_25f9, 0x98af_28f4_28e9_cd20),
            (12000, 0x7abb_c90a_142b_1b07, 0x63ce_7b39_4d0c_70e0),
            (13500, 0xcf66_b4d2_7226_8b8f, 0x73e4_3f39_1a4a_b8d3),
            (15000, 0xb841_1ee7_e8aa_0d78, 0x4e7a_df34_2975_ee7f),
            (16500, 0x5393_fb78_8cab_53cb, 0x1592_cb57_2974_5235),
            (18000, 0xcb31_fb5a_4e7e_8c26, 0x6fc0_739a_9176_2a1d),
        ],
        last: 0xbc02_3e64_fdcf_2c4d,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x9d7b_32d8_7649_8521, 0x54e6_a261_9b39_a3c1),
            (3000, 0x94cd_e0f9_5ab0_38fc, 0x2090_a596_5b00_f203),
            (4500, 0x9252_2975_36c4_ff7c, 0xaf23_fabe_be54_56a2),
            (6000, 0x2962_2b6a_a196_f712, 0xdba5_e6d5_68b4_6cda),
            (7500, 0x6e72_2c12_347b_5486, 0x2d02_820c_e922_d722),
            (9000, 0x5f38_0e0c_47e2_bb22, 0x1a84_7b72_b26e_dc29),
            (10500, 0x2dc8_3efe_72d2_3844, 0x8c60_2dd4_e1f2_f61e),
            (12000, 0xd013_f8b8_6b1f_2613, 0x203e_d571_a991_d9d9),
            (13500, 0x726e_d82a_a488_0cc1, 0x3501_2004_acc3_fb52),
            (15000, 0x2a00_63b3_4d6b_1578, 0x145b_8a44_6377_db39),
            (16500, 0xeb4a_5b0a_2f1d_15a0, 0x485e_4637_b679_5dcc),
            (18000, 0x4bb9_5349_04a4_1314, 0xe512_b2e9_9c65_6a43),
        ],
        last: 0xac96_5b51_ad2a_e04a,
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
            (12000, 0x5c09_5235_4e7d_c954, 0x6144_f7ec_2266_2f15),
            (13500, 0xde96_8065_6479_0907, 0x1a3c_1ffa_51b9_5b5e),
            (15000, 0xfa5b_cbf9_2185_eea2, 0xb465_d917_9706_5095),
            (16500, 0x201c_14cc_fccc_70bd, 0xe525_67f4_4a7a_5923),
            (18000, 0x0bef_45cd_50ac_06a4, 0xb1b9_cbed_f635_2e32),
        ],
        last: 0xb924_9aec_69b4_87d7,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x2fae_68e3_728b_a7d9, 0x7350_ca6e_a929_2a67),
            (3000, 0x5502_af73_6b5a_597c, 0xce7b_6b31_a4df_0646),
            (4500, 0x0979_d73a_376b_eeea, 0xdd0d_1e04_f54f_cb2a),
            (6000, 0x407d_c454_bb58_2962, 0x4ddd_78ee_d771_a6a2),
        ],
        last: 0x2dcf_5b38_96f3_b3fe,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x9941_d8dd_1733_f061, 0x4b20_b560_70cf_38eb),
            (3000, 0x9cde_0cf0_fff0_e24a, 0xc02c_fce4_1d41_b64e),
            (4500, 0xc80a_eb4a_c047_87e3, 0x3ebd_4e37_c741_013b),
            (6000, 0xa263_9287_2ace_8e40, 0xf9af_1ead_433e_b551),
            (7500, 0x5211_e068_c74a_977b, 0x8859_5ccf_f14d_7fdc),
            (9000, 0x04a2_0a75_faa5_40ef, 0x4f85_72d0_22d5_9127),
        ],
        last: 0x6e5a_5463_5eda_731e,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfb4a_d503_6818_7715, 0xb8c3_99ed_ca43_7188),
            (3000, 0x4c1c_9bf8_a513_5660, 0xd668_5ce8_7ede_92ce),
            (4500, 0x4def_d504_554c_ddba, 0x4045_f536_a352_aec6),
            (6000, 0xce5b_8d3c_2f9e_9682, 0x02a3_d2e1_8e53_728a),
        ],
        last: 0xdab3_ddeb_d800_2580,
    },
];
