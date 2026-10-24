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
        (1500, 0xa79e_62b2_7e22_64ce),
        (3000, 0x8f33_67cf_2719_a280),
        (4500, 0x79a0_1294_edf0_9f2d),
        (6000, 0xf294_69ec_d1c3_e28e),
    ],
    &[
        (1500, 0xd9fc_5329_a20f_28c3),
        (3000, 0x291f_9a3c_8fdf_5185),
        (4500, 0x4a95_e58b_7f31_f857),
        (6000, 0x66f6_a4f5_69dc_2860),
    ],
    &[
        (1500, 0xe0a2_22d7_da21_46a3),
        (3000, 0x87ba_86e0_f580_e298),
        (4500, 0xa074_d9fb_866e_54eb),
        (6000, 0x3a1f_9188_91de_1497),
    ],
    &[
        (1500, 0x6d8c_d00b_c941_ca2a),
        (3000, 0x2b5c_b970_4c29_ac2b),
        (4500, 0x9765_33cb_d6e9_2c62),
        (6000, 0x07d1_5c9e_43ff_9dda),
    ],
    &[
        (1500, 0xbd95_0429_5980_3b06),
        (3000, 0xe019_2d7e_3889_7032),
        (4500, 0x1e37_1a25_ab77_94c8),
        (6000, 0xc03b_c8b7_fc0e_3c76),
    ],
    &[
        (1500, 0x55bf_e927_7fb7_ec42),
        (3000, 0x7ab7_48b5_bae8_d607),
        (4500, 0xdb25_fda1_212a_14aa),
        (6000, 0x1ca4_3bac_61d9_8bc0),
    ],
    &[
        (1500, 0x6f50_9c93_1b43_ebf3),
        (3000, 0xd1f7_c76f_c210_8eb5),
        (4500, 0xc245_f732_7eae_b1ed),
        (6000, 0x748f_1f6c_592a_5036),
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
            (1500, 0x6b5f_f407_a394_f036, 0x5a7b_a73b_eb82_9830),
            (3000, 0xb79f_d4c3_f241_c648, 0x62a3_4407_f6ce_7bcf),
            (4500, 0x2fb6_089e_8afc_9bc3, 0x3d99_2d80_f7f6_136f),
            (6000, 0xd628_a9f7_c5a4_4f17, 0xef16_f1bf_fb77_1f66),
            (7500, 0x0d2d_7fbc_1dd5_e30c, 0x1de1_b357_6ded_ce2c),
            (9000, 0x126d_6e1a_9d51_cc79, 0x2b00_54c4_58d4_e3c3),
            (10500, 0x6a64_473c_a21e_7092, 0x4fdc_0936_c5f9_5ea6),
            (12000, 0x9b60_27d6_9a45_75d2, 0xe392_2d69_ae14_6ed0),
            (13500, 0xf9d6_0e41_4646_f694, 0xf504_5119_ece0_2135),
            (15000, 0xed08_296f_f44a_ebd3, 0x13c4_1d4c_bf31_e80d),
            (16500, 0x28ce_2dc4_774a_791d, 0x0a2c_a654_c8f2_9cb0),
            (18000, 0x1ed3_afc2_058b_5005, 0x272b_d648_2a34_3da7),
        ],
        last: 0xa10c_e04f_3585_d990,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xc450_c14b_afae_fa63, 0xf178_2841_6a3d_0cc9),
            (3000, 0x3ee0_b4dc_d93a_44d5, 0x57bb_e590_1102_e1fc),
            (4500, 0xc901_5890_fa73_7849, 0x41b3_a932_62a4_fb70),
            (6000, 0x8f80_93bf_ca6e_d303, 0xeaa5_030b_cc9d_a5f2),
            (7500, 0x2d2b_5594_c2b0_6914, 0x64e2_16a8_06ba_efb8),
            (9000, 0xa30b_6a9d_d31a_3ae3, 0xc879_945c_ce75_c406),
            (10500, 0xadba_f5d5_bf1e_29f1, 0x72e4_5453_8292_bd39),
            (12000, 0x594d_89ce_1c30_d88f, 0xc41e_5d9d_3c3f_7522),
            (13500, 0x3f5a_da68_35e6_5d2f, 0x7d95_29c0_e73a_6fd8),
            (15000, 0x9e84_b3d6_15f1_dd59, 0x50cb_fcb3_d6dd_5e72),
            (16500, 0xae1b_66cc_25c3_c5e5, 0x64a0_73d3_38fd_2a9b),
            (18000, 0x6a13_73b4_8b70_2886, 0x6b70_66d9_be67_8780),
        ],
        last: 0xba44_face_8ba2_f80d,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd717_835d_394e_877b, 0x9f85_b185_42e5_2b64),
            (3000, 0xba7e_b894_2cb9_ffdd, 0x173a_a757_da46_27c1),
            (4500, 0xf6b7_c9de_1111_5543, 0x43c3_1fb9_509f_85b8),
            (6000, 0xc1fc_07e7_2f74_9d8e, 0x20ac_ef78_ac2a_f79b),
            (7500, 0xaacc_8559_555a_c4db, 0x4a6b_b6e3_ab3d_93c9),
            (9000, 0xe586_ff05_8549_fb6f, 0xd3e8_5f61_9c34_7acf),
            (10500, 0xa660_c443_4d6c_395c, 0xae1a_694e_1ce5_4c07),
            (12000, 0x3571_ca8d_5e46_1e4e, 0x485b_7a69_868c_b69c),
            (13500, 0x691c_fdfd_a6ea_23eb, 0x2be3_1dc6_ee80_191d),
            (15000, 0x9003_a7f0_e030_fd54, 0x3afd_8421_cb42_966e),
            (16500, 0x2119_121b_86f4_4329, 0xb6d1_92c6_a2d1_9572),
            (18000, 0x4f53_c681_d78a_2bdc, 0xbade_2529_165d_cb44),
        ],
        last: 0x15eb_71ee_f604_e8f8,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xce36_e823_4ff2_e131, 0x301d_7e5c_308b_971e),
            (3000, 0x5a17_c171_3fcf_7d07, 0x98b9_75f1_ba43_01a9),
            (4500, 0x5135_0c95_5d93_300c, 0x44ed_002b_f833_3856),
            (6000, 0xe053_9e10_5eba_9f51, 0x4c24_cbbd_cb1d_ff6a),
            (7500, 0xd38c_97f9_8727_6453, 0x866c_de12_1eb0_41de),
            (9000, 0x0906_43f5_13ac_a97d, 0x964f_c1b8_aa8d_97ba),
            (10500, 0xd94e_4c1e_cf3f_f754, 0x2d10_f64e_ae49_81d8),
            (12000, 0x9b80_4933_4f96_1be1, 0xd5f3_3280_b18d_dab2),
            (13500, 0x6cc7_f938_8a94_4a3c, 0x0541_d011_12cf_ea7b),
            (15000, 0x514c_2218_7d03_45d5, 0xcd8a_7eba_3eca_ebdf),
            (16500, 0x8a4d_2502_5d55_555c, 0x4180_f62e_2925_c47b),
            (18000, 0x23f1_3e3d_838e_e626, 0x904b_41bd_594b_d763),
        ],
        last: 0x9d83_4d7a_bf0b_eba2,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3a15_269f_85e0_41fc, 0x54e6_537a_0714_adaf),
            (3000, 0x22a6_b2d5_637e_2df4, 0x2b72_d2ab_49a5_8d17),
            (4500, 0xdb93_e914_2e7f_c903, 0x6101_d824_fe95_f5c8),
            (6000, 0xc37f_c4a1_9f7b_4d9e, 0x071f_b116_c3a4_6fce),
            (7500, 0xec14_4d22_549c_e466, 0x202f_c672_70a9_7c72),
            (9000, 0x71cb_3cbd_4181_879f, 0x1184_a31d_e8af_3c2d),
            (10500, 0xbee5_7545_09f1_ea6b, 0xf38b_a119_7b0d_0a4a),
            (12000, 0x9e30_cdec_3764_86d8, 0x6d7c_3fe2_2872_3d32),
        ],
        last: 0x9513_54d2_951a_3f7f,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x62fe_48a8_1db8_ba86, 0x41ce_7b7c_bd61_2dbe),
            (3000, 0xc820_a07a_c538_fd07, 0x2657_541b_95fe_f25d),
            (4500, 0x9a56_2079_44c6_fb81, 0x3f46_f758_9fbc_b765),
            (6000, 0x97e2_9cd4_3a1d_1511, 0x3237_5d07_ba21_72be),
            (7500, 0xa968_65b2_5c07_f979, 0xa34d_5318_06df_34bf),
            (9000, 0x58a8_aeeb_58fc_2cfd, 0x176f_aae7_4463_3754),
            (10500, 0x1cce_3481_f94f_3dff, 0x5529_7bc2_7a7c_c08a),
            (12000, 0x7b73_8691_cc30_5b6f, 0xa570_135b_3a78_f20e),
        ],
        last: 0x2be8_df09_c7bd_48b7,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd49d_c244_a87d_1567, 0x2540_9096_0c73_b247),
            (3000, 0x7f4d_e3ef_8a6f_8963, 0x3414_52b7_5949_f1d3),
            (4500, 0xc525_4137_2c7f_7b3f, 0x8500_376f_538b_6028),
            (6000, 0x6051_9e65_2141_dd3e, 0xd224_a858_fab1_35b1),
            (7500, 0xf4f2_fef9_3626_e8ef, 0xc9ad_aee1_4e14_71d5),
        ],
        last: 0xcf22_61d5_0802_6de4,
    },
];
