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
    assert_eq!(MatchSecrets::seeded(SEED), first, "a seed plays its match again");
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
            (1500, 0xfdd8_670c_7b88_01c2, 0x1377_e90d_e54e_8fba),
            (3000, 0x2ef1_79b1_a500_4089, 0x4854_0f08_8482_2f2c),
            (4500, 0x2a3d_ed6a_1d08_fc8b, 0x4eea_a6a4_4f88_06ac),
            (6000, 0xec9d_1d5d_9847_741c, 0x5ee6_b1d1_0838_40ec),
            (7500, 0xada5_d9c3_6089_be0e, 0xfbb6_69b7_825a_34df),
            (9000, 0x1335_d3f3_524b_9033, 0xf515_a062_95c8_2013),
            (10500, 0xb6f9_0f2a_6239_0aab, 0x691e_242c_414b_78cf),
            (12000, 0xee3a_9934_17a0_e149, 0xdd31_cd6a_d47e_cad3),
            (13500, 0x88c0_51a6_87c6_62b2, 0x6711_3077_21cb_f75e),
            (15000, 0x7b49_b18e_0d13_474a, 0x05fa_f92f_da72_b9ab),
            (16500, 0x1197_562a_60bf_c9a7, 0xbc21_0c40_a003_b11e),
            (18000, 0xaced_e921_8c6d_9959, 0x2c8b_ed6e_9ec7_0b84),
        ],
        last: 0xc548_bb8a_9473_bfa8,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x5909_c2b4_3ec6_4b8f, 0x6fe0_6b69_a34d_d7b7),
            (3000, 0x4757_601f_066e_587f, 0x91cd_6d3d_3169_7538),
            (4500, 0x700a_eb77_9d5b_f0b5, 0x263d_31c0_53aa_9b81),
            (6000, 0xd92e_42b3_2e02_22dd, 0x4090_90c5_82cc_80fd),
            (7500, 0xacba_c1d3_7b92_3ef5, 0xaa68_2b2e_2b5b_6455),
            (9000, 0xa0ad_4d06_a51b_debf, 0x73d3_1157_8449_da5b),
            (10500, 0x98dd_d195_0596_1c5a, 0x6376_f83a_ec3f_767b),
            (12000, 0xaba6_348a_9155_ea34, 0x5fb7_c709_51fc_9ba0),
            (13500, 0xe901_1fd7_4658_3fd8, 0x6acc_e6e2_2afd_cd8b),
            (15000, 0xa7b0_6367_983d_7132, 0x6bdd_b103_7f46_1997),
            (16500, 0x970a_617e_d71f_daeb, 0xaf2e_1c74_bc31_534a),
            (18000, 0x69cb_055e_69c6_c40a, 0x1e01_2966_f1cf_1d63),
        ],
        last: 0x19ba_db29_4f27_a5a5,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xcc80_347d_e135_07a8, 0xf578_b9f3_d081_a713),
            (3000, 0xee34_0c7e_fa08_a81d, 0x4f43_659a_37bc_e555),
            (4500, 0x723a_9353_74ba_65f8, 0x9cbf_395a_fb9e_afd4),
            (6000, 0xf926_15ef_2d6e_820e, 0x4a11_29b5_4e52_c1b8),
            (7500, 0xb497_5cca_4550_b010, 0xcd79_c321_86bd_ba32),
            (9000, 0x0cca_a20c_51ab_2185, 0x217f_f63e_bdcf_0b15),
            (10500, 0x91f5_9cb9_7b09_d6dd, 0x80fb_ebf7_7f9a_a018),
            (12000, 0xf2e7_2ed4_4ce2_6ee8, 0xbac9_4b63_7aed_d03c),
            (13500, 0x9495_9218_1d5f_ef45, 0x56f7_7da9_4d05_0841),
            (15000, 0xe92b_f641_c26a_af38, 0x6071_54c6_d353_2bce),
            (16500, 0x5832_6588_13c4_9a7f, 0x964c_796e_be8f_745e),
            (18000, 0x4b8e_34d6_e2ec_b09a, 0xeaad_ed66_c573_f8cd),
        ],
        last: 0x3703_c0c0_3c84_cc1c,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xb957_93df_6229_1014, 0x4dd4_a361_8c19_b1a7),
            (3000, 0x6e9e_6f7f_12fd_20a7, 0x1543_940a_05d6_5b14),
            (4500, 0x6b86_180f_7ac3_5994, 0x0f70_fe76_21a9_1148),
            (6000, 0xc152_4b39_4ab7_2387, 0x3a0f_b056_8626_80d6),
            (7500, 0xdf76_b62c_bc9a_9f96, 0x1fe0_371d_0aa2_53bc),
            (9000, 0xfa3a_6114_8171_47cd, 0x23ce_f014_7992_ba33),
            (10500, 0xbfb7_6018_41f9_2769, 0x41be_f3af_1a25_007b),
            (12000, 0x19ab_4406_c42b_4156, 0x7a65_70de_7f7f_6450),
            (13500, 0xccce_7422_8d5a_8d3e, 0x1809_eb84_1b6b_7133),
            (15000, 0x43dc_762d_b414_748a, 0xc0c8_0016_13a5_0084),
            (16500, 0xe576_2e12_8fe0_190d, 0xf8d8_1653_3427_909a),
            (18000, 0x0182_1613_2b6a_dec1, 0x9d42_5d6d_8a09_7cba),
        ],
        last: 0x8d09_4806_50a4_c0d3,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc9f7_30db_186e_fe36, 0x9ded_3418_01d2_c5f1),
            (3000, 0x4f55_af54_7a0e_dace, 0x390b_e193_1869_7cd7),
            (4500, 0x04e9_ab42_3706_87ec, 0x4ca5_3903_943d_4489),
            (6000, 0xe362_caad_2bce_87f5, 0xd037_911d_0f78_b759),
        ],
        last: 0x9187_3bfc_7d99_7657,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xc9a9_431f_65a4_ce75, 0x4313_3435_30d3_275e),
            (3000, 0x4f42_3309_e3c1_2b4a, 0x717b_50ea_44cb_3c2e),
            (4500, 0x2246_c7db_d360_96cc, 0xead6_6cad_c2f7_395e),
            (6000, 0xe7ce_ce70_0d75_d5f9, 0xfaf1_8735_edae_f94e),
            (7500, 0xb122_c200_7649_975c, 0x2fa2_f9bf_6ea1_e967),
        ],
        last: 0xb61a_0235_991d_808f,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb821_1fe1_5ae5_3050, 0x2525_a4fe_7272_5b73),
            (3000, 0x45e6_5327_62f8_e7bc, 0xf3f8_d808_17bd_7341),
            (4500, 0xa7e5_f42e_fe77_c259, 0xb5ac_f13d_4c7d_1cee),
            (6000, 0xfa08_62be_e38b_2d5c, 0x29f6_c89a_d47b_720d),
            (7500, 0x5a44_f6d6_0c25_33cb, 0x72d1_a1ae_0e92_e5f5),
        ],
        last: 0x1306_065e_1fd9_ad9a,
    },
];
