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
        (1500, 0xcf7c_860e_9947_ff20),
        (3000, 0x42b9_3756_95ec_e150),
        (4500, 0x5203_1d3e_ff56_8a32),
        (6000, 0x2618_d1be_93b8_0b1e),
    ],
    &[
        (1500, 0x5b15_a4d7_ff15_7e5d),
        (3000, 0x522e_ae29_001f_7524),
        (4500, 0xf743_e227_7067_c645),
        (6000, 0xee65_9ccf_c651_c975),
    ],
    &[
        (1500, 0x00b7_44be_be65_620c),
        (3000, 0xda41_ccc8_163c_5ab4),
        (4500, 0x0ddd_0bed_8c4d_3a9d),
        (6000, 0x80a5_6fe2_6067_b80d),
    ],
    &[
        (1500, 0x5d40_64b7_c7e8_ea16),
        (3000, 0x3303_49ad_0a27_ff9a),
        (4500, 0xc46f_ade6_7f0f_a4a0),
        (6000, 0x3e7d_bd60_4f22_159f),
    ],
    &[
        (1500, 0x8eb4_2314_f5b9_f08b),
        (3000, 0x7d5f_4c09_29dd_e3be),
        (4500, 0xd8bb_f559_ad8f_139a),
        (6000, 0xa7a5_f528_70ee_89fd),
    ],
    &[
        (1500, 0x5543_9ecf_e0ab_c46b),
        (3000, 0xd285_f2ea_7514_709e),
        (4500, 0xd6c8_ebc5_44e6_ff71),
        (6000, 0x9758_b336_9bcd_d450),
    ],
    &[
        (1500, 0x1b79_0cd3_f28e_3f8d),
        (3000, 0x2aa8_57df_a99a_5374),
        (4500, 0xd68a_7e34_e04e_cfcb),
        (6000, 0xab23_e098_d4fe_e195),
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
            (1500, 0xf2a5_1b8b_3b9a_9bc0, 0xa39f_583a_3f1d_f592),
            (3000, 0xbd71_cc5f_66a5_0364, 0x193d_5187_9a32_b137),
            (4500, 0x45ef_fc92_a444_3553, 0x1cf0_b0e6_c3f1_86eb),
            (6000, 0xd233_177d_cd2a_45e4, 0xeeae_13d1_9a23_54f7),
            (7500, 0x788d_7ff2_210e_6296, 0xeec1_be0c_7c91_7405),
            (9000, 0xf3f9_7657_1dc7_1414, 0x95a3_a271_214e_5fe5),
            (10500, 0xc293_b526_a2fc_d3a8, 0x6b11_fb20_ef95_de23),
            (12000, 0x25a6_7cf7_ebc6_bed1, 0x8202_48e9_fa6e_1219),
            (13500, 0x890c_d4ae_d2c3_4a01, 0xd6a5_8c7b_d582_1b70),
            (15000, 0x93f9_91e1_5021_ba80, 0xa3b3_d5a8_aef1_08ee),
            (16500, 0xca48_1bef_01bc_44a2, 0xb165_c0b5_8c6b_a75d),
            (18000, 0xb6aa_2806_af08_58b4, 0x15b4_a62f_1650_e41e),
        ],
        last: 0x482b_78e6_524e_3abe,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x7e06_15ea_ec9c_8363, 0xa045_7c0f_dffe_ee05),
            (3000, 0xceac_4520_0c2d_ee4b, 0xf603_bbf8_796c_14b2),
            (4500, 0xfeb6_5a5b_1845_4003, 0xc8e5_9cba_dd4d_2449),
            (6000, 0x0934_8dab_8b5f_c627, 0x98f9_203c_5566_3fe4),
            (7500, 0xd95e_a4ea_028a_4551, 0x1b37_1438_5c7c_1bd8),
            (9000, 0x863e_8bc5_4686_e2f7, 0x222b_5058_24f3_4395),
            (10500, 0x4de0_2b92_dd57_51f2, 0x388b_15c1_b314_a871),
            (12000, 0xd6e1_48d9_6b44_810f, 0x4ca7_bb6e_23b5_2838),
            (13500, 0x72bb_059c_1dec_98e4, 0xc004_72b2_156f_ffef),
            (15000, 0xed0b_733e_acd3_2b50, 0x4532_037f_c5ca_7038),
            (16500, 0x9f17_c6b3_df24_d6f6, 0x8a67_9000_da8b_ceac),
            (18000, 0x18d4_6c82_5027_a314, 0x2361_caa1_78a5_b2a4),
        ],
        last: 0x7c4f_9093_c3df_f7aa,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x1097_e3e6_9ccd_b95a, 0xb3c6_9498_ddd1_5397),
            (3000, 0xddc1_eadb_7d96_d218, 0xbb77_8a50_91bf_b724),
            (4500, 0xdbbe_979f_b145_9901, 0x8dae_549d_0a4d_324c),
            (6000, 0x01a8_4527_c7e1_69a3, 0xda78_0c31_a43e_ea99),
            (7500, 0x3e35_781c_6afe_1de6, 0x950d_c464_1fb7_f069),
            (9000, 0x4b7a_6e20_3352_2d25, 0x87ef_94ad_467a_58e0),
            (10500, 0xa1ed_04d6_56b9_9ae9, 0x581f_d3f6_f17a_556e),
            (12000, 0xe44f_02a1_5800_78da, 0xe9e8_c906_b172_03f0),
            (13500, 0xdab7_0383_77c8_dd36, 0x4987_10bd_7985_b3f2),
            (15000, 0xf515_7111_e620_3f35, 0x00bd_1925_ab26_775a),
            (16500, 0x5e87_d1b8_b21a_90e6, 0x7652_81cf_d353_94da),
            (18000, 0xa105_b17f_3c88_d735, 0x2cf7_49fe_5db6_a7a4),
        ],
        last: 0xe50f_822c_b965_9412,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xed43_1534_7ac5_ee23, 0xa293_164a_f260_1d45),
            (3000, 0xcf0d_4b66_86ea_1b47, 0xc31b_717a_af58_5c45),
            (4500, 0x3d5f_ddaa_3225_ab0f, 0x138e_99b1_f0e6_6868),
            (6000, 0x6c2d_a0e3_fd84_d818, 0x14bd_6f86_3c9e_9b86),
            (7500, 0x881d_b00a_4c3d_bf03, 0x8cb4_b364_68a1_1fd1),
            (9000, 0x1a6b_fd91_79c2_fa61, 0x85a8_7076_2583_fe44),
            (10500, 0xc630_d035_129d_04ac, 0x38c6_3b1e_6671_a20c),
            (12000, 0x782c_7e0d_60ef_eb7d, 0xaed2_522e_d577_ea85),
            (13500, 0x7622_ef21_6958_37b6, 0x33c0_885f_c499_385f),
            (15000, 0x7283_1900_8c4e_7857, 0x59c0_653a_f09d_0eea),
            (16500, 0x8c6e_e8e1_6121_c0a4, 0x9750_dc32_be16_7503),
            (18000, 0xb1c1_5bf9_654d_0e01, 0x4fcc_e920_a092_0c43),
        ],
        last: 0xebdf_b5be_bdbe_8c12,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb993_0edb_3b9c_8114, 0xf8f9_380e_2fc2_e3fb),
            (3000, 0x53d3_c6c8_09fc_8418, 0x8e7e_b77a_8b0b_e09c),
            (4500, 0xacb5_4e98_5a82_16d9, 0x9b08_c483_2e26_b82e),
            (6000, 0xe4fd_b704_3343_27cb, 0xd226_9f45_8282_87e4),
            (7500, 0xc92b_6a66_8acf_35e8, 0xf70a_afe7_234d_afd8),
            (9000, 0xcb43_efe0_0e3e_b08c, 0x6e92_0a6d_116d_f508),
            (10500, 0x3274_50da_5e5f_24db, 0x2615_6dda_d974_7719),
        ],
        last: 0x1c5e_f5b6_eb4e_4370,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5687_59f2_6ab6_d318, 0x9d23_92b0_d18f_ce83),
            (3000, 0xe686_c14a_47a5_2ee0, 0x2b22_cdf2_cb13_5d9c),
            (4500, 0x47b7_70f1_6300_7c25, 0x45c4_a38b_1e02_58f9),
            (6000, 0xc495_3937_bac1_04b5, 0x63a8_0d3b_5eac_353a),
            (7500, 0x381d_81dd_f73f_7280, 0xcd03_e639_f98c_d4d8),
        ],
        last: 0x86de_1832_07df_f053,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x057a_bfa4_c906_fcac, 0x5395_ea84_d21e_40a0),
            (3000, 0xcdea_28dd_7946_bb71, 0x6e1b_c34a_7c8d_0f4b),
            (4500, 0x6fbe_03f3_18af_b9cc, 0xf65f_952b_2ebe_642e),
            (6000, 0x75e6_e290_dcba_e141, 0xf59d_113f_c29c_e3a0),
            (7500, 0x1d36_71ac_80db_6e2d, 0xe3e0_0f37_0c15_c438),
            (9000, 0x5183_e591_37d5_40a0, 0x88f3_70ee_0e8e_334b),
        ],
        last: 0x29cd_2af9_645c_a699,
    },
];
