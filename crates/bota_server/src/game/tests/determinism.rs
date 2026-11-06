//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_core::{Catalogue, map_of, rules};
use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, Fnv, MatchConfig, MatchSecrets, World};

/// Ticks between two pinned checkpoints.
const CHECKPOINT_TICKS: u32 = 1_500;
/// Ticks a match on a map without a cap is played for.
const UNCAPPED_TICKS: u32 = 18_000;
/// Ticks of the tests that fork a match or compare two plays of its start.
const FORK_TICKS: u32 = CHECKPOINT_TICKS;
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
                item: ItemId(self.below(u64::from(PINNED_CATALOGUE.items())) as u16),
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
        economy_events: Vec::new(),
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
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
        objectives: bota_core::map_of(pinned.map).default_objectives,
        runes: bota_core::map_of(pinned.map).default_runes,
        catalogue: PINNED_CATALOGUE,
    }
}

/// The items the pinned matches play with: the catalogue as it stood when
/// the pins were recorded. An item appended after them leaves every pin as
/// it is, as it leaves every older replay; moving this to cover new items
/// re-records the pins.
const PINNED_CATALOGUE: Catalogue = match Catalogue::first(55) {
    Some(catalogue) => catalogue,
    None => panic!("the pinned catalogue is a prefix of this build's"),
};

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
        let events = world.advance_trusted(&commands);
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

