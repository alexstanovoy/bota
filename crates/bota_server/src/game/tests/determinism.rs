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
        (1500, 0xb03a_5dab_eddf_d642),
        (3000, 0x7434_e256_88d0_a71c),
        (4500, 0x44b0_5211_fb71_52bc),
        (6000, 0x2a75_0d11_7e12_2f5d),
    ],
    &[
        (1500, 0xb017_fb35_c53a_955e),
        (3000, 0x5592_dce8_6538_ac82),
        (4500, 0xf5a5_13a3_07bc_7a51),
        (6000, 0x247c_4057_98cb_fecf),
    ],
    &[
        (1500, 0xfcb0_6c20_4ac0_d493),
        (3000, 0xbae9_13ea_39e8_f245),
        (4500, 0x28b1_bc96_9a2f_4efe),
        (6000, 0x5a73_bf7c_d090_69f5),
    ],
    &[
        (1500, 0x37ee_15e8_abf8_8ca3),
        (3000, 0xd697_9f77_2a4d_f965),
        (4500, 0xe112_7b7c_fb81_05a2),
        (6000, 0x47cd_07bc_16ea_366f),
    ],
    &[
        (1500, 0x205b_1f63_eab6_ed4c),
        (3000, 0x4cad_75d8_764e_92e9),
        (4500, 0x2ce2_8607_d5b3_7ae2),
        (6000, 0x6246_46d7_874d_76a8),
    ],
    &[
        (1500, 0xbb2d_66b9_2a43_dfb6),
        (3000, 0x1d34_734f_4e6b_391e),
        (4500, 0x8833_5dd7_a332_5f87),
        (6000, 0x1b5b_2b88_ed61_174c),
    ],
    &[
        (1500, 0xf36b_bbf2_1bd8_ab01),
        (3000, 0xd89d_96c9_cd4c_06a8),
        (4500, 0x6ee7_5db4_8782_9cf9),
        (6000, 0x5d7d_934d_5a54_da89),
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
            (1500, 0xc29c_af6c_5b56_4d2a, 0xe0d5_2bb0_3216_f04e),
            (3000, 0x5fb5_fd6d_9dda_ea65, 0xfc9d_997a_c70d_12e8),
            (4500, 0x6415_fc65_ddf0_b3e0, 0xb7e9_5717_8d59_3322),
            (6000, 0x271c_1f7d_5196_c977, 0x3a44_0c41_a367_e2e2),
            (7500, 0x340d_5b79_6468_2442, 0xc3ca_577a_4430_2272),
            (9000, 0xdbc5_5bc1_7aeb_ed02, 0x1ab5_821f_d422_cbf4),
            (10500, 0x941c_3b7f_3187_987b, 0xba0c_0fc1_34b5_242e),
            (12000, 0x8e74_da41_60fa_a2ab, 0xa562_a957_c313_6c56),
            (13500, 0xe098_946c_f16a_2318, 0x5a26_8c20_6815_583c),
            (15000, 0x8bf5_5f0c_540c_f6e2, 0xd5f7_d027_373c_5773),
            (16500, 0x4dec_5a1d_6e18_89be, 0x0d44_4185_abd5_b298),
            (18000, 0x43a3_ea78_2aed_bc87, 0x20bd_836e_b8bf_5a9e),
        ],
        last: 0x1939_e1c3_3f98_4dde,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xbb9a_2ca8_e14b_f4c9, 0x3c95_b674_56da_02a2),
            (3000, 0x3036_5378_6447_9606, 0x8f2c_4129_f7ff_1a34),
            (4500, 0x1bc6_955d_4783_aed1, 0xe6b0_d695_6536_90de),
            (6000, 0x962c_94bd_dd26_5729, 0xc467_7f08_dfbf_2943),
            (7500, 0xfe47_9ff7_261c_2647, 0xea9f_9066_42cc_4728),
            (9000, 0x5f9c_67bb_03a5_987e, 0x1081_68db_f45d_ac72),
            (10500, 0x18bc_ad81_0409_72f5, 0x7c03_d6d0_9cfd_bb28),
            (12000, 0xc5bc_6507_9116_12be, 0x9f52_e749_dc4d_7047),
            (13500, 0x04e6_f046_b000_b532, 0x4e14_76ae_d601_2153),
            (15000, 0x480b_b344_d19c_55e8, 0xfca6_ac59_5511_79d9),
            (16500, 0x94bb_671a_c53c_f1c8, 0x1cb9_80b4_e18d_2ef2),
            (18000, 0x2066_c37f_770a_e2d9, 0x68a7_16aa_218b_0df3),
        ],
        last: 0xce33_06d1_4b5c_6acd,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaad1_6bbf_84f9_ac6c, 0xad1c_25e2_169b_2dde),
            (3000, 0x7284_38fb_1d6e_f674, 0xe9d1_4706_6ea6_7202),
            (4500, 0x4b88_5fef_337a_abe6, 0x1807_ed33_8982_6546),
            (6000, 0xa35f_aedb_57c6_fcde, 0x12e2_508e_5cc4_2aed),
            (7500, 0xbc07_5b87_e76b_e996, 0x07c3_794b_fcee_a148),
            (9000, 0x8aa9_ee45_4eb4_5ad9, 0x1d8d_2fde_a90f_85a2),
            (10500, 0x2192_c307_e360_9552, 0x7b78_e52a_234e_79ef),
            (12000, 0x1db6_ea5f_d5f5_5159, 0x8625_3f84_5db3_470b),
            (13500, 0x22d3_5237_9130_70c1, 0x2980_536f_e822_4f09),
            (15000, 0xf949_3f3c_9ac1_1ae0, 0x8dd4_05ed_2ce6_aa2d),
            (16500, 0xe4e0_d8f0_e90a_c756, 0x4ae2_5272_22ae_7a67),
            (18000, 0x676c_06cc_dbb2_e948, 0x2df9_1a4b_561d_b74b),
        ],
        last: 0x7621_c214_1afe_9309,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe302_3c28_9021_704d, 0x4fab_0705_4523_829e),
            (3000, 0x2e5d_657a_37bb_a0b3, 0xd75a_5fe4_bec2_5c81),
            (4500, 0xe6d0_1c5b_2b95_4e5b, 0x6884_c4cc_5df8_833e),
            (6000, 0x4465_81ea_f33c_8518, 0x3eda_3f7a_9b70_2d02),
            (7500, 0x75f6_24f0_f148_b2f3, 0xa72e_46d4_bd8d_3d7f),
            (9000, 0x1a7e_0401_473d_5907, 0xfd8c_2bf7_ad2b_4241),
            (10500, 0x4878_7fb8_7eec_f719, 0x6767_5291_705d_f575),
            (12000, 0x9702_55c7_95ea_b29b, 0xf95b_e9e5_2342_220c),
            (13500, 0x1c70_97c0_6cc3_a9cc, 0x1688_1a1c_185d_6090),
            (15000, 0x1bde_5f3c_6ca7_3fc5, 0xe39b_2db5_c0b5_06db),
            (16500, 0x48a8_2e16_2106_2d73, 0x152f_bcbf_8ab4_39ee),
            (18000, 0x95e9_b0db_9a4e_f02e, 0x6eec_63a3_09b4_ffbe),
        ],
        last: 0xc210_b49f_5c9b_a4e7,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xfdea_e5f3_1e9a_c79a, 0xe596_7071_4a3d_ae13),
            (3000, 0xc580_ea2a_6618_4e3f, 0x5256_330f_59a2_ba3e),
            (4500, 0x3e07_d1d7_7eb1_841f, 0x39a3_7901_bf55_6483),
            (6000, 0x7b92_6424_14e4_3205, 0xd005_1142_c766_0717),
            (7500, 0x3ce4_1b9f_b478_f01e, 0xb657_2815_5540_6840),
            (9000, 0x499e_ff06_949e_e863, 0xbdf4_157f_a79e_20a3),
            (10500, 0x29a7_79a5_6b21_b5c0, 0xc2d4_c9ce_a720_ae81),
        ],
        last: 0x87e5_c2fe_15a0_643f,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbb4a_0f5b_7377_4c88, 0x9ef3_c317_a860_5229),
            (3000, 0x8afe_6406_92af_4658, 0x3827_6366_30b6_dbc7),
            (4500, 0x81b4_c283_84a2_7d69, 0x20a3_de03_c193_ca48),
            (6000, 0x38b9_6d73_f629_ed05, 0x9fbc_8aca_c92c_b65a),
            (7500, 0x0fa3_04e0_f413_60cf, 0x2471_45e8_f139_567e),
            (9000, 0x5aaa_0e69_de05_27b2, 0x7264_adbb_425c_9e2c),
            (10500, 0x8955_2319_b0ba_f33a, 0x0d87_7198_dd9b_e0f3),
            (12000, 0x904b_0491_b0cd_e1f1, 0x017d_8457_8969_8038),
            (13500, 0xdd14_7766_3ffc_59bf, 0xc95c_bef3_2da8_c1fb),
            (15000, 0x3407_128c_c882_d1eb, 0x9d49_7776_9936_d198),
        ],
        last: 0x8e12_e166_9872_3135,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x93b6_924c_e0e3_5d4e, 0x313c_22a6_4d24_e469),
            (3000, 0x6fdb_d1fb_51cd_cb67, 0x938f_9210_1278_c724),
            (4500, 0xb609_984a_6d55_6a43, 0x51f8_c551_9b17_e1ff),
            (6000, 0x91c3_8fad_63ff_fd7b, 0x29e1_db01_1aff_b3f5),
            (7500, 0x7abf_bfc6_9c90_49b9, 0x65b4_5f06_123d_0836),
            (9000, 0xbf01_8669_5bc1_dfb1, 0xf503_bacb_14fa_16c9),
            (10500, 0xeed4_e5e6_0253_9094, 0x93ca_5532_8b6d_fa65),
        ],
        last: 0x65e6_cbfc_bb31_b056,
    },
];
