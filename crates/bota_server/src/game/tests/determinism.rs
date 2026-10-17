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
            (1500, 0xe705_b592_ec70_737c, 0x8a4d_7eaa_4c48_fcfc),
            (3000, 0x5a40_0bbc_b752_6120, 0xa84c_09d8_ac58_d18a),
            (4500, 0x04fb_ab3c_d91e_8e04, 0xc2c1_ab1d_7279_84f9),
            (6000, 0x7dfb_4981_430d_6301, 0xf477_67bd_4403_4a5d),
            (7500, 0x3e22_4a56_beac_cf5c, 0x8241_368e_3bc9_adce),
            (9000, 0x9336_c31d_0f7c_15a4, 0xcce8_83eb_6777_6823),
            (10500, 0x948c_33be_cca6_ce77, 0x4d33_4d52_694f_ce12),
            (12000, 0x89e8_5de3_605b_d4ee, 0xf972_a654_5946_0446),
            (13500, 0xa50a_aac6_666c_255f, 0x54fc_628a_3294_bfae),
            (15000, 0x1e96_1b09_2d49_00bd, 0x16ea_a5cd_a2cd_4b7e),
            (16500, 0x2791_564d_a724_fb72, 0x6103_511e_0889_ed3b),
            (18000, 0x3c4e_5974_f339_b57b, 0xe0d7_5890_337e_124c),
        ],
        last: 0xaa01_44ae_4b02_feda,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x6ecd_3ed9_4f34_6998, 0x94d8_0013_65d0_8d6a),
            (3000, 0xdef6_e2c1_8d38_b794, 0x2791_9610_abd9_821d),
            (4500, 0x887e_0bf9_f67c_39ee, 0x7931_0b32_fdd9_8981),
            (6000, 0xbf48_4d10_c542_8a9b, 0x7e92_3734_f4a5_c626),
            (7500, 0x2ef3_3a65_5ae6_7876, 0x9343_bffd_f419_10bb),
            (9000, 0x46ae_3340_930f_c666, 0xe572_0d9d_80ba_a05b),
            (10500, 0x3314_b846_10c6_a232, 0xa253_0d62_8be1_819a),
            (12000, 0x2bff_2991_d791_fa48, 0x802c_31b3_2bbc_1df6),
            (13500, 0x1b9f_29d7_3241_77a1, 0xfecf_fdb0_9214_3dc6),
            (15000, 0x285b_51ef_aa9d_1f50, 0xac74_c443_0ef2_70f0),
            (16500, 0xc2fc_d570_8c52_3b52, 0x9819_fa37_7416_12b6),
            (18000, 0x56de_1aad_f9cf_16af, 0xd851_fdc2_018e_d3e8),
        ],
        last: 0xcb0e_68b4_dd62_39e4,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xfe3a_3a96_9bd7_2888, 0xa789_aa33_2d70_c446),
            (3000, 0x1d7a_61e1_07d8_9061, 0xf59e_40b5_ae6a_ef4a),
            (4500, 0x67a3_3ba9_7e32_9840, 0x063b_67f3_f7db_10ba),
            (6000, 0xed6a_e46c_8f90_93f4, 0xdf68_ffef_a945_01ed),
            (7500, 0x872d_dec6_1d14_1461, 0x672a_a887_d3f6_c922),
            (9000, 0x7ec3_570c_a606_c4bb, 0xfef6_487c_b483_bb3b),
            (10500, 0xfca6_e130_a3b8_c275, 0xbb6c_3081_3aa2_b5c5),
            (12000, 0x1a2d_6d71_877f_8ef2, 0xd509_36a1_dc25_8ad4),
            (13500, 0x0193_724f_de9c_bd44, 0xbb82_d41e_c2bf_3cf4),
            (15000, 0x493a_cd4a_0196_5aca, 0xeffe_3141_6cb4_7df0),
            (16500, 0x1efa_f060_8aeb_dd29, 0xd9a5_6565_5403_3910),
            (18000, 0xbae4_cecd_8507_7039, 0x99ad_3829_0b30_a457),
        ],
        last: 0x53c8_0601_8104_bb0d,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xf62b_826c_101a_adce, 0xd41d_dcc3_f07c_2b6a),
            (3000, 0x4fbe_4011_2a1a_fccf, 0xcfa7_32a4_e06a_8a29),
            (4500, 0x8bc9_4d98_ef49_72d0, 0x7f4e_537f_cb53_02e7),
            (6000, 0x29cf_0d03_3d0b_8066, 0x818a_d873_a5d0_e360),
            (7500, 0xc62c_5912_1478_67b7, 0x4c96_1775_33b9_457e),
            (9000, 0xc5cc_b05f_41b8_9c4a, 0xb36f_8376_6dbd_f087),
            (10500, 0x7a00_177d_2101_9a4c, 0x9215_b443_f51b_bb0b),
            (12000, 0xe4a5_d3b8_b405_85b4, 0x28a3_5d98_fee7_08ba),
            (13500, 0x762e_4768_99a4_dcb3, 0x10ab_d4d2_1bad_d46c),
            (15000, 0x87b3_f160_58a1_6701, 0xd82a_bdc6_c1d4_7dc6),
            (16500, 0xfd0b_7fd9_c40b_2a73, 0x8942_0f66_f0a9_ff06),
            (18000, 0xb39d_8d7c_8f55_a268, 0xe783_fd28_dc2f_c3d8),
        ],
        last: 0x92c3_805d_4617_05e6,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbbf4_4428_a326_7908, 0x1d8d_edc2_2baf_3485),
            (3000, 0x59cf_a13d_dc3a_ad46, 0x1923_be75_242d_3e2f),
            (4500, 0x3274_24cf_2244_2de7, 0x56c4_cfea_dc22_e8c2),
            (6000, 0x7338_7b9c_6832_26d7, 0x6596_aab3_fc70_9fa4),
            (7500, 0xb083_e6ab_8218_fc26, 0xf33d_9474_e976_1142),
            (9000, 0xdf9d_19c5_a390_bfea, 0x3d6c_ad89_5ad8_2f52),
        ],
        last: 0x5cbf_2fd3_bdf3_a4e8,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x140f_f1fd_c7f7_284f, 0x3cf9_0304_5707_76ad),
            (3000, 0x4635_90b0_913c_2912, 0x30c1_8459_0186_b123),
            (4500, 0x997d_5d76_cfa1_ca9e, 0x82c6_a597_edee_9aac),
            (6000, 0xe261_c5f8_166a_9f13, 0xac51_daea_84be_2269),
        ],
        last: 0xcce3_5e39_bb72_68d2,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xa82a_3ac9_224d_0a89, 0x4177_1cb0_2e20_643b),
            (3000, 0x599d_610a_fd69_cafb, 0x3db3_7631_d4bb_80a5),
            (4500, 0x668d_0ac8_5923_5786, 0x10d8_4599_b243_6c3c),
            (6000, 0x5c23_b1f0_1044_6fa1, 0xbb5d_0022_4265_7790),
        ],
        last: 0x24e3_b6cc_70df_0753,
    },
];
