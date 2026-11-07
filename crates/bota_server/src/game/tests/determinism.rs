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
        (1500, 0x695c_5b38_a2a8_c139),
        (3000, 0xf9f6_76da_fb3b_d7f5),
        (4500, 0xa0df_7c45_4b4a_63e4),
        (6000, 0x4754_f2eb_0c07_ee48),
    ],
    &[
        (1500, 0xc456_ee47_123a_3ccb),
        (3000, 0xf2f8_7976_1de9_079b),
        (4500, 0xe3b3_2b98_347e_4104),
        (6000, 0x7ec9_a349_cc11_a3c5),
    ],
    &[
        (1500, 0x9d08_a551_0fe5_7350),
        (3000, 0x7e8a_7ade_3f40_720d),
        (4500, 0xa8d5_bd25_a8ab_ad88),
        (6000, 0xc3e7_34b9_3112_3453),
    ],
    &[
        (1500, 0x490e_033b_ac8b_d6f5),
        (3000, 0xc490_77e0_02e2_a5a6),
        (4500, 0xfa83_e01a_9cd6_c67e),
        (6000, 0x622b_10a0_3b7d_da8b),
    ],
    &[
        (1500, 0x591c_eafc_7ecf_ed33),
        (3000, 0x2933_fa65_10b2_1999),
        (4500, 0x27d3_3bd7_79af_7e5c),
        (6000, 0x0f37_cdda_ed4b_6a7e),
    ],
    &[
        (1500, 0x656d_794a_54ab_9c7e),
        (3000, 0x3e16_63c6_ba1f_d1f2),
        (4500, 0xdb89_da16_2b25_62e7),
        (6000, 0x5049_e503_ba86_4191),
    ],
    &[
        (1500, 0x6f96_429b_e2ec_593b),
        (3000, 0x0aa8_65f7_dc92_46e3),
        (4500, 0x1ec1_4ed2_59a0_3927),
        (6000, 0x76a5_cbfd_55fe_13b5),
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
            (1500, 0xb9fb_444c_35c2_cade, 0x1b28_33b8_06ce_fbc5),
            (3000, 0x5e9c_a39a_2265_6a17, 0x105e_1b2e_1688_90d0),
            (4500, 0x321c_e495_ee15_5c0b, 0xbf79_7a9c_c578_39a7),
            (6000, 0x1cb8_7df7_3b7a_39b7, 0xdab4_785c_bb38_102a),
            (7500, 0x8ea4_fc17_fb38_15e8, 0xeb11_2317_b169_89b6),
            (9000, 0x3891_74c3_7bf0_aa79, 0x507f_12a1_9704_cf39),
            (10500, 0x63b7_5d1a_cd84_0083, 0x7097_d62c_d3a2_f597),
            (12000, 0xdee9_9a24_f9b3_ffac, 0x98c5_4b7e_0463_e722),
            (13500, 0xcaa0_26d5_2e7d_2900, 0x82aa_046f_92ab_4f5c),
            (15000, 0xbba8_6440_77bd_d4d9, 0xc9f6_7270_4789_4556),
            (16500, 0xe64b_ed6a_fddb_cfbf, 0xff8a_5328_a2d7_a5d1),
            (18000, 0x9428_a4b9_0ddd_b0d6, 0x5854_8d2a_c787_32d3),
        ],
        last: 0x06bd_79c1_10d3_11fd,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x572b_588d_9e3d_ba2e, 0xaa0c_b426_4e46_d8d0),
            (3000, 0x43c1_e29f_fd93_f293, 0xd22a_2e9a_072e_b66f),
            (4500, 0x649b_54ec_f93c_bf14, 0xa2bc_ebb9_51f2_7cf7),
            (6000, 0xaab3_fc7c_8f4a_ef1e, 0xbe1a_a22e_60ca_cbac),
            (7500, 0x8e3e_67aa_6416_fbb0, 0xda61_5be5_1350_c4ae),
            (9000, 0x96a1_ec68_c8d9_6ca4, 0xa1c4_383d_141a_4b3d),
            (10500, 0x9f6c_066c_123f_8170, 0xbbc4_94ca_3d76_0fa5),
            (12000, 0xb68e_9116_641a_db8f, 0x9163_4b9e_cd19_f656),
            (13500, 0x46ce_802d_524b_cb53, 0xa321_8da0_924b_c8c9),
            (15000, 0x566f_e685_40a6_e32a, 0x3e90_5746_d79a_dc32),
            (16500, 0x32eb_3256_c435_d535, 0xe58b_3c5f_6f46_128c),
            (18000, 0x7745_3866_303f_502b, 0xae70_5f7f_2ab5_d59a),
        ],
        last: 0xffdd_15ac_a00f_4733,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd5e1_6ce7_c660_36c6, 0x7e8b_b49e_572a_7069),
            (3000, 0x3aba_14d0_1523_42bb, 0xd5de_62d3_50d8_eb8a),
            (4500, 0xca93_2da0_15eb_07bd, 0x26e0_1915_a986_a773),
            (6000, 0x3ee1_2277_1f7a_4420, 0x3679_91ff_598e_8a6e),
            (7500, 0x0ed5_d6d6_ceca_015f, 0x9ccf_0990_d02a_37ce),
            (9000, 0x5f8f_d210_d130_5012, 0x3623_2843_f590_651d),
            (10500, 0xc5e2_63ca_a4fb_ada3, 0x8f0e_b235_a4d5_daee),
            (12000, 0x38e7_64c3_7ec6_35cc, 0xbfab_7f7a_3cf3_4064),
            (13500, 0x1a5e_cd88_7ec7_dea5, 0x481d_6521_91fa_4d70),
            (15000, 0xdd93_243f_a94d_f002, 0xfec4_31d9_2299_532c),
            (16500, 0xab45_03ae_f8cb_b6f1, 0x06fd_7523_21ab_0abb),
            (18000, 0xdf9e_e0ce_d9f6_d773, 0x3984_77b6_1d06_b00b),
        ],
        last: 0xc351_93fa_d9cc_79cb,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x1dae_2a12_aea1_b245, 0x8c89_51ff_4d29_f287),
            (3000, 0xb24d_b286_5214_31b1, 0x5186_cd5a_dc63_e8a1),
            (4500, 0xd49d_b2bc_38f1_d90f, 0xf76a_a940_37ca_49c0),
            (6000, 0x163e_3543_2bcc_f070, 0xc081_d1ec_72d3_5bc9),
            (7500, 0x0a30_0ecd_1ac0_e81f, 0x79db_e480_6311_5462),
            (9000, 0xecef_3cb2_f059_e649, 0x0bf3_b6fc_0eb9_5932),
            (10500, 0x20f7_ce17_1a62_d9f4, 0xb406_eaae_bb06_7f75),
            (12000, 0x133d_5b68_4310_856b, 0xfa22_74ba_02ce_bdd8),
            (13500, 0x002d_3d0f_d95c_994b, 0xe219_abc5_77fa_f2a0),
            (15000, 0x355f_0049_a23b_07e5, 0x1a46_8464_96f2_c623),
            (16500, 0x1d92_539e_2bf3_7cc2, 0x4260_ece5_cb8e_0b8d),
            (18000, 0xb3e7_9dde_f0c9_8e0f, 0xaec4_cd36_61e7_1dff),
        ],
        last: 0xd494_c9ca_4c04_4756,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc6e7_7970_90af_bd98, 0x4281_1521_ee66_0d4c),
            (3000, 0xa437_2615_7908_4407, 0x5626_c17d_7c25_25a6),
            (4500, 0x8e9f_3f4e_c0fd_8d85, 0x9b32_5015_9277_154b),
            (6000, 0xbe83_2e89_290a_7c9f, 0xa5e4_5bf6_5a63_73fa),
            (7500, 0x6b47_3d40_89a5_e506, 0xb4f9_2ace_ebb7_290c),
            (9000, 0x3303_7476_6e37_9a69, 0x5742_1775_46a5_a405),
        ],
        last: 0x5cf1_2978_5fa2_88f8,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd7eb_c546_5cde_94a0, 0x4e0f_d29d_1161_1e97),
            (3000, 0x5c75_6d5a_9d20_b40e, 0xbfbd_cf78_76f8_f3b8),
            (4500, 0x5cb0_7909_0ba0_77e9, 0x9f4e_2410_fd12_0ed7),
            (6000, 0xb881_d672_1b92_c06b, 0xaaa0_d882_b685_5d0f),
            (7500, 0xe142_7959_3455_4ef3, 0x0621_b072_c8b6_737f),
            (9000, 0x6f29_145d_0aaf_15f8, 0xb33c_0066_e0df_bd6b),
            (10500, 0x6cdc_a929_00b2_4d10, 0x2cfa_d584_4dd8_83ae),
            (12000, 0x2b09_d6be_a5fd_ac14, 0xde3b_1fab_8609_5bb7),
            (13500, 0x9659_d004_6588_385c, 0x04b2_7e36_25fb_809b),
        ],
        last: 0x0e29_290c_7d6e_4b60,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5e88_543d_9f1c_83a0, 0x7698_5fa4_6b25_a1b2),
            (3000, 0xc772_fe25_4baa_8b0b, 0x3ccc_1ccd_4e30_2485),
            (4500, 0xf9bd_de57_805e_7dc2, 0x6288_3d42_ce88_5f8b),
            (6000, 0x892b_9048_0709_bfc1, 0x12f9_574a_8013_0297),
            (7500, 0xa302_2071_a37e_661b, 0x9d42_53b9_7e46_2bf6),
            (9000, 0x2343_9e30_065f_cac5, 0xda9c_11b5_d1e4_bee9),
        ],
        last: 0xdc28_0cf0_e0e3_b95f,
    },
];
