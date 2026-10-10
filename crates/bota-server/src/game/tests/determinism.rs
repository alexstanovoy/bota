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
            (3000, 0x63e4_c4df_2d1e_f878, 0x9342_5466_c74c_a292),
            (4500, 0x7975_5d3b_2de1_80c4, 0x500d_d3d7_04c2_63dd),
            (6000, 0x0db5_4522_9f9d_e2fc, 0xcd6a_6ee3_53ba_eb0f),
            (7500, 0x9bb8_85bd_61f8_4566, 0x8993_e96a_de24_8031),
            (9000, 0x45b6_1ee0_bf34_ccd4, 0x235c_9147_85a9_1ce1),
            (10500, 0x4143_6729_5297_dcd5, 0xe0dd_ec61_9702_caeb),
            (12000, 0x3c16_78ba_cb3e_4448, 0x96ba_d7a6_d5e0_083e),
            (13500, 0xa157_09d8_9139_f613, 0xf554_7d7c_0eec_8104),
            (15000, 0xd7ad_f0de_3481_248d, 0xa95f_51fb_6515_0e15),
            (16500, 0x71df_5c9a_ebd9_51f5, 0xb1c5_c346_7f6e_75a9),
            (18000, 0xdc07_c216_2f48_a76f, 0x824c_61ef_fea9_febc),
        ],
        last: 0xdf6e_8290_2bb4_b572,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0xa148_1d05_d920_e372, 0xf46d_ac4b_b5de_6db2),
            (3000, 0x07b1_3167_065f_942a, 0xe7c1_0a04_b9d1_5645),
            (4500, 0x5402_de1a_1fd6_1c2e, 0x2147_160c_aedc_4fab),
            (6000, 0x526f_1ed5_e0b5_bd6a, 0x2b95_7e00_74ee_5087),
            (7500, 0xadbc_1090_e810_78e2, 0x2434_15a9_869b_1dff),
            (9000, 0x2865_4e91_1270_4eb8, 0x59f5_4deb_6ad5_4a3a),
            (10500, 0x2c57_aae2_2e34_893e, 0x11cc_2b1d_4626_14a1),
            (12000, 0xad1e_390a_a3d5_1ca7, 0xbbeb_eb91_2b0b_3cbf),
            (13500, 0x0d65_097c_ef0d_fba7, 0x474e_9bfb_7a0d_3110),
            (15000, 0xe23d_6909_68d5_a6a8, 0x8686_c42e_831a_a9c8),
            (16500, 0xddb1_429f_831c_273e, 0x4b47_9165_6e8c_afab),
            (18000, 0x3fd0_f06c_0050_6a68, 0x0ec4_f79d_f44e_bd43),
        ],
        last: 0x5698_56f7_4742_2bbb,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0x901f_1cf8_81e5_c4fe, 0xae6c_9551_e7fe_d462),
            (3000, 0x58e9_b219_8fd7_55a3, 0x148e_f63e_848d_c362),
            (4500, 0x271e_d5fa_26b8_cd09, 0x9313_09c7_f698_4da0),
            (6000, 0x5ba5_df45_52b9_6ad2, 0xd99b_78d6_1648_a03b),
            (7500, 0x1e7a_237e_8f3d_50fc, 0xfb8d_5780_cd9c_0d60),
            (9000, 0xf0f5_cf4c_ef7d_ef31, 0x093c_efc1_4a44_d0d5),
            (10500, 0x0a96_bef1_82f4_7a16, 0x2889_c29a_26c1_da09),
            (12000, 0x9d57_8d4a_b937_a29a, 0xcb72_38a7_4eea_bdb2),
            (13500, 0xfe07_3feb_77c5_21fb, 0xec96_668d_71a4_261c),
            (15000, 0xd57a_61e2_81ef_cf78, 0xa2b0_c2ca_da27_d9e2),
            (16500, 0x64d5_5253_6f89_0d41, 0x7449_26ec_09ea_417f),
            (18000, 0x7513_5ad9_8ae8_b4bc, 0xd5ea_62c2_8a60_330a),
        ],
        last: 0x0a3f_2029_1db9_da02,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xbb28_a820_2d54_338c, 0x51c0_2b35_874f_98b4),
            (3000, 0x1fbc_0291_7813_1ec2, 0x3f0d_99db_6d45_f532),
            (4500, 0x1d05_55ed_0835_963f, 0x2ac0_f78d_c4af_4546),
            (6000, 0x4063_ba36_53e6_f2f4, 0xaff0_8350_f590_0582),
            (7500, 0xcd4f_ed4e_5305_eda6, 0x1c2c_933f_c6df_b97b),
            (9000, 0xa786_5512_7403_0c5a, 0xb183_fa8c_08a4_cb48),
            (10500, 0xc931_1a01_5e0c_66c4, 0xa216_1cd1_ac56_6516),
            (12000, 0x1472_03de_b2ef_67f5, 0xb6f1_eabb_0ecd_d773),
            (13500, 0x5ae1_8107_d74c_9075, 0x2a4c_eaf3_dc9a_dab0),
            (15000, 0x86cd_e7e7_ab1f_cd37, 0xb077_0dd7_20b2_76bf),
            (16500, 0x6380_574b_295f_6523, 0x6a1f_1a13_3674_5c09),
            (18000, 0x790a_6e81_8784_134f, 0xd23c_7f3c_8b8f_6692),
        ],
        last: 0xb6a1_3d3a_dad7_87bb,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x2f04_17a3_ff00_bee3, 0x3a7a_198d_e79c_86c4),
            (3000, 0x978e_0251_c0bf_300b, 0x9c06_4192_a350_4f3c),
            (4500, 0x4d2a_df38_8816_0dd6, 0x62e6_169d_5742_395b),
            (6000, 0xd1f6_c7b7_a18a_8d14, 0xffc6_8640_375f_7d36),
            (7500, 0x077e_149d_d28d_5b45, 0x60b7_6009_deb2_4310),
        ],
        last: 0xa263_aab6_92bc_09b0,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x58c2_37a0_e671_cb9b, 0xe26c_2918_3af4_5fe3),
            (3000, 0x3d70_cfe8_e7a4_bedb, 0x157a_8bca_b09b_d497),
            (4500, 0x688d_3c84_d735_662c, 0x2ff0_dc15_d29e_f672),
            (6000, 0xa89b_eae5_4195_7ff8, 0x3282_ff86_6998_b340),
            (7500, 0xadc1_8404_4c8e_f2e0, 0x2905_38a3_a331_1dfb),
        ],
        last: 0xe6a4_5b76_17ba_38d4,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x1d8f_1e56_0f2f_4132, 0xcd8d_d0c3_6823_1be0),
            (3000, 0xe38d_6b2d_a188_44ce, 0x9c86_f26c_26dd_fe3c),
            (4500, 0x8f5c_a605_bf28_fab3, 0x1640_0d42_5601_976b),
            (6000, 0xa990_ea74_f588_8cb7, 0x53d3_d982_e787_4ec8),
            (7500, 0xa13b_4a8a_7299_273a, 0xe85d_d0ab_2276_d8c9),
        ],
        last: 0x5d35_da22_b320_f619,
    },
];
