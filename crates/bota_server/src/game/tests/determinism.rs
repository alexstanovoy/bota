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
            (1500, 0xc29c_af6c_5b56_4d2a, 0x1b56_35f7_d7b3_b877),
            (3000, 0x3a7e_8f11_288a_2d58, 0xa210_34a8_0207_9fa2),
            (4500, 0x3fac_1d61_8d72_6c20, 0x555c_e63a_19cc_b317),
            (6000, 0xba7a_e67a_686a_d99b, 0x49fe_28af_0045_1ae6),
            (7500, 0xa7ca_8528_9b80_4a7b, 0x39de_cc2c_b0a4_42e3),
            (9000, 0x4382_979c_7da8_1958, 0xc313_3de4_da9c_675a),
            (10500, 0x3bfa_98e5_a2c7_a733, 0xc29d_c693_ef7f_5f4a),
            (12000, 0xfb1c_3227_2deb_fe94, 0x644b_d9bd_8b03_65a7),
            (13500, 0xe212_d05c_4923_501c, 0xc3fa_acc5_34a0_a868),
            (15000, 0x1455_10d6_8f0d_dc63, 0x75b0_8449_54d5_3d14),
            (16500, 0x0d4c_d37b_f72e_3392, 0x3210_f070_14fd_3ad3),
            (18000, 0x7bef_7db1_ad03_3b22, 0x9275_5196_6253_184a),
        ],
        last: 0xaebc_ecaf_6a66_d690,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0xbb9a_2ca8_e14b_f4c9, 0x7553_fbe6_5f28_7075),
            (3000, 0x3036_5378_6447_9606, 0xc0ec_3da8_a4b4_c4a2),
            (4500, 0x1bc6_955d_4783_aed1, 0x46ab_d6c3_721b_129f),
            (6000, 0x962c_94bd_dd26_5729, 0x083b_bc55_a272_5210),
            (7500, 0xfe47_9ff7_261c_2647, 0x93a3_3f57_40f4_7f71),
            (9000, 0x1019_5a1d_a954_fbc8, 0xca3f_5592_707b_f715),
            (10500, 0xd7a0_5e90_cf2c_8b72, 0xb3a1_f686_aefa_9e56),
            (12000, 0x7f4d_5b30_f2b2_b91d, 0xe8e0_7ef8_7faf_dfb9),
            (13500, 0x51cf_02c2_6f0e_11cd, 0x7a89_b868_7da6_9733),
            (15000, 0xb33b_2504_7f3f_a35a, 0xbf44_ad1e_7ddf_8ed7),
            (16500, 0x94c7_77bf_5e31_0295, 0xaa3a_4e3e_686f_3be9),
            (18000, 0xf977_cba6_9510_9207, 0xab25_0120_2f47_f802),
        ],
        last: 0xa5ba_f7b3_fb03_3b98,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0xaad1_6bbf_84f9_ac6c, 0xf8ab_6248_9efb_8641),
            (3000, 0x7284_38fb_1d6e_f674, 0x6a78_937d_26fd_1652),
            (4500, 0x4b88_5fef_337a_abe6, 0x192c_eba4_2081_f9e3),
            (6000, 0xa35f_aedb_57c6_fcde, 0xe007_1ebb_5a50_69cf),
            (7500, 0xbc07_5b87_e76b_e996, 0x4914_50f4_1714_672b),
            (9000, 0x8aa9_ee45_4eb4_5ad9, 0x099d_f36d_8a77_8f76),
            (10500, 0x6418_bdd8_44a0_7bcc, 0xa386_3aec_2641_ad8e),
            (12000, 0x429c_e7e8_1bef_0378, 0x96ca_e724_5252_70d4),
            (13500, 0x84b9_464b_b301_5f1f, 0x374a_8aa9_7890_a731),
            (15000, 0xf0ce_19e5_85e7_7908, 0x2e01_46aa_87d8_57f3),
            (16500, 0x2ad7_9c3d_19dd_986b, 0x4e24_5d5c_68b9_7422),
            (18000, 0x0766_c112_dfbd_0eb6, 0x0ff1_a96d_3513_67a3),
        ],
        last: 0x88fb_66d9_1968_1c5a,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0xe302_3c28_9021_704d, 0x46b6_95e5_973b_843a),
            (3000, 0x6eab_e1f6_b5ef_0566, 0xccb1_a3ec_12fc_68bc),
            (4500, 0xb290_2a28_590c_9499, 0x8d28_1932_c717_9c97),
            (6000, 0x8d85_fc23_fe3d_3087, 0x5abf_6fb8_d56b_dfb3),
            (7500, 0x5b0e_dc4f_9f3c_4e31, 0x1729_f887_55f6_47e0),
            (9000, 0xa209_c234_e7d5_bd49, 0x9c1d_a611_b553_2f88),
            (10500, 0xd924_363f_72aa_097c, 0xc12d_2ebb_811a_9cdc),
            (12000, 0x817b_1246_8141_7c9c, 0x34c9_569e_98f2_638e),
            (13500, 0xd00a_62f2_1ee2_7865, 0x0a46_3d3d_0636_ab37),
            (15000, 0x1f0a_a375_d213_6497, 0xf7c2_d0ee_3aa7_642a),
            (16500, 0xa2fb_b2fa_45d6_77e1, 0x5e8e_633c_bbb6_f18d),
            (18000, 0xbf00_76cb_b60b_05f6, 0xd5a4_63c1_4e24_41b6),
        ],
        last: 0x2fda_2a94_3a61_c0d3,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xfdea_e5f3_1e9a_c79a, 0xa3b3_234a_c3f4_be24),
            (3000, 0xc580_ea2a_6618_4e3f, 0x4872_400f_28a4_22cc),
            (4500, 0x3e07_d1d7_7eb1_841f, 0x4b43_f9e0_19c3_befb),
            (6000, 0x7b92_6424_14e4_3205, 0x893d_7a5b_e40a_0c0e),
            (7500, 0x3ce4_1b9f_b478_f01e, 0xd1d4_e036_d429_0994),
            (9000, 0x499e_ff06_949e_e863, 0xa21e_3fe4_5e84_6a32),
            (10500, 0x29a7_79a5_6b21_b5c0, 0xdade_e810_3890_158d),
        ],
        last: 0x8407_0702_1e00_b236,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xbb4a_0f5b_7377_4c88, 0x9814_48a0_ef49_ebd2),
            (3000, 0x1009_2453_f47f_36d0, 0x7268_2a6d_b468_2113),
            (4500, 0x5db9_35b2_a22c_adf1, 0xe01c_4d67_e03f_216d),
            (6000, 0xfa33_95df_e1c4_8c7b, 0x631c_0180_1c12_13f1),
            (7500, 0x3ad2_7c92_32fe_6131, 0x091a_6bbd_df2e_a917),
            (9000, 0x142f_17db_31dc_d38e, 0x6686_3a16_9548_f2dc),
            (10500, 0x7d91_eade_71fc_ac59, 0x1df2_278e_e128_456d),
            (12000, 0x68e6_4a8e_925f_a40c, 0x3a5c_3221_f94c_cf22),
            (13500, 0xdd8f_a675_35f9_9393, 0x65e8_9cf9_4fbe_f220),
            (15000, 0x910a_6b08_a102_a936, 0x7f3c_2b99_c8e6_355b),
            (16500, 0x4bae_ce5d_1e85_be07, 0x2b79_348d_ee8b_9402),
        ],
        last: 0x6729_5ab5_bf3c_5da0,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x93b6_924c_e0e3_5d4e, 0x7f62_e25a_969b_7826),
            (3000, 0x6fdb_d1fb_51cd_cb67, 0xc6c6_c118_0d50_e7a6),
            (4500, 0x29b3_a6b3_b2cb_a6db, 0x29e2_5127_df19_a99d),
            (6000, 0x46c9_fa9d_d78a_1903, 0x1539_de56_29ef_82f9),
            (7500, 0xc30e_e44a_e293_ab0f, 0x9b4c_0e86_4ce0_ebf0),
            (9000, 0xce4a_643e_d673_51ba, 0x4ee6_1337_bea1_fb41),
            (10500, 0x5e7b_1b5c_5d7a_35ed, 0xd4fb_07a6_1487_5e02),
        ],
        last: 0xc641_31b4_c967_20a7,
    },
];
