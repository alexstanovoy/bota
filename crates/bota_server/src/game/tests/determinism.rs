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
    while world.tick < FORK_TICKS && world.victor().is_none() {
        if world.tick == FORK_TICKS / 2 {
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
        (1500, 0xcf7c_860e_9947_ff20),
        (3000, 0x42b9_3756_95ec_e150),
        (4500, 0x3f98_634e_7fe9_beca),
        (6000, 0x39bd_adf6_6b9c_7264),
    ],
    &[
        (1500, 0x5b15_a4d7_ff15_7e5d),
        (3000, 0x522e_ae29_001f_7524),
        (4500, 0xf743_e227_7067_c645),
        (6000, 0x331a_37af_2a15_8ea4),
    ],
    &[
        (1500, 0x66ae_40cc_10f9_650c),
        (3000, 0x59c3_00db_6bad_c21c),
        (4500, 0xdbb6_d23a_29f1_af52),
        (6000, 0x30e1_42b4_4f4d_c8b1),
    ],
    &[
        (1500, 0x2b20_37d2_7640_68a8),
        (3000, 0x93bf_ca65_6cc4_c8e9),
        (4500, 0x7359_d2ba_f651_8500),
        (6000, 0x5e0e_9f62_6ca0_d9de),
    ],
    &[
        (1500, 0x8eb4_2314_f5b9_f08b),
        (3000, 0xfd85_9bb2_22ed_4400),
        (4500, 0x87c5_661b_e122_5a9c),
        (6000, 0x1f4a_6b3e_a8c5_6e3b),
    ],
    &[
        (1500, 0x5543_9ecf_e0ab_c46b),
        (3000, 0xd285_f2ea_7514_709e),
        (4500, 0xd6c8_ebc5_44e6_ff71),
        (6000, 0x9758_b336_9bcd_d450),
    ],
    &[
        (1500, 0x1b79_0cd3_f28e_3f8d),
        (3000, 0x2aa8_57df_a99a_5374),
        (4500, 0xd68a_7e34_e04e_cfcb),
        (6000, 0xab23_e098_d4fe_e195),
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
            (1500, 0xf2a5_1b8b_3b9a_9bc0, 0xa39f_583a_3f1d_f592),
            (3000, 0xbd71_cc5f_66a5_0364, 0x193d_5187_9a32_b137),
            (4500, 0x45ef_fc92_a444_3553, 0x1cf0_b0e6_c3f1_86eb),
            (6000, 0xd233_177d_cd2a_45e4, 0xeeae_13d1_9a23_54f7),
            (7500, 0x0b9c_71a4_706b_2718, 0x15a0_b641_7bd7_545d),
            (9000, 0x39fd_845a_1215_e32a, 0x72c7_d57f_c178_cd70),
            (10500, 0xaf1a_064d_4dbe_b3db, 0xc136_d374_e97c_c789),
            (12000, 0x6b70_c8a4_a6c7_fb7d, 0xe5a1_9122_8c80_a290),
            (13500, 0x53ad_4981_a88a_2bbe, 0x54ad_d1cc_8b46_231c),
            (15000, 0xb091_1f53_abc6_dfe3, 0x0943_1104_c2c2_7392),
            (16500, 0xb487_6440_888a_e965, 0xf610_8298_8d4b_9985),
            (18000, 0x82d7_2451_81f4_093b, 0x9d83_0653_6707_cf39),
        ],
        last: 0x4c34_03cf_113f_b09c,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7e06_15ea_ec9c_8363, 0xa045_7c0f_dffe_ee05),
            (3000, 0xceac_4520_0c2d_ee4b, 0xf603_bbf8_796c_14b2),
            (4500, 0xfeb6_5a5b_1845_4003, 0xc8e5_9cba_dd4d_2449),
            (6000, 0x0934_8dab_8b5f_c627, 0x98f9_203c_5566_3fe4),
            (7500, 0xd95e_a4ea_028a_4551, 0x1b37_1438_5c7c_1bd8),
            (9000, 0x863e_8bc5_4686_e2f7, 0x222b_5058_24f3_4395),
            (10500, 0x4de0_2b92_dd57_51f2, 0x388b_15c1_b314_a871),
            (12000, 0x8a04_2787_373e_4181, 0x0624_736c_f20a_87e5),
            (13500, 0x5ee5_9a3f_7081_5e5c, 0x7315_7407_2103_effc),
            (15000, 0x5421_5ad8_8895_d7d6, 0xfadc_ccfa_d159_8cec),
            (16500, 0x9e90_0927_3630_9f87, 0x57d7_318a_a846_bb89),
            (18000, 0x618e_0a5e_fed4_2ba9, 0x0316_63aa_a3be_3b19),
        ],
        last: 0x280a_805e_5af6_d962,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x1feb_6d10_e7bb_f5d6, 0x9fdc_c0de_de83_62b7),
            (3000, 0xfb4d_411b_9e01_ad9d, 0xade8_f6eb_2b43_382b),
            (4500, 0x008c_6921_3ea8_7ff8, 0xb696_6d0e_008c_9e07),
            (6000, 0x30ba_b6aa_4731_62ec, 0x7def_a0f7_735e_18c9),
            (7500, 0xccb1_6f2b_07f2_27ca, 0x34ba_2def_b485_eb04),
            (9000, 0x0c58_e856_4ce8_2453, 0x6b4e_56dc_89a6_6302),
            (10500, 0xf3e4_c840_bc4d_52b2, 0xbace_4523_51a7_5863),
            (12000, 0x217e_f18c_095c_3faa, 0xcad3_1259_e806_9cc5),
            (13500, 0x806a_c93b_df5f_2bb5, 0xe3e0_1761_1975_6174),
            (15000, 0x1b16_522c_d239_36c6, 0x0e44_b0e0_b6c8_1334),
            (16500, 0x754a_4063_a137_89c8, 0xd116_418b_021a_9eee),
            (18000, 0x4162_ce74_9e98_c590, 0x46f8_264a_3c04_96d0),
        ],
        last: 0x0b62_861f_3f02_f89a,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xb1ab_a5e4_7f96_9f7d, 0xc8bd_ae8b_060f_6289),
            (3000, 0x7f20_3ea7_da1b_6c69, 0x0a57_5a13_15bf_6ffb),
            (4500, 0x5726_9114_1358_6200, 0xd890_2263_92cf_0802),
            (6000, 0xb189_3226_3265_aea6, 0x5fcf_d289_a0ad_d94d),
            (7500, 0x11f1_7c19_108e_f96d, 0x6ec0_7b86_a468_fac8),
            (9000, 0x5cc7_113d_8641_d950, 0xa93f_fb2b_d4a8_d986),
            (10500, 0xadca_c390_eb89_105c, 0xbedc_8472_d856_49ce),
            (12000, 0xbaec_6714_24a8_5e7d, 0xc9c1_a00f_d905_abd1),
            (13500, 0xc427_652d_9d73_df09, 0x130d_8eb6_0f1d_30ec),
            (15000, 0x1570_b24c_8c80_40fa, 0xd8b2_7d79_cf75_01bc),
            (16500, 0x15be_25d9_3341_dd9c, 0x9812_1385_ea7d_185b),
            (18000, 0x1a8b_84e8_6141_730c, 0xb3f1_5e6d_2470_246f),
        ],
        last: 0x32eb_73ed_07ca_5737,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5111_00e8_c150_1252, 0xb396_0cb2_d7f0_e4b9),
            (3000, 0xd00e_f5a1_f0b7_4e5e, 0x1cfb_587c_52e3_ce18),
            (4500, 0xec11_3742_8101_facf, 0xd27c_2c7b_a465_60c0),
            (6000, 0xa3be_16c8_eab1_fa0d, 0x6f7f_fd61_36f7_a89a),
            (7500, 0xbf80_ff4e_94cb_9be2, 0xd608_ab38_59c4_d5c8),
            (9000, 0x06cb_7ef7_fdd0_b3e2, 0xd4ac_b5a9_9a68_7aea),
            (10500, 0x799a_8372_3060_14dd, 0x48b2_069d_fc53_1ccc),
        ],
        last: 0x0bef_48a5_9634_d164,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5687_59f2_6ab6_d318, 0x9d23_92b0_d18f_ce83),
            (3000, 0xe686_c14a_47a5_2ee0, 0x2b22_cdf2_cb13_5d9c),
            (4500, 0x47b7_70f1_6300_7c25, 0x45c4_a38b_1e02_58f9),
            (6000, 0xc495_3937_bac1_04b5, 0x63a8_0d3b_5eac_353a),
            (7500, 0x381d_81dd_f73f_7280, 0xcd03_e639_f98c_d4d8),
        ],
        last: 0x86de_1832_07df_f053,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x057a_bfa4_c906_fcac, 0x5395_ea84_d21e_40a0),
            (3000, 0xcdea_28dd_7946_bb71, 0x6e1b_c34a_7c8d_0f4b),
            (4500, 0x6fbe_03f3_18af_b9cc, 0xf65f_952b_2ebe_642e),
            (6000, 0x75e6_e290_dcba_e141, 0xf59d_113f_c29c_e3a0),
            (7500, 0xcff6_1bb9_db2f_1eaa, 0x1f4d_0365_e74d_b47d),
            (9000, 0x10f9_d4b1_b684_9f6f, 0x5c03_c1d1_49f2_f33a),
        ],
        last: 0x5674_b88a_4bd4_8156,
    },
];
