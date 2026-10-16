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
            (1500, 0xa9f6_bfa5_6814_0056, 0x6cce_1cbc_b256_a013),
            (3000, 0x7e42_101d_d674_7be9, 0x9deb_237a_87af_cd1a),
            (4500, 0x530b_584d_c08e_bf1c, 0x3618_d500_79e5_d413),
            (6000, 0xcc7d_1a69_2a3e_7f94, 0x6fb0_b1e7_f65e_820e),
            (7500, 0x2e2d_ffc9_16e4_e5b3, 0x3bc1_b31d_34e8_7779),
            (9000, 0x3777_5766_9283_5c0f, 0x8e20_1675_6fa9_1660),
            (10500, 0xb62c_b875_4ac2_f6a1, 0xae9d_5c8c_8303_58aa),
            (12000, 0xe2d2_b600_ab89_769f, 0x22c7_600f_d414_c6ad),
            (13500, 0x36f2_2847_aa0c_63d7, 0xe636_0f96_7211_20f6),
            (15000, 0xf757_0c54_5375_0b7a, 0x04c5_da2e_9e18_76e6),
            (16500, 0x6fad_3c57_7eaa_839d, 0xb184_bb59_a10b_7408),
            (18000, 0x5e1a_51be_bb09_a6ed, 0x3fca_883d_1db7_1c32),
        ],
        last: 0x6f17_79e2_3c68_e52f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x8a86_5f21_b4a2_51b8, 0xe5d0_142d_3141_2ee9),
            (3000, 0xc5a6_d611_a7a8_3602, 0xe478_0204_5228_6016),
            (4500, 0x964a_13f9_9a09_fae2, 0xa7be_c6e3_18f4_f5e2),
            (6000, 0xcd5b_2afe_542e_a9c9, 0xb7f4_1a7c_73ab_7e3c),
            (7500, 0x4f46_9a8d_3701_8abf, 0x1f62_6819_870f_a3f7),
            (9000, 0x0903_4cd3_7aae_5682, 0x3dc6_45ac_da12_c67d),
            (10500, 0xf40a_0cd6_e1a4_8a5a, 0x83a8_9633_4fc0_8a65),
            (12000, 0x6cb9_f0a8_6175_9cd8, 0x5dff_5687_2230_07e9),
            (13500, 0x6778_0e8b_36e5_643c, 0x8b65_a4bc_37b3_3c82),
            (15000, 0x9644_bd10_7564_5a9b, 0x5c7e_dbc9_dca7_f09e),
            (16500, 0x4fab_0f8e_dff2_b7ab, 0xe6be_d298_5169_cb74),
            (18000, 0xf6a4_df8c_303d_8691, 0x662d_9df0_36fc_f7a4),
        ],
        last: 0x0ae3_9cfd_82a2_56bf,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x3ec8_fce6_2c3a_eac5, 0xfa59_d36a_4486_15d7),
            (3000, 0xd58d_2ac2_9abb_70eb, 0x29c7_b459_03a7_6a70),
            (4500, 0x2df0_0f43_af59_bb0c, 0x85ec_662c_899a_229e),
            (6000, 0xf81a_3b62_588b_1aa3, 0xeac8_833c_57b1_cf5e),
            (7500, 0x672e_5f82_ba3a_cf34, 0xb133_4133_56b7_bc32),
            (9000, 0xb86d_8f78_4c9b_2200, 0x88d7_82c7_e172_9a65),
            (10500, 0xe67f_f8cf_6400_fdb4, 0x91ba_6d1c_ab80_b18f),
            (12000, 0xdf18_071c_b3a3_9c41, 0xd7d4_5f33_6f88_6315),
            (13500, 0xc693_2363_6369_44f9, 0x6f99_df5b_46a2_63be),
            (15000, 0x9d0d_1fb0_c6bb_a8d8, 0x3a48_1442_275e_e043),
            (16500, 0xf61d_373e_b387_c0ff, 0x1361_29a8_00fe_5011),
            (18000, 0x7348_9099_b22b_16a2, 0x0228_cdda_830e_bffc),
        ],
        last: 0xd459_08bd_a28c_707b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe44e_be41_4780_cc0e, 0xc932_7b5c_64af_f199),
            (3000, 0x7c96_b713_eab4_cd6f, 0xe733_0efd_f79e_48f7),
            (4500, 0xb877_f190_acde_9394, 0xd4b1_aa25_ac54_a349),
            (6000, 0x3c27_9f3f_c028_152a, 0x85e1_d085_54eb_3e32),
            (7500, 0x57f0_a57d_d31a_9e70, 0x33b0_19ce_cb65_e7e7),
            (9000, 0x322b_226d_d399_b636, 0xdc43_43ab_48f5_15d0),
            (10500, 0xbc79_081c_d02c_52fc, 0xc6c5_8b14_273a_5644),
            (12000, 0x9e3b_81fc_5d10_0c42, 0x786a_8cac_a15c_c29c),
            (13500, 0x443e_e5d4_1bfe_872b, 0x64a3_a677_fa30_ab79),
            (15000, 0xd9b9_1776_c501_6acb, 0x3f0e_3884_c27e_6b02),
            (16500, 0x3b76_ac4e_7db9_f294, 0x4836_47e0_3388_1932),
            (18000, 0x1bc0_b9af_a2c8_42f4, 0x1e2a_0d2e_3e5b_1b50),
        ],
        last: 0xe7a4_af88_b690_be4a,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7307_7c41_469d_0d92, 0xa4d7_ff1e_426d_65df),
            (3000, 0x578a_5a4b_e380_676c, 0x0f0b_198f_4a29_9cf8),
            (4500, 0xe9d7_d0ee_281f_bd61, 0xb6a7_1c04_6a28_5998),
            (6000, 0x5523_4693_ae44_addc, 0xe436_5e34_87b8_b2bc),
            (7500, 0xa802_ebb8_7cab_2bc5, 0xc891_9cec_a072_9b3c),
        ],
        last: 0xb957_1dd0_a199_37ec,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf9d3_782d_1ba2_5ba8, 0x8611_09a2_c237_6a57),
            (3000, 0x6972_da45_5887_7eb1, 0xb6ae_15a2_9d8f_fdc7),
            (4500, 0x15a3_7bb7_747c_5a4b, 0x1b0d_ec3d_a134_6749),
            (6000, 0x0481_1e69_6862_7a77, 0xb4f7_34da_41b4_242a),
            (7500, 0xabc2_7c25_5536_f187, 0x10ad_4d24_20e0_bb6f),
            (9000, 0xb17a_825d_2460_9ce5, 0x6534_34ba_d641_1694),
        ],
        last: 0xc0a1_0366_3c0f_0d5d,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xad7d_d755_0977_9af9, 0xfa38_a565_ad32_26a5),
            (3000, 0x5496_6fb7_bbbe_158d, 0x89df_1f9b_90b8_6549),
            (4500, 0xb192_c03d_e0e1_d202, 0x65bc_510c_4541_7269),
            (6000, 0x934f_e40c_4d9a_47ef, 0x1df7_d33c_69e6_5655),
            (7500, 0x6aeb_21da_b24d_03fd, 0xb503_1628_9ae2_c257),
            (9000, 0x6f2f_98ce_d70b_1962, 0xaf43_e9a9_b45e_989f),
            (10500, 0xaf09_27fd_17be_ea60, 0xb95b_2aba_ffbe_c143),
            (12000, 0x9bc7_325d_5da2_b938, 0x6c3e_d21e_a415_6028),
        ],
        last: 0xf09b_e18e_d0b8_ea8f,
    },
];
