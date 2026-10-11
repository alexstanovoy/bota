//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, TalentSlot,
    Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Command, EventVisibility, Fnv, ITEMS, MatchConfig, World, map_of, rules};

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
            10 if self.below(2) == 0 => Order::Learn {
                slot: AbilitySlot(self.below(me.abilities.len() as u64) as u8),
            },
            10 => Order::LearnTalent {
                talent: TalentSlot(self.below(u64::from(TalentSlot::COUNT)) as u8),
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

/// The orders every seat gives this tick, validated as a trainer does, with
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
    if map == crate::game::MAP2_ID {
        crate::game::MAP2_TICK_CAP
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
            (1500, 0xb298_2894_e137_143b, 0x175d_c470_fcfc_36ed),
            (3000, 0x2299_f9b9_b7f1_ed59, 0x2071_7417_c656_98ba),
            (4500, 0x15ca_87e9_83e1_a1bb, 0x8265_515c_1261_0167),
            (6000, 0x9421_2b1b_efa2_9dc7, 0xc6ae_f9f5_fa5c_f31d),
            (7500, 0x3064_9713_67af_b248, 0xa415_0767_edde_4e6f),
            (9000, 0xef92_48c6_80ed_8b70, 0x5e01_1992_7935_b691),
            (10500, 0x9aa2_1f9a_278d_da17, 0xd667_6401_3ca4_b49f),
            (12000, 0xa95e_8362_4ceb_665c, 0x4d87_26c9_5168_56a1),
            (13500, 0x65b8_cfdb_7a88_1e1f, 0xd9b6_8167_aa11_eefa),
            (15000, 0x0434_1b33_4858_9ef0, 0x6976_2c7c_4cf9_fffe),
            (16500, 0x00e7_bdbc_417e_f09c, 0x3a42_9722_f649_f785),
            (18000, 0xdc1a_a52b_f513_f8be, 0x8066_8524_7162_82ab),
        ],
        last: 0x1ca1_73e9_c36d_c2df,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xf1ac_3c0e_5fb7_c028, 0x011d_14b2_012f_4d2f),
            (3000, 0xd34c_70d7_f2be_2394, 0x13f8_69cd_d8ca_d3fa),
            (4500, 0xc7c3_792a_ee1d_f687, 0x4aff_71fc_a15c_fe67),
            (6000, 0x5046_5395_5708_dc30, 0xc6c6_3806_cbe6_b8fd),
            (7500, 0x8dec_f00b_64e8_c41e, 0x9552_8fb5_0227_36eb),
            (9000, 0x7799_5e2e_939e_8812, 0x535f_a756_83f1_83d7),
            (10500, 0x31b8_d1a4_a0ab_8072, 0x3207_1333_1f12_a2e7),
            (12000, 0x22e1_4aac_aa1c_8df4, 0x7463_f440_406a_3ad2),
            (13500, 0xe7a4_cd3e_fb41_9f52, 0xfbab_5e8d_bc84_3cd1),
            (15000, 0xb555_5ac3_e000_286b, 0xa7d2_5453_d3f4_7ec5),
            (16500, 0x7847_d00c_7b18_9539, 0x189f_a13a_3527_e000),
            (18000, 0xb74e_d969_136d_f775, 0x57c6_4cec_1286_61f2),
        ],
        last: 0xdccb_9213_edea_dbaf,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xc020_9bc3_484c_c843, 0x90f0_ccbe_a0e9_1c73),
            (3000, 0x0b22_828c_3515_945e, 0x6cfb_3e21_689e_61a8),
            (4500, 0x0d42_deef_9fae_d8cc, 0x9b9e_7efe_c7c2_2f8a),
            (6000, 0xb507_11de_7bb6_b7f7, 0x5ac7_7592_e97f_92db),
            (7500, 0xb14e_9d72_fbbf_7e75, 0x3102_1d73_aa2e_4c3c),
            (9000, 0xc3c6_7a9e_2458_4a38, 0x37d2_b041_567b_6c1a),
            (10500, 0x7e92_b787_dc3a_dc3a, 0xe096_d23e_c443_ead7),
            (12000, 0xf64f_2d94_6a36_44f3, 0x7ca3_1bef_b5a9_27f2),
            (13500, 0xeacb_148c_f277_ee58, 0xcdda_a9dc_b132_927c),
            (15000, 0x97d9_5059_b0b2_db33, 0x6eb3_00fa_d5ff_4e0d),
            (16500, 0x5007_0749_66d3_f40f, 0xf4bf_9f4d_c7d4_4b8c),
            (18000, 0xaf49_f305_9a0b_22aa, 0x1e86_74e6_2a17_07c4),
        ],
        last: 0xb14e_5e0c_36df_3cb1,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xa2ff_f0d2_d0a3_860b, 0x9e30_a59d_c434_8006),
            (3000, 0x5f17_5c75_a4f2_7bb0, 0x524b_1ec2_d5bf_006c),
            (4500, 0x3e51_84bb_d384_68ab, 0xe564_723e_4a3e_fe8c),
            (6000, 0x627a_2b4c_ee77_ad0c, 0x27f6_08ec_ddc7_f557),
            (7500, 0xb999_c51c_45ea_2b0c, 0x5e0a_6908_24fd_26b7),
            (9000, 0x1c55_295a_b675_9f0c, 0xcfe5_4ed1_d0b4_06af),
            (10500, 0xb2d0_0e2e_d28f_3437, 0x5d2e_b4e1_28fc_366f),
            (12000, 0xe1d8_2818_f893_91f3, 0xa094_d5fb_adac_138e),
            (13500, 0x814c_c107_a452_777b, 0x79a4_8584_6222_3906),
            (15000, 0xeadd_65fd_1972_d719, 0xf329_8034_ec69_04b6),
            (16500, 0xd2c5_3320_b788_35d0, 0xe16b_2b67_e705_3a8e),
            (18000, 0x427b_bc2f_58eb_e1cf, 0x7ac8_6b8f_9031_6362),
        ],
        last: 0x95e2_537b_34db_37cf,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x58aa_3d31_c68d_333a, 0xeca6_eb13_e038_c797),
            (3000, 0xc49e_2c56_69d3_8ac2, 0x9f08_a59a_57e7_63b0),
            (4500, 0xf86d_7bd0_26a0_1098, 0xa687_d0b1_5546_d6bd),
            (6000, 0x6371_7ca2_e09f_91b3, 0x65cd_69ae_8df7_1830),
            (7500, 0x3237_8f77_849b_1095, 0x6d09_41e9_acd8_d354),
            (9000, 0x49cf_7a22_2351_a93c, 0x1ea2_17dd_c8b3_a887),
        ],
        last: 0xa120_c330_0f43_d9f3,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd800_e8c2_edac_d3d4, 0xc500_72e4_2517_178f),
            (3000, 0xa731_5914_aff5_9001, 0xccdc_19ae_6025_5235),
            (4500, 0xfdb1_6466_7e64_a937, 0x28f5_0bdd_e414_9963),
            (6000, 0x5a24_1d8f_8a66_607e, 0x29d2_b764_1927_6e16),
            (7500, 0xa1eb_6bf7_1862_2117, 0xbce9_3f47_8103_0bf3),
            (9000, 0x275c_627f_7ab3_a2ca, 0x54fc_81a1_df15_dada),
        ],
        last: 0x574f_e527_ab63_8770,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd580_df39_b66b_54af, 0x6362_5412_484a_7c4e),
            (3000, 0x9c5d_a810_ade8_2350, 0x5eaa_efd2_0c17_f0b1),
            (4500, 0x2ff2_d1f9_057a_9b94, 0xef39_0a66_ae85_c879),
            (6000, 0xa678_7095_7033_0a69, 0x6756_af46_9025_b77d),
            (7500, 0x5934_cc03_a37d_2f4c, 0x4a92_8857_91c4_799c),
            (9000, 0x0a34_e45c_5894_b287, 0x6bba_8848_dfc0_7886),
        ],
        last: 0x8e65_1183_7568_7f10,
    },
];
