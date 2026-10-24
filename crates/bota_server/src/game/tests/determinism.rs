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
        (1500, 0x9167_6837_860b_f1d3),
        (3000, 0x5ecc_4453_be39_fb79),
        (4500, 0xbf7f_99bf_fb7e_b451),
        (6000, 0xdac5_63d3_17e8_32df),
    ],
    &[
        (1500, 0x5793_9f55_f17c_f629),
        (3000, 0x5e9c_70b9_7910_78f5),
        (4500, 0x57ee_79ca_4e0a_174c),
        (6000, 0x4bc3_2f45_ba56_610a),
    ],
    &[
        (1500, 0x2975_5b81_ec78_bb24),
        (3000, 0x287a_4ae7_9d1e_c639),
        (4500, 0x6e8f_a21c_4cdd_8d5d),
        (6000, 0xcf6a_50f8_384d_5ca3),
    ],
    &[
        (1500, 0x8f96_73ea_a640_62fa),
        (3000, 0x94b2_a1fa_4449_2d17),
        (4500, 0x4501_472d_65e6_f42e),
        (6000, 0x4919_9106_d0e9_8a53),
    ],
    &[
        (1500, 0x88c7_261b_59c2_b3b9),
        (3000, 0x3311_9841_55df_8998),
        (4500, 0x360e_2482_acf3_d12f),
        (6000, 0x7389_230f_e692_59fe),
    ],
    &[
        (1500, 0x53f3_cc9e_ef32_2941),
        (3000, 0xa420_a97b_9008_8843),
        (4500, 0xfe99_ab00_6d2f_e649),
        (6000, 0xf69b_caef_9f20_8395),
    ],
    &[
        (1500, 0x7c84_1915_6a99_f47d),
        (3000, 0xf8a6_d73e_9ed6_d763),
        (4500, 0x88e8_a273_b5ee_ee65),
        (6000, 0x4c5f_3816_b58c_2ca5),
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
            (1500, 0xf595_e25b_c004_9db7, 0x1a9e_39be_ce88_8caa),
            (3000, 0x2dda_2d1f_bf0f_ba76, 0x0b43_1172_869d_f1a8),
            (4500, 0x9989_c2ad_0772_f718, 0x6d4a_0b2f_1f71_0d98),
            (6000, 0x4ef7_2650_7302_427f, 0xe39e_9b89_2cdb_1ed6),
            (7500, 0xc193_9dcd_abcd_7f2b, 0xf8f5_9331_24f9_9096),
            (9000, 0x11a3_1952_b39f_c087, 0xc35c_a205_fd5e_2430),
            (10500, 0x688d_97ec_16c8_f081, 0x3c27_2c78_8279_3846),
            (12000, 0xb652_5224_1938_3b2d, 0xb17f_cebd_26c2_2705),
            (13500, 0x2a6c_5685_468d_a7b3, 0x8e92_8be7_e33d_becc),
            (15000, 0x41e3_2cbc_4e1a_3241, 0x9d37_9706_6601_ce9c),
            (16500, 0x0298_5193_49c0_4aa5, 0xa4b2_0d74_aa82_cd9c),
            (18000, 0x8003_193b_c712_ef4c, 0xefc1_4309_46ce_4577),
        ],
        last: 0xb568_d9e7_0308_2926,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x6c23_0b40_df20_2d89, 0x742c_fc98_6427_b77d),
            (3000, 0x1340_9c5e_dfcb_d136, 0x83c8_d647_e510_4b94),
            (4500, 0x4341_469f_bbf2_7e87, 0x2861_d429_1782_7e5c),
            (6000, 0xda0d_c82c_0815_89f7, 0x2870_298b_03d2_fee0),
            (7500, 0x2858_702b_ef7e_c745, 0x04e4_d329_2a28_db3b),
            (9000, 0xc468_bf95_5fc8_5167, 0x539f_20f8_bb00_9f8e),
            (10500, 0x992e_10e9_3203_e4a7, 0xf10e_8738_42cc_923a),
            (12000, 0xf698_9e50_6e83_5f02, 0x8073_2c86_d04e_1a15),
            (13500, 0xfa44_e273_7465_38d9, 0x35e2_2a9e_84c6_c61d),
            (15000, 0x7a56_3588_3d91_b4e6, 0x18df_fc1a_2ad4_8f03),
            (16500, 0x68b6_3880_8d48_f2b7, 0xe72c_eecf_b1a3_e268),
            (18000, 0x011e_8a82_f2eb_19e9, 0xc665_79ea_a39b_cd8e),
        ],
        last: 0xf589_3186_2167_af6f,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3217_830c_3b7f_42e1, 0x2fe7_736e_7c5c_326e),
            (3000, 0xf4de_8d0f_081a_b4ee, 0x479e_afdb_8298_7ca3),
            (4500, 0x376b_9326_95ab_27f6, 0xd753_ba62_536c_1990),
            (6000, 0x2b05_95f7_121d_9d4b, 0x4fbd_dae9_01ea_96b5),
            (7500, 0x6e98_4433_df3d_199f, 0x77b9_de2f_cf27_6bd9),
            (9000, 0x5c4b_9607_5d6d_1b7e, 0x3d7e_47a5_c66a_91af),
            (10500, 0xce52_6f16_a78f_e00c, 0x5a2d_7ede_d5eb_0054),
            (12000, 0x690a_8c18_051c_50ae, 0x2a73_a2f9_f0b2_9ac3),
            (13500, 0x66d8_0798_4796_e6bc, 0xb9d3_5fd5_1737_b0d6),
            (15000, 0xbaf7_c0b8_adf3_bc58, 0x755f_ff1f_84b1_7f3a),
            (16500, 0xf965_ad88_7833_f187, 0xa17f_e09b_4992_45e6),
            (18000, 0x0488_441a_ed74_0e1e, 0x56eb_6de2_a14a_37aa),
        ],
        last: 0xfeb0_4bf6_7542_5a1b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xf59b_08dc_3763_948f, 0x9eb1_60e5_fa31_d38c),
            (3000, 0x7780_7a55_8508_caf8, 0xaa16_f73a_254a_43d0),
            (4500, 0xea4d_2b25_d079_5646, 0x4b66_583c_4723_97a5),
            (6000, 0xd95e_7ec6_441d_d4ab, 0x162b_ba5c_a01b_087e),
            (7500, 0xb147_7972_c480_c1ad, 0xd71b_8374_1dd5_26b2),
            (9000, 0x0045_289f_28fd_9da8, 0x44c2_1819_772a_fe6c),
            (10500, 0x87cc_1360_fead_1250, 0xd150_8529_ddc2_8dc3),
            (12000, 0x25ce_47e4_2cda_68d2, 0x0bc3_663a_9f98_b139),
            (13500, 0xfaf3_b65c_e4ea_3d2d, 0xa80a_bd36_f4f6_a4a6),
            (15000, 0xeeb2_733e_398f_f0a0, 0x600d_7c44_dbe7_fff0),
            (16500, 0x02b4_9013_69d5_bfee, 0xf00e_abdb_1987_0f6c),
            (18000, 0x64e4_1194_4bba_7563, 0xf57c_d144_c9ba_7137),
        ],
        last: 0xdde4_fe5b_ae3b_ff90,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x226c_65ec_52ec_25f9, 0xb658_f123_2b87_bb16),
            (3000, 0x7d38_b105_a21f_0b23, 0x8d7f_245c_e88d_9c8b),
            (4500, 0xa98e_3d75_cba0_c493, 0x6d87_af34_5fc3_f29c),
            (6000, 0x853a_3549_92ee_d4b3, 0xb132_0c0c_f871_8986),
            (7500, 0x7200_b3f2_fa08_51c1, 0xb3a4_87d0_4db9_3586),
            (9000, 0xe3fa_720c_b6b3_e250, 0x3c90_8ec8_a77b_86ce),
        ],
        last: 0x7046_c848_fe6d_9496,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xfde3_75f7_7557_fa29, 0x36c5_d99f_38dd_14b1),
            (3000, 0x148a_b88d_1dd5_f3dc, 0xef44_0da5_c9b2_20b8),
            (4500, 0xc309_793c_a37f_cf49, 0xcfce_1049_88cf_876e),
            (6000, 0x37ba_6560_0189_9e02, 0xa4ca_ccae_fc0b_5e1e),
            (7500, 0x0132_8c5c_8cb2_59f2, 0x9d49_ffac_75ee_3666),
        ],
        last: 0x7e9f_4550_3292_bc5f,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf8dc_8986_6a3a_bed2, 0x416e_ef22_5f79_615a),
            (3000, 0x6ce3_1608_c104_bdf2, 0x0365_140a_d025_9df8),
            (4500, 0x4ebe_04f2_6a55_13d2, 0x2a63_2efd_fdf4_47d7),
            (6000, 0xd330_3e60_04a6_b253, 0xa4ea_5689_19cf_13e0),
            (7500, 0xc21c_d45e_5f22_f992, 0x869c_e5b7_3879_323e),
        ],
        last: 0xcfd6_c956_5552_24b9,
    },
];
