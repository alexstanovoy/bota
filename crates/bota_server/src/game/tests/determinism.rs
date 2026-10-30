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
        (1500, 0xf186_7dc2_b538_6757),
        (3000, 0x4191_1ffc_3d00_b6f5),
        (4500, 0xbad4_ee04_b9fb_af15),
        (6000, 0xe3b1_28ea_b61b_3ebb),
    ],
    &[
        (1500, 0xb265_44b1_6d40_a725),
        (3000, 0x69a4_e436_7be3_93b0),
        (4500, 0x1f73_2b57_b778_5144),
        (6000, 0xbc86_57f6_5197_5f2c),
    ],
    &[
        (1500, 0x5f1e_332c_8d2c_b334),
        (3000, 0x99cf_44ed_a3cd_e638),
        (4500, 0x40a6_8a33_52ef_fa0a),
        (6000, 0xcbe1_f678_7211_686b),
    ],
    &[
        (1500, 0x9ffe_6e95_f0a6_e144),
        (3000, 0x82a0_9466_6421_f968),
        (4500, 0x13c8_0d13_3596_96c0),
        (6000, 0x6c41_a2f9_04a7_ed03),
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
            (1500, 0x8b35_614e_b1aa_44c9, 0x2c29_3939_e812_2639),
            (3000, 0xbc90_b0ae_494e_f99d, 0x8b51_f02d_e837_1ca0),
            (4500, 0x7f51_cb9e_1252_5d4a, 0x7f0a_2f39_b588_46ed),
            (6000, 0x7ca8_705b_25be_af2b, 0xc805_44bb_d945_22c7),
            (7500, 0x6ae6_c57a_927a_d25d, 0x53bf_90aa_9f96_73c2),
            (9000, 0xb6e7_dd5e_54a0_9918, 0xeb9e_5a98_fdb8_5454),
            (10500, 0x22b5_ba40_8ed1_6bcd, 0xa4e7_e7c6_33d6_fefb),
            (12000, 0x84b0_be62_674c_042c, 0x7ca8_e461_25b6_0758),
            (13500, 0xb973_2c7f_0d5e_03a0, 0xb3d7_e381_df51_a491),
            (15000, 0x76f7_ff31_9629_157a, 0xf5ba_28b4_6cff_e9cf),
            (16500, 0xfe0b_2339_2398_f921, 0xf15d_0f0d_8589_115c),
            (18000, 0x188c_a694_b77b_8a19, 0x51c4_6a57_eecc_0fc2),
        ],
        last: 0x9023_4453_4712_7e86,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x928e_cd61_f16a_63dd, 0x07e8_ce39_aa3e_8be7),
            (3000, 0xf381_6e8c_2657_3cd3, 0xb34a_3a86_263a_f86c),
            (4500, 0xb2c2_d588_4405_354c, 0x75f6_5a68_7ea9_f33a),
            (6000, 0x9119_7717_0517_4ef9, 0xac56_e6b6_81ff_f8e7),
            (7500, 0x1c92_9bec_71bb_12b8, 0x2d78_69a2_d523_771d),
            (9000, 0x329f_b73a_6be1_9892, 0x8676_4309_2529_4682),
            (10500, 0x4003_4537_55ff_86f6, 0x3166_9034_3d4c_83bf),
            (12000, 0xea15_23da_dd39_3c75, 0x6107_3788_c9e1_8888),
            (13500, 0xcb8b_8680_2713_d174, 0x978e_4509_03b6_2705),
            (15000, 0xd191_5b1f_240e_ee3c, 0x5e60_b927_3e18_74aa),
            (16500, 0x76aa_53b8_648a_34b0, 0x5b8c_91db_9fec_232a),
            (18000, 0x1a57_d620_c3fc_4579, 0xcfbd_5603_a16e_fcf3),
        ],
        last: 0x729d_8be5_1682_96bc,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xe1c7_a565_f838_03d4, 0xbe98_08e5_0616_9311),
            (3000, 0x5777_6d67_b4d9_1c5f, 0xf549_7d2c_e185_e72d),
            (4500, 0x8939_2027_c979_7816, 0xf10e_1703_b969_4f10),
            (6000, 0x9a65_5a75_90cb_ed6a, 0x0c01_32a2_2c9a_cf8d),
            (7500, 0x401b_e949_9145_309c, 0x5b7e_7bef_e0a7_3bc7),
            (9000, 0x0d97_2eb6_7744_5abb, 0x95e8_355a_47b6_5069),
            (10500, 0x3bc2_103a_31f5_90cc, 0xa408_d1d9_241f_ab71),
            (12000, 0x3efc_daa4_2d3c_a3d9, 0x5842_234d_27fb_71f4),
            (13500, 0x7c42_98e4_6db1_839b, 0x379c_1ca8_6492_a36e),
            (15000, 0xe78a_f1ef_5b7e_d15e, 0x901d_c451_27e8_fc69),
            (16500, 0xbc1c_0eaa_19ab_c5bb, 0xe2b8_fd2b_bab9_091c),
            (18000, 0xb530_7e8a_b814_4156, 0xdd20_daea_8577_74b5),
        ],
        last: 0x298d_d761_7413_7e0e,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xd1f9_343c_9425_6ae3, 0xd67c_ace0_d65c_e35d),
            (3000, 0x6045_c2c4_17b3_7167, 0x95fc_dc9d_f7f8_b997),
            (4500, 0xf11a_86aa_c083_163d, 0xcc1f_267b_e2c9_0b76),
            (6000, 0xe865_f794_97a3_0941, 0x0644_36f8_d02d_73a6),
            (7500, 0x37f6_49b1_2358_2557, 0x1fff_d8ba_f005_8c86),
            (9000, 0x9078_65b9_f725_7f3a, 0x5044_dbc3_3ddb_e176),
            (10500, 0x786c_3d46_d675_8b60, 0x8c0a_7abc_3662_80b3),
            (12000, 0xd2e3_0d3a_3830_b3b8, 0x4626_a51e_d320_44c7),
            (13500, 0xffb6_cef8_a9c9_da1e, 0x0d6d_c61e_597c_ff10),
            (15000, 0x2d1b_5951_650f_4107, 0x1767_e549_a414_1394),
            (16500, 0x9ff5_1392_abc2_a1de, 0x3638_7a00_10ca_9735),
            (18000, 0x8cff_3c54_a6e3_127f, 0xf24d_e5fa_7924_7c1f),
        ],
        last: 0x5b88_89f9_458a_3206,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0571_29a0_1c95_5505, 0x40fa_0a63_2795_654e),
            (3000, 0xadea_b22d_29d7_9f4c, 0xc4d3_a2fa_2351_63f0),
            (4500, 0x8ebc_4590_5e19_4b3f, 0x14dc_5b3a_d1ba_c173),
            (6000, 0xda55_3ca5_44b7_5378, 0x05ca_ae14_7a89_fd5e),
            (7500, 0xec63_0cad_e747_0227, 0x6df0_9604_4db5_d2e1),
            (9000, 0xe304_3238_12c2_c20b, 0xff0f_540f_031b_07f4),
        ],
        last: 0x731e_e2e8_935d_6210,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd9bb_35c0_4dde_01e5, 0x7b56_590c_62f5_d9e8),
            (3000, 0xe5e8_7c55_dac6_b40f, 0x2c9d_88b4_da0b_9417),
            (4500, 0xa4eb_61ab_8687_cb7a, 0x0552_59a6_6426_e31f),
            (6000, 0xb6e9_2148_356d_551e, 0x3387_5685_beda_14d6),
            (7500, 0xb80b_7153_2e92_fd35, 0x1221_8f7b_26b0_c411),
            (9000, 0x9d09_1776_8e45_9468, 0x9c5c_c22b_8352_6527),
            (10500, 0xb31f_3909_33dd_7ef8, 0x4f59_2d0f_15e8_4666),
            (12000, 0xdbae_2189_9e66_403d, 0xcf98_8399_afdd_8713),
        ],
        last: 0xc499_feb5_3147_aaa6,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3983_4873_dd21_ba13, 0x1320_bd4b_ee05_5e59),
            (3000, 0xd116_88c5_9a8c_4aa4, 0x0ef3_035c_5eb4_93ba),
            (4500, 0x6f26_4ccc_c471_411a, 0xde74_6abc_408b_6694),
            (6000, 0xb08d_6860_3183_8a68, 0xef6b_4aca_5efb_6acc),
            (7500, 0x88be_48f9_cce9_d7f8, 0x8c39_f584_8f43_5c11),
            (9000, 0x75a2_3f0e_db4c_7be6, 0x3732_99a8_b3ab_2922),
            (10500, 0x53a7_b0a7_3adc_1d2f, 0xd9ac_34d6_ceed_9920),
        ],
        last: 0xc50a_7e17_6739_006f,
    },
];
