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
            (1500, 0xed65_f4c6_b733_e69d, 0x8ac9_3c94_f9b1_4374),
            (3000, 0xfb5d_8887_24c3_19b7, 0x8faa_6a5b_7613_361f),
            (4500, 0xde64_5eec_4581_cc46, 0xd64f_c279_c1c3_a813),
            (6000, 0xcc1e_21a5_7b66_13ae, 0x8764_5c92_9d8b_8f1a),
            (7500, 0xc97e_c516_fc07_6c9b, 0xace3_3361_d0c8_a833),
            (9000, 0x4a43_a6e6_e1b3_69f2, 0xf2c6_59d2_b306_1ab9),
            (10500, 0x6013_2f5c_50bf_c628, 0xecca_d459_ef11_cfd7),
            (12000, 0x887a_54db_58fe_abd3, 0xe20a_8dff_ede0_129e),
            (13500, 0x6dfe_4d90_aaab_0852, 0x77da_8ad3_f7b0_ab74),
            (15000, 0x5cb1_f86e_ed3d_4151, 0xc717_6a38_1a3c_8378),
            (16500, 0x7b95_c939_5b6f_361a, 0x3e26_b96c_5a02_4132),
            (18000, 0xe34f_10cd_d854_f812, 0x91e6_1f20_ca08_68b9),
        ],
        last: 0xebe6_0407_c79c_eb48,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x796e_50df_de83_d85e, 0xf49b_aba6_c1a1_9977),
            (3000, 0xa30b_2cc3_a8ca_e526, 0x2a0b_6b5c_a2a2_25e2),
            (4500, 0x440d_8a8a_b403_2ebf, 0xf86e_3d01_9a7f_09d9),
            (6000, 0xc80c_0ee3_4db5_b897, 0xa8a6_b337_fc76_3fd8),
            (7500, 0x7a37_1429_32fd_84d7, 0x5520_99ce_254c_aa1e),
            (9000, 0xce4b_ac1a_ae26_1c80, 0x2e49_8bb3_adde_d790),
            (10500, 0x21d8_c17d_59ea_0133, 0x5400_9189_7647_e465),
            (12000, 0xb8d9_2203_0708_19d0, 0x3d68_f219_0b92_78df),
            (13500, 0x193b_38a9_b54e_09ae, 0x9959_99d4_b1f0_53bd),
            (15000, 0xf68b_896e_1f52_60ca, 0xef40_21fe_bc51_a81f),
            (16500, 0x4f16_05da_aae5_29f2, 0x5913_f41f_0a76_6ad9),
            (18000, 0x1142_9fce_8b0f_e31d, 0x2c84_c3b4_a73f_6b1d),
        ],
        last: 0xf4d6_f479_d54e_35aa,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd540_59bb_5cf4_32ef, 0xc9f2_4dc3_d343_41ae),
            (3000, 0x7be4_3b76_f874_9b40, 0x6c38_cfaa_f227_e9a4),
            (4500, 0x0562_9fb8_0a5c_f8f5, 0x638e_31ef_1e0e_2933),
            (6000, 0x15e5_045f_44d3_809b, 0x1269_314b_a537_3efc),
            (7500, 0x7246_2775_bf8d_efa6, 0x28f5_32e0_cfbb_a2c6),
            (9000, 0x469c_2540_d1d4_3d69, 0x7ff0_6536_82df_c584),
            (10500, 0xaa0b_f8f2_8b4e_9fed, 0x9ad6_c621_499e_cdc9),
            (12000, 0x3a1f_1ce4_e140_7473, 0x08ea_6a09_ed89_d4e6),
            (13500, 0x74b0_574f_93c9_8d56, 0xa073_991f_d020_b3c9),
            (15000, 0x94f8_4e2e_2ac0_fec5, 0x4d55_e735_45f0_7989),
            (16500, 0xd093_b1e8_3e0d_fd49, 0xd3b1_25d1_fdba_4802),
            (18000, 0x44b0_6cef_d6b7_bb68, 0x7b15_e658_fe64_6047),
        ],
        last: 0xf721_6a70_0d6d_0ef0,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x0046_baa4_9698_0735, 0x6f8f_ab74_4c38_b3c6),
            (3000, 0xbbf0_f2a6_4dcb_c187, 0x8bf6_ec4d_5b61_2c32),
            (4500, 0x676b_dec0_0d94_95f7, 0x93e3_f4dc_557a_7dfd),
            (6000, 0x8a02_a4a4_2a10_654b, 0xfba0_c007_4d66_536e),
            (7500, 0xa386_5358_979d_49d5, 0x0118_3d3e_441a_2d01),
            (9000, 0x5d58_9b89_62da_3917, 0x3400_7ba3_3d64_480d),
            (10500, 0x6200_e25d_623c_f94c, 0x2dea_c2b8_f188_8d26),
            (12000, 0x25cd_28cf_5371_dc86, 0xf7e2_8866_8c29_2788),
            (13500, 0x6116_85d0_e907_8016, 0x4e83_a09f_89ed_8897),
            (15000, 0x0209_db49_5bd5_df7f, 0x111c_6645_616a_742f),
            (16500, 0x9767_5876_228f_9c63, 0xb897_3a65_8a5a_6603),
            (18000, 0x48e4_177b_a06d_b6e2, 0xe550_c84a_fd3f_1c59),
        ],
        last: 0xcbff_1627_6e2e_1c84,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xe2bc_7160_ca63_fde2, 0x52ad_8bd4_9c1c_594f),
            (3000, 0x24d1_8fb2_6eb9_d4dd, 0x77fe_f219_f843_c860),
            (4500, 0xd31c_1712_6e0c_c356, 0x05d0_026f_6490_633d),
            (6000, 0xcb97_cef9_47d3_7843, 0xc805_4063_be05_6417),
            (7500, 0x2889_323c_bae2_c91d, 0x0da0_459c_6e60_81c0),
            (9000, 0x8c19_26ef_90c6_82e7, 0x2641_39f6_f994_13d4),
        ],
        last: 0xb238_cca2_c0aa_9326,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x41c3_df51_a0a7_48de, 0x01b5_4ffb_b0b7_f221),
            (3000, 0xde55_cc8f_f83c_5e55, 0x791e_5e42_ffc2_4851),
            (4500, 0xa8e8_26de_9e18_420b, 0x1d25_992f_e95e_17c9),
            (6000, 0x3d43_1263_9bde_8d03, 0x2b3f_582f_b2f5_cfc7),
            (7500, 0x649d_2935_a43f_0415, 0x9c74_8c9b_2597_5d81),
        ],
        last: 0x3985_1e4e_60c6_4c40,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x343a_7417_5433_f06f, 0x83d8_c666_89cd_0522),
            (3000, 0x5f82_8941_5e04_965d, 0x1ea3_ef00_d5f5_5aa6),
            (4500, 0x186a_e811_b3ee_570c, 0x4ae3_fd14_1036_0885),
            (6000, 0x635c_8344_4d95_9ed9, 0x0430_cd50_8687_9904),
            (7500, 0x7adf_827d_0eda_36db, 0x5c80_c4e6_0da4_b5d4),
            (9000, 0xde33_5c9f_c0f3_a592, 0x3610_a5f8_9bed_b752),
        ],
        last: 0x703f_5cdf_e553_dd78,
    },
];
