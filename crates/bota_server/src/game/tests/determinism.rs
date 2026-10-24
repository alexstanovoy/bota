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
        (1500, 0x51ef_afbc_ceee_5134),
        (3000, 0xabb7_8bfe_bd07_5dc8),
        (4500, 0x8bfe_afec_b9a8_dd88),
        (6000, 0x675e_a744_c5cf_06bc),
    ],
    &[
        (1500, 0xd032_664d_0b7b_e3a9),
        (3000, 0xf03c_34ae_76a4_1da4),
        (4500, 0x7e2f_22ab_345c_6341),
        (6000, 0xf625_2a15_322a_c00c),
    ],
    &[
        (1500, 0xf8ac_86ba_1adf_a47c),
        (3000, 0xdd1c_49ab_1031_529a),
        (4500, 0x830e_8e0f_885a_b200),
        (6000, 0x5828_5477_f82a_eaa0),
    ],
    &[
        (1500, 0xbd79_468f_7049_a91e),
        (3000, 0x1c7e_6851_e1e2_b775),
        (4500, 0x4476_50de_8b5d_ca0e),
        (6000, 0x57bd_5c6e_6138_6ea9),
    ],
    &[
        (1500, 0xf7f8_aa3d_dd2f_b8af),
        (3000, 0x0982_eaa7_2fb8_75ac),
        (4500, 0x0016_6bc1_ed0e_fb17),
        (6000, 0x391d_ed3b_b793_4458),
    ],
    &[
        (1500, 0x0bc8_7315_11a9_71a7),
        (3000, 0xae4a_6036_8206_2fe6),
        (4500, 0x494a_cb22_b711_40f7),
        (6000, 0x3546_6054_8305_7753),
    ],
    &[
        (1500, 0x7b46_8c65_d4d2_5e9f),
        (3000, 0xfb05_aab7_c95f_e124),
        (4500, 0xfc69_fbdd_0e9d_d87e),
        (6000, 0x0147_947c_f9e3_cae7),
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
            (1500, 0x85fe_35a6_bc89_311c, 0xa39f_583a_3f1d_f592),
            (3000, 0xfa58_70fd_e6f5_07da, 0x193d_5187_9a32_b137),
            (4500, 0x2a54_1796_a96d_44d5, 0x472a_9939_beca_5874),
            (6000, 0x2c1d_7ad6_47cc_da12, 0x93eb_ed71_8b60_620a),
            (7500, 0x654b_5cee_a611_1579, 0x1689_9240_ba29_6a11),
            (9000, 0x543f_366e_3fe3_113d, 0x1b65_1630_e7e8_2ed9),
            (10500, 0x8c0d_88f9_d15c_9f21, 0x24e7_1f36_e060_b62d),
            (12000, 0x9462_05a5_0dff_232c, 0x85b3_109d_6aaf_31d1),
            (13500, 0x5f16_1e5f_a938_ba13, 0xe21d_b657_0cfe_f96d),
            (15000, 0xb327_23d8_b902_6ee7, 0x2fef_7477_8193_2605),
            (16500, 0x4497_bac6_b35c_2bcd, 0x72ed_103e_0d03_2aa4),
            (18000, 0x97bb_3568_32ae_a034, 0xf771_9d87_e836_fcf9),
        ],
        last: 0xb773_38a1_67ca_93bf,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xad3c_5855_0c8b_4435, 0xa045_7c0f_dffe_ee05),
            (3000, 0x633c_66ee_adce_3e7b, 0xf603_bbf8_796c_14b2),
            (4500, 0xd857_f8f3_0eae_36c7, 0xc8e5_9cba_dd4d_2449),
            (6000, 0xbb20_2000_9d43_e98d, 0x34a7_40c4_1a49_66f5),
            (7500, 0xec59_40e1_c79f_8231, 0x429e_e4a9_80f6_e4a9),
            (9000, 0x7094_67de_ff88_88bd, 0xc38b_1741_f662_d228),
            (10500, 0xe230_87bc_c883_7446, 0x6132_e842_666b_5344),
            (12000, 0xe610_6c5e_1c5d_e13a, 0x85e9_9b6f_5615_2d48),
            (13500, 0x1c94_61ad_43db_37dd, 0x9c06_366d_7f77_dc2d),
            (15000, 0x3464_afdd_4861_269e, 0xb935_3865_764b_125f),
            (16500, 0x9faa_ac5c_51b0_cab5, 0x9c22_d153_e41c_2ee1),
            (18000, 0x4c1f_be32_4a2a_1102, 0xdb2e_0a95_ba3b_a933),
        ],
        last: 0x7541_f3e2_63b7_bdac,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x5c08_9c46_afd9_4264, 0xb3c6_9498_ddd1_5397),
            (3000, 0xbe10_4db3_604a_bb36, 0xf94e_0261_cf7c_e440),
            (4500, 0x2144_07fa_5eef_1b3c, 0x8e3c_52f0_c632_a8b5),
            (6000, 0xb581_d292_5276_b45a, 0x436e_9acf_1563_ba7e),
            (7500, 0x2783_78f1_4966_7049, 0x3471_0e97_7b86_9233),
            (9000, 0xe0f2_ae56_782a_ccca, 0xf399_47e1_6288_2df2),
            (10500, 0xec86_87c3_bfa2_8264, 0x66fd_0d48_2edc_7fac),
            (12000, 0x9f0e_4833_9e57_32b6, 0x8eeb_f80e_abcc_8130),
            (13500, 0x7739_a00c_a5a9_764a, 0x6123_8b13_953f_2c75),
            (15000, 0xf320_ff17_551f_691d, 0x1d05_e7a7_f5e6_0c4e),
            (16500, 0x7dec_9425_5add_a389, 0x2760_95b6_8899_bb1d),
            (18000, 0x893a_e114_7380_6f4b, 0xbda8_3fa0_d211_4689),
        ],
        last: 0xfc67_6b96_6069_a2ce,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x06c2_a657_c67e_ce76, 0xce15_1995_d45a_d7b6),
            (3000, 0xdba3_d962_3589_c362, 0x5a0b_23c9_7e89_13ac),
            (4500, 0xd52c_36a4_9984_6298, 0x29a5_3a11_bc44_6772),
            (6000, 0x83b0_c564_cc08_2160, 0x57b5_f7ef_6966_8c2a),
            (7500, 0xb900_39ec_1ded_963a, 0xa641_4e50_69f8_77ea),
            (9000, 0x8ebd_132c_cca1_1235, 0xa362_b05f_f5ef_1daf),
            (10500, 0x0d44_cf88_82e6_71c8, 0xc571_b00d_2190_88c0),
            (12000, 0xb611_9fdd_c78f_ad66, 0xcc9f_4baa_0ce5_9c8e),
            (13500, 0x5ae2_4c87_4a0c_b282, 0x9f19_7e6b_eada_5352),
            (15000, 0x720c_805e_2fd9_d5fb, 0x2f52_1c03_5b01_55df),
            (16500, 0x04a9_0097_b3ee_fc1c, 0x8d66_5118_d913_75ec),
            (18000, 0x06fc_ad7e_7dd9_4842, 0x602a_fcfa_c7d3_b66a),
        ],
        last: 0xbd51_ab66_94f6_8528,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x8357_ea07_4eda_b520, 0xf8f9_380e_2fc2_e3fb),
            (3000, 0x2147_bff3_7f5e_a570, 0x8e7e_b77a_8b0b_e09c),
            (4500, 0xe735_bedb_a3ff_b4a3, 0x9b08_c483_2e26_b82e),
            (6000, 0xf865_4b32_adbb_bd9a, 0xddb3_5067_805e_287b),
            (7500, 0x0572_6dda_4508_0ea1, 0xb192_d752_d681_e4df),
            (9000, 0x6910_5764_6993_832d, 0xde21_f4ac_a6d8_fac5),
            (10500, 0xf992_17fc_3547_dfd3, 0xe3ad_4343_15c8_f72b),
            (12000, 0xa31c_fbd6_fe1f_e11c, 0x8b61_6dee_ff58_c5fd),
        ],
        last: 0xf9aa_2a26_bc90_c952,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x584b_e695_79db_0e60, 0x9d23_92b0_d18f_ce83),
            (3000, 0x84fa_267d_97fd_5792, 0x2b22_cdf2_cb13_5d9c),
            (4500, 0x9303_001e_bcf4_b167, 0x4dad_9954_0444_b502),
            (6000, 0xc7f9_2614_de29_b215, 0x8a69_167b_f7c9_bce5),
            (7500, 0x2e0a_e0fa_3c49_0144, 0x6def_3615_2f9e_a130),
            (9000, 0x9f95_dfb0_f7d8_f160, 0x2798_25bb_b432_5f7d),
        ],
        last: 0x27d4_6a55_b6dd_e1a6,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x04d3_8e42_b4ce_1eae, 0x5395_ea84_d21e_40a0),
            (3000, 0xcd80_7757_5ee6_0f71, 0x6e1b_c34a_7c8d_0f4b),
            (4500, 0x73e5_60f4_3e42_8aac, 0xf65f_952b_2ebe_642e),
            (6000, 0xc970_46ae_3332_44dc, 0xa227_85ff_bee9_702d),
            (7500, 0xde8b_b337_34ae_379f, 0xe65b_c2cd_479a_b6a0),
            (9000, 0xccde_8e8c_c238_ed54, 0x93dd_1d2b_84da_b52a),
        ],
        last: 0x1e1c_a5c7_2c0c_60ee,
    },
];
