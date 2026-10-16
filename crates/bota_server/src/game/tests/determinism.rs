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
            (1500, 0xb0cf_71f6_636c_3e37, 0x1192_d91e_b2d5_0631),
            (3000, 0xc23d_3b56_81bc_12bf, 0x0d81_b821_c770_ce36),
            (4500, 0x8794_d650_4b69_0dbb, 0x4f8c_26b1_e4b8_d169),
            (6000, 0x395e_bdef_5b5c_3031, 0xd069_8a57_2c97_5cbd),
            (7500, 0x26b5_6828_3ff3_6911, 0xef4d_e7a1_e01a_8b98),
            (9000, 0x51c2_81df_ca19_7ff3, 0x17d9_f33e_54f5_b38a),
            (10500, 0xa0b5_8833_5d2f_d0e4, 0x7314_3b42_70af_64a9),
            (12000, 0x60d5_561a_9f81_8768, 0xc16e_3871_c910_5149),
            (13500, 0x5d47_6708_926c_d7f8, 0x11c9_d9e2_b6ef_ff66),
            (15000, 0xc75e_4360_58fe_86c0, 0x0dbb_86b8_b22f_eabe),
            (16500, 0x8dc6_f1a5_07ce_555e, 0xfbc2_9412_a1cb_fc6f),
            (18000, 0x4f51_d121_b4b4_9ebe, 0xa27e_c359_4322_eb5e),
        ],
        last: 0x960d_2cb7_d8ce_ca4c,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x5909_c2b4_3ec6_4b8f, 0x6fe0_6b69_a34d_d7b7),
            (3000, 0xf573_906a_64f0_be64, 0x254e_d4d2_fdff_db29),
            (4500, 0xeb22_7522_21a0_08aa, 0x8555_205c_a863_108e),
            (6000, 0x39a1_6af7_2f92_b188, 0xa5e8_2c40_c541_4636),
            (7500, 0x9d1d_a97b_0aad_0295, 0xecc3_209b_8a19_7a38),
            (9000, 0x0bbe_24f0_4936_4e48, 0xf6d9_5ade_d19b_8066),
            (10500, 0x90ed_f8bb_36a8_0b60, 0x62e7_3413_29e6_2bb9),
            (12000, 0x8d57_762e_b5c4_944d, 0xe55b_f3b3_9ace_6726),
            (13500, 0xfbbb_4545_97e5_8095, 0xea52_0324_3f85_ede9),
            (15000, 0xac3c_2067_ebd5_58f0, 0xf154_d7a4_6f4a_359a),
            (16500, 0xc033_96cd_ae73_5dea, 0xd6cc_c79f_9081_9006),
            (18000, 0xaf15_e9cd_f374_6ddd, 0x7eb2_8105_ef32_3cb7),
        ],
        last: 0x765b_19eb_4998_5ac6,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x5859_9c98_dddb_788e, 0x1725_3c15_ecf0_2e23),
            (3000, 0x5024_e01a_8092_152f, 0xdaf0_9cea_d73e_eefb),
            (4500, 0x6820_fcde_e263_5229, 0x764a_1768_0735_5100),
            (6000, 0x325e_9fc0_0573_b6db, 0x95c9_5783_f76c_68fb),
            (7500, 0xd5a5_3eab_c307_2e39, 0x0395_b10f_bb79_f19a),
            (9000, 0x573c_abf0_1a1a_7bc8, 0xd56e_5d10_cf3a_2ccb),
            (10500, 0x992f_829b_a6b2_c714, 0xf389_35b6_d7a3_4938),
            (12000, 0x5f01_866e_193c_0950, 0x115e_7d5c_a9da_30a4),
            (13500, 0x3d80_18c4_ed06_9445, 0xc45b_6241_4709_6044),
            (15000, 0x8169_1916_7abb_4683, 0x4a6a_a170_a848_c003),
            (16500, 0x918b_539f_3933_a202, 0xb664_a60f_7693_1e77),
            (18000, 0x0399_c974_84a7_cdd3, 0xc4c4_8559_a498_e233),
        ],
        last: 0x28d9_454f_86a6_7f9a,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xb179_99e1_7479_a041, 0x991d_4310_3b91_6762),
            (3000, 0xe2b9_8400_ce05_5fc1, 0x4d47_cd9e_f392_abde),
            (4500, 0xbdb7_2f3c_c281_ca92, 0xc670_d506_5d91_97f5),
            (6000, 0x0d3d_0ddd_4698_b6a9, 0x9426_9550_9c2e_21b7),
            (7500, 0xddb3_d1da_3336_f406, 0x03c0_d4d0_a6c0_8315),
            (9000, 0x8838_d0d1_17d7_3287, 0x43a4_0a71_46d8_665c),
            (10500, 0xafc6_8eab_226a_f9af, 0x33d2_dc9d_a764_2eae),
            (12000, 0x8cab_33b0_2c23_7609, 0x2a00_0d0d_7f0b_9537),
            (13500, 0xa5f0_37c7_02f6_90bb, 0x4dc0_c014_8185_727f),
            (15000, 0x61ff_fbd5_9182_ef29, 0xd084_4d1f_7d48_36b1),
            (16500, 0x09d7_5173_e586_73be, 0x05bd_d0e7_7757_ef70),
            (18000, 0x17d9_f73a_fa4a_3392, 0x53e1_36a5_6b68_c70a),
        ],
        last: 0xe2c4_840d_4da0_f074,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc9f7_30db_186e_fe36, 0x9ded_3418_01d2_c5f1),
            (3000, 0x6a82_bd21_c4c2_f887, 0xb637_8b56_173c_bd1f),
            (4500, 0xcd9e_77dc_8b3c_74b0, 0x7012_9938_d1aa_d284),
            (6000, 0x3183_9963_f832_5b83, 0xcf60_c868_620b_7efd),
        ],
        last: 0x169e_1dcf_f72c_c4c2,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc9a9_431f_65a4_ce75, 0x4313_3435_30d3_275e),
            (3000, 0xf903_93f1_e290_32ad, 0x824d_8931_6df2_2ee8),
            (4500, 0x7036_6b09_335b_9ae3, 0x7482_0602_0122_227b),
            (6000, 0x6050_1fc1_59fb_c29d, 0x9828_a930_749c_d73e),
        ],
        last: 0x3287_296a_f008_cfa0,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb821_1fe1_5ae5_3050, 0x2525_a4fe_7272_5b73),
            (3000, 0xbdaf_36c9_b09a_c9f6, 0x46b8_6c88_b610_1990),
            (4500, 0x0db6_df0e_5b48_547f, 0x2fde_f682_464c_9aaf),
            (6000, 0x68eb_6993_8f30_626c, 0xca5d_0ddf_74a8_ca4f),
            (7500, 0xab73_44f4_1785_ae13, 0x397e_500b_f0ac_1de3),
        ],
        last: 0x2aa4_29f9_0b17_5e39,
    },
];
