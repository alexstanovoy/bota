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
        (1500, 0xd8c9_f486_c45d_4ea4),
        (3000, 0x2550_954a_4f8f_c6b4),
        (4500, 0x461a_dee0_3234_0257),
        (6000, 0x30bb_bc12_0793_2f30),
    ],
    &[
        (1500, 0xf5d4_0856_d01b_6ec1),
        (3000, 0x8732_639a_0859_3f9d),
        (4500, 0x2670_cedb_4d18_07a6),
        (6000, 0xdfb5_cd8a_cfcc_464d),
    ],
    &[
        (1500, 0x5f1e_332c_8d2c_b334),
        (3000, 0x1430_ecf4_16f9_dc98),
        (4500, 0xdd69_423d_6834_e671),
        (6000, 0xb402_4bba_1234_8cd5),
    ],
    &[
        (1500, 0x9ffe_6e95_f0a6_e144),
        (3000, 0xcd99_68dd_2f52_c32e),
        (4500, 0x636c_c967_ca5a_3f06),
        (6000, 0xe1a9_812a_26ad_19f0),
    ],
    &[
        (1500, 0x8580_8c55_4396_c16d),
        (3000, 0xaaa1_9cf9_5387_706c),
        (4500, 0xb108_20fc_9654_fc01),
        (6000, 0x5758_95d8_2fd7_4e2d),
    ],
    &[
        (1500, 0x0d1e_ed7a_12ab_2f22),
        (3000, 0x1f8c_6a73_fce4_6b07),
        (4500, 0x844c_a877_8f01_ed4c),
        (6000, 0x27af_59f1_ea20_b386),
    ],
    &[
        (1500, 0x7d14_0c4c_570c_7070),
        (3000, 0xbdc6_9679_a634_e70e),
        (4500, 0xb229_81c0_05b1_e7d2),
        (6000, 0x12d7_d6b0_8344_bebb),
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
            (1500, 0xafb9_be72_dba1_eb9a, 0x5de1_ebd0_7c6c_8ab6),
            (3000, 0x0a74_dec3_5de7_f9b9, 0xf351_7685_9065_4649),
            (4500, 0xbe46_204b_1d02_e62d, 0x6d1b_f932_2bfa_7dd6),
            (6000, 0xc145_0607_260c_bd81, 0x18cb_41f7_ef48_2119),
            (7500, 0xb751_c9c9_9686_bd5b, 0xc681_5c75_5f43_d6d3),
            (9000, 0x80c0_603c_5234_f219, 0x3d98_6c97_dd83_1fc5),
            (10500, 0xe402_1681_ee75_c29a, 0xd297_405a_b884_82a9),
            (12000, 0x9fb6_f7b0_3c49_31e4, 0x89a9_1eaa_c9e0_5848),
            (13500, 0xc873_13bd_8c34_2558, 0xecce_e700_d458_e105),
            (15000, 0x3254_568d_90ae_1084, 0x67ae_24e6_84c6_c8fd),
            (16500, 0xc4ca_6a19_4c07_e57e, 0x0f6b_9fd6_1b66_e9c2),
            (18000, 0xcb2a_9f13_cb58_73f6, 0x164b_a935_c810_af5e),
        ],
        last: 0xa5e6_5963_9eff_9883,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe71a_1ae4_e04c_92bb, 0xfc42_ba0c_6ede_ce14),
            (3000, 0x75de_9578_5d8f_7082, 0x4ebb_6e35_a2cd_dd2f),
            (4500, 0x053b_a251_1f2f_0209, 0xcf17_b0d0_5778_7084),
            (6000, 0x2e29_e0a3_6eb0_de74, 0x80df_992d_b8d3_eb11),
            (7500, 0x2f8c_3e21_1259_27ec, 0x268b_f771_df30_30a1),
            (9000, 0x7ab5_bfea_a8c5_de12, 0xbc25_d24a_3e6e_3152),
            (10500, 0xf967_5dad_8353_00bb, 0x0a06_0d11_bcf2_375e),
            (12000, 0xa226_27c3_9143_b93d, 0xd095_309a_230a_3326),
            (13500, 0xf4c2_857d_a287_4035, 0x9704_6027_f2a2_510e),
            (15000, 0xd484_196a_2be8_233a, 0xd4d1_ddc4_2bb9_e582),
            (16500, 0xc2a4_79fe_c296_a0e8, 0x28ad_79bc_1f45_048a),
            (18000, 0x43ef_8935_5d36_cc88, 0x56b8_359d_992e_88c2),
        ],
        last: 0xc3d5_79f2_d05b_6d16,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xf3b9_5918_8951_519b, 0xc1ef_e725_289f_ca5c),
            (3000, 0xc339_c479_1931_36ef, 0x02d2_84e1_ca42_17e8),
            (4500, 0x7069_fdc2_9da7_17e1, 0xcd83_17b4_8ed1_40d3),
            (6000, 0x9ba2_456b_83c8_1e4f, 0xcfdc_7996_211c_c539),
            (7500, 0x2444_4b2e_cb52_73cd, 0xb161_fe25_5ba2_5fe5),
            (9000, 0xcd6c_8c3c_0eb3_69c3, 0xf293_6842_9b01_f79d),
            (10500, 0x8339_de4d_7f15_922c, 0x67bd_a5dc_3661_80f7),
            (12000, 0x058c_ec7d_9b9a_4d3b, 0x928f_4f58_c917_240c),
            (13500, 0x45f6_6c20_c190_56e3, 0xed17_2442_e50a_58c7),
            (15000, 0x6562_2979_0858_07d5, 0xf2f4_8cc3_6b7c_c53a),
            (16500, 0xec6a_7f5a_3ad0_e684, 0xfaa3_7cb9_5589_c397),
            (18000, 0xe05d_513a_c7d7_4a05, 0xf009_6d18_0f85_fb1e),
        ],
        last: 0x5377_b515_5757_ba89,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xd1f9_343c_9425_6ae3, 0x42f8_e3b2_01d7_0c17),
            (3000, 0x6045_c2c4_17b3_7167, 0x170c_9fa5_fd7b_c7ab),
            (4500, 0x9518_fd8c_5018_b532, 0xa483_6899_ef06_cacd),
            (6000, 0x38d9_dd6b_d769_a3e0, 0xc541_6eda_9911_258b),
            (7500, 0x3c7f_35a8_134d_d81e, 0x42b9_28e5_db06_a2b9),
            (9000, 0x3052_9265_9c91_f082, 0x6c41_6c97_419b_d9d7),
            (10500, 0x347f_b73d_59d7_b36d, 0x1428_d1a6_528e_26cf),
            (12000, 0xbf0a_8a84_8baa_f66a, 0x3fc9_cba6_d683_cdb5),
            (13500, 0x3248_82f9_4748_db99, 0xc97a_0342_b10e_8cf6),
            (15000, 0xaa1b_af99_cae8_5918, 0xb695_55b5_9ec3_ef0b),
            (16500, 0x29cb_642f_f451_965c, 0x014d_454f_b574_d844),
            (18000, 0xbd92_d518_6d29_bcc6, 0x39cc_f3d8_2d6e_e6e4),
        ],
        last: 0x1e63_7729_bfdf_32d0,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0571_29a0_1c95_5505, 0x459f_c5c7_f144_f59c),
            (3000, 0xadea_b22d_29d7_9f4c, 0x172a_f7d2_4ea9_f4bd),
            (4500, 0x8ebc_4590_5e19_4b3f, 0xa40a_07d6_2549_e58e),
            (6000, 0xda55_3ca5_44b7_5378, 0x0d8a_2080_0f89_1355),
            (7500, 0xec63_0cad_e747_0227, 0xdb95_e9b9_b023_02d9),
            (9000, 0xe304_3238_12c2_c20b, 0xb6e4_ef94_96f7_4716),
        ],
        last: 0xfba2_0483_cc9d_e30d,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd9bb_35c0_4dde_01e5, 0xad4c_f03a_f1d3_a8a9),
            (3000, 0xe5e8_7c55_dac6_b40f, 0xcc73_c25b_8941_f502),
            (4500, 0xa4eb_61ab_8687_cb7a, 0xdf00_cb13_437a_57a2),
            (6000, 0xb6e9_2148_356d_551e, 0x82c6_3e2f_4ce4_acd5),
            (7500, 0xb80b_7153_2e92_fd35, 0x3dba_9ac8_de7d_1849),
            (9000, 0x9d09_1776_8e45_9468, 0x3c65_7259_a45e_ef39),
            (10500, 0xb31f_3909_33dd_7ef8, 0xd337_c27c_a765_2cd4),
            (12000, 0xdbae_2189_9e66_403d, 0xc24e_3bab_bd8d_0207),
        ],
        last: 0xa1ad_a82b_a12d_5e5e,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3983_4873_dd21_ba13, 0x89e4_1f41_9431_6a02),
            (3000, 0xd116_88c5_9a8c_4aa4, 0x5631_88d1_6aeb_df49),
            (4500, 0x6f26_4ccc_c471_411a, 0x8991_0e8f_3cef_d054),
            (6000, 0xb08d_6860_3183_8a68, 0xae64_a93c_572f_a44d),
            (7500, 0x88be_48f9_cce9_d7f8, 0x1a6d_903b_4ad6_5475),
            (9000, 0x75a2_3f0e_db4c_7be6, 0x0bd3_27a9_da58_a1b8),
            (10500, 0x53a7_b0a7_3adc_1d2f, 0x0292_6594_cefe_920d),
        ],
        last: 0xde8f_864c_0843_bbad,
    },
];
