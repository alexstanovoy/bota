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
        (3000, 0xe099_19e3_333f_3f62),
        (4500, 0xfaa8_05c1_34e5_f320),
        (6000, 0xe84f_a402_cf9b_d71e),
    ],
    &[
        (1500, 0xff02_a1d5_82b3_9d6a),
        (3000, 0xb4e6_1ba5_f1c6_c4b5),
        (4500, 0x8bfd_e2ac_2444_471c),
        (6000, 0x914c_7d5a_d554_43f9),
    ],
    &[
        (1500, 0x5458_3b64_e41b_2178),
        (3000, 0x9842_251e_7ef5_e3cd),
        (4500, 0x98f3_8523_7536_e18c),
        (6000, 0x17e8_7814_f3da_06a0),
    ],
    &[
        (1500, 0x7e9c_56d1_f98b_fc62),
        (3000, 0xde9a_f6a5_8ef2_917b),
        (4500, 0x9b16_4cff_a6f8_e88e),
        (6000, 0xee76_efc0_79a9_77d7),
    ],
    &[
        (1500, 0x83e0_ec13_7650_5cad),
        (3000, 0x5e83_051a_0a0a_695e),
        (4500, 0x40f1_f762_8326_9d1e),
        (6000, 0x5ac7_0e21_6838_18cb),
    ],
    &[
        (1500, 0x85f8_221e_8c68_ada0),
        (3000, 0x655f_eb17_a117_ebb0),
        (4500, 0xc173_47c8_3984_c592),
        (6000, 0xbce5_d47a_1fc5_b42e),
    ],
    &[
        (1500, 0xfa97_30e8_a9d2_0735),
        (3000, 0x0ff1_ea87_acd4_3bc2),
        (4500, 0x79db_5c0c_0e2c_0d9a),
        (6000, 0x1796_5caa_9733_c1e8),
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
            (3000, 0x7fc0_2f15_b065_ebd4, 0x9e6d_2cd9_4627_d6e2),
            (4500, 0x120f_b86e_c473_44ac, 0x0cc0_a980_bf83_ed7b),
            (6000, 0xf12b_60f5_c995_f7af, 0x5e8a_b628_5141_aafc),
            (7500, 0x86a9_6a08_fb37_0d22, 0x952e_43fc_efb1_82e1),
            (9000, 0x5d42_5dcc_bbd0_83b5, 0x521d_130d_032e_2c06),
            (10500, 0x994e_fd2b_21de_daa0, 0xa583_41b0_ca8e_acf4),
            (12000, 0xe03e_7f4e_f211_82e1, 0x8913_b6cb_c115_b9bb),
            (13500, 0x4ce2_9fff_0408_1ba7, 0x2107_4999_95a0_1ef8),
            (15000, 0xd240_9022_bceb_d89a, 0xfab7_1b18_9441_fd57),
            (16500, 0xff2c_8dd8_7708_e1f1, 0x93c6_86d5_064c_8ae0),
            (18000, 0x5a38_717d_e9e0_ea8a, 0x244a_a05d_3e44_6e68),
        ],
        last: 0x26f4_04ed_0a62_3602,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7b83_7436_e0fb_1d46, 0x3bf9_b0d0_6ebb_3728),
            (3000, 0x9dac_c63c_6a48_fb9f, 0xed25_af9e_c722_e779),
            (4500, 0xcbad_44e9_4e20_27f8, 0x031e_2040_fe35_78a6),
            (6000, 0x8399_1a36_9b2d_bb90, 0x12c9_c92c_2aab_ba1a),
            (7500, 0x3283_0630_6e85_8bde, 0x106d_84bf_e566_7d92),
            (9000, 0x5902_854d_f3dd_5f07, 0xc20a_2631_146b_0363),
            (10500, 0xcb2d_f187_c098_bd2e, 0x1732_2151_3590_1995),
            (12000, 0xdb2a_9de2_9802_8d4b, 0x3369_6382_0179_5756),
            (13500, 0xcfe0_dff3_144a_889b, 0x27bf_cf5b_17a9_858c),
            (15000, 0xac3f_3f23_6ebc_4988, 0xaf5e_69f2_3882_58ad),
            (16500, 0xb8dc_4175_9d8d_bcd5, 0x1357_fc2b_1443_7f48),
            (18000, 0xa033_76bd_8254_5bf6, 0xbe58_4be0_db14_c15e),
        ],
        last: 0x31f1_e27e_a257_1892,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x7984_9483_fdfe_0a14, 0x24ad_1828_95a0_858b),
            (3000, 0x7e0c_abc7_9a18_31a5, 0x52a2_a092_8c3e_025a),
            (4500, 0x69c0_f2e6_6478_fc2c, 0x270b_f043_7b55_c7a3),
            (6000, 0xcb16_c71f_2e88_12d3, 0xf9b0_19ba_5c60_04ab),
            (7500, 0x3c08_ce0f_b0c4_1f64, 0xc2ac_d523_15cf_213b),
            (9000, 0x206b_ed2a_beea_dcff, 0x3bd9_0863_a54d_657e),
            (10500, 0x39fc_e5ca_a46a_9a61, 0x7474_2336_8537_98d1),
            (12000, 0xf98c_2ba6_08be_933f, 0x0893_03ac_0ad2_75f7),
            (13500, 0x86ca_df3a_5a36_1c0f, 0xd296_f233_7d15_142f),
            (15000, 0xd013_03c1_17c4_4dd9, 0x71ff_876b_8960_b82b),
            (16500, 0xeeea_a0de_36ac_372e, 0x7931_f98f_31ef_67cf),
            (18000, 0xa5ce_2d08_66ea_3248, 0x5c45_a0de_7abe_ba9b),
        ],
        last: 0x9706_3680_3793_088c,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x3fce_acf9_3e11_e047, 0xf583_f81f_6832_3acf),
            (3000, 0xfba7_cb45_b00a_7610, 0xc248_a597_c83c_3078),
            (4500, 0x6ebb_60f7_53be_4a0f, 0x7cfb_7c29_1c48_3a87),
            (6000, 0x3d78_ef55_a719_cbbb, 0x4139_905b_d73d_f7bb),
            (7500, 0x72d6_6a02_89f3_f274, 0xc4e1_57d0_7df0_63a4),
            (9000, 0xb440_e46e_e172_e745, 0xa69d_5f49_f0c8_83f9),
            (10500, 0xa3ab_9cb1_3d23_72e4, 0xb92e_4983_db18_a324),
            (12000, 0xd7ec_1d45_249b_740b, 0x03ec_34cf_0d6f_ebdc),
            (13500, 0x4378_ba91_1ff6_963e, 0x6c41_889a_228f_b571),
            (15000, 0xda3e_b78c_c276_50e0, 0x3166_cf7a_d397_8e07),
            (16500, 0x25a1_d9dd_39e0_2f31, 0xf38c_1b08_e7ee_d09c),
            (18000, 0x5e01_653b_46da_fc1b, 0x051a_801b_b38d_667c),
        ],
        last: 0x0132_22c5_cf60_22b2,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x24f2_2194_ac14_decc, 0x5dda_f9ab_6685_c361),
            (3000, 0xd6dd_86cf_b7e8_6a31, 0x3c28_9648_e870_949f),
            (4500, 0x91a2_a713_c557_1c49, 0x92e9_5bf0_4092_9f39),
            (6000, 0xe16c_ed4e_9fe6_f188, 0x2c1b_ce2d_de4f_0c8d),
            (7500, 0x458e_9a97_976e_db2b, 0xaf59_9862_5957_d85f),
            (9000, 0x4cee_043d_524e_f7b2, 0xab28_b386_2248_b229),
        ],
        last: 0xf8c9_b747_f0b5_9e47,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf2e1_5ef9_9668_7203, 0x237a_4c08_33fb_4a81),
            (3000, 0x8be8_44c4_7224_0ab1, 0xa694_1841_4cef_6414),
            (4500, 0xebe0_c856_ee02_c809, 0xa9b7_1d48_e80d_9e9d),
            (6000, 0x58a0_b856_01b6_a303, 0x9d2a_7542_cc37_1591),
            (7500, 0x2b2f_2025_f22a_3f6d, 0x1854_016d_bdac_f518),
            (9000, 0x904b_2137_78ac_8713, 0x84f6_2735_9d6e_e691),
        ],
        last: 0x9702_66fa_b4e6_7cb6,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5aa4_cbb8_74d9_d56e, 0xb9af_838f_268b_aed3),
            (3000, 0x1f06_8235_8020_d363, 0x6633_b17c_be1e_91f7),
            (4500, 0x131e_df63_1b78_7c3a, 0xf1ef_bb07_fe6f_f710),
            (6000, 0x5900_d2b7_c02f_5d9d, 0x76a6_435c_9725_1bae),
            (7500, 0xe507_3736_9d87_737f, 0xc936_3afb_4163_8038),
            (9000, 0x5755_7ab3_7462_c196, 0x3e30_2d39_3336_5e3b),
            (10500, 0x4ba4_1cef_23c0_c3a8, 0x0137_a644_7f66_c10d),
            (12000, 0x87ba_df42_36be_a89f, 0xf7a4_923d_fbd3_e6a7),
        ],
        last: 0x702f_8bac_cff2_dd8c,
    },
];
