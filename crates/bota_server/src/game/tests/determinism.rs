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
        (3000, 0x53bd_a561_f786_4cc0),
        (4500, 0x25f3_29e3_0dec_0ddd),
        (6000, 0x1d88_5fcd_50d8_4e5a),
    ],
    &[
        (1500, 0xff02_a1d5_82b3_9d6a),
        (3000, 0xe221_3a87_810e_3be0),
        (4500, 0x04e1_747e_2dd4_10a9),
        (6000, 0xb720_f5ed_3637_687f),
    ],
    &[
        (1500, 0xcf2e_2ddc_160f_de3e),
        (3000, 0x63f2_38bd_5e4f_0438),
        (4500, 0x4b20_b21b_127a_062e),
        (6000, 0x5643_317c_a012_25e0),
    ],
    &[
        (1500, 0xe1c3_da13_1607_414c),
        (3000, 0x48f4_e2bf_abd0_86b9),
        (4500, 0xd180_648d_86b5_b02f),
        (6000, 0x33e4_b29e_5210_e893),
    ],
    &[
        (1500, 0xbb90_78dd_b2a3_5cc9),
        (3000, 0xdace_d50a_b280_9049),
        (4500, 0xf4af_13c8_2137_d9c0),
        (6000, 0x610c_6866_efbf_652e),
    ],
    &[
        (1500, 0xfca7_36a7_fae9_71cc),
        (3000, 0xdcd9_d3ee_8f5f_324e),
        (4500, 0xc01f_8d5f_8110_eee2),
        (6000, 0xb7af_0a3c_e34e_6393),
    ],
    &[
        (1500, 0x3806_7800_eda2_d079),
        (3000, 0x5605_1a13_b291_c206),
        (4500, 0xd746_4287_9e20_80ca),
        (6000, 0x2386_e0a2_ee47_e137),
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
            (1500, 0x2c5d_361b_78b3_e64e, 0x0a80_d54b_5285_272c),
            (3000, 0xa415_6274_ea88_3e88, 0xa1a7_4b9b_4d15_5e48),
            (4500, 0xcce0_208f_52a6_224b, 0x3f65_39fc_320a_544b),
            (6000, 0xf129_8500_4e0a_9d57, 0x0706_5a67_1337_ecf3),
            (7500, 0xc005_434d_8baf_8dc5, 0x624b_b80d_d852_421f),
            (9000, 0x2128_22ac_f8b4_be06, 0x69fa_6ecd_8973_aa1d),
            (10500, 0xeb64_1121_c98c_545e, 0x6624_7a26_7328_14fe),
            (12000, 0xcf2f_63c8_54e8_3d93, 0xcc63_b9d9_33ae_6896),
            (13500, 0x8a51_8698_2798_fa05, 0xa8af_eb9d_7ff8_9d95),
            (15000, 0xca33_824f_5d4f_08ca, 0x622c_b982_93e6_2767),
            (16500, 0x7048_e277_77ac_682b, 0x1a04_dfa8_80cf_d01e),
            (18000, 0xa76f_055a_1b57_216c, 0x955b_5d35_864b_fd12),
        ],
        last: 0x13e1_ecfc_949c_0b71,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7b83_7436_e0fb_1d46, 0xb0d9_ea0a_cb36_a13c),
            (3000, 0x09e0_5202_20c3_d4d3, 0xbfd3_6d1c_615b_6a41),
            (4500, 0xa17c_f785_127b_7c84, 0xf71b_2030_e903_e6c6),
            (6000, 0xc038_4e2a_fe17_f011, 0x4a07_7125_63f9_8708),
            (7500, 0xa5ed_3c08_7aa4_5150, 0x612e_812b_8fcc_b9f4),
            (9000, 0xbfbf_ed32_af01_715f, 0x3ddc_fe83_6b99_8e46),
            (10500, 0xe910_0420_34af_6aa8, 0xf0c3_4f88_7618_5afa),
            (12000, 0x0ef5_f7e8_01bd_bcdc, 0xbd6a_c6cf_b798_84bc),
            (13500, 0x4bee_a291_0d23_d734, 0x91f7_91cb_b34e_699b),
            (15000, 0x90ee_8ace_ce8c_9e82, 0xc259_ed66_753c_c97f),
            (16500, 0xa051_64ca_0e8c_cf79, 0x394a_d8ee_dc01_d7b2),
            (18000, 0x2710_cb0b_0441_1483, 0x78e7_14b1_5fd9_35b9),
        ],
        last: 0xa641_2fa7_bd8d_a65b,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3f8d_124f_f58e_b040, 0x0a7a_897c_a5c2_67c3),
            (3000, 0x5e9e_53ed_af18_b4aa, 0xf958_5b72_7738_5869),
            (4500, 0x3a3b_bdb4_800b_bafd, 0x58be_211a_7630_0976),
            (6000, 0xc0fb_0219_9dd3_b4b4, 0x5de3_3572_8593_427d),
            (7500, 0x5403_7b58_3b13_d77f, 0xb6ad_2b46_9c2f_3c08),
            (9000, 0xbbbd_6c20_97ef_beeb, 0x37d2_d1ea_a7a4_ebc6),
            (10500, 0xf554_5927_ea79_815a, 0x3163_c1ef_b9ed_025b),
            (12000, 0xc3f7_0746_e8c3_a2a0, 0x85ad_3bf3_bdc3_144b),
            (13500, 0x5850_0a9d_dbcf_5f59, 0x17d2_ea92_add1_a8e4),
            (15000, 0x0ee9_8dcf_c75e_c187, 0xe9a6_348d_a5fa_c75e),
            (16500, 0xf146_4ba0_8399_99cd, 0x525d_21c9_06ed_d01e),
            (18000, 0x6813_8cea_4a8c_6afd, 0xbcb7_401e_d36c_1557),
        ],
        last: 0x32d0_6c8a_4c2c_7780,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x67d6_62cc_fa2f_f2ba, 0x3e3b_169c_2167_e938),
            (3000, 0xcfd9_679c_aa43_412d, 0x94b7_85d9_486b_53f7),
            (4500, 0x29d6_ac65_3bf8_aa31, 0x396b_812a_8126_ef11),
            (6000, 0x2ff7_90dc_b946_b4cc, 0x356d_ed80_9944_09f8),
            (7500, 0x6fe3_755c_9502_779d, 0x406a_6290_472f_44e0),
            (9000, 0xec9e_408d_96a1_679e, 0x7b43_385c_b71a_d395),
            (10500, 0x0abf_f9e0_342d_92b5, 0xe684_8656_ae89_5821),
            (12000, 0x8380_ceeb_2529_c492, 0xcdf1_fcb9_48ab_50ad),
            (13500, 0x5d4a_b583_9523_0233, 0x5dba_0514_c9eb_f030),
            (15000, 0x3a3c_cb4e_dc6c_3d68, 0x6dbd_1030_bfb3_7c8b),
            (16500, 0xf1d1_9b24_abe0_fd09, 0xab54_2afc_af21_6d0f),
            (18000, 0x32d1_8ef2_6c24_542e, 0x678b_6865_62b4_7e4a),
        ],
        last: 0x05ba_387d_be6d_123f,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x592b_4590_5ffe_4680, 0x4e19_7305_b579_06da),
            (3000, 0xc353_b959_2169_a229, 0x2597_22ab_51db_a0fa),
            (4500, 0x6d95_c6a7_6b6e_0252, 0x25cf_e02f_742e_fe3b),
            (6000, 0x3385_4f1f_e0ed_9ea7, 0x45b0_ca42_829e_32ab),
            (7500, 0x6a00_9418_2399_7a57, 0x54bd_0736_2c5b_1321),
            (9000, 0xb336_cf46_3575_6fdc, 0x1d8c_d1a3_685e_9150),
        ],
        last: 0xb889_0b43_2445_7fd1,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5fbb_2231_fd27_7eaa, 0xe1c2_b539_b045_aff1),
            (3000, 0x1afb_884e_a4ec_0c6b, 0xb5c8_c7f3_13f2_30b5),
            (4500, 0x5bc8_20f7_b181_07a8, 0x0d99_4a67_1bdf_a6b0),
            (6000, 0xebac_2a3a_09a3_471c, 0xf7c7_0d2b_c1bd_b4d4),
            (7500, 0x9bc3_bea4_4f73_f208, 0xfaa6_26df_e917_7294),
            (9000, 0xe1a8_65a5_b54c_8375, 0x24c7_3c94_8380_65de),
        ],
        last: 0x9fbf_328f_e88d_b045,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x1431_8d9c_3625_3052, 0x6d9c_1603_d562_cff2),
            (3000, 0xe2d1_d987_b484_7606, 0xde0b_cec4_d437_1020),
            (4500, 0x998b_d119_0cc6_6ca3, 0x1024_b331_29cb_4597),
            (6000, 0x9038_c3e4_09a3_fbcd, 0x4ece_b9ea_e8a1_8291),
            (7500, 0x9f8b_0a01_1d06_148f, 0x4b38_1f42_0477_183c),
            (9000, 0x3347_9904_a79e_8dc2, 0x8ee2_2923_4788_f915),
        ],
        last: 0x8b7f_22f9_9e31_f909,
    },
];
