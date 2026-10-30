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
        (1500, 0x0ac9_f168_7ac9_3c00),
        (3000, 0xdaf2_dbbb_3a88_7110),
        (4500, 0x2c66_0a74_0a99_5187),
        (6000, 0xcef1_e3ee_b71b_69f1),
    ],
    &[
        (1500, 0xf9ce_6b20_27aa_9bdd),
        (3000, 0x0eeb_df14_87c5_1a5b),
        (4500, 0x1ab6_38ab_996b_1444),
        (6000, 0x6b5a_f8c5_00f6_adb3),
    ],
    &[
        (1500, 0x064d_edb2_4ddd_5c6d),
        (3000, 0xaf6a_7cbe_4a90_e95b),
        (4500, 0x054c_4e5f_78f4_2998),
        (6000, 0x7f57_1dd2_7223_f896),
    ],
    &[
        (1500, 0x7107_b6b6_fdb5_62c0),
        (3000, 0xcc42_e418_8cb8_4f06),
        (4500, 0xf588_03a7_13a8_bb1c),
        (6000, 0xe41e_268e_d647_445e),
    ],
    &[
        (1500, 0x2f69_d682_0e3a_c151),
        (3000, 0x012f_5249_c808_6e8d),
        (4500, 0x9c42_b88e_c2bf_ccf9),
        (6000, 0xa24a_20af_02f6_1e61),
    ],
    &[
        (1500, 0xdaba_7ebb_cfb8_1111),
        (3000, 0x1136_3065_12b9_d3db),
        (4500, 0x93a2_c880_55a5_c14c),
        (6000, 0x591b_4d0d_a42b_cb5a),
    ],
    &[
        (1500, 0x0f97_69dd_1472_29cf),
        (3000, 0x55d6_a9f8_1cf1_bb9b),
        (4500, 0xf671_36d0_ea35_c484),
        (6000, 0x7dd8_875e_3ee2_e22f),
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
            (1500, 0x27ba_b301_61a6_4400, 0x6b29_d536_ba17_8836),
            (3000, 0xf529_a896_e304_d682, 0x4f51_eb68_4cf0_ed10),
            (4500, 0xc7c4_f43a_7269_57dc, 0x8d12_85de_3071_f273),
            (6000, 0x6d5d_ae8e_acd9_2799, 0x5161_3d2d_3323_41f2),
            (7500, 0xf93f_a3e4_224b_8224, 0x902c_4edf_aedd_5c70),
            (9000, 0x254e_7eea_6766_ee68, 0x39fc_b1ae_a216_426f),
            (10500, 0x41ff_0234_73d3_f659, 0x7b48_b26f_2feb_1d52),
            (12000, 0x9b4f_05cb_55a8_c2f4, 0x91b8_1e78_63da_1928),
            (13500, 0x0695_4919_2894_8c10, 0x3c04_0ca1_551e_7619),
            (15000, 0xdd60_251c_9faf_fc88, 0x4c56_13bb_c31b_90dc),
            (16500, 0xc5e4_59f5_1b30_0658, 0x27fe_741a_a32c_78d4),
            (18000, 0x47e0_ee85_0ef5_164e, 0xbac2_f68c_706e_17a5),
        ],
        last: 0xdf36_4ddf_f003_4db9,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe2c2_54a9_54c8_8f7f, 0x35f6_b769_9033_610e),
            (3000, 0xfe13_be8c_f91c_d896, 0x1b92_7702_c736_78f5),
            (4500, 0x4678_8de5_6d56_6272, 0xe3c1_b40f_ad45_534e),
            (6000, 0x2839_c20d_b479_3cac, 0xc166_aa61_17a8_f359),
            (7500, 0x54f5_76b2_af8e_ac3c, 0xc4f1_2e79_12ab_43cd),
            (9000, 0x31a8_b6b5_f628_da7b, 0x9c47_0214_d460_a7db),
            (10500, 0x622d_cbad_8886_0238, 0x0758_e0c0_f498_cc55),
            (12000, 0x8ee4_7cc8_34cb_ba95, 0xd0dd_dc12_cc7f_858c),
            (13500, 0xcb1f_b0b4_9749_bcf5, 0xfb66_4a91_12bd_df54),
            (15000, 0x9fab_f1e9_3129_357b, 0x8c8a_2286_407b_0b1a),
            (16500, 0xff0d_cfbe_023e_c6e6, 0x9519_d701_aee7_ccbc),
            (18000, 0x729b_fcb2_7e7c_3709, 0x85d6_fc46_5374_f800),
        ],
        last: 0xe025_77fd_151d_8573,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x00ae_32e1_83d5_a068, 0xf834_f5d6_5751_01f7),
            (3000, 0x105f_9e15_9106_f25d, 0x6202_a9ce_814c_4b15),
            (4500, 0x0a53_8119_ae63_c4fd, 0x5871_6800_4bfc_43b2),
            (6000, 0xc04b_0093_fbdd_3805, 0x6f92_4a1c_8f4c_2e5c),
            (7500, 0x61d6_9789_b895_f18c, 0xe830_dea0_6346_45bb),
            (9000, 0x6dfe_55ba_9057_fe99, 0xd0df_1c16_0231_36e7),
            (10500, 0x7770_2958_a298_a49b, 0x248c_185b_3e8e_df1b),
            (12000, 0xb645_7323_cde6_d0a7, 0xb02f_1d58_0be7_9fb0),
            (13500, 0xe911_0a75_83dd_8dda, 0x86d6_5229_3852_0674),
            (15000, 0x2ccc_1dbd_bde4_5ffe, 0xa1f2_3a29_b662_133e),
            (16500, 0x646e_5990_2b67_e659, 0xd4aa_6c31_4afc_7407),
            (18000, 0x09fe_98cc_b070_305d, 0x01e6_c68e_5f08_17be),
        ],
        last: 0xed68_99d7_60ee_be31,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x736c_938a_b7cf_6e30, 0x9c87_dfd6_6e72_8cde),
            (3000, 0xf32e_c64b_ff82_f944, 0xd237_c97d_bef7_65a7),
            (4500, 0x697f_a77b_ce3c_327f, 0xd2e8_c3d1_f49c_ae36),
            (6000, 0x9202_0b2d_32b2_7a5a, 0xf37d_06c6_5ccd_93ca),
            (7500, 0x8de7_da5d_b33e_d1e2, 0x52ff_63f8_f5b4_a710),
            (9000, 0x00cb_2fd4_c2db_1f58, 0x61d9_6eb9_f856_13eb),
            (10500, 0xc7e8_cbb0_49d5_f6c8, 0x3b4c_67b9_2ce6_9098),
            (12000, 0x07c7_7c82_1592_57d9, 0xeb7b_9969_65f6_ce7a),
            (13500, 0xab29_71a0_9a07_4f84, 0x477f_fe81_4199_734e),
            (15000, 0x2d3d_4f4d_8396_6291, 0xe9c7_4f81_62c6_493d),
            (16500, 0xb171_4e3a_a271_81e0, 0xe552_a0d1_eb46_f10a),
            (18000, 0xbc3d_f71b_16ef_849d, 0x3622_0f49_5a25_86f8),
        ],
        last: 0xc4db_c00a_87a5_2999,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x75f6_cf4f_2c2b_0e46, 0x3e38_8f56_6c2a_a4f0),
            (3000, 0xe5a0_d51c_c759_7e57, 0x243d_ff96_c246_cb2e),
            (4500, 0xb3f8_4258_918e_4696, 0xd105_408b_ded9_1366),
            (6000, 0x21fb_36e2_4e78_c9c1, 0x0ecd_55d9_38a0_dfd2),
            (7500, 0xbb73_3d5e_ed08_920d, 0xa2b8_b7ed_008f_b651),
            (9000, 0x8c56_8a24_1384_b60b, 0x3d85_a52c_bbde_f975),
        ],
        last: 0x20f0_1544_fba3_bda1,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x899b_4cc1_3735_30fe, 0x34cc_61dc_a44c_a052),
            (3000, 0x827a_152d_d8ea_d85b, 0xbc1f_1133_d4a1_e8de),
            (4500, 0x921d_edc2_3caf_4e72, 0x6779_fe17_5ebb_9b54),
            (6000, 0x671e_572a_ce6f_825f, 0x4a71_75f6_c8d8_72da),
            (7500, 0x6262_7538_b2c2_fe38, 0x2b5d_10f8_5234_3297),
            (9000, 0x809b_d64c_8fe3_f0d3, 0xc7f9_9e0b_cc8d_958c),
            (10500, 0xb2b8_dad8_9b32_497c, 0xc925_7bec_cd05_e1fe),
            (12000, 0xee5c_823f_b81f_cd5d, 0x8c97_ef4d_a9b2_07a9),
        ],
        last: 0xaa01_90e7_1e3e_7a22,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xdda1_c267_e5ec_a4de, 0x62d3_0a06_7d02_f439),
            (3000, 0x4b6f_6635_6e89_63f3, 0x784d_786a_f551_07d9),
            (4500, 0xa9f5_c12f_b2c8_9d55, 0x3ac1_d984_b8ea_a2f2),
            (6000, 0xcd3b_5f7c_7501_8f00, 0x9ae0_c4e2_f157_e0e0),
            (7500, 0x253e_7e3c_3e45_fa17, 0x3f93_896c_54e6_67b9),
            (9000, 0xdc9c_83e2_1c22_c8f8, 0x53ed_d60d_3df4_2334),
            (10500, 0x1e88_f331_65a5_a903, 0x4598_86c3_55fd_9ddb),
        ],
        last: 0xc30f_7e73_a8f4_adea,
    },
];
