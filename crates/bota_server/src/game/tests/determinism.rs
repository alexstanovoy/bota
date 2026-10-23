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
            (1500, 0x5392_3800_5a71_e17b, 0xf944_2286_34c3_ba3f),
            (3000, 0x1827_1d5e_2482_46a5, 0x2c5c_6149_c808_bf11),
            (4500, 0xeba7_e4f5_6833_aef0, 0x69b0_93a8_88cd_80ff),
            (6000, 0x6727_9e52_eeef_6112, 0xe422_55b0_1bca_b4f5),
            (7500, 0x3b80_bedc_355b_bea1, 0xb7ae_0784_1041_649c),
            (9000, 0xd472_07ab_9f10_c582, 0x1e1d_8fbe_f771_c7fe),
            (10500, 0x6612_357a_4503_d62e, 0x1372_704c_a96e_2582),
            (12000, 0x31be_b02e_e871_a607, 0xc58c_6b35_3c56_de52),
            (13500, 0x87ad_2a88_34c0_85d6, 0xa438_a246_2ac2_80f3),
            (15000, 0x1473_b5ef_a4fe_0a85, 0x67b6_0a5e_2e7b_6111),
            (16500, 0xa702_a658_115f_981a, 0x86e3_64f0_d07e_4843),
            (18000, 0x0b65_2858_29a9_3b1e, 0x49b8_36dd_6ba1_726f),
        ],
        last: 0xa083_c595_d732_9d5a,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xd867_9dfa_ffef_de40, 0xcf4e_4ec8_3678_d5cc),
            (3000, 0x535e_df36_bb87_162c, 0x67da_3cf2_e095_688b),
            (4500, 0x1a5d_48bf_6ca6_0aa7, 0x38cc_87dd_0a83_850e),
            (6000, 0x7f65_3352_43d7_aead, 0x6b0e_e65f_30de_21c4),
            (7500, 0xbd2b_cd2a_8438_d29b, 0x5e7e_f1b7_1854_5328),
            (9000, 0x5685_6fb3_ad66_e768, 0x2322_d436_a7b8_f532),
            (10500, 0xed8d_a151_2824_b603, 0xae34_94b7_7b07_949b),
            (12000, 0x22e6_6426_a4f0_3770, 0xc105_159b_4c58_652c),
            (13500, 0x7e53_d5b7_bf4f_70fc, 0x57b2_2b28_ae43_1f57),
            (15000, 0x4977_929c_38b5_22e4, 0xe06a_5ca9_7ddd_f9e4),
            (16500, 0xb122_a8e7_deda_2d9c, 0x3eff_3679_fda1_18ea),
            (18000, 0x1949_ed2a_23f0_6e7f, 0x4612_56c5_c080_783e),
        ],
        last: 0xdf82_b427_1cd1_75e9,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3e72_ba7c_e8f7_0daf, 0xba5b_0196_e371_b887),
            (3000, 0x55bb_cbb1_21db_ad80, 0x8d15_b063_9096_89dd),
            (4500, 0x7e5d_5c07_7d0e_a9d5, 0xec27_062a_426f_9ce5),
            (6000, 0xfc43_391b_8ae7_d51b, 0xed8b_17a2_6f32_3514),
            (7500, 0xc4d7_58ed_8b8c_3fe6, 0x3e02_af38_82f1_a856),
            (9000, 0x1355_5b9a_c71b_8829, 0x8948_09b8_f42e_b36a),
            (10500, 0x7904_7a77_6ca3_5e2d, 0x3cc9_502c_8c82_6f0c),
            (12000, 0x4486_88fd_c666_1ef3, 0x231e_6383_5e5b_7f6a),
            (13500, 0x2052_c379_c463_9a69, 0x6103_f475_ed33_18cb),
            (15000, 0x64f5_b4e7_e931_04a9, 0x46ae_417c_86f2_cb19),
            (16500, 0x765e_5bdb_cdd4_88a3, 0x5aa5_735a_0cf7_a848),
            (18000, 0xed5c_c3db_5653_fbef, 0x69c2_8694_ac45_d356),
        ],
        last: 0x8f25_2a68_ea80_988b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x44b7_155f_cbba_c7d5, 0x01e2_3234_aacc_15bd),
            (3000, 0x6245_b7bc_d5cd_6e07, 0xdb73_41b2_fa5a_60df),
            (4500, 0xdfc3_0688_3386_a277, 0x377c_5ffe_1b37_e083),
            (6000, 0x62c4_3a06_e673_7fcb, 0x4559_2c9c_e57c_ec79),
            (7500, 0x6f29_fe7f_78c6_4c15, 0xf820_128a_ef81_087b),
            (9000, 0xabd6_370b_1e97_d3d7, 0x9484_8d1b_2523_314e),
            (10500, 0xf09d_be3a_8855_d1ec, 0xfeaa_3aae_22fc_1f8d),
            (12000, 0x44f4_9feb_cb94_dbe6, 0x37ad_501b_5da6_1554),
            (13500, 0xfc25_9ad2_bfd5_3636, 0x5420_29d1_f079_a1ce),
            (15000, 0xb7f2_d5d8_1376_81ff, 0x7ece_17f2_09a0_caec),
            (16500, 0x5ed9_2005_4ee0_4b23, 0x2032_3888_d5c6_fc54),
            (18000, 0x5851_72b7_34d9_c7c2, 0x2124_c4d7_16bb_1f59),
        ],
        last: 0x730c_0e87_e0b4_57ac,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7047_2c96_1f81_5434, 0xe4f0_4ea2_c560_3c57),
            (3000, 0x78ea_2c40_f9ee_8a65, 0x291a_8231_3ba7_b65f),
            (4500, 0x3dc8_3b10_f9ff_752a, 0x37b7_6928_a275_2603),
            (6000, 0x31cd_9e24_7bc9_f53f, 0x4cfc_f9f6_5419_8c1b),
            (7500, 0xfb73_ea86_9b6b_aa49, 0x1d6e_a67f_04e2_0a25),
            (9000, 0x7f3b_bb3e_7226_5f85, 0x200a_e205_33c4_3ed1),
        ],
        last: 0x75f6_418d_10c0_5af0,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x9acd_c0b3_cafb_b548, 0x78fd_7a79_e8e8_91c2),
            (3000, 0x3ec2_7751_b93f_18d1, 0x81f2_2ada_2eb6_187f),
            (4500, 0x0d5a_a544_1546_2443, 0x64fc_78fe_2fdd_9e8a),
            (6000, 0x40f9_10dc_022d_3e2f, 0xb44e_f1fd_8410_23c9),
            (7500, 0x1a58_69b2_9ab4_ef8d, 0x850d_ec04_cabe_e04f),
        ],
        last: 0x6139_5f17_871a_2ef6,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xae29_1fcd_f513_28dd, 0x91f2_c77e_9c7f_33e2),
            (3000, 0xa7e8_985e_f894_e0f3, 0xf177_b29c_accd_0ad1),
            (4500, 0x76d1_67a1_3cc5_fb18, 0xd01b_1005_3929_852b),
            (6000, 0x45ec_875d_3953_3157, 0xcbc6_0747_9725_fff1),
            (7500, 0x52b2_9315_886c_7ac9, 0x7e53_1dd1_021d_a8c3),
            (9000, 0x7168_b96c_70c2_9477, 0xe345_fbaa_42f2_2641),
        ],
        last: 0xbd46_a202_07bd_c537,
    },
];
