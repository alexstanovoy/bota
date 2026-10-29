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
        (1500, 0x25f5_8120_74cb_dede),
        (3000, 0x30cd_509b_963c_9932),
        (4500, 0xe28c_d983_e611_2dc4),
        (6000, 0x9583_4348_3ae4_f385),
    ],
    &[
        (1500, 0xb73a_af81_9853_b986),
        (3000, 0x52f2_2009_62c8_41cf),
        (4500, 0xdfda_1ca6_dc44_18db),
        (6000, 0x1992_1d82_579a_0a09),
    ],
    &[
        (1500, 0x2c8b_2657_ca99_8d8d),
        (3000, 0x5091_017f_5e0c_32ab),
        (4500, 0xb85e_c311_c359_6f6d),
        (6000, 0xa90f_b74a_9091_d800),
    ],
    &[
        (1500, 0xe010_2d8c_56fd_3f5e),
        (3000, 0x7906_3137_ee96_10f1),
        (4500, 0x9b2d_cabb_d7e0_2628),
        (6000, 0x8fcf_da4e_352f_fc4b),
    ],
    &[
        (1500, 0x2dc0_abf5_ac9d_0cc0),
        (3000, 0x3ff9_0a1d_a7ea_21eb),
        (4500, 0x42b0_61e3_45d5_a2f5),
        (6000, 0xcf25_888d_6513_429d),
    ],
    &[
        (1500, 0xd4ef_48da_6bbd_51f9),
        (3000, 0xbf31_b3db_6bb7_8fd8),
        (4500, 0xa7b1_94da_b40e_2eae),
        (6000, 0x92ef_e806_d408_d482),
    ],
    &[
        (1500, 0x8c05_9143_f25f_9909),
        (3000, 0xe01c_d17d_4c2e_dfd3),
        (4500, 0x7168_afa8_5982_cb13),
        (6000, 0x57a6_5977_a7e7_85b6),
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
            (1500, 0x0449_a22e_f7a8_4400, 0xaedd_c97c_540b_19be),
            (3000, 0x70cb_eecf_681c_5554, 0x3acd_a720_a251_fc17),
            (4500, 0x9732_42f6_97ba_7db5, 0x5371_1f17_c301_782f),
            (6000, 0x9498_0c49_ffdc_a7c5, 0x9331_1c41_a6a3_077a),
            (7500, 0x7e45_26ee_69a4_8a85, 0xcd10_fccd_3635_3e4b),
            (9000, 0x5eed_d898_9914_26bf, 0xaa1e_a6cf_de2c_5d4a),
            (10500, 0x2749_e82d_cf71_1524, 0x912b_ce7a_4a20_869b),
            (12000, 0x92ff_a24e_4945_e098, 0x82a2_4c13_a273_3772),
            (13500, 0xcc9f_89ac_1947_8ba2, 0xcde6_1a8c_9c42_bd52),
            (15000, 0xbb20_d2bf_f7f6_df4e, 0x82c2_1f38_b5f3_c4be),
            (16500, 0x7b60_52a4_4ba8_14b8, 0xe1f6_cb04_d4de_6278),
            (18000, 0x268f_f7f0_62dc_7999, 0x887c_fb47_9f09_4181),
        ],
        last: 0x485e_3978_d6ad_141a,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7c00_8521_9466_f9c6, 0x5882_9661_3345_11b3),
            (3000, 0x0cc2_c9fb_b7c1_13f5, 0xc58b_bca4_5fdb_49c1),
            (4500, 0x0d0a_8e8c_a9a1_997b, 0xf493_472d_2d66_ae8e),
            (6000, 0x13c3_6f4a_d626_9fc9, 0x813b_f6ce_55c0_6a03),
            (7500, 0xeed5_e3b3_90da_2df6, 0x7e1c_38b2_f474_f2b3),
            (9000, 0xa144_a751_f166_40bd, 0x6a79_18ff_a5ff_15f8),
            (10500, 0x871d_128c_61e4_55b1, 0x82f4_0b11_ea4a_06b9),
            (12000, 0xd5e2_dde7_cb74_5948, 0xdfae_a563_ef36_fff4),
            (13500, 0x80ab_bdca_6b6b_aa58, 0x7765_9ff1_87f8_7ebd),
            (15000, 0x7416_c15d_a422_7269, 0x3f5d_0125_cf8e_3a54),
            (16500, 0xa185_dc6f_be0f_ca8f, 0xdc51_f0fe_fd7d_1248),
            (18000, 0x7bbc_9ec7_2a5a_70d3, 0x2d82_6362_225b_3186),
        ],
        last: 0x2e78_a7aa_612c_2428,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x9737_b782_9e91_fa0f, 0x84a4_0e7a_58aa_9568),
            (3000, 0xc389_481b_6737_13f9, 0x2c74_3038_87d5_73eb),
            (4500, 0x8818_e0ca_d885_d91a, 0x1231_1a59_609d_5423),
            (6000, 0x9542_3b15_75bb_29ee, 0x647c_61c9_9e9c_5575),
            (7500, 0x123a_48b6_8238_496e, 0x5103_55f9_22aa_cee2),
            (9000, 0xb305_efcf_3ab6_3645, 0xad7c_830c_8f64_3959),
            (10500, 0x4b73_9154_5801_ebc9, 0xcdd1_bc56_eba4_2a3c),
            (12000, 0x8ffd_e572_f9eb_18ec, 0x839f_1980_4728_53ac),
            (13500, 0xa74f_8046_2899_4382, 0x9476_8723_a021_a0e5),
            (15000, 0x9d22_ccc9_05d3_2fcd, 0x0ba5_b891_444a_e29e),
            (16500, 0x7e28_c85e_5510_1114, 0x0520_171b_485d_6032),
            (18000, 0x92d9_730e_5bb7_b358, 0xa21d_3c3d_a9c8_a4f2),
        ],
        last: 0x477d_9bb0_5972_43a7,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe58b_8b79_99b6_0749, 0x31ba_7aaf_e256_eedf),
            (3000, 0xf95e_2070_2f16_f7cf, 0x4da3_0331_6a59_ae0d),
            (4500, 0x7dbc_4c06_2237_3892, 0x537a_b530_4ecb_b5f7),
            (6000, 0xbf44_e53f_9cab_44aa, 0xf0c6_3045_7162_45cb),
            (7500, 0xfc29_7d46_b7c5_0414, 0x6855_4595_3b44_c701),
            (9000, 0x83f3_cd5e_4daf_8af8, 0xfdfb_ec20_591a_348f),
            (10500, 0x12ab_8189_c057_3c50, 0x7283_47d6_bbe5_87c4),
            (12000, 0xb51a_e41c_9ddf_0f2b, 0x1cf6_e75a_47e8_1a2a),
            (13500, 0x16e6_57f1_308b_4f29, 0xcdd9_7f88_fed9_6148),
            (15000, 0x4e61_84b0_b613_cbe6, 0x5bf0_ba47_8880_03f1),
            (16500, 0x1bf5_92d1_d929_e4fe, 0xf9a5_9f2d_3816_97a7),
            (18000, 0xdb45_92b4_4df3_0467, 0x0fe2_2d83_daf5_0541),
        ],
        last: 0x07d8_6948_26fd_c0b8,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x31e5_787e_78ab_a4f0, 0x573f_f23e_007f_8589),
            (3000, 0xc5a8_1d63_4451_ea8d, 0xd40b_5036_58ba_80db),
            (4500, 0x4998_e022_703d_c27c, 0x15cf_2b85_d568_0f7d),
            (6000, 0x34d3_5d7f_5150_56f3, 0x6625_9017_90bf_6f4b),
            (7500, 0xa701_a922_9a00_0c21, 0x07d4_3feb_4164_aed6),
        ],
        last: 0x16a4_2868_074c_5a5b,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x36c7_577e_ba90_be64, 0xed3a_f854_559a_c531),
            (3000, 0xb24e_045e_53e8_3565, 0x41cf_5690_2fdc_28b5),
            (4500, 0x5670_4250_ea2e_a213, 0xfe11_3cf3_edef_a129),
            (6000, 0xa578_7ea8_de71_b8f1, 0x49e1_99be_4560_dffc),
            (7500, 0x3966_f481_6eba_35a8, 0xbf79_48e9_36b6_9fc9),
        ],
        last: 0x0e07_76c5_a5ed_f3f2,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3a26_a426_ae8d_0bd4, 0x15b9_90d6_1519_aef7),
            (3000, 0x4317_4d61_b11f_d38e, 0x3aa5_1da4_ca64_eaa0),
            (4500, 0xdd82_4b97_b19f_895e, 0x4dfd_bb5f_eb8e_5857),
            (6000, 0xd26e_eba6_b5b9_60ba, 0x3e0a_ce57_637e_2a61),
            (7500, 0x3562_a7dd_d4b8_5a71, 0xaa13_0794_c29e_10e0),
            (9000, 0x51fd_60a2_090b_ce90, 0x0f46_32b3_1089_8145),
        ],
        last: 0x884d_0d3c_444f_fe6d,
    },
];
