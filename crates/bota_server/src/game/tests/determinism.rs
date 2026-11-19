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
        (1500, 0xc04a_293f_24e1_c132),
        (3000, 0x949e_a800_91ce_31c3),
        (4500, 0x26ae_95b5_7c0d_d270),
        (6000, 0x5295_ee8b_0587_2f30),
    ],
    &[
        (1500, 0xff02_a1d5_82b3_9d6a),
        (3000, 0x8bf7_0f17_fa9c_8057),
        (4500, 0xb0dd_5aae_e3e3_1dc0),
        (6000, 0x0997_433e_bbd2_3980),
    ],
    &[
        (1500, 0x5960_3e12_c5ab_b4ec),
        (3000, 0xe7e9_75e0_3133_ade1),
        (4500, 0xd92a_aa74_fe3f_1c6a),
        (6000, 0xf258_8279_b967_b1c4),
    ],
    &[
        (1500, 0xcc37_79e0_f034_6cee),
        (3000, 0x43db_b2d5_3882_ef8e),
        (4500, 0x3047_2f32_713f_1ee0),
        (6000, 0xda2c_9881_74f4_243d),
    ],
    &[
        (1500, 0xbb90_78dd_b2a3_5cc9),
        (3000, 0xdace_d50a_b280_9049),
        (4500, 0x40f8_b1cc_9882_5751),
        (6000, 0x1be6_8f60_b322_3a7f),
    ],
    &[
        (1500, 0xfca7_36a7_fae9_71cc),
        (3000, 0x64de_b382_062d_4084),
        (4500, 0xb52d_2864_42e5_bcc1),
        (6000, 0x5809_8fbb_0e3a_b566),
    ],
    &[
        (1500, 0x3806_7800_eda2_d079),
        (3000, 0xa3a2_90a8_a380_bd26),
        (4500, 0xd839_d78a_459c_08b5),
        (6000, 0xbadb_82da_ff42_9e29),
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
            (1500, 0x2c5d_361b_78b3_e64e, 0xb4b0_86f1_e113_f3c6),
            (3000, 0x4d74_9f4a_a0b6_7070, 0xf208_2023_4b88_4aae),
            (4500, 0x239f_805b_5322_aeb8, 0x660f_72c8_c83f_1035),
            (6000, 0xa973_7e6f_a9f6_64e5, 0xc628_f3c6_8cdb_f2d3),
            (7500, 0xc71e_3bbb_b228_9108, 0xeb89_784a_c7e0_d91b),
            (9000, 0xc5c0_aa58_11b8_ef52, 0xc298_da7c_d754_4c63),
            (10500, 0x0afd_9be9_b23c_d669, 0xcfe4_a308_f7a8_6f37),
            (12000, 0xabff_9f0c_d679_5d27, 0xb7e6_219b_17fe_4f19),
            (13500, 0xda98_ef98_7616_87f3, 0x1cf1_ad60_81f1_1fc7),
            (15000, 0x45ed_c0c8_3cc9_fe9e, 0xd53e_b260_3d60_53ae),
            (16500, 0x06b4_e461_73d8_0f16, 0x2240_2a8e_8be9_f31c),
            (18000, 0x9a12_ef40_3a81_5704, 0x957c_5c76_d866_62d3),
        ],
        last: 0xa6a6_f3ba_9a85_b483,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7b83_7436_e0fb_1d46, 0x3bf9_b0d0_6ebb_3728),
            (3000, 0xc893_6f8e_f1c0_4b09, 0xd3c3_12be_1199_a5b1),
            (4500, 0xf203_f885_38e8_05c1, 0xd1dd_0e41_66af_f8b7),
            (6000, 0x0cad_ae38_faf7_70fe, 0xdd58_4467_b27d_84c2),
            (7500, 0x7d40_f774_8957_b064, 0x33f3_0152_6749_2037),
            (9000, 0xdb9f_096d_9696_ce6e, 0x117d_26df_faeb_e125),
            (10500, 0x0e7a_2729_1744_4dd3, 0x39fd_da13_d0b9_dc62),
            (12000, 0x13c8_42b1_7553_cfa0, 0x64d3_6acd_3fec_6ecf),
            (13500, 0x2513_cf87_cd24_17b5, 0xb99a_0289_d5fd_f259),
            (15000, 0x3e63_91f6_6b0e_ae9d, 0x4c0c_d607_8524_2984),
            (16500, 0x5902_81d4_69ca_8f56, 0x0d7a_7630_2511_b69b),
            (18000, 0x365f_d6e6_df28_900c, 0xaf41_beae_704b_438f),
        ],
        last: 0x3cb8_1b30_2dcc_cdd0,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaf90_9f45_fa7f_5b62, 0x97ad_9b05_6819_2c81),
            (3000, 0x8cf2_a6a9_a90b_10d9, 0x23c8_a5a0_d742_b760),
            (4500, 0xb9a0_edcc_4d51_b2f9, 0x0dce_0a18_a98e_ed14),
            (6000, 0x8eac_572a_9068_bc11, 0xccc4_3621_e059_001c),
            (7500, 0x2517_86f5_d707_0efe, 0xdad8_2360_b115_e5ad),
            (9000, 0x7656_2ed3_0d57_10ec, 0x48b8_b033_2923_4039),
            (10500, 0xd355_0f32_0510_a4a1, 0x322c_17cb_3605_f25f),
            (12000, 0x682e_ea6d_c0df_5cd5, 0x2ef8_ef0b_b787_3993),
            (13500, 0x0e99_fc53_d8f0_d8ea, 0x5c01_0d9f_54ae_fdcc),
            (15000, 0x3c1a_eca3_1987_8729, 0x1bc7_d529_cf0e_4f43),
            (16500, 0x7123_795d_5cce_ae4f, 0x715a_f8e6_5acf_5618),
            (18000, 0x9cb1_0d3c_ea8a_8e82, 0x6842_3706_dd10_4af7),
        ],
        last: 0xe741_0f8c_1656_45ac,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2ba4_2dbe_47bb_e2a6, 0xcb5e_ab35_f697_c75e),
            (3000, 0x445f_4d29_c3d8_29d4, 0x3e89_12d4_ecdd_aee3),
            (4500, 0x6c40_bb0c_3761_032b, 0x26d4_51c2_c58f_d15e),
            (6000, 0x4dc8_f23d_ca35_0e96, 0x4d92_efed_ec73_799f),
            (7500, 0x91c3_32cd_e71f_d5c5, 0xe3ab_81a2_6cd3_5a10),
            (9000, 0xfe54_8e36_251a_3413, 0xa998_4d4c_7e34_ebfb),
            (10500, 0xa632_399b_3644_4cc0, 0x9e4f_8054_ae62_51aa),
            (12000, 0x6b0d_2725_0eec_fee6, 0xb026_cee2_ca84_8e82),
            (13500, 0x0e31_b41f_a6f9_1fa5, 0xb3b8_1698_155b_6d86),
            (15000, 0xe11c_51c7_1b7c_6903, 0xaa92_5004_2604_eca2),
            (16500, 0x2295_e1e7_af24_d3de, 0xf7a8_08a5_3d2b_15a0),
            (18000, 0x9f29_6eee_27e9_1dfb, 0xd9e5_c14b_f685_75e4),
        ],
        last: 0x49fd_1a09_78d3_dcb8,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x592b_4590_5ffe_4680, 0xc65f_ce66_d9ad_4b1d),
            (3000, 0xc353_b959_2169_a229, 0x7de0_6d16_1bd6_6c76),
            (4500, 0x6d95_c6a7_6b6e_0252, 0x14c5_bdee_5257_ae1e),
            (6000, 0x3385_4f1f_e0ed_9ea7, 0x0bfb_ef4d_df0e_bd46),
            (7500, 0x6a00_9418_2399_7a57, 0x5287_2884_0de9_8882),
            (9000, 0x0f7d_6fb6_a765_5031, 0xa970_e3b5_c8fd_cb0d),
        ],
        last: 0x1e9f_aac7_df91_b022,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5fbb_2231_fd27_7eaa, 0xfd1e_b3e0_110f_9291),
            (3000, 0xa114_d57c_4158_62e9, 0xc999_e710_c94c_d337),
            (4500, 0xb1a9_dd79_548e_688f, 0xc3f1_9454_8484_72e4),
            (6000, 0x2691_8c17_1db0_df03, 0xec4d_6977_b243_4f99),
            (7500, 0x370f_35aa_a99c_f717, 0xb32f_4e86_a993_aaeb),
            (9000, 0x2a6b_9e8a_1f03_f3e3, 0x84c8_8fac_a5cc_5b17),
            (10500, 0x8f2b_ce01_4a29_20fb, 0xbc13_a609_fb1b_00be),
            (12000, 0x955a_51fd_d8e7_bd08, 0xab58_8eb1_05e8_849f),
        ],
        last: 0x04d1_4ab1_cd1a_f02d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x1431_8d9c_3625_3052, 0x3444_ca48_4b2a_d867),
            (3000, 0xe2d1_d987_b484_7606, 0x46c6_093c_1853_a125),
            (4500, 0x998b_d119_0cc6_6ca3, 0x0aed_8053_d2c7_c6ba),
            (6000, 0x9038_c3e4_09a3_fbcd, 0x7c6d_7a8e_3441_ad0c),
            (7500, 0x0975_497f_4b72_1752, 0x33c8_9dd5_3bad_1814),
        ],
        last: 0xb00f_6368_3277_de47,
    },
];
