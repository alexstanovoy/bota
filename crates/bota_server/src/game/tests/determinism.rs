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
            (3000, 0xb645_96d2_af14_fa46, 0x80d1_402c_79c1_459a),
            (4500, 0x0648_0d95_2ced_bf6a, 0x5a7e_31ff_a02c_29da),
            (6000, 0xe653_429d_95cf_e85e, 0x4540_25c7_2088_88da),
            (7500, 0xb9fa_73e9_c192_0e7b, 0x38dc_cb2d_2314_8a71),
            (9000, 0x6b45_37cc_17d9_9a02, 0x9c38_7454_4f3e_3142),
            (10500, 0x3613_ac43_4bdd_6792, 0x8f48_4501_edd2_d10b),
            (12000, 0x378a_689f_ea97_2472, 0x7372_cca3_476e_a645),
            (13500, 0x9042_92d9_2376_f64b, 0x633b_027a_fc6c_b882),
            (15000, 0x081c_c822_9229_0a0c, 0xca08_dc83_fc02_5416),
            (16500, 0x89e9_0d5e_5af7_6e9f, 0xe5d7_ecef_0f67_66a5),
            (18000, 0x3bf4_db1c_0190_fcaa, 0xb48c_df2f_73ff_0122),
        ],
        last: 0xe2d1_a29b_7c2c_9d94,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe1ce_6693_0ac8_a396, 0x2ea9_730f_f4b1_5fa2),
            (3000, 0x9037_9bf5_4c12_e0c7, 0xde56_7e2e_dd3a_d3b6),
            (4500, 0x6179_be9c_0390_79fe, 0x16a1_38e2_a454_12f9),
            (6000, 0x5ca0_d139_4869_06e8, 0xaecd_b113_8e10_7138),
            (7500, 0x98bc_ece8_963e_e4cc, 0x26df_127e_a731_7fd9),
            (9000, 0x710b_8e02_b682_ed4e, 0x2ae1_90ff_6beb_9d88),
            (10500, 0xd847_5c3c_6a01_717d, 0x91e1_06d3_256c_030a),
            (12000, 0xa507_4dac_96eb_9015, 0x979a_8eee_ff7a_692e),
            (13500, 0x0a4b_0d97_1b14_b50f, 0x66de_f1bc_3eae_2e76),
            (15000, 0xaccd_3724_645b_bd22, 0x6977_cac3_b0a4_75b8),
            (16500, 0x2589_b12e_4c69_2748, 0x743b_7773_6300_56e1),
            (18000, 0x00e4_1ae2_304f_8bf2, 0x5daf_58cf_9f30_cb5e),
        ],
        last: 0xb963_2e6e_0ec4_db40,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x4bf1_ecdb_e523_2f1b, 0x70d1_f09a_4c5e_afaa),
            (3000, 0xb905_6efa_0f7b_0e85, 0x1ae6_d89c_cd27_bfcc),
            (4500, 0x33fe_590f_36c8_d3a3, 0xd6b5_d147_0c67_e996),
            (6000, 0x88fc_c3f6_394b_a810, 0x7939_dbb4_3b4c_332b),
            (7500, 0x8a88_abcd_9fcc_ccc7, 0x16af_9637_f8e3_277a),
            (9000, 0xfe86_9599_3b00_17f5, 0xb9af_cafc_3422_658c),
            (10500, 0x7c2a_0e81_98e6_956c, 0x1909_347d_9975_1bb8),
            (12000, 0x8a7a_16be_efbd_3967, 0xdfbc_9b7c_1219_ef50),
            (13500, 0x7307_5871_a72b_f0a7, 0xd5fb_8cb6_4b17_c3e8),
            (15000, 0x8033_f861_8cbc_0013, 0x3d45_394b_9176_04eb),
            (16500, 0x5445_9f36_b7a0_013e, 0xd4c9_d04f_dc0a_0194),
            (18000, 0xbeb9_ce71_36c3_d60d, 0xd41e_5672_0236_59e5),
        ],
        last: 0xda74_3e7c_2021_edcd,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x79e9_de9e_eb1a_478b, 0xad59_4807_4f40_30cc),
            (3000, 0x15cf_02ee_85a8_10c5, 0xdda1_2d20_eb47_f158),
            (4500, 0x4f38_ed7f_4e09_ba6d, 0x8d5b_26f1_f0bf_08b3),
            (6000, 0xa616_591b_df5a_4844, 0xb63a_4c3b_ed03_096d),
            (7500, 0xc9c8_1a19_9366_4ca7, 0xd6d1_08f5_17fa_0e31),
            (9000, 0x9c9f_c987_7f86_f5f8, 0x80f4_efa3_1179_ca95),
            (10500, 0xde3a_0bec_b086_a860, 0x0ea9_49de_f2e8_71f4),
            (12000, 0x1077_6614_2820_9fd2, 0xb4ce_3715_8a2d_35e2),
            (13500, 0x97ec_a0cb_66d9_0e72, 0xfd7d_2259_581a_4ba5),
            (15000, 0xba64_2a50_5e6b_5af4, 0x8cf8_16c2_6e8c_f10b),
            (16500, 0x3d9b_86fe_0489_94db, 0x33d5_10bd_e7d6_9fca),
            (18000, 0xc886_3781_f80c_ddf9, 0xecf8_d78e_c955_ed72),
        ],
        last: 0x65e4_45cf_ac13_aa91,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x64a4_8e5c_2731_982c, 0xefd3_dc2a_df06_f98a),
            (3000, 0xda43_9769_5f19_f5e5, 0x0991_42c7_5f3e_8429),
            (4500, 0x0332_0085_a641_03c9, 0xcd8b_bfc0_ed0f_bd1f),
            (6000, 0xebf1_123c_4283_2e6b, 0x0006_e2d7_8557_b17d),
            (7500, 0x0114_8867_e658_85c3, 0x721f_5c6f_001f_da0f),
            (9000, 0xc9b2_a646_2da5_95b7, 0x1a80_ad40_55f8_4fb0),
        ],
        last: 0x56b6_1643_83f4_1cfd,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x33f1_e541_fa36_2d71, 0xe236_7c80_bc14_887e),
            (3000, 0xe01e_2aa0_10ae_b06b, 0xf48b_99c8_e133_1afc),
            (4500, 0xb9f5_aecc_72e3_4af4, 0x2dcf_4669_1965_237b),
            (6000, 0xdcd9_5673_2df5_c717, 0xec3c_d931_bef7_4f45),
            (7500, 0x79e4_4bde_aa28_2cc8, 0xc821_c730_13e4_f1a1),
            (9000, 0xc9d4_1918_8500_c60d, 0x625e_78bc_85bd_f8ba),
            (10500, 0x3f78_1a90_63f9_a903, 0x19c6_0412_dc19_e52d),
            (12000, 0x30a1_0d22_a3bf_08e7, 0x1ceb_0891_d845_4012),
        ],
        last: 0x3671_a4d9_20ba_81de,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x44c8_bfe9_f98a_20fe, 0xedfd_a251_385c_c533),
            (3000, 0xa864_4172_4156_5923, 0xb73f_631e_b17a_2189),
            (4500, 0xdc85_7c08_9efb_5650, 0x614a_075b_0f79_5a27),
            (6000, 0xafee_6185_b6d5_8e13, 0xea00_7de9_1c23_486a),
            (7500, 0xc419_1c86_4610_cfa6, 0xe019_7943_692f_ebc0),
            (9000, 0xb64f_bece_2098_11e7, 0xf40e_05ee_431f_0bc8),
            (10500, 0xab39_f7b2_8be3_5270, 0x3c0d_fbef_dce7_3769),
        ],
        last: 0x27e8_6890_3b8c_29c1,
    },
];
