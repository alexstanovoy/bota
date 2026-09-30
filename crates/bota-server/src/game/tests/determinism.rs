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
            (1500, 0x73cc_3c6e_af5c_4b9a, 0xf331_f9d5_96d9_767d),
            (3000, 0xe519_05ba_481d_49dc, 0xeaee_139a_933c_1110),
            (4500, 0x79ae_c89a_c4e9_e827, 0x7116_f60e_de84_29f9),
            (6000, 0x2627_0080_22fe_0623, 0x8fdb_c6df_2a0c_3565),
            (7500, 0x4dbc_792f_7a08_c929, 0xf337_6c5c_3d35_845b),
            (9000, 0xda79_771a_aaf6_8a00, 0x5675_95b1_2e81_706d),
            (10500, 0x5bf9_6e97_5d8b_11fc, 0x05ec_4e68_6ac8_f594),
            (12000, 0xc244_9657_0c84_a927, 0xf5cf_7ebe_95ff_b61c),
            (13500, 0xf409_75c4_16c5_c0ea, 0x44ef_bd06_f7c2_fab3),
            (15000, 0x00a0_1f2d_3dc4_08f5, 0x2632_b5c0_71e6_4b93),
            (16500, 0x6ad9_e2e8_82f5_18dc, 0x0c6e_ae80_8419_ae2b),
            (18000, 0x33f9_2f7f_08fc_1559, 0xf6a5_7266_dbb2_a72c),
        ],
        last: 0x9e37_2061_50ec_7b89,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xac2c_8531_0a45_07d1, 0xec34_c019_e78b_2f29),
            (3000, 0x595e_94b1_b18a_1083, 0x5273_ad19_1620_261a),
            (4500, 0xf733_3c90_0ee7_aaa0, 0x2533_e375_2dac_8605),
            (6000, 0x7fda_371e_3854_d996, 0x03f0_5d06_7187_9c4f),
            (7500, 0x2918_c62e_79fb_036c, 0xa096_cfcf_1ce4_097a),
            (9000, 0x2651_4557_b164_f1fa, 0x0924_8196_7938_50fb),
            (10500, 0x5ae3_aa57_9134_c3ba, 0x49d2_6d07_f54a_f5cf),
            (12000, 0x7a26_0142_9d8b_daa1, 0x1db7_d6e9_4ba0_4daa),
            (13500, 0x73bf_c7b5_a648_7c2a, 0xb853_57fa_fa38_bbf1),
            (15000, 0x6abc_7e1d_d2af_a108, 0x4155_fd36_e420_3f98),
            (16500, 0x6ded_1785_daf8_f451, 0x34ed_9e5c_02b8_af9a),
            (18000, 0x3be2_b6de_5c6a_0a86, 0x67a5_34bc_c57b_ac7a),
        ],
        last: 0x696a_ca58_47e3_a147,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xf2c3_0579_1881_f08b, 0x4695_c605_8787_78b7),
            (3000, 0x6524_3497_69f7_8af0, 0x6aa9_6293_54a6_2e39),
            (4500, 0xa644_7629_ee7a_97a2, 0x4c92_5b49_d83b_44fa),
            (6000, 0x9326_49f8_9a37_c7ac, 0x9fc1_649d_6137_b2fb),
            (7500, 0xefe7_9b16_2f8b_7f24, 0xf8a7_0ef5_9d9c_184f),
            (9000, 0xb2bb_8a09_9292_73a9, 0x6615_7db4_d220_293d),
            (10500, 0xf498_19fc_daf3_0fe5, 0x61a6_7fa3_f346_2524),
            (12000, 0xab08_7ede_d8f0_155c, 0xbdf9_45bb_7511_1bb2),
            (13500, 0x4846_e23c_72bb_f0ed, 0xaabe_6ceb_cea9_3dd0),
            (15000, 0x9553_05ad_8e64_ce30, 0x1599_2fd4_49ea_d37c),
            (16500, 0xe674_0fcc_5e05_4063, 0x3c34_e617_050e_a606),
            (18000, 0x6d6d_7b71_6ffe_178c, 0xbab6_0613_d794_9440),
        ],
        last: 0xcdc7_9d36_f3cb_481b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3310_5d12_9cf6_004b, 0x6e13_55ae_afac_d74f),
            (3000, 0xacee_2194_8595_7e0f, 0x9448_8608_4783_dee4),
            (4500, 0xe5f3_2058_0d70_8e30, 0xe8df_3364_4a0f_b95e),
            (6000, 0xe08e_d528_347c_db7a, 0x4a82_02e0_c8dd_8575),
            (7500, 0xe209_d230_f1dd_ca71, 0x9e9c_8b33_6e7c_8f33),
            (9000, 0x3d15_2ee9_1223_72b0, 0xa46a_65df_4307_f061),
            (10500, 0x4484_4985_e426_885e, 0x3207_28dd_27de_15dc),
            (12000, 0x9fdc_08d9_9680_b664, 0x8874_e50e_2eda_3db6),
            (13500, 0xecab_460e_8744_c432, 0x8e84_46d9_bd5d_3ae5),
            (15000, 0x8dfc_dbf4_55de_7b57, 0xf79c_7eb2_0da3_2b95),
            (16500, 0xd456_80ef_8f31_73a6, 0xcda5_aa61_ea43_83b3),
            (18000, 0x9d79_a985_5ccc_0ddb, 0x73a1_697e_b029_ce3b),
        ],
        last: 0x8cf1_8d06_91e5_a889,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x2fae_68e3_728b_a7d9, 0x706b_eaba_33fe_04ad),
            (3000, 0x5502_af73_6b5a_597c, 0x7194_f36b_75a9_4922),
            (4500, 0x0979_d73a_376b_eeea, 0xf3b3_bda0_fc52_4fdf),
            (6000, 0x407d_c454_bb58_2962, 0xdd56_cb1a_8761_0859),
        ],
        last: 0x736d_7ca6_5043_6e28,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x9941_d8dd_1733_f061, 0x31c1_a8f6_0701_a154),
            (3000, 0x9cde_0cf0_fff0_e24a, 0x4bf1_12fd_c0c0_73d6),
            (4500, 0xc80a_eb4a_c047_87e3, 0x7c56_f6c8_eb97_b1de),
            (6000, 0xa263_9287_2ace_8e40, 0x0915_0060_4ddd_c13e),
            (7500, 0x5211_e068_c74a_977b, 0x52ec_1c43_d8ff_6997),
            (9000, 0x04a2_0a75_faa5_40ef, 0x3faa_a34d_279c_b09b),
        ],
        last: 0xf160_5f90_63da_09af,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xfb4a_d503_6818_7715, 0xc26a_1f58_2d35_2fca),
            (3000, 0x4c1c_9bf8_a513_5660, 0x51a3_be67_ef48_bcb6),
            (4500, 0x4def_d504_554c_ddba, 0xc118_d5fe_8c76_a7c5),
            (6000, 0xce5b_8d3c_2f9e_9682, 0x8e8b_1a52_8859_68da),
        ],
        last: 0xae0b_a054_ff18_0e88,
    },
];
