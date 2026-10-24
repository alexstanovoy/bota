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
            (1500, 0x27ba_b301_61a6_4400, 0xa39f_583a_3f1d_f592),
            (3000, 0xf529_a896_e304_d682, 0x578f_8667_e8a8_de7e),
            (4500, 0xc7c4_f43a_7269_57dc, 0xa8b0_8c17_b5c3_bacd),
            (6000, 0x6d5d_ae8e_acd9_2799, 0xf330_8bca_4665_889f),
            (7500, 0xf93f_a3e4_224b_8224, 0x4f66_0e7e_5194_540b),
            (9000, 0x254e_7eea_6766_ee68, 0xaa00_eff4_5d0d_2eb1),
            (10500, 0x41ff_0234_73d3_f659, 0x1a57_db90_379f_4b7a),
            (12000, 0x9b4f_05cb_55a8_c2f4, 0x8c87_39ff_804f_145b),
            (13500, 0x0695_4919_2894_8c10, 0x3c9c_7792_99bf_02c4),
            (15000, 0xdd60_251c_9faf_fc88, 0xb030_b309_c2b8_a36c),
            (16500, 0xc5e4_59f5_1b30_0658, 0xaea4_56c3_0db3_1955),
            (18000, 0x47e0_ee85_0ef5_164e, 0x15dd_2c77_6d73_3ef0),
        ],
        last: 0x9fd7_7b04_2f69_6370,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xe2c2_54a9_54c8_8f7f, 0xa045_7c0f_dffe_ee05),
            (3000, 0xfe13_be8c_f91c_d896, 0x0d64_673e_8e03_b9f2),
            (4500, 0x4678_8de5_6d56_6272, 0x3f92_dcdc_a63c_1f83),
            (6000, 0x2839_c20d_b479_3cac, 0x24b4_3540_3b06_99f7),
            (7500, 0x54f5_76b2_af8e_ac3c, 0x1759_befb_1d8e_cd84),
            (9000, 0x31a8_b6b5_f628_da7b, 0x9344_9444_28b5_b119),
            (10500, 0x622d_cbad_8886_0238, 0x3913_27b7_04b3_ccc6),
            (12000, 0x8ee4_7cc8_34cb_ba95, 0x84ba_02a0_c7b6_256f),
            (13500, 0xcb1f_b0b4_9749_bcf5, 0xd101_4489_a992_e1a5),
            (15000, 0x9fab_f1e9_3129_357b, 0x9e1d_69ac_27e9_7871),
            (16500, 0xff0d_cfbe_023e_c6e6, 0xcc79_e21e_6fce_4c8b),
            (18000, 0x729b_fcb2_7e7c_3709, 0xa76d_4b5e_e5a2_c19d),
        ],
        last: 0x6cd9_f0e3_3d06_d6c5,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x00ae_32e1_83d5_a068, 0xb826_502d_5251_12ee),
            (3000, 0x105f_9e15_9106_f25d, 0xba5d_866f_b013_77b4),
            (4500, 0x0a53_8119_ae63_c4fd, 0xeb30_9f25_56f5_4e0b),
            (6000, 0xc04b_0093_fbdd_3805, 0x63c0_8228_4092_9c21),
            (7500, 0x61d6_9789_b895_f18c, 0x3755_495c_fef8_d8a5),
            (9000, 0x6dfe_55ba_9057_fe99, 0x21fa_2927_fbf0_915e),
            (10500, 0x7770_2958_a298_a49b, 0x70b1_97b9_e9b5_4364),
            (12000, 0xb645_7323_cde6_d0a7, 0xd188_1ea7_4aab_da78),
            (13500, 0xe911_0a75_83dd_8dda, 0x2dc5_4871_5c25_b4f9),
            (15000, 0x2ccc_1dbd_bde4_5ffe, 0xbe7d_eb99_c11a_a120),
            (16500, 0x646e_5990_2b67_e659, 0x0296_4f2f_311d_07f7),
            (18000, 0x09fe_98cc_b070_305d, 0xc05e_fa08_61fe_36b5),
        ],
        last: 0xc5a7_e433_7e2b_1728,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x736c_938a_b7cf_6e30, 0xe18b_14e4_c1e7_e35c),
            (3000, 0xf32e_c64b_ff82_f944, 0xb0a3_f060_f1f9_ff60),
            (4500, 0x697f_a77b_ce3c_327f, 0x092e_0f45_0ea1_1a65),
            (6000, 0x9202_0b2d_32b2_7a5a, 0x09b4_8788_d1d5_5bdf),
            (7500, 0x8de7_da5d_b33e_d1e2, 0x10ac_3e29_43ba_5ca3),
            (9000, 0x00cb_2fd4_c2db_1f58, 0x7276_5123_0fe7_e07c),
            (10500, 0xc7e8_cbb0_49d5_f6c8, 0x1136_7715_fa4a_ac1f),
            (12000, 0x07c7_7c82_1592_57d9, 0xd0b7_f9d6_8efa_a713),
            (13500, 0xab29_71a0_9a07_4f84, 0x98c4_a305_d3e4_1257),
            (15000, 0x2d3d_4f4d_8396_6291, 0x100b_c751_57a8_f98d),
            (16500, 0xb171_4e3a_a271_81e0, 0x540e_22ad_6ebe_f409),
            (18000, 0xbc3d_f71b_16ef_849d, 0x7cd7_d7e2_fd2c_f602),
        ],
        last: 0xc3e2_c96b_bdcd_8feb,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x75f6_cf4f_2c2b_0e46, 0xf8f9_380e_2fc2_e3fb),
            (3000, 0xe5a0_d51c_c759_7e57, 0x8e7e_b77a_8b0b_e09c),
            (4500, 0xb3f8_4258_918e_4696, 0x5cf4_c2fb_80c0_d3b8),
            (6000, 0x21fb_36e2_4e78_c9c1, 0xf019_8033_8be4_c3eb),
            (7500, 0xbb73_3d5e_ed08_920d, 0x0afb_ff71_9051_7ae1),
            (9000, 0x8c56_8a24_1384_b60b, 0x7019_e432_1ef2_bf18),
        ],
        last: 0xb107_c590_3bde_bc07,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x899b_4cc1_3735_30fe, 0x9d23_92b0_d18f_ce83),
            (3000, 0x827a_152d_d8ea_d85b, 0x2b22_cdf2_cb13_5d9c),
            (4500, 0x921d_edc2_3caf_4e72, 0x760f_64c8_dd13_4ab9),
            (6000, 0x671e_572a_ce6f_825f, 0xea22_853b_7bea_6cd5),
            (7500, 0x6262_7538_b2c2_fe38, 0xe05b_ec6f_8575_867c),
            (9000, 0x809b_d64c_8fe3_f0d3, 0x8acd_b3b8_8237_d6c8),
            (10500, 0xb2b8_dad8_9b32_497c, 0x4d01_b89e_a41e_570c),
            (12000, 0xee5c_823f_b81f_cd5d, 0x3523_e4c1_d96c_1fb9),
        ],
        last: 0x591b_3978_9018_cdc1,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xdda1_c267_e5ec_a4de, 0x5395_ea84_d21e_40a0),
            (3000, 0x4b6f_6635_6e89_63f3, 0x6e1b_c34a_7c8d_0f4b),
            (4500, 0xa9f5_c12f_b2c8_9d55, 0x1586_7167_65ec_9e34),
            (6000, 0xcd3b_5f7c_7501_8f00, 0x555e_c6de_4cbe_b1bd),
            (7500, 0x253e_7e3c_3e45_fa17, 0x211e_e672_454c_f135),
            (9000, 0xdc9c_83e2_1c22_c8f8, 0x8756_d329_d27f_8ff3),
            (10500, 0x1e88_f331_65a5_a903, 0xaaad_7c7c_5968_b717),
        ],
        last: 0xb1d3_8365_ff90_1983,
    },
];
