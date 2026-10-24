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
                queue: false,
                seq: 0,
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
            (1500, 0x1d0c_badb_a196_6bd9, 0x6e67_708d_a325_6c75),
            (3000, 0x14ef_4c33_69a0_9a15, 0xd9fa_a9d4_9455_2e90),
            (4500, 0x472b_083d_225d_2c9b, 0x840b_2d12_e35f_fd4a),
            (6000, 0xebf5_34a0_4dde_6ab7, 0x6cf8_5d1c_d7c2_b66a),
            (7500, 0x0cf1_79ce_328a_dfda, 0x9067_4603_f6bd_2ecd),
            (9000, 0x41b9_ed99_bab3_8166, 0x2abf_7e89_08f2_7aca),
            (10500, 0xaa5c_8c13_cf25_1e99, 0x8e64_da10_548a_be2c),
            (12000, 0xef09_ec3f_7fb5_d437, 0x4ea3_0b4e_1f84_82d9),
            (13500, 0x9b4d_dc29_f948_7943, 0xf83f_dbe7_ae20_8174),
            (15000, 0x1feb_8408_d8e0_3bd2, 0x7fb3_a6a7_4d71_b90d),
            (16500, 0x951d_f789_54c4_208d, 0x9d68_1aa0_2f73_1f68),
            (18000, 0xcc4e_e1bd_9734_f4d9, 0xe6ca_e2d8_9e06_ff42),
        ],
        last: 0xa12e_147a_1838_4574,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xaa74_9783_cf35_5ef3, 0x52da_9cbe_c208_88e4),
            (3000, 0xca23_d97e_82bc_2ec7, 0x09d1_63c5_d5c6_17c8),
            (4500, 0x140d_a34f_1efe_232f, 0xd486_df0c_ed7a_511e),
            (6000, 0x9fde_5124_cc8e_f577, 0xbcbe_b871_9f7f_6646),
            (7500, 0x3bb1_089d_3072_9364, 0x198c_9b1c_835c_107e),
            (9000, 0x1499_b0de_0849_fe0e, 0x9e29_b4f3_f59d_540c),
            (10500, 0x8172_748f_7574_d466, 0xaa67_2bf2_6402_b5e3),
            (12000, 0x34fd_b580_40a6_23d5, 0x8e0e_55c9_9bc8_b3f2),
            (13500, 0xbc85_3223_abfb_2476, 0xd86f_d42c_8cb6_1742),
            (15000, 0x8a35_86f5_49ec_638b, 0xac9d_1cd9_5ce8_40eb),
            (16500, 0x215c_e63e_c2d4_a40f, 0x0190_a730_a3dc_e861),
            (18000, 0x9a4f_33eb_007a_df01, 0x461c_e100_80ce_19a0),
        ],
        last: 0xe5dc_29a3_3bf7_7e3a,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xa449_2868_aead_af5a, 0x8da9_f033_0c2a_a5ca),
            (3000, 0x3fdc_876d_60bf_5da8, 0x2a3a_3f06_b342_7685),
            (4500, 0x6210_f234_ef6a_6ad4, 0x85af_fb16_8584_cf7f),
            (6000, 0x2219_e201_04a0_1856, 0x8d14_9d73_1078_6990),
            (7500, 0xc6be_12bb_414c_6e7c, 0x1563_901c_53ed_7c6b),
            (9000, 0x7f37_8a4b_3303_2262, 0x19d2_138b_f4f9_50cf),
            (10500, 0x5fad_0ed9_1c09_4b9c, 0xc3ca_be48_0173_65db),
            (12000, 0x7309_e541_4c89_a558, 0xb724_ffdc_dce4_136a),
            (13500, 0x5817_ff35_e2a7_b6a0, 0x15fd_4f00_bd52_e03d),
            (15000, 0x55bb_0754_2b78_9e05, 0x523b_9eb0_9fc6_5c1d),
            (16500, 0xa41f_ecfb_3009_c229, 0x5103_b56d_d966_b306),
            (18000, 0xc962_ce59_2435_eac9, 0xc3c8_e0bd_1060_920f),
        ],
        last: 0xa061_f178_5c88_f9b5,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2837_aa78_241c_a0c3, 0x039b_19e3_a417_00b3),
            (3000, 0x102c_6ac8_c0b5_3475, 0x702a_f14c_a214_65e5),
            (4500, 0xd643_5d00_8e65_e431, 0xc86d_3fee_7a0d_8911),
            (6000, 0xe22c_9b1a_14d7_b468, 0xde8c_0d70_5aca_e524),
            (7500, 0x6da7_b640_f380_d631, 0xcb45_038b_95cc_e743),
            (9000, 0x53c4_e0b7_cae9_c0b8, 0x994f_28fb_aa48_37d4),
            (10500, 0x34ff_e6c6_3649_8eda, 0x5ca3_d28d_abef_2947),
            (12000, 0x9fda_4aa9_50a6_15f9, 0xe1db_eaf3_a614_e89b),
            (13500, 0x3c57_26ab_8eef_9dba, 0x8f1b_8f65_b24e_67e6),
            (15000, 0xf2d3_c18c_5e7b_3579, 0xcf6a_1e58_5b8c_a4fa),
            (16500, 0xedab_288d_cdb3_20b8, 0x20ac_8817_5c75_5244),
            (18000, 0x0585_d994_6419_0d78, 0x107d_3d1a_7628_32c6),
        ],
        last: 0x955b_b4ef_d803_2ea2,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf160_3773_269f_743f, 0x7d4c_437b_feb2_5eee),
            (3000, 0xa51e_7748_baf1_82fa, 0x7f51_824a_c6f5_bbfc),
            (4500, 0xfe32_b70d_3760_1f4d, 0x0826_2b27_f070_2dd2),
            (6000, 0xe226_101d_94da_6393, 0xe13f_2554_fe6a_5fdd),
            (7500, 0x9050_9686_a305_326b, 0x2734_3ea6_12b6_fad8),
            (9000, 0x5819_4a87_4607_4ec1, 0xf3ec_fde7_a94b_8948),
            (10500, 0xcae1_969f_c7b6_fe13, 0x6b29_e998_f7cb_e659),
            (12000, 0x2749_ff69_506e_a446, 0x5591_c147_e5da_2ff9),
        ],
        last: 0xe1de_55c1_8033_bc85,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb45f_e0af_ac3a_d827, 0xc71a_53d2_f239_fe78),
            (3000, 0x1952_cd4d_c25b_0de2, 0xb370_a25c_8d85_9abd),
            (4500, 0xb7dc_0e4c_304b_fde2, 0xf70d_2d11_6dcc_3268),
            (6000, 0x3175_b6ce_2a00_610c, 0x5363_376e_f0c8_46bc),
            (7500, 0xfa50_ea69_b9b3_951b, 0x4225_124b_ac2d_762d),
            (9000, 0xd7cb_750c_433a_a9cc, 0x37e1_e591_15aa_c2cc),
        ],
        last: 0x2928_29ff_d156_bc53,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x8e32_751b_b2c7_e8c2, 0xf33b_4c4c_bba7_7696),
            (3000, 0x3acf_791e_046c_9808, 0x1779_6409_28a6_8c4a),
            (4500, 0xa179_53a8_de56_9199, 0xb141_02d2_1e7e_1853),
            (6000, 0x2c74_fb4c_a0d6_3caa, 0x4ee0_bb60_5155_ec64),
            (7500, 0x5394_2e0f_7437_1d34, 0xfb42_d4c4_ce85_f7d9),
            (9000, 0xee9b_0215_5d77_753f, 0x4b48_8b78_0e52_00b2),
            (10500, 0xa064_849e_a815_dc87, 0xd031_6493_fada_4b0a),
        ],
        last: 0xb9c7_7486_f652_386e,
    },
];
