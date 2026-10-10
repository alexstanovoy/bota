//! The determinism contract: whole matches driven by seeded order streams
//! replay to pinned world, view and event fingerprints.

use bota_proto::{
    AbilitySlot, Aim, HeroId, ItemId, ItemSlot, MapId, Order, Pick, ServerMsg, SlotId, Target,
    Team, TickMode, UnitKind, UnitView, Vec2, WorldView, encode_frame_to_vec,
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
            10 => Order::Learn {
                slot: AbilitySlot(self.below(me.abilities.len() as u64) as u8),
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
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x1d70_ed4b_0793_0d5d, 0x03d1_4a5d_3a2b_9853),
            (3000, 0x62bf_6539_c8d7_676d, 0x0305_f36f_0063_bb90),
            (4500, 0x2515_0d8b_6f16_a97a, 0x02a3_05ba_cd5c_a919),
            (6000, 0x6a3c_8f3d_4cca_3f6a, 0x9aba_1316_40c6_4ada),
            (7500, 0xa675_f58f_6430_c789, 0x6b76_d736_e2f3_d967),
            (9000, 0xd404_a364_babb_78f4, 0xdc1b_2cc8_4601_6b1b),
            (10500, 0x97e8_4dbe_899b_7781, 0xee76_4488_f136_3263),
            (12000, 0xdc6d_355b_ba29_e8cb, 0x9dcd_0367_be7b_506d),
            (13500, 0x4834_63be_644e_1970, 0x22bc_d44f_cea3_717c),
            (15000, 0x815d_151e_d8c3_cb8c, 0xe564_9e10_1f22_350e),
            (16500, 0xdb1f_5016_4778_adda, 0x840f_57d0_e566_5446),
            (18000, 0x0e14_b3cb_20fe_2b49, 0x53b9_1742_b9c3_f5c3),
        ],
        last: 0x714f_adf7_cc07_06db,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x9366_077b_5ad9_494b, 0x18e1_66ee_37a2_6248),
            (3000, 0x7ebb_c6da_5a66_4fc1, 0x3466_976f_5a60_531d),
            (4500, 0xc659_7bf4_8262_ae98, 0x307f_5eda_6286_7ac6),
            (6000, 0xa18f_2b93_b9c2_9f5d, 0x0f60_f584_df66_15f8),
            (7500, 0x4458_da32_f660_a0c0, 0x46a2_171d_988c_6d28),
            (9000, 0xecbe_ef04_1bee_deab, 0xf3a6_29c4_999d_add4),
            (10500, 0xc0a8_5916_07b2_3541, 0xc663_df51_acac_6728),
            (12000, 0x426b_32aa_9154_b129, 0x5323_c982_8b59_27d0),
            (13500, 0x4b6e_a7ac_f5d5_4ab9, 0x1925_67e0_6440_3b1e),
            (15000, 0x3277_86ce_0489_0027, 0x5ac9_ce14_c5ce_eea5),
            (16500, 0x92bd_f35b_c4f2_8af5, 0x3d2b_643d_a3a2_04a5),
            (18000, 0x7c3e_1501_c68e_5307, 0x77bb_ab2f_5108_363f),
        ],
        last: 0x82a5_e695_c929_3acc,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x3467_ab5d_a110_4a2a, 0x9391_3207_ca65_69c6),
            (3000, 0xb6a1_2cbc_6c60_881e, 0x4562_efbe_5ed7_65bd),
            (4500, 0x9e91_8110_458c_56bb, 0x0ce8_ff75_3bb9_0a66),
            (6000, 0xbf52_99b7_70d5_c785, 0x7e8e_83fd_14a4_86e1),
            (7500, 0xe2b7_97c8_e010_fad3, 0x7d52_52f5_556d_3793),
            (9000, 0xbd7d_42f4_2b98_9a2e, 0xa1a3_886f_6803_cb02),
            (10500, 0xd5a2_395d_a981_8a7d, 0x89d6_5c16_a041_8b0d),
            (12000, 0x24d3_3893_de2b_bfe5, 0x3949_8458_03cb_4ac6),
            (13500, 0x9a90_e093_5306_1fbc, 0x7df7_04e7_9fd4_b7f9),
            (15000, 0x5e65_4206_ed30_3f11, 0x159a_cbb7_d288_93c2),
            (16500, 0xffd1_f755_0a4a_5682, 0xe0c3_f05c_3446_cad9),
            (18000, 0xa00e_dc50_a3c4_9c66, 0xc808_44de_9674_b3a9),
        ],
        last: 0x54a7_ce60_c77b_29b4,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xdd90_4f55_0404_eeb3, 0xaea1_c716_a6c9_69fa),
            (3000, 0x3aa0_d10f_ed2a_5bff, 0x6ed1_7ae2_61d9_0390),
            (4500, 0xaea5_d59e_36a0_81ea, 0xee9a_2e2e_89aa_9f72),
            (6000, 0x6429_6f1e_8d54_85c7, 0x62c7_dd29_67a9_2981),
            (7500, 0x1faa_96c9_d1f7_45e1, 0xafe8_897b_9111_8170),
            (9000, 0x8c74_35ad_067c_2dc2, 0xae63_f4fc_f87d_c104),
            (10500, 0x7781_caa0_6738_3ebe, 0x9a46_27a9_ab0f_eca8),
            (12000, 0xdaba_3ba2_feb3_5ac2, 0x9f55_7c61_f142_d69b),
            (13500, 0x6c52_e4e4_7c52_ff18, 0x81bb_3214_eef5_8045),
            (15000, 0x422d_35f7_7742_c62a, 0x508e_3cbb_60bb_2bb2),
            (16500, 0x62b0_5c6c_27a9_2605, 0x3d5e_e07b_46f7_e38d),
            (18000, 0x154b_be7a_c6dd_4fde, 0x5cdb_7066_7c90_a3ae),
        ],
        last: 0x92bf_7ee3_924d_887a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x3612_f0fb_aa4a_5597, 0xa6b0_4924_1c2d_974d),
            (3000, 0x451f_5791_c592_e898, 0x2b70_2ed3_92d1_bb68),
            (4500, 0x54c0_43fb_f9f1_cd17, 0x93f4_6136_0185_9266),
            (6000, 0xab52_ec1c_ecb4_3346, 0x1df2_6151_f107_233f),
        ],
        last: 0x0a3a_89f7_2380_2ec2,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x83c6_ca1c_dc7b_875d, 0x5624_9551_0111_b5f9),
            (3000, 0xae9d_1550_79d2_2c78, 0x3d08_bc73_85a0_886d),
            (4500, 0x1db3_36ce_8d78_4f31, 0x427b_cfb1_d62b_9aae),
            (6000, 0x13d5_8b07_a0fd_3d88, 0x49e8_7f7d_8968_71e0),
            (7500, 0xbacd_8044_3fcf_a48d, 0x80bb_0428_983b_b6ac),
        ],
        last: 0xd713_5bc7_bed6_df2a,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xea96_8160_b2cc_6796, 0x71a9_68ae_9b60_90da),
            (3000, 0x6b23_874f_48e7_22b6, 0xf158_17a4_9797_4e86),
            (4500, 0x93be_43a8_62ad_265b, 0x0869_8c71_8e0d_c708),
            (6000, 0x2844_6e22_fbc2_4105, 0xbd16_0b4e_177f_1cfd),
        ],
        last: 0xbf3d_102f_50a9_a7dd,
    },
];
