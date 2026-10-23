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
            (1500, 0xf27f_1eb9_05a9_9e15, 0xbb37_85f9_b3af_a845),
            (3000, 0x5083_90e6_300b_b524, 0xeef3_7348_01ed_999a),
            (4500, 0xcfb3_c737_ead1_ffa1, 0xfde2_74c2_5f56_7180),
            (6000, 0x179a_575c_6407_a1fb, 0x13d9_0b50_85f7_37de),
            (7500, 0x1e78_c262_64ed_5054, 0xff88_c569_9336_8c9b),
            (9000, 0xcf54_8c57_d582_24bd, 0xf295_dc55_c483_4076),
            (10500, 0xee59_90ef_8ed9_58ed, 0xa269_2301_d0f8_0fb6),
            (12000, 0x7bd6_d528_1ae5_7cb1, 0x523c_1239_8065_11e5),
            (13500, 0xd2b8_3377_49bd_e6d5, 0xcb29_bca6_55d0_fc38),
            (15000, 0x5fc8_2a4a_30e8_5ed9, 0x19d2_a744_2341_df72),
            (16500, 0x7f4d_c8e9_b94a_6744, 0x1ae5_a598_fe4e_9d09),
            (18000, 0x36e7_9acf_e4c3_ba0e, 0x8027_a904_5a55_9e39),
        ],
        last: 0x2b37_3d79_f8ce_404f,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x5964_4352_8507_fc46, 0x7c0b_8225_deaf_bf56),
            (3000, 0xea12_c88b_b165_ae51, 0x353d_696a_c82f_85c1),
            (4500, 0xa6a4_8cb9_7090_b79a, 0xb0c0_508a_d0e4_4392),
            (6000, 0x1d7e_6a73_59f8_bbcf, 0x8468_614d_e572_17ef),
            (7500, 0xf138_5b85_3fa1_f90e, 0xda69_696c_66f3_8207),
            (9000, 0x3fb6_bfc4_71d9_3274, 0x80e3_0bf1_265b_3c38),
            (10500, 0x1566_8651_822d_3a3b, 0x9c36_529a_fd40_1a67),
            (12000, 0xb306_ab81_0b0a_e9dd, 0x5f30_c76e_bbcf_8759),
            (13500, 0x35e5_1c21_e6e9_307f, 0x6b34_4de0_f5b1_dc4e),
            (15000, 0xc8cb_db80_9666_115d, 0xa27d_3798_bcd5_e07a),
            (16500, 0xe2ea_9faf_35d1_6247, 0x94de_6125_8fe8_0ee3),
            (18000, 0xd584_b3c9_d4c5_872e, 0x5fb8_ebf5_28cd_475b),
        ],
        last: 0x9eff_b10f_e99b_46af,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x676a_c19e_2e0e_f2fb, 0x471f_edd0_aff7_d647),
            (3000, 0xec0b_70bc_413f_68ce, 0x9dfa_3692_f2c0_908b),
            (4500, 0xb854_1427_961f_640f, 0x455c_3892_771f_4493),
            (6000, 0xc7ef_5536_7abd_35d0, 0x2b68_9b7c_a66f_6dd5),
            (7500, 0x24b8_e585_9aae_7664, 0xfe37_2bb2_74d3_3d0d),
            (9000, 0x7a78_db27_296a_9ca8, 0x97d7_7b77_87a4_4f2e),
            (10500, 0x2323_f127_867e_d96c, 0x8179_75fb_f0f3_852d),
            (12000, 0x49e4_209c_9841_4b8f, 0x9439_f0be_915c_5cf6),
            (13500, 0x92c2_6478_8d0a_606d, 0x5dd1_7092_79fc_2d26),
            (15000, 0xd21b_f7b6_c6e1_7cc7, 0xd8a6_b518_1212_5747),
            (16500, 0xe326_bf36_00f2_00ac, 0xbb65_d7c0_bd64_d44c),
            (18000, 0x5423_3cf6_6aa4_e5f1, 0x60bb_aab6_f777_adb0),
        ],
        last: 0x3ccd_5aa2_2a43_c348,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x0192_25b5_c460_f0a1, 0x92d9_9768_157d_039c),
            (3000, 0x1420_49a8_b89d_b47f, 0x4e6b_6afb_fe26_4e0f),
            (4500, 0xb18a_dc67_aacd_7f5f, 0xe7e5_65a4_2269_c246),
            (6000, 0x4629_a3c3_8c8f_75f1, 0x794d_3748_0a04_8273),
            (7500, 0xa5ae_55f1_69da_27a0, 0x5f47_1329_de2e_ee76),
            (9000, 0xb303_71bb_daa4_b2da, 0x3517_5da4_9d2f_2909),
            (10500, 0x1fd8_6883_b0c7_8618, 0xe41a_4d41_ee3e_7069),
            (12000, 0x1251_5e2c_818f_b55f, 0xfe0d_26c4_ee93_ee70),
            (13500, 0x31f7_77ff_f16a_3551, 0xec13_81bd_aa8c_01f4),
            (15000, 0x197e_82fe_7d85_5c0c, 0x2865_5d91_e626_059f),
            (16500, 0x7012_e57b_17ae_7bbe, 0xb135_e586_3531_a6e5),
            (18000, 0xcb7f_5747_7d0c_c9e2, 0xcf39_024f_f115_cbdd),
        ],
        last: 0xe10f_23ad_9f1a_bf92,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7047_2c96_1f81_5434, 0x0c65_6338_0b8f_6c94),
            (3000, 0x7fdf_3d65_0714_80ba, 0x7cf6_201e_13cc_2946),
            (4500, 0x7e43_b4e8_a2f1_831e, 0x531c_fd80_a8fe_1fb7),
            (6000, 0x3031_9591_7f1b_6fdd, 0x289f_afab_30b4_ebf8),
            (7500, 0xc4a2_121a_4405_39cb, 0xf078_c2aa_1aed_b570),
        ],
        last: 0x1ce0_73fc_157e_bb4c,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6867_f8c6_0fb1_a2ca, 0xda0d_2041_fb16_c6af),
            (3000, 0x7cba_da61_eac5_541b, 0xe72e_48da_9a63_bb45),
            (4500, 0x172a_6f6f_4717_2eff, 0x2c82_49b8_4bca_c9f2),
            (6000, 0xb3a7_29ba_979d_fcea, 0xe821_931c_4d7f_050f),
            (7500, 0xdb03_3e42_80fe_a4f1, 0x9a08_4663_ecd0_a6db),
            (9000, 0x7909_4bc2_a532_0e37, 0x1e2e_6b7d_6d45_78a7),
            (10500, 0x14d3_b3b0_aa22_e22a, 0xf3a5_d337_365d_e09e),
        ],
        last: 0x3865_6d43_971c_3bb3,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x5a51_896d_6428_d723, 0x5b53_fe57_e8d6_bd3a),
            (3000, 0x2c72_d0b1_3e26_4ef6, 0xfe82_b67c_4d93_e7ed),
            (4500, 0xd2de_6d3b_3969_36bb, 0xd9fb_cbb5_3f47_4ce0),
            (6000, 0xa098_4c68_8fa6_155b, 0xebd2_a2b8_b666_c9fc),
            (7500, 0x6714_c293_6c99_ddf3, 0x3ae6_1cdb_4d37_0571),
            (9000, 0x6f41_d73e_50e5_c46c, 0x9c44_3c2a_7ade_0825),
            (10500, 0x8be6_0e1f_e775_e7cd, 0xfad1_e03c_d6a9_ef7e),
        ],
        last: 0x6acb_700f_3e60_cf63,
    },
];
