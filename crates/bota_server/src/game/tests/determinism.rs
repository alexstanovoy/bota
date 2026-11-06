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
        (6000, 0x85a3_3580_1dca_55df),
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
        (4500, 0xf68b_c1b9_9a9c_4e40),
        (6000, 0x6cb9_b2e3_3181_6dc2),
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
        (6000, 0x288e_0cdb_a4e4_1a08),
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
            (4500, 0x321c_e495_ee15_5c0b, 0xe1c7_268a_7c90_2217),
            (6000, 0x1cb8_7df7_3b7a_39b7, 0x76bc_af54_8921_ed3a),
            (7500, 0x8ea4_fc17_fb38_15e8, 0x81a9_6afa_b3c4_2ed2),
            (9000, 0x3891_74c3_7bf0_aa79, 0x6959_8f7c_4df8_cec3),
            (10500, 0x63b7_5d1a_cd84_0083, 0x6074_1597_1e7a_48db),
            (12000, 0xdee9_9a24_f9b3_ffac, 0x7baf_a15e_df5a_0716),
            (13500, 0xcaa0_26d5_2e7d_2900, 0xb92b_de11_d7e4_c4b0),
            (15000, 0xbba8_6440_77bd_d4d9, 0x7da2_876b_8472_eb0c),
            (16500, 0xe64b_ed6a_fddb_cfbf, 0xcd38_580e_1d3f_afed),
            (18000, 0x5386_b7e3_b9d4_cfb4, 0xee55_f35b_dabb_7010),
        ],
        last: 0x161c_7df1_3e94_6381,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x572b_588d_9e3d_ba2e, 0xaa0c_b426_4e46_d8d0),
            (3000, 0x43c1_e29f_fd93_f293, 0x03d4_e940_0ae5_7a22),
            (4500, 0x649b_54ec_f93c_bf14, 0x072c_5dbc_2c88_92dc),
            (6000, 0xaab3_fc7c_8f4a_ef1e, 0x478e_816a_5d32_f827),
            (7500, 0x8e3e_67aa_6416_fbb0, 0x913e_10d5_c226_dcbc),
            (9000, 0x96a1_ec68_c8d9_6ca4, 0xeda0_bb28_ccce_d7b2),
            (10500, 0x9f6c_066c_123f_8170, 0x828a_a013_cf8b_d218),
            (12000, 0xb68e_9116_641a_db8f, 0x06b8_ad87_dda0_794b),
            (13500, 0x9092_242a_5204_f71c, 0x54fe_5f0e_6922_98bc),
            (15000, 0xc795_0c97_5472_6063, 0x7f5e_c7ab_6832_fbbd),
            (16500, 0x7553_f8dc_346e_be9c, 0xc06c_433e_ccc3_2193),
            (18000, 0x09a6_f794_1d54_1631, 0x0d63_5736_c519_8078),
        ],
        last: 0xb232_9b44_fcc8_f14a,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd5e1_6ce7_c660_36c6, 0x7e8b_b49e_572a_7069),
            (3000, 0x3aba_14d0_1523_42bb, 0xa3fc_1b32_9e52_6be0),
            (4500, 0xca93_2da0_15eb_07bd, 0x7604_9855_062b_204d),
            (6000, 0x3ee1_2277_1f7a_4420, 0x741e_ab90_81f9_a32c),
            (7500, 0x0ed5_d6d6_ceca_015f, 0xafbe_f8b5_6ee7_f14a),
            (9000, 0x5f8f_d210_d130_5012, 0x8d41_7987_bc4c_7b95),
            (10500, 0xc5e2_63ca_a4fb_ada3, 0xb95f_a662_6812_c27e),
            (12000, 0x6fdb_ac8f_74e7_5a81, 0x08fc_af19_ee6a_115e),
            (13500, 0xd268_47a6_0af4_aa08, 0x4b10_25db_7446_2936),
            (15000, 0x22f8_2be4_4a69_8ca4, 0x9de0_41c7_e486_455e),
            (16500, 0xfda8_827f_f3b4_abb6, 0x4640_2730_2f47_8ee4),
            (18000, 0x7a18_2d81_d73a_06dd, 0x8d41_a615_01a7_b93e),
        ],
        last: 0x15fa_c6da_d93d_f8be,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x1dae_2a12_aea1_b245, 0x0e6f_29d2_d97b_3b39),
            (3000, 0xb24d_b286_5214_31b1, 0x6844_dd6a_03ef_93cf),
            (4500, 0xd49d_b2bc_38f1_d90f, 0xa698_edeb_639e_7bfc),
            (6000, 0x163e_3543_2bcc_f070, 0x06bd_5d7a_bad2_3d44),
            (7500, 0x0a30_0ecd_1ac0_e81f, 0xa59c_1374_13b7_024f),
            (9000, 0xecef_3cb2_f059_e649, 0x1339_9047_7e9c_f141),
            (10500, 0x20f7_ce17_1a62_d9f4, 0xe4f2_3b69_0cf7_2c8f),
            (12000, 0x133d_5b68_4310_856b, 0x0791_1332_6611_8abe),
            (13500, 0x002d_3d0f_d95c_994b, 0xde56_57be_508d_aa1e),
            (15000, 0x355f_0049_a23b_07e5, 0xe129_63df_6663_2027),
            (16500, 0x1d92_539e_2bf3_7cc2, 0xca3e_355e_0af9_e0a3),
            (18000, 0x60fd_d4b5_b5f2_ee9b, 0x5ba9_3e63_0e81_dd4b),
        ],
        last: 0xb198_5c0c_cab9_84f0,
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
            (7500, 0xb450_5f1e_dbff_4d98, 0x327b_54bb_579a_22ed),
            (9000, 0x38a8_15ac_450e_1f36, 0x7f8c_8a2e_eabd_8d22),
        ],
        last: 0xed86_6ad3_3c2e_14ce,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd7eb_c546_5cde_94a0, 0x4e0f_d29d_1161_1e97),
            (3000, 0x5c75_6d5a_9d20_b40e, 0xbfbd_cf78_76f8_f3b8),
            (4500, 0x5cb0_7909_0ba0_77e9, 0x93f3_6eb2_5031_d0cf),
            (6000, 0xb881_d672_1b92_c06b, 0x7fab_4654_988c_5a2f),
            (7500, 0xe142_7959_3455_4ef3, 0x9d96_7d72_a87e_3f5f),
            (9000, 0x6f29_145d_0aaf_15f8, 0xf351_90b6_047b_d70b),
            (10500, 0x6cdc_a929_00b2_4d10, 0xec80_956f_8601_754e),
            (12000, 0x2b09_d6be_a5fd_ac14, 0x0db2_dcb2_fb38_8657),
            (13500, 0x97f8_b662_a692_86bf, 0xa316_ca94_2e04_11ed),
        ],
        last: 0xf598_942f_cb8f_f4dc,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5e88_543d_9f1c_83a0, 0x7698_5fa4_6b25_a1b2),
            (3000, 0xc772_fe25_4baa_8b0b, 0x3ccc_1ccd_4e30_2485),
            (4500, 0xf9bd_de57_805e_7dc2, 0x6288_3d42_ce88_5f8b),
            (6000, 0x892b_9048_0709_bfc1, 0xf2c4_2b22_1e9b_11a8),
            (7500, 0xae91_41db_c278_6b74, 0x3cec_451b_e280_32f7),
            (9000, 0x106d_66b7_3979_cd33, 0xb293_55f9_b59d_5231),
            (10500, 0xa8a4_f151_46b2_501c, 0x4400_9a80_0b9a_fe7d),
        ],
        last: 0xc11c_dd15_0f78_3f87,
    },
];
