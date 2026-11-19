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
        (1500, 0x21bc_02f2_7f70_7668),
        (3000, 0x2cef_c4cd_99d3_22b7),
        (4500, 0x6fad_b5ab_d88d_2a75),
        (6000, 0x05af_c42b_5a18_e412),
    ],
    &[
        (1500, 0x3b31_1927_f5d8_7f80),
        (3000, 0x9d62_a390_082f_f1da),
        (4500, 0x1eac_3237_e387_7eae),
        (6000, 0xfe8b_7a71_7614_8307),
    ],
    &[
        (1500, 0xe800_5be3_be28_8b7b),
        (3000, 0xe922_5a0a_52b1_097f),
        (4500, 0xfc8b_678d_9607_9485),
        (6000, 0x23fa_30fd_c9d8_37c7),
    ],
    &[
        (1500, 0x8a0a_21e5_673c_84ac),
        (3000, 0xf1c2_3062_8c82_cc62),
        (4500, 0xfa12_6164_a447_568c),
        (6000, 0xc29d_4f34_3832_18e8),
    ],
    &[
        (1500, 0x9e5d_9c3f_63cc_feaf),
        (3000, 0x1595_f857_8f1c_4a42),
        (4500, 0x17bd_1b5c_163e_2d12),
        (6000, 0x732e_81d6_7697_cdeb),
    ],
    &[
        (1500, 0x7907_297f_902e_9b26),
        (3000, 0xcbc5_7876_8e1e_843a),
        (4500, 0x8c63_cbf7_c872_e937),
        (6000, 0x62d4_f154_e618_ede5),
    ],
    &[
        (1500, 0x11b2_41dc_9d4e_d753),
        (3000, 0x41df_d424_25bb_3742),
        (4500, 0x4f68_2a63_4199_a883),
        (6000, 0x1e64_d230_ee6f_263e),
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
            (1500, 0x384e_9be8_02a7_828e, 0xbbce_5feb_55af_ae6a),
            (3000, 0xf667_9826_4eb1_21af, 0x7ac1_56a2_6774_d189),
            (4500, 0x2693_8199_93d8_80f1, 0x9572_4e09_5d5f_beae),
            (6000, 0x3749_107e_2754_580d, 0x8951_5bc3_38de_8913),
            (7500, 0xb291_3e6b_1b3c_cb11, 0xc5cb_100d_a9ef_f73d),
            (9000, 0x8b44_0b04_7d6c_b521, 0x3d97_462b_843f_1d1f),
            (10500, 0x65e2_a6d1_effd_8ed4, 0x775c_e9fd_2ef8_0838),
            (12000, 0x3dae_d280_cb99_86e0, 0x075e_7754_2efe_c060),
            (13500, 0x7d83_f50d_5e90_c78b, 0x859c_a303_6060_6a6f),
            (15000, 0x400e_191f_df01_8123, 0xe333_9343_ca31_820a),
            (16500, 0x0ba4_cd1d_2bfd_7540, 0x02bc_50cc_2d52_cf4b),
            (18000, 0x9275_865f_2263_22b5, 0xcd8b_c9c5_4fe3_d502),
        ],
        last: 0x5793_a8a5_1560_8dd0,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xabf7_5d88_a388_a09c, 0x5758_53f6_6bb3_9936),
            (3000, 0x5e08_04a1_3150_f9bd, 0xdf7c_6ebb_d77e_96da),
            (4500, 0x37fe_f4a6_bf47_85fc, 0x1da5_00da_30f0_485c),
            (6000, 0x76f4_f85f_1681_41d1, 0x7bdf_50dc_4a72_f92d),
            (7500, 0xea2c_f7ca_8a58_ecc8, 0x3ece_8372_653b_6da2),
            (9000, 0x3d98_4bff_3656_60a2, 0x7ade_5e47_d7cf_2c71),
            (10500, 0x69a5_f16e_69a6_bfbb, 0x93bf_7086_0cc6_b87d),
            (12000, 0x49f1_83e2_5391_03b2, 0xddd1_e129_149c_b753),
            (13500, 0xd184_1a60_eb1e_249f, 0x6615_58af_bb60_1ca1),
            (15000, 0x20fd_47cf_7b29_eecc, 0x63f7_e793_f774_66cf),
            (16500, 0xc570_e02d_b528_fc34, 0xa1d1_7b53_89f8_761d),
            (18000, 0x8d76_e67a_06e4_4f4c, 0xa9e2_924d_49af_1576),
        ],
        last: 0xbfbb_a176_2081_512a,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xc83e_404d_9c67_c880, 0x4755_1d63_59ca_2c89),
            (3000, 0xe679_3f64_8729_7b12, 0xab42_ab50_0f5d_f804),
            (4500, 0x7ce7_edff_2e0a_eb20, 0xf7ce_0fb3_4dc6_23c3),
            (6000, 0x10d5_2ea9_8f6b_92a8, 0xaa3a_e454_8381_16b4),
            (7500, 0xc808_7d8b_8a4a_6a17, 0xc733_560d_eeb5_6254),
            (9000, 0x6e0e_131d_9062_d8db, 0xdfe7_f36a_6444_cce4),
            (10500, 0x5957_dd8f_37d1_32bb, 0x38be_169b_bfcc_56d5),
            (12000, 0xaacc_c251_736c_80dc, 0x0f83_f420_fbb2_2f36),
            (13500, 0x8e4b_139d_29d8_71bc, 0x1423_f3ae_0154_079a),
            (15000, 0xb451_daf3_051e_070d, 0xe986_4f99_d094_a390),
            (16500, 0xc638_6d77_b9dd_0350, 0x67a5_d093_9c8f_d028),
            (18000, 0x6acc_2e6b_ce4b_1c13, 0x80be_7eec_e7c9_154a),
        ],
        last: 0xc79a_5abf_5987_c38d,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x6c45_24dd_cf36_e034, 0xe4ca_3e3a_f035_5375),
            (3000, 0xff36_bab6_9fee_faa1, 0x4c32_7c2c_cdc6_3852),
            (4500, 0xbcd1_f565_2c7b_44a7, 0x9fbb_c7bc_bb52_a53b),
            (6000, 0xb445_36e9_345f_1f6b, 0xce8f_8b5e_4415_a4b0),
            (7500, 0x7bbd_386b_ac69_fc82, 0x63b3_6d76_428a_8c87),
            (9000, 0xc43f_ba0d_7d34_ec42, 0x0a63_c3d0_e90a_547c),
            (10500, 0xcb6a_4f23_f59a_4917, 0x6f45_b3f6_32e9_79cc),
            (12000, 0x1d47_dff2_02bc_1d25, 0xd5de_3360_7277_7a05),
            (13500, 0xbe56_ff1f_558a_5a81, 0xaf1c_7624_b2f8_8665),
            (15000, 0x7ae5_13fc_ebdb_8d9f, 0xf9f2_47b6_c353_1401),
            (16500, 0xcf5f_59aa_6be6_393f, 0x1f6b_607c_7828_b95c),
            (18000, 0x9a21_6f42_474a_6859, 0x3ce4_c8d2_933e_400b),
        ],
        last: 0x1b9b_49a0_146f_f87d,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2a8d_9d5b_86d5_a6f8, 0x53ed_68c1_5262_4254),
            (3000, 0x41be_6d65_673b_7bbd, 0x881c_d6db_d376_0249),
            (4500, 0x2bdf_7ae7_11d1_5ffd, 0xd998_c449_5ad2_c920),
            (6000, 0xd61b_75ea_6344_4a7b, 0x2a08_d2f4_3608_b2ef),
            (7500, 0x9af3_d422_183b_07d4, 0x6e28_0dd5_e228_138e),
            (9000, 0x6962_a226_4d07_ce1f, 0xea6c_7a01_d1f2_f1a8),
            (10500, 0x0844_5f0a_c81f_b64a, 0x5763_0d88_0543_4363),
            (12000, 0x4c63_5b4d_a83d_b707, 0xb8fc_63d8_3bc9_504b),
        ],
        last: 0xdfb3_7f4a_e027_ece5,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5dcb_8788_90dc_ae30, 0x3b6e_e171_4d4a_0050),
            (3000, 0x4cf2_80b0_0dca_006e, 0xf666_f4f7_4cb4_632f),
            (4500, 0x69b4_8824_9e67_05e7, 0x9b44_8627_3a2e_add4),
            (6000, 0x3e16_b250_10ff_8a69, 0xefa0_3756_4f1c_e47f),
            (7500, 0x4e53_c455_a5c9_7ed7, 0xe25e_f62e_9476_ab05),
            (9000, 0x3996_a6cb_20e1_d58d, 0x277d_b20e_fe08_acb8),
            (10500, 0xe02a_9569_a4c1_a412, 0x06c6_5073_048e_bdfb),
        ],
        last: 0x1fd7_ef8d_3e67_1f2d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7f7b_d8af_d353_5678, 0xbb5c_03a8_d3bc_6a19),
            (3000, 0x59d6_2a82_529a_00b0, 0xd6da_8dc7_9a9a_585f),
            (4500, 0xf935_e2ea_2c68_520c, 0x7a8c_8fe1_9391_e450),
            (6000, 0x71b1_c906_3e67_86d1, 0xb0e9_6235_436c_54e7),
            (7500, 0xbab7_9cc3_201e_535e, 0xee7c_6e49_a876_ce94),
            (9000, 0xb909_1d3f_eddd_7c17, 0xe471_8bef_3e77_ae77),
            (10500, 0x9572_2a5f_e716_dd23, 0xd0b6_4e55_3f84_cc32),
            (12000, 0x2b07_c8ef_5798_68bc, 0xd196_b10a_1336_083c),
            (13500, 0x6ff8_c879_7f52_4684, 0xbe97_dfd3_ff7f_b06f),
            (15000, 0x7a78_b827_9491_6c89, 0x5887_c3ba_be7a_6498),
            (16500, 0x7033_33f2_35a2_fc51, 0xa76b_80a6_bad1_50b9),
        ],
        last: 0x4761_48cb_afc1_6e87,
    },
];
