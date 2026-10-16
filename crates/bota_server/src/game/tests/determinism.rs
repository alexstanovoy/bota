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
            (1500, 0x62ac_11b3_34ce_81e4, 0x54af_25ed_029f_4770),
            (3000, 0x914d_be78_fe06_fa33, 0x1b11_af3b_f0e9_bcda),
            (4500, 0xc73b_448c_72de_74e5, 0xef63_46af_1995_0ed2),
            (6000, 0x4a3d_ae90_f4f5_8340, 0x828b_d9bd_fdbf_5595),
            (7500, 0x04cb_9c5b_136b_4970, 0x7692_4cd7_a93a_a4af),
            (9000, 0xe07e_4e9e_2de3_67ab, 0x40e8_d7a5_77d7_33d0),
            (10500, 0xba89_fa39_a244_e4f8, 0x39c7_92d0_8e15_7e69),
            (12000, 0x414f_0391_72c5_62ba, 0x5907_c30e_bdf7_fb8d),
            (13500, 0xc363_8915_4b2c_2ae5, 0x9e62_f142_ab90_ec37),
            (15000, 0x7e2b_27b5_7087_2c7a, 0xd626_6706_23a3_3c59),
            (16500, 0x4307_16b3_0e58_a764, 0xb4ad_d541_c271_ff38),
            (18000, 0xb1be_fd57_c0ac_dc92, 0xfa22_e204_82be_ec69),
        ],
        last: 0xe9ef_5602_4f68_d039,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x6dff_cb78_0815_8b47, 0xfaad_1d66_c5f5_9d6f),
            (3000, 0xf6d3_f9cc_06ca_239f, 0x6808_f1bf_bd7f_96dc),
            (4500, 0xa681_f433_c87e_ab75, 0x724e_a237_f29d_6a19),
            (6000, 0x0ab6_5ff5_d131_34ee, 0x511e_3665_dd1a_52a3),
            (7500, 0x40da_4c7d_bbe3_64cc, 0xfd2f_31d0_651a_5870),
            (9000, 0xe4fe_3d35_7baa_6e3b, 0xce84_3ef0_e4d9_555d),
            (10500, 0xbaf8_ba13_a34f_ff93, 0x482f_67d6_a933_75ce),
            (12000, 0xae30_0f52_01d1_4e6b, 0x276f_af7c_1b54_2ed2),
            (13500, 0x45be_b223_de73_8ca8, 0x0038_b2cb_b58c_8571),
            (15000, 0x0f14_605c_dfbd_c3b1, 0x2801_a78e_2298_8990),
            (16500, 0xfbd7_ab11_2393_1ce2, 0x85d9_f469_780b_8e17),
            (18000, 0xc979_d10e_28e0_dbfb, 0x3f93_9f7f_d873_1475),
        ],
        last: 0x257c_20f8_6984_2f5e,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x218c_6256_2867_8e66, 0x86db_a3a2_d3db_b772),
            (3000, 0xb3b3_2521_66ce_5bea, 0x7141_9034_f3d3_8b03),
            (4500, 0xdbb6_a829_f9a0_f032, 0x342b_79f2_895b_e2e1),
            (6000, 0xc67d_94d6_140c_905a, 0xfc3d_a92c_aa92_fe14),
            (7500, 0xb318_49b0_6c70_176b, 0x9f0d_0bee_7751_e474),
            (9000, 0x015f_a65b_1a6c_4e2a, 0x0e1c_ed45_1f0e_6318),
            (10500, 0xf54e_e812_d356_d633, 0xf3d2_e5e5_2746_a1fe),
            (12000, 0x33c1_58e6_9697_a349, 0x8104_dbab_3a03_3241),
            (13500, 0x3651_8080_c25b_9a32, 0x53d4_a6c7_6f2d_b63b),
            (15000, 0xf09f_48aa_e6ab_b87e, 0xe0c8_2492_7ada_b62d),
            (16500, 0x68ec_c28f_ebff_e0ea, 0x7016_e62d_30d6_57cf),
            (18000, 0x7a37_7d7a_ae96_9e9c, 0xe0a4_3ff5_d38b_dd7a),
        ],
        last: 0x3d32_399d_a0b6_9dde,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x715f_1df7_1e4b_23a4, 0xd4aa_e1c9_b81d_5d37),
            (3000, 0x7bd2_7240_7b9c_19a9, 0xb756_f510_49b5_7bf6),
            (4500, 0xb555_f3c0_1f77_45a8, 0xdcba_d94b_37a7_2c27),
            (6000, 0x99dc_13af_288b_0976, 0x6c20_56e8_d661_7080),
            (7500, 0x77e5_d0b3_91e0_e374, 0xcabf_be0d_c3c2_7971),
            (9000, 0x8b16_a3ba_cef6_4c38, 0x5853_4fe3_ab66_262a),
            (10500, 0x4a13_c96d_197a_aebe, 0x542b_3a88_0dd3_5db8),
            (12000, 0xb972_95ad_6aa9_8dcb, 0x6eae_49df_d3b6_afd5),
            (13500, 0x1a77_7c92_4a1f_43a7, 0xfd9b_ce50_4484_740c),
            (15000, 0xf728_41e5_2bca_f214, 0xbdb2_837f_b8e8_dac7),
            (16500, 0x0789_b416_fb22_4847, 0xc47c_6618_2cf5_d614),
            (18000, 0x6a0b_2cfd_654a_03b5, 0x4d44_6634_4dc2_a769),
        ],
        last: 0x6310_bc19_887b_b758,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6781_1c3a_f936_7f0a, 0xd0ca_e56f_53d7_0648),
            (3000, 0xba44_3984_d68a_9f68, 0x1a4d_4575_ef0f_cc68),
            (4500, 0xc3f7_8049_0e3e_35cf, 0x5af2_5f45_bc14_48fa),
            (6000, 0x04fc_4a67_6c19_5215, 0xd4ec_3067_76b4_5d22),
            (7500, 0x2435_9fdf_79a3_95b2, 0x3020_82d1_b995_b63d),
            (9000, 0x0405_7f90_aa8e_9bd5, 0xe61f_0826_971d_29aa),
        ],
        last: 0xd63b_d779_da79_b4a8,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x91db_0272_f20e_effb, 0x0829_cd1d_5a50_5783),
            (3000, 0x75d5_0864_8a21_7808, 0x0e37_121f_3a27_6881),
            (4500, 0xa262_e40c_3b8e_20c8, 0x6e36_a833_6b5d_b896),
            (6000, 0x091c_d51f_a5bc_d40c, 0x1915_64d9_c676_d372),
            (7500, 0xddda_b4f1_94ca_b0f3, 0x8da8_ea84_3297_bc96),
            (9000, 0x79c0_9b8c_c6f4_3f1c, 0x93dc_b985_c4e6_4e23),
        ],
        last: 0x2df9_a8d2_4a66_8ab1,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xb731_40e3_0c54_14f9, 0x1207_dda3_8796_70be),
            (3000, 0x7855_4b63_a228_0640, 0x2fe9_3c3a_ee2e_6d19),
            (4500, 0x8ad5_2520_eca7_b62c, 0xfdcd_36e8_ce5a_e780),
            (6000, 0x1638_a77b_68ea_813c, 0xf365_2025_c157_3ddf),
            (7500, 0x381a_8825_3f79_a5ca, 0x08f9_9067_6512_df76),
            (9000, 0x955d_8c4b_87de_a5a7, 0x0008_8fd9_3960_f3b5),
        ],
        last: 0x516e_e96d_6967_cf59,
    },
];
