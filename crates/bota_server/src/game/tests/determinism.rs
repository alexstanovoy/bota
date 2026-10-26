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
        (1500, 0x854f_b6c2_6b73_3202),
        (3000, 0x32ef_4e0f_8018_7977),
        (4500, 0xb6c5_20fc_514b_2a41),
        (6000, 0x114c_13b4_ee42_6a98),
    ],
    &[
        (1500, 0x340f_635f_ccf5_4f6d),
        (3000, 0xe0fc_f308_c916_3598),
        (4500, 0x334b_70e9_f6e3_6835),
        (6000, 0x8568_9be0_8219_5283),
    ],
    &[
        (1500, 0xff9d_7eca_546d_8ce4),
        (3000, 0x0382_083d_0177_0e72),
        (4500, 0x4b73_5395_1259_53fa),
        (6000, 0x9c12_79b5_97af_1a8d),
    ],
    &[
        (1500, 0x3de3_a90c_ddfb_9807),
        (3000, 0xa04e_80a2_3b26_3999),
        (4500, 0x0d30_f291_0873_d5d9),
        (6000, 0x5217_73b0_47c3_7cc3),
    ],
    &[
        (1500, 0x62db_127b_a6ba_9326),
        (3000, 0x5364_ff3c_7566_1644),
        (4500, 0x7d4a_e777_2b4d_d94f),
        (6000, 0xcb1b_6d2b_9639_0f6a),
    ],
    &[
        (1500, 0xf16e_d704_8229_0fff),
        (3000, 0x9b59_7951_3c73_5c04),
        (4500, 0x9f22_4c92_e6bf_a7e6),
        (6000, 0xc5a8_17af_6d23_5f7b),
    ],
    &[
        (1500, 0x8c05_9143_f25f_9909),
        (3000, 0xfb7c_b13f_11d7_7213),
        (4500, 0x0959_a80a_5686_133c),
        (6000, 0x030e_8755_65ad_5b9a),
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
            (1500, 0x4884_136e_f711_1afe, 0x903e_1ec2_6f8a_80a5),
            (3000, 0xbbe8_6cfe_56e6_843a, 0xae61_b214_abac_e879),
            (4500, 0xb266_9be0_3b79_02cb, 0x6e85_aa53_b5b5_018c),
            (6000, 0x67ef_88b8_4c86_cfb3, 0xea9d_8465_474a_4265),
            (7500, 0x2e30_31a7_e26a_fe4c, 0x8d97_1ef2_a3f1_c0b9),
            (9000, 0x3a96_e0fa_34ff_ba91, 0xfc97_755e_aa77_e450),
            (10500, 0x4a19_30ae_a82e_e3ae, 0xafd0_ec9b_bd3f_e961),
            (12000, 0x7bd6_ab9d_03b2_62b2, 0x06dc_10ac_f82e_4a88),
            (13500, 0x54d8_cd35_4fa7_f819, 0xae5a_d516_1df6_fce5),
            (15000, 0x938f_c511_1da8_f1aa, 0x049c_9468_e114_565a),
            (16500, 0x6005_21d2_9c2f_5e52, 0xd71f_4387_9587_1d9b),
            (18000, 0x09be_c561_a65c_7881, 0xdb7a_2f71_fcaa_3afc),
        ],
        last: 0xf8ed_9257_d0fb_27aa,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x57d9_068c_aa5e_1358, 0x3ebd_2238_2639_3214),
            (3000, 0x4fc8_3d1b_8e59_8a1b, 0xae39_64c5_b644_acb6),
            (4500, 0x9c69_5c93_4c9e_42d1, 0x51bf_2171_de80_b4de),
            (6000, 0x1341_2979_e2b5_b51c, 0x0fdd_160f_4b12_5d14),
            (7500, 0xa8e1_99a7_de51_f378, 0xa302_dddc_98eb_f7f5),
            (9000, 0xbda7_3991_5451_f216, 0xc781_827f_0686_0c7c),
            (10500, 0xeeb0_bb4f_275c_3dab, 0x63de_e1d7_ddfc_f0dc),
            (12000, 0x69bc_c836_180f_8219, 0xebb8_d6b7_132e_1ca0),
            (13500, 0xeec8_2846_92bc_a2b9, 0x52e8_5fa9_3079_89f1),
            (15000, 0xaa9b_a2cf_e2d0_572a, 0xd59d_87cb_068a_f029),
            (16500, 0x84b3_17d9_0b23_f7d5, 0xd9eb_0c52_f889_5456),
            (18000, 0x740f_07c3_4ac2_fb23, 0x13ca_3c42_fb9e_c62d),
        ],
        last: 0xc8fc_876f_bfb1_380a,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xe838_f82c_9edd_9fef, 0x3713_7672_2a59_f147),
            (3000, 0xcb72_17a0_14ba_08f8, 0x41fd_099c_9a51_e7ca),
            (4500, 0x89bf_6af8_5339_b8d8, 0x8028_55ec_196b_bde8),
            (6000, 0x31de_47fc_49dc_d237, 0x1fd2_1130_c6ef_c3fd),
            (7500, 0xe7de_ef2e_ff84_27a9, 0x5a14_a2b8_94c0_120b),
            (9000, 0x6dcd_be25_4776_34ee, 0x9c78_f393_07db_58b3),
            (10500, 0xbeff_0781_a53d_4241, 0x4e51_8b43_0ee8_638c),
            (12000, 0xb03c_7ef9_b83d_0024, 0x15ae_0350_80ea_eb6d),
            (13500, 0xae1a_931d_7de0_53d7, 0x596c_bf93_5128_45f2),
            (15000, 0x1bda_4da3_cbd6_b686, 0x9be7_ebf1_edfb_669c),
            (16500, 0x16a7_bbaa_1fd4_af52, 0x3ff0_e2dd_692c_cba0),
            (18000, 0xb267_2667_22f0_e026, 0x155f_d9a7_639f_0c82),
        ],
        last: 0x8bfd_43be_3321_d098,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x6b0e_5a74_9178_5cdf, 0x506f_161b_a76e_0a3a),
            (3000, 0xaf7f_3a93_6201_a8e5, 0xeb08_0c30_f4c6_78c0),
            (4500, 0xdca9_9d4d_16e5_8fb4, 0x9585_dd24_4250_9252),
            (6000, 0x9ffb_a390_bbe2_aba2, 0xb5ed_43d7_c3ba_4c06),
            (7500, 0xe9fe_f198_479f_5cc0, 0x7e49_fa4a_270e_bf9e),
            (9000, 0xdfd9_0737_8a5f_2ca3, 0xa063_471b_ce15_ecee),
            (10500, 0x7d1e_8772_bb7f_787c, 0x362f_59aa_bcef_1f41),
            (12000, 0xb848_e492_5603_84f8, 0x18d5_7b90_48c7_a52b),
            (13500, 0xfb09_4034_1d9e_a466, 0x136b_2faa_0aae_533f),
            (15000, 0x4fa2_f7c5_40bb_aff9, 0x884d_aac3_26f8_6085),
            (16500, 0xf85a_defc_957c_c4e6, 0x6304_d1da_f9ff_b06d),
            (18000, 0x22ee_aa36_4eb1_352c, 0x0ee7_252d_b850_dc00),
        ],
        last: 0xe7da_6b91_637f_3c72,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4289_b356_6ce8_a759, 0x05a4_7e8c_f5be_7811),
            (3000, 0xbcbb_2922_4000_bbb7, 0xbb99_9eec_a9f7_c725),
            (4500, 0x1f58_808e_a650_41a5, 0xe3b6_f50d_4781_4a50),
            (6000, 0x9df5_9faa_775f_c52c, 0x74a0_7e21_b532_8abb),
            (7500, 0x3dc2_08fe_49e2_1fc9, 0x5583_a555_56cc_07ed),
        ],
        last: 0x8493_30a4_5965_70c7,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x98fe_99db_779b_dbf9, 0xc920_d981_be7d_58c8),
            (3000, 0xa8ab_0252_8794_eb17, 0x9378_144d_3671_5d76),
            (4500, 0x3754_76a1_2c06_c7b7, 0x4a8d_3bc6_e5f8_1991),
            (6000, 0x3d4f_6fd7_7fec_345a, 0xe82e_0976_a5b2_f4d7),
            (7500, 0xe08c_b612_de7a_b611, 0xddb9_2716_fecb_45f8),
        ],
        last: 0x0bf9_372d_873f_93d2,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3a26_a426_ae8d_0bd4, 0x23b3_7312_5ac7_0063),
            (3000, 0x4b8f_a72e_8f41_7a31, 0x65c5_4d3c_21af_7c4e),
            (4500, 0x27ab_9d49_1e7c_bda7, 0x56dc_8b0d_f884_621e),
            (6000, 0x8057_330e_3190_192d, 0xa81b_4ee2_c3cd_0875),
            (7500, 0xea3e_9808_d2b7_11f6, 0x34bd_2c80_fd6b_083c),
            (9000, 0x0411_15a2_1910_a84f, 0xf00b_4659_54aa_765c),
            (10500, 0x28e4_e06d_e49e_9f11, 0x49e1_d9c6_47c1_362c),
        ],
        last: 0xfc5c_acf6_747d_552e,
    },
];
