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
        (1500, 0x779d_f9b9_31df_5e26),
        (3000, 0xb11c_a917_cfa9_79a8),
        (4500, 0xc00d_d0bb_900f_9e59),
        (6000, 0xe904_d651_1fdb_6f9f),
    ],
    &[
        (1500, 0xad94_75d7_f898_09cc),
        (3000, 0xc782_6742_b548_42e5),
        (4500, 0x683a_d4f4_aace_84e4),
        (6000, 0x7e37_130d_28ad_980d),
    ],
    &[
        (1500, 0xc0e7_c3f9_47c2_d23f),
        (3000, 0x6528_a559_049a_57b8),
        (4500, 0xa08f_46ab_d902_2606),
        (6000, 0x9658_5ece_4f10_1a6a),
    ],
    &[
        (1500, 0xea31_a5b6_481b_a545),
        (3000, 0x3b59_68c6_a4d8_2b5e),
        (4500, 0xec8e_cebc_5fdb_fb94),
        (6000, 0x5497_f4ef_bf40_d914),
    ],
    &[
        (1500, 0x7e77_3757_f5e9_c2b6),
        (3000, 0x0263_036d_0ff5_6724),
        (4500, 0x0c48_8575_bbee_5972),
        (6000, 0x46c3_bfca_a780_f2a3),
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
            (3000, 0xb645_96d2_af14_fa46, 0x43b1_9718_dce5_54e4),
            (4500, 0x0648_0d95_2ced_bf6a, 0xf990_802c_bbf3_861c),
            (6000, 0xe653_429d_95cf_e85e, 0xeedb_5673_ec6e_26b0),
            (7500, 0xb9fa_73e9_c192_0e7b, 0x7f30_c348_1cb9_4b37),
            (9000, 0x6b45_37cc_17d9_9a02, 0x4000_5742_37f6_a848),
            (10500, 0xdad9_754d_64c7_f589, 0x70ec_240c_4d5e_dfdc),
            (12000, 0x8276_7191_bcdc_261c, 0xf3f5_5b95_74b9_4d10),
            (13500, 0x1d4a_9f07_cb55_45c2, 0x6eca_798d_c9b4_9cfe),
            (15000, 0x9230_1979_b860_1fa5, 0x8113_fd4f_92f9_3791),
            (16500, 0xe347_ed43_9aa5_209f, 0x11ab_cbe4_3ae9_c318),
            (18000, 0xa68d_fd4a_b354_f4b0, 0xb0df_5a2b_313b_fad2),
        ],
        last: 0x84fe_9bf4_5941_28c9,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x837d_305e_2bda_8544, 0x1d6b_ab44_4094_a675),
            (3000, 0x4e82_6dc4_417b_05c5, 0xe857_18c7_f72c_00b7),
            (4500, 0x6431_435c_71b2_86bb, 0x175e_b221_cc4d_986f),
            (6000, 0x86c5_faad_5f3c_45b8, 0x2bdc_28ed_692f_0342),
            (7500, 0xa9ad_d13a_f7cb_eb87, 0xa391_c96e_ce7a_e95a),
            (9000, 0x0d25_c713_11ca_c936, 0x6ded_9cdd_76e9_3761),
            (10500, 0x6cde_db85_2055_744c, 0x2b07_a968_ea4f_abbc),
            (12000, 0x1a27_3027_8dad_1723, 0xef3e_4959_4651_8535),
            (13500, 0x9506_f120_fec8_1b55, 0x255c_c4e1_f236_bcc4),
            (15000, 0x8481_be9e_5add_eb64, 0x4ccb_f080_8f30_899f),
            (16500, 0x1c43_e4bb_170b_3cd8, 0x9de9_a430_71c2_0bf5),
            (18000, 0xc1fc_8055_d4f7_5f59, 0x72f8_8cb1_2e75_ec48),
        ],
        last: 0x07c1_a3ff_7a49_4264,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x245a_17c3_69a1_059a, 0x8a5f_e999_1688_0820),
            (3000, 0xa41c_cc75_388d_f679, 0xd43b_d976_b7c1_b306),
            (4500, 0x20f5_9c58_16f9_babf, 0xb93b_7c10_018b_85d0),
            (6000, 0x1899_bc4d_2422_b058, 0xaf33_f57a_e9f4_83aa),
            (7500, 0xacff_3bc9_f26d_da86, 0x44e7_7c1a_0325_42f1),
            (9000, 0x226d_9779_e7cf_fdb5, 0x9ffa_41a8_ce7b_88bf),
            (10500, 0x78f7_82ad_29a9_2c18, 0x8e71_772b_2019_8c19),
            (12000, 0x9f1c_f9f1_2247_bf62, 0x4846_836d_abc5_8e31),
            (13500, 0x1026_5dbf_8ec0_65e9, 0x54e3_bdd1_3b90_40b7),
            (15000, 0xf563_d4f6_f604_bcd9, 0xd8ca_392b_05c3_9428),
            (16500, 0x34fb_c506_4d7c_a630, 0x1a4a_1ba4_c203_5710),
            (18000, 0x4fcf_1375_fd56_6111, 0x33db_ce23_5d00_e1c6),
        ],
        last: 0xb994_be9e_dbb7_8e36,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x79e9_de9e_eb1a_478b, 0x1e01_b682_db01_fb05),
            (3000, 0x15cf_02ee_85a8_10c5, 0x0c23_64fb_3cda_3deb),
            (4500, 0x4f38_ed7f_4e09_ba6d, 0x61c8_b3db_014e_6962),
            (6000, 0xef34_df6f_232d_fa00, 0x99a0_46f2_d04a_44b6),
            (7500, 0x074b_f32c_2c0d_f945, 0x0d53_2ce1_d736_233c),
            (9000, 0xfd45_b22e_4ccc_56c7, 0x7e86_2331_79c1_ac78),
            (10500, 0x24f2_ac67_4aff_c820, 0x709b_4a06_b868_4701),
            (12000, 0x2885_a327_57f5_9fef, 0xff31_8d96_251e_0501),
            (13500, 0x3f1d_0d5f_f3bc_72bf, 0xd84c_3a9c_012f_ddd8),
            (15000, 0xace3_8054_3bd6_bbe9, 0xd760_8b31_4095_3084),
            (16500, 0x00bb_2a93_c66d_ca16, 0xa5b8_ec96_28dd_7270),
            (18000, 0xd96a_b792_eb96_e490, 0x970b_d932_d160_75ec),
        ],
        last: 0x22ee_28a2_068d_dd7d,
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
            (1500, 0xf5b7_9e15_b098_0f90, 0x5a43_cfc1_4b4e_9904),
            (3000, 0x5b6c_283d_0aeb_b402, 0xcade_589c_f10c_9041),
            (4500, 0xf72d_310b_a838_5219, 0xa2f9_ed19_574d_382f),
            (6000, 0x91fc_8611_8049_aa29, 0xbfbb_69da_d38e_9322),
            (7500, 0x4fd6_b97e_5c07_da8f, 0xca45_4e9b_f2fd_d019),
            (9000, 0x3158_2f07_6e5e_fb8e, 0x69f3_22e0_f79a_bff1),
            (10500, 0x8664_f033_fc2b_07d6, 0x4f98_786a_56ab_5748),
            (12000, 0xbc75_c3a0_c0b6_6eb4, 0x3097_c496_f18f_b7fa),
            (13500, 0xd76e_8c1d_625a_9d03, 0x0d5c_3e93_3dfb_7a8b),
        ],
        last: 0x313e_beaf_78b1_46b9,
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
