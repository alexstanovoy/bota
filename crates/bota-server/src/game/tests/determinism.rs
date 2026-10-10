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
            (1500, 0x3bcd_f64d_58cf_a2c8, 0xe67b_e050_48a6_f7c1),
            (3000, 0x9594_5af4_5d96_36fe, 0xaecf_0088_3757_a01e),
            (4500, 0x2337_5510_aafe_58e4, 0x1d0e_a6ec_bc01_7a39),
            (6000, 0xb6ac_f640_60f5_864a, 0x7358_da92_63bc_a232),
            (7500, 0x4717_977d_03fe_a79b, 0xe007_81c4_6779_553a),
            (9000, 0x9be7_fe04_b8ae_12cc, 0x7584_e973_8d5e_63c0),
            (10500, 0xbd9e_bbc0_346d_08d2, 0xc7fc_aec5_88de_445f),
            (12000, 0xab0c_1a17_b595_f600, 0x6611_a30a_8946_a317),
            (13500, 0x970d_52a6_1bd8_ddb6, 0xf59a_f49c_22f6_e1bb),
            (15000, 0x8eb4_7053_6d33_7323, 0xf2d6_5ca5_f1a9_43e4),
            (16500, 0x21c7_cc1e_19c4_dde2, 0x8897_7ca8_3ae5_2bc6),
            (18000, 0x8ce1_e0c4_ef8d_34c3, 0xf0b8_977f_2a02_05bb),
        ],
        last: 0xb7d3_4a12_d184_137f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[0, 1, 2, 0],
        checkpoints: &[
            (1500, 0x6757_cb40_6af2_c343, 0x041c_469f_555d_aad0),
            (3000, 0x5186_761d_4ef5_6c6e, 0x2675_99fe_4ada_1830),
            (4500, 0x806f_ab6c_4e6a_9a53, 0x5c8c_c3b7_9629_9edd),
            (6000, 0xf061_bd18_8df3_d7b9, 0x6f86_40f8_f3a5_0e98),
            (7500, 0x9d3f_7d25_0312_c619, 0x8500_1272_3acf_53e9),
            (9000, 0xaa06_261a_1b32_1cc8, 0x562f_15e3_0117_d341),
            (10500, 0xcf80_f02c_b2f6_5b4b, 0xb1ed_a4ee_bf9b_b23e),
            (12000, 0x0487_e541_93d5_7793, 0x8558_1575_b35a_711b),
            (13500, 0x198b_6ed1_9c70_3a30, 0x0158_9f47_d3a4_1b3e),
            (15000, 0x1cdf_e791_b3d8_4050, 0x108f_3d5f_73f5_2f42),
            (16500, 0x7537_db74_b1c5_7ef5, 0xfb8f_7f5d_3bfb_7a96),
            (18000, 0x07e1_5dbd_2bbe_5fe3, 0xa703_69fd_1ae9_57ac),
        ],
        last: 0x58b5_494c_43d9_a9af,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[2, 1],
        checkpoints: &[
            (1500, 0xb940_8166_321f_cbc0, 0x3a90_9932_d3df_4ee6),
            (3000, 0x6211_6ad5_8150_73ae, 0xeb49_093b_aa66_858f),
            (4500, 0xdbf0_d795_e37c_d97c, 0xa12c_baee_f285_4c13),
            (6000, 0x4713_57db_232c_114b, 0x4a05_e905_241d_35ef),
            (7500, 0x5bbf_0110_5917_1857, 0x063d_e6fb_3aad_33f7),
            (9000, 0xf0c1_02d8_43e4_0499, 0xe6fc_75af_b698_d7fe),
            (10500, 0x7c86_874b_f5ca_7cb6, 0x45b4_c0cb_4213_19a1),
            (12000, 0xeb09_066b_e9c7_9440, 0xa71b_35ab_24a8_c0d0),
            (13500, 0x135f_e20c_f027_6f05, 0xc2e9_98a5_8cda_afb4),
            (15000, 0x075c_43ff_0881_15ce, 0x9ea9_7880_7e6f_59ba),
            (16500, 0x4beb_3302_ec85_8921, 0x0ca4_1032_258b_a0cd),
            (18000, 0x673a_37eb_ec4f_3349, 0xc7fb_7f61_746c_30dd),
        ],
        last: 0x3643_607c_87a7_1e84,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x18e8_eacb_91a3_a155, 0x9a59_fc8a_9808_2861),
            (3000, 0xf5ed_a6bf_48a2_bebc, 0xaeff_4c03_6b75_ec55),
            (4500, 0x38b7_b9de_1c73_15c2, 0xe247_2e59_1eee_1f9a),
            (6000, 0x03eb_bf05_cb84_e016, 0x9f23_63dd_b90a_2072),
            (7500, 0x6856_de89_1d07_a361, 0x09ef_0b34_8bec_7235),
            (9000, 0xe4d7_a188_4947_f21e, 0x6e4c_d1c0_3d00_257a),
            (10500, 0xb55e_37a0_9ba6_e57a, 0xcaa0_435f_d511_f3d2),
            (12000, 0x677e_1a24_a8b1_3920, 0x9d6d_338d_e395_0a35),
            (13500, 0xd3d3_fc9b_8bec_4d4c, 0xb0b7_773b_2481_c013),
            (15000, 0x0546_eb58_cffa_de84, 0xc5db_f35b_fb92_c24e),
            (16500, 0xa251_6301_4d44_44b8, 0x0c1f_e485_6dfc_8077),
            (18000, 0x992a_d6c1_23d8_9748, 0xf652_fbc9_142b_6489),
        ],
        last: 0x9c27_8650_2f70_4a3c,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xeff0_009f_839e_4f3a, 0xbd28_bb70_514f_693e),
            (3000, 0x39eb_9ba5_cb5f_69a8, 0x6bf6_1271_7e87_41a2),
            (4500, 0xf54a_e578_8ae8_30f1, 0x943a_0bb2_5ab0_62c0),
            (6000, 0xf716_f556_4cde_e5b3, 0x94de_e3a0_cf88_7489),
            (7500, 0xa855_8e3d_47e4_207f, 0x610e_64cc_0193_69e1),
        ],
        last: 0x4dab_76f3_aec8_ee79,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0xa618_99a1_4a6d_01bf, 0x546f_965f_3a57_1d2f),
            (3000, 0x7315_7f08_5534_8b15, 0x503c_288a_2a24_09ad),
            (4500, 0x15be_c305_4860_e519, 0x11c0_5109_4b89_9a8e),
            (6000, 0xf5d8_fcf9_dd8f_cdfa, 0x5393_e1cb_bd59_c4d6),
            (7500, 0x498b_b269_44bb_32ee, 0x1ba4_3b79_72a5_8710),
            (9000, 0x3e84_84f2_b86e_61c4, 0xc982_6746_42d7_56fb),
        ],
        last: 0x4b9d_23d7_81fd_7724,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[2, 2],
        checkpoints: &[
            (1500, 0x412a_5d9d_8a24_bc3d, 0xcf86_76bc_c45a_4faf),
            (3000, 0xf9c8_f789_3629_625c, 0x06ef_fbd3_eda3_4514),
            (4500, 0x79bf_af2a_9718_47e9, 0x564f_90e2_f15b_585d),
            (6000, 0x8318_7ea3_cac9_dfd9, 0x1518_7a80_4ac4_ce77),
        ],
        last: 0x14c8_711c_1e94_b42f,
    },
];
