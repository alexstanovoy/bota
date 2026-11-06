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
        (1500, 0xcc79_d221_a860_41d6),
        (3000, 0x86fb_d82d_197a_7582),
        (4500, 0xb947_663d_b34c_0c09),
        (6000, 0x654b_314b_1086_ea08),
    ],
    &[
        (1500, 0x7b7b_cea6_a68b_1811),
        (3000, 0xcc51_be6e_76cd_2107),
        (4500, 0xe675_111a_d6a4_d874),
        (6000, 0x3f23_090b_d37f_12a7),
    ],
    &[
        (1500, 0x87ff_8cb6_5f3c_a16b),
        (3000, 0xd7cc_d857_7f8c_4a0e),
        (4500, 0x002d_6930_5790_71ef),
        (6000, 0x8890_7d36_26e2_5a8f),
    ],
    &[
        (1500, 0xe518_19e8_aad9_56e1),
        (3000, 0x9e3e_e508_0266_7624),
        (4500, 0x1584_4ed0_c01d_3fdf),
        (6000, 0x58de_df22_340b_23c5),
    ],
    &[
        (1500, 0x2a35_30d4_2cac_4169),
        (3000, 0xd47a_f71c_720e_55a4),
        (4500, 0xfe09_5f7b_e83f_1a3e),
        (6000, 0x2d54_1043_cacf_99f4),
    ],
    &[
        (1500, 0x35b7_aba7_739a_276a),
        (3000, 0x731f_9ee8_e4dc_9df7),
        (4500, 0x9a2a_7ab9_8355_9067),
        (6000, 0x052b_3367_1a4d_5c6e),
    ],
    &[
        (1500, 0xb5fd_6231_4a33_9136),
        (3000, 0xe11c_5ca5_26ff_6ff5),
        (4500, 0xb2ea_ea77_3909_ce24),
        (6000, 0x8458_31da_35f0_6e11),
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
            (1500, 0xda70_2ebc_01e6_8ea1, 0x62a8_efab_62a0_17a7),
            (3000, 0xb99f_4888_57dd_a337, 0x26fa_7893_28aa_1090),
            (4500, 0x5e2c_a2bc_38fd_588a, 0xfa32_1ab5_60b8_8154),
            (6000, 0x019c_61ca_e358_56ab, 0x70e5_ee57_312b_7cee),
            (7500, 0x6d28_dae1_d33e_9e21, 0xbbcf_3ff7_3205_3298),
            (9000, 0x65a5_686a_949c_a50e, 0x90b1_bdc6_df1d_1a5d),
            (10500, 0xd488_9023_4b45_665a, 0xac60_92dc_be8f_c4cc),
            (12000, 0xb63c_b90a_fd1c_e224, 0x13bc_a547_9da2_5206),
            (13500, 0x47a5_9c72_43eb_f00d, 0x77e3_0a56_e9f8_4170),
            (15000, 0x0726_648d_2afb_d641, 0x0525_933e_032a_196c),
            (16500, 0xcde6_82b7_90ff_2de9, 0xf2cb_fc1e_64ad_bd5e),
            (18000, 0x42b3_b9c4_809f_45bf, 0x52fa_1067_b4bd_57e3),
        ],
        last: 0x947c_ebeb_f1f6_8e3f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xdf46_86c8_fe2f_9704, 0xbfd8_9a34_0fc7_5317),
            (3000, 0x9b29_1ee2_30f4_e6d7, 0xe235_d9f9_42f5_406d),
            (4500, 0x9efd_73f3_0c5a_08ea, 0x9391_8bbe_0a2f_30f8),
            (6000, 0x0393_1d39_cc7d_1daf, 0xf348_4d5f_9c32_1f6c),
            (7500, 0x1fb9_b017_18ad_86d2, 0xf576_1e9c_fcec_3367),
            (9000, 0x9f61_4a8b_1649_cf8f, 0xa54e_27c0_9db3_84fd),
            (10500, 0x34dd_3d30_13dc_4ed1, 0x469e_f1b6_e772_a1d4),
            (12000, 0x052a_de7f_1fc4_376a, 0xa65b_a48b_6881_f57b),
            (13500, 0x67a0_56a5_93c3_5692, 0xd347_437b_ee19_29c9),
            (15000, 0xad1c_a7a1_8036_8148, 0xac7c_62f2_de15_07f6),
            (16500, 0x95ba_db96_3f51_383e, 0x0afc_4a4d_aac8_4ba8),
            (18000, 0xa844_f9a1_2259_6903, 0x0d6f_c588_76e2_09eb),
        ],
        last: 0x3441_5580_cd09_f2d2,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xab0d_18a9_c2d5_608e, 0x4b15_a022_419a_4a84),
            (3000, 0xb016_a6ae_5ed5_23da, 0x929d_9d48_5f44_e13c),
            (4500, 0x5e59_f4e8_bed5_a479, 0x2f26_aff5_aed0_5e8b),
            (6000, 0x8558_95a0_9869_ba85, 0xb478_8076_f71b_0c2c),
            (7500, 0xfdc8_4865_f2ed_97a1, 0x9e6e_dd4c_5b96_cdbb),
            (9000, 0xd5ac_b2f4_28d9_a536, 0x5723_94f7_34f1_d840),
            (10500, 0xddbf_8efc_3530_b922, 0xe6f4_4f5e_5f13_69fe),
            (12000, 0x7d5e_bfe8_bfeb_4645, 0x41f1_a08e_e537_18b3),
            (13500, 0x7ba4_647a_c5fd_ceb3, 0x782e_d82b_fcde_7755),
            (15000, 0xbebc_b715_e427_532f, 0xf651_670e_ca16_c275),
            (16500, 0x871c_930b_d73c_13ae, 0x1e52_a2a0_87b1_69ec),
            (18000, 0x40d4_8c54_a79f_259a, 0xfafb_7b87_2543_ac48),
        ],
        last: 0x54cf_37af_eb68_0ec3,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2d8c_88d7_dde6_4af6, 0xdab3_492d_0788_2b61),
            (3000, 0x3fd8_e914_5d53_481e, 0xa121_fa2f_8ac0_0b8a),
            (4500, 0xf514_59ac_1e5f_97bf, 0xd3b0_fe7e_6976_39e2),
            (6000, 0xf9f7_f39f_f0ca_7f4d, 0x610a_0ff8_a7e2_c4be),
            (7500, 0x929b_4c32_e1f5_2a72, 0x32fb_a154_5399_b50d),
            (9000, 0xd74d_3e09_5534_f2f3, 0x6479_f47a_a6ee_d92d),
            (10500, 0x9563_4784_93eb_aba4, 0xc921_caa6_a3d2_9acc),
            (12000, 0x502c_2a7d_15e4_9a93, 0x1c46_7f0d_a2ca_31d7),
            (13500, 0x6268_3aca_8e92_e1b2, 0x97b7_9bfe_b443_dd51),
            (15000, 0x62e3_5cee_64d0_bca6, 0xeeec_34ab_0cc0_6dd9),
            (16500, 0x4a8e_039f_6756_197a, 0xa4c2_67bc_8fd9_bc92),
            (18000, 0xa158_665b_2f64_a769, 0x3b5e_ad4a_0f47_475a),
        ],
        last: 0x5258_ca7a_222a_1405,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x97b1_5dfb_8cba_e449, 0x60d7_f03b_9435_bc81),
            (3000, 0x1561_81b2_85af_732c, 0x2e64_9415_f152_70a3),
            (4500, 0xa816_c4fa_ed31_807d, 0xb004_1e7f_26c1_cff6),
            (6000, 0x52d2_cb9c_5fac_ed23, 0x1b7e_6ba4_942c_558e),
            (7500, 0x586e_e10d_0f42_2e40, 0x803d_abba_521c_4e93),
            (9000, 0xead9_6e5d_5506_2956, 0x37c3_db1d_a875_97c3),
        ],
        last: 0xd457_3d36_0a52_7ee3,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x24b5_708d_8408_fa6f, 0xa01a_4899_0aeb_903f),
            (3000, 0x54f6_f37c_133c_8c11, 0xbe7b_6ce3_0791_d682),
            (4500, 0xf7c3_bce7_f9ef_8d5c, 0x4560_9734_19d0_a09c),
            (6000, 0xebf3_8bbb_c1ee_fd52, 0xd5e7_5452_5925_0162),
            (7500, 0xb69d_b220_7514_9cd9, 0x82a1_3947_6cf6_9f02),
            (9000, 0xdb51_dc64_1461_f8fe, 0x70f5_caa2_337f_6c7e),
            (10500, 0x38b9_a176_bfc9_9ab3, 0x1460_3ee4_d782_9df7),
            (12000, 0x0ed7_a710_6225_09fb, 0xba12_939c_a160_476c),
        ],
        last: 0x4d05_10ae_1d79_c3c2,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6785_5dec_e89a_d6db, 0x1c27_9f1f_3929_0151),
            (3000, 0x5d3e_910b_d848_fa9d, 0xbc74_06ce_19ed_4cea),
            (4500, 0xd5be_65a4_4a7d_9bc5, 0x9b40_71b9_21b7_c21a),
            (6000, 0xbd96_4b3f_1074_1f85, 0xf312_6bef_ca71_9f81),
            (7500, 0x5dd1_74f1_4454_dae2, 0xc0d2_2d99_5c45_8774),
            (9000, 0x9fd5_e114_c21e_8b50, 0x02e4_f742_161f_0cc2),
            (10500, 0xaff7_22e2_5671_75e9, 0x3073_35c9_3799_4852),
        ],
        last: 0x6bd2_74c2_41a7_a245,
    },
];
