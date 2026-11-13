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
    /// Digest of the final tick, world, serialized stream, winner and match statistics.
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
        (1500, 0xdfa1_9deb_73d6_224f),
        (3000, 0x076c_d2d5_75d7_0eca),
        (4500, 0x0643_53c1_c1cb_713c),
        (6000, 0x1313_31bd_5b4f_9955),
    ],
    &[
        (1500, 0x96bc_a62d_d937_bc6f),
        (3000, 0x1b29_d671_1b0a_bad9),
        (4500, 0x64c9_337c_a2bf_4a48),
        (6000, 0xb00a_b3ff_cf87_298d),
    ],
    &[
        (1500, 0x02e9_b12b_db4b_eb93),
        (3000, 0xf000_da27_c9c9_f0d7),
        (4500, 0x528f_19b5_5782_2060),
        (6000, 0x3a26_290d_5145_21f7),
    ],
    &[
        (1500, 0x08fa_2c68_c091_5175),
        (3000, 0xd1fb_3984_10f8_28fa),
        (4500, 0x1346_dbf5_efaa_9a09),
        (6000, 0xc072_142f_43c3_be2b),
    ],
    &[
        (1500, 0x54cf_a900_29f5_2273),
        (3000, 0xcf5a_9eda_dfff_c2b9),
        (4500, 0x65fd_13be_d632_f71c),
        (6000, 0x8ef1_59a4_49d9_d5fe),
    ],
    &[
        (1500, 0xe334_68e4_2459_b01e),
        (3000, 0x2d72_0c23_0d70_e6d2),
        (4500, 0xe95b_1977_f634_69a7),
        (6000, 0x2d71_927f_a67f_fc11),
    ],
    &[
        (1500, 0x7b7e_523a_2d74_c8db),
        (3000, 0x6c55_4378_39a0_c6c3),
        (4500, 0x4389_7b29_a6a7_2807),
        (6000, 0x0d7a_8fee_a4d0_ea75),
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
            (1500, 0xdd75_12d5_70ad_350b, 0x32f1_f001_be60_bdea),
            (3000, 0x585a_7309_39dc_1325, 0xb14e_f49a_9354_cb06),
            (4500, 0x68cc_6d65_3acd_74e8, 0x836f_10de_51e8_30c9),
            (6000, 0x83e0_7ed6_1985_b085, 0x7299_de1e_9c9e_d22d),
            (7500, 0x8adc_0bf0_0922_e962, 0xc37b_6579_f225_e6a5),
            (9000, 0x3ba5_3d88_7db6_521f, 0x9e1d_f5ad_8eec_1288),
            (10500, 0xce26_a876_54a2_2613, 0xf32f_d781_3dec_59e2),
            (12000, 0xadb0_6a9c_7d33_ffc4, 0x2fa1_1c5a_d591_693a),
            (13500, 0x8ddf_40a3_0a18_2e61, 0x4411_1716_7f2f_6c9c),
            (15000, 0xd601_c669_dc5b_51ad, 0x4708_1bd0_d615_2a96),
            (16500, 0x4895_b63e_cce8_3fc3, 0x250b_6f36_2417_1bf2),
            (18000, 0x630c_7bc0_2a83_9cfa, 0xf021_ef14_a8d2_62cd),
        ],
        last: 0x90de_49e2_d588_9bb2,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x35cb_a16a_03ad_9ee2, 0x5c1f_4368_0380_2d39),
            (3000, 0x1067_c76a_47c4_b5d8, 0x0817_539f_89d8_acb2),
            (4500, 0xa6dd_337b_ee1a_86a1, 0x7a23_0a94_dc38_d68f),
            (6000, 0x1f9b_71bd_260a_f3d5, 0xf289_2989_af4a_af18),
            (7500, 0x05d7_74f5_0a0d_561f, 0xbf75_b4e6_ccd8_75f0),
            (9000, 0x1131_7284_ec27_571c, 0x18c2_6655_98bd_1b04),
            (10500, 0x6fa3_5378_8559_1c5f, 0x1ab6_81f7_0c0a_8ed8),
            (12000, 0x1d3c_2b28_c593_2da8, 0xb7ff_0d57_25ac_87db),
            (13500, 0xc616_bb07_8a13_55d1, 0x9d3b_22be_22de_e75b),
            (15000, 0x5fb9_f6e1_b15a_9017, 0x239e_6a35_40b0_7d4e),
            (16500, 0x2bfa_f7d7_77b6_9591, 0x789e_4d80_872c_bf13),
            (18000, 0xa278_1b7c_9ebe_27c9, 0xa278_7d54_f29d_13c8),
        ],
        last: 0x4101_a1c4_7457_f624,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x7da6_cb00_877d_8750, 0xb5db_24cf_bf4c_f7ea),
            (3000, 0x5d7a_7c2a_1cff_7fb2, 0x1ed6_f374_5e99_f62c),
            (4500, 0x6202_97f4_b1ce_2073, 0xca51_7f56_adb6_65d1),
            (6000, 0x4bb5_1053_2e23_a126, 0x525f_caf5_7970_be88),
            (7500, 0x7938_1af8_65db_939a, 0xeefb_eac6_3c04_e93a),
            (9000, 0x300a_5a43_8882_94d0, 0x2b94_9966_f0bd_3249),
            (10500, 0x9da9_667d_edcd_883a, 0x33aa_bb63_8d4c_4b57),
            (12000, 0x5fb2_fe03_d205_ec0e, 0x88b7_75ca_63ab_8725),
            (13500, 0x73ab_b019_d3b9_6486, 0x68f4_4929_50d1_5040),
            (15000, 0x0f97_9d02_29d0_68b5, 0xea81_439b_dc5f_5711),
            (16500, 0x1eb4_e52a_e441_2ecc, 0x1762_23b0_20d4_b0ee),
            (18000, 0x256e_bc34_5b15_9fbb, 0x2183_48c9_46f1_0faa),
        ],
        last: 0xd3e7_9c65_3981_42ab,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x4775_93f1_0658_c9a5, 0x2772_4732_3227_8ec8),
            (3000, 0xd192_d6ed_0b56_dff1, 0xc3a7_5d41_c48a_d436),
            (4500, 0x2548_1e93_02d9_dd18, 0x1da7_d983_7540_82e0),
            (6000, 0xc2aa_6e17_4c69_ab0c, 0x6f46_c348_a161_2458),
            (7500, 0x4570_d244_b62c_9a09, 0xa3cf_1e2c_d850_4f4d),
            (9000, 0x3401_60c4_8d85_b7ec, 0x3504_b9ce_8221_608d),
            (10500, 0x74c9_9273_61d7_9a6a, 0x8333_3236_7970_3964),
            (12000, 0x4d1d_1c5c_6598_2ea1, 0x26c4_ee4e_e4da_3dd4),
            (13500, 0x7ef4_287d_ecd1_e42b, 0xad88_6129_49dd_0bee),
            (15000, 0xcbd3_438f_756f_87a3, 0x3750_e411_e2d5_795c),
            (16500, 0xbc35_7495_11cb_04b7, 0x9d6d_10d0_3775_8cca),
            (18000, 0xd2b4_6bc2_46be_c310, 0xd2a4_8384_4835_3946),
        ],
        last: 0x62ab_299c_b0e0_c2ac,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xa99d_121e_6dd5_1bb8, 0x9ff0_d7e6_fe84_815c),
            (3000, 0xc2e9_733f_dc61_fc07, 0x82f2_bc48_63f9_2fc4),
            (4500, 0xa811_5a1d_03f1_c305, 0xd230_4fd3_8bbb_abdb),
            (6000, 0xa754_21f1_ac9c_aa5f, 0xe162_d749_9a3c_2937),
            (7500, 0x38a5_b9b6_2892_49c6, 0x0d84_30d7_0b43_c591),
            (9000, 0xc50f_2682_7e99_4449, 0xe429_6d62_7326_ff33),
        ],
        last: 0x96eb_2d5a_4764_41cf,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x1372_cb85_da1c_4600, 0xa2db_331b_8f5a_481b),
            (3000, 0x29d3_2ef6_43cc_f90e, 0x50c5_7691_efd9_1159),
            (4500, 0x6cd7_0ea7_1f32_a3c9, 0x37a5_21d9_ddd4_8ab7),
            (6000, 0xb1f1_9fcb_c9cb_558b, 0xb556_65c2_aa03_689a),
            (7500, 0xdcd3_c69b_0356_a4d3, 0x0631_96a8_7bf6_ea08),
            (9000, 0x7a49_3613_9566_2098, 0x3601_0881_4515_d1c2),
            (10500, 0x11a9_ad4c_0ff9_8210, 0xc46c_cff2_dd17_cca4),
            (12000, 0xa430_4c44_f052_cd34, 0xfa87_466d_89b9_794a),
            (13500, 0x5765_1a8a_1a9b_31dc, 0x4f12_dabf_a900_720b),
        ],
        last: 0x0057_f171_57ba_bd9a,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7b92_10f5_f130_d500, 0xcb40_19da_c2c2_05bf),
            (3000, 0x9d4f_1ae1_3770_3e4b, 0xb0fd_d696_90c9_213e),
            (4500, 0x9d75_b907_ec13_1422, 0x10e6_9da8_fe52_64a3),
            (6000, 0xf3d4_6f94_ac53_e5a1, 0xc1b3_f9a0_40ea_8708),
            (7500, 0x6b3e_c873_324e_db5b, 0xaf2d_a4c8_2b10_ec44),
            (9000, 0xfb7e_aa3b_637a_e745, 0x96d6_ebec_1543_1b97),
        ],
        last: 0xfc15_839a_02ad_6003,
    },
];
