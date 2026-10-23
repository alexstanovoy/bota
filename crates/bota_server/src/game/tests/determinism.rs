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
            (1500, 0xf27f_1eb9_05a9_9e15, 0xac14_8d59_cbf3_239d),
            (3000, 0x5083_90e6_300b_b524, 0x820b_a1ae_75e6_8dc6),
            (4500, 0x513f_bd8e_8d77_7855, 0x8c24_06aa_1311_c818),
            (6000, 0xb31a_dddd_19de_e5e6, 0x1d44_77e1_f7cd_8a4f),
            (7500, 0x1e78_c262_64ed_5054, 0xf6da_7908_e3ed_99bf),
            (9000, 0xcf54_8c57_d582_24bd, 0xe994_a7d6_5d5a_dbc8),
            (10500, 0xc1fa_64bb_c603_c68f, 0xa3e9_bdba_3ac4_0fa4),
            (12000, 0x7bd6_d528_1ae5_7cb1, 0xddd8_3ae9_4da0_053b),
            (13500, 0xbe07_bf15_f2d1_62d4, 0x95ea_d002_77a9_2762),
            (15000, 0x930a_650d_bf5e_254a, 0x1281_e0e2_adc7_4eeb),
            (16500, 0xf928_88d9_c933_9a97, 0x4acc_e62d_87aa_36ba),
            (18000, 0xf937_9860_6601_75b3, 0x02d1_0f76_5344_bd61),
        ],
        last: 0x4874_dd77_b553_d280,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x5964_4352_8507_fc46, 0x5646_38c5_155a_5646),
            (3000, 0x7007_990e_9dee_3611, 0xff2a_d0f8_7730_b019),
            (4500, 0xd474_52de_8959_464a, 0x611f_1213_4e5c_2114),
            (6000, 0xb7ce_b176_79cf_55c7, 0xc564_edf0_c36e_20b1),
            (7500, 0xf138_5b85_3fa1_f90e, 0x1411_ca54_2356_0467),
            (9000, 0xf925_d235_7d45_437e, 0x7fd2_e252_0933_4ec2),
            (10500, 0x8e32_01e3_a8e2_5bae, 0x181c_26b2_6a65_808e),
            (12000, 0x6711_a675_e266_d100, 0x5be9_4592_588d_4d8c),
            (13500, 0x182e_a423_e656_3e90, 0x8656_b106_5dd0_732d),
            (15000, 0xb9bc_0ee2_7dc5_f2ec, 0x32d8_23cc_13eb_ea37),
            (16500, 0x8e6a_98a5_58ea_d1d9, 0x06b1_7347_2859_6885),
            (18000, 0xdd0e_7ea4_074a_d6ac, 0x883a_2c6c_7923_d110),
        ],
        last: 0xfbcf_d75e_72de_437b,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x676a_c19e_2e0e_f2fb, 0x471f_edd0_aff7_d647),
            (3000, 0x76aa_ef65_d6b7_735a, 0x2e4c_314b_9eeb_4905),
            (4500, 0x4f01_e9f6_7b6a_ac17, 0x3b8b_f0c6_af14_844f),
            (6000, 0x7252_3a11_09f0_7b4b, 0xb073_86e7_b83a_c787),
            (7500, 0x24b8_e585_9aae_7664, 0xa753_151e_ba86_af30),
            (9000, 0xb7e1_aaf1_b658_1bbf, 0x61cf_e629_c469_1f51),
            (10500, 0xcb4e_a122_aae3_4aa2, 0x5e30_e64e_4277_c1e7),
            (12000, 0xd5c8_e994_4e61_ce2e, 0x1515_d203_1e4e_9154),
            (13500, 0x92c2_6478_8d0a_606d, 0xb04e_d34a_abc9_daad),
            (15000, 0x7b2e_9688_0b45_4d47, 0xc531_2a6a_380d_3868),
            (16500, 0xd5c8_9513_df98_c2c0, 0xb8db_435e_aabc_40bd),
            (18000, 0xe636_1fd7_bbce_e527, 0x4d13_b6e3_4d1c_1e82),
        ],
        last: 0x0dd5_0e1c_0fa3_992c,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x0192_25b5_c460_f0a1, 0x92d9_9768_157d_039c),
            (3000, 0x1420_49a8_b89d_b47f, 0xffd0_19df_1b3b_03d5),
            (4500, 0xb18a_dc67_aacd_7f5f, 0xde46_54f1_f264_d50a),
            (6000, 0x4629_a3c3_8c8f_75f1, 0xe417_0bad_9be1_f22d),
            (7500, 0xa5ae_55f1_69da_27a0, 0x6123_b909_75f2_872d),
            (9000, 0xb303_71bb_daa4_b2da, 0xf9a3_9a79_0821_b5f0),
            (10500, 0x1fd8_6883_b0c7_8618, 0xee91_808a_19c1_a518),
            (12000, 0xd636_c8d3_3ea4_136d, 0xd288_2bf9_a3c5_ed4b),
            (13500, 0xefb0_4539_9f39_f971, 0x2507_684a_ded5_af68),
            (15000, 0xb462_5e8a_8f1b_c8a5, 0x9094_76aa_a788_e257),
            (16500, 0x79b4_4c54_f658_8612, 0x47f2_01eb_d5d3_59ce),
            (18000, 0x49d9_42e3_1abf_b5bb, 0x5136_7519_da7b_e971),
        ],
        last: 0x26af_3865_b9a9_1807,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7047_2c96_1f81_5434, 0x0c65_6338_0b8f_6c94),
            (3000, 0x7fdf_3d65_0714_80ba, 0x7cf6_201e_13cc_2946),
            (4500, 0x7e43_b4e8_a2f1_831e, 0x531c_fd80_a8fe_1fb7),
            (6000, 0x3031_9591_7f1b_6fdd, 0x289f_afab_30b4_ebf8),
            (7500, 0xc4a2_121a_4405_39cb, 0xf078_c2aa_1aed_b570),
        ],
        last: 0x1ce0_73fc_157e_bb4c,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6867_f8c6_0fb1_a2ca, 0xda0d_2041_fb16_c6af),
            (3000, 0x7cba_da61_eac5_541b, 0xe72e_48da_9a63_bb45),
            (4500, 0x172a_6f6f_4717_2eff, 0x2c82_49b8_4bca_c9f2),
            (6000, 0xb3a7_29ba_979d_fcea, 0xe821_931c_4d7f_050f),
            (7500, 0xdb03_3e42_80fe_a4f1, 0x9a08_4663_ecd0_a6db),
            (9000, 0x7909_4bc2_a532_0e37, 0x1e2e_6b7d_6d45_78a7),
            (10500, 0x14d3_b3b0_aa22_e22a, 0xf3a5_d337_365d_e09e),
        ],
        last: 0x3865_6d43_971c_3bb3,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xae29_1fcd_f513_28dd, 0x2c86_537f_23f3_8ccc),
            (3000, 0x2c72_d0b1_3e26_4ef6, 0x2c9b_1d35_9f28_c36f),
            (4500, 0xd2de_6d3b_3969_36bb, 0xdd8a_c97b_2cf5_256e),
            (6000, 0xa098_4c68_8fa6_155b, 0x3204_84ee_4aab_a9ca),
            (7500, 0x6714_c293_6c99_ddf3, 0x24dd_568d_fcc8_3d25),
            (9000, 0x6579_df47_e07e_b61b, 0x3f68_3543_3a9c_6239),
            (10500, 0x412a_6285_bbb7_a912, 0x1009_8f06_fbe6_6ce0),
        ],
        last: 0x197c_e11a_a02e_d4ff,
    },
];
