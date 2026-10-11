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
            (1500, 0x99e8_a84a_9a73_f517, 0xefad_22d1_a272_e515),
            (3000, 0x3f8b_6779_1d08_991a, 0xb6d1_f117_d19a_93f1),
            (4500, 0x0eca_962e_0a65_bf11, 0x4b30_caa9_569c_3ddc),
            (6000, 0x3c1e_b780_28be_ceda, 0x1d7d_15ae_cdff_23cc),
            (7500, 0xc44b_d304_8253_7156, 0x10f4_28f7_c006_7043),
            (9000, 0x37a1_0ad7_625b_adfb, 0x771f_04c7_c844_6a51),
            (10500, 0x2e50_5baa_69bf_757d, 0xe33a_37fb_dcf1_5165),
            (12000, 0x62ab_82ff_5182_ae31, 0xf983_631e_6b26_12ba),
            (13500, 0x3e9c_c299_c640_2576, 0xf629_bd69_6772_cad1),
            (15000, 0xe51d_0399_0bfe_643a, 0x63bd_d407_b95b_4dda),
            (16500, 0x1c7d_9376_a5dc_aa88, 0xe2d2_61d0_1e59_52ca),
            (18000, 0x8251_bd52_b1b6_62f8, 0xa225_976f_057c_9dbf),
        ],
        last: 0x806a_8638_6fdc_8de8,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x69fb_e57a_4ff6_353e, 0x8cd9_3271_2543_e0f8),
            (3000, 0x7608_ca5c_a8b5_240e, 0x4821_a16b_8d52_0411),
            (4500, 0x3660_fc07_b5f5_7f85, 0x5db3_b130_dcba_d523),
            (6000, 0x632d_f2f4_7805_ce5b, 0xec01_f176_bacf_679d),
            (7500, 0x6eb9_4062_a8b8_6068, 0x9bd0_2eed_2767_48da),
            (9000, 0xc75a_c6f5_e13a_56df, 0xf047_459e_83a2_1425),
            (10500, 0xa026_ba88_2f38_b634, 0x200f_c921_2e08_2b4d),
            (12000, 0x514f_3d92_a743_00d4, 0x56cf_d13b_f4d8_146b),
            (13500, 0x725a_eea1_141f_6179, 0xc744_a928_bed1_d68d),
            (15000, 0x936c_fbc3_c6e3_a2fc, 0x5b9e_2272_3759_2395),
            (16500, 0x0940_cc46_ecd9_770c, 0xaa87_2fa7_3768_fbbc),
            (18000, 0x3898_2b5b_5246_e357, 0xa58e_2859_e945_9a46),
        ],
        last: 0xa333_2a0c_3f2b_b373,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xcc80_347d_e135_07a8, 0xf578_b9f3_d081_a713),
            (3000, 0xee34_0c7e_fa08_a81d, 0x4f43_659a_37bc_e555),
            (4500, 0x723a_9353_74ba_65f8, 0x9cbf_395a_fb9e_afd4),
            (6000, 0xf926_15ef_2d6e_820e, 0x4a11_29b5_4e52_c1b8),
            (7500, 0xb497_5cca_4550_b010, 0xcd79_c321_86bd_ba32),
            (9000, 0x0cca_a20c_51ab_2185, 0x217f_f63e_bdcf_0b15),
            (10500, 0x91f5_9cb9_7b09_d6dd, 0x80fb_ebf7_7f9a_a018),
            (12000, 0xf2e7_2ed4_4ce2_6ee8, 0xbac9_4b63_7aed_d03c),
            (13500, 0x9495_9218_1d5f_ef45, 0x56f7_7da9_4d05_0841),
            (15000, 0xe92b_f641_c26a_af38, 0x6071_54c6_d353_2bce),
            (16500, 0x5832_6588_13c4_9a7f, 0x964c_796e_be8f_745e),
            (18000, 0x4b8e_34d6_e2ec_b09a, 0xeaad_ed66_c573_f8cd),
        ],
        last: 0x3703_c0c0_3c84_cc1c,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xb957_93df_6229_1014, 0x4dd4_a361_8c19_b1a7),
            (3000, 0x6e9e_6f7f_12fd_20a7, 0x1543_940a_05d6_5b14),
            (4500, 0x6b86_180f_7ac3_5994, 0x0f70_fe76_21a9_1148),
            (6000, 0xc152_4b39_4ab7_2387, 0x3a0f_b056_8626_80d6),
            (7500, 0xdf76_b62c_bc9a_9f96, 0x1fe0_371d_0aa2_53bc),
            (9000, 0xfa3a_6114_8171_47cd, 0x23ce_f014_7992_ba33),
            (10500, 0xbfb7_6018_41f9_2769, 0x41be_f3af_1a25_007b),
            (12000, 0x19ab_4406_c42b_4156, 0x7a65_70de_7f7f_6450),
            (13500, 0xccce_7422_8d5a_8d3e, 0x1809_eb84_1b6b_7133),
            (15000, 0x43dc_762d_b414_748a, 0xc0c8_0016_13a5_0084),
            (16500, 0xe576_2e12_8fe0_190d, 0xf8d8_1653_3427_909a),
            (18000, 0x0182_1613_2b6a_dec1, 0x9d42_5d6d_8a09_7cba),
        ],
        last: 0x8d09_4806_50a4_c0d3,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7527_1669_5dba_3a8d, 0xf9ac_27dd_4659_9e71),
            (3000, 0x4608_68b0_3647_56cc, 0x955f_068a_5b5c_e8be),
            (4500, 0x39fa_e455_076d_3d61, 0x7154_90b4_59c6_f764),
            (6000, 0x27af_3a6f_08d5_1c78, 0x7b1c_2b5e_0498_17a4),
        ],
        last: 0x1964_0af2_7026_8b04,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xe024_81aa_a935_0846, 0x00a5_1504_f4e5_6953),
            (3000, 0x63cc_db03_e7c0_c91f, 0xa699_649d_0db9_6f90),
            (4500, 0x917f_9981_79c5_5cd3, 0xa09d_eef5_6cdf_a4e1),
            (6000, 0xd815_4f3a_d1a5_d33f, 0x7c81_bfe3_73b1_b46c),
            (7500, 0x2e11_2e4e_c84a_aa72, 0xbef7_003d_96e6_af8c),
        ],
        last: 0xd8cb_88b2_117e_19c1,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2f7f_4b1c_59b9_6e78, 0x5442_2847_238c_5eb6),
            (3000, 0xa73a_ad1a_735b_0a38, 0xb0da_0c3b_790b_f498),
            (4500, 0x1bd8_0a91_aca1_b120, 0x9fc2_e577_106b_e439),
            (6000, 0x7878_0416_856a_42f1, 0x06f6_2ba1_b3e5_0386),
            (7500, 0x4bef_7327_88dd_3d18, 0xf0cb_cf55_40e6_4a94),
            (9000, 0x7a7c_2714_174d_bbf5, 0x5b6b_784f_a1f9_7592),
        ],
        last: 0xe884_c4f6_8277_0e99,
    },
];