/// Replays the pinned matches on a map and checks them against their pins,
/// the whole match.
fn check(map: MapId) {
    for pinned in PINS.iter().filter(|pinned| pinned.map == map) {
        assert!(map_of(pinned.map).id == pinned.map, "the map is registered");
        let whole = match_ticks(pinned.map);
        let (checkpoints, last) = replay(pinned, whole);
        let expected: Vec<_> = pinned
            .checkpoints
            .iter()
            .copied()
            .filter(|&(tick, _, _)| tick <= whole)
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
            world.advance_trusted(&[]);
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
    while world.tick < FORK_TICKS && world.victor().is_none() {
        if world.tick == FORK_TICKS / 2 {
            fork = Some(world.clone());
        }
        let commands = if world.tick.is_multiple_of(DECISION_TICKS) {
            decide(&world, &mut drivers, &mut digest, Eyes::Side)
        } else {
            Vec::new()
        };
        let events = world.advance_trusted(&commands);
        if let Some(fork) = &mut fork {
            let kinds = |events: &[crate::game::Event]| {
                events.iter().map(|event| event.kind()).collect::<Vec<_>>()
            };
            assert_eq!(
                kinds(&fork.advance_trusted(&commands)),
                kinds(&events),
                "tick {}",
                world.tick
            );
            assert_eq!(fork.hash(), world.hash(), "tick {}", world.tick);
        }
    }
    assert!(fork.is_some(), "the match ran past the fork");
}

/// Ticks the fog-blind replays run: four checkpoints.
const BLIND_TICKS: u32 = 4 * CHECKPOINT_TICKS;

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
            world.advance_trusted(&commands);
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
        (1500, 0x42c5_6ec8_7305_e2cd),
        (3000, 0x2d51_8a93_c992_7738),
        (4500, 0x394f_811c_b005_c3c0),
        (6000, 0x9a71_331b_5be8_cf95),
    ],
    &[
        (1500, 0x3418_9753_172f_3b76),
        (3000, 0x24bd_f377_1766_e6d7),
        (4500, 0xc0bf_9374_200d_82f3),
        (6000, 0xa315_3f2d_0374_648b),
    ],
    &[
        (1500, 0x5434_8e81_402e_787c),
        (3000, 0xa08f_c8f4_36a4_4091),
        (4500, 0x74f2_9ce5_dfa2_d7db),
        (6000, 0x74e8_d574_146a_72e4),
    ],
    &[
        (1500, 0xc0e7_c3f9_47c2_d23f),
        (3000, 0x6528_a559_049a_57b8),
        (4500, 0x32b2_9126_44b8_e628),
        (6000, 0x045d_1ef4_ffc4_1b4f),
    ],
    &[
        (1500, 0x43fe_30f7_b660_8095),
        (3000, 0x80fc_d202_b34e_7d19),
        (4500, 0x2f97_dc3f_b376_8ba5),
        (6000, 0x55ad_ec10_e705_5a1e),
    ],
    &[
        (1500, 0x10e3_28a8_1eb7_2a29),
        (3000, 0x1654_1199_af8f_d3de),
        (4500, 0x4c99_97f0_67fa_fceb),
        (6000, 0x2f85_d34d_f3c0_ea18),
    ],
    &[
        (1500, 0x30ec_5599_abd3_c74d),
        (3000, 0xf3a8_6b73_196d_2a2b),
        (4500, 0x40f1_2e84_7f34_68bd),
        (6000, 0x7612_5919_454d_40a7),
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
            (1500, 0x6c37_c2ff_3e30_e931, 0xb059_3bd2_08c1_6c1c),
            (3000, 0xb645_96d2_af14_fa46, 0xef28_859f_cf43_016c),
            (4500, 0x0648_0d95_2ced_bf6a, 0x21fe_1900_d8a3_28c0),
            (6000, 0xe653_429d_95cf_e85e, 0x4cb8_bcab_94fe_4e5e),
            (7500, 0xb9fa_73e9_c192_0e7b, 0xf7ca_f551_c67a_e568),
            (9000, 0x6b45_37cc_17d9_9a02, 0x7144_48dd_d579_8216),
            (10500, 0x5f19_f5e0_862f_cb92, 0x57e4_a637_a5d7_48fb),
            (12000, 0xf4c3_cbbb_3726_d572, 0xe6ab_20e3_b940_30f0),
            (13500, 0xce35_cef9_b9eb_b14b, 0x9577_377e_3a74_14b2),
            (15000, 0x6ccc_6d9a_d1db_8e0c, 0xec5a_0adb_72bd_0472),
            (16500, 0x605b_da82_15e5_aa9f, 0x01be_fd8b_1c2a_d85b),
            (18000, 0x145f_49bc_6c4e_8f85, 0x5809_0a7b_4f37_2040),
        ],
        last: 0xede7_9672_ccf3_92bc,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe1ce_6693_0ac8_a396, 0x2ea9_730f_f4b1_5fa2),
            (3000, 0x9037_9bf5_4c12_e0c7, 0x3cd4_c982_3d8b_4b1c),
            (4500, 0x6179_be9c_0390_79fe, 0x9c09_8f9d_089f_7633),
            (6000, 0x5ca0_d139_4869_06e8, 0x8fff_a287_57cc_055d),
            (7500, 0x98bc_ece8_963e_e4cc, 0x2790_987c_6800_9190),
            (9000, 0x710b_8e02_b682_ed4e, 0x456b_4dd5_5084_c057),
            (10500, 0x8194_713a_3560_dd7d, 0x8a1e_ab47_a989_15e7),
            (12000, 0x7291_5e0f_7c02_ac15, 0xca4e_a049_1eae_180f),
            (13500, 0x9aab_573d_c463_9f0f, 0x6dba_c8f2_399b_ad53),
            (15000, 0x2ab2_e1c8_6efc_7922, 0x5c89_ee8d_bca3_9485),
            (16500, 0x84cd_357e_a2b9_8348, 0xf6ed_d841_ecd4_6e6f),
            (18000, 0x22f8_9478_82d3_c1af, 0x929b_20ec_cd74_a989),
        ],
        last: 0x83d3_8584_1996_767e,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x4bf1_ecdb_e523_2f1b, 0x4bc2_9f0d_aeae_9ae9),
            (3000, 0xb905_6efa_0f7b_0e85, 0x568e_edef_7a54_d121),
            (4500, 0x33fe_590f_36c8_d3a3, 0x9573_da41_6e2c_1208),
            (6000, 0x88fc_c3f6_394b_a810, 0x7b0c_98ea_fc36_c362),
            (7500, 0x8a88_abcd_9fcc_ccc7, 0xdd01_2ce8_4092_bef6),
            (9000, 0xfe86_9599_3b00_17f5, 0x19b4_71f5_2566_e248),
            (10500, 0x7c2a_0e81_98e6_956c, 0x592b_dc63_da83_a32b),
            (12000, 0x8a7a_16be_efbd_3967, 0x0467_5bea_e798_191d),
            (13500, 0x7307_5871_a72b_f0a7, 0x2e27_fa82_b860_888c),
            (15000, 0x8033_f861_8cbc_0013, 0x1360_af10_7782_18d7),
            (16500, 0x5445_9f36_b7a0_013e, 0x11c0_61b6_3769_46d4),
            (18000, 0xbeb9_ce71_36c3_d60d, 0x583a_5f32_bc53_efca),
        ],
        last: 0x1e73_7354_161f_9e50,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x79e9_de9e_eb1a_478b, 0x6703_ccab_3ae2_adea),
            (3000, 0x15cf_02ee_85a8_10c5, 0xb670_df15_df95_faf1),
            (4500, 0x4f38_ed7f_4e09_ba6d, 0x4a3e_2b90_d07b_d806),
            (6000, 0xa616_591b_df5a_4844, 0x95ad_7aca_2c03_ebc6),
            (7500, 0xc9c8_1a19_9366_4ca7, 0xdcf0_78dd_e57b_46cd),
            (9000, 0x9c9f_c987_7f86_f5f8, 0x0670_3a1a_5ac4_ac59),
            (10500, 0xde3a_0bec_b086_a860, 0x4cb0_ede3_66cb_01d5),
            (12000, 0x1077_6614_2820_9fd2, 0x93df_48aa_5df5_1299),
            (13500, 0x97ec_a0cb_66d9_0e72, 0xdb39_8e9d_068e_9268),
            (15000, 0xba64_2a50_5e6b_5af4, 0x403d_023d_d747_6b66),
            (16500, 0x3d9b_86fe_0489_94db, 0x399c_e6dc_a99c_4da4),
            (18000, 0xc886_3781_f80c_ddf9, 0xf322_d64e_876b_9edb),
        ],
        last: 0x0071_8e33_762f_ac61,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x64a4_8e5c_2731_982c, 0x55ef_798f_0d69_9e04),
            (3000, 0xda43_9769_5f19_f5e5, 0x21ac_a037_60d2_96a1),
            (4500, 0x0332_0085_a641_03c9, 0x1ea4_1b80_7c6d_4de8),
            (6000, 0xebf1_123c_4283_2e6b, 0x0911_69a1_86a2_0c68),
            (7500, 0x0114_8867_e658_85c3, 0xdc4e_e198_4694_d986),
            (9000, 0xc9b2_a646_2da5_95b7, 0xbf48_c192_87ef_2119),
        ],
        last: 0x01a5_14a3_d507_af84,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x33f1_e541_fa36_2d71, 0xcf2d_2f95_7b24_7f60),
            (3000, 0xe01e_2aa0_10ae_b06b, 0xbf25_fac2_47ba_508f),
            (4500, 0xb9f5_aecc_72e3_4af4, 0x02ab_a846_95b1_c92c),
            (6000, 0xdcd9_5673_2df5_c717, 0x90da_58c1_dccf_f5cf),
            (7500, 0x79e4_4bde_aa28_2cc8, 0x76f9_ccf2_77a7_e3a8),
            (9000, 0xc9d4_1918_8500_c60d, 0xff4f_fc09_2e0b_3445),
            (10500, 0x3f78_1a90_63f9_a903, 0xd33d_c67a_ee07_5130),
            (12000, 0x30a1_0d22_a3bf_08e7, 0xe698_9af8_6e89_e1a3),
        ],
        last: 0x7535_a033_c77e_0c00,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x44c8_bfe9_f98a_20fe, 0xce60_695e_c29e_96ff),
            (3000, 0xa864_4172_4156_5923, 0xdf6b_6b64_0a84_11f7),
            (4500, 0xdc85_7c08_9efb_5650, 0x7af0_5c3a_955f_66a7),
            (6000, 0xafee_6185_b6d5_8e13, 0x18d2_b55c_f07d_20fb),
            (7500, 0xc419_1c86_4610_cfa6, 0x9803_0b9c_469f_de49),
            (9000, 0xb64f_bece_2098_11e7, 0xba18_2f88_bdac_fc12),
            (10500, 0xab39_f7b2_8be3_5270, 0x612f_023c_d33f_1987),
        ],
        last: 0xad7d_f859_3598_9dc0,
    },
];
