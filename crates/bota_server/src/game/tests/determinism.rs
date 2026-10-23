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
            (1500, 0x5392_3800_5a71_e17b, 0x45e8_d0e1_cdf5_b3e2),
            (3000, 0x1827_1d5e_2482_46a5, 0x754a_0aa3_5d8a_1fca),
            (4500, 0xeba7_e4f5_6833_aef0, 0x5372_04f2_9dc3_f599),
            (6000, 0x6727_9e52_eeef_6112, 0xa6f9_6883_168f_cbcd),
            (7500, 0x3b80_bedc_355b_bea1, 0x8261_d644_1904_b920),
            (9000, 0xd472_07ab_9f10_c582, 0x311b_8c9e_c5be_d67e),
            (10500, 0x6612_357a_4503_d62e, 0xc58e_b974_0182_2e88),
            (12000, 0x31be_b02e_e871_a607, 0x374f_4fbd_ec52_89f8),
            (13500, 0x87ad_2a88_34c0_85d6, 0xe240_0a70_d4d6_c4d9),
            (15000, 0x1473_b5ef_a4fe_0a85, 0x5aa8_5642_985e_26a0),
            (16500, 0xa702_a658_115f_981a, 0x87b6_260c_9c6e_2553),
            (18000, 0x0b65_2858_29a9_3b1e, 0x11c8_c9f5_bcb8_6c97),
        ],
        last: 0x98e3_650b_0b8a_3755,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xd867_9dfa_ffef_de40, 0xabc6_8841_2555_5e20),
            (3000, 0x535e_df36_bb87_162c, 0xe0f7_eea5_d4f3_3fb8),
            (4500, 0x1a5d_48bf_6ca6_0aa7, 0x7a08_d7e6_3183_96a3),
            (6000, 0x7f65_3352_43d7_aead, 0x2b48_2b48_69e0_8b7d),
            (7500, 0xbd2b_cd2a_8438_d29b, 0xdb2c_9dc1_eed9_19d5),
            (9000, 0x5685_6fb3_ad66_e768, 0xb2a2_e88e_16c4_a5a0),
            (10500, 0xed8d_a151_2824_b603, 0x510a_27d0_ec8e_6ba4),
            (12000, 0x22e6_6426_a4f0_3770, 0xcba7_bd5e_ae8e_d7d0),
            (13500, 0x7e53_d5b7_bf4f_70fc, 0x22dc_dbe3_5c4a_eaa3),
            (15000, 0x4977_929c_38b5_22e4, 0x5e49_550f_8922_0d5b),
            (16500, 0xb122_a8e7_deda_2d9c, 0xed11_e2df_f501_3397),
            (18000, 0x1949_ed2a_23f0_6e7f, 0xa29c_5235_e10c_b267),
        ],
        last: 0xf997_2713_3073_880b,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3e72_ba7c_e8f7_0daf, 0x9bb5_2893_9fd4_d97b),
            (3000, 0x55bb_cbb1_21db_ad80, 0x8ae4_bffe_bfb6_4aaa),
            (4500, 0x7e5d_5c07_7d0e_a9d5, 0x24b8_3903_0a5a_c9ca),
            (6000, 0xfc43_391b_8ae7_d51b, 0xca6c_70b8_5919_23cb),
            (7500, 0xc4d7_58ed_8b8c_3fe6, 0xbc39_6154_a0a1_52e5),
            (9000, 0x1355_5b9a_c71b_8829, 0xde91_f8b9_568b_7a43),
            (10500, 0x7904_7a77_6ca3_5e2d, 0x0c70_44a5_0127_3c7c),
            (12000, 0x4486_88fd_c666_1ef3, 0x4905_f46e_a8d5_62df),
            (13500, 0x2052_c379_c463_9a69, 0xcd51_9d48_4f9f_2f28),
            (15000, 0x64f5_b4e7_e931_04a9, 0xdc9f_90a6_5619_7a0a),
            (16500, 0x765e_5bdb_cdd4_88a3, 0x9d89_9ff9_c0d8_6076),
            (18000, 0xed5c_c3db_5653_fbef, 0xd6cc_d541_18c1_60b4),
        ],
        last: 0xc000_13e8_5255_c487,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x44b7_155f_cbba_c7d5, 0x6c36_a027_6a2b_8809),
            (3000, 0x6245_b7bc_d5cd_6e07, 0xbe60_0ae5_9925_86e6),
            (4500, 0xdfc3_0688_3386_a277, 0xad5b_9e3e_eb38_22e4),
            (6000, 0x62c4_3a06_e673_7fcb, 0x54dc_5363_8ed8_e3b3),
            (7500, 0x6f29_fe7f_78c6_4c15, 0x31c2_105a_0600_67ba),
            (9000, 0xabd6_370b_1e97_d3d7, 0x721e_ce5f_ef86_bc15),
            (10500, 0xf09d_be3a_8855_d1ec, 0x285b_d6cb_c976_f1a9),
            (12000, 0x44f4_9feb_cb94_dbe6, 0xfdb1_0b0b_5d85_89aa),
            (13500, 0xfc25_9ad2_bfd5_3636, 0xe9d7_3014_d430_97ee),
            (15000, 0xb7f2_d5d8_1376_81ff, 0x5aeb_f19c_2e88_96f4),
            (16500, 0x5ed9_2005_4ee0_4b23, 0x1817_3e0e_53ec_d0b4),
            (18000, 0x5851_72b7_34d9_c7c2, 0x5da4_f552_f7f5_76c1),
        ],
        last: 0x962a_d568_a692_2dc8,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7047_2c96_1f81_5434, 0xddea_5d1d_c3c4_af0e),
            (3000, 0x78ea_2c40_f9ee_8a65, 0xf49a_d39f_bb5f_6746),
            (4500, 0x3dc8_3b10_f9ff_752a, 0xbdf7_33aa_9dc1_814f),
            (6000, 0x31cd_9e24_7bc9_f53f, 0xc73a_19b0_0f99_ccf0),
            (7500, 0xfb73_ea86_9b6b_aa49, 0x8531_205e_82e6_e81e),
            (9000, 0x7f3b_bb3e_7226_5f85, 0x17fb_58e7_99de_517a),
        ],
        last: 0x882e_33f1_b944_3c79,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x9acd_c0b3_cafb_b548, 0xfaa8_f25f_87bc_37e5),
            (3000, 0x3ec2_7751_b93f_18d1, 0x4b8c_6c94_c517_3381),
            (4500, 0x0d5a_a544_1546_2443, 0x44d5_418f_320f_61ae),
            (6000, 0x40f9_10dc_022d_3e2f, 0xcc76_810c_2b24_0e04),
            (7500, 0x1a58_69b2_9ab4_ef8d, 0xa6ed_91e8_d100_7190),
        ],
        last: 0x684a_25d8_89e3_dba3,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xae29_1fcd_f513_28dd, 0xfead_f097_8b3b_0a24),
            (3000, 0xa7e8_985e_f894_e0f3, 0xcc40_548c_b8f9_b4dc),
            (4500, 0x76d1_67a1_3cc5_fb18, 0xaa0b_0d83_8442_a024),
            (6000, 0x45ec_875d_3953_3157, 0x3eab_291f_16c8_a37e),
            (7500, 0x52b2_9315_886c_7ac9, 0xd4e5_ea7a_ff6f_541b),
            (9000, 0x7168_b96c_70c2_9477, 0x2514_6f7c_8486_355f),
        ],
        last: 0x1fe3_be36_e79f_5d9e,
    },
];
