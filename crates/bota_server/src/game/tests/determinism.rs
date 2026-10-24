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
            (1500, 0x1d0c_badb_a196_6bd9, 0xfa62_a1b0_1c54_e8ea),
            (3000, 0x14ef_4c33_69a0_9a15, 0x1468_1f2a_d57f_1891),
            (4500, 0x472b_083d_225d_2c9b, 0xc19d_6b6f_da37_54c1),
            (6000, 0xebf5_34a0_4dde_6ab7, 0x2f24_7e07_5e1d_e9cc),
            (7500, 0x0cf1_79ce_328a_dfda, 0x00f9_6ed1_5557_5f96),
            (9000, 0x41b9_ed99_bab3_8166, 0x411a_82ec_cabb_f0f9),
            (10500, 0xaa5c_8c13_cf25_1e99, 0xb710_ccaf_d593_b4a8),
            (12000, 0xef09_ec3f_7fb5_d437, 0x616f_bfbc_a66c_c089),
            (13500, 0x9b4d_dc29_f948_7943, 0xa0dd_1e32_efea_8865),
            (15000, 0x1feb_8408_d8e0_3bd2, 0x20ca_02c2_1fb9_d7de),
            (16500, 0x951d_f789_54c4_208d, 0x3d74_5a19_dd2f_da3c),
            (18000, 0xcc4e_e1bd_9734_f4d9, 0xd57f_ba59_4260_dfdf),
        ],
        last: 0x8e0f_1d18_e2d4_c85a,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xaa74_9783_cf35_5ef3, 0x4d04_5ddf_9484_ca5f),
            (3000, 0xca23_d97e_82bc_2ec7, 0x6e54_66dc_ccfd_722d),
            (4500, 0x140d_a34f_1efe_232f, 0xe3c1_6a47_cb8b_664f),
            (6000, 0x9fde_5124_cc8e_f577, 0x9bbc_3a03_98ca_f2b4),
            (7500, 0x3bb1_089d_3072_9364, 0x9a81_86f3_fb0c_c536),
            (9000, 0x1499_b0de_0849_fe0e, 0x8f3e_8941_2b12_f8b9),
            (10500, 0x8172_748f_7574_d466, 0x5cee_cdd9_8be4_47fb),
            (12000, 0x34fd_b580_40a6_23d5, 0xef65_424d_7dd4_78dc),
            (13500, 0xbc85_3223_abfb_2476, 0xb0e7_54ae_6d89_824d),
            (15000, 0x8a35_86f5_49ec_638b, 0x481f_03c9_cde1_4538),
            (16500, 0x215c_e63e_c2d4_a40f, 0x6860_b4f3_4f30_14f3),
            (18000, 0x9a4f_33eb_007a_df01, 0xa1c0_5256_41bd_e175),
        ],
        last: 0x5e97_49e6_788f_6269,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xa449_2868_aead_af5a, 0xb3ca_a2f1_0489_3cff),
            (3000, 0x3fdc_876d_60bf_5da8, 0x49ed_a222_eb2e_6e58),
            (4500, 0x6210_f234_ef6a_6ad4, 0xc189_3bd0_bd8b_f7b3),
            (6000, 0x2219_e201_04a0_1856, 0xc58e_bb7c_1ce6_7d12),
            (7500, 0xc6be_12bb_414c_6e7c, 0x2e26_a977_e54c_e8f5),
            (9000, 0x7f37_8a4b_3303_2262, 0xaa0f_7bfd_95f7_bda1),
            (10500, 0x5fad_0ed9_1c09_4b9c, 0xa86d_aa91_2cef_5477),
            (12000, 0x7309_e541_4c89_a558, 0xff8f_653b_6897_43e6),
            (13500, 0x5817_ff35_e2a7_b6a0, 0x0df4_bd23_a061_3924),
            (15000, 0x55bb_0754_2b78_9e05, 0x5a1c_077d_6519_3096),
            (16500, 0xa41f_ecfb_3009_c229, 0xb968_1c68_fd4a_b989),
            (18000, 0xc962_ce59_2435_eac9, 0xeec5_1513_38b3_e4f3),
        ],
        last: 0xaee3_e8db_c07f_0033,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x2837_aa78_241c_a0c3, 0x7e20_8851_a956_3201),
            (3000, 0x102c_6ac8_c0b5_3475, 0x49c8_2332_e0c3_5fd7),
            (4500, 0xd643_5d00_8e65_e431, 0xb8a7_1533_bbae_dcf1),
            (6000, 0xe22c_9b1a_14d7_b468, 0x1941_319b_c2a0_8d06),
            (7500, 0x6da7_b640_f380_d631, 0x9484_8cf2_c405_8c17),
            (9000, 0x53c4_e0b7_cae9_c0b8, 0x82de_60ce_4a3e_424f),
            (10500, 0x34ff_e6c6_3649_8eda, 0x12f8_fc13_8676_92b5),
            (12000, 0x9fda_4aa9_50a6_15f9, 0x628d_df34_a8d7_2bc2),
            (13500, 0x3c57_26ab_8eef_9dba, 0xf789_4c10_1778_243e),
            (15000, 0xf2d3_c18c_5e7b_3579, 0x09ab_b320_1db7_344f),
            (16500, 0xedab_288d_cdb3_20b8, 0x2ddf_af7c_9584_ca54),
            (18000, 0x0585_d994_6419_0d78, 0xfabc_7c25_c6ce_6cad),
        ],
        last: 0xa35f_e909_ddf2_05da,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xf160_3773_269f_743f, 0x009a_759e_2da6_1c69),
            (3000, 0xa51e_7748_baf1_82fa, 0x4653_eaf9_8f26_9488),
            (4500, 0xfe32_b70d_3760_1f4d, 0x33d3_c60d_8a60_7180),
            (6000, 0xe226_101d_94da_6393, 0x616f_dce8_a9d0_b977),
            (7500, 0x9050_9686_a305_326b, 0x7ec0_6d64_a5ac_e576),
            (9000, 0x5819_4a87_4607_4ec1, 0x601a_2016_8c0f_aca1),
            (10500, 0xcae1_969f_c7b6_fe13, 0x096d_910d_1e1b_26bd),
            (12000, 0x2749_ff69_506e_a446, 0xc5e7_1932_a177_4b82),
        ],
        last: 0xed74_2f05_7be5_0a35,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb45f_e0af_ac3a_d827, 0x34b3_f1ae_d4e5_ceb7),
            (3000, 0x1952_cd4d_c25b_0de2, 0x0521_5b17_2d79_8efe),
            (4500, 0xb7dc_0e4c_304b_fde2, 0x9af8_7e3c_abe2_2ee6),
            (6000, 0x3175_b6ce_2a00_610c, 0x6ede_1cba_55c9_3ecb),
            (7500, 0xfa50_ea69_b9b3_951b, 0x5931_c231_5676_0223),
            (9000, 0xd7cb_750c_433a_a9cc, 0x762d_7d6d_a9aa_2ac4),
        ],
        last: 0x287c_de68_63f4_c6e8,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x8e32_751b_b2c7_e8c2, 0x4cec_7881_ba1a_0e80),
            (3000, 0x3acf_791e_046c_9808, 0x9e00_65ab_f4d9_2c0d),
            (4500, 0xa179_53a8_de56_9199, 0x1a8d_fb0a_7dec_5e87),
            (6000, 0x2c74_fb4c_a0d6_3caa, 0xf4e6_1cec_ff3f_2e61),
            (7500, 0x5394_2e0f_7437_1d34, 0xb4fd_8674_9212_27c5),
            (9000, 0xee9b_0215_5d77_753f, 0x59fb_c5c4_c843_b382),
            (10500, 0xa064_849e_a815_dc87, 0xcd03_13de_7058_39cc),
        ],
        last: 0xd61f_c8c3_ad7d_0cb2,
    },
];
