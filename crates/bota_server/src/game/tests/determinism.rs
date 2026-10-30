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
        (1500, 0xd534_2b53_d2c8_f583),
        (3000, 0xbdca_624f_0090_d784),
        (4500, 0xff41_111f_55d3_3dc6),
        (6000, 0x34b6_d632_e5c9_0463),
    ],
    &[
        (1500, 0x3acd_12e8_957f_12c5),
        (3000, 0x93c5_2e17_d864_365c),
        (4500, 0x11dd_9a6b_1c5b_8d68),
        (6000, 0x2b23_2f84_f972_fb70),
    ],
    &[
        (1500, 0xb651_1f36_583b_dfbf),
        (3000, 0x3448_ecf0_eaad_7556),
        (4500, 0x98fa_7b04_4d0d_372d),
        (6000, 0x44c4_f1f1_6b71_2e17),
    ],
    &[
        (1500, 0xd111_edae_f5e9_9984),
        (3000, 0xd261_85ea_39dd_25fd),
        (4500, 0xf733_666a_2073_e47c),
        (6000, 0x9b43_88f8_194e_cd01),
    ],
    &[
        (1500, 0x4703_d15d_a071_b0d7),
        (3000, 0xfc7f_fdde_00fd_6d49),
        (4500, 0x9d58_133f_9763_2399),
        (6000, 0xb4b3_783e_b57c_94f6),
    ],
    &[
        (1500, 0xc8f0_8cb9_ba38_9e1e),
        (3000, 0xa512_ac45_6e0e_1842),
        (4500, 0xe784_1729_bcc2_b989),
        (6000, 0x4dcd_771e_3465_dfc1),
    ],
    &[
        (1500, 0xe629_f651_0d41_7398),
        (3000, 0x2507_2c37_9162_c47f),
        (4500, 0x0599_f1da_50dd_11fd),
        (6000, 0x0680_3296_3bf2_0d0e),
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
            (1500, 0x9cbb_bb1a_cc2f_edbd, 0xb854_33e8_bdae_a8ce),
            (3000, 0xb8e9_4532_c7eb_9d5b, 0x30ec_2955_b8c4_c9ee),
            (4500, 0xbcea_8bf3_e812_bd29, 0xc6cc_1f47_88ab_b67c),
            (6000, 0x57e8_d455_8cc2_076e, 0xb484_b8c8_0dbc_2525),
            (7500, 0x0e19_a48d_51d6_dce0, 0xd7f7_e0b5_ce5d_79fa),
            (9000, 0x83e5_4cd3_1d66_2986, 0x3803_0e6f_9aef_83c1),
            (10500, 0xf630_903d_4ace_7bca, 0xe6b4_0e2e_0b5e_1588),
            (12000, 0x1c77_0053_11fe_de65, 0x5f56_defa_48da_c6f3),
            (13500, 0xd88d_457a_0f7c_3800, 0x4bc0_916d_2752_ac52),
            (15000, 0x276c_9708_d3d5_5cc7, 0xbfe8_e777_948d_3894),
            (16500, 0xe4c2_4ac3_85e5_b050, 0x3366_6db1_96d3_cb7b),
            (18000, 0xf252_6992_9ef9_3176, 0xf23a_048d_4845_b51d),
        ],
        last: 0xa09a_2598_beae_e7dd,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xa10c_4108_f2ab_9019, 0xf1cb_d9e8_e093_a190),
            (3000, 0xc07d_ea98_ca66_d00b, 0xb41c_404a_5820_fe10),
            (4500, 0x9bf6_4c84_d93e_fdec, 0xb417_7823_1b1a_faa7),
            (6000, 0x15b5_42e9_684e_7862, 0xa024_5716_4bbb_a855),
            (7500, 0xbea3_4192_eda6_9c80, 0xb1fc_94bc_7c04_ecc2),
            (9000, 0xd335_ee0d_d57a_53de, 0x037c_bf98_eb98_ca34),
            (10500, 0xacfa_c45d_7b83_79e0, 0xf1ef_eb91_d59a_9dc0),
            (12000, 0xff0a_06d9_f7e8_f63a, 0x6daf_dd68_0fcc_ac7a),
            (13500, 0x82ca_6e2d_3ea8_6df7, 0x726e_d026_9c04_0f7c),
            (15000, 0x9a57_5335_329b_5ed5, 0xae8d_d5c3_8650_660e),
            (16500, 0x2043_8ba9_c032_1030, 0xe43c_8d5e_096c_7d2b),
            (18000, 0x6976_1f2a_3ee7_6d95, 0x6f7b_df02_17e1_9c6d),
        ],
        last: 0x69ee_6572_8cda_97ca,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xa2ff_0207_fce9_0ecc, 0x3166_daf9_f2ff_5e68),
            (3000, 0xc8e0_6184_ba13_e376, 0xb106_992c_a25c_4d06),
            (4500, 0x8c32_ebc7_fae6_dc6e, 0x0a09_e102_5ac7_11a1),
            (6000, 0xc196_020e_7aef_8607, 0xc32a_5a40_2785_92da),
            (7500, 0xfe74_578d_b8e9_a677, 0x1ee6_b203_a8cb_c58f),
            (9000, 0x62d7_c8fe_72e4_8b97, 0x6570_77ae_7c11_fae7),
            (10500, 0xcd12_c960_7982_a222, 0xdfa9_e2ec_8f86_ffd9),
            (12000, 0x288c_1b2d_cb8d_9d0b, 0xd81c_1e1d_33c8_9e95),
            (13500, 0x2d83_7abc_349a_9d8d, 0x8c12_8199_d6e7_2a5f),
            (15000, 0xa1eb_504e_d27b_ee8f, 0xc8fa_d98a_4dc0_0ae2),
            (16500, 0xacc1_cad0_bf33_8844, 0xebd0_81cc_deb4_1358),
            (18000, 0x4acd_6d43_c47b_dc62, 0x42b9_981d_01da_5a35),
        ],
        last: 0x7957_28c9_f30b_ca66,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x7e48_f6d3_1136_4e6a, 0x56db_a8b3_2bf9_fb3e),
            (3000, 0x862c_003a_2b37_2db9, 0x0549_914e_553b_bcbf),
            (4500, 0xfc3b_0cae_a021_0675, 0x9e95_20f5_636e_3e63),
            (6000, 0x428f_48a9_c32a_a1c3, 0xcd98_bf59_787f_3455),
            (7500, 0x2288_1b73_f2b7_cb95, 0xd0ea_3757_312d_61da),
            (9000, 0xab28_3135_a3db_6762, 0x5c5c_7f27_b271_91a1),
            (10500, 0xd001_52f7_45c6_d359, 0x19f9_5301_1833_bde7),
            (12000, 0xcf9d_db15_2a7f_d3bd, 0x2d86_aa45_d74d_17a3),
            (13500, 0xa690_f466_9929_599e, 0xae53_f3d5_fba4_7afb),
            (15000, 0xfb9f_ba08_a8ea_2d04, 0xbabc_70bd_7957_2b93),
            (16500, 0x6905_aa6c_8746_d689, 0x220f_1f9a_958f_6adc),
            (18000, 0x8e82_ef05_6411_9efb, 0x37db_b548_8d61_8301),
        ],
        last: 0x8e76_bb3a_38cc_69a4,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0787_fcdc_c203_a9e3, 0x30bd_f4f3_0475_f26a),
            (3000, 0x4545_874f_2c44_d073, 0x1d64_983d_1659_61f6),
            (4500, 0x24d8_0e55_bfd5_3950, 0xce58_2308_22d2_11ad),
            (6000, 0xd28c_4665_cca2_3bc0, 0x53ee_08bc_2564_dadf),
            (7500, 0x4a0d_9dff_70ba_0f5e, 0xdf6d_1db4_5b8c_1ad4),
            (9000, 0x5f8d_c0d9_cf43_7162, 0xc9b9_d738_e7cd_2eca),
            (10500, 0xddc1_3956_1a20_25bb, 0x20d8_29e2_3176_8ba5),
        ],
        last: 0x69b7_37b6_7903_bcbc,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2546_bb39_d080_7071, 0x433d_f3b0_adbc_b1ec),
            (3000, 0x9002_5d7e_ed66_2ab6, 0xe897_e02a_81a6_6c5f),
            (4500, 0x240c_66a0_1d9c_eba7, 0x338a_72fb_74c3_6fbd),
            (6000, 0xf8b9_e845_fc7b_15ac, 0xfc72_fc2c_51c5_11c4),
            (7500, 0x389c_47c7_364e_8967, 0xf8aa_2fc9_983d_d2e3),
        ],
        last: 0x1008_8217_5b7a_861b,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xe597_70f5_ca0e_838b, 0xe86f_25fa_fdff_9774),
            (3000, 0x590c_a32a_5dae_03ce, 0x4c78_a1b4_69cb_6c65),
            (4500, 0x070a_b450_56ad_6b91, 0x9187_177a_55f8_5a87),
            (6000, 0x42ce_3589_7602_dbb8, 0x81b2_0037_3004_58d6),
            (7500, 0x4052_71a4_f5b2_ccf6, 0x3e6c_a537_c241_7d48),
            (9000, 0x8945_48eb_4c2f_2a0d, 0x42b3_1f46_e562_a6d3),
        ],
        last: 0xf6b3_70c8_e542_3e59,
    },
];
