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
            (1500, 0xe81b_40fd_32e9_578c, 0x87de_d1bd_31cf_c35b),
            (3000, 0xb823_fba3_70eb_cff4, 0x6c5e_4f40_91b4_44dd),
            (4500, 0x2877_f9d8_767b_c050, 0x8f04_2935_3683_f813),
            (6000, 0x7290_f0e1_0dfa_cf22, 0x04c7_6566_d881_b496),
            (7500, 0x9f34_01ae_5e20_1e2c, 0xcbcf_1806_f88f_ff62),
            (9000, 0xc9b6_e485_4446_206d, 0x5f94_be8e_0811_fdda),
            (10500, 0x4da6_b610_fec0_e81d, 0x16c0_8308_7069_8edf),
            (12000, 0x56ce_c098_c1a0_19e7, 0xf5d5_b7ec_ea9d_7945),
            (13500, 0x17e7_3e63_a4c3_0609, 0xb5f8_075a_5feb_bac3),
            (15000, 0xaeee_55a1_4b15_b4cd, 0xf1ef_8adf_9de7_8bb0),
            (16500, 0xd874_5ca0_fd5b_0cea, 0xf32f_20c5_6405_e0da),
            (18000, 0x292f_efd4_b2e8_0637, 0x5626_dfe1_4962_e772),
        ],
        last: 0x04b9_6a61_5d56_b5b1,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x845d_9965_1652_4c4b, 0x1b11_0c18_5cb8_9b2d),
            (3000, 0xa87d_8f36_9106_9e15, 0x7914_f5cd_a4c9_d89f),
            (4500, 0x0b51_53f3_d071_ee63, 0x0c61_70a2_a2c5_fa8c),
            (6000, 0x839b_64eb_90c3_2e9b, 0x797a_a1a6_fa13_b5e0),
            (7500, 0xa0e8_2e51_9d2a_c7d0, 0xfdbf_4ad8_32a0_7779),
            (9000, 0x60a4_1620_ca63_faf0, 0x7bcf_1073_9987_ddb9),
            (10500, 0x5b40_87a3_3368_3dd0, 0x5de1_c8c1_d0cc_9d9d),
            (12000, 0x0a17_02fb_7042_01c2, 0x319f_492a_61be_b374),
            (13500, 0x444e_2689_7ef9_0acf, 0xa5ec_afa1_9eaf_e35c),
            (15000, 0xe0ef_36a1_1049_8738, 0xed66_5b3a_9b96_d1c3),
            (16500, 0x7c04_81fb_805d_7930, 0x8677_8849_b334_0d42),
            (18000, 0x56a8_ac1d_c84f_4d6f, 0x7bcd_007e_9fc5_544d),
        ],
        last: 0xc3f6_a784_7307_98f7,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x2c66_1ff3_bc08_af8b, 0x13c5_6551_71d1_f00c),
            (3000, 0x3176_2a4b_3858_e7c9, 0xd4c6_33bc_b579_7e74),
            (4500, 0x0a27_ab10_c894_305c, 0xf66e_b0c2_30ed_4e0e),
            (6000, 0xb52a_749f_1c38_c443, 0x6bcd_da67_d8ef_b2d8),
            (7500, 0x8394_9551_8a6f_f95c, 0x2c1f_57c5_b8fd_26a9),
            (9000, 0x8e4d_d715_9c00_3d37, 0xb4e8_68df_38a6_855c),
            (10500, 0xf231_52bd_4eb8_5fc9, 0x63d5_6b01_f69d_abc6),
            (12000, 0x2b80_bffb_4958_12a8, 0xd704_9821_d1f9_e96e),
            (13500, 0xb2b2_cda7_9e4c_02e6, 0xbd03_dab8_1a0e_ea62),
            (15000, 0xeb44_4efd_370b_a210, 0x8cb4_997b_fd0c_8535),
            (16500, 0x5081_9761_e768_e55c, 0xf823_70aa_317e_6fc6),
            (18000, 0xf85d_a5ed_4249_e94d, 0x5427_99f6_583f_143e),
        ],
        last: 0x2a95_9334_774a_ba7b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x68da_9808_98ca_bb58, 0xdd46_5ede_05b6_3556),
            (3000, 0x2e1b_fda3_c74f_2007, 0x9899_c8f3_ea12_52ca),
            (4500, 0x0e2c_8283_af67_e60e, 0xdd47_5bfa_0caf_522d),
            (6000, 0x78b0_cf2d_ca47_2331, 0x6054_884c_305e_4574),
            (7500, 0x23b5_2317_8dee_62cd, 0x6e5c_dcad_4ced_2f96),
            (9000, 0x20fc_9a64_f0d2_df56, 0x19cd_3e2f_e901_d017),
            (10500, 0xf352_fb10_0d3e_8398, 0xfe1e_a8c0_203f_6946),
            (12000, 0xf7fc_ec0f_5f64_d5eb, 0x9359_44c3_880e_b9c0),
            (13500, 0x77b2_5612_8aa1_2a9e, 0x926e_661c_ea4f_a855),
            (15000, 0xff34_7ac5_5486_53eb, 0x76c9_07bb_5ef1_b95e),
            (16500, 0xdb1f_264c_a1c7_aecf, 0x5c30_3dbe_13db_3ba5),
            (18000, 0xd868_a99a_a404_80e8, 0x3349_4e9a_9dd1_0c6e),
        ],
        last: 0xc4cc_eee5_c514_63d4,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4e86_3f21_99c9_4213, 0xaac9_c9f6_c779_0b0b),
            (3000, 0x6bcf_6cc4_9e3c_b2bb, 0xb650_7d2a_00ab_d38f),
            (4500, 0x9530_e224_69db_1589, 0xd74d_acbf_4fbf_743a),
            (6000, 0x22d0_3ade_ae67_e886, 0xe14a_ad51_dcff_1397),
            (7500, 0xce0b_cd82_a31a_15a4, 0x1e8d_8b74_0228_367a),
        ],
        last: 0x3cae_f2ac_e63b_fa93,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf87b_c596_5da6_619d, 0xbe70_17e7_91a7_755d),
            (3000, 0x96cf_ea30_5264_118a, 0x3980_ab5b_0392_97a1),
            (4500, 0x8156_4f0d_be30_a3b4, 0x68bc_6991_cc65_35a2),
            (6000, 0xc858_0874_c09a_37c7, 0xc25d_7f0a_bf9d_017b),
            (7500, 0x54df_391b_944d_f858, 0x47dd_1bb5_c9a4_abe9),
        ],
        last: 0x7c1c_d788_f2e8_1617,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0e40_6e7c_980a_3cfe, 0x4d0e_a993_7c78_36fb),
            (3000, 0x58d1_aca9_7581_9f07, 0x7d9b_eb1e_f5b6_52b8),
            (4500, 0xa496_9e30_59b1_aaf8, 0xad9c_3ed7_3661_5dd1),
            (6000, 0x2755_9027_0466_699c, 0xe15f_6411_97d3_61b4),
            (7500, 0x9a67_05af_dfd3_bf98, 0xb68b_ebe9_60b8_baef),
            (9000, 0xfe06_e283_f722_64c3, 0x199f_e86e_d210_cb41),
            (10500, 0xb0ea_eb20_b418_45c9, 0xd0c9_2d3b_a948_8ad6),
        ],
        last: 0xd20f_7183_adaa_fa75,
    },
];
