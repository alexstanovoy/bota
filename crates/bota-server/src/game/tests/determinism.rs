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
            (1500, 0x2659_b652_744b_42fd, 0xfec7_d9c1_0931_597c),
            (3000, 0x9f29_1bc9_e7ae_344f, 0xd2c8_e7c0_80dc_0387),
            (4500, 0x4018_f2d2_f92b_6f99, 0xfedf_21da_0779_3b78),
            (6000, 0x3ece_b91b_0091_c6ce, 0x0f9a_8958_1404_07c8),
            (7500, 0xb320_8fbb_1779_1947, 0x7e0d_bb08_a924_06db),
            (9000, 0xfcdc_fec4_d3c2_faa2, 0xbec3_5d3a_84e4_a8b7),
            (10500, 0x4e8a_2c50_8c4a_3bf7, 0xdfbd_aeef_f5d0_f8eb),
            (12000, 0x6b0c_b34b_b61e_ad8b, 0x66aa_8343_a48a_55eb),
            (13500, 0xf52b_dc9b_8e09_8747, 0x5f38_5b8b_fdf1_1258),
            (15000, 0x07d3_acd2_fa7e_ad17, 0x2308_d4c3_2049_675d),
            (16500, 0x1863_8994_aa27_c177, 0x4b2d_454b_b8cd_2938),
            (18000, 0x66af_25dd_e0b5_a87b, 0xbbd4_bac4_481f_ab4c),
        ],
        last: 0x001a_3d84_3b2b_3f3d,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x0fe8_e60e_fc85_ac66, 0xc80b_e03e_efa9_7957),
            (3000, 0x2b35_dc1e_bf41_a5ff, 0x1a3a_8779_5188_5722),
            (4500, 0x1a15_0a5e_6e39_a6f0, 0x98a3_97d8_4840_fc64),
            (6000, 0x6cbd_fdec_d829_4c6d, 0xb8f0_2352_d644_6273),
            (7500, 0xc3d0_bab1_77a5_caec, 0x9a8d_aabe_1e67_74ec),
            (9000, 0xad2a_2977_fa19_97a0, 0x4aec_1889_b9c0_680c),
            (10500, 0x4043_dff1_eb15_8d38, 0x4839_8786_c4b7_fb8d),
            (12000, 0x791f_dd8e_d5e9_da2b, 0xdfd9_d3cb_d47f_36f4),
            (13500, 0x673f_5d09_78b8_6e1c, 0xc73f_33bf_be6f_8db3),
            (15000, 0xa1da_6ecf_25ad_325b, 0x9aec_f40b_755e_1999),
            (16500, 0x0c65_85d4_24f4_40ff, 0xa036_b5b2_41db_4d2a),
            (18000, 0x6233_ad1e_6aa8_093c, 0x1c1d_1d75_4f45_920a),
        ],
        last: 0xf25e_2f1c_a7f6_a653,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x710f_5b84_aa60_306a, 0x107e_5f1a_0489_1664),
            (3000, 0x1049_df59_cb3b_73d3, 0xa574_cdfd_b10f_87eb),
            (4500, 0xfe01_b972_50a7_25d2, 0x5af2_f034_d6d5_4a95),
            (6000, 0xaf00_afb0_f2c3_3be3, 0x3dbf_dae4_7e1a_786f),
            (7500, 0x7cdd_9890_78d5_8618, 0x8a30_62fa_6746_8c09),
            (9000, 0x38b9_1899_e095_79af, 0xfd8e_1d79_0390_c2b2),
            (10500, 0xc3e3_b609_dec6_ed87, 0x7e02_4dcd_1c6b_e9c8),
            (12000, 0x874a_657d_f9da_5ac6, 0x8159_78b1_048c_3028),
            (13500, 0x1d8e_23c5_e01d_2a33, 0x615b_52ba_4753_afb4),
            (15000, 0x6d09_2b8c_e28f_6d94, 0x999d_3bcc_ec6c_4bbf),
            (16500, 0xb980_5ddf_e56c_662d, 0x97ad_f6aa_5362_6a19),
            (18000, 0x6b94_184a_3fb4_bed2, 0x36fc_fd47_fbaa_2373),
        ],
        last: 0xd703_f311_63cf_4aab,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x53d0_391d_0063_d5e1, 0x818e_9a79_a08b_31f7),
            (3000, 0xb17b_d901_5c95_b093, 0x245e_7131_5a3c_ede1),
            (4500, 0x8e24_2f20_9095_e833, 0x1d04_2530_dba7_f8bc),
            (6000, 0xd1a5_f069_35f6_fa0a, 0xba6f_9f34_dc15_ceb4),
            (7500, 0xe2f8_8a01_a4b8_4d8b, 0x2501_4288_afe8_b85e),
            (9000, 0xfa30_c453_658f_90a1, 0xe6ee_b5af_7683_2b8c),
            (10500, 0xf881_b265_29d6_4de9, 0x7230_33ad_013f_04ac),
            (12000, 0x214e_b7de_0494_f589, 0xfa02_a57a_e9aa_a7b9),
            (13500, 0xd13d_d8c3_0d5f_e068, 0xc39a_11cb_b527_9726),
            (15000, 0x9436_6fec_40b4_6c14, 0x4229_21fe_cc6c_713a),
            (16500, 0xe2d0_0f55_6fab_0603, 0xddc7_7b2d_c4f8_94fe),
            (18000, 0x8380_8ba3_db6c_20de, 0xcb57_62f3_7f29_9705),
        ],
        last: 0xc214_6099_d3ab_57e9,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xf2b4_3e03_421b_d8ba, 0x258e_14a6_47d5_ae9b),
            (3000, 0x6e6b_6522_c95b_6110, 0x9c6c_bc92_cbf2_3594),
            (4500, 0xa1c8_d16f_572c_93b9, 0x5b9d_607e_721a_ca50),
            (6000, 0x0001_a919_56de_34a5, 0x0999_11c4_7f7a_18e2),
        ],
        last: 0x324a_86ac_8ac4_d565,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x447d_365a_bac8_9ea9, 0xd828_ab1e_b738_698b),
            (3000, 0xb553_31b2_7979_dd8b, 0x9ef6_3fb4_dfdd_e06b),
            (4500, 0xacaa_779d_cc07_b8e6, 0x7bed_3aa9_3cf2_2b67),
            (6000, 0xa334_dc78_ce38_ccb4, 0xcdc2_59fc_0b66_80be),
            (7500, 0x368f_b0cd_682d_35aa, 0x3381_f90e_545c_41bb),
            (9000, 0x18c6_6d0a_4ed3_048f, 0x6de1_a5ba_695c_98f2),
        ],
        last: 0xbe8d_0286_eacc_3ab8,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x51a1_a753_42e5_7962, 0x3e87_ebd4_cdeb_9a26),
            (3000, 0xe9f6_f6b4_e894_8fd9, 0xc055_36ba_a846_afb9),
            (4500, 0xf5b8_cca3_0ca0_170a, 0x4dd5_451c_de90_34cb),
            (6000, 0x6326_c196_8d41_da52, 0x6647_6df6_97bb_f1ae),
            (7500, 0x63de_aaa6_6613_a9a0, 0x296e_d840_0666_a2ee),
        ],
        last: 0xe011_f031_0ab4_2408,
    },
];
