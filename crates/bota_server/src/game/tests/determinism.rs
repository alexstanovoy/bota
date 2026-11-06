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
        (1500, 0xd8c9_f486_c45d_4ea4),
        (3000, 0x2550_954a_4f8f_c6b4),
        (4500, 0x461a_dee0_3234_0257),
        (6000, 0x30bb_bc12_0793_2f30),
    ],
    &[
        (1500, 0xf5d4_0856_d01b_6ec1),
        (3000, 0x8732_639a_0859_3f9d),
        (4500, 0x2670_cedb_4d18_07a6),
        (6000, 0xdfb5_cd8a_cfcc_464d),
    ],
    &[
        (1500, 0x5f1e_332c_8d2c_b334),
        (3000, 0x1430_ecf4_16f9_dc98),
        (4500, 0xdd69_423d_6834_e671),
        (6000, 0xb402_4bba_1234_8cd5),
    ],
    &[
        (1500, 0x9ffe_6e95_f0a6_e144),
        (3000, 0xcd99_68dd_2f52_c32e),
        (4500, 0x636c_c967_ca5a_3f06),
        (6000, 0xe1a9_812a_26ad_19f0),
    ],
    &[
        (1500, 0x8580_8c55_4396_c16d),
        (3000, 0xaaa1_9cf9_5387_706c),
        (4500, 0xb108_20fc_9654_fc01),
        (6000, 0x5758_95d8_2fd7_4e2d),
    ],
    &[
        (1500, 0x0d1e_ed7a_12ab_2f22),
        (3000, 0x1f8c_6a73_fce4_6b07),
        (4500, 0x844c_a877_8f01_ed4c),
        (6000, 0x27af_59f1_ea20_b386),
    ],
    &[
        (1500, 0x7d14_0c4c_570c_7070),
        (3000, 0xbdc6_9679_a634_e70e),
        (4500, 0xb229_81c0_05b1_e7d2),
        (6000, 0x12d7_d6b0_8344_bebb),
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
            (1500, 0xafb9_be72_dba1_eb9a, 0xf5ce_efa7_76d5_ff54),
            (3000, 0x0a74_dec3_5de7_f9b9, 0x973d_7f82_7f0a_cbf9),
            (4500, 0xbe46_204b_1d02_e62d, 0xc2b9_5f5f_bffa_9349),
            (6000, 0xc145_0607_260c_bd81, 0x62c1_6c0a_c937_c477),
            (7500, 0x7dfd_d42a_afe5_0ee8, 0x01fa_bca2_e84f_b235),
            (9000, 0xc871_9316_688e_4937, 0x3134_02ac_2a7f_94e8),
            (10500, 0xedfc_ac7d_0f7a_8e8d, 0x122a_49ea_16ca_f528),
            (12000, 0x2aa6_65ec_486a_fc28, 0x4c22_d08f_9100_27b5),
            (13500, 0x4c93_97f0_dd14_b62e, 0x17dd_0b90_a7c0_8c54),
            (15000, 0x6256_3f09_ce87_368a, 0xad29_f518_2744_850f),
            (16500, 0xad71_245f_bb76_faf2, 0x8cf7_0a31_387f_bd84),
            (18000, 0xa678_4a91_ae9c_6948, 0xe6ea_37c0_c9bb_e479),
        ],
        last: 0x7349_4d38_61e6_75c0,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe71a_1ae4_e04c_92bb, 0xd394_d966_6469_fa50),
            (3000, 0x75de_9578_5d8f_7082, 0x6ee7_ad5d_b0ae_33f2),
            (4500, 0x053b_a251_1f2f_0209, 0xd629_15e2_66c4_6897),
            (6000, 0x2e29_e0a3_6eb0_de74, 0x7e0f_0066_97ec_c5f4),
            (7500, 0x25b7_ec7b_e503_708a, 0x3f80_e14d_7a04_149f),
            (9000, 0x084b_0d38_33d3_4c79, 0x0860_8cc6_b4a5_a139),
            (10500, 0x9ebb_3562_e0e7_ef5f, 0x6425_a622_e31e_2c69),
            (12000, 0x2ad7_2fe7_7f2d_c18a, 0xc4fe_d5df_3b1a_cb31),
            (13500, 0xe175_9646_89ac_a438, 0xa9ae_54bb_12e6_9c68),
            (15000, 0x5064_a251_bd10_4c1c, 0x86ad_4b29_b1e0_7f68),
            (16500, 0x92bf_8f34_5063_9766, 0x0bcd_b650_1c06_5221),
            (18000, 0x84f4_ca5c_6b94_cafc, 0x39b4_7c92_7e49_58b7),
        ],
        last: 0x1cda_4d3d_a0ed_5869,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xf3b9_5918_8951_519b, 0x7141_49b5_42fb_e211),
            (3000, 0xc339_c479_1931_36ef, 0x0fc7_447f_366c_84ed),
            (4500, 0x7069_fdc2_9da7_17e1, 0xc6bb_ba0b_a239_3771),
            (6000, 0x9ba2_456b_83c8_1e4f, 0x501d_02dd_0c6f_57cb),
            (7500, 0x2444_4b2e_cb52_73cd, 0xcd4d_db4d_8714_a20d),
            (9000, 0xcd6c_8c3c_0eb3_69c3, 0xcb9c_0173_b531_560f),
            (10500, 0x8339_de4d_7f15_922c, 0x316c_10c5_56e9_6795),
            (12000, 0x058c_ec7d_9b9a_4d3b, 0x895e_5083_1e5d_1c6c),
            (13500, 0x45f6_6c20_c190_56e3, 0x59da_6e4f_9a4c_1e65),
            (15000, 0x6562_2979_0858_07d5, 0x64ff_0c87_676a_11c2),
            (16500, 0xec6a_7f5a_3ad0_e684, 0x2d94_3fae_ff64_31d3),
            (18000, 0xe05d_513a_c7d7_4a05, 0xb35c_36f7_f24e_6721),
        ],
        last: 0x3b35_8126_8e1a_8f18,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xd1f9_343c_9425_6ae3, 0xf125_848d_6481_e69c),
            (3000, 0x6045_c2c4_17b3_7167, 0x265e_5275_c729_9bd5),
            (4500, 0x9518_fd8c_5018_b532, 0x4475_9f0f_5997_d0f8),
            (6000, 0x38d9_dd6b_d769_a3e0, 0x9f17_ad7e_d17c_68d5),
            (7500, 0x3c7f_35a8_134d_d81e, 0x68e0_9ae6_9af3_d31c),
            (9000, 0x3052_9265_9c91_f082, 0xfae6_cd8e_451a_9e50),
            (10500, 0x347f_b73d_59d7_b36d, 0x5fff_6f50_a0fd_3a38),
            (12000, 0xbf0a_8a84_8baa_f66a, 0xd11a_c2d5_11db_d204),
            (13500, 0x3248_82f9_4748_db99, 0xb613_05fb_0d1f_a44f),
            (15000, 0xaa1b_af99_cae8_5918, 0x5dbb_a211_b98d_e742),
            (16500, 0x29cb_642f_f451_965c, 0xae18_ec54_c2d5_3b97),
            (18000, 0xbd92_d518_6d29_bcc6, 0xa72d_edf9_e8c3_d89b),
        ],
        last: 0xd363_b717_568b_eee7,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0571_29a0_1c95_5505, 0x0e3a_bd42_8a5f_89ca),
            (3000, 0xadea_b22d_29d7_9f4c, 0x0b3e_c8e8_9a3b_d92d),
            (4500, 0x8ebc_4590_5e19_4b3f, 0x67a2_4d50_e8a5_43ff),
            (6000, 0xda55_3ca5_44b7_5378, 0xfbc9_5237_a911_fbb8),
            (7500, 0xec63_0cad_e747_0227, 0x36c3_2ffd_6df2_9544),
            (9000, 0xe304_3238_12c2_c20b, 0x3fab_9fdf_067a_c22a),
        ],
        last: 0xc3de_9209_ab92_a648,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd9bb_35c0_4dde_01e5, 0x3095_f779_6eb2_34e1),
            (3000, 0xe5e8_7c55_dac6_b40f, 0xda51_dd3a_112a_7714),
            (4500, 0xa4eb_61ab_8687_cb7a, 0xa4d6_ed92_6747_6f43),
            (6000, 0xb6e9_2148_356d_551e, 0x1075_aea4_8beb_c8e5),
            (7500, 0xb80b_7153_2e92_fd35, 0xc319_1eb8_4eff_548c),
            (9000, 0x9d09_1776_8e45_9468, 0x1fec_839e_2cd1_7a80),
            (10500, 0xb31f_3909_33dd_7ef8, 0xd17f_884c_13a7_d7fa),
            (12000, 0xdbae_2189_9e66_403d, 0x428b_c8b5_72a8_dd52),
        ],
        last: 0x008f_923a_0d24_8721,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3983_4873_dd21_ba13, 0xfdc8_e592_5f8f_ee6d),
            (3000, 0xd116_88c5_9a8c_4aa4, 0x0766_c446_5a2d_f47c),
            (4500, 0x6f26_4ccc_c471_411a, 0x83ab_ebdb_146b_e31c),
            (6000, 0xb08d_6860_3183_8a68, 0x559f_532a_ccc1_f279),
            (7500, 0x88be_48f9_cce9_d7f8, 0x3db2_ef64_85d0_e6d0),
            (9000, 0x75a2_3f0e_db4c_7be6, 0xf74a_131e_97f0_6932),
            (10500, 0x53a7_b0a7_3adc_1d2f, 0x2e8a_4da3_f72f_deb3),
        ],
        last: 0xe4e5_0b67_f6ea_d003,
    },
];
