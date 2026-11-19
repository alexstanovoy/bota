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
        (1500, 0xbf5a_deae_6662_484e),
        (3000, 0x7fb4_3a5d_f3b2_41d5),
        (4500, 0x9d73_a539_bd60_8d4b),
        (6000, 0x68a8_8b1c_fd89_b5f8),
    ],
    &[
        (1500, 0x9336_82ac_640b_ac36),
        (3000, 0x2263_d529_f2e4_e078),
        (4500, 0xdf88_6ce1_3c9a_76b8),
        (6000, 0xf908_9ef5_bf8d_5fbd),
    ],
    &[
        (1500, 0x9e25_7f83_645d_c4fd),
        (3000, 0xc6c0_a0ac_047a_cd51),
        (4500, 0xc1de_38c6_5584_164f),
        (6000, 0xb141_9213_b5d7_99e5),
    ],
    &[
        (1500, 0x3b91_aa29_d9ee_886a),
        (3000, 0x07ce_7370_bdc5_ed74),
        (4500, 0xe659_b27c_05d8_7996),
        (6000, 0x4fdd_192a_dc1b_341a),
    ],
    &[
        (1500, 0x7b8d_b2bd_555f_a051),
        (3000, 0xd978_55b6_ac84_aa3c),
        (4500, 0xed52_f009_b5b2_b998),
        (6000, 0x2222_bb3d_3be5_ae25),
    ],
    &[
        (1500, 0x6b5c_b49d_07f5_e070),
        (3000, 0x413a_943a_e4ba_607c),
        (4500, 0x150b_a922_72cb_3e7d),
        (6000, 0xa87c_3e49_559e_2b87),
    ],
    &[
        (1500, 0x397f_c7ce_62fd_a66d),
        (3000, 0x8e45_d8bf_83b7_ecbc),
        (4500, 0xc217_d214_ed23_3409),
        (6000, 0x699c_9481_54cd_76c4),
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
            (1500, 0xd0d7_ec48_937f_c454, 0xfaf5_873f_bd2d_a55c),
            (3000, 0x2f85_51b3_44c2_4685, 0x81ba_a782_85a3_eb07),
            (4500, 0x4641_8e0d_5e8e_56df, 0x2260_53be_f4e7_6a89),
            (6000, 0x7438_2c19_c8dd_70f7, 0xe5d0_e2dc_0051_4fcf),
            (7500, 0x4d29_9bfd_76a9_e6eb, 0xf1ab_5014_f5d4_c548),
            (9000, 0xdf98_4ea3_d615_68d7, 0xf00b_978c_7de9_e5cb),
            (10500, 0x9a1b_109b_d8f4_cb92, 0xbb1a_714b_cf37_93c7),
            (12000, 0x3c12_44f6_25c0_9b0e, 0xc34e_0fea_9e44_87c8),
            (13500, 0x5df5_edd6_dc38_d6ad, 0x3bac_1721_d779_4f5e),
            (15000, 0x87f3_3726_1b35_6ffd, 0x57a4_e887_396c_9226),
            (16500, 0xcdff_7ed9_012f_99aa, 0x43d3_5c47_cd0d_2363),
            (18000, 0x6344_fd0b_4b35_cadb, 0x9c46_b697_a34e_d36f),
        ],
        last: 0xe03d_9047_52db_41e1,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xb86b_ff1a_6a55_5b86, 0xb76d_c6dd_97b5_7b26),
            (3000, 0xbc77_7c6a_b989_c0e3, 0xbe2a_b426_dde6_9d9e),
            (4500, 0xd29d_fc37_a709_e946, 0xce7e_c730_8f0e_3c18),
            (6000, 0x4eaf_dd37_d6dd_186f, 0xdb0d_7bb2_7b5c_4bf6),
            (7500, 0x76aa_0ca7_41cf_8676, 0x3386_395f_9290_de8a),
            (9000, 0x02ee_395e_3598_c8f0, 0xbcb6_46dd_7472_ca53),
            (10500, 0x4206_a95a_55d3_b789, 0xc22c_8dd9_c360_bcfb),
            (12000, 0xca4f_5fe4_c11a_7a10, 0x2fbe_08bb_97e0_d034),
            (13500, 0xb612_6c70_82ed_1511, 0x5a03_a11f_9f3b_1f15),
            (15000, 0x02fd_21b3_485c_f55e, 0xb0ce_0f32_96f2_0f74),
            (16500, 0xca5f_c865_9abd_5c1e, 0xaa1a_5894_4ffa_f1a1),
            (18000, 0xd4d2_234d_3ef9_f326, 0xaafe_39cf_5cd6_9b0f),
        ],
        last: 0x9a8c_4d5d_5855_a7b7,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xc45d_9bc8_9855_4ce2, 0xea15_7a62_f3e9_88da),
            (3000, 0x9e5c_4577_fec4_0350, 0x885e_f362_a463_af59),
            (4500, 0x6558_be4f_5ba8_02c2, 0xf1f2_d7d1_7a1b_afca),
            (6000, 0xc27a_6590_2f05_4752, 0xe142_9190_60c6_eef6),
            (7500, 0xdd6c_984b_cfaa_c7a5, 0xd06c_b1f5_7b84_17e3),
            (9000, 0x8c7e_4743_a045_e75d, 0xc07e_195b_e213_b028),
            (10500, 0xa160_2da5_dc44_91e5, 0x963d_6d97_a05e_8529),
            (12000, 0x8efe_3f79_e704_3b1e, 0x4f64_ec83_8c34_b420),
            (13500, 0xf858_e5e5_1c02_4e62, 0x9752_565a_a7e1_7b55),
            (15000, 0x66d2_b724_32cd_486f, 0x6d87_71aa_fbfe_95e0),
            (16500, 0x063c_1b25_2969_c346, 0xb7fd_7b25_8f7f_ac24),
            (18000, 0x2ecf_35c7_da06_e401, 0x9132_db22_9865_c124),
        ],
        last: 0xe11e_3cfd_e89d_3095,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x8564_1cf1_cdba_4106, 0x701a_a901_36f1_a06d),
            (3000, 0x7451_fe6d_b54e_9f47, 0x888d_d3a6_7978_ea62),
            (4500, 0x5ffa_ca2a_527c_ad2d, 0x6aa5_38e8_9837_56f0),
            (6000, 0xd1a7_2ffb_8653_118d, 0x925c_cdbe_bbab_fe68),
            (7500, 0x47e8_afd6_5406_4750, 0x013c_52a0_c723_40a5),
            (9000, 0xcef8_f38b_8a91_03d0, 0x6b9d_8a1d_8d8a_cb7e),
            (10500, 0x8100_d464_7136_1369, 0xcc96_ad12_1fd0_5904),
            (12000, 0xaaa9_e6a2_399b_3af3, 0xe12d_7e3d_1a62_e182),
            (13500, 0x2b77_db62_c0a3_616b, 0x9728_5fcb_e8d4_1d8d),
            (15000, 0x632a_8ec4_8662_e03d, 0x216a_4e76_60bb_63a4),
            (16500, 0x23d9_2d31_f161_b53d, 0x2fda_e557_11e1_3134),
            (18000, 0x5fad_dad7_de34_dc1f, 0x0744_4a37_8e3b_ab35),
        ],
        last: 0xc49a_941e_0de4_e136,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc322_f045_d32a_4e66, 0xb726_f743_ecc1_53ba),
            (3000, 0x42a1_c8c3_bf4a_74a3, 0x1276_8b8d_71a4_b097),
            (4500, 0x7ff2_54a2_61fe_fbd7, 0xac36_5f05_2602_08d9),
            (6000, 0xe821_eacc_55ac_d8f5, 0xc44e_7140_ce06_813c),
            (7500, 0x7abb_9575_7301_7652, 0x1a78_ea7c_a321_f0cf),
            (9000, 0x1565_35b2_3f49_5749, 0x1894_0eca_320f_a5fc),
            (10500, 0xe058_c925_3d6e_1114, 0xdc16_0988_bbc0_64f2),
            (12000, 0x1a23_13de_7180_8915, 0xa37a_e1b1_ef79_f0b3),
        ],
        last: 0x8505_475c_1837_6ec0,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf78d_c977_4b5a_6732, 0x4d06_3584_d841_01da),
            (3000, 0xb2d3_bdd6_f116_f1e4, 0x0601_3795_8da0_79a0),
            (4500, 0x86fc_467a_677a_00bd, 0x4b70_a380_4ecb_ec53),
            (6000, 0x10f2_9229_5663_982f, 0x2af5_5efe_63c8_6e7b),
            (7500, 0x52e6_9f50_df9b_2d51, 0xe156_3e4f_7a1b_c525),
            (9000, 0xa301_af66_062f_f83b, 0xc6f2_8a81_080d_8029),
            (10500, 0xaeb5_fb6d_b143_e4cc, 0x792c_6bdd_7e9c_3d4c),
        ],
        last: 0x0036_0a04_2d54_66d3,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x95f3_b4ca_62ed_fa7e, 0xac79_0db5_0981_0b73),
            (3000, 0x4d13_542d_2d09_cef2, 0x2cef_47a4_b2e7_fd05),
            (4500, 0x22d5_b194_6216_e0fe, 0xdba1_1469_cc80_3f32),
            (6000, 0x6b6d_cd6d_6fda_56d7, 0xbec3_c33a_0c0b_587d),
            (7500, 0x2b13_5117_10a9_e3d0, 0x9ac8_11eb_53b7_424d),
            (9000, 0x0831_5c81_8917_1d59, 0xaa6c_ea29_7497_482e),
            (10500, 0x09f4_16dd_7782_c379, 0xca1a_9103_187d_a610),
            (12000, 0x1b0c_62c2_3092_2566, 0xe0ed_614e_54f9_ca4c),
            (13500, 0x9b1d_8f85_1204_5936, 0xbd57_9429_2020_259d),
            (15000, 0x1745_a763_0cb6_db67, 0x2ba6_85c3_864a_0c09),
            (16500, 0xe3f8_b3ea_3592_f6ab, 0x4016_0988_defc_5db5),
        ],
        last: 0x0a4a_7add_948c_49d6,
    },
];
