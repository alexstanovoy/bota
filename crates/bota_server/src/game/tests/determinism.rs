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
        (1500, 0x7363_4673_2340_0759),
        (3000, 0x2a72_701e_804c_44e4),
        (4500, 0xac4b_b2b8_eabf_5a1e),
        (6000, 0x1a0d_921d_4a10_7f53),
    ],
    &[
        (1500, 0xc6db_3117_ee24_705a),
        (3000, 0x46e2_ad3a_384e_1d3b),
        (4500, 0x544a_7980_78e6_57e3),
        (6000, 0x67df_c84d_37a7_2d29),
    ],
    &[
        (1500, 0xac93_360a_04c0_8a58),
        (3000, 0x2ce7_d364_914c_5b55),
        (4500, 0x8aa9_c28c_81f1_f030),
        (6000, 0x3c4c_6b33_8402_51b2),
    ],
    &[
        (1500, 0xae6e_0d81_09f1_3abd),
        (3000, 0x0d46_466d_2c06_4cdc),
        (4500, 0xbbb6_249b_b6db_d3e3),
        (6000, 0x5b99_f03b_b68e_67dc),
    ],
    &[
        (1500, 0x29c5_e67a_a9a0_6c72),
        (3000, 0x6fa7_17bf_7920_cd54),
        (4500, 0x4589_1eb9_8021_09ec),
        (6000, 0x88db_e6e0_71bf_9f09),
    ],
    &[
        (1500, 0x2d45_1990_c69d_7101),
        (3000, 0xc9db_6e71_787b_036b),
        (4500, 0x915e_7f7d_2746_bc34),
        (6000, 0x99a2_d834_b0fb_5e50),
    ],
    &[
        (1500, 0x4927_becb_e28b_c519),
        (3000, 0xa0d2_b32c_9f0e_5e2a),
        (4500, 0x715e_6a7e_073a_7ffa),
        (6000, 0x93a5_312a_03dc_193d),
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
            (1500, 0xa74f_f3b5_9f63_e935, 0x9c21_79aa_c774_5230),
            (3000, 0x3997_c863_824a_da6b, 0x8089_ea2e_251b_41a6),
            (4500, 0x921c_dec7_4b22_ced5, 0xf956_f96f_f73c_ba2e),
            (6000, 0x913e_ca14_9093_749c, 0xddce_9e94_9276_f941),
            (7500, 0x97bf_258b_1eb1_475f, 0x3f19_80ea_6440_2b49),
            (9000, 0xc9e9_83cb_90c7_e6fb, 0xf26b_278d_37fa_4893),
            (10500, 0xfcaf_2a6f_ce22_c8db, 0xbd43_dfe1_d7e0_2e74),
            (12000, 0x58c5_ee71_fb50_0d2c, 0x5746_e6c0_0021_137d),
            (13500, 0xb025_9ae2_33ca_5a31, 0x649f_5845_df8f_a9bf),
            (15000, 0x9c6e_6ad0_bc20_8e6e, 0xb413_e28b_ff19_f7fb),
            (16500, 0x6eb1_e852_e020_2c2d, 0x8a61_095e_5501_c680),
            (18000, 0xdd29_adff_9214_65e1, 0xbdd3_195d_3619_6756),
        ],
        last: 0x749a_6a2e_16e0_2ef0,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xac9c_df53_446e_ca50, 0x8331_be87_336e_5658),
            (3000, 0xa561_e1f8_515a_7874, 0x5597_72a0_7eb8_57d7),
            (4500, 0xda89_b772_d88b_1995, 0x631e_e0a0_1fbc_7b22),
            (6000, 0x85d2_2723_d058_58b7, 0x3d9e_b545_8f08_4b30),
            (7500, 0xc2ff_db37_5dae_f16d, 0xf60c_c5d5_2a19_6432),
            (9000, 0x5003_01f9_c443_b755, 0x22bc_0b49_2f02_d1e8),
            (10500, 0x47a5_97f9_ca3d_ee23, 0xbc0e_aa12_9f0d_0dca),
            (12000, 0xb4aa_b1c2_6138_f52f, 0xdc3c_fbd3_0660_7fd2),
            (13500, 0x3c5d_1495_091b_e02a, 0x0184_f7f2_3f38_8bb5),
            (15000, 0xbd9d_51bb_6faa_8af6, 0x9037_4605_7161_bf23),
            (16500, 0x2355_264c_e684_3b01, 0xc526_512a_03d3_9b3c),
            (18000, 0x87ed_a5de_1b50_3354, 0xb013_e837_2911_93f6),
        ],
        last: 0xa3ad_ef4a_a503_c02f,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x326f_15db_57d4_afe3, 0xed8a_5113_ff32_6348),
            (3000, 0xa308_4d59_61f2_e58d, 0xdc00_7d64_05ef_763f),
            (4500, 0x5527_3e15_5896_3865, 0xe931_4e63_fd33_1252),
            (6000, 0x476f_8b56_e661_b96c, 0xe6f8_a109_10bb_61e5),
            (7500, 0xbe55_e7d8_de4c_1f7c, 0xe543_8a8b_fa1a_61f0),
            (9000, 0x481f_54fb_058b_2860, 0xcb8e_3fd1_247d_76fe),
            (10500, 0xc4b1_464a_9775_2af5, 0xc8df_0577_f2e0_8105),
            (12000, 0x4665_d217_57bc_f5c6, 0xc3df_588c_c2d3_be22),
            (13500, 0x2c16_004c_ed52_e1c8, 0x3059_5dd9_7ca7_0784),
            (15000, 0x3443_0bcd_f4de_55b6, 0xbd75_0511_e185_cdbb),
            (16500, 0x2ad8_ef49_0536_5f89, 0x30e8_7e9d_8f63_9bec),
            (18000, 0x2610_5d4e_a05b_9d05, 0xb9e0_c349_923e_a44c),
        ],
        last: 0xf4f7_58da_e5c1_1e81,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x54c5_c225_237c_f23d, 0xd6ff_1ed5_10a4_6e7c),
            (3000, 0xdd7a_7d15_864b_6cba, 0xa57f_8cd8_43fb_049b),
            (4500, 0xec19_4ff2_a746_52e0, 0xdd22_3502_12b1_4569),
            (6000, 0xb9f0_44bb_ebfb_94fc, 0xb799_46e1_8112_01bd),
            (7500, 0x6305_bf2b_116a_5a2a, 0x353c_ba74_18f8_2274),
            (9000, 0xc557_be30_36f6_2fe9, 0x9e68_ebe5_3d89_86de),
            (10500, 0xd9b3_db8f_ecbd_808a, 0xd6b3_1320_7208_8410),
            (12000, 0x84d7_9a7e_1d20_9afe, 0x9b06_5e9e_a20f_c0fe),
            (13500, 0x98d2_fe1b_d527_e51f, 0x7bc6_fb58_1fcf_90b4),
            (15000, 0x7273_f60d_0748_1ac7, 0xcb79_064c_7ad6_e246),
            (16500, 0x0c5d_d936_abed_862a, 0xc0a7_2cc2_7285_34c0),
            (18000, 0xed17_ec69_b41d_b55c, 0x7b55_2281_ffbb_10e4),
        ],
        last: 0x72f8_22d8_a966_bc9b,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x32be_2c1d_4ee1_dbf0, 0x8dc8_4a27_b88e_928b),
            (3000, 0x9f0d_1694_edd3_9052, 0x556c_aacb_5890_d820),
            (4500, 0x9f1d_8301_28bd_f8cd, 0xa6dc_950d_d106_2f99),
            (6000, 0x026f_6a37_97e7_534f, 0x84b8_5091_4c1e_8d12),
            (7500, 0xd15e_4d17_e8b4_2031, 0x9958_6ec4_e9de_2043),
            (9000, 0x86e4_fb38_c1ac_e8c1, 0x0190_e3e8_0a5b_9f38),
            (10500, 0x0d33_1302_e948_4146, 0xd7c1_479e_114d_399f),
        ],
        last: 0x5acc_f786_fd12_e16b,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x119c_7ead_c30c_c112, 0xc44e_67cb_ce67_0e6d),
            (3000, 0x550b_b2d6_27fb_d9e3, 0x548b_68e2_3eb5_406f),
            (4500, 0x6cee_1544_662c_7774, 0x87f1_de2a_c533_7243),
            (6000, 0x65d2_d479_3041_6c21, 0x481a_8ba1_a4d5_6fbe),
            (7500, 0x1243_0b40_c1aa_5f68, 0xfe0a_dbaf_9309_061d),
        ],
        last: 0xc081_a9d6_c34e_1202,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x90a3_6d0f_0603_5a68, 0xd683_1685_5a92_0acb),
            (3000, 0xf67d_c880_2d4f_6c65, 0x2ec2_387d_567b_2ab7),
            (4500, 0x8a80_7b86_85ce_eb42, 0x74a0_1f8c_ae67_3ef2),
            (6000, 0xedec_c46e_6ea8_2fb1, 0xf3b5_3efc_bc70_1c2e),
            (7500, 0x622c_1c0d_e543_742f, 0xb8d7_47db_e46a_2e37),
            (9000, 0x371f_cbdc_01ae_1539, 0x5b92_c5e4_3780_2f6d),
        ],
        last: 0xf2f5_7edc_4668_4240,
    },
];
