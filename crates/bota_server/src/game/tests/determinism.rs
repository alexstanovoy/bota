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
        (1500, 0xb03a_5dab_eddf_d642),
        (3000, 0x7434_e256_88d0_a71c),
        (4500, 0x44b0_5211_fb71_52bc),
        (6000, 0x2a75_0d11_7e12_2f5d),
    ],
    &[
        (1500, 0xb017_fb35_c53a_955e),
        (3000, 0x5592_dce8_6538_ac82),
        (4500, 0xf5a5_13a3_07bc_7a51),
        (6000, 0x913d_38f8_4609_de7b),
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
            (1500, 0xc29c_af6c_5b56_4d2a, 0x2d6f_9d41_e643_d81f),
            (3000, 0x3a7e_8f11_288a_2d58, 0xed8b_4275_75ec_e220),
            (4500, 0x3fac_1d61_8d72_6c20, 0xe072_a55f_e0d4_ca53),
            (6000, 0xba7a_e67a_686a_d99b, 0xf6c8_3d3f_283d_eda4),
            (7500, 0xa7ca_8528_9b80_4a7b, 0x31dc_8220_1298_faff),
            (9000, 0x4382_979c_7da8_1958, 0x348f_3862_c0e7_d588),
            (10500, 0x3bfa_98e5_a2c7_a733, 0x00f0_032e_8bb5_b608),
            (12000, 0xfb1c_3227_2deb_fe94, 0xa5dd_2475_3d0a_4020),
            (13500, 0xe212_d05c_4923_501c, 0x5380_926b_6960_0ecd),
            (15000, 0x1455_10d6_8f0d_dc63, 0xc8f6_aa4d_33fa_e5aa),
            (16500, 0x0d4c_d37b_f72e_3392, 0x0062_bda9_c6ce_d22f),
            (18000, 0x7bef_7db1_ad03_3b22, 0x7bf2_63da_1916_7537),
        ],
        last: 0x41b6_16fc_4752_d9f8,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xbb9a_2ca8_e14b_f4c9, 0xd368_fa9e_8103_99db),
            (3000, 0x3036_5378_6447_9606, 0xbe6a_a115_0450_b3d0),
            (4500, 0x1bc6_955d_4783_aed1, 0x07dd_46b7_d3fe_272b),
            (6000, 0x962c_94bd_dd26_5729, 0x70b1_72a1_e457_63d1),
            (7500, 0xfe47_9ff7_261c_2647, 0xe943_2422_a294_609a),
            (9000, 0x1019_5a1d_a954_fbc8, 0x0fff_66ef_ff17_1829),
            (10500, 0xd7a0_5e90_cf2c_8b72, 0xe33a_1537_59b0_81a7),
            (12000, 0x7f4d_5b30_f2b2_b91d, 0x1e1d_2a85_525b_3600),
            (13500, 0x51cf_02c2_6f0e_11cd, 0x06e8_cf40_a7c7_1991),
            (15000, 0xb33b_2504_7f3f_a35a, 0x72ac_be4f_5e78_1485),
            (16500, 0x7623_2438_0312_e319, 0xc451_0f1e_7170_00b7),
            (18000, 0x8a2f_db2c_a065_3763, 0x5a7a_69d6_8da3_df8f),
        ],
        last: 0x96ba_b888_7a1b_5cba,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaad1_6bbf_84f9_ac6c, 0x0ac1_5b35_ad32_14ca),
            (3000, 0x7284_38fb_1d6e_f674, 0x67ad_18ce_252f_3a2a),
            (4500, 0x4b88_5fef_337a_abe6, 0x902d_f2bd_461a_0ad6),
            (6000, 0xa35f_aedb_57c6_fcde, 0xebe5_41cb_2420_eb27),
            (7500, 0xbc07_5b87_e76b_e996, 0xe25c_41a4_b238_b426),
            (9000, 0x8aa9_ee45_4eb4_5ad9, 0x1bad_7ed3_7592_3acf),
            (10500, 0x6418_bdd8_44a0_7bcc, 0xc162_9cf9_794d_399b),
            (12000, 0x429c_e7e8_1bef_0378, 0xf7b6_7252_57ef_77a2),
            (13500, 0x84b9_464b_b301_5f1f, 0xdf23_1bfd_8884_0599),
            (15000, 0xf0ce_19e5_85e7_7908, 0x483c_524b_eba9_0ad7),
            (16500, 0x2ad7_9c3d_19dd_986b, 0x8be2_d3c7_c728_3edb),
            (18000, 0x0766_c112_dfbd_0eb6, 0xa0e1_240a_e7b1_2b80),
        ],
        last: 0x076c_2417_53fc_15fc,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe302_3c28_9021_704d, 0xd68b_d8ad_77a5_dd19),
            (3000, 0x6eab_e1f6_b5ef_0566, 0xca90_2dc0_8ef1_b2e7),
            (4500, 0xb290_2a28_590c_9499, 0x28ff_9254_0830_eed0),
            (6000, 0x8d85_fc23_fe3d_3087, 0x5030_5956_8112_78a9),
            (7500, 0x5b0e_dc4f_9f3c_4e31, 0x5fbc_c90a_ce0d_1f36),
            (9000, 0xad4a_58c5_07bc_7abd, 0x5dd4_3286_1a7c_b2ca),
            (10500, 0x1b55_39f8_c3ee_ee68, 0x3308_4d4e_3fe1_0900),
            (12000, 0x1135_ae96_bd05_b7e0, 0xbd62_a913_c213_ff8f),
            (13500, 0x342c_5cb7_0095_2b59, 0x50e4_93e4_d4fb_34f1),
            (15000, 0xff55_b7fd_cec8_05ab, 0xb9a5_aee3_df48_fc96),
            (16500, 0x6dd1_c02f_560d_f1ad, 0x2236_e86f_6f29_a055),
            (18000, 0x6258_242a_f225_40ba, 0xace3_bef4_572d_a56b),
        ],
        last: 0x9fee_766d_28e3_921b,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xfdea_e5f3_1e9a_c79a, 0x7437_7e95_87be_81ae),
            (3000, 0xc580_ea2a_6618_4e3f, 0xfcfe_791a_7685_1a8a),
            (4500, 0x3e07_d1d7_7eb1_841f, 0x225d_8dae_03b7_35a0),
            (6000, 0x7b92_6424_14e4_3205, 0xc0ff_2ab2_7781_7b89),
            (7500, 0x3ce4_1b9f_b478_f01e, 0xea1b_db94_cc72_5661),
            (9000, 0x499e_ff06_949e_e863, 0x7d68_35f2_5304_5d5a),
            (10500, 0x29a7_79a5_6b21_b5c0, 0x590c_9e41_4356_a335),
        ],
        last: 0x33de_4dbc_9967_851a,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbb4a_0f5b_7377_4c88, 0x9f9c_f929_7c53_fef1),
            (3000, 0x1009_2453_f47f_36d0, 0xb297_8b85_24fe_4dff),
            (4500, 0x5db9_35b2_a22c_adf1, 0x686b_72bd_db93_6cbd),
            (6000, 0xfa33_95df_e1c4_8c7b, 0x8217_f018_95eb_ed78),
            (7500, 0x3ad2_7c92_32fe_6131, 0x2475_e03b_c4f6_66d6),
            (9000, 0x142f_17db_31dc_d38e, 0xd8ac_f2f1_8929_98ff),
            (10500, 0x7d91_eade_71fc_ac59, 0xbbca_d5bf_6e28_e0f3),
            (12000, 0x68e6_4a8e_925f_a40c, 0x9477_a685_d68d_eeb0),
            (13500, 0xdd8f_a675_35f9_9393, 0x81c1_2a6f_316f_67c8),
            (15000, 0x910a_6b08_a102_a936, 0x61e3_b856_1375_f574),
            (16500, 0x4bae_ce5d_1e85_be07, 0x2130_e042_6b26_1882),
        ],
        last: 0xb96e_6201_a870_90dc,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x93b6_924c_e0e3_5d4e, 0x6f50_7761_be72_4c09),
            (3000, 0x6fdb_d1fb_51cd_cb67, 0x0638_8a3d_22c9_cdba),
            (4500, 0x29b3_a6b3_b2cb_a6db, 0x35a0_54a1_630d_444c),
            (6000, 0x46c9_fa9d_d78a_1903, 0x2c5a_adbb_3237_5589),
            (7500, 0xc30e_e44a_e293_ab0f, 0x5dfe_1f87_8b0c_80df),
            (9000, 0xce4a_643e_d673_51ba, 0x5910_2668_8029_f437),
            (10500, 0x5e7b_1b5c_5d7a_35ed, 0x2a44_8c15_05e9_3d98),
        ],
        last: 0xb34a_4f80_4493_b011,
    },
];
