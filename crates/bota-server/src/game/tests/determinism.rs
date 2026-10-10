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
            (1500, 0x2f94_8303_44cf_ed6d, 0xcf6f_7697_5ed0_e9c7),
            (3000, 0xec34_6857_f992_ad28, 0x08a5_929a_1d68_d1c1),
            (4500, 0xf463_bb9f_94af_82bc, 0xbfde_69da_40f5_8a26),
            (6000, 0x6648_f2b0_ac95_06d8, 0xf75c_f28d_f701_fa8f),
            (7500, 0x3927_7796_dbf7_fb8d, 0x156d_c743_ae04_a1f3),
            (9000, 0xc968_21f0_515c_92dd, 0x0144_4437_33fc_f476),
            (10500, 0xb0d5_c4aa_2453_2275, 0x5bca_b270_d6ae_123c),
            (12000, 0xbecc_7deb_d6e3_3c2c, 0xd791_8c5d_3c17_c8f3),
            (13500, 0xf201_6b05_f977_4990, 0x9f4a_4d81_3e0d_690b),
            (15000, 0x9dfa_d948_1105_05c6, 0x36d0_bfc3_b9ad_dea2),
            (16500, 0x9441_c338_2d3d_ccea, 0xd4ad_41d3_916b_49d0),
            (18000, 0x2df4_3c58_3e30_c3c0, 0xaed7_d61a_69e5_86dd),
        ],
        last: 0xbe1d_8bc7_fb64_623d,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xf9ec_33b7_88d4_eabe, 0x5145_803d_cd3c_1556),
            (3000, 0xf1dd_463e_fab3_9263, 0xcc7a_93e4_edac_626c),
            (4500, 0xef02_b04d_9e42_868c, 0x0817_d5d0_6563_4831),
            (6000, 0x72fe_b65f_4a5b_b88c, 0x2be4_ffc5_706d_2a5c),
            (7500, 0xc0a2_4d66_a09f_40a2, 0x50ec_3847_8a78_2fd7),
            (9000, 0x2a33_d8f1_3454_6bd7, 0xbe45_a3ae_ae74_4f4b),
            (10500, 0x0493_5c6a_a702_a48b, 0xc02e_fa08_eb8e_b242),
            (12000, 0x209a_0b4b_005a_a8ce, 0x5120_3776_d6d2_df50),
            (13500, 0x3a39_a1b8_5799_a49c, 0xe1b9_b78b_fa6d_27aa),
            (15000, 0xc66b_6653_becf_9a1d, 0xac16_c483_bdba_6201),
            (16500, 0x7759_a0ee_3e23_2d59, 0xaf66_2344_6478_4e17),
            (18000, 0xbef1_d2c1_44b7_b62f, 0xee3e_0e1e_1b72_4859),
        ],
        last: 0x6de6_948d_b252_257e,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xa273_7608_ee76_6b98, 0xc998_946b_def0_e8fd),
            (3000, 0xecdb_fd49_9dfd_9569, 0xb9be_2858_6287_1f83),
            (4500, 0xaf78_a4db_a780_15e5, 0xf651_6c0b_75de_3e9e),
            (6000, 0x54cd_c014_170b_0297, 0xcae5_e634_a5b6_625c),
            (7500, 0xcdd5_b944_ae91_9f34, 0xb0a3_0a53_876c_0290),
            (9000, 0x0558_f3d8_c4f9_2e02, 0x471b_b6e0_e3d4_544a),
            (10500, 0xc8bf_657d_5db3_ea1e, 0x7e28_265f_854c_9d6e),
            (12000, 0xbb4f_b283_c9ce_5f66, 0xa84e_42cf_fd86_9cc4),
            (13500, 0xd9e5_e47b_736a_51ef, 0xdde6_acde_ec64_0eb9),
            (15000, 0xd830_9fba_5de4_d1d8, 0x1d06_fafc_0d83_fbe3),
            (16500, 0x194f_8e92_26a4_45f1, 0x9bc5_2108_8ea8_21a0),
            (18000, 0xef0e_a749_cf0d_8833, 0x2504_15c8_40e4_4964),
        ],
        last: 0x2a61_bb5d_2c53_6755,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x992f_7360_a789_8947, 0x2188_2e7a_a37e_964f),
            (3000, 0x9a84_c4fd_22c8_42bf, 0x1ca3_30ad_b053_05c0),
            (4500, 0x1c7d_71a6_757e_aa40, 0xf8b5_c6bf_34b8_375e),
            (6000, 0x94a1_a879_5b41_6f4b, 0xca2b_b722_c07d_7588),
            (7500, 0x856e_9bf7_87db_7493, 0x67fe_0de5_b7f7_0092),
            (9000, 0x8774_edb8_7583_ea51, 0xc0a0_dc57_812a_47f2),
            (10500, 0x781e_ae69_975c_9b21, 0xff0a_5d1f_d2e1_c981),
            (12000, 0xc01f_80cb_53b6_ff5f, 0x8f5d_979d_e302_bfa9),
            (13500, 0x4519_febb_62b5_229c, 0x6088_aa89_f105_ab6b),
            (15000, 0x2b3d_14b1_461a_7d5f, 0xf6dc_cdfc_1c37_3c1b),
            (16500, 0x8a03_6d4f_58c9_f192, 0x565a_6a5d_8d4c_c3ef),
            (18000, 0xed8c_2b81_6931_2901, 0x2c3b_469f_5a1d_75a6),
        ],
        last: 0xd334_e579_9e64_8a2a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x99ac_0700_fa87_1222, 0x1723_75ae_ad0e_8b98),
            (3000, 0x9cec_b674_d866_ee56, 0x35d2_83b7_ffe1_8634),
            (4500, 0x3287_5903_391a_1892, 0x283c_10a1_70a3_4d9f),
            (6000, 0x7730_0d6d_990c_8c32, 0x9f49_d978_c01b_c285),
        ],
        last: 0xfdb0_f2ed_2dcf_8437,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x1740_642c_c316_9fba, 0x9dd3_de42_9896_8eeb),
            (3000, 0x49f5_59b5_503f_2418, 0xc47a_29a6_4b1a_c1bf),
            (4500, 0x9006_5202_ee77_94ff, 0x564f_eb30_29c6_8e31),
            (6000, 0x9097_df60_8acf_0e55, 0xb9ef_90c9_3925_0b40),
            (7500, 0xa4ba_c761_336d_745b, 0x0a4c_3438_c72d_27fe),
            (9000, 0x76a8_de63_ed53_7f40, 0x51ce_1e63_89fe_1b13),
        ],
        last: 0xbdba_71d0_c868_b57d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x383a_9b34_900a_b6d0, 0x6a89_6649_971c_ff30),
            (3000, 0xa753_07a4_42cb_2a6e, 0xe109_cdb4_3b54_fffe),
            (4500, 0xed56_39ae_8b2a_f28c, 0xec4c_c13c_ac9f_db37),
            (6000, 0xa91c_6df3_a0db_2dcf, 0x96cd_343c_0726_0be4),
        ],
        last: 0x6f41_f4e1_743a_f1a2,
    },
];
