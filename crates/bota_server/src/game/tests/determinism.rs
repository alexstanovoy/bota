//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_core::{ITEMS, map_of, rules};
use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, EventVisibility, Fnv, MatchConfig, MatchSecrets, World};

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
        stream: pinned.seed,
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

/// The orders every seat gives this tick, validated as a driver does, with
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
    if map == bota_core::MAP2_ID {
        bota_core::MAP2_TICK_CAP
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

#[test]
fn the_published_match_id_gives_away_neither_the_seed_nor_the_key() {
    const SEED: u64 = 9_000_001;
    let first = MatchSecrets::seeded(SEED);
    let next = MatchSecrets::seeded(SEED + 1);
    assert_eq!(
        MatchSecrets::seeded(SEED),
        first,
        "a seed plays its match again"
    );
    for (seed, secrets) in [(SEED, first), (SEED + 1, next)] {
        let label = secrets.match_id.to_le_bytes();
        assert_ne!(secrets.match_id, seed);
        // What a client could rebuild from the label: the old scheme's key
        // (the seed's bytes repeated) and the key of the label as a seed.
        let repeated: [u8; 32] = std::array::from_fn(|i| label[i % label.len()]);
        assert_ne!(secrets.master_key, repeated);
        assert_ne!(
            secrets.master_key,
            MatchSecrets::seeded(secrets.match_id).master_key
        );
    }
    let played = |secrets: MatchSecrets| {
        let pinned = PINS.iter().find(|pinned| pinned.map == bota_core::MAP2_ID);
        let mut cfg = config(pinned.expect("a pinned map 2 match"));
        cfg.master_key = secrets.master_key;
        cfg.stream = secrets.stream;
        cfg.match_id = secrets.match_id;
        let mut world = World::for_match(&cfg, cfg.rng());
        while world.tick < CHECKPOINT_TICKS {
            world.advance(&[]);
        }
        world.hash()
    };
    assert_ne!(played(first), played(next), "neighbouring seeds play apart");
}

/// A world cloned mid-match plays on exactly as the original does: a fork for what-if play
/// sees the same match.
#[test]
fn a_cloned_world_plays_on_as_the_original() {
    let pinned = PINS
        .iter()
        .find(|pinned| pinned.map == bota_core::MAP2_ID)
        .expect("a Map2 match is pinned");
    let cfg = config(pinned);
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut drivers: Vec<Driver> = (0..cfg.picks.len() as u64)
        .map(|slot| Driver {
            state: ((pinned.seed << 8) | (slot + 1)).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
        })
        .collect();
    let mut digest = Fnv::new();
    let mut fork = None;
    while world.tick < DEBUG_TICKS && world.victor().is_none() {
        if world.tick == DEBUG_TICKS / 2 {
            fork = Some(world.clone());
        }
        let commands = if world.tick.is_multiple_of(DECISION_TICKS) {
            decide(&world, &mut drivers, &mut digest)
        } else {
            Vec::new()
        };
        let events = world.advance(&commands);
        if let Some(fork) = &mut fork {
            let kinds = |events: &[crate::game::Event]| {
                events
                    .iter()
                    .map(|event| event.kind.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                kinds(&fork.advance(&commands)),
                kinds(&events),
                "tick {}",
                world.tick
            );
            assert_eq!(fork.hash(), world.hash(), "tick {}", world.tick);
        }
    }
    assert!(fork.is_some(), "the match ran past the fork");
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
            (1500, 0x5a1e_5720_012f_d227, 0x541c_3fa9_8b8a_3d26),
            (3000, 0x1cf7_4eb4_09fc_901e, 0x42f2_389f_c208_92f3),
            (4500, 0x5d58_ab16_138e_401f, 0x6e75_ef40_e640_fd3e),
            (6000, 0xc4e1_3e5d_ec9c_ada0, 0x4781_ce05_7f95_dff9),
            (7500, 0x39d4_1827_db7f_daad, 0x7d9d_c331_332e_96c1),
            (9000, 0xc57c_04ce_1492_113f, 0x764a_bbe9_50dd_c55d),
            (10500, 0x2e82_e5fa_26a2_a13f, 0x0193_f41f_0d28_f4c8),
            (12000, 0x5cfc_fbfc_b1fc_8487, 0x00fc_72e5_04e4_1c16),
            (13500, 0x80cc_1f77_edfb_7526, 0xfeab_6f95_8604_a73a),
            (15000, 0x3426_e438_f03d_59fb, 0x04d9_d3c2_40f5_e9d8),
            (16500, 0x94b6_e11e_f4da_8f0e, 0xe35b_8cf5_bdae_48d0),
            (18000, 0x90fb_9f50_d208_9e8d, 0xa935_2b87_efb5_19bd),
        ],
        last: 0xcad7_9b69_fd0c_23ca,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xf504_4936_ca48_67ea, 0x72b4_8f8b_abdb_792e),
            (3000, 0x5035_5105_49d3_e495, 0x0e03_30c8_e7b0_7a21),
            (4500, 0xc253_8edd_327f_1659, 0x2b1b_dc93_1bf9_91d4),
            (6000, 0x42f9_13f1_86af_7534, 0x3f17_d136_d621_ee9e),
            (7500, 0x4f22_32ed_9dab_79fa, 0xc873_2903_d061_9388),
            (9000, 0x9910_a9e6_99ab_3fa8, 0xd407_8f25_e6c2_eede),
            (10500, 0x8850_c4ef_f471_be1f, 0xdf9b_facf_facf_9327),
            (12000, 0xe4df_e840_8cf7_d1d0, 0x06c9_780e_acb7_7ecd),
            (13500, 0xb92b_2904_27af_0ef6, 0xd411_3c48_1d4a_636e),
            (15000, 0xe7d2_54d1_12a6_27a3, 0x7678_f349_1918_5afc),
            (16500, 0x3fbc_2043_9fd4_54ca, 0x7c7c_dddc_ccc9_8bd1),
            (18000, 0x58db_d29b_311b_8f64, 0xc5de_d246_d988_3959),
        ],
        last: 0xbddd_a9f8_aba6_1edb,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd86e_5969_d748_7d3e, 0xf7c6_9dd7_1239_bf40),
            (3000, 0x869e_7808_004a_ad74, 0x155b_378e_ead9_70a1),
            (4500, 0xf141_beed_ab43_3bcf, 0x3f28_14c5_d7fc_0662),
            (6000, 0x11d1_d22a_4e2e_63b3, 0xc85f_5860_0038_23a3),
            (7500, 0xb4f8_a3c6_3948_ab18, 0xcf79_d2ba_3906_22db),
            (9000, 0x384e_fbe0_350d_f92f, 0x3093_b249_c732_57a5),
            (10500, 0x8810_96f3_2a33_f147, 0xc1d3_8cd3_eba3_68a1),
            (12000, 0x0251_8735_5677_3c77, 0x3f09_d8d0_c33f_e537),
            (13500, 0x90d2_8da4_d081_8bc6, 0x8cfe_cbd9_47b5_226b),
            (15000, 0x7f6b_ef03_be0b_ea11, 0x4ee7_ef80_558a_d1ff),
            (16500, 0xd809_027a_e815_2cfc, 0x7519_58b7_5812_2d22),
            (18000, 0xe2f3_3e00_fa16_adf5, 0x7833_9103_7a90_a540),
        ],
        last: 0xb0b4_992b_57f5_6835,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x95f3_28c6_e176_da3f, 0x5602_9e33_74e0_fcc0),
            (3000, 0x164f_3722_11c7_1366, 0x2764_12be_54e8_808f),
            (4500, 0xf80f_baa7_ab7a_666d, 0xa422_c1a9_4a2b_07d5),
            (6000, 0x335d_b200_794b_38fa, 0xa13d_7711_b858_8423),
            (7500, 0xc730_3bc9_4671_f788, 0x076a_15bf_d48d_0524),
            (9000, 0x9878_bfbb_21ca_b8f2, 0xd7c8_9ba2_56a5_c66c),
            (10500, 0x5b26_3dca_a163_e67b, 0x4c34_4ef5_0f57_3a4e),
            (12000, 0x8012_c4cf_4de5_fa1b, 0x552d_9afa_5f8f_dd2d),
            (13500, 0x289b_edda_f2e5_21a2, 0xc50f_27a3_629f_84d0),
            (15000, 0x12b6_29fd_6e0c_32ec, 0x4442_9543_7b98_b757),
            (16500, 0x1157_7f3c_7031_98b6, 0xcb3e_8bde_fc05_f90e),
            (18000, 0xba1d_473a_eca1_1c0d, 0xf124_1618_3d25_8ed8),
        ],
        last: 0x4f76_1fdf_b9bc_2d97,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2b33_5296_008f_1861, 0x61cc_3708_e019_0ef6),
            (3000, 0x3967_f5db_9968_33c4, 0x71dd_fbdd_e395_07e3),
            (4500, 0xc8ae_ca67_8e2a_9d06, 0x6b3a_4686_12e8_2f66),
            (6000, 0x5a47_7582_accd_9283, 0x4f1a_0833_5fda_6e4f),
            (7500, 0x3347_f39a_7cf6_a11e, 0xf3e0_1b86_3539_a7e7),
            (9000, 0x17d4_61b8_8777_2601, 0xba36_a2a8_b5f5_9b95),
        ],
        last: 0xf0ca_10b8_c26a_6bcf,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb11d_254d_019f_bd5a, 0xdc78_50f9_371c_1498),
            (3000, 0xdb50_7f3e_8c31_ec85, 0x4fca_2ca7_e239_794f),
            (4500, 0xe3fa_ad7f_341a_6ddb, 0x8adb_6cd8_55ca_c650),
            (6000, 0x582d_39e2_432b_f1e5, 0xe9fc_0ea9_ce44_2ecf),
            (7500, 0x05a8_c3a0_bcf0_454c, 0x571f_1012_1757_319a),
        ],
        last: 0xdedf_1b0c_69c5_b27e,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xad7d_d755_0977_9af9, 0xfa38_a565_ad32_26a5),
            (3000, 0xaeed_1baf_7da0_7f5b, 0x3a7b_f575_9c6a_80d7),
            (4500, 0xbfb2_867b_2bbb_8299, 0x68f3_3a24_bffb_7026),
            (6000, 0xf0c3_d223_38ba_7b13, 0xd381_f26e_17b4_b937),
            (7500, 0xb9b9_0354_4d70_a78a, 0x483b_fa54_1504_ed29),
        ],
        last: 0x1387_abdf_a96f_6dd2,
    },
];
