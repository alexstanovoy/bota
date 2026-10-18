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
            (1500, 0xe81b_40fd_32e9_578c, 0x9217_148d_36cd_dd64),
            (3000, 0xb823_fba3_70eb_cff4, 0xfe58_e4fc_56de_31fb),
            (4500, 0x2877_f9d8_767b_c050, 0xe5a8_1f42_7de3_9e36),
            (6000, 0x7290_f0e1_0dfa_cf22, 0xc7aa_a043_c6b8_bda9),
            (7500, 0x9f34_01ae_5e20_1e2c, 0x9511_4bac_ad2c_9dac),
            (9000, 0xc9b6_e485_4446_206d, 0x8e9e_a14a_3aa4_b593),
            (10500, 0x4da6_b610_fec0_e81d, 0xe795_bf9a_b402_e831),
            (12000, 0x56ce_c098_c1a0_19e7, 0x945c_e6c0_359a_19f9),
            (13500, 0x17e7_3e63_a4c3_0609, 0xf5f4_a572_f5e4_bf38),
            (15000, 0xaeee_55a1_4b15_b4cd, 0x7c49_4144_7f9f_1de2),
            (16500, 0xd874_5ca0_fd5b_0cea, 0x792f_c3bd_f476_06cd),
            (18000, 0x292f_efd4_b2e8_0637, 0xd60f_5ac0_c2c8_520f),
        ],
        last: 0x2cb2_33ec_cf1c_1c5b,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x845d_9965_1652_4c4b, 0x77b9_989f_f666_dbc1),
            (3000, 0xa87d_8f36_9106_9e15, 0xdd45_ff05_ba63_fda9),
            (4500, 0x0b51_53f3_d071_ee63, 0x2f50_f9dd_6be3_c9d6),
            (6000, 0x839b_64eb_90c3_2e9b, 0x9251_e3f8_0245_a944),
            (7500, 0xa0e8_2e51_9d2a_c7d0, 0x9933_f365_665c_8c4e),
            (9000, 0x60a4_1620_ca63_faf0, 0x09f5_d29c_7bd4_57af),
            (10500, 0x5b40_87a3_3368_3dd0, 0x9827_bb1a_9511_47f4),
            (12000, 0x0a17_02fb_7042_01c2, 0x4701_a3ed_29bf_7ed9),
            (13500, 0x444e_2689_7ef9_0acf, 0x2f8f_f0cd_8df1_a892),
            (15000, 0xe0ef_36a1_1049_8738, 0xe5c0_628a_be28_5cbb),
            (16500, 0x7c04_81fb_805d_7930, 0x67ff_4f13_4a37_3fe6),
            (18000, 0x56a8_ac1d_c84f_4d6f, 0x18b4_5aa6_8816_f11b),
        ],
        last: 0x37ec_74b8_5501_7962,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x2c66_1ff3_bc08_af8b, 0x7d3f_116b_8a4b_400e),
            (3000, 0x3176_2a4b_3858_e7c9, 0xb763_2197_540a_a1cf),
            (4500, 0x0a27_ab10_c894_305c, 0xe072_7f97_5fe1_e53c),
            (6000, 0xb52a_749f_1c38_c443, 0x86de_b5cf_3c7d_e619),
            (7500, 0x8394_9551_8a6f_f95c, 0x0dd0_68d7_2212_b1ad),
            (9000, 0x8e4d_d715_9c00_3d37, 0x3bcb_8d49_d71f_5464),
            (10500, 0xf231_52bd_4eb8_5fc9, 0x827e_aa8a_966b_e07a),
            (12000, 0x2b80_bffb_4958_12a8, 0x40a1_93d3_e08c_ef96),
            (13500, 0xb2b2_cda7_9e4c_02e6, 0x0cd8_e2d8_c735_9747),
            (15000, 0xeb44_4efd_370b_a210, 0xfcae_cdd9_6de9_d68e),
            (16500, 0x5081_9761_e768_e55c, 0x0121_47cf_5865_9c2c),
            (18000, 0xf85d_a5ed_4249_e94d, 0x0ea4_ba78_cce6_7f71),
        ],
        last: 0x0896_e473_ffd5_a27e,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x68da_9808_98ca_bb58, 0xf595_bd59_49f8_6f73),
            (3000, 0x2e1b_fda3_c74f_2007, 0x2b6f_0163_806a_3daf),
            (4500, 0x0e2c_8283_af67_e60e, 0x4516_28f4_1e32_95f9),
            (6000, 0x78b0_cf2d_ca47_2331, 0xd861_4967_01d9_9b52),
            (7500, 0x23b5_2317_8dee_62cd, 0x0962_d9c6_9dd8_3904),
            (9000, 0x20fc_9a64_f0d2_df56, 0x1d09_bb7c_da77_0e0f),
            (10500, 0xf352_fb10_0d3e_8398, 0x0379_0e5b_7649_19a4),
            (12000, 0xf7fc_ec0f_5f64_d5eb, 0xd758_75db_36fb_0d6a),
            (13500, 0x77b2_5612_8aa1_2a9e, 0x8343_ce3b_6ad2_7584),
            (15000, 0xff34_7ac5_5486_53eb, 0xbcf0_e32e_8822_2adf),
            (16500, 0xdb1f_264c_a1c7_aecf, 0xd90d_360f_4488_8102),
            (18000, 0xd868_a99a_a404_80e8, 0x0790_a2e9_497a_0751),
        ],
        last: 0xc409_0f9f_2be2_dc8b,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4e86_3f21_99c9_4213, 0x7398_a2ef_5d51_d29b),
            (3000, 0x6bcf_6cc4_9e3c_b2bb, 0x2ce3_4bab_0d41_7f23),
            (4500, 0x9530_e224_69db_1589, 0x4f9f_7b57_8694_34de),
            (6000, 0x22d0_3ade_ae67_e886, 0x79be_1085_ea0f_f691),
            (7500, 0xce0b_cd82_a31a_15a4, 0x45b9_83e3_a1bb_6aa9),
        ],
        last: 0xa8ef_2c40_bb85_68c1,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf87b_c596_5da6_619d, 0x1ad3_9797_797f_dabb),
            (3000, 0x96cf_ea30_5264_118a, 0x11a5_96cd_882b_5a73),
            (4500, 0x8156_4f0d_be30_a3b4, 0x87d8_5675_7e91_2b0b),
            (6000, 0xc858_0874_c09a_37c7, 0x7a84_8e12_a650_557b),
            (7500, 0x54df_391b_944d_f858, 0x2bec_9a0e_2017_1d4b),
        ],
        last: 0x1310_e315_3b70_12e0,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x0e40_6e7c_980a_3cfe, 0x3c62_d4c6_2913_2635),
            (3000, 0x58d1_aca9_7581_9f07, 0x04b6_7d5a_46b4_4f15),
            (4500, 0xa496_9e30_59b1_aaf8, 0x1d54_83c1_c759_4245),
            (6000, 0x2755_9027_0466_699c, 0xe194_f02e_fadc_9b97),
            (7500, 0x9a67_05af_dfd3_bf98, 0x5fcb_42e2_127e_571f),
            (9000, 0xfe06_e283_f722_64c3, 0x1f96_8aca_c060_c6ac),
            (10500, 0xb0ea_eb20_b418_45c9, 0x5a36_c90b_2f38_7aeb),
        ],
        last: 0x2f70_3464_56ad_6284,
    },
];
