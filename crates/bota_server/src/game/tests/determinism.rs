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
        (1500, 0xb003_239d_268a_69a4),
        (3000, 0xbadd_8a59_85da_1903),
        (4500, 0x0a54_e334_3e98_e5f7),
        (6000, 0xbed5_b63a_b855_8be4),
    ],
    &[
        (1500, 0xff5a_9cec_25b9_7861),
        (3000, 0xcaab_144c_5858_78ea),
        (4500, 0x5af2_d99a_f7f5_a212),
        (6000, 0x7738_848a_c0c4_5ead),
    ],
    &[
        (1500, 0x6802_fef6_87e7_74a5),
        (3000, 0x33c4_0c04_dfa7_7589),
        (4500, 0xb07d_c9e7_390e_ed42),
        (6000, 0x33a7_88d2_b286_4ea6),
    ],
    &[
        (1500, 0x3f13_c0c1_93c3_037b),
        (3000, 0x5789_4ee8_c2b4_9676),
        (4500, 0xa36a_f0e4_759a_628b),
        (6000, 0x3d22_74e6_14c1_2aca),
    ],
    &[
        (1500, 0x2e9e_5683_937a_1fcd),
        (3000, 0x4bc5_5a6c_bb59_bb0f),
        (4500, 0xfcfb_6ced_0db4_f0dc),
        (6000, 0x265b_8cc1_4e53_1582),
    ],
    &[
        (1500, 0x5ac7_a155_510e_f742),
        (3000, 0x7ee7_49f3_81d3_fca7),
        (4500, 0xa72c_95f1_32f5_81d5),
        (6000, 0x8280_7e8e_877a_db42),
    ],
    &[
        (1500, 0xb7f5_3600_562d_a670),
        (3000, 0x2a4c_c6f6_5c00_22ef),
        (4500, 0x5428_f407_ef6a_29b0),
        (6000, 0xac89_582f_5e7c_edf1),
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
            (1500, 0x1f94_f2a2_de6e_417a, 0xf5ce_efa7_76d5_ff54),
            (3000, 0x5a92_f950_ee66_6dbb, 0x6fce_5fbc_9e85_2bf3),
            (4500, 0x3e67_a6bb_8a8d_8cd6, 0xfc63_b8c7_97cf_0df5),
            (6000, 0x8ab2_4520_95ef_f430, 0x3897_871a_1dd8_a449),
            (7500, 0x1a6a_8c11_8ce2_5f7a, 0x2ffa_5834_c440_1353),
            (9000, 0xc42b_eace_ee0a_b567, 0xcda1_517f_991c_2946),
            (10500, 0xda64_2fa8_011a_0e6e, 0x72a6_ece7_4cec_efb4),
            (12000, 0xbfef_973a_99da_296c, 0xcf97_7350_5388_a829),
            (13500, 0xf6d0_f458_c12a_4046, 0x7a77_89b0_07aa_5d9a),
            (15000, 0xf5a4_5b9f_de82_0492, 0x8961_f9e2_fb91_9f69),
            (16500, 0xc949_68d5_b827_117b, 0x2ca0_a468_4827_59be),
            (18000, 0xc9f7_ef80_e90d_a173, 0xb922_af13_e067_f67d),
        ],
        last: 0x8a99_6217_f134_18da,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xde72_aa2a_894e_6d3b, 0xd394_d966_6469_fa50),
            (3000, 0xfb37_f147_5563_932a, 0x6ee7_ad5d_b0ae_33f2),
            (4500, 0x652a_3b5c_c971_c4cc, 0xb271_31f7_e1de_4d57),
            (6000, 0xc892_90f5_34e2_225b, 0x0cb9_bc91_0377_c6d4),
            (7500, 0x1587_fccf_b5ba_4245, 0x8f9c_874a_0b6c_84a5),
            (9000, 0x5510_9293_8fba_5472, 0x1a22_075c_7661_2675),
            (10500, 0x3240_a746_0430_7a2a, 0x3250_dc87_529a_9251),
            (12000, 0x0d73_6d07_30db_1c9b, 0x7cc5_8468_d4da_ecc5),
            (13500, 0x74fa_7a59_467c_b480, 0xa755_8d6f_ef38_151a),
            (15000, 0xf3eb_88c6_9608_2481, 0x7b4f_6bf8_ad6d_9d1c),
            (16500, 0xf2e1_0723_c29f_a95f, 0xe623_ebd7_877e_64ef),
            (18000, 0x72dd_1ecc_aade_c2f4, 0xf408_eef1_dfe6_96dd),
        ],
        last: 0xefaf_8baf_6992_c435,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x4dbb_fab6_5cb9_76f2, 0xdbc5_33ec_8d09_1441),
            (3000, 0x6169_4399_64dd_2b90, 0x530a_a0f8_b472_13ad),
            (4500, 0xda11_4151_cc37_fec3, 0xb79e_0e46_64ac_7841),
            (6000, 0xe7b6_af5a_3ae2_2ab2, 0x97af_8d7c_f3be_5569),
            (7500, 0x4f2a_f1bb_91dd_3cce, 0x22a4_bed1_d598_c43b),
            (9000, 0x9e6b_aaff_6838_8629, 0x901d_994c_1a67_834f),
            (10500, 0xce00_8b9a_4c8f_6914, 0x569f_a2a6_74a5_5899),
            (12000, 0x1b05_7ab5_2277_5e1c, 0x85cd_ef48_74e4_d884),
            (13500, 0x7757_71d3_5cc8_6c43, 0xf585_0d71_e775_0004),
            (15000, 0xe104_e485_09e5_ae8e, 0x75a2_b489_033d_b1d5),
            (16500, 0xbcfe_b3dd_70fd_97a7, 0x2168_6912_7103_a5a2),
            (18000, 0x17fa_adc8_e735_7c09, 0xd870_d092_1392_e21a),
        ],
        last: 0x2ff8_0427_16fe_6a79,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xd11c_0a1b_ad8b_1fee, 0x2de1_1656_b266_71b8),
            (3000, 0xa28b_db2c_1020_14a3, 0xb00a_a074_c44f_1daf),
            (4500, 0xae81_e8e2_d4ef_673c, 0x9fd0_6895_7cc3_02c9),
            (6000, 0x5e62_08d0_ac0a_65a3, 0x2a22_fc66_e322_d7b0),
            (7500, 0xe730_c2df_047b_a78f, 0xb39b_e599_daf7_c699),
            (9000, 0xfa4b_1b0b_7b9b_cf59, 0x723e_59c5_914d_7395),
            (10500, 0x4bb9_4c2d_51e6_f69b, 0x5008_365d_859f_738d),
            (12000, 0xbd31_d91c_30a5_88cc, 0x483d_e511_d2a3_58f8),
            (13500, 0x626e_d0ff_5391_6011, 0x91fc_1e81_c7a6_62b0),
            (15000, 0x92bf_0688_28c3_82d8, 0x83d3_df88_cb35_13db),
            (16500, 0x7dd4_bf14_8b1b_57f1, 0xf989_aad7_df61_2a5c),
            (18000, 0x68d7_d156_67ad_0b9d, 0xe0cf_1f96_f693_0fca),
        ],
        last: 0x1cbf_70e1_e8ff_087a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xa340_15b2_e3c0_3265, 0x0e3a_bd42_8a5f_89ca),
            (3000, 0x626d_3ae9_84c2_2f14, 0x8a47_fb9c_1f20_75cf),
            (4500, 0x9278_0213_52df_fb0b, 0xba1c_1a29_b368_67bf),
            (6000, 0x2ba6_b4b3_8e12_1128, 0x4a44_a969_ada6_1b12),
            (7500, 0x5bfd_c9e3_0eaf_7e58, 0x9e98_6b5b_c6c5_e2b2),
            (9000, 0x1367_6032_8fd8_a326, 0xa60d_cf30_7562_c3a4),
        ],
        last: 0x5785_8f87_1300_b604,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbe92_03d5_bfbf_e705, 0x3095_f779_6eb2_34e1),
            (3000, 0xd18a_2ad1_be9a_c2cf, 0x39ae_3128_1ad3_1ac8),
            (4500, 0xc7a0_db08_552a_d30b, 0xa11b_ce9c_fa8a_31b3),
            (6000, 0x4034_1710_b6b4_e201, 0xbe23_b4fe_c7c8_5cf9),
            (7500, 0x623c_4d5e_929c_20e7, 0x8688_7743_2ef3_440c),
            (9000, 0x7757_90bb_b312_d7b0, 0x7f7a_ea86_0441_8dc4),
            (10500, 0x4943_dd05_aa55_3c5e, 0x4c3a_3495_9321_0438),
            (12000, 0xbc5b_a904_2ba4_62fa, 0x1687_646d_c42f_5e1c),
        ],
        last: 0xe2ce_f093_f324_4f63,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbc97_94b6_4449_2893, 0xfdc8_e592_5f8f_ee6d),
            (3000, 0x28ea_75a6_b9d9_1afc, 0xca16_bdb2_848a_16f6),
            (4500, 0xa5c2_a2bc_28a5_0534, 0xb317_ca5b_8335_eea0),
            (6000, 0xd403_c25d_8078_a36d, 0x9311_182d_b0d7_ddd1),
            (7500, 0xcd71_0e7a_cd17_9c58, 0xbb56_7d21_b851_ed0e),
            (9000, 0xd6fd_5546_4c05_c286, 0xf4ff_d0a1_e794_d338),
            (10500, 0x9752_6269_62ef_2cf8, 0x4e04_30e6_55f5_597d),
        ],
        last: 0xd2c7_62bf_5c74_f79c,
    },
];
