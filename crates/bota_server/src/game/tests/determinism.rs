//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_core::{ITEMS, map_of, rules};
use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, Fnv, MatchConfig, MatchSecrets, World};

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

/// What a seat's driver decides from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Eyes {
    /// Its side's view, through the fog.
    Side,
    /// The view of everything, whatever the fog shows: orders that depend
    /// on the simulation alone, never on what a side is told.
    All,
}

/// The orders every seat gives this tick, validated as a driver does, with
/// each refusal folded into the digest.
fn decide(world: &World, drivers: &mut [Driver], digest: &mut Fnv, eyes: Eyes) -> Vec<Command> {
    let mut commands = Vec::new();
    for (seat, driver) in world.seats.iter().zip(drivers.iter_mut()) {
        let view = match eyes {
            Eyes::Side => world.view(seat.team),
            Eyes::All => world.view_full(),
        };
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
    replay_with(pinned, horizon, Eyes::Side)
}

/// [`replay`] with drivers deciding from the given view.
fn replay_with(pinned: &Pinned, horizon: u32, eyes: Eyes) -> (Vec<(u32, u64, u64)>, Option<u64>) {
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
            decide(&world, &mut drivers, &mut digest, eyes)
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
                    events: crate::game_loop::visible_events(&events, Some(team)),
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
            decide(&world, &mut drivers, &mut digest, Eyes::Side)
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

/// Ticks the fog-blind replays run: one checkpoint in debug, four in
/// release.
const BLIND_TICKS: u32 = if cfg!(debug_assertions) {
    CHECKPOINT_TICKS
} else {
    4 * CHECKPOINT_TICKS
};

/// The world hashes of every pinned match played by drivers that see
/// everything, at each checkpoint within [`BLIND_TICKS`].
fn blind_hashes() -> Vec<Vec<(u32, u64)>> {
    PINS.iter()
        .map(|pinned| {
            replay_with(pinned, BLIND_TICKS, Eyes::All)
                .0
                .into_iter()
                .map(|(tick, world, _)| (tick, world))
                .collect()
        })
        .collect()
}

/// Prints the world hashes of the fog-blind replays, to record
/// [`BLIND_PINS`] afresh.
#[test]
#[ignore = "records pins"]
fn print_blind_pins() {
    for hashes in blind_hashes() {
        println!("blind {hashes:#018x?}");
    }
}

/// Whatever a side's view of a tick shows, an order given on that view may
/// name: no unit or item on the ground it was shown is refused as unknown,
/// over the first checkpoints of every pinned match.
#[test]
fn whatever_a_view_shows_an_order_on_it_may_name() {
    for pinned in &PINS {
        let cfg = config(pinned);
        let mut world = World::for_match(&cfg, cfg.rng());
        let mut drivers: Vec<Driver> = (0..cfg.picks.len() as u64)
            .map(|slot| Driver {
                state: ((pinned.seed << 8) | (slot + 1)).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            })
            .collect();
        let mut digest = Fnv::new();
        while world.tick < BLIND_TICKS && world.victor().is_none() {
            let commands = if world.tick.is_multiple_of(DECISION_TICKS) {
                each_shown_is_nameable(&world);
                decide(&world, &mut drivers, &mut digest, Eyes::Side)
            } else {
                Vec::new()
            };
            world.advance(&commands);
        }
    }
}

/// Asserts that every seat with a hero standing may name in an order every
/// unit and item on the ground its side's view shows.
fn each_shown_is_nameable(world: &World) {
    for seat in world.seats.iter().filter(|seat| seat.unit.is_some()) {
        let view = world.view(seat.team);
        let named = view.units.iter().map(|unit| Order::Attack {
            target: Target::Unit(unit.id),
        });
        let taken = view.loot.iter().map(|loot| Order::Take {
            target: Target::Unit(loot.id),
        });
        for order in named.chain(taken) {
            assert_ne!(
                world.validate_order(seat.slot, None, &order),
                Err(bota_proto::RejectReason::UnknownTarget),
                "tick {}: seat {} was shown what {order:?} names",
                world.tick,
                seat.slot.0
            );
        }
    }
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

/// What a side is told must never change what the simulation does: driven
/// by orders that do not depend on the fog, the pinned matches replay to
/// these world hashes (four checkpoints, one in debug) however the views
/// are worked out. A change to the rules moves them; a change to what the
/// sides are shown does not.
#[test]
fn what_the_sides_are_shown_never_changes_the_simulation() {
    for (hashes, pinned) in blind_hashes().iter().zip(BLIND_PINS) {
        let expected: Vec<_> = pinned
            .iter()
            .copied()
            .filter(|&(tick, _)| tick <= BLIND_TICKS)
            .collect();
        assert_eq!(hashes, &expected);
    }
}

/// The world hashes [`what_the_sides_are_shown_never_changes_the_simulation`]
/// holds the fog-blind replays to, one list per pinned match.
const BLIND_PINS: [&[(u32, u64)]; 7] = [
    &[
        (1500, 0xf629_3521_d333_4f33),
        (3000, 0x0eec_7236_d109_63b5),
        (4500, 0xda4d_ba71_10e7_45f1),
        (6000, 0x7ff7_9e94_94e9_99ca),
    ],
    &[
        (1500, 0xae25_85a8_a211_b835),
        (3000, 0x0b68_478f_2f50_933d),
        (4500, 0x49bb_748b_5ba1_a8c1),
        (6000, 0xfffe_2a47_c4d3_7e4c),
    ],
    &[
        (1500, 0x8670_6f9f_880f_7919),
        (3000, 0x3ed1_a93b_145d_b2bf),
        (4500, 0x4d96_6b27_c801_a613),
        (6000, 0x0b72_1d2e_a27b_a2fb),
    ],
    &[
        (1500, 0x6016_aca9_d37a_8374),
        (3000, 0xab42_9657_ee4a_f6f1),
        (4500, 0xc7f8_34cf_5eee_f372),
        (6000, 0x58c4_45db_a76c_01a1),
    ],
    &[
        (1500, 0x6e6d_6a32_f55d_8ee5),
        (3000, 0x9660_f91b_7cf8_e5a7),
        (4500, 0x24c2_56b2_d59f_8be2),
        (6000, 0x54a1_0397_6fdc_a1ed),
    ],
    &[
        (1500, 0x5478_93e0_6942_cbd9),
        (3000, 0x6e69_282c_1065_99bb),
        (4500, 0xa0b0_402f_537d_71b0),
        (6000, 0x9218_5155_9daa_119a),
    ],
    &[
        (1500, 0xf704_5df1_b122_bb89),
        (3000, 0x9acc_4dbc_7f75_6e13),
        (4500, 0x884b_87d1_a6bb_28b3),
        (6000, 0xd5d7_59ee_b91a_1477),
    ],
];

/// Every pinned match, recorded from the simulation as it stood when the
/// contract was laid.
const PINS: [Pinned; 7] = [
    Pinned {
        map: MapId(0),
        seed: 1,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x3a29_dd2a_0294_062d, 0xa9d6_30f4_5b3e_2a58),
            (3000, 0x1ed9_ec24_e98f_e348, 0xe41e_e0a9_f5ca_1eb4),
            (4500, 0x9e1a_c3de_5973_063c, 0x0586_4dce_2251_28f4),
            (6000, 0xccfb_f0de_5ff0_b196, 0x3834_7245_025b_b377),
            (7500, 0x1433_be2e_be9b_fc41, 0xc209_c2dc_bc62_281f),
            (9000, 0x827a_695c_ae17_7fa0, 0xf411_3b7e_4d98_3ffa),
            (10500, 0x663d_02e3_6a0d_9c84, 0x9e40_e6b6_e71f_f19c),
            (12000, 0x3549_921d_c80f_70eb, 0xb699_2e89_5e12_de4a),
            (13500, 0x8b37_9c80_8e29_a6c4, 0x5e1e_cde1_625e_752a),
            (15000, 0xe46a_691a_3fcd_d035, 0x1a81_e9de_e5d6_a191),
            (16500, 0xd684_6e99_ce80_d4fc, 0x7b55_37cd_77ef_ebdf),
            (18000, 0x070f_b80a_7c67_bebe, 0xbc7c_71f2_cb67_65a1),
        ],
        last: 0x7e45_33e5_100b_7b85,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x836f_e1f0_002f_748d, 0xeee8_2b1b_226a_fa74),
            (3000, 0xe92b_3569_9545_84c8, 0x16b2_78d3_8ae5_3207),
            (4500, 0x1c58_a0fe_81fe_8308, 0x73b4_a80c_8dec_b960),
            (6000, 0x5415_efbf_9149_f16a, 0x9f52_7e5d_1ce9_6283),
            (7500, 0xa115_1df5_412c_ed71, 0x50c3_4ccb_2e13_dbcf),
            (9000, 0x6512_dbeb_bd63_8ed6, 0x1b8e_9706_3c12_6bf0),
            (10500, 0x0318_5114_c490_b7b1, 0xa7e8_d73a_2c72_5e9b),
            (12000, 0xcd38_2bfb_e9ea_0c1b, 0x1fe0_c5e1_23ea_a35c),
            (13500, 0x4a12_631b_af41_cac3, 0x5cf6_a37f_3d31_ffb0),
            (15000, 0x5d01_b77f_2594_7cdf, 0x51f3_8f0e_f852_1316),
            (16500, 0xfbd9_1a14_a8a5_c704, 0x3e70_92ab_d951_a990),
            (18000, 0x2c8d_8fed_86ae_baad, 0xf076_bcb1_b4e4_367a),
        ],
        last: 0x96a2_f30e_ea44_8307,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x5c4c_763a_cb00_96ff, 0x58c1_fc87_6f63_5314),
            (3000, 0x0eda_3bdb_d9ea_9822, 0x82fc_702f_8528_e4b1),
            (4500, 0x7ba0_6ea8_cc5b_8a3b, 0xd44a_698f_0ef9_7fc1),
            (6000, 0xe122_cd83_9a6c_e1b7, 0x224d_6ba7_19f5_e5ef),
            (7500, 0xe026_75aa_f074_632b, 0xea84_0b97_a9f7_b2aa),
            (9000, 0xd4f5_b698_cec0_1d52, 0x443a_6c3e_22c0_5bfa),
            (10500, 0x1f18_de18_a278_d5dd, 0x9112_e044_ef2e_210d),
            (12000, 0xc58f_0e5f_9971_46ca, 0x6b8c_987b_597a_f490),
            (13500, 0x0b1d_3bb7_9186_cc25, 0x728a_7dee_9c0d_f85c),
            (15000, 0x1ddc_be32_3cb6_2771, 0xbc62_b58c_b7a8_de21),
            (16500, 0xd9f7_34b5_fd3e_3196, 0xcc9b_db69_3746_966a),
            (18000, 0xa75f_a62f_ae83_b17c, 0xcff6_fa6a_fea5_3a85),
        ],
        last: 0x0d9b_945c_c3d9_dbad,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x88a0_3f2e_ef78_af6f, 0x6cab_d93d_64cf_7232),
            (3000, 0xed0f_221d_1e85_53e2, 0x066c_8a34_75b3_aa7d),
            (4500, 0x1cba_8530_04fd_83ac, 0x67ae_46bd_1010_0cfd),
            (6000, 0x479d_8de9_3df3_8f79, 0xe040_c8c8_62dc_21dd),
            (7500, 0x7bfa_2266_f417_b03f, 0x7fc5_57ac_1f1a_42eb),
            (9000, 0x2efd_3589_4f1e_bc9f, 0xd555_7b2c_56dd_f5a2),
            (10500, 0x03d9_530c_cc74_b6b2, 0x2f17_d100_b635_81aa),
            (12000, 0x3012_3f62_6d0d_ae55, 0xf68d_5beb_f409_a641),
            (13500, 0xb31c_4bd0_a945_cf93, 0xc61b_3753_6e86_7883),
            (15000, 0x9d86_62e7_c86e_cefc, 0xc1da_ae9a_3818_90d8),
            (16500, 0xda62_96ea_78f5_0327, 0xa896_0eae_ceb3_03de),
            (18000, 0x25ee_1cf0_7d93_7688, 0x8c47_d966_1d37_b0b8),
        ],
        last: 0xc20b_b441_30bb_10bf,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4a9a_fa6d_a848_e8c3, 0x1b30_30a0_a335_ad8b),
            (3000, 0x7788_a7de_4b03_9cce, 0xbd00_9e31_ebbc_303c),
            (4500, 0xc607_a695_cdf8_e6ba, 0xafba_7c42_fb1c_e2b1),
            (6000, 0x953f_750a_da88_c646, 0x8209_6c27_ae00_d9ba),
            (7500, 0x4452_fd67_0b93_5737, 0x3a38_0a69_3f97_2c8f),
            (9000, 0x18fb_975a_844d_7c84, 0x2c6d_b88d_b4b0_418c),
        ],
        last: 0x4880_d6c7_037c_3deb,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xeb0c_052c_104f_8bc1, 0x88ef_bded_7698_625a),
            (3000, 0x6041_9e48_5c88_2946, 0xac88_88d5_0b5a_62ba),
            (4500, 0x9a25_248e_8edc_70a1, 0xb1f2_b425_73f5_5868),
            (6000, 0x8cc0_9a83_217d_3cfe, 0x3827_86bb_9d90_5220),
            (7500, 0xe1f3_50c6_a9d3_bb70, 0xc753_176f_ded6_c71e),
        ],
        last: 0x62a7_e331_943a_1753,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x97fe_f2e2_3388_7b32, 0x19f8_917e_65fb_c72d),
            (3000, 0xef42_f021_bef4_d834, 0x1f4c_3efa_51c2_19d4),
            (4500, 0x3353_c3fb_5667_fb0a, 0x51cd_22a1_6327_5d62),
            (6000, 0x64d8_0076_4fe8_d0e5, 0x3394_09de_9dab_60bb),
            (7500, 0xbd4c_aa74_c4f8_2ce6, 0x07ca_1f48_8e63_2266),
        ],
        last: 0xacfa_94d4_f213_c86d,
    },
];
