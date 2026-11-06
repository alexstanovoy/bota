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
    while world.tick < FORK_TICKS && world.victor().is_none() {
        if world.tick == FORK_TICKS / 2 {
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
                events.iter().map(|event| event.kind()).collect::<Vec<_>>()
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
        (1500, 0xa9cf_c186_ec51_5796),
        (3000, 0xda32_f104_83a1_4002),
        (4500, 0x892f_24bf_0d70_cd69),
        (6000, 0x8e2b_4af0_2053_faa8),
    ],
    &[
        (1500, 0x882c_a577_ddb1_b657),
        (3000, 0x53ca_5adc_e0b3_f1cc),
        (4500, 0xc257_aaec_4421_fe19),
        (6000, 0x1c4e_74a0_8c50_4df9),
    ],
    &[
        (1500, 0x87ff_8cb6_5f3c_a16b),
        (3000, 0xd7cc_d857_7f8c_4a0e),
        (4500, 0x002d_6930_5790_71ef),
        (6000, 0x8890_7d36_26e2_5a8f),
    ],
    &[
        (1500, 0xe518_19e8_aad9_56e1),
        (3000, 0x9e3e_e508_0266_7624),
        (4500, 0x51e6_6e6f_c252_4df9),
        (6000, 0xa286_bc9c_9793_58e7),
    ],
    &[
        (1500, 0x2a35_30d4_2cac_4169),
        (3000, 0xd47a_f71c_720e_55a4),
        (4500, 0xfe09_5f7b_e83f_1a3e),
        (6000, 0x2d54_1043_cacf_99f4),
    ],
    &[
        (1500, 0x35b7_aba7_739a_276a),
        (3000, 0x731f_9ee8_e4dc_9df7),
        (4500, 0x9a2a_7ab9_8355_9067),
        (6000, 0x052b_3367_1a4d_5c6e),
    ],
    &[
        (1500, 0xb5fd_6231_4a33_9136),
        (3000, 0xe11c_5ca5_26ff_6ff5),
        (4500, 0xb2ea_ea77_3909_ce24),
        (6000, 0x8458_31da_35f0_6e11),
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
            (1500, 0x4573_3963_713e_90ba, 0xf5ce_efa7_76d5_ff54),
            (3000, 0x37cb_d923_4837_73a9, 0xbd25_8c49_ee73_f86f),
            (4500, 0x9f5c_f9e7_ac90_39b9, 0x93df_302f_4cf0_e94c),
            (6000, 0xa9ee_3c04_257d_8eeb, 0xbe5e_57b5_1cda_46e1),
            (7500, 0xb70c_e27c_8c8a_af81, 0x0eed_3fe5_16d6_a2e6),
            (9000, 0x7fc5_6bcf_740e_408e, 0x8d53_fa57_1bc1_8570),
            (10500, 0x1304_5288_e7aa_76ba, 0x4da6_ba1a_89e1_312c),
            (12000, 0x9952_0541_e809_7704, 0x53a7_01f5_09bb_b612),
            (13500, 0xdd0b_27bd_1e2a_359f, 0x9bfa_1cec_c6e0_26a2),
            (15000, 0xbcad_5b0e_6909_693c, 0xf619_c5f6_c720_5dc1),
            (16500, 0x5e55_ce3b_7c99_0842, 0xa53f_311c_5f4e_a3db),
            (18000, 0x62b3_1e28_9e2a_3212, 0xe732_906a_ac30_e3d5),
        ],
        last: 0x7151_892c_76b6_817a,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xc8ce_da6a_df4c_a321, 0x19dc_97f0_82fc_1af7),
            (3000, 0x5589_3f64_0219_543b, 0x5a79_9719_48aa_e5a3),
            (4500, 0xf378_9cf7_a452_4149, 0x0c51_6ad5_959f_e6e2),
            (6000, 0xddf7_5c83_1d45_1745, 0x95bf_4090_3da4_d8b7),
            (7500, 0x6b1d_7e21_8efe_1d1a, 0x73b2_cc8a_6d33_0ce6),
            (9000, 0x7376_f142_5970_d3c2, 0x679b_69a9_1539_64d7),
            (10500, 0x1ead_bb10_70b0_ae20, 0x1372_1b9d_8e03_7267),
            (12000, 0x27af_a687_c228_d1a6, 0x223f_f284_8fe9_c728),
            (13500, 0x47cd_ddcb_d9af_a3e4, 0x574d_ff46_2899_11d0),
            (15000, 0x90ca_f938_53a7_5928, 0xd5fa_ee16_ecf9_d122),
            (16500, 0xd480_73ef_7bb9_b609, 0x9369_ee14_26b6_56d7),
            (18000, 0xbe93_ab02_a62b_dc28, 0x3257_ba3f_5f7a_0ad5),
        ],
        last: 0x3d9e_67d6_cb0f_2343,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xab0d_18a9_c2d5_608e, 0x99f6_c452_a934_2857),
            (3000, 0xb016_a6ae_5ed5_23da, 0x72a3_7b76_293c_5371),
            (4500, 0xc853_91f7_f315_acb7, 0x97b2_3e26_e764_d1ed),
            (6000, 0x5002_179c_525a_9c3b, 0x8d06_6cfb_0dc8_2479),
            (7500, 0x7691_c77b_e37a_8117, 0x90a5_c059_a049_80b0),
            (9000, 0x4a3c_7a61_8c82_24a4, 0x2f74_c574_dc5e_01c1),
            (10500, 0x170a_0c0a_775f_dde2, 0x6dcb_1cfe_93eb_86ab),
            (12000, 0x6fda_6962_5b17_dda4, 0xab12_16ab_bf79_cbc4),
            (13500, 0xee6f_bba4_a38c_6b2e, 0x62f1_76d5_2ad1_921c),
            (15000, 0xcf16_b28d_a5b5_88c5, 0xf670_9904_b926_e015),
            (16500, 0x1dcd_5668_d4f5_e723, 0x16a2_fe06_5955_b534),
            (18000, 0xef89_afe9_e3aa_2cc3, 0xa43d_2af3_3084_c4f8),
        ],
        last: 0x8bd7_1f0c_d00d_6a04,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2d8c_88d7_dde6_4af6, 0x35d8_55da_5595_cf6f),
            (3000, 0x3fd8_e914_5d53_481e, 0x0ddc_25af_6901_6d6d),
            (4500, 0xf514_59ac_1e5f_97bf, 0x064d_c2d1_7787_f920),
            (6000, 0xf9f7_f39f_f0ca_7f4d, 0xc64a_f8d5_4bc8_f7fa),
            (7500, 0x929b_4c32_e1f5_2a72, 0x0065_f90a_2e25_c278),
            (9000, 0xd74d_3e09_5534_f2f3, 0xdd84_bb64_2a3e_d8e9),
            (10500, 0x9563_4784_93eb_aba4, 0x8fb9_b454_f792_595b),
            (12000, 0x502c_2a7d_15e4_9a93, 0x193b_5827_c862_5a35),
            (13500, 0x6268_3aca_8e92_e1b2, 0xe098_5c00_1dc9_959a),
            (15000, 0x62e3_5cee_64d0_bca6, 0x416d_1134_42fb_bc7d),
            (16500, 0x1af7_a79b_9249_a780, 0xcf6b_b614_3cca_869c),
            (18000, 0x18bc_3e02_e37d_30f3, 0x418d_409f_e54e_9ec0),
        ],
        last: 0xbdf6_f546_da2a_7844,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x97b1_5dfb_8cba_e449, 0x1deb_4988_62cf_fbc7),
            (3000, 0x1561_81b2_85af_732c, 0x517c_f598_4625_7b94),
            (4500, 0xa816_c4fa_ed31_807d, 0x73fb_7036_7bef_2008),
            (6000, 0x52d2_cb9c_5fac_ed23, 0xaa60_24e9_b604_8a71),
            (7500, 0x586e_e10d_0f42_2e40, 0x16cf_5d42_15b8_23b3),
            (9000, 0xead9_6e5d_5506_2956, 0x8951_be39_2119_1504),
        ],
        last: 0x903f_e94b_72ed_e7d7,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x24b5_708d_8408_fa6f, 0x3095_f779_6eb2_34e1),
            (3000, 0x54f6_f37c_133c_8c11, 0x77dc_0a91_c846_4938),
            (4500, 0xf7c3_bce7_f9ef_8d5c, 0x7dec_9af2_88f9_f20f),
            (6000, 0xebf3_8bbb_c1ee_fd52, 0xac2f_b851_6522_250b),
            (7500, 0xb69d_b220_7514_9cd9, 0x625f_155f_d9e8_7792),
            (9000, 0xdb51_dc64_1461_f8fe, 0x6d4d_776e_be3f_a6b7),
            (10500, 0x38b9_a176_bfc9_9ab3, 0x8ceb_3e87_94ad_1235),
            (12000, 0x0ed7_a710_6225_09fb, 0x17c3_a6ca_c58a_076f),
        ],
        last: 0x5e8d_53ca_00fc_643e,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6785_5dec_e89a_d6db, 0xfdc8_e592_5f8f_ee6d),
            (3000, 0x5d3e_910b_d848_fa9d, 0xbb0e_7edf_e144_6b42),
            (4500, 0xd5be_65a4_4a7d_9bc5, 0x1c2b_c685_bb8a_624e),
            (6000, 0xbd96_4b3f_1074_1f85, 0x1001_cb74_e7ab_d292),
            (7500, 0x5dd1_74f1_4454_dae2, 0xb167_2b48_2024_0269),
            (9000, 0x9fd5_e114_c21e_8b50, 0xa019_6108_9827_f6b9),
            (10500, 0xaff7_22e2_5671_75e9, 0xd95b_09f4_f470_f96c),
        ],
        last: 0x5f80_5bf1_f496_9e77,
    },
];
