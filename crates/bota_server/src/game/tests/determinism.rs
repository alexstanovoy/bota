//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_core::{ITEMS, map_of, rules};
use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, Fnv, MatchConfig, MatchSecrets, World};

/// Ticks between two pinned checkpoints.
const CHECKPOINT_TICKS: u32 = 1_500;
/// Ticks a match on a map without a cap is played for.
const UNCAPPED_TICKS: u32 = 18_000;
/// Ticks a debug build plays: the pinned prefix it can afford.
const DEBUG_TICKS: u32 = CHECKPOINT_TICKS;
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
                item: ItemId(self.below(ITEMS.len() as u64) as u16),
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
        tick_rate: rules::TICKS_PER_SECOND as u16,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

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

/// Replays the pinned matches on a map and checks them against their pins:
/// the whole match in release, the prefix it affords in debug.
fn check(map: MapId) {
    for pinned in PINS.iter().filter(|pinned| pinned.map == map) {
        assert!(map_of(pinned.map).id == pinned.map, "the map is registered");
        let whole = match_ticks(pinned.map);
        let horizon = if cfg!(debug_assertions) {
            DEBUG_TICKS
        } else {
            whole
        };
        let (checkpoints, last) = replay(pinned, horizon);
        let expected: Vec<_> = pinned
            .checkpoints
            .iter()
            .copied()
            .filter(|&(tick, _, _)| tick <= horizon)
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
    while world.tick < DEBUG_TICKS && world.victor().is_none() {
        if world.tick == DEBUG_TICKS / 2 {
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

/// Ticks the fog-blind replays run: one checkpoint in debug, four in
/// release.
const BLIND_TICKS: u32 = if cfg!(debug_assertions) {
    CHECKPOINT_TICKS
} else {
    4 * CHECKPOINT_TICKS
};

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
        (1500, 0xb03a_5dab_eddf_d642),
        (3000, 0x7434_e256_88d0_a71c),
        (4500, 0x44b0_5211_fb71_52bc),
        (6000, 0x2a75_0d11_7e12_2f5d),
    ],
    &[
        (1500, 0xb017_fb35_c53a_955e),
        (3000, 0x5592_dce8_6538_ac82),
        (4500, 0xf5a5_13a3_07bc_7a51),
        (6000, 0x247c_4057_98cb_fecf),
    ],
    &[
        (1500, 0xfcb0_6c20_4ac0_d493),
        (3000, 0xbae9_13ea_39e8_f245),
        (4500, 0x28b1_bc96_9a2f_4efe),
        (6000, 0x5a73_bf7c_d090_69f5),
    ],
    &[
        (1500, 0x37ee_15e8_abf8_8ca3),
        (3000, 0xd697_9f77_2a4d_f965),
        (4500, 0xe112_7b7c_fb81_05a2),
        (6000, 0x47cd_07bc_16ea_366f),
    ],
    &[
        (1500, 0x205b_1f63_eab6_ed4c),
        (3000, 0x4cad_75d8_764e_92e9),
        (4500, 0x2ce2_8607_d5b3_7ae2),
        (6000, 0x6246_46d7_874d_76a8),
    ],
    &[
        (1500, 0xbb2d_66b9_2a43_dfb6),
        (3000, 0x1d34_734f_4e6b_391e),
        (4500, 0x8833_5dd7_a332_5f87),
        (6000, 0x1b5b_2b88_ed61_174c),
    ],
    &[
        (1500, 0xf36b_bbf2_1bd8_ab01),
        (3000, 0xd89d_96c9_cd4c_06a8),
        (4500, 0x6ee7_5db4_8782_9cf9),
        (6000, 0x5d7d_934d_5a54_da89),
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
            (1500, 0xc29c_af6c_5b56_4d2a, 0x114b_2bfb_d803_6fdf),
            (3000, 0x3a7e_8f11_288a_2d58, 0xaf81_ff8c_b5bf_a511),
            (4500, 0x3fac_1d61_8d72_6c20, 0x09d0_6290_4c0d_daa7),
            (6000, 0xba7a_e67a_686a_d99b, 0xf649_648c_21ee_1562),
            (7500, 0xa7ca_8528_9b80_4a7b, 0x7ac9_da11_6d17_e0ae),
            (9000, 0x4382_979c_7da8_1958, 0xd9f3_8e1e_9abd_8a81),
            (10500, 0x3bfa_98e5_a2c7_a733, 0x1d2a_f4be_f8d1_2cc0),
            (12000, 0xfb1c_3227_2deb_fe94, 0x216a_77eb_39d1_a23e),
            (13500, 0xe212_d05c_4923_501c, 0xcb8f_9f4d_51ca_1a55),
            (15000, 0x1455_10d6_8f0d_dc63, 0x1712_f66e_0385_05ba),
            (16500, 0x0d4c_d37b_f72e_3392, 0x5ba6_f13c_409e_707c),
            (18000, 0x7bef_7db1_ad03_3b22, 0x0ad0_eaa7_1432_4bea),
        ],
        last: 0x32ba_86a7_93ee_af45,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xbb9a_2ca8_e14b_f4c9, 0x7d32_8eb9_bf4c_dfb2),
            (3000, 0x3036_5378_6447_9606, 0x6c58_bf19_8c29_cecf),
            (4500, 0x1bc6_955d_4783_aed1, 0x9eba_d68b_8212_adec),
            (6000, 0x962c_94bd_dd26_5729, 0x6e5b_6006_0785_7ebd),
            (7500, 0xfe47_9ff7_261c_2647, 0x6535_76cf_fed6_3730),
            (9000, 0x1019_5a1d_a954_fbc8, 0x7503_2fd2_0fd5_bd3c),
            (10500, 0xd7a0_5e90_cf2c_8b72, 0x55f7_4b4e_7b99_4a78),
            (12000, 0x7f4d_5b30_f2b2_b91d, 0x0fe0_1226_b8f7_1232),
            (13500, 0x51cf_02c2_6f0e_11cd, 0x2319_208c_1cd7_302f),
            (15000, 0xb33b_2504_7f3f_a35a, 0xd126_b02c_aeb6_972c),
            (16500, 0x94c7_77bf_5e31_0295, 0x9461_4219_75a7_b565),
            (18000, 0xf977_cba6_9510_9207, 0x0b36_7149_a7ec_fe02),
        ],
        last: 0x9b72_4ec8_81f6_e427,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaad1_6bbf_84f9_ac6c, 0x8e57_a362_129a_9f95),
            (3000, 0x7284_38fb_1d6e_f674, 0xba7d_c133_aa47_e5e5),
            (4500, 0x4b88_5fef_337a_abe6, 0x9771_02b5_1686_1a7c),
            (6000, 0xa35f_aedb_57c6_fcde, 0x1165_bf59_6d5e_1f5c),
            (7500, 0xbc07_5b87_e76b_e996, 0x0036_066d_7343_fc7d),
            (9000, 0x8aa9_ee45_4eb4_5ad9, 0xa3fb_2318_cf7e_cca2),
            (10500, 0x6418_bdd8_44a0_7bcc, 0x641e_6b99_3828_a62e),
            (12000, 0x429c_e7e8_1bef_0378, 0x5c83_4578_c5ce_ba2c),
            (13500, 0x84b9_464b_b301_5f1f, 0x2b21_1823_3d47_4250),
            (15000, 0xf0ce_19e5_85e7_7908, 0x3f0c_8b49_3874_a9f3),
            (16500, 0x2ad7_9c3d_19dd_986b, 0x229c_f641_d538_712c),
            (18000, 0x0766_c112_dfbd_0eb6, 0x94a6_9e6a_8f95_e28f),
        ],
        last: 0xcdd9_eadb_ba02_0225,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe302_3c28_9021_704d, 0xe10d_c2e3_22a2_6677),
            (3000, 0x6eab_e1f6_b5ef_0566, 0x0a6d_59ee_0d7b_8fdb),
            (4500, 0xb290_2a28_590c_9499, 0x10a7_4fb2_a58c_c92b),
            (6000, 0x8d85_fc23_fe3d_3087, 0x4bad_9c17_dcd8_1496),
            (7500, 0x5b0e_dc4f_9f3c_4e31, 0x6849_c097_9704_8df9),
            (9000, 0xa209_c234_e7d5_bd49, 0x4000_e7cc_15d7_30d4),
            (10500, 0xd924_363f_72aa_097c, 0x75b9_3aab_fe71_9905),
            (12000, 0x817b_1246_8141_7c9c, 0xd58d_ab64_a3ba_5159),
            (13500, 0xd00a_62f2_1ee2_7865, 0x893e_7eac_6faa_0c1e),
            (15000, 0x1f0a_a375_d213_6497, 0xbe1f_4fd2_881d_1b7b),
            (16500, 0xa2fb_b2fa_45d6_77e1, 0x2b2a_1f71_cf03_2dac),
            (18000, 0xbf00_76cb_b60b_05f6, 0xd0e7_d3fa_b6b6_32b3),
        ],
        last: 0xf47e_50e7_2132_d5ea,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xfdea_e5f3_1e9a_c79a, 0xe596_7071_4a3d_ae13),
            (3000, 0xc580_ea2a_6618_4e3f, 0xb404_a279_6057_3c6c),
            (4500, 0x3e07_d1d7_7eb1_841f, 0xc72a_bf76_74c0_4dfb),
            (6000, 0x7b92_6424_14e4_3205, 0x74ad_b688_be03_b022),
            (7500, 0x3ce4_1b9f_b478_f01e, 0x3c91_b14a_76b7_6c6d),
            (9000, 0x499e_ff06_949e_e863, 0xb212_eafb_a137_b60f),
            (10500, 0x29a7_79a5_6b21_b5c0, 0xfcf2_454a_5844_d20f),
        ],
        last: 0x0f3e_18e5_3b78_4883,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbb4a_0f5b_7377_4c88, 0x9ef3_c317_a860_5229),
            (3000, 0x1009_2453_f47f_36d0, 0xaa55_fc24_b872_0dd4),
            (4500, 0x5db9_35b2_a22c_adf1, 0x95e6_4ffc_31c1_da1a),
            (6000, 0xfa33_95df_e1c4_8c7b, 0x3439_5588_0079_1c5b),
            (7500, 0x3ad2_7c92_32fe_6131, 0xe1fb_e0b1_9aa7_2c56),
            (9000, 0x142f_17db_31dc_d38e, 0x984e_1a07_e888_04a6),
            (10500, 0x7d91_eade_71fc_ac59, 0x3ef2_7497_6f4b_6989),
            (12000, 0x68e6_4a8e_925f_a40c, 0x6edb_57da_2dec_3fc0),
            (13500, 0xdd8f_a675_35f9_9393, 0x5b4f_78af_d9b3_6385),
            (15000, 0x910a_6b08_a102_a936, 0x6d90_f0b4_74c3_9aa0),
            (16500, 0x4bae_ce5d_1e85_be07, 0xeb0f_7315_88a7_d80f),
        ],
        last: 0xbef7_bdbc_8093_a628,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x93b6_924c_e0e3_5d4e, 0x90e5_b0d8_91e3_09d5),
            (3000, 0x6fdb_d1fb_51cd_cb67, 0xe0ce_ee2e_6a87_4a64),
            (4500, 0x29b3_a6b3_b2cb_a6db, 0xa834_f3c2_3714_3e44),
            (6000, 0x46c9_fa9d_d78a_1903, 0x8489_e51c_dc91_b21d),
            (7500, 0xc30e_e44a_e293_ab0f, 0x7c17_e83c_1a2c_88ce),
            (9000, 0xce4a_643e_d673_51ba, 0x4f5e_6d0f_69f7_3586),
            (10500, 0x5e7b_1b5c_5d7a_35ed, 0x931b_6c8b_1b3b_dc82),
        ],
        last: 0x758f_3d16_25d4_862b,
    },
];
