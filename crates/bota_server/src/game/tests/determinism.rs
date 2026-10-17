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
            (1500, 0xe8ba_c1f6_20c9_18fd, 0xf930_82e6_f9d4_61f6),
            (3000, 0x1fa7_c4ae_dc34_133c, 0x8a50_0d71_cd6f_f1c8),
            (4500, 0x3c54_955a_aea8_1d3d, 0x7c55_dc3e_51cd_3e25),
            (6000, 0x7bb4_225b_f450_7ace, 0xcc11_46e1_d1f8_2408),
            (7500, 0xcad4_6797_72cd_1804, 0x3ff2_ad6f_6ffb_aa76),
            (9000, 0x5d71_b24b_be13_3508, 0x0f53_dc69_d121_f948),
            (10500, 0x828e_6605_ab6e_dbc1, 0xba63_6bee_8a0d_4ed2),
            (12000, 0x7c30_3c7f_2526_ead0, 0x1037_96ba_f146_bdbd),
            (13500, 0x4428_6101_248b_a799, 0xdec9_7d1f_a6ab_9432),
            (15000, 0x729c_71b4_1d3e_ac5b, 0x328f_8bf1_7831_5778),
            (16500, 0x745b_ac55_04ab_a1a9, 0xaa46_0285_6656_0a0f),
            (18000, 0x01b7_ca2e_058b_52cf, 0x27a7_0be1_65ef_6914),
        ],
        last: 0x04ec_6862_5bf8_3798,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x322c_6cc0_4e82_a5c5, 0x7190_c757_b628_2aca),
            (3000, 0x2cef_f179_46ec_a0fe, 0x2439_266b_7c5f_996d),
            (4500, 0xf106_d22a_d908_4840, 0xffa3_f942_f163_8ca4),
            (6000, 0xb502_953b_9fa5_7faf, 0xcc7c_7d6a_6b85_8eba),
            (7500, 0x8192_8df2_26a9_bce1, 0x2965_fb61_fbb7_faf1),
            (9000, 0xcbee_de78_be69_d3bc, 0xbdda_e65f_5fa3_8757),
            (10500, 0xeed9_680c_8b3c_c4f3, 0x0006_36fe_80a9_c202),
            (12000, 0x3272_5fc5_af42_5015, 0x043e_ecb2_11a4_b1c4),
            (13500, 0x8715_1783_c812_3f40, 0xc1ec_dee3_77a2_ed46),
            (15000, 0x4b6f_f7cd_eaa2_98d5, 0xb60a_456a_caef_5377),
            (16500, 0x865a_1b54_03c6_4d2a, 0xda21_ea4a_b1ba_327c),
            (18000, 0x7eb9_ddcc_6889_d7a4, 0x79db_8eee_be27_377f),
        ],
        last: 0x676b_f7c8_0997_55cd,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x74ea_c335_12aa_413b, 0x7308_0498_9d19_7926),
            (3000, 0xcb2b_3a5c_3343_a9ba, 0xc445_4fe1_fc8b_ec65),
            (4500, 0x7519_c7fd_c264_10f2, 0x899d_4ed6_171b_1216),
            (6000, 0x9a7d_c28a_24ce_164d, 0x75f2_ad54_196b_ccd0),
            (7500, 0x2f7b_9825_37f2_00e7, 0xab66_ccad_c26e_7e48),
            (9000, 0xbe97_25f8_bd04_68c6, 0x2962_b336_171e_13b8),
            (10500, 0x8b3f_79ad_f98c_fe38, 0x1a34_7edb_143c_281b),
            (12000, 0x6a7a_1a97_9d03_3aff, 0x9299_f84b_d9f5_cb92),
            (13500, 0x6352_9cf2_4ad0_5131, 0xcb6c_5b54_19ae_869a),
            (15000, 0x5d35_62f7_b8cc_2583, 0x4f17_cdc1_c3b7_2a0c),
            (16500, 0x2b75_f383_fa82_3acf, 0x7f6a_d8e1_4c85_033e),
            (18000, 0xb310_f4aa_c30b_4cb5, 0x3490_7ab1_d3b0_a8e2),
        ],
        last: 0x2f8e_56eb_a36d_8f5a,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xc761_ffa7_9d08_d18e, 0x01f2_0c53_2d6e_9fdf),
            (3000, 0x0f35_887b_17d0_4b31, 0x7957_a886_4fda_21c1),
            (4500, 0xc7f5_5801_fe52_ddd7, 0x73e9_b5e6_11f0_741b),
            (6000, 0x77d0_9e41_09cd_28f8, 0x26bf_0436_4237_fb4a),
            (7500, 0x2ef7_533b_be49_f193, 0x2a04_8146_2136_4c4c),
            (9000, 0x3473_9a32_39b8_b53a, 0x41cf_590b_9419_bec2),
            (10500, 0x859d_b73a_cb8d_61c2, 0x5b1f_d83a_63cb_1ab1),
            (12000, 0x4378_d310_22c5_cd0d, 0xbbbf_56d9_3f8d_46ce),
            (13500, 0x4f54_462c_69d3_59d0, 0x9e50_5ea3_1341_bb09),
            (15000, 0x7029_eb95_5278_b6d3, 0x7a3b_a14d_85a8_2012),
            (16500, 0x2269_9105_4e40_d6be, 0x1d05_eb8f_e250_a07b),
            (18000, 0x7ff5_dc37_6205_78e8, 0x833d_195d_af2d_2991),
        ],
        last: 0x06ac_75e8_8941_10c0,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0d3e_7aef_78c4_c258, 0x1d13_b33c_cc39_0b20),
            (3000, 0x3959_721d_75b3_736b, 0xbe6f_0db4_07c9_2037),
            (4500, 0xbbbf_f48b_4bac_22bd, 0xf461_ad71_d617_825e),
            (6000, 0x5b64_f041_c88a_448e, 0xf464_222c_2e24_f6c1),
            (7500, 0x6ba8_abc2_26f5_4066, 0x4ca1_0766_2d01_9c87),
            (9000, 0xcf31_7721_9a95_c542, 0x6a31_7f38_5efd_7384),
            (10500, 0xa454_f02f_1663_0a94, 0x6357_1d43_4978_89d8),
        ],
        last: 0xb378_5b44_b71a_980a,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x2bad_2eef_2b21_0ad1, 0xfffc_9bff_fd63_84db),
            (3000, 0x8a1c_076c_290e_772e, 0xe4e9_6ef0_bac6_6bc7),
            (4500, 0x5769_6bdf_d9ca_2130, 0x2367_7ed9_2452_ad2e),
            (6000, 0x5ca1_3289_4c58_85f5, 0xd93b_ec35_ad21_9642),
            (7500, 0x841a_f638_d9e3_93f0, 0x8a3f_83b7_baae_81b0),
        ],
        last: 0x9feb_7908_4054_418c,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x8c32_986a_6008_1248, 0x12f6_73bb_eb12_a93e),
            (3000, 0x6191_c288_5310_93fd, 0xeebf_edf2_eac6_34d8),
            (4500, 0x2547_b106_a86e_b358, 0xa521_c367_8a03_77fa),
        ],
        last: 0x6396_f96a_0d78_6ea5,
    },
];
