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
        (1500, 0x5b6d_47c9_2cfb_a733),
        (3000, 0x28c8_94d4_b140_fd0a),
        (4500, 0xcecd_e564_82fd_dd4e),
        (6000, 0x1dde_8152_5923_fbf9),
    ],
    &[
        (1500, 0xb73a_af81_9853_b986),
        (3000, 0xbc3d_ede4_23e7_f1ff),
        (4500, 0xbdf9_ef50_d1bf_e16b),
        (6000, 0x678b_6ee5_f021_915e),
    ],
    &[
        (1500, 0xdc73_54ba_076f_16c8),
        (3000, 0x4877_e32b_36c1_2857),
        (4500, 0xdf21_9151_e105_4113),
        (6000, 0xe019_3728_eeb5_75fb),
    ],
    &[
        (1500, 0x50fb_b83c_68d8_2369),
        (3000, 0xc4a4_2d26_7eda_7962),
        (4500, 0xe18f_a456_dab7_f358),
        (6000, 0xb7ff_4c81_aba7_8232),
    ],
    &[
        (1500, 0x2ff1_daac_408d_5dd3),
        (3000, 0x7593_11d7_2da5_e790),
        (4500, 0xb9f0_68bd_72d9_7ff7),
        (6000, 0x301a_1757_8e47_8e1e),
    ],
    &[
        (1500, 0xd4ef_48da_6bbd_51f9),
        (3000, 0x0188_3497_e20d_92f8),
        (4500, 0xbc83_e8ce_9373_750c),
        (6000, 0xd536_cc7f_122d_e7d7),
    ],
    &[
        (1500, 0x8c05_9143_f25f_9909),
        (3000, 0xfb7c_b13f_11d7_7213),
        (4500, 0x0959_a80a_5686_133c),
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
            (1500, 0x5eb5_36e1_cba9_d3c8, 0x450a_8c72_7ffa_c23b),
            (3000, 0x5219_1d94_bbe9_99dd, 0x0982_5832_3a0c_73b4),
            (4500, 0x1d81_5c75_c0f8_4b88, 0xe5fa_337d_b6e8_b895),
            (6000, 0x09ad_f8da_5fd6_dbc4, 0xfb8f_296d_e8e8_6193),
            (7500, 0x8f13_dd43_b22a_b866, 0xbad7_940c_a4a2_75b8),
            (9000, 0x79b5_20a2_8ccd_6516, 0x4a3a_34ad_4688_3324),
            (10500, 0x6c5d_138a_ab21_610e, 0xc88a_91ec_ac5a_c4ad),
            (12000, 0x055c_8019_fb7c_8d9e, 0x0ff3_19af_b02c_fa75),
            (13500, 0x62a7_b8ad_cbbe_6207, 0x69ef_df5f_d63d_d573),
            (15000, 0x28e9_b634_df05_b956, 0x8816_f1c5_81a0_dcb1),
            (16500, 0xec62_352b_6cc3_cf69, 0xa235_5239_dc88_3ec5),
            (18000, 0xdda7_9b1d_9756_25f4, 0x59b2_a889_fb0c_cde0),
        ],
        last: 0xe4fe_eafe_bd3d_7ca9,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x0e5a_a05b_c587_9fce, 0x2f01_048d_35c3_5ee1),
            (3000, 0x2e83_c320_c477_5c62, 0xc9d5_3193_2e7e_87a4),
            (4500, 0xb94d_8175_c77e_9100, 0xbe0a_5ef3_d0f3_1d3b),
            (6000, 0xbbf2_e3ac_63a4_d5dc, 0x80e3_e49f_acca_8752),
            (7500, 0x57d0_3189_11cf_a21b, 0xddbc_51a4_8ad9_343e),
            (9000, 0xeeae_6133_f1a2_243c, 0x9830_b113_dcb7_52e1),
            (10500, 0x82a1_c059_c876_4fc0, 0x8959_8e1c_8b0f_a23e),
            (12000, 0x0d85_258c_70a5_6b6d, 0x9109_e558_1f9a_e005),
            (13500, 0xd5ba_88c0_92d1_b757, 0xc3c4_0338_2687_734e),
            (15000, 0x8f6a_4a09_8353_086a, 0x8c4d_6588_69fe_39fe),
            (16500, 0xc5a3_c59a_3e86_3b2f, 0x0a79_2ee3_6ef7_0705),
            (18000, 0xf879_2da7_7d16_8dca, 0x580b_d58a_25ac_ad79),
        ],
        last: 0xc62e_07c5_148a_abba,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xa7f6_b1d6_7ed5_e164, 0x8221_7c83_b9b9_d584),
            (3000, 0xd2ae_0ff7_646e_c999, 0x454d_5b23_c42f_e737),
            (4500, 0x1e10_deae_245f_93b7, 0xa041_2744_f217_07a3),
            (6000, 0x0760_aa26_cafe_ddd1, 0x8cbc_c54f_a804_1593),
            (7500, 0xe350_4be8_8a29_a4cd, 0x34cd_37b1_96d0_6166),
            (9000, 0xcada_64b5_e72c_4800, 0x3088_09c1_aac1_3044),
            (10500, 0xded0_52f6_ddf7_2a68, 0x6551_fb81_ff09_5823),
            (12000, 0x407b_2b5c_a33e_9f8e, 0x373d_f3c8_95d4_fd21),
            (13500, 0xf852_6b18_6d33_2716, 0x6315_94b8_003d_3808),
            (15000, 0xafdf_0b85_dd47_58d8, 0xbec0_a472_7fad_e635),
            (16500, 0x7af7_a40b_6458_c440, 0xf436_8228_5c67_c59e),
            (18000, 0xbb42_a2d2_c2b9_c1f8, 0x0fba_ce0b_a99e_dac6),
        ],
        last: 0x3e8b_c6d2_9145_2de4,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x7d0e_2ff3_58ff_cad7, 0x171b_a06e_f5ec_b249),
            (3000, 0x5019_02f8_badb_a74f, 0xfbb9_ae7d_0f76_b6f9),
            (4500, 0x5710_a370_721e_7df2, 0x8d1d_7d4b_5bd8_66aa),
            (6000, 0xc389_8013_ec64_7344, 0xaaad_709b_d953_34da),
            (7500, 0x8fba_5a4a_7893_6e30, 0x9af4_0e6c_8bce_e627),
            (9000, 0xccbd_fb91_0924_38e0, 0x904b_ebfe_9629_f083),
            (10500, 0xf382_f113_7992_be7f, 0x50e6_5354_15b7_734b),
            (12000, 0x6070_ab68_1906_53f4, 0x85a8_af42_361a_daee),
            (13500, 0xd97e_8d85_6254_b383, 0xd522_778c_f821_4665),
            (15000, 0x0220_5d97_be35_7862, 0x6af2_eb59_1d63_e108),
            (16500, 0xc721_726f_fad0_37df, 0x628d_9dff_f89b_e405),
            (18000, 0x8e01_5884_aaa5_c0a9, 0xfa14_b403_94f8_5747),
        ],
        last: 0x5da6_fadb_c366_aaae,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb823_7434_4491_9e30, 0x1e6b_cb98_bb0f_529b),
            (3000, 0xfa46_ef39_1dff_a255, 0xefb5_87d3_d4ef_ad01),
            (4500, 0x989c_5d23_a054_4c33, 0x2d1c_7af1_fbec_ede9),
            (6000, 0x82ef_9a78_da66_0655, 0x66ea_1036_12d0_8a95),
            (7500, 0x3593_86b4_17cf_fd60, 0x15a2_6517_ade4_4351),
            (9000, 0x8a21_b5b7_c7f9_a225, 0xec7b_b195_19b4_ec81),
        ],
        last: 0x468f_8dc8_cb42_cce6,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x36c7_577e_ba90_be64, 0xb900_96e6_5651_b232),
            (3000, 0xd463_150f_b01c_4db2, 0xb25f_e711_fed8_8ba5),
            (4500, 0x6ef4_6880_748a_0fc6, 0x74b0_cbea_f6e7_8513),
            (6000, 0xf5ff_8e99_b699_3ed8, 0xee75_5e8d_3f04_be37),
        ],
        last: 0x8c34_0962_9d17_f56c,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3a26_a426_ae8d_0bd4, 0x23b3_7312_5ac7_0063),
            (3000, 0x19bd_3e0d_27f2_99a4, 0x0cc0_39e3_60d7_9275),
            (4500, 0xd489_5aff_61de_c814, 0x801c_6b7a_14f7_1425),
            (6000, 0x7a60_6cd1_573e_c042, 0x3773_275c_9e94_0011),
            (7500, 0x485b_9c21_347a_4e57, 0x5363_877f_d6fe_4e76),
            (9000, 0x7c6f_2636_dfa6_e1a0, 0xa391_d48c_03d5_f6bb),
            (10500, 0x1c94_67d4_bde9_368c, 0xd02f_f01d_602a_4590),
            (12000, 0x6340_01f0_83c6_a356, 0x2f60_65bd_04a8_1e25),
            (13500, 0x2ab5_46d8_4776_6e96, 0x0b50_acb6_438c_5a94),
            (15000, 0x5a83_bd68_e595_f5c5, 0x5332_67bc_9a3b_c50f),
            (16500, 0x2d89_8da0_2c55_881e, 0x7d70_fe12_0bbd_ff2f),
        ],
        last: 0xffec_0b19_ab9c_3c71,
    },
];
