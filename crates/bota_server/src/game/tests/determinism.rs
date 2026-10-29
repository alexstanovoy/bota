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
        objectives: bota_core::map_of(pinned.map).default_objectives,
        runes: bota_core::map_of(pinned.map).default_runes,
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
        (1500, 0x3ad4_084d_bc75_3a9e),
        (3000, 0xffcb_8fee_1572_7cc6),
        (4500, 0xfe3a_106b_39b5_d4c8),
        (6000, 0xa3b7_fd54_a271_d609),
    ],
    &[
        (1500, 0x762b_fbb8_51c3_c5ba),
        (3000, 0xc3e2_9b55_0fe8_cac7),
        (4500, 0xe41b_a028_d170_be83),
        (6000, 0x7d36_7878_3cb2_e5e9),
    ],
    &[
        (1500, 0xa5a5_2d6e_90ee_a6e5),
        (3000, 0xc26e_9d7e_d0df_12f7),
        (4500, 0x4167_0808_da68_2908),
        (6000, 0xf154_881d_8edd_410a),
    ],
    &[
        (1500, 0xe9ad_68d3_65d1_dab2),
        (3000, 0xc08b_fd5f_5945_9c55),
        (4500, 0xb738_1a0c_0294_3269),
        (6000, 0x9abe_d81a_93df_ee1e),
    ],
    &[
        (1500, 0xac26_5c90_bff7_a6f0),
        (3000, 0xad7d_e9d1_6640_984f),
        (4500, 0x9a93_1cef_d976_8d99),
        (6000, 0xde63_5bf2_7104_1b8d),
    ],
    &[
        (1500, 0xc081_ad3b_6ad8_08f1),
        (3000, 0x4984_1135_b5ee_00fc),
        (4500, 0xc7b1_3886_0ea4_16c6),
        (6000, 0xd6c4_cdf8_7cf8_56ea),
    ],
    &[
        (1500, 0x5380_e6a5_e8b1_992d),
        (3000, 0xe4f3_9aa9_b8e6_11b7),
        (4500, 0xedf0_a966_0557_03eb),
        (6000, 0xa209_add5_984d_a28e),
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
            (1500, 0xb8c3_871d_f777_fcf4, 0xaedd_c97c_540b_19be),
            (3000, 0xa2fc_2165_222e_6c7c, 0x3acd_a720_a251_fc17),
            (4500, 0x530e_6f8c_d432_9acd, 0x5371_1f17_c301_782f),
            (6000, 0x1ce5_74e0_dd1b_3ce1, 0x9331_1c41_a6a3_077a),
            (7500, 0x8a90_4600_dd95_9ad5, 0xb57f_0543_bd46_222e),
            (9000, 0x0f53_5767_232a_d9a3, 0x6978_e3c6_25a5_b375),
            (10500, 0x721c_88a4_e356_0138, 0x2442_bfcc_730f_64cc),
            (12000, 0xc34e_f19b_e841_1814, 0xbdfe_243c_8430_782f),
            (13500, 0x348e_b5f8_bd3f_fef6, 0x8a8f_6c79_46c7_63b8),
            (15000, 0x83f6_876c_f8a2_8a32, 0xf3f6_0c7d_bd66_4450),
            (16500, 0x6ebf_800e_433c_5a50, 0x93b9_7881_926a_51c2),
            (18000, 0x2c68_64e9_dc66_18f5, 0x4441_b4f2_5c9c_9b1f),
        ],
        last: 0xcbf0_5f4c_9e3a_f2b5,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7b91_c0c4_203a_cd82, 0x9ce9_cc48_5fd0_d148),
            (3000, 0x2e20_f361_e330_aab5, 0x2eec_91c0_7dcf_fb40),
            (4500, 0xa4a4_5a59_187a_543f, 0x0a80_1f7e_60e5_cd81),
            (6000, 0x15e5_84d4_26fa_3491, 0x5c33_9d99_f496_5813),
            (7500, 0xf252_fd0b_9531_487a, 0x62b0_bd47_4ae3_03af),
            (9000, 0x2f48_1224_bd0c_6ffd, 0x9e8f_6155_e95e_0b0c),
            (10500, 0xf632_f051_8f2b_1f45, 0xce84_6b30_4e53_821d),
            (12000, 0x6b2f_8677_ead2_5f10, 0x1f2d_de6d_0c56_c0a8),
            (13500, 0x8813_b424_025c_4968, 0xcf39_f371_038f_4e41),
            (15000, 0x45d2_2564_4ec6_4221, 0x001d_3aa7_8716_0700),
            (16500, 0x2658_1781_753f_d147, 0x83e3_3586_b897_a327),
            (18000, 0xd4b7_2617_802b_0ec3, 0x293b_f390_3d88_b3bd),
        ],
        last: 0xf4ea_bf16_4dbc_a7f9,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xfba9_f98d_8368_360b, 0x84a4_0e7a_58aa_9568),
            (3000, 0xced4_1bde_d401_2a21, 0x2c74_3038_87d5_73eb),
            (4500, 0xcf7f_1959_ee41_cc96, 0x1231_1a59_609d_5423),
            (6000, 0xc102_b117_ae48_ad6f, 0xdd9f_0f69_5f7f_0721),
            (7500, 0xdc24_dedf_c247_cbc8, 0xe18d_dc0e_5a56_b560),
            (9000, 0x6440_a63e_a374_e644, 0xe8d9_b490_11de_b78d),
            (10500, 0x7b27_1597_d9e4_8bad, 0x0f77_c6e3_9e80_a647),
            (12000, 0x2d98_93f7_c91f_df86, 0xc0c4_a923_69bf_5b9f),
            (13500, 0xa3ee_f716_aa75_8ff5, 0xb53c_051f_3763_3016),
            (15000, 0x34df_0724_a568_2fdd, 0xbf1a_7feb_6a5e_25a2),
            (16500, 0x1903_07d0_6ac8_4183, 0x31e1_e741_d88d_69d0),
            (18000, 0x421d_8a71_6641_5fb9, 0x19b2_3e21_60b1_07f8),
        ],
        last: 0x8a4c_3e8b_9d1f_e850,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x3bec_fe67_e377_ec59, 0x50c3_3a5b_c1ef_5f57),
            (3000, 0x6073_8b79_6d79_a72b, 0xa30e_9154_6770_c2f5),
            (4500, 0x1d8e_2cc1_fda0_21ce, 0x8cfc_2aad_46ab_4505),
            (6000, 0x5b6f_2986_2e10_8a26, 0xaefa_a2d4_bb27_9921),
            (7500, 0x5428_3046_45ad_7dba, 0x8924_5644_07b9_6ed4),
            (9000, 0x7112_d97c_a4f1_f30d, 0x2f4a_d8ed_f9fc_673c),
            (10500, 0x7254_082e_2cd5_8a43, 0xf1af_434a_dc38_2b2f),
            (12000, 0x2a3c_87d0_3408_3cb0, 0x95c0_5549_3890_f103),
            (13500, 0x3d36_0a8e_efd8_463d, 0x5b6d_14ec_c501_8de6),
            (15000, 0xf75b_51ab_f488_066a, 0x305e_990e_242e_9c8d),
            (16500, 0x66d1_2917_4ffc_0788, 0x0aa3_3cc1_8710_4184),
            (18000, 0x5b8a_f455_8a08_b1f0, 0x9531_caa6_bc59_714c),
        ],
        last: 0xd66e_5044_4bc9_3811,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x313b_1a37_bdad_d838, 0xff79_55cc_db56_ca76),
            (3000, 0x820c_fdf0_1aae_89e9, 0x2608_d47f_81cc_dc5e),
            (4500, 0x4b67_b514_bcc6_8820, 0x9a8b_3014_470e_0f00),
            (6000, 0x97d6_b14e_5c5f_27c7, 0xd3b1_03fc_1d27_784b),
            (7500, 0x476f_2a6d_18b4_7599, 0xf84e_ec2c_8b52_d558),
        ],
        last: 0x03cf_3e69_e76e_5ae6,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5f5c_dcb7_2f42_e05c, 0xed3a_f854_559a_c531),
            (3000, 0x89eb_3a6a_5cf4_daa5, 0x41cf_5690_2fdc_28b5),
            (4500, 0x3cab_6385_b684_c28b, 0xfe11_3cf3_edef_a129),
            (6000, 0xf20b_5673_3b17_5d9d, 0x49e1_99be_4560_dffc),
            (7500, 0x93de_1743_8cda_b7b0, 0xbf79_48e9_36b6_9fc9),
        ],
        last: 0x4fae_04ff_4d70_2a24,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x1df0_df0c_11b7_f9f4, 0x15b9_90d6_1519_aef7),
            (3000, 0x841c_a2a2_ceb5_efd6, 0x3aa5_1da4_ca64_eaa0),
            (4500, 0x961b_b45b_1b72_40d6, 0x4dfd_bb5f_eb8e_5857),
            (6000, 0xe878_546c_5471_bc72, 0x3e0a_ce57_637e_2a61),
            (7500, 0x967d_3ecf_3af2_83dd, 0xaa13_0794_c29e_10e0),
            (9000, 0xc1ad_2a61_f394_41c0, 0x0f46_32b3_1089_8145),
        ],
        last: 0xe020_116f_15ca_72c8,
    },
];
