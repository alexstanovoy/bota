//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_core::{ITEMS, map_of, rules};
use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, EventVisibility, Fnv, MatchConfig, MatchSecrets, World};

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

/// The orders every seat gives this tick, validated as a driver does, with
/// each refusal folded into the digest.
fn decide(world: &World, drivers: &mut [Driver], digest: &mut Fnv) -> Vec<Command> {
    let mut commands = Vec::new();
    for (seat, driver) in world.seats.iter().zip(drivers.iter_mut()) {
        let view = world.view(seat.team);
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
            decide(&world, &mut drivers, &mut digest)
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
                    events: events
                        .iter()
                        .filter(|event| match event.visible_to {
                            EventVisibility::Everyone => true,
                            EventVisibility::OneTeam(side) => side == team,
                        })
                        .map(|event| event.kind.clone())
                        .collect(),
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
            decide(&world, &mut drivers, &mut digest)
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

/// Every pinned match, recorded from the simulation as it stood when the
/// contract was laid.
const PINS: [Pinned; 7] = [
    Pinned {
        map: MapId(0),
        seed: 1,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xf2f3_2b23_2ebe_f903, 0x0cb7_7ca1_c217_b682),
            (3000, 0xc53d_bec7_2f9e_94ee, 0x9571_287f_48be_6c2d),
            (4500, 0x1fe1_9722_2896_f1fe, 0x7f22_3bb4_d4e5_9e07),
            (6000, 0xf511_3789_76e1_8307, 0x1bec_cf42_f99f_5b3d),
            (7500, 0xd61e_8a4c_8fea_3d8e, 0x4161_9205_bc40_4d3c),
            (9000, 0x6672_d074_71bd_b5be, 0xd213_bb16_39c8_26cf),
            (10500, 0xa60f_3ec7_5557_7b5a, 0x7519_36da_8255_e060),
            (12000, 0x2a2c_6cc4_abb3_c85c, 0x0f5d_2671_4d24_34a1),
            (13500, 0x39c0_e187_3ae6_d254, 0x18f0_da33_525c_ce0a),
            (15000, 0xb90d_b6fc_7b21_7a31, 0x6890_63af_62a8_0487),
            (16500, 0xab45_5492_d704_f1fb, 0x58e4_12f0_dbf6_b89e),
            (18000, 0x235a_833a_e86c_00a6, 0x28e3_3891_6dd1_365a),
        ],
        last: 0x68d1_2968_ecac_0ba0,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x1054_ef14_0f9d_2a68, 0xa9e9_d01f_0fa8_0a9a),
            (3000, 0x85e7_ce6c_23f2_e525, 0x1eda_64eb_bfe8_0900),
            (4500, 0xe7bc_3c4f_260b_1de5, 0xaab5_555e_78e0_3e87),
            (6000, 0x2d6d_1123_2ec1_e101, 0x81ac_a89b_131b_840a),
            (7500, 0xe2b6_bf00_b7f7_8fde, 0x3619_c6d1_646d_6ca9),
            (9000, 0x7b47_ef0e_358b_79b7, 0x29e7_bb66_350d_bc16),
            (10500, 0xb89f_f19a_1208_9930, 0x7d04_14c2_fac6_0aff),
            (12000, 0xdbb7_885d_2307_cd21, 0x0252_3ccb_a95b_e43d),
            (13500, 0x390f_bca6_2627_4800, 0x747d_6909_ae56_faec),
            (15000, 0x1a18_1729_33fb_c943, 0x1543_d419_c7e3_d6f3),
            (16500, 0x9971_b618_c0c8_8092, 0x47db_dd22_d024_2852),
            (18000, 0xff7a_9fa4_b60f_3a5e, 0xa7fd_5179_2f85_ff1e),
        ],
        last: 0xba47_fa85_f73f_7a80,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x7834_c9c5_b095_f43d, 0x5d13_45c7_8e10_23f4),
            (3000, 0x096f_3e90_4770_8a2d, 0x524f_91d5_03b1_8081),
            (4500, 0xbc6d_9924_107c_445c, 0xe6cb_3d64_1846_aa1b),
            (6000, 0x07e7_a7dc_88d5_bc1d, 0xcdd6_2170_7c8a_21fc),
            (7500, 0xfa0f_0370_28c9_4555, 0x6bdb_dc95_188f_2f51),
            (9000, 0xca04_0eb9_bfc6_ffd0, 0xded0_e475_5f5c_fd92),
            (10500, 0xebe9_8d0f_d2fb_0104, 0x88ec_4419_fba8_5760),
            (12000, 0x4c19_6f68_967e_754c, 0xe680_96aa_63ce_a45f),
            (13500, 0x7e55_4dce_ca26_5b7b, 0x3caa_f925_4ce4_6240),
            (15000, 0x137e_3551_71a2_fb4d, 0xb1fe_c2ca_c029_bc52),
            (16500, 0x5cb6_d62b_d4cb_7628, 0x8632_2b4d_66c5_b2b2),
            (18000, 0x3e2d_fbe9_b648_8cba, 0x4679_8d6e_161e_07c1),
        ],
        last: 0x6b55_6c41_57a0_08ca,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x66ea_1afe_7013_0acf, 0xa156_db63_c128_8ac7),
            (3000, 0x13f7_bd50_843f_03da, 0x1884_58ae_5787_bd08),
            (4500, 0x2885_8259_4d21_6fb7, 0x5a76_9094_7716_0a4f),
            (6000, 0x25f8_863a_855a_8f1d, 0xb69a_edcf_2afd_993a),
            (7500, 0xeb6e_14c2_5040_8609, 0xc46e_0100_9f67_34a3),
            (9000, 0x9b86_b761_7216_5c03, 0xa984_ca6a_69f2_befe),
            (10500, 0x2ece_6253_9f25_b676, 0xc8cd_148d_c81c_4c70),
            (12000, 0x212b_50d8_1247_844e, 0x70f2_1e3c_088c_9766),
            (13500, 0x3aac_e1f0_eeab_2ccf, 0x9bbf_4cf9_6196_2ab3),
            (15000, 0x8f24_d579_dc77_aea6, 0xad5f_1a52_60b0_79b9),
            (16500, 0x21aa_bf8e_442b_ae37, 0xae55_7689_539f_1860),
            (18000, 0x64a8_e017_9553_8598, 0x34f5_8592_012e_a66d),
        ],
        last: 0x50e3_4f4e_7d7d_a944,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7ae1_d907_ffc0_47b4, 0x1672_1792_1116_8379),
            (3000, 0x4d57_d743_a282_956d, 0x3c52_82cf_fd2d_0fb2),
            (4500, 0xbb5f_c4fb_87fc_8cfb, 0x6560_7fe1_3217_1b07),
            (6000, 0x7e87_aada_81f9_ef71, 0x3f83_abca_9fff_a33e),
            (7500, 0x8486_72ca_32d6_cba8, 0x6c01_958c_f896_cd9f),
            (9000, 0xb869_bd5a_90a7_ecd2, 0x0924_1471_c333_5e55),
            (10500, 0x68bd_2bce_ba98_cd26, 0x03e6_4686_56d1_21a7),
        ],
        last: 0xf132_a4a6_312c_0d6f,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x47f6_2b9e_51fe_d485, 0xe5da_04e3_2f8a_b9e2),
            (3000, 0xe857_ac15_830e_7d04, 0x0327_15a7_9a22_a222),
            (4500, 0x6d1b_52e2_61b8_c83c, 0xb15d_4b15_7f4f_dc25),
            (6000, 0xd365_e83e_3dd5_8391, 0xdf73_b97e_4c31_94e4),
            (7500, 0xe5ea_43ba_8efc_d846, 0x0f02_2e69_26ae_f798),
        ],
        last: 0xa857_4ec5_ba40_ee44,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3270_5e7a_013e_d43f, 0x4d31_efcc_157d_81b7),
            (3000, 0x5973_3339_9163_dd50, 0x69cc_c7d8_65e3_e236),
            (4500, 0xf5ec_c042_c9da_0187, 0x7f6e_054d_9a03_c0cc),
            (6000, 0x2c97_7537_4252_b6ba, 0x3f37_a939_e8c2_d4a9),
            (7500, 0x8789_9a82_108c_4fe6, 0x108d_8292_e3de_3939),
            (9000, 0x37e5_31ac_0b5d_444f, 0x92e9_d17b_8fb0_a15e),
        ],
        last: 0x2710_4bbf_16b8_f370,
    },
];
