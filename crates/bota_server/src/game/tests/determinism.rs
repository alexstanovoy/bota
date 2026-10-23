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
            (1500, 0xf27f_1eb9_05a9_9e15, 0xbdc6_dbc9_6705_d3b1),
            (3000, 0x020e_0047_1a58_1f88, 0xfa9a_3811_8873_05a0),
            (4500, 0xa603_4d2d_a267_8d18, 0x0700_9437_a502_1d24),
            (6000, 0x7425_3e33_c4c4_4165, 0xf312_78b5_34b3_e048),
            (7500, 0xaa56_e221_fbab_d9b9, 0xbd12_2888_ed6c_cedc),
            (9000, 0xa011_fdf2_edd1_d366, 0x41a6_25f2_04dd_efb5),
            (10500, 0x1ae2_85f9_6a75_45b1, 0x5c9c_0229_f5a4_9619),
            (12000, 0x36a4_9ad2_63f1_08e3, 0x3990_ed0d_04ee_dc3f),
            (13500, 0x9a8d_a7e9_c7d2_19f2, 0x66d5_369c_ab1f_dfb0),
            (15000, 0x53d9_dae1_57c9_d656, 0xea53_28f5_ce57_4942),
            (16500, 0x8417_c2d3_1092_7261, 0x55d4_1c83_6775_d58c),
            (18000, 0xd5e3_3584_f493_a551, 0x84b5_be36_b35b_3a9c),
        ],
        last: 0x9273_0c45_820e_9b6b,
    },
    Pinned {
        map: MapId(0),
        seed: 2,
        heroes: &[1, 0, 1, 1],
        checkpoints: &[
            (1500, 0x5964_4352_8507_fc46, 0xe0a0_daf6_afee_5855),
            (3000, 0x46ac_98c9_fe32_5d54, 0x715f_c9c2_3176_a435),
            (4500, 0xe3f1_f88e_c964_12d6, 0xd51c_5932_24dc_6ea6),
            (6000, 0x4274_42a7_9fac_8e5d, 0x2047_9e68_bcfd_83d6),
            (7500, 0xd46a_9fdc_a740_4d8f, 0xad5b_18c3_b3ea_85b9),
            (9000, 0xbf7c_92bf_944e_c072, 0x2b7e_c177_322b_9096),
            (10500, 0xfacc_f020_96de_d532, 0x4785_df3a_63a5_c736),
            (12000, 0x489e_7b04_7dd9_24ec, 0xbeee_b15a_a7b5_cb87),
            (13500, 0x5f19_86a1_a10a_933c, 0x5d28_40f5_18e4_8311),
            (15000, 0x70df_a7e1_f21b_81e1, 0x7ef6_40f4_e449_0e53),
            (16500, 0x10f0_d315_73bc_c78a, 0x9775_e4b5_cd8f_b95c),
            (18000, 0x3097_90f5_79ed_6c26, 0x6812_8118_7236_85e3),
        ],
        last: 0x18d5_8340_3ca4_62c7,
    },
    Pinned {
        map: MapId(1),
        seed: 3,
        heroes: &[1, 0],
        checkpoints: &[
            (1500, 0x676a_c19e_2e0e_f2fb, 0x98a8_2767_337d_9c62),
            (3000, 0xd5c0_7bb4_4dc9_da83, 0x815a_dc9e_d743_5250),
            (4500, 0x10ff_47a6_f1b9_0d92, 0x13c1_ff14_b8c2_5fcf),
            (6000, 0x750f_a9e1_78bb_6611, 0xfd7d_c038_5329_9809),
            (7500, 0xba1e_09f0_846f_ecf6, 0xb18f_f4e1_ee08_9f41),
            (9000, 0xb806_7436_4650_a14f, 0x406d_1f6b_b6b2_2532),
            (10500, 0x69ad_78c1_d98c_f6be, 0xe931_b0e6_114c_f48b),
            (12000, 0x3271_67d2_3cf3_9b70, 0x103c_a4dc_e128_16ff),
            (13500, 0x650d_4f06_0874_56e1, 0xaf04_5d02_8a52_8322),
            (15000, 0xc6de_5951_9685_be9f, 0x1a67_a2e2_e8c4_c71a),
            (16500, 0xc050_0007_8e86_5d0c, 0x5e5a_6443_2b00_6de0),
            (18000, 0xc252_e63a_c276_91bd, 0x3917_a5bc_5a2c_1919),
        ],
        last: 0xf529_adb7_e823_8c34,
    },
    Pinned {
        map: MapId(1),
        seed: 4,
        heroes: &[0, 1],
        checkpoints: &[
            (1500, 0x1358_ae66_8f55_5c71, 0x2f64_b39b_94cc_1979),
            (3000, 0xb268_b990_dbe3_dc1a, 0xa908_a904_1bc3_25b7),
            (4500, 0xbee4_41fb_0dd2_a4a2, 0xf13f_1302_0ff3_89d6),
            (6000, 0x8944_0b17_58b0_b792, 0xa687_f10d_b8fd_9b7c),
            (7500, 0x400e_71f1_ac32_2408, 0x8cf7_e828_6755_746f),
            (9000, 0x6594_1786_7bcc_6aec, 0xdef3_29b9_dfba_20c5),
            (10500, 0x7dd9_c5a5_5abb_97de, 0xea89_bac2_f929_ba3a),
            (12000, 0xbbdc_2144_bd69_6431, 0xe6a8_56b2_01a9_d295),
            (13500, 0xc902_c6c0_3b6a_da2a, 0x7487_7c01_7006_e626),
            (15000, 0xbead_5bf5_2406_6c57, 0x02cf_3342_dd70_e6f8),
            (16500, 0xd13c_319d_7291_c9a4, 0xf15c_4a2f_f0db_a72e),
            (18000, 0xa56f_1cad_acb3_382b, 0x7c18_dae3_3323_9a89),
        ],
        last: 0x9a2f_da12_a4da_0b03,
    },
    Pinned {
        map: MapId(2),
        seed: 5,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x7047_2c96_1f81_5434, 0xddea_5d1d_c3c4_af0e),
            (3000, 0x7fdf_3d65_0714_80ba, 0xf020_2dae_e6f5_f70f),
            (4500, 0x4345_3f1c_2706_3115, 0x5de5_72cb_d456_e6cb),
            (6000, 0x2b43_1a39_b6f4_86ae, 0xcf11_01c7_9397_179e),
            (7500, 0x8b83_1f8f_0bb4_7790, 0x042f_488a_0dee_536c),
        ],
        last: 0x75d2_6fda_ae01_95b1,
    },
    Pinned {
        map: MapId(2),
        seed: 6,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0x6867_f8c6_0fb1_a2ca, 0x375b_5094_e816_416a),
            (3000, 0x5746_f6ce_ed65_5862, 0x69d2_ee25_41d8_52c9),
            (4500, 0x3c6d_cdc2_830c_2c70, 0xa4b7_4e41_f260_5fe3),
            (6000, 0x87be_1bdf_4db7_c193, 0x3aab_457c_d996_300e),
            (7500, 0xc17b_b6b4_620b_30aa, 0x9744_6751_a751_5e26),
            (9000, 0x1fd8_ef06_a597_1b91, 0x6373_627e_d8a4_7e14),
            (10500, 0x4e2e_2aa2_6f64_9a48, 0x3695_69cc_6255_ce21),
        ],
        last: 0x402e_c4db_def1_a802,
    },
    Pinned {
        map: MapId(2),
        seed: 7,
        heroes: &[1, 1],
        checkpoints: &[
            (1500, 0xae29_1fcd_f513_28dd, 0xfead_f097_8b3b_0a24),
            (3000, 0xcd52_e689_f437_6a47, 0x4944_3c79_447d_69a0),
            (4500, 0x6f53_0000_ad49_85e1, 0x4c72_2bfa_d407_b02e),
            (6000, 0x04cb_18fb_c0ae_3265, 0x5621_ac1a_62d5_54cc),
            (7500, 0x7f42_13ba_d4a2_1cb7, 0xf8c1_e7ad_0f0e_4ddc),
            (9000, 0x0910_8839_4dcc_c18d, 0x72aa_354c_c162_5c46),
            (10500, 0x3f30_6c5f_8702_a966, 0x4ea7_6312_c61f_1046),
        ],
        last: 0x1c76_ccac_00a6_79fd,
    },
];
