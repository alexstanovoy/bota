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
            10 => match self.below(3) {
                0 => Order::Learn {
                    slot: AbilitySlot(self.below(me.abilities.len() as u64) as u8),
                },
                1 => Order::LearnTalent {
                    talent: TalentSlot(self.below(u64::from(TalentSlot::COUNT)) as u8),
                },
                _ => Order::LearnAttributeBonus,
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
        starting_gold_lost: Vec::new(),
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
                queue: false,
                seq: 0,
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
            (1500, 0x59fc_5a5e_61aa_2477, 0xfa62_a1b0_1c54_e8ea),
            (3000, 0x8202_d718_e186_db13, 0x8ab6_7f91_9414_7368),
            (4500, 0x0b79_9bc0_5010_1fa3, 0xb220_8351_cfe0_6b3a),
            (6000, 0x5843_adf4_80b9_19e1, 0x7ea5_32c7_a4f6_bf7c),
            (7500, 0xd14b_8a01_6fa8_3d27, 0x20fe_e2e0_b1ac_d4a8),
            (9000, 0xa715_1295_72ec_9e1c, 0x159a_f800_5c44_4957),
            (10500, 0x704b_e0ef_84c9_ed6f, 0x28ba_c584_0b35_85c2),
            (12000, 0x9b9b_e690_419f_0ef2, 0xcd5f_8de0_2ef2_82ff),
            (13500, 0x0c02_84d7_8b09_fb39, 0x8c4d_eae4_6547_3c5a),
            (15000, 0x0b1f_3e2f_9432_2acb, 0x1c41_24f8_0ac8_f326),
            (16500, 0x5db6_8a89_c521_5b44, 0x3026_18b1_1f1e_91be),
            (18000, 0x45cb_4b09_1dc8_8b7b, 0xe817_b857_4e46_3a33),
        ],
        last: 0xb9c7_f1fa_e6e9_7622,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x2daf_d9b0_bb6f_8159, 0x4d04_5ddf_9484_ca5f),
            (3000, 0x02d5_7a89_284d_8282, 0x6b87_19d6_eb9e_fac0),
            (4500, 0x1bb5_9eb4_f2dc_8959, 0x73d7_64b6_ff40_2833),
            (6000, 0xa8d4_6120_8681_88e1, 0x5619_654d_085f_b2c3),
            (7500, 0x2261_a09b_226d_dc64, 0x3a0f_1b19_8fd0_53f7),
            (9000, 0x2971_c837_1420_75b4, 0x371b_4b6b_4c5e_0058),
            (10500, 0x59ed_7f76_b443_737d, 0xce06_98b1_6df6_f404),
            (12000, 0x618a_3ee2_b50f_1c50, 0x396d_f5c1_2326_a923),
            (13500, 0x05cf_6b1a_b2c2_773d, 0x2908_c4d7_a3c9_d544),
            (15000, 0xad4b_48fe_71b7_47cf, 0xf909_ad61_ae6a_9228),
            (16500, 0x485e_09e1_bc86_9f27, 0x2f3f_8ea6_225c_0851),
            (18000, 0xb71c_04ff_3b89_694b, 0xfcc8_89e2_99db_6730),
        ],
        last: 0x7318_282c_6617_56d9,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xb971_08cd_f7be_7d30, 0xb3ca_a2f1_0489_3cff),
            (3000, 0x3cf7_0ec0_a123_a8c8, 0x8b70_f7c2_479f_f4ee),
            (4500, 0x87a9_8e97_c598_ad7e, 0xfedf_27f8_57c8_4b7c),
            (6000, 0xcdd8_bccb_f40d_ee8d, 0x1ce1_d0ea_50b9_5feb),
            (7500, 0x6e8d_6149_a14c_3060, 0xae9f_e75f_aeb0_4651),
            (9000, 0xc202_df9b_bdda_9161, 0xc75b_ff4d_d731_4d56),
            (10500, 0x79ee_b799_9ff1_bdc3, 0x5276_1e73_ab50_6d6d),
            (12000, 0x20a1_033a_c54e_b589, 0xeed0_c633_9fab_a3e5),
            (13500, 0x9304_65a6_ac92_1454, 0x0543_eedc_d70c_b389),
            (15000, 0x8cb9_641a_e91c_4e23, 0xa9d8_8935_3374_5146),
            (16500, 0x0497_1c09_3217_ba65, 0x14ee_d12a_fe31_7a7a),
            (18000, 0x0af7_05b5_ba81_80cc, 0x27b1_7806_b423_ef33),
        ],
        last: 0xc2fd_d882_d4b7_5d79,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2458_ce14_680f_6edc, 0x2e50_18df_2204_2d6b),
            (3000, 0xdc2f_d85f_ff7b_61e1, 0xbe3a_5492_b79a_ec4f),
            (4500, 0x3716_ddb1_41bb_03cb, 0x5f13_ac93_f0fb_7435),
            (6000, 0xa70e_c7a7_dd29_b7ee, 0xe37d_d606_4da5_dbe9),
            (7500, 0x815c_7b9a_02e9_639b, 0x1c68_b330_973c_9262),
            (9000, 0x7e11_fb5a_e028_da44, 0x5f34_229c_d2d9_2228),
            (10500, 0xb2ac_b9ad_6513_9a65, 0x98ab_3729_49b9_1b28),
            (12000, 0x8db7_1ea5_d6e3_b96a, 0x2b10_4a44_3f80_3f65),
            (13500, 0xdfa9_a06f_bbf9_e998, 0x9538_2dc4_03d7_5ced),
            (15000, 0x5cf8_d35e_f07a_a95a, 0xa752_84d6_89c5_ce04),
            (16500, 0x6f17_e4df_b016_b4d7, 0x483e_17b3_d00b_ac23),
            (18000, 0x7e7f_5cfa_ee18_9bef, 0xe99e_8956_738d_fc9b),
        ],
        last: 0x7201_9b85_d379_ff07,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4daf_a205_2372_23e1, 0x009a_759e_2da6_1c69),
            (3000, 0x74d1_8c6c_9e86_ea17, 0x4653_eaf9_8f26_9488),
            (4500, 0x56dc_110d_4d44_9bad, 0x6c24_0b29_88d0_eb8f),
            (6000, 0x91f5_3f16_8a9f_c0ff, 0x8329_6c70_2c1e_4074),
            (7500, 0x938b_4867_4f05_fb4d, 0xb7a5_9af1_e416_4df9),
        ],
        last: 0x88ed_3f5a_1bda_6436,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x605a_341c_8560_1123, 0x34b3_f1ae_d4e5_ceb7),
            (3000, 0x0362_a2e6_01d8_07bb, 0xa33d_191d_05d1_faee),
            (4500, 0x01a5_0f20_00f5_457b, 0x4784_3525_cc05_e51f),
            (6000, 0x01a1_04dd_95eb_46de, 0xb008_3ede_341e_5d95),
            (7500, 0xf1b1_630b_3fb7_73a8, 0xab9c_7b07_3013_d7d8),
        ],
        last: 0x49e4_7a86_5544_fa09,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x8e32_751b_b2c7_e8c2, 0x4cec_7881_ba1a_0e80),
            (3000, 0xe484_8d49_e209_59e7, 0x20e0_75bd_8a1c_0194),
            (4500, 0xcd15_6dca_a363_9320, 0xb836_dcd6_aa8e_246c),
            (6000, 0x2226_cb8f_f34d_57f8, 0x4684_07fd_2c30_2ae0),
            (7500, 0x6152_57a5_c1fb_e5e8, 0x3e92_3bff_8938_cde3),
            (9000, 0x2dee_b35c_902b_23e2, 0x74d6_a357_377e_14ec),
        ],
        last: 0x4ad8_48c3_0798_aab6,
    },
];
