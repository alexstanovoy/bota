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
            (1500, 0x4002_20de_21e3_734a, 0x48b3_d008_fd83_ccc1),
            (3000, 0x63e4_c4df_2d1e_f878, 0xd4d6_52f5_122a_cb66),
            (4500, 0x7975_5d3b_2de1_80c4, 0x467b_f4a4_3922_89f0),
            (6000, 0x0db5_4522_9f9d_e2fc, 0x9f66_8997_c71e_0f54),
            (7500, 0x9bb8_85bd_61f8_4566, 0x33b4_ea2b_9930_6e33),
            (9000, 0x45b6_1ee0_bf34_ccd4, 0x3365_a2d8_e095_834c),
            (10500, 0x9ec5_5932_4a0c_d514, 0x6594_c9b7_8ef9_96a2),
            (12000, 0xce6e_ff0a_5a80_9114, 0x3702_371a_af70_a7b8),
            (13500, 0x0ee7_ab42_6dec_efe3, 0x0e79_8964_7449_b940),
            (15000, 0x1a63_fb0e_2001_cda9, 0xd5eb_38ec_2cfe_6d1c),
            (16500, 0xb35f_b597_d5d7_4843, 0x1fa6_a076_d221_693a),
            (18000, 0xa8b4_6fc2_a0cb_cd96, 0x34eb_1304_8ebc_a933),
        ],
        last: 0x49fd_feb1_d21b_406f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xa148_1d05_d920_e372, 0xf46d_ac4b_b5de_6db2),
            (3000, 0x07b1_3167_065f_942a, 0xc072_d19f_6a69_39f5),
            (4500, 0x5402_de1a_1fd6_1c2e, 0x0827_930c_601f_a058),
            (6000, 0x526f_1ed5_e0b5_bd6a, 0x162a_7627_df24_5a2b),
            (7500, 0xadbc_1090_e810_78e2, 0x3bfd_206a_8d79_cc1d),
            (9000, 0x2865_4e91_1270_4eb8, 0x939b_a326_bb21_26f5),
            (10500, 0x2c57_aae2_2e34_893e, 0xb7ae_9eb3_5d56_a7b9),
            (12000, 0xad1e_390a_a3d5_1ca7, 0x1df1_e725_6e5b_5ace),
            (13500, 0x0d65_097c_ef0d_fba7, 0x3163_92ee_00da_e45c),
            (15000, 0xe23d_6909_68d5_a6a8, 0x0666_3f99_04d1_e3d4),
            (16500, 0xddb1_429f_831c_273e, 0xee12_7417_d4cc_174a),
            (18000, 0x3fd0_f06c_0050_6a68, 0x53f7_2a17_b179_a3f9),
        ],
        last: 0xe679_b118_0ec4_b45a,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x901f_1cf8_81e5_c4fe, 0xdd39_91cc_adcc_c8db),
            (3000, 0xa137_6c7c_44dc_a4b8, 0xad77_8f01_838c_237e),
            (4500, 0x1f00_39c0_db1d_b082, 0x0cea_0d2f_ed1f_f1a5),
            (6000, 0x57c3_1145_8be6_9094, 0xa3af_080f_1270_1916),
            (7500, 0x6925_0642_a791_b6da, 0x994a_4c16_5c33_8a44),
            (9000, 0x7cb8_322a_45c4_df03, 0x2476_5d26_4fc7_3278),
            (10500, 0x23b5_451f_f594_627b, 0x88fc_f124_8557_0d29),
            (12000, 0xb587_e0c3_771f_a829, 0x0fe2_e399_48d9_765e),
            (13500, 0xb745_b26d_659a_c721, 0xb656_58fe_2693_bd05),
            (15000, 0x53e5_1589_cd69_9a26, 0xe802_5097_8dc3_63ef),
            (16500, 0xc198_9a1a_fa87_8d20, 0x9bd6_ee93_567a_19dd),
            (18000, 0xa97a_4093_0746_af85, 0x4db3_e023_89cb_ec7a),
        ],
        last: 0x3d76_cd8b_2551_718b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xbb28_a820_2d54_338c, 0x5652_2c8c_4dd0_61a0),
            (3000, 0x1fbc_0291_7813_1ec2, 0xd837_33b0_5c91_9f0e),
            (4500, 0xe5f9_23cb_12f1_0ae2, 0x811f_8139_a3d9_e7d3),
            (6000, 0x80c9_82e4_df68_5e5c, 0xffa8_375b_058d_db36),
            (7500, 0xf792_8dd9_376f_ff1c, 0x1432_b58c_2820_0e3a),
            (9000, 0x6f70_26a5_6acb_d367, 0x4c1a_3f5d_9326_8887),
            (10500, 0x92df_3fd2_f336_efae, 0x4613_fe59_20a4_289d),
            (12000, 0x3d2c_6c13_2fef_eaee, 0x2c9d_4e31_4091_9c2f),
            (13500, 0x5598_d74f_848d_6b84, 0xa023_76c4_5412_341d),
            (15000, 0xd518_8676_641a_c92a, 0xb9e9_4791_4d71_7009),
            (16500, 0x8f80_96f6_529a_b4a8, 0xb3c3_0548_291b_4a81),
            (18000, 0x8272_d4e9_a975_7d84, 0x43e7_2804_49d1_5155),
        ],
        last: 0x2577_f059_cc89_e0e6,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x2f04_17a3_ff00_bee3, 0x3a7a_198d_e79c_86c4),
            (3000, 0x978e_0251_c0bf_300b, 0xea07_f17b_85d8_f4c8),
            (4500, 0x4d2a_df38_8816_0dd6, 0x77fd_11bc_9eae_0c2f),
            (6000, 0xd1f6_c7b7_a18a_8d14, 0x6921_c646_6e8d_7331),
            (7500, 0x077e_149d_d28d_5b45, 0xa272_6c75_fd6c_70c1),
        ],
        last: 0x48ba_193e_2dae_221b,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x58c2_37a0_e671_cb9b, 0xe26c_2918_3af4_5fe3),
            (3000, 0x3d70_cfe8_e7a4_bedb, 0x6b7b_da05_79c8_0049),
            (4500, 0x688d_3c84_d735_662c, 0xe7e1_d654_a0c1_2f4a),
            (6000, 0xa89b_eae5_4195_7ff8, 0xf2b5_7670_a6f1_2a49),
            (7500, 0xadc1_8404_4c8e_f2e0, 0xef5d_6963_1a63_1c78),
        ],
        last: 0x7b00_ab1d_909e_ca52,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x1d8f_1e56_0f2f_4132, 0xcd8d_d0c3_6823_1be0),
            (3000, 0xb397_1bb7_0ffc_b325, 0x2b60_497e_d7ff_6fdd),
            (4500, 0x2360_920e_0e31_d6d0, 0x7ab1_1800_b765_0bdd),
            (6000, 0x54cf_d42f_2f38_3655, 0xf529_2875_5cfa_777e),
            (7500, 0x532d_e23b_194e_2c4b, 0xb241_ca41_a74e_f593),
        ],
        last: 0xd408_abda_ed4b_6555,
    },
];
