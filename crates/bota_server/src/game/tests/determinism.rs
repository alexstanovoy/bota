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
    /// Digest of the final tick, world, serialized stream, winner and match statistics.
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
        (1500, 0xc04a_293f_24e1_c132),
        (3000, 0x949e_a800_91ce_31c3),
        (4500, 0xda89_c0c9_f497_ef33),
        (6000, 0x1552_3d85_ec78_2982),
    ],
    &[
        (1500, 0xff02_a1d5_82b3_9d6a),
        (3000, 0x8bf7_0f17_fa9c_8057),
        (4500, 0x9c98_543e_83f0_5174),
        (6000, 0xa9e4_0ef3_e16e_96d0),
    ],
    &[
        (1500, 0x5960_3e12_c5ab_b4ec),
        (3000, 0x585a_55f8_1769_e3a1),
        (4500, 0x8232_01d2_e647_52b0),
        (6000, 0x3526_382e_70c9_5e0c),
    ],
    &[
        (1500, 0xcc37_79e0_f034_6cee),
        (3000, 0x43db_b2d5_3882_ef8e),
        (4500, 0x3047_2f32_713f_1ee0),
        (6000, 0xda2c_9881_74f4_243d),
    ],
    &[
        (1500, 0xbb90_78dd_b2a3_5cc9),
        (3000, 0x91df_7baf_fb74_611a),
        (4500, 0x9b86_8e18_ad89_b90a),
        (6000, 0xaa44_9432_977b_cff6),
    ],
    &[
        (1500, 0xfca7_36a7_fae9_71cc),
        (3000, 0x03fc_84d4_f7e8_8424),
        (4500, 0x2563_4e24_8e6e_f7eb),
        (6000, 0xfcc1_1552_d7c7_e275),
    ],
    &[
        (1500, 0x3806_7800_eda2_d079),
        (3000, 0x6266_9de9_2afb_56fa),
        (4500, 0xc600_f1e7_8811_1352),
        (6000, 0x46bf_b28d_e4ce_fb4c),
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
            (1500, 0x2c5d_361b_78b3_e64e, 0xb4b0_86f1_e113_f3c6),
            (3000, 0x4d74_9f4a_a0b6_7070, 0xf208_2023_4b88_4aae),
            (4500, 0x2284_a593_8335_c2c3, 0x660f_72c8_c83f_1035),
            (6000, 0x87f0_9211_c4c8_b49a, 0xc628_f3c6_8cdb_f2d3),
            (7500, 0x8ebe_fbba_e6e8_fdf7, 0xeb89_784a_c7e0_d91b),
            (9000, 0x1eb4_62b7_fe3b_3549, 0xc298_da7c_d754_4c63),
            (10500, 0x361c_d7a0_6c1d_9e72, 0xcfe4_a308_f7a8_6f37),
            (12000, 0x0c0d_851d_32bc_e6dc, 0xb7e6_219b_17fe_4f19),
            (13500, 0x8660_e5fe_70ba_30ea, 0x1cf1_ad60_81f1_1fc7),
            (15000, 0x0f1e_7027_3035_6b4f, 0xd53e_b260_3d60_53ae),
            (16500, 0xcb51_f58b_c86f_d047, 0x2240_2a8e_8be9_f31c),
            (18000, 0xf66d_111a_4aac_579f, 0x957c_5c76_d866_62d3),
        ],
        last: 0xb9d5_d15a_e0e7_6708,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7b83_7436_e0fb_1d46, 0x3bf9_b0d0_6ebb_3728),
            (3000, 0xc893_6f8e_f1c0_4b09, 0xd3c3_12be_1199_a5b1),
            (4500, 0x81e4_27ea_97f9_64bd, 0x020c_5fbb_185b_9273),
            (6000, 0xb08d_1b57_c127_eff7, 0xe6ae_2340_886d_fefb),
            (7500, 0x1b61_7644_58ef_dbdb, 0x6c0e_a953_b729_a977),
            (9000, 0xdb77_2ac6_ff9a_0a23, 0x47f5_587f_08cb_170f),
            (10500, 0x29e7_fb14_c133_e9de, 0xa1b8_cab7_9641_ddb2),
            (12000, 0x90d6_37f8_e4b8_3c6a, 0xa973_5dc5_8c4c_61ae),
            (13500, 0x80b9_04af_77e0_1c47, 0xfa61_0208_35bd_7e05),
            (15000, 0x9b36_c62f_9203_185f, 0x43e6_2d62_3953_9f00),
            (16500, 0xd42b_e763_06d9_f2d0, 0x879d_31bf_fd30_10c3),
            (18000, 0x3e79_913e_9140_7aa6, 0x1c5a_3b42_4d3a_9251),
        ],
        last: 0xb2d4_0605_e8a5_00d1,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaf90_9f45_fa7f_5b62, 0x97ad_9b05_6819_2c81),
            (3000, 0xc7c3_0bda_04fb_a63f, 0x138d_4de5_f8b1_91e7),
            (4500, 0x0071_95e7_9320_85fe, 0x9a9d_1776_e807_35ef),
            (6000, 0x7597_4e71_dbc2_01a9, 0x75f3_c3ef_a2f3_47a4),
            (7500, 0xc8c3_3b54_0408_701b, 0x714d_8380_912d_7d0a),
            (9000, 0x7201_c010_524a_1559, 0x66af_29cb_36b2_6f18),
            (10500, 0x5f2e_694e_4266_e73a, 0x098d_9a57_946b_caf8),
            (12000, 0x85a1_f2e7_899d_4bf0, 0x2401_f0ab_a72a_b354),
            (13500, 0xc7a7_45cd_e436_8328, 0x59e4_a933_e1bd_ed1b),
            (15000, 0xd3ad_2cc5_5054_9062, 0x1d96_f12a_3820_2c51),
            (16500, 0x6553_ec54_9276_fcfd, 0x47eb_73b2_eee8_8914),
            (18000, 0xe715_41e3_b1ab_1c9e, 0x29cc_ccbb_05d8_aa18),
        ],
        last: 0x90ce_aef4_3c97_5c07,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2ba4_2dbe_47bb_e2a6, 0xcb5e_ab35_f697_c75e),
            (3000, 0x445f_4d29_c3d8_29d4, 0x3e89_12d4_ecdd_aee3),
            (4500, 0x6c40_bb0c_3761_032b, 0x26d4_51c2_c58f_d15e),
            (6000, 0x4dc8_f23d_ca35_0e96, 0x4d92_efed_ec73_799f),
            (7500, 0x91c3_32cd_e71f_d5c5, 0xe3ab_81a2_6cd3_5a10),
            (9000, 0xfe54_8e36_251a_3413, 0xa998_4d4c_7e34_ebfb),
            (10500, 0x6b0a_bd2f_f59d_946e, 0x07a9_00d3_2812_3d1a),
            (12000, 0x1ae9_232b_8486_a713, 0x069d_075e_c640_5ed9),
            (13500, 0xac1e_0282_5be6_5f8a, 0x295e_2b34_aced_a53d),
            (15000, 0x2423_1346_92a6_91e5, 0xd349_14aa_214c_6092),
            (16500, 0x380c_296c_491b_b31a, 0xde01_c1b1_d179_dc3d),
            (18000, 0x9225_8b62_c8c9_f47f, 0x250f_d25c_f75b_0fc7),
        ],
        last: 0xc714_cbbe_582a_aa12,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x592b_4590_5ffe_4680, 0xc65f_ce66_d9ad_4b1d),
            (3000, 0x5ce8_eea9_b4a9_9a19, 0x7de0_6d16_1bd6_6c76),
            (4500, 0x8734_a2ef_bf14_a91a, 0x14c5_bdee_5257_ae1e),
            (6000, 0x9e63_05c1_1fcf_b5ed, 0x0bfb_ef4d_df0e_bd46),
            (7500, 0x61d3_980e_43b9_2969, 0x5287_2884_0de9_8882),
            (9000, 0x4c7d_099a_b0e3_fe59, 0xa970_e3b5_c8fd_cb0d),
        ],
        last: 0xa537_b925_7f5a_0829,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5fbb_2231_fd27_7eaa, 0xfd1e_b3e0_110f_9291),
            (3000, 0x4178_c1d9_6c7f_018b, 0x7de8_1deb_c20a_840f),
            (4500, 0x226b_f928_21e1_a988, 0x0b1c_3c1a_8909_1bbe),
            (6000, 0xe688_8ac5_3bbb_4b41, 0xe653_6a4e_a2c7_ce66),
            (7500, 0x14ff_47a3_9ee9_462b, 0x149f_ccc4_ef28_97a1),
        ],
        last: 0x31bb_4198_6425_a8e0,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x1431_8d9c_3625_3052, 0x3444_ca48_4b2a_d867),
            (3000, 0xa93e_ba8c_ea18_9a64, 0x46c6_093c_1853_a125),
            (4500, 0x7523_1116_615e_1df4, 0x6edb_f60d_9ce6_7e76),
            (6000, 0xd15e_ca8f_37ff_1ca8, 0xaedb_4b2a_f276_ab69),
            (7500, 0x5055_d64c_a560_13d5, 0xd1eb_e2de_e09e_2519),
        ],
        last: 0x1550_4999_2f8b_e3eb,
    },
];
