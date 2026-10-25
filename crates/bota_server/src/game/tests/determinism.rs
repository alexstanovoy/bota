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
        (1500, 0x7b7c_d552_78f3_41c9),
        (3000, 0x5640_df2c_c047_97cf),
        (4500, 0x0fc2_366c_553b_5d90),
        (6000, 0xf15c_de6d_84a3_7207),
    ],
    &[
        (1500, 0x01cd_8ca8_387d_db6e),
        (3000, 0x66dc_46fc_b997_7eaf),
        (4500, 0x56cb_68ff_dd38_ab32),
        (6000, 0x4ecf_9764_f7b8_f9be),
    ],
    &[
        (1500, 0x3e14_6246_1032_ff03),
        (3000, 0x62b9_f484_83c8_74af),
        (4500, 0x6b0d_f814_9bfe_afa4),
        (6000, 0x07d0_4b3f_50e1_13f5),
    ],
    &[
        (1500, 0x9e97_a874_0f87_4d20),
        (3000, 0xbd29_b00d_e3a2_74c7),
        (4500, 0x8d94_1236_d02b_e86c),
        (6000, 0x8cca_b5c3_0de8_f936),
    ],
    &[
        (1500, 0x04e9_448b_c935_319f),
        (3000, 0x19c6_02cb_a2e0_acab),
        (4500, 0x00c4_fe2c_1086_4390),
        (6000, 0x7dd4_cce4_02a4_7209),
    ],
    &[
        (1500, 0xcfe7_3e27_43b8_913e),
        (3000, 0xdc8c_f141_cb4f_bf6e),
        (4500, 0x4b95_dc4f_4e7d_3f2a),
        (6000, 0xdcd7_f916_9e54_ddf1),
    ],
    &[
        (1500, 0x89d8_7054_b2f4_8fa6),
        (3000, 0x9960_968f_6a60_4570),
        (4500, 0xe717_2336_dbf8_8c00),
        (6000, 0x6f5f_bd21_afef_e6ab),
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
            (1500, 0x6e05_66fe_7aa8_bb1f, 0x715b_f1c3_9a72_eaa1),
            (3000, 0xa246_928e_950a_f0bd, 0xe4e4_2bc1_32f3_dfb3),
            (4500, 0x3dd6_966d_ec34_8717, 0x5c75_a787_6ad7_4a1d),
            (6000, 0x8fa7_0e5c_4707_ca9d, 0x010e_b042_7d51_3c42),
            (7500, 0xef4b_d4ee_1249_472d, 0xf587_7437_2980_f85d),
            (9000, 0x8aac_8870_94f4_1599, 0x4047_91b9_f630_53e8),
            (10500, 0x574a_bc78_a75d_5ab7, 0x903f_6f6e_52ed_64b5),
            (12000, 0x2fe8_7d3b_a90d_6ca5, 0x5df1_73b1_a35f_69fe),
            (13500, 0x0e2f_1de5_929b_ced3, 0x55be_7e3a_1ed2_c05d),
            (15000, 0x1724_cc28_8dd2_d43e, 0x9c36_86f3_526e_ad7f),
            (16500, 0x1dbd_23ec_9426_bfeb, 0x00aa_96c1_e665_1093),
            (18000, 0xf75d_2672_4b2a_d44e, 0xb48a_7dc1_a4bd_1116),
        ],
        last: 0x3dee_f37f_a1c9_7f42,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xc538_4c78_8573_70c4, 0x95c2_e3fb_6790_82e4),
            (3000, 0xdc8c_4d37_230a_521d, 0x2c30_54ae_1d1c_d47c),
            (4500, 0x2c68_4417_a67f_8f5c, 0x9ae9_166f_b813_a93a),
            (6000, 0x75c6_ab57_3a77_71fb, 0x3da3_cac8_aecd_2283),
            (7500, 0x1513_83bb_c9f2_6765, 0xb877_1b72_704f_da35),
            (9000, 0xe9bc_cdb7_10f3_de78, 0x0bbe_2ce7_0c76_c01d),
            (10500, 0x398c_d3bf_f56a_a71d, 0xc646_c30b_d04d_727d),
            (12000, 0xac36_7e75_88c0_393e, 0x41e1_eddd_9fcd_c045),
            (13500, 0x74b4_8aea_2e28_a0da, 0x1506_54d9_9eed_5a39),
            (15000, 0xf591_ff2c_7199_ba43, 0x83ed_a0d8_d07d_6328),
            (16500, 0x6700_0262_e770_2abb, 0x853e_cbff_d100_bf89),
            (18000, 0xc20c_0b97_6104_6d6c, 0x1e2f_fe6a_cbfc_a24a),
        ],
        last: 0xee7a_c919_2d74_5e5b,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd7ca_5ced_8822_8ec5, 0x42b0_0e66_7eae_a341),
            (3000, 0x0908_950e_b3ac_9839, 0x5f15_4959_13cd_9ccd),
            (4500, 0xf5e8_09da_b80b_fdd9, 0xd7aa_d7b7_d77f_d0d0),
            (6000, 0xffc3_fe17_75bd_6ad5, 0x1ca1_c1e5_bad6_4510),
            (7500, 0xb2cc_28a3_e441_c00b, 0xf358_2b23_f212_dc54),
            (9000, 0x99af_dad3_4181_b736, 0xd22c_3ab1_4346_8cbd),
            (10500, 0x9b15_5f3c_55f8_edb4, 0x0260_0eef_0c4a_3b3a),
            (12000, 0x2a46_d1fb_84cb_ae09, 0x0110_df16_7314_453f),
            (13500, 0x3192_6abb_a7bb_970b, 0x7f3b_8a25_5281_0e56),
            (15000, 0xf57a_d5cf_cb9e_5cda, 0x9c9b_78aa_6492_6e67),
            (16500, 0x228e_cdce_6572_1bdf, 0x2643_7a37_40c1_6ab5),
            (18000, 0xa737_ab67_97e1_6beb, 0x42ff_b351_421d_9ece),
        ],
        last: 0xee0c_723f_7e24_13be,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xcc88_a604_940c_3d67, 0xcebc_fac7_9b2e_b4ff),
            (3000, 0x4f73_b590_361b_fd40, 0x9e72_12db_0751_deda),
            (4500, 0x8a73_adb2_98ca_67fd, 0xbecf_009c_e8ad_f725),
            (6000, 0xd06e_5da5_e9b6_5a49, 0xeb05_ffa4_79c7_fbf4),
            (7500, 0x07bc_154b_a9fc_7d13, 0x64ff_282c_092d_0647),
            (9000, 0xc28a_a750_4353_cb22, 0x1f80_9804_24e2_39ac),
            (10500, 0xe53f_133e_1000_32ff, 0xf041_93e9_9eda_5a2c),
            (12000, 0xbc8d_2399_fdc8_b16c, 0x7e86_b806_0923_06a7),
            (13500, 0xb1a7_3443_7d68_6084, 0xd3da_641d_2ba4_b73c),
            (15000, 0xc413_0f04_495f_bb15, 0xe43c_c1e4_9f45_5850),
            (16500, 0x78c5_7c66_588a_87c3, 0x1f22_4f21_f454_391b),
            (18000, 0x47b4_18e4_c1bb_c17c, 0xf1b1_7377_9e83_66d0),
        ],
        last: 0x1adc_a100_a9ef_e4d2,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xa4d8_1460_922f_0bc7, 0x6c84_6499_f116_55ad),
            (3000, 0x113d_a285_246d_6740, 0xb351_7b52_2422_0222),
            (4500, 0xa5f3_33d0_d6e6_7458, 0xb806_462c_ccf2_66fe),
            (6000, 0x4418_1e0d_edcb_a9fe, 0x5859_b6fa_ef35_9f0b),
            (7500, 0xed85_9615_aeaf_8fed, 0x2f60_fd77_b6f6_351a),
            (9000, 0xbf4d_693d_82da_fe3b, 0xa9bf_f759_d4c4_b548),
        ],
        last: 0x8a14_1588_d5b3_6286,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd9da_3a8b_d126_c0e9, 0x6e59_5106_4bb6_3e9b),
            (3000, 0xf88b_d6f3_f411_d1e3, 0xda7a_eec8_a1a4_8b1c),
            (4500, 0xae82_cd39_3fe0_d6e4, 0xf63b_01bf_47b3_89df),
            (6000, 0xb007_94e3_c6ec_37f5, 0xc243_a8e3_a956_db2a),
            (7500, 0xff46_4b19_58e7_2f77, 0x037a_a7a4_2711_c356),
        ],
        last: 0x4429_c1a4_7f63_c86c,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc258_82b3_24e5_febd, 0x02fb_9704_896c_443c),
            (3000, 0x4802_8eb5_51b6_82b7, 0x5310_84b6_93b9_1a65),
            (4500, 0x9f50_f0a0_386a_ba55, 0xa525_a6a8_4c5e_e06e),
            (6000, 0x2807_fd45_a003_0af7, 0x92ef_b229_2462_50b8),
            (7500, 0x9f18_a695_7d41_f9e6, 0xf41c_18eb_7432_c831),
            (9000, 0x3a62_ebc7_d699_55e8, 0xf9ae_8053_b855_d726),
        ],
        last: 0xb607_3b4b_27d2_ba64,
    },
];
