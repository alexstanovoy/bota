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
            (1500, 0x859c_d743_d926_8479, 0xa628_6b6c_fc68_8bb8),
            (3000, 0x3de9_dad8_89e3_cbfe, 0x6b5c_1392_5b80_cc5d),
            (4500, 0x80c5_7092_1b55_017f, 0xfd66_4043_ec76_d5d5),
            (6000, 0xf8e9_cd8b_e2f3_baca, 0x5e52_a659_60e8_bdc0),
            (7500, 0xffdd_5d58_14e8_299a, 0x3a66_265e_d72b_ecf7),
            (9000, 0x22b7_7ab4_1626_aa93, 0xec11_3dda_e055_56ff),
            (10500, 0xe46b_d052_5bb8_bbdd, 0xa926_c9b7_c1d2_6ec3),
            (12000, 0x81ac_b7d4_002d_90b9, 0x34ee_7ff6_4a55_693c),
            (13500, 0x81a7_e1b3_0a4c_32e1, 0x871d_f577_53c9_1316),
            (15000, 0x80b1_5b5a_1cf1_0fa7, 0xe0ee_91c3_b5dd_eb37),
            (16500, 0xa458_c1ba_b47b_afe4, 0xb440_3184_d338_9121),
            (18000, 0x71d6_3b03_37ee_758f, 0xab64_6a58_1117_a761),
        ],
        last: 0xe6d9_8379_1827_dfeb,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x62bb_cd6a_a0ba_3543, 0x668f_c549_2ed0_59a7),
            (3000, 0x3c05_6d9d_ba09_e44e, 0x615b_4419_879c_6554),
            (4500, 0xcede_7e51_fcb9_5164, 0xe7b3_5375_e1c2_396c),
            (6000, 0x0032_ba98_3629_f5a6, 0x403b_622e_a45d_6de7),
            (7500, 0x4d0c_5122_fa75_bb89, 0x49bd_1a34_8c4c_c83e),
            (9000, 0xb7ea_f522_2896_0ec5, 0x699f_aaba_6bb8_729b),
            (10500, 0x09c6_cdfe_bede_fa7f, 0xc95e_a8b5_cc80_914f),
            (12000, 0xfbce_0b5c_a66b_4629, 0x7b10_9b7b_0b5b_a8c6),
            (13500, 0x3916_79ea_fec3_ca1b, 0x24cf_259d_d494_4d54),
            (15000, 0xc1a3_b1d2_4194_4c03, 0x150e_dbbc_d48f_2134),
            (16500, 0x0477_77da_9554_142f, 0x645c_bd41_f8ad_3a27),
            (18000, 0x6136_8303_0266_e282, 0xdd1a_5881_0c00_0807),
        ],
        last: 0xe04b_b31e_b72a_8708,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x1b3a_3309_5008_83cb, 0x8b7e_37ae_ec23_d538),
            (3000, 0xe6a1_c5ca_8fbe_6440, 0x7806_020e_c4cb_c9d1),
            (4500, 0x9fbb_400c_1872_5e40, 0x813f_8793_6ef6_8644),
            (6000, 0xa7f1_6984_da16_d8d5, 0xe2fa_8491_b30c_6cfc),
            (7500, 0xcb52_8ef0_527a_8dac, 0x7be9_8934_3605_65e3),
            (9000, 0x9482_267d_dffb_fbc6, 0xfd4d_816c_7477_68ff),
            (10500, 0xe0a5_6d40_5b12_8716, 0xe748_99ec_3a7b_bf2b),
            (12000, 0x35d9_8876_153f_23e5, 0xdd12_9944_1e6e_17ba),
            (13500, 0x2466_5b92_9b91_e784, 0xf215_4bc3_7e69_7c68),
            (15000, 0x025d_6eb9_1a94_fd95, 0x1e73_0fa9_c5f2_b835),
            (16500, 0x36fb_d9ab_6d4d_511a, 0x1095_df33_f95a_eec1),
            (18000, 0x7b1b_66c5_f628_245c, 0x5ad2_fd8f_1c9c_fd87),
        ],
        last: 0x4aad_af80_103e_2304,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x8c4a_badb_bcf7_acea, 0x7fa4_a65b_11d7_da94),
            (3000, 0xabaa_3834_40d0_56a0, 0xb6ee_b950_2e88_9aea),
            (4500, 0xa847_2aac_390f_5521, 0x0ca0_ceaa_95d8_8d24),
            (6000, 0x1403_d34e_cd09_dc92, 0xa51b_5642_b44a_e24c),
            (7500, 0x3ba2_4e57_24f0_de62, 0xbdcc_1b79_778f_eb10),
            (9000, 0x6641_2b6c_8fd3_b4e1, 0x3feb_bf18_54d7_890c),
            (10500, 0xda24_48f9_1937_0a76, 0x7101_1c0d_e64c_4f0b),
            (12000, 0x1350_8558_1049_242f, 0xa78c_820d_d293_a047),
            (13500, 0x4a9a_75b2_4901_0ac1, 0xde5d_f37a_f376_1b73),
            (15000, 0xf550_a541_8b95_6059, 0x208a_ecab_8e9f_02df),
            (16500, 0x40d9_e573_0c4f_3281, 0x5d9d_bbce_d153_0b3d),
            (18000, 0xe9bc_4c18_5cec_53a2, 0x4c89_9408_0ade_68ea),
        ],
        last: 0x2414_1897_41d4_9dcc,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x9572_5b60_34c3_ded4, 0xfb3f_47c5_b060_4e13),
            (3000, 0x4ac5_85a2_46dc_1619, 0x7d10_4ea0_35f4_79f7),
            (4500, 0x2fac_2012_abd4_0b9f, 0x0298_0a4d_de1d_02e3),
            (6000, 0x9d17_24cf_4bab_3732, 0xd5a4_bb43_bb70_bf44),
            (7500, 0x15fa_56c6_fbdf_e8e0, 0x0650_620b_1317_39a1),
        ],
        last: 0x9b34_05ae_5b6b_8987,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x3459_3bde_41d6_b9b2, 0x5d99_cef5_88c0_f721),
            (3000, 0xbf85_9e1e_4dfa_564e, 0x50d8_9246_2727_8b66),
            (4500, 0x3376_472d_5783_f64b, 0x67f0_14bc_77dd_8269),
            (6000, 0x3580_77bc_6ebf_2c5f, 0xcf72_9196_8025_deb0),
            (7500, 0x43fa_6445_238e_4444, 0xe5b7_5ba6_67d1_1a02),
            (9000, 0x6801_0d4b_6429_3dbd, 0x6044_b986_3d89_ec27),
            (10500, 0xbf66_f4de_e489_df33, 0xf56b_e928_39f0_f0d0),
        ],
        last: 0x94e4_7a52_8b12_17a9,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x4421_3430_f283_d0f8, 0x9c72_aef3_13b0_5125),
            (3000, 0x9804_461d_fddf_9756, 0x89b0_0310_65f5_c325),
            (4500, 0xc89c_527f_7c67_9976, 0xa6b6_0d1b_eb59_c9bb),
            (6000, 0x3d96_2cfd_9269_e08b, 0x3ab6_4465_9de0_0057),
            (7500, 0x958e_b99c_b445_07ad, 0xe4c5_aeab_0794_9c97),
            (9000, 0x064d_8e59_56b9_fb5b, 0x9d9f_be25_c27b_6ae1),
            (10500, 0x8116_e88f_d0fe_1763, 0xc62e_88f1_357e_41ac),
        ],
        last: 0x809a_a5c2_30f3_e537,
    },
];
