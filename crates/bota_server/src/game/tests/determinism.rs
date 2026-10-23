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
            (1500, 0x0e4b_c445_52ea_675b, 0x1bdf_922c_1bbe_6a05),
            (3000, 0x4fab_3c3c_d130_5a05, 0x62bd_5c0a_72f6_78ef),
            (4500, 0x19b3_93e7_a6d3_c450, 0xc10c_16f7_8f67_a44d),
            (6000, 0xb96f_ea59_be74_2a92, 0x83ab_5333_ad23_1a55),
            (7500, 0xd619_a99c_cf8c_c821, 0x7239_d570_d558_c50a),
            (9000, 0xe2b1_8db9_80f8_a902, 0xc1de_6a79_dbcf_5980),
            (10500, 0xb654_bfc6_3e4d_866e, 0xc2d6_b20e_5efe_a30e),
            (12000, 0xf238_e3df_f4ab_b887, 0xc04c_c4f8_e2c5_e0f7),
            (13500, 0xb714_6dd1_d976_7d36, 0xf324_a934_390e_0a6e),
            (15000, 0x29ed_2a91_8fe3_19e5, 0xfecb_f891_6e0f_c8da),
            (16500, 0x07ef_c7b8_9f23_4b9a, 0xc8b1_0e14_d1b6_f7d0),
            (18000, 0xcbf6_53b8_166c_4ede, 0xebef_f5bd_9594_bb2b),
        ],
        last: 0x5c58_56d5_ddf2_f5f3,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xffde_f7d3_ed1b_0bc0, 0xc413_b496_cb19_925d),
            (3000, 0x2365_ecb7_71b2_d5cc, 0x8db2_17a0_9edc_0c18),
            (4500, 0xec86_6ead_cecd_3507, 0xbaa4_fc20_cc16_081a),
            (6000, 0x18ef_d910_1942_802d, 0xe1f6_7626_c81b_7deb),
            (7500, 0x7c49_9b16_d7af_e11b, 0x8df3_4adc_1e23_ef49),
            (9000, 0x9307_d161_8abf_cb88, 0xff1a_7896_eb2f_dead),
            (10500, 0x9e1c_64f0_0cb8_1183, 0xe599_d085_0401_b662),
            (12000, 0x5569_9d7d_ecf3_6250, 0x4394_459d_be90_b08d),
            (13500, 0x388c_c6a6_0625_111c, 0xfd31_48bd_02fb_606a),
            (15000, 0x7791_6ed5_a876_8f44, 0x170a_4ca6_c971_ba33),
            (16500, 0x2d4c_a6aa_db8a_210c, 0x722b_c0c2_9ba2_46ce),
            (18000, 0xcbf7_9004_1bf5_e1ef, 0xc7ff_307b_1018_e333),
        ],
        last: 0x87e4_3988_c6fb_0b64,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xd540_59bb_5cf4_32ef, 0x8ceb_92b7_8313_eeec),
            (3000, 0x7be4_3b76_f874_9b40, 0x47ec_a579_3e69_186d),
            (4500, 0x0562_9fb8_0a5c_f8f5, 0x3cde_8a75_9391_5456),
            (6000, 0x15e5_045f_44d3_809b, 0x89ee_49cd_08ff_00c1),
            (7500, 0x7246_2775_bf8d_efa6, 0x1cb9_6c5c_4945_4c1e),
            (9000, 0x469c_2540_d1d4_3d69, 0xd2a6_93df_63dd_dbb3),
            (10500, 0xaa0b_f8f2_8b4e_9fed, 0xd536_ba9d_e384_6dd9),
            (12000, 0x3a1f_1ce4_e140_7473, 0x6738_4704_ade4_3fbc),
            (13500, 0xc1c9_953c_eed1_2949, 0x9218_a391_0b42_2e5b),
            (15000, 0x716e_9d3c_615b_8d49, 0x5939_9f0d_620e_97ca),
            (16500, 0x8c3a_d2f9_8da9_d7c3, 0x1061_1c9d_004a_47bf),
            (18000, 0xa563_5337_9b3b_2a6f, 0x5231_c251_d297_6958),
        ],
        last: 0x740a_7cad_fed4_069b,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x0046_baa4_9698_0735, 0x16b7_db0a_a920_b077),
            (3000, 0xbbf0_f2a6_4dcb_c187, 0xb569_17d8_69d4_e0a1),
            (4500, 0x676b_dec0_0d94_95f7, 0x7ef7_e23a_1a65_65b8),
            (6000, 0x8a02_a4a4_2a10_654b, 0xe64f_9a72_b900_9cb4),
            (7500, 0xa386_5358_979d_49d5, 0x72ad_f0ca_899c_2cfc),
            (9000, 0x5d58_9b89_62da_3917, 0x2b71_3e17_9d10_c459),
            (10500, 0x6200_e25d_623c_f94c, 0x3254_a458_231d_fecf),
            (12000, 0x25cd_28cf_5371_dc86, 0xc5b1_458b_83a6_3698),
            (13500, 0x6116_85d0_e907_8016, 0xa9d0_b119_7454_9fdc),
            (15000, 0x0209_db49_5bd5_df7f, 0x3687_8897_0600_2bfb),
            (16500, 0x9767_5876_228f_9c63, 0x062c_af05_fbce_eb6f),
            (18000, 0x48e4_177b_a06d_b6e2, 0x2cad_fa8d_ad18_3435),
        ],
        last: 0x1bb1_d190_9ddd_7328,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xde74_8046_3bea_a994, 0x5598_646d_9446_6c8f),
            (3000, 0x4992_b86b_50f8_ec25, 0x251c_f7b9_82e8_c4a8),
            (4500, 0x9abd_c6f5_19f6_cfaa, 0x67fe_3fcd_66aa_3716),
            (6000, 0x7505_6049_775f_a7ff, 0xec2d_1ea1_37ba_6893),
            (7500, 0x3565_f719_62fe_69e9, 0x0146_b714_b8b9_4457),
            (9000, 0x5822_e18c_f142_7a25, 0x9023_b532_dd96_8eab),
        ],
        last: 0xbd6f_f1dc_e8d4_742a,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xd06f_4630_ac28_b468, 0x292d_9053_4d11_644f),
            (3000, 0xa6b8_d134_3344_ae51, 0x99f7_f3d4_4921_d7bf),
            (4500, 0x9f43_ed1c_2ca0_31a3, 0xc55c_ae22_53c6_c8ec),
            (6000, 0x5269_8052_e410_3c4f, 0x1cc6_96e2_297a_70af),
            (7500, 0xe0a3_8fe4_df3d_ef4d, 0x4185_9731_c002_bd2e),
        ],
        last: 0xac42_79f0_3913_aaf6,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xe9a8_28aa_d40a_c19d, 0x2614_0d43_7cb7_d890),
            (3000, 0xea6f_bcf0_24a6_e793, 0x47d0_9129_8a38_afb4),
            (4500, 0x0a7a_75b8_970e_c798, 0xdb21_5438_f6b9_eb8d),
            (6000, 0x1489_f125_0c1a_6717, 0x9a6a_6e45_47f8_a852),
            (7500, 0x1f83_7989_25e2_b3e9, 0xe9d9_ea75_953a_4b7f),
            (9000, 0x1b55_5cc7_3571_5cb7, 0x85c9_df3c_8e4d_15a7),
        ],
        last: 0xc7e4_c239_f648_cb3b,
    },
];
