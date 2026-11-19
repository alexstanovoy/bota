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
        (1500, 0x6b0e_9467_c383_7892),
        (3000, 0xe081_d854_a27d_1dda),
        (4500, 0xc9bc_d12b_8b5a_19da),
        (6000, 0x1d8b_0474_5a7a_5c4c),
    ],
    &[
        (1500, 0xf667_06ed_2dde_07ae),
        (3000, 0xbc42_aacf_63a7_0b9e),
        (4500, 0x6eca_02fd_7323_f033),
        (6000, 0xbb3e_8da0_f370_07da),
    ],
    &[
        (1500, 0x3fc9_ea94_50b3_d323),
        (3000, 0x2b3f_3736_8ae1_ffeb),
        (4500, 0xf3cf_765c_838c_5b47),
        (6000, 0xfd59_d063_fab1_e356),
    ],
    &[
        (1500, 0x6842_aac5_d184_ea2f),
        (3000, 0x8a4f_65a3_cb87_5b5c),
        (4500, 0x707f_9e10_dd7e_1e6c),
        (6000, 0x1fb3_ba87_2abb_aa8d),
    ],
    &[
        (1500, 0x9632_bfa0_09da_9513),
        (3000, 0x553a_fe94_d295_2f29),
        (4500, 0x75d9_8eb4_f49e_3c90),
        (6000, 0xe968_7e80_4568_aab7),
    ],
    &[
        (1500, 0xfb32_d3e4_c25d_403e),
        (3000, 0xc8ee_b79a_add7_2dbe),
        (4500, 0xc6fd_974c_d02b_eff6),
        (6000, 0x7bbd_c921_94eb_358b),
    ],
    &[
        (1500, 0x98eb_0344_838f_71a3),
        (3000, 0x0083_145b_5a77_5c5a),
        (4500, 0x39c6_73f2_5b80_f7a7),
        (6000, 0x56cd_364d_166c_5bd4),
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
            (1500, 0xb57b_af11_3330_e800, 0x5c6c_f55c_9820_b314),
            (3000, 0x0b5c_55c2_4234_36c2, 0xaf65_c12a_29a4_bd19),
            (4500, 0xe3e4_8511_9138_2c20, 0x39db_1565_a6a3_5e88),
            (6000, 0xc40c_3a26_f6cc_3b60, 0xc9fe_6153_6229_5eac),
            (7500, 0x1369_b7a2_17b0_54cd, 0xd3f4_0053_e57a_02a5),
            (9000, 0x340f_7907_d8d5_0c5b, 0x1090_cae7_0dfb_6804),
            (10500, 0x58ec_59bb_de5a_ce0d, 0xaf88_da88_c5d9_b6f6),
            (12000, 0xfeb5_f0e1_613e_de21, 0x8635_6064_97e6_00f0),
            (13500, 0xe066_8ae7_2d29_bdc3, 0xf64c_f8b4_4c6b_36c7),
            (15000, 0xdabe_b59f_1d67_155b, 0x2e89_2054_4ab9_806a),
            (16500, 0xdcfd_2db4_ad75_7e42, 0x928e_0238_2df8_e225),
            (18000, 0x7c08_13c0_e953_cec9, 0x2430_f25b_5a28_8fd3),
        ],
        last: 0x52df_84ea_a89a_3d2e,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x8cd0_9810_06b9_4bb2, 0xca0c_0993_52d9_56b5),
            (3000, 0x77a0_fccb_dd3e_ea74, 0x36c0_472d_f01b_0b67),
            (4500, 0x104e_14bd_6e46_1aa8, 0xa8ab_c67c_4d69_ac04),
            (6000, 0x92a6_4a0d_8932_8879, 0x657c_8b20_d3e6_e869),
            (7500, 0xcb6b_353e_cb86_bd68, 0xe9f8_5aeb_2d05_cdb4),
            (9000, 0xc255_4a60_47df_cda6, 0x0c48_9fe2_eaff_e982),
            (10500, 0xb10b_b9f4_baa6_0b6f, 0x549f_ea63_4c91_2187),
            (12000, 0xdc5f_c82c_c611_0109, 0x546c_e6e5_0ca3_216b),
            (13500, 0x7364_7485_bfab_3b72, 0x04fb_a5db_7dfa_f7ca),
            (15000, 0x5799_6d22_0a63_8099, 0x2aae_6a8e_0d11_40e1),
            (16500, 0x6ff3_1929_1243_088d, 0xacd3_ba77_978f_5fe4),
            (18000, 0x32c0_a70a_8d05_1a78, 0xb98c_a11f_f212_f8b6),
        ],
        last: 0x2dae_20e6_67d5_5ba4,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xeddf_a8ff_2451_a340, 0x002f_f6b2_37dc_72b6),
            (3000, 0x93cf_ab3b_ae42_7086, 0xd50f_50d0_9496_c163),
            (4500, 0xc9fa_149c_9fe9_c9b2, 0xd65b_c383_efe8_686b),
            (6000, 0x668f_3589_834c_d9cd, 0x7f32_9250_86f5_81f9),
            (7500, 0x183e_0449_c912_41f9, 0x0e73_dd6a_d58a_757d),
            (9000, 0xfc10_494b_0126_1a74, 0xde1e_c104_2689_8bae),
            (10500, 0xa536_2607_6a7f_025f, 0x9301_732e_e146_783d),
            (12000, 0x8f1c_8227_0b7e_926b, 0x2102_eb20_ffbe_521a),
            (13500, 0x500a_0780_6e6a_1c30, 0x1ec0_13a3_d99c_0160),
            (15000, 0x6125_9230_33a3_670c, 0xbfc0_8fbe_fe9f_6110),
            (16500, 0x5ef4_5f20_d55a_43d5, 0xc245_8c31_ee5b_876c),
            (18000, 0x81ed_3461_9dc5_706a, 0x50b4_3683_9601_9aaf),
        ],
        last: 0x8e40_ba3a_2267_c42d,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xaf7d_632d_252c_1788, 0x5605_dd07_2636_3c99),
            (3000, 0xb4d8_3311_010c_de55, 0x7b0f_2670_b717_3e85),
            (4500, 0x4b46_051d_53d7_bea2, 0x6cd5_9c59_ae81_f81b),
            (6000, 0xad70_07a3_9c9c_cf5c, 0x2062_94c7_6394_4fec),
            (7500, 0x07c7_f1d6_2462_b33f, 0x6ae0_a771_c3a8_1d69),
            (9000, 0x897f_fa1e_239c_931e, 0x7a5a_f98a_2c28_76a0),
            (10500, 0xadb2_ac7e_109b_f212, 0x4617_b498_2492_19cb),
            (12000, 0xa8f1_babb_efe8_8428, 0x77bb_03be_5106_f0b5),
            (13500, 0x8a15_55e3_1649_8812, 0x61eb_1042_751e_9767),
            (15000, 0x0805_6084_4b6f_6aa1, 0x7bfe_6a54_7d53_6bfe),
            (16500, 0x1552_38cf_3ca3_691a, 0x11b7_6160_24cc_1b66),
            (18000, 0xbf3e_6238_1a17_16e3, 0x07df_66eb_ae25_17c3),
        ],
        last: 0x316b_d32c_0ae3_44b0,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xea63_3ac0_67d7_6bf8, 0x0102_5e2f_8394_544d),
            (3000, 0xdf5f_c927_dce8_cff7, 0xbeb1_bf0f_22d2_83a4),
            (4500, 0x34df_4dbc_d177_1b70, 0xafd2_8dbb_ddca_4ded),
            (6000, 0x824a_e4ca_b7e9_2cba, 0x0d25_7668_9296_98cc),
            (7500, 0x9a0b_0b1e_5bef_9d79, 0x8a15_edaf_1eb2_491e),
            (9000, 0x3f02_02aa_f9d4_c15b, 0x8da1_c141_ebe6_3465),
            (10500, 0x83a9_d228_a1d1_51f2, 0x944c_c0e5_9c0a_0529),
            (12000, 0xacb1_75a9_fdfd_1bf1, 0xcac2_9106_0deb_604f),
            (13500, 0x6ef4_7941_5c3c_9110, 0xf966_df26_ae4f_9d4e),
        ],
        last: 0x3244_6aa7_3939_5754,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4498_6be7_0c78_3b70, 0xcc65_d5ec_d251_0f2a),
            (3000, 0x129d_a3e2_e5d4_dcb2, 0xa766_75c3_364b_e57e),
            (4500, 0xf606_81f9_508f_c76c, 0xe836_eb17_d2d1_fe87),
            (6000, 0x53db_0c07_a43b_f7fe, 0xf23a_092b_d684_706e),
            (7500, 0x2f80_1dd8_c989_62bc, 0x4a5d_6f55_9143_e50b),
        ],
        last: 0xc2ea_febe_aa21_b639,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0ddd_c92a_2beb_e748, 0x0469_fc8b_f935_ba9c),
            (3000, 0xfa5f_4495_523a_7f6a, 0xfabf_853b_0977_b9ec),
            (4500, 0xd4d4_6ef6_63bd_0410, 0x75dc_93f3_c3fe_94ea),
            (6000, 0x5cf4_6fd9_eb05_bc33, 0x6cab_2509_0228_8d51),
            (7500, 0x545d_bc33_4760_364e, 0x697d_ef85_9afd_0c91),
            (9000, 0x5809_acc7_f159_61c6, 0xa755_43c2_613b_8353),
            (10500, 0x0ae8_9622_4859_6d32, 0xe6cd_69cb_7bd3_edea),
            (12000, 0x2e3d_6151_89c7_f101, 0xb91b_38a7_3553_5218),
            (13500, 0x4560_6a34_9f45_44b4, 0x9dda_4c01_cf5b_d602),
            (15000, 0xcfce_a71a_d29f_8a5d, 0x5cfc_cd0d_6012_ad3e),
            (16500, 0x9d87_cedf_53a4_47cf, 0x466b_73a2_8825_439a),
            (18000, 0xf99e_8225_31cc_3677, 0x1fea_348f_492a_6c43),
            (19500, 0x83f9_c636_0a72_5be8, 0x6732_7440_eea1_cbc5),
            (21000, 0x9a3c_07a2_e514_d1cd, 0xaf10_f7ce_416f_3890),
        ],
        last: 0x38f8_a25b_3461_7118,
    },
];
