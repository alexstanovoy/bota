//! The hero demo game (Dota's Hero Demo on its own small map) as its one player meets it: an
//! in-process match of one seat with cheats on, driven by orders and cheats through the order
//! path, every check read from what the seat was sent.

use bota_core::{DEMO_ID, HERO_DEMO_ID, attackable_from, map_of, rules};
use bota_proto::{
    AbilitySlot, Cheat, DamageKind, EntityId, HeroId, ItemId, MapId, Order, OutgoingOrder,
    PlayerView, RejectReason, ServerMsg, SlotId, Target, Team, UnitKind, UnitView, Vec2, WorldView,
};

use super::support::config;
use crate::game::{MatchConfig, OrderDelay};
use crate::local::LocalMatch;

/// How far from its aim the landing search looks: its rings out to the last.
const LANDING_EXTENT: i32 = rules::SCROLL_LANDING_RINGS * rules::SCROLL_LANDING_STEP;
/// Ground no body stands on anywhere near: far outside the demo map.
const NO_GROUND: Vec2 = Vec2::from_ints(2000, 2000);

/// The ticks a hero of the hero demo lies dead.
const DEMO_WAIT: u32 = rules::DEMO_RESPAWN_SECONDS * rules::TICKS_PER_SECOND;

/// A one-seat match, as a Demo Hero is started, and what its seat was last sent.
struct Demo {
    local: LocalMatch,
    seq: u32,
    view: Option<WorldView>,
    rejected: Vec<RejectReason>,
    over: bool,
}

impl Demo {
    fn on(map: MapId) -> Demo {
        let cfg = MatchConfig {
            map,
            cheats: true,
            objectives: map_of(map).default_objectives,
            runes: map_of(map).default_runes,
            ..config()
        };
        let (local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
        let mut demo = Demo {
            local,
            seq: 0,
            view: None,
            rejected: Vec::new(),
            over: false,
        };
        demo.take(opening);
        demo
    }

    fn take(&mut self, streams: Vec<Vec<ServerMsg>>) {
        self.rejected.clear();
        for msg in streams.into_iter().flatten() {
            match msg {
                ServerMsg::Snapshot { view } => self.view = Some(view),
                ServerMsg::OrderRejected { reason, .. } => self.rejected.push(reason),
                ServerMsg::MatchOver { .. } => self.over = true,
                _ => {}
            }
        }
    }

    fn step(&mut self, orders: &[Order]) {
        let named: Vec<_> = orders.iter().map(|&order| (None, order)).collect();
        self.step_naming(&named);
    }

    /// One tick with these orders, each to the unit it names (nobody: one's hero).
    fn step_naming(&mut self, orders: &[(Option<EntityId>, Order)]) {
        let seen = self.local.tick();
        let out: Vec<OutgoingOrder> = orders
            .iter()
            .map(|&(unit, order)| {
                self.seq += 1;
                OutgoingOrder {
                    seq: self.seq,
                    unit,
                    order,
                    seen,
                    queue: false,
                }
            })
            .collect();
        let streams = self.local.step(&[&out]).expect("a step");
        self.take(streams);
    }

    fn cheat(&mut self, cheat: Cheat) {
        self.step(&[Order::Cheat { cheat }]);
        assert_eq!(self.rejected, [], "{cheat:?} was turned down");
    }

    /// One order to a unit, which must not be turned down.
    fn order(&mut self, unit: EntityId, order: Order) {
        self.step_naming(&[(Some(unit), order)]);
        assert_eq!(self.rejected, [], "{order:?} to {unit:?} was turned down");
    }

    /// A cheat aimed at a unit, which must not be turned down.
    fn cheat_at(&mut self, unit: EntityId, cheat: Cheat) {
        self.order(unit, Order::Cheat { cheat });
    }

    /// A hero of `hero` stood up by the panel's ENEMY a little way along the lane, and its handle.
    fn spawn_enemy(&mut self, hero: HeroId) -> EntityId {
        let before = self.view().players.len();
        self.cheat(Cheat::CreateHero {
            hero,
            enemy: true,
            at: rules::DEMO_LANE_CORNERS[1],
        });
        assert_eq!(
            self.view().players.len(),
            before + 1,
            "a seat was made for it"
        );
        self.cheat(Cheat::AllVision { on: true });
        let row = self.view().players.last().expect("its row");
        assert_eq!(row.team, Team::Dire);
        row.unit.expect("its hero")
    }

    /// The seat a hero belongs to, as the view tells it.
    fn row_of(&self, hero: EntityId) -> &PlayerView {
        self.view()
            .players
            .iter()
            .find(|row| row.unit == Some(hero))
            .expect("the hero's seat")
    }

    fn unit(&self, id: EntityId) -> &UnitView {
        self.view().unit(id).expect("the unit is seen")
    }

    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step(&[]);
        }
    }

    fn view(&self) -> &WorldView {
        self.view.as_ref().expect("a snapshot was sent")
    }

    fn me(&self) -> &PlayerView {
        self.view().player(SlotId(0)).expect("the seat's row")
    }

    fn hero(&self) -> Option<&UnitView> {
        self.view().unit(self.me().unit?)
    }

    fn units(&self, kind: UnitKind) -> impl Iterator<Item = &UnitView> {
        self.view()
            .units
            .iter()
            .filter(move |unit| unit.kind == kind)
    }

    /// Kills the hero by Dota's `-suicide` where it stands, and answers where that was.
    fn kill_hero(&mut self) -> Vec2 {
        let fell = self.hero().expect("the hero stands").pos;
        self.cheat(Cheat::Suicide);
        assert!(self.me().respawn_left > 0, "the hero lies dead");
        fell
    }
}

fn lane_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepRanged
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepSiege
    )
}

/// Whether two spots are no further apart than `units`.
fn near(one: Vec2, other: Vec2, units: i32) -> bool {
    one.within(other, rules::units(units))
}

#[test]
fn the_hero_demo_starts_the_hero_at_the_maps_player_start_between_roshan_and_the_lane() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.run(1);
    let at = demo.hero().expect("the hero stands").pos;
    assert!(
        near(at, rules::DEMO_HERO_STARTS[0], 150),
        "the hero came in at {at:?}"
    );
}

#[test]
fn the_hero_demos_waves_walk_out_at_once_and_meet_between_the_two_towers() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.run(1);
    let walking = demo.units(UnitKind::CreepMelee).count();
    assert!(walking > 0, "no creep walks on the first tick of the game");
    // Each side's wave spawns at its own end of the lane; the first blows are struck where
    // the two meet, which is clear of both towers.
    let towers = [
        rules::DEMO_RADIANT_TOWERS[0].2,
        rules::DEMO_DIRE_TOWERS[0].2,
    ];
    let mut met = None;
    for _ in 0..60 * rules::TICKS_PER_SECOND {
        demo.step(&[]);
        let hurt = demo
            .view()
            .units
            .iter()
            .find(|unit| lane_creep(unit.kind) && unit.hp < unit.max_hp);
        if let Some(hurt) = hurt {
            met = Some(hurt.pos);
            break;
        }
    }
    let met = met.expect("the two waves never fought within a minute");
    // A quarter of the way from either tower: sixteen times the squared distance.
    let gap = towers[0].distance_squared(towers[1]);
    for tower in towers {
        assert!(
            16 * met.distance_squared(tower) > gap,
            "the waves met at {met:?}, beside the tower at {tower:?}"
        );
    }
}

#[test]
fn the_hero_demo_sends_a_wave_down_its_lane_every_thirty_seconds() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::AllVision { on: true });
    let mut seen = std::collections::BTreeSet::new();
    let minutes = 2;
    for _ in 0..minutes * 60 * rules::TICKS_PER_SECOND {
        demo.step(&[]);
        for unit in demo
            .view()
            .units
            .iter()
            .filter(|unit| lane_creep(unit.kind))
        {
            seen.insert((unit.team, unit.id));
        }
    }
    let waves = minutes * 60 * rules::TICKS_PER_SECOND / rules::WAVE_PERIOD_TICKS + 1;
    let per_wave = (rules::MELEE_PER_WAVE + rules::RANGED_PER_WAVE) as usize;
    for team in [Team::Radiant, Team::Dire] {
        let count = seen.iter().filter(|(side, _)| *side == team).count();
        assert_eq!(count, waves as usize * per_wave, "{team:?}'s creeps");
    }
}

/// Where a hero stands in Roshan's pit on the demo map, a little off his spot.
const IN_THE_PIT: Vec2 = Vec2::from_ints(5800, 9300);

#[test]
fn roshan_stands_in_the_hero_demos_pit_and_is_struck_only_from_inside_it() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    // The pit's walls hide him from the lane: the hero goes in to look.
    demo.cheat(Cheat::AllVision { on: true });
    assert_eq!(
        demo.units(UnitKind::Roshan).count(),
        0,
        "Roshan seen from outside his pit"
    );
    demo.cheat(Cheat::Teleport { at: IN_THE_PIT });
    let roshan = demo
        .units(UnitKind::Roshan)
        .next()
        .expect("Roshan stands on the demo map")
        .pos;
    assert!(
        near(roshan, rules::DEMO_ROSHAN_PIT, 150),
        "Roshan at {roshan:?}"
    );
    let map = map_of(HERO_DEMO_ID);
    assert!(attackable_from(
        map,
        UnitKind::Roshan,
        rules::DEMO_ROSHAN_PIT
    ));
    assert!(!attackable_from(
        map,
        UnitKind::Roshan,
        rules::DEMO_HERO_STARTS[0]
    ));
}

#[test]
fn a_hero_of_the_hero_demo_rises_where_it_fell_four_seconds_later() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let lane = rules::DEMO_LANE_CORNERS[0];
    demo.cheat(Cheat::Teleport { at: lane });
    let fell = demo.kill_hero();

    // The tick it falls on counts as the first of its four seconds.
    demo.run(DEMO_WAIT - 2);
    assert!(
        demo.me().respawn_left > 0,
        "the hero rose before four seconds"
    );
    demo.run(1);

    let risen = demo.hero().expect("the hero stands again").pos;
    assert!(
        near(risen, fell, 100),
        "the hero fell at {fell:?} and rose at {risen:?}"
    );
}

#[test]
fn a_hero_of_the_hero_demo_rises_where_it_fell_each_time_at_any_level() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::Levels {
        count: rules::HERO_MAX_LEVEL,
    });
    for spot in rules::DEMO_LANE_CORNERS {
        demo.cheat(Cheat::Teleport { at: spot });
        let fell = demo.kill_hero();
        demo.run(DEMO_WAIT - 1);
        // The waves fight along the lane: where one stands on the spot, the hero rises on the
        // clear ground the landing search finds beside it.
        let risen = demo.hero().expect("the hero stands again").pos;
        assert!(
            demo.local.world_mut().clearance.stands_clear(risen),
            "it rose on closed ground at {risen:?}"
        );
        assert!(
            near(risen, fell, LANDING_EXTENT),
            "fell at {fell:?}, rose at {risen:?}"
        );
    }
}

#[test]
fn a_hero_of_the_sandbox_map_still_rises_at_its_fountain_after_the_games_wait() {
    let mut demo = Demo::on(DEMO_ID);
    demo.cheat(Cheat::Teleport {
        at: rules::DEMO_LANE_CORNERS[0],
    });
    demo.kill_hero();
    demo.run(DEMO_WAIT - 1);
    assert!(
        demo.me().respawn_left > 0,
        "the hero rose after four seconds"
    );
    demo.run(demo.me().respawn_left);
    let risen = demo.hero().expect("the hero stands again").pos;
    assert!(
        near(
            risen,
            bota_core::hero_spawn_pos(map_of(DEMO_ID), Team::Radiant),
            300
        ),
        "the hero rose at {risen:?}"
    );
}

#[test]
fn nothing_the_player_does_ends_the_hero_demo() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::AllVision { on: true });
    demo.cheat(Cheat::Levels {
        count: rules::HERO_MAX_LEVEL,
    });
    // Both towers fall, Roshan falls, and the hero dies again and again.
    demo.cheat(Cheat::Teleport { at: IN_THE_PIT });
    let marks: Vec<_> = demo
        .view()
        .units
        .iter()
        .filter(|unit| matches!(unit.kind, UnitKind::Tower | UnitKind::Roshan))
        .map(|unit| unit.id)
        .collect();
    assert_eq!(marks.len(), 3, "two towers and Roshan");
    for mark in marks {
        for _ in 0..100 {
            if demo.view().unit(mark).is_none() {
                break;
            }
            demo.cheat(Cheat::Damage {
                target: bota_proto::Target::Unit(mark),
                amount: 1000,
                kind: DamageKind::Pure,
            });
        }
        assert!(demo.view().unit(mark).is_none(), "{mark:?} still stands");
    }
    for _ in 0..3 {
        demo.cheat(Cheat::Refresh);
        demo.kill_hero();
        demo.run(DEMO_WAIT - 1);
    }
    demo.run(2 * 60 * rules::TICKS_PER_SECOND);
    assert!(!demo.over, "the hero demo ended");
    assert!(demo.hero().is_some(), "the hero stands");
}

/// The levels of the abilities a point raises (those that go above one level), in slot order.
fn learned(unit: &UnitView) -> Vec<u8> {
    unit.abilities
        .iter()
        .filter(|ability| ability.max_level > 1)
        .map(|ability| ability.level)
        .collect()
}

#[test]
fn a_player_orders_an_enemy_hero_the_panel_spawned_as_its_own() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let from = demo.unit(pudge).pos;
    let to = rules::DEMO_LANE_CORNERS[2];
    demo.order(
        pudge,
        Order::Move {
            target: Target::Pos(to),
        },
    );
    demo.run(3 * rules::TICKS_PER_SECOND);
    let now = demo.unit(pudge).pos;
    assert!(
        now.distance_squared(to) < from.distance_squared(to),
        "the enemy hero stayed at {now:?}"
    );
    // Teleport, aimed at it, puts it where the player points.
    demo.cheat_at(pudge, Cheat::Teleport { at: to });
    assert!(near(demo.unit(pudge).pos, to, 150), "not teleported");
}

#[test]
fn a_player_levels_an_enemy_hero_and_learns_unlearns_and_maxes_its_abilities() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    demo.cheat_at(pudge, Cheat::Levels { count: 1 });
    assert_eq!(
        demo.row_of(pudge).level,
        2,
        "LEVEL UP raised the aimed hero"
    );
    assert_eq!(demo.me().level, 1, "and not the player's");
    demo.order(
        pudge,
        Order::Learn {
            slot: AbilitySlot(0),
        },
    );
    assert_eq!(
        learned(demo.unit(pudge))[0],
        1,
        "a point learned its first ability"
    );
    demo.cheat_at(
        pudge,
        Cheat::Levels {
            count: rules::HERO_MAX_LEVEL,
        },
    );
    demo.cheat_at(pudge, Cheat::MaxAbilities);
    let maxed: Vec<u8> = demo
        .unit(pudge)
        .abilities
        .iter()
        .filter(|ability| ability.max_level > 1)
        .map(|ability| ability.max_level)
        .collect();
    assert_eq!(
        learned(demo.unit(pudge)),
        maxed,
        "LEVEL MAX learned everything"
    );
    demo.cheat_at(pudge, Cheat::UnlearnAbilities);
    assert!(
        learned(demo.unit(pudge)).iter().all(|level| *level == 0),
        "every point came back: {:?}",
        learned(demo.unit(pudge))
    );
    assert_eq!(
        demo.row_of(pudge).level,
        rules::HERO_MAX_LEVEL,
        "its level stayed"
    );
    demo.order(
        pudge,
        Order::Learn {
            slot: AbilitySlot(3),
        },
    );
    assert_eq!(
        learned(demo.unit(pudge))[3],
        1,
        "the points are there to spend again"
    );
}

#[test]
fn a_player_buys_an_item_for_an_enemy_hero_and_it_lands_in_that_heros_bag() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    // At its own fountain, its home shop; the Hero Demo's purchases cost nothing.
    let home = bota_core::hero_spawn_pos(map_of(HERO_DEMO_ID), Team::Dire);
    demo.cheat_at(pudge, Cheat::Teleport { at: home });
    let gold = demo.me().gold;
    demo.order(
        pudge,
        Order::Buy {
            item: ItemId::BLINK_DAGGER,
        },
    );
    let carried = demo
        .unit(pudge)
        .items
        .iter()
        .flatten()
        .any(|item| item.id == ItemId::BLINK_DAGGER);
    assert!(carried, "the Blink Dagger is in the enemy hero's bag");
    assert_eq!(demo.me().gold, gold, "the player paid nothing");
}

#[test]
fn a_purchase_in_the_hero_demo_costs_nothing() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::Teleport {
        at: bota_core::hero_spawn_pos(map_of(HERO_DEMO_ID), Team::Radiant),
    });
    let gold = demo.me().gold;
    demo.step(&[Order::Buy {
        item: ItemId::BLINK_DAGGER,
    }]);
    assert_eq!(
        demo.rejected,
        [],
        "a Blink Dagger costs more than the starting gold"
    );
    assert_eq!(demo.me().gold, gold);
}

#[test]
fn a_removed_hero_leaves_the_match_and_its_place_serves_the_next() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let slot = demo.row_of(pudge).slot;
    demo.cheat_at(pudge, Cheat::RemoveHero);
    assert!(demo.view().player(slot).is_none(), "its seat is gone");
    assert!(demo.view().unit(pudge).is_none(), "its hero is gone");
    let next = demo.spawn_enemy(HeroId::SHADOW_FIEND);
    assert_eq!(
        demo.row_of(next).slot,
        slot,
        "the next hero takes the place"
    );
}

#[test]
fn the_panel_cannot_remove_the_players_own_hero_nor_aim_at_nothing() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let own = demo.me().unit.expect("the hero");
    for unit in [None, Some(own)] {
        demo.step_naming(&[(
            unit,
            Order::Cheat {
                cheat: Cheat::RemoveHero,
            },
        )]);
        assert_eq!(demo.rejected, [RejectReason::BadCheat], "aimed at {unit:?}");
    }
    assert!(demo.hero().is_some());
}

#[test]
fn a_hero_the_panel_spawned_dies_and_rises_where_it_fell() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let spot = rules::DEMO_LANE_CORNERS[2];
    demo.cheat_at(pudge, Cheat::Teleport { at: spot });
    demo.cheat_at(pudge, Cheat::Suicide);
    assert!(demo.view().unit(pudge).is_none(), "it lies dead");
    demo.run(DEMO_WAIT - 1);
    assert!(near(demo.unit(pudge).pos, spot, 150), "it rose elsewhere");
}

/// The new cheats of the Hero Demo, each as the panel gives it.
const PANEL_CHEATS: [Cheat; 3] = [
    Cheat::RemoveHero,
    Cheat::MaxAbilities,
    Cheat::UnlearnAbilities,
];

#[test]
fn a_match_without_cheats_refuses_the_hero_demos_cheats_and_control_of_other_heroes() {
    // What a matchmaking service launches for people playing each other, rated or a lobby
    // without cheats, and a match started directly: none honours cheats.
    let cfg = MatchConfig {
        picks: super::support::fiend_against_fiend(),
        ..config()
    };
    assert!(!cfg.cheats);
    let (mut local, opening) =
        LocalMatch::new(&cfg, &[OrderDelay::NONE, OrderDelay::NONE]).expect("a match");
    let enemy = opening[1]
        .iter()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => view.player(SlotId(1)).and_then(|row| row.unit),
            _ => None,
        })
        .expect("the other side's hero");
    let mut seq = 0;
    for cheat in PANEL_CHEATS {
        for (unit, why) in [
            (None, RejectReason::NoCheats),
            (Some(enemy), RejectReason::NotYourUnit),
        ] {
            seq += 1;
            let order = OutgoingOrder {
                seq,
                unit,
                order: Order::Cheat { cheat },
                seen: local.tick(),
                queue: false,
            };
            let streams = local.step(&[&[order], &[]]).expect("a step");
            let refused: Vec<_> = streams[0]
                .iter()
                .filter_map(|msg| match msg {
                    ServerMsg::OrderRejected { reason, .. } => Some(*reason),
                    _ => None,
                })
                .collect();
            assert_eq!(refused, [why], "{cheat:?} aimed at {unit:?}");
        }
    }
    // Nor does anyone order the other side's hero.
    seq += 1;
    let order = OutgoingOrder {
        seq,
        unit: Some(enemy),
        order: Order::Move {
            target: Target::None,
        },
        seen: local.tick(),
        queue: false,
    };
    let streams = local.step(&[&[order], &[]]).expect("a step");
    assert!(
        streams[0].iter().any(|msg| matches!(
            msg,
            ServerMsg::OrderRejected {
                reason: RejectReason::NotYourUnit,
                ..
            }
        )),
        "the other side's hero took an order"
    );
}

#[test]
fn with_cheats_on_a_player_still_cannot_order_another_players_hero() {
    let cfg = MatchConfig {
        picks: super::support::fiend_against_fiend(),
        cheats: true,
        ..config()
    };
    let (mut local, opening) =
        LocalMatch::new(&cfg, &[OrderDelay::NONE, OrderDelay::NONE]).expect("a match");
    let other = opening[1]
        .iter()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => view.player(SlotId(1)).and_then(|row| row.unit),
            _ => None,
        })
        .expect("the other player's hero");
    for order in [
        Order::Move {
            target: Target::None,
        },
        Order::Cheat {
            cheat: Cheat::Levels { count: 1 },
        },
    ] {
        let given = OutgoingOrder {
            seq: 1,
            unit: Some(other),
            order,
            seen: local.tick(),
            queue: false,
        };
        let streams = local.step(&[&[given], &[]]).expect("a step");
        assert!(
            streams[0].iter().any(|msg| matches!(
                msg,
                ServerMsg::OrderRejected {
                    reason: RejectReason::NotYourUnit,
                    ..
                }
            )),
            "{order:?} reached another player's hero"
        );
    }
}

/// The demo map's Radiant tower, whose footprint nothing may stand in.
fn radiant_tower() -> Vec2 {
    rules::DEMO_RADIANT_TOWERS[0].2
}

#[test]
fn a_hero_whose_spot_is_taken_by_a_structure_rises_on_clear_ground_beside_it() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::Teleport {
        at: radiant_tower() + Vec2::from_ints(0, 300),
    });
    demo.kill_hero();
    // As where a hero thrown through the air falls: the spot it is remembered at lies in the
    // tower's footprint.
    let world = demo.local.world_mut();
    world.seats[0].respawn_spot.at = Some(radiant_tower());
    demo.run(DEMO_WAIT - 1);
    let risen = demo.hero().expect("the hero stands again").pos;
    let world = demo.local.world_mut();
    assert!(
        world.clearance.stands_clear(risen),
        "it rose in the tower at {risen:?}"
    );
    assert!(
        near(risen, radiant_tower(), LANDING_EXTENT),
        "it rose far off, at {risen:?}"
    );
}

#[test]
fn a_hero_whose_spot_another_hero_holds_rises_beside_it() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let spot = rules::DEMO_LANE_CORNERS[0];
    demo.cheat(Cheat::Teleport { at: spot });
    let fell = demo.kill_hero();
    // An allied hero the panel spawns stands idle on the very spot.
    demo.cheat(Cheat::CreateHero {
        hero: HeroId::PUDGE,
        enemy: false,
        at: fell,
    });
    let ally = demo
        .view()
        .players
        .last()
        .and_then(|row| row.unit)
        .expect("the ally");
    let held = demo.unit(ally).pos;
    assert!(near(held, fell, 50), "the ally stands at {held:?}");
    demo.run(DEMO_WAIT - 2);
    let risen = demo.hero().expect("the hero stands again").pos;
    let held = demo.unit(ally).pos;
    let apart = rules::units(
        bota_core::hero_def(HeroId::PUDGE)
            .expect("Pudge")
            .unit
            .collision,
    );
    assert!(
        !risen.within(held, apart),
        "the hero rose on the ally at {held:?}"
    );
    assert!(near(risen, fell, 400), "it rose far off, at {risen:?}");
}

#[test]
fn a_hero_that_fell_under_the_enemy_fountain_rises_protected_at_its_own() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let enemy_fountain = map_of(HERO_DEMO_ID).fountains[1];
    demo.cheat(Cheat::Teleport {
        at: enemy_fountain + Vec2::from_ints(-600, -600),
    });
    demo.kill_hero();
    demo.run(DEMO_WAIT - 1);
    let hero = demo.hero().expect("the hero stands again");
    let home = bota_core::hero_spawn_pos(map_of(HERO_DEMO_ID), Team::Radiant);
    assert!(near(hero.pos, home, 300), "it rose at {:?}", hero.pos);
    assert!(
        hero.statuses.bits & bota_proto::StatusFlags::INVULNERABLE != 0,
        "risen at its fountain, it is untouchable as any respawn there"
    );
}

#[test]
fn a_bought_back_hero_of_the_hero_demo_rises_where_it_fell_and_owes_no_longer_wait() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.cheat(Cheat::Gold { amount: 10_000 });
    let spot = rules::DEMO_LANE_CORNERS[1];
    demo.cheat(Cheat::Teleport { at: spot });
    let fell = demo.kill_hero();
    demo.step(&[Order::Buyback]);
    assert_eq!(demo.rejected, [], "buyback is there as anywhere");
    let risen = demo.hero().expect("bought back at once").pos;
    assert!(
        near(risen, fell, 150),
        "fell at {fell:?}, rose at {risen:?}"
    );
    // The next death lies the fixed four seconds, the buyback's penalty forgotten.
    demo.kill_hero();
    demo.run(DEMO_WAIT - 1);
    assert!(
        demo.hero().is_some(),
        "the death after a buyback lasted longer"
    );
}

#[test]
fn a_hero_with_no_clear_ground_near_its_spot_waits_a_second_and_then_rises_at_its_fountain() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.kill_hero();
    demo.local.world_mut().seats[0].respawn_spot.at = Some(NO_GROUND);
    demo.run(DEMO_WAIT - 1);
    demo.run(rules::DEMO_RISE_WAIT_TICKS - 1);
    assert!(
        demo.me().respawn_left > 0,
        "it rose with no ground to rise on"
    );
    demo.run(1);
    let risen = demo.hero().expect("it rose at last").pos;
    let home = bota_core::hero_spawn_pos(map_of(HERO_DEMO_ID), Team::Radiant);
    assert!(near(risen, home, 300), "it rose at {risen:?}");
}

#[test]
fn a_hero_whose_spot_clears_while_it_waits_rises_beside_the_spot() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    demo.kill_hero();
    demo.local.world_mut().seats[0].respawn_spot.at = Some(NO_GROUND);
    demo.run(DEMO_WAIT - 1 + 5);
    assert!(
        demo.me().respawn_left > 0,
        "it rose with no ground to rise on"
    );
    // The ground clears: its spot is open lane now.
    let lane = rules::DEMO_LANE_CORNERS[0];
    demo.local.world_mut().seats[0].respawn_spot.at = Some(lane);
    demo.run(1);
    let risen = demo.hero().expect("it rose").pos;
    assert!(near(risen, lane, LANDING_EXTENT), "it rose at {risen:?}");
}

#[test]
fn a_hero_rises_at_its_fountain_only_when_it_fell_within_the_enemy_fountains_reach() {
    let hero = bota_core::hero_def(HeroId::SHADOW_FIEND)
        .expect("Shadow Fiend")
        .unit;
    let reach = crate::game::systems::fountain_reach(hero).to_int();
    let enemy_fountain = map_of(HERO_DEMO_ID).fountains[1];
    let home = bota_core::hero_spawn_pos(map_of(HERO_DEMO_ID), Team::Radiant);
    // Along the diagonal out of the Dire base: 7071 / 10000 of a distance on each axis.
    let out = |distance: i32| {
        let side = distance * 7071 / 10000;
        enemy_fountain - Vec2::from_ints(side, side)
    };
    for (distance, at_home) in [(reach - 60, true), (reach + 60, false)] {
        let mut demo = Demo::on(HERO_DEMO_ID);
        demo.kill_hero();
        demo.local.world_mut().seats[0].respawn_spot.at = Some(out(distance));
        demo.run(DEMO_WAIT - 1);
        let risen = demo.hero().expect("it rose").pos;
        assert_eq!(
            near(risen, home, 300),
            at_home,
            "fell {distance} from the enemy fountain (reach {reach}), rose at {risen:?}"
        );
    }
}

/// Two players in one match with cheats on, Radiant in seat 0 and Dire in seat 1, as a lobby
/// whose owner turned cheats on plays the Hero Demo.
struct Pair {
    local: LocalMatch,
    seq: u32,
    views: [Option<WorldView>; 2],
    rejected: [Vec<RejectReason>; 2],
}

impl Pair {
    fn on(map: MapId, cheats: bool) -> Pair {
        let cfg = MatchConfig {
            map,
            cheats,
            picks: super::support::fiend_against_fiend(),
            objectives: map_of(map).default_objectives,
            runes: map_of(map).default_runes,
            ..config()
        };
        let (local, opening) =
            LocalMatch::new(&cfg, &[OrderDelay::NONE, OrderDelay::NONE]).expect("a match");
        let mut pair = Pair {
            local,
            seq: 0,
            views: [None, None],
            rejected: [Vec::new(), Vec::new()],
        };
        pair.take(opening);
        pair
    }

    fn take(&mut self, streams: Vec<Vec<ServerMsg>>) {
        for (seat, stream) in streams.into_iter().enumerate() {
            self.rejected[seat].clear();
            for msg in stream {
                match msg {
                    ServerMsg::Snapshot { view } => self.views[seat] = Some(view),
                    ServerMsg::OrderRejected { reason, .. } => self.rejected[seat].push(reason),
                    _ => {}
                }
            }
        }
    }

    /// One tick in which `seat` gives one order naming `unit`.
    fn give(&mut self, seat: usize, unit: Option<EntityId>, order: Order) {
        self.seq += 1;
        let given = [OutgoingOrder {
            seq: self.seq,
            unit,
            order,
            seen: self.local.tick(),
            queue: false,
        }];
        let mut orders: [&[OutgoingOrder]; 2] = [&[], &[]];
        orders[seat] = &given;
        let streams = self.local.step(&orders).expect("a step");
        self.take(streams);
    }

    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            let streams = self.local.step(&[&[], &[]]).expect("a step");
            self.take(streams);
        }
    }

    fn view(&self, seat: usize) -> &WorldView {
        self.views[seat].as_ref().expect("a snapshot")
    }

    /// The hero seat 1 stands up on its own side with the panel's ALLY, a created hero of Dire.
    fn dire_creates_an_ally(&mut self) -> EntityId {
        self.give(
            1,
            None,
            Order::Cheat {
                cheat: Cheat::CreateHero {
                    hero: HeroId::PUDGE,
                    enemy: false,
                    at: rules::DEMO_LANE_CORNERS[2],
                },
            },
        );
        assert_eq!(self.rejected[1], []);
        let row = self.view(1).players.last().expect("the created seat");
        assert!(row.slot.0 >= 2 && row.team == Team::Dire, "{row:?}");
        row.unit.expect("its hero")
    }
}

#[test]
fn a_player_who_did_not_create_a_hero_controls_it_too() {
    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    // Seat 0, of the other side, orders the hero seat 1 created.
    let to = rules::DEMO_LANE_CORNERS[0];
    pair.give(
        0,
        Some(pudge),
        Order::Move {
            target: Target::Pos(to),
        },
    );
    assert_eq!(
        pair.rejected[0],
        [],
        "the second player's order was turned down"
    );
    // Its own side sees it walk.
    let at = |pair: &Pair| pair.view(1).unit(pudge).expect("its side sees it").pos;
    let from = at(&pair);
    pair.run(3 * rules::TICKS_PER_SECOND);
    let now = at(&pair);
    assert!(
        now.distance_squared(to) < from.distance_squared(to),
        "it stayed at {now:?}"
    );
}

#[test]
fn a_player_ordering_the_other_sides_created_hero_spends_only_its_own_sides_glyph_and_scan() {
    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    // Both sides' Glyphs ready and both sides' first Scan charges in.
    pair.run(rules::GLYPH_FIRST_USE_AFTER_HORN);
    let world = pair.local.world_mut();
    let now = world.tick;
    assert!(world.glyphs.iter().all(|glyph| glyph.ready_at <= now));
    let charges =
        |world: &crate::game::World| [world.scan_ledger(0).charges, world.scan_ledger(1).charges];
    let before = charges(world);
    assert!(before.iter().all(|held| *held > 0), "{before:?}");
    // Seat 0 (Radiant) names Dire's created hero for the Glyph and the Scan.
    pair.give(0, Some(pudge), Order::Glyph);
    assert_eq!(pair.rejected[0], []);
    pair.give(
        0,
        Some(pudge),
        Order::Scan {
            at: rules::DEMO_LANE_CORNERS[1],
        },
    );
    assert_eq!(pair.rejected[0], []);
    let world = pair.local.world_mut();
    let now = world.tick;
    assert!(
        world.glyphs[0].ready_at > now,
        "Radiant's Glyph was not the one used"
    );
    assert!(
        world.glyphs[1].ready_at <= now,
        "Dire's Glyph was spent by a Radiant player"
    );
    assert_eq!(
        charges(world),
        [before[0] - 1, before[1]],
        "Dire's Scan was spent by a Radiant player"
    );
}

#[test]
fn every_order_names_whose_it_is_and_only_a_heros_doing_reaches_a_controlled_hero() {
    use crate::game::systems::{Reach, order_reach};
    let none = Target::None;
    let unit = [
        Order::Move { target: none },
        Order::Attack { target: none },
        Order::Cast {
            slot: AbilitySlot(0),
            target: none,
        },
        Order::Buy {
            item: ItemId::BLINK_DAGGER,
        },
        Order::Learn {
            slot: AbilitySlot(0),
        },
        Order::LearnAttributeBonus,
    ];
    let seat = [
        Order::Glyph,
        Order::Buyback,
        Order::Scan { at: Vec2::ZERO },
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    ];
    assert!(unit.iter().all(|order| order_reach(order) == Reach::Unit));
    assert!(seat.iter().all(|order| order_reach(order) == Reach::Seat));
}

#[test]
fn with_cheats_off_nobody_controls_a_created_hero_through_the_same_door() {
    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    // The same world with its cheats gone: the one gate closes control.
    pair.local.world_mut().cheats = false;
    pair.give(
        0,
        Some(pudge),
        Order::Move {
            target: Target::None,
        },
    );
    assert_eq!(pair.rejected[0], [RejectReason::NotYourUnit]);
    pair.give(
        1,
        Some(pudge),
        Order::Move {
            target: Target::None,
        },
    );
    assert_eq!(pair.rejected[1], [RejectReason::NotYourUnit]);
}

#[test]
fn an_order_naming_a_removed_hero_is_refused_even_after_its_place_is_taken_again() {
    let mut demo = Demo::on(HERO_DEMO_ID);
    let removed = demo.spawn_enemy(HeroId::PUDGE);
    let place = demo.row_of(removed).slot;
    demo.cheat_at(removed, Cheat::RemoveHero);
    let still = Order::Move {
        target: Target::None,
    };
    demo.step_naming(&[(Some(removed), still)]);
    assert_eq!(
        demo.rejected,
        [RejectReason::NotYourUnit],
        "the removed hero took an order"
    );
    let next = demo.spawn_enemy(HeroId::SHADOW_FIEND);
    assert_eq!(
        demo.row_of(next).slot,
        place,
        "the new hero took the old one's place"
    );
    assert_ne!(next, removed);
    demo.step_naming(&[(Some(removed), still)]);
    assert_eq!(
        demo.rejected,
        [RejectReason::NotYourUnit],
        "the old handle reached the new hero"
    );
    demo.order(next, still);
}

#[test]
fn a_match_set_up_in_process_refuses_the_hero_demo_without_cheats() {
    // In process, as tests and environments build a match.
    let cfg = MatchConfig {
        map: HERO_DEMO_ID,
        cheats: false,
        objectives: map_of(HERO_DEMO_ID).default_objectives,
        runes: map_of(HERO_DEMO_ID).default_runes,
        ..config()
    };
    let refused = LocalMatch::new(&cfg, &[OrderDelay::NONE])
        .err()
        .expect("refused");
    assert_eq!(
        refused.to_string(),
        "match setup was refused: map 3 is played with cheats on"
    );
}

#[test]
fn a_controlled_heros_purchases_and_sales_are_its_own_seats_and_the_givers_gold_never_moves() {
    // A cheat match on the sandbox map, whose purchases cost what the shop asks.
    let mut pair = Pair::on(DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    let place = pair.view(1).players.last().expect("its row").slot;
    let gold = |pair: &mut Pair, slot: SlotId| {
        let world = pair.local.world_mut();
        world.seat(slot).expect("the seat").gold
    };
    // Seat 0 gives the created hero gold (a cheat lands on the hero it is aimed at), takes it
    // to its own fountain's shop, and buys and sells for it.
    pair.give(
        0,
        Some(pudge),
        Order::Cheat {
            cheat: Cheat::Gold { amount: 5_000 },
        },
    );
    assert_eq!(pair.rejected[0], []);
    let home = bota_core::hero_spawn_pos(map_of(DEMO_ID), Team::Dire);
    pair.give(
        0,
        Some(pudge),
        Order::Cheat {
            cheat: Cheat::Teleport { at: home },
        },
    );
    assert_eq!(pair.rejected[0], []);
    let (giver, hero) = (gold(&mut pair, SlotId(0)), gold(&mut pair, place));
    assert_eq!(hero, 5_000);
    pair.give(
        0,
        Some(pudge),
        Order::Buy {
            item: ItemId::BLINK_DAGGER,
        },
    );
    assert_eq!(pair.rejected[0], []);
    let cost = bota_core::item_def(ItemId::BLINK_DAGGER)
        .expect("Blink")
        .cost;
    assert_eq!(gold(&mut pair, place), hero - cost, "the hero's seat paid");
    assert_eq!(gold(&mut pair, SlotId(0)), giver, "the giver paid nothing");
    pair.give(
        0,
        Some(pudge),
        Order::Sell {
            slot: bota_proto::ItemSlot(0),
        },
    );
    assert_eq!(pair.rejected[0], []);
    assert!(
        gold(&mut pair, place) > hero - cost,
        "the sale credited the hero's seat"
    );
    assert_eq!(
        gold(&mut pair, SlotId(0)),
        giver,
        "the sale credited the giver nothing"
    );
}

#[test]
fn a_player_cannot_buy_back_a_hero_it_controls_for_it_is_another_seats() {
    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    pair.give(
        0,
        Some(pudge),
        Order::Cheat {
            cheat: Cheat::Suicide,
        },
    );
    assert_eq!(pair.rejected[0], []);
    pair.give(0, Some(pudge), Order::Buyback);
    assert_eq!(pair.rejected[0], [RejectReason::NotYourUnit]);
}

#[test]
fn removing_a_non_last_created_hero_and_creating_another_keeps_seats_sorted_and_assists_intact() {
    use bota_client_core::{ClientSession, MatchStatic, SessionOptions};

    let cfg = MatchConfig {
        map: HERO_DEMO_ID,
        cheats: true,
        objectives: map_of(HERO_DEMO_ID).default_objectives,
        runes: map_of(HERO_DEMO_ID).default_runes,
        ..config()
    };
    let (mut local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session: Option<ClientSession> = None;
    let mut seq = 0u32;
    let mut fold = |streams: Vec<Vec<ServerMsg>>| -> (WorldView, Vec<RejectReason>) {
        let mut rejected = Vec::new();
        for msg in streams.into_iter().flatten() {
            if let ServerMsg::MatchStart { ref info } = msg {
                let statics = MatchStatic::new(info).expect("valid match info");
                session = Some(
                    ClientSession::seat(statics, SlotId(0), None, SessionOptions::default())
                        .expect("a client session"),
                );
                continue;
            }
            if let ServerMsg::OrderRejected { reason, .. } = msg {
                rejected.push(reason);
            }
            session
                .as_mut()
                .expect("session started")
                .ingest(msg, &mut ())
                .expect("client session accepts every server message");
        }
        let view = session
            .as_ref()
            .and_then(|s| s.current())
            .expect("a snapshot")
            .clone();
        (view, rejected)
    };
    let _ = fold(opening);
    let mut step = |given: Option<(Option<EntityId>, Order)>| -> WorldView {
        let out: Vec<OutgoingOrder> = given
            .into_iter()
            .map(|(unit, order)| {
                seq += 1;
                OutgoingOrder {
                    seq,
                    unit,
                    order,
                    seen: local.tick(),
                    queue: false,
                }
            })
            .collect();
        let (next, rejected) = fold(local.step(&[&out]).expect("a step"));
        assert_eq!(rejected, [], "{given:?} was refused");
        next
    };

    let _ = step(Some((
        None,
        Order::Cheat {
            cheat: Cheat::AllVision { on: true },
        },
    )));
    let spot = rules::DEMO_LANE_CORNERS[1];
    let view = step(Some((
        None,
        Order::Cheat {
            cheat: Cheat::CreateHero {
                hero: HeroId::PUDGE,
                enemy: true,
                at: spot,
            },
        },
    )));
    let hero_a = view
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("hero A in slot 1");

    let view = step(Some((
        None,
        Order::Cheat {
            cheat: Cheat::CreateHero {
                hero: HeroId::SHADOW_FIEND,
                enemy: true,
                at: spot + Vec2::from_ints(200, 0),
            },
        },
    )));
    let hero_b = view
        .player(SlotId(2))
        .and_then(|row| row.unit)
        .expect("hero B in slot 2");

    // Hero B hurts the player's hero so seat 0 records hero B's seat index in its assist ledger.
    let own = view
        .player(SlotId(0))
        .and_then(|row| row.unit)
        .expect("own hero");
    let own_pos = view.unit(own).expect("own unit").pos;
    let _ = step(Some((
        Some(hero_b),
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: own_pos + Vec2::from_ints(150, 0),
            },
        },
    )));
    let mut view = step(Some((
        Some(hero_b),
        Order::Attack {
            target: Target::Unit(own),
        },
    )));
    let hp_before = view.unit(own).expect("own unit").hp;
    for _ in 0..3 * rules::TICKS_PER_SECOND {
        view = step(None);
        if view.unit(own).is_some_and(|u| u.hp < hp_before) {
            break;
        }
    }
    assert!(
        view.unit(own).expect("own unit").hp < hp_before,
        "hero B damaged own hero"
    );

    // Remove the non-last created hero (slot 1), then create another hero that reuses slot 1.
    let view = step(Some((
        Some(hero_a),
        Order::Cheat {
            cheat: Cheat::RemoveHero,
        },
    )));
    assert!(view.player(SlotId(1)).is_none(), "slot 1 is free");

    let view = step(Some((
        None,
        Order::Cheat {
            cheat: Cheat::CreateHero {
                hero: HeroId::PUDGE,
                enemy: true,
                at: spot,
            },
        },
    )));
    assert!(
        view.players.windows(2).all(|w| w[0].slot < w[1].slot),
        "players must be sorted by slot: {:?}",
        view.players.iter().map(|p| p.slot).collect::<Vec<_>>()
    );
    assert!(
        view.talents.windows(2).all(|w| w[0].slot < w[1].slot),
        "talents must be sorted by slot: {:?}",
        view.talents.iter().map(|t| t.slot).collect::<Vec<_>>()
    );
    let hero_c = view
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("hero C looked up via sorted players");

    // Orders to hero C work.
    let from = view.unit(hero_c).expect("hero C stands").pos;
    let to = rules::DEMO_LANE_CORNERS[2];
    let mut view = step(Some((
        Some(hero_c),
        Order::Move {
            target: Target::Pos(to),
        },
    )));
    for _ in 0..2 * rules::TICKS_PER_SECOND {
        view = step(None);
    }
    let now = view.unit(hero_c).expect("hero C stands").pos;
    assert!(
        now.distance_squared(to) < from.distance_squared(to),
        "hero C moved toward {to:?}, stayed at {now:?}"
    );

    // Kill the player's hero with sourceless cheat damage: hero B (slot 2) keeps its assist,
    // and hero C (slot 1, newly inserted before hero B) gets none.
    for _ in 0..10 {
        if view.player(SlotId(0)).expect("own row").respawn_left > 0 {
            break;
        }
        view = step(Some((
            None,
            Order::Cheat {
                cheat: Cheat::Damage {
                    target: Target::None,
                    amount: 1000,
                    kind: DamageKind::Pure,
                },
            },
        )));
    }
    assert!(
        view.player(SlotId(0)).expect("own row").respawn_left > 0,
        "own hero died"
    );
    assert_eq!(
        view.player(SlotId(2)).expect("hero B").assists,
        1,
        "hero B kept its assist credit across seat insertion"
    );
    assert_eq!(
        view.player(SlotId(1)).expect("hero C").assists,
        0,
        "hero C did not inherit hero B's assist credit"
    );
}

/// The ability slot of `ability` on `unit`, as the view lists its book.
fn ability_slot_of(unit: &UnitView, ability: bota_proto::AbilityId) -> AbilitySlot {
    let idx = unit
        .abilities
        .iter()
        .position(|a| a.id == ability)
        .unwrap_or_else(|| panic!("{ability:?} missing from unit book"));
    AbilitySlot(idx as u8)
}

#[test]
fn removing_a_created_hero_with_rot_on_clears_the_rot_mark() {
    use bota_proto::{AbilityId, Inflictor};

    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let rot = ability_slot_of(demo.unit(pudge), AbilityId::ROT);
    demo.order(pudge, Order::Learn { slot: rot });
    demo.order(
        pudge,
        Order::Cast {
            slot: rot,
            target: Target::None,
        },
    );
    demo.run(2);
    assert!(
        demo.view()
            .projectiles
            .iter()
            .any(|p| p.inflictor == Some(Inflictor::Ability(AbilityId::ROT))),
        "Rot mark is shown while Rot burns"
    );

    demo.cheat_at(pudge, Cheat::RemoveHero);
    demo.run(1);
    assert!(
        demo.view()
            .projectiles
            .iter()
            .all(|p| p.inflictor != Some(Inflictor::Ability(AbilityId::ROT))),
        "Rot mark survived RemoveHero"
    );
}

#[test]
fn removing_a_created_hero_channelling_dismember_ends_stun_bites_and_mark() {
    use bota_proto::{AbilityId, Inflictor, StatusFlags};

    let mut demo = Demo::on(HERO_DEMO_ID);
    let own = demo.me().unit.expect("own hero");
    // Give own hero high max HP so Dismember bites never kill it and damage is cleanly observable.
    demo.cheat(Cheat::Levels { count: 25 });
    demo.cheat(Cheat::Refresh);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    demo.cheat_at(pudge, Cheat::Levels { count: 10 });
    let dismember = ability_slot_of(demo.unit(pudge), AbilityId::DISMEMBER);
    demo.order(pudge, Order::Learn { slot: dismember });
    let own_pos = demo.unit(own).pos;
    demo.cheat_at(
        pudge,
        Cheat::Teleport {
            at: own_pos + Vec2::from_ints(120, 0),
        },
    );
    demo.order(
        pudge,
        Order::Cast {
            slot: dismember,
            target: Target::Unit(own),
        },
    );
    // Advance until Dismember takes hold (cast point finishes and victim is stunned).
    for _ in 0..rules::TICKS_PER_SECOND {
        demo.run(1);
        if demo.unit(own).statuses.bits & StatusFlags::STUNNED != 0 {
            break;
        }
    }
    assert_ne!(
        demo.unit(own).statuses.bits & StatusFlags::STUNNED,
        0,
        "Dismember stunned the victim"
    );
    assert!(
        demo.view()
            .projectiles
            .iter()
            .any(|p| p.inflictor == Some(Inflictor::Ability(AbilityId::DISMEMBER))),
        "Dismember mark is shown during channel"
    );

    demo.cheat_at(pudge, Cheat::RemoveHero);
    assert_eq!(
        demo.unit(own).statuses.bits & StatusFlags::STUNNED,
        0,
        "victim must not remain stunned after RemoveHero"
    );
    assert!(
        demo.view()
            .projectiles
            .iter()
            .all(|p| p.inflictor != Some(Inflictor::Ability(AbilityId::DISMEMBER))),
        "Dismember mark must not survive RemoveHero"
    );
    let hp_after_removal = demo.unit(own).hp;
    // Run through the rest of the channel duration: no phantom bites may reduce HP.
    for _ in 0..4 * rules::TICKS_PER_SECOND {
        demo.run(1);
        let hp = demo.unit(own).hp;
        assert!(
            hp >= hp_after_removal,
            "victim took post-removal Dismember bite: hp {hp} < {hp_after_removal}"
        );
    }
}

#[test]
fn removing_a_created_hero_channelling_a_twin_gate_warp_puts_both_gates_out() {
    use bota_core::DOTA_TWIN_GATES;
    use bota_proto::{EffectId, ObjectiveKind, ObjectiveSet};

    let cfg = MatchConfig {
        map: MapId(2),
        cheats: true,
        objectives: ObjectiveSet::of(ObjectiveKind::TwinGates),
        runes: bota_proto::RuneSet::NONE,
        ..config()
    };
    let (local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut demo = Demo {
        local,
        seq: 0,
        view: None,
        rejected: Vec::new(),
        over: false,
    };
    demo.take(opening);
    demo.cheat(Cheat::AllVision { on: true });

    let gate_pos = DOTA_TWIN_GATES[0].at;
    let far_gate_pos = DOTA_TWIN_GATES[1].at;
    // Stand own hero near the far gate so Radiant sees both gates' glow.
    demo.cheat(Cheat::Teleport {
        at: far_gate_pos + Vec2::from_ints(0, 170),
    });
    let gate_id = demo
        .view()
        .units
        .iter()
        .find(|u| u.kind == UnitKind::TwinGate && near(u.pos, gate_pos, 100))
        .expect("north-west Twin Gate")
        .id;

    demo.cheat(Cheat::CreateHero {
        hero: HeroId::PUDGE,
        enemy: false,
        at: gate_pos + Vec2::from_ints(0, -170),
    });
    let created = demo
        .view()
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("created hero");
    demo.order(
        created,
        Order::Move {
            target: Target::Unit(gate_id),
        },
    );
    for _ in 0..2 * rules::TICKS_PER_SECOND {
        demo.run(1);
        let lit = demo
            .units(UnitKind::TwinGate)
            .filter(|g| g.effects.iter().any(|e| e.id == EffectId::TWIN_GATE_ACTIVE))
            .count();
        if lit == 2 {
            break;
        }
    }
    assert_eq!(
        demo.units(UnitKind::TwinGate)
            .filter(|g| g.effects.iter().any(|e| e.id == EffectId::TWIN_GATE_ACTIVE))
            .count(),
        2,
        "both Twin Gates glow while channelling warp"
    );

    demo.cheat_at(created, Cheat::RemoveHero);
    demo.run(1);
    let still_lit = demo
        .units(UnitKind::TwinGate)
        .filter(|g| g.effects.iter().any(|e| e.id == EffectId::TWIN_GATE_ACTIVE))
        .count();
    assert_eq!(still_lit, 0, "Twin Gates stayed lit after RemoveHero");
}

#[test]
fn removing_a_created_hero_channelling_an_outpost_capture_releases_capture_progress() {
    use bota_core::CAPTURE_TICKS;
    use bota_proto::{ObjectiveKind, ObjectiveSet, StatusFlags};

    let dota_map = MapId(0);
    let cfg = MatchConfig {
        map: dota_map,
        cheats: true,
        objectives: ObjectiveSet::of(ObjectiveKind::Outposts),
        runes: bota_proto::RuneSet::NONE,
        ..config()
    };
    let (local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut demo = Demo {
        local,
        seq: 0,
        view: None,
        rejected: Vec::new(),
        over: false,
    };
    demo.take(opening);
    demo.cheat(Cheat::AllVision { on: true });
    demo.cheat(Cheat::StartGame);
    demo.run(2);

    // Bring down Dire's mid tier-1 and tier-2 towers so Dire's Outpost loses its capture guard.
    for tier in [1u8, 2u8] {
        let (_, _, tower_pos) = *map_of(dota_map)
            .dire_towers
            .iter()
            .find(|(lane, t, _)| *lane == rules::LANE_MID && *t == tier)
            .expect("a Dire mid tower");
        let tower = demo
            .view()
            .units
            .iter()
            .find(|u| u.kind == UnitKind::Tower && near(u.pos, tower_pos, 100))
            .expect("Dire mid tower")
            .id;
        for _ in 0..20 {
            if demo.view().unit(tower).is_none() {
                break;
            }
            demo.cheat(Cheat::Damage {
                target: Target::Unit(tower),
                amount: 1000,
                kind: DamageKind::Pure,
            });
        }
        assert!(demo.view().unit(tower).is_none(), "Dire mid T{tier} fell");
    }

    let dire_outpost_pos = map_of(dota_map)
        .outposts
        .iter()
        .find(|o| o.home == Team::Dire)
        .expect("Dire outpost")
        .at;
    let outpost = demo
        .view()
        .units
        .iter()
        .find(|u| u.kind == UnitKind::Outpost && near(u.pos, dire_outpost_pos, 100))
        .expect("Dire outpost unit")
        .id;
    assert_eq!(demo.unit(outpost).team, Team::Dire);

    // Spawn a created Radiant hero beside Dire's Outpost and channel halfway through capture.
    demo.cheat(Cheat::CreateHero {
        hero: HeroId::PUDGE,
        enemy: false,
        at: dire_outpost_pos + Vec2::from_ints(120, 0),
    });
    let created = demo
        .view()
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("created hero");
    demo.order(
        created,
        Order::Move {
            target: Target::Unit(outpost),
        },
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        demo.run(1);
        if demo.unit(created).statuses.bits & StatusFlags::CHANNELLING != 0 {
            break;
        }
    }
    assert_ne!(
        demo.unit(created).statuses.bits & StatusFlags::CHANNELLING,
        0,
        "created hero is channelling Outpost capture"
    );
    demo.run(CAPTURE_TICKS / 2);
    assert_eq!(demo.unit(outpost).team, Team::Dire, "halfway, not yet captured");

    // Remove the created hero mid-capture.
    demo.cheat_at(created, Cheat::RemoveHero);

    // Now the player's own Radiant hero channels the same Outpost: it must take a full
    // CAPTURE_TICKS from scratch rather than inheriting the removed hero's progress.
    demo.cheat(Cheat::Teleport {
        at: dire_outpost_pos + Vec2::from_ints(120, 0),
    });
    demo.step(&[Order::Move {
        target: Target::Unit(outpost),
    }]);
    assert_eq!(demo.rejected, []);
    let mut channel_ticks = 0u32;
    for _ in 0..2 * CAPTURE_TICKS {
        demo.run(1);
        if demo.unit(outpost).team == Team::Radiant {
            break;
        }
        if demo.hero().is_some_and(|h| h.statuses.bits & StatusFlags::CHANNELLING != 0) {
            channel_ticks += 1;
        }
    }
    assert_eq!(
        demo.unit(outpost).team,
        Team::Radiant,
        "player hero completed the capture"
    );
    assert!(
        channel_ticks >= CAPTURE_TICKS - 1,
        "player hero inherited removed hero's capture progress: completed in {channel_ticks} ticks (expected ~{CAPTURE_TICKS})"
    );
}

#[test]
fn unlearning_abilities_turns_off_active_rot_and_stops_its_burn() {
    use bota_proto::{AbilityId, EffectId, Inflictor};

    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let rot = ability_slot_of(demo.unit(pudge), AbilityId::ROT);
    demo.order(pudge, Order::Learn { slot: rot });
    demo.order(
        pudge,
        Order::Cast {
            slot: rot,
            target: Target::None,
        },
    );
    demo.run(2);
    assert!(
        demo.unit(pudge)
            .effects
            .iter()
            .any(|e| e.id == EffectId::ROT),
        "Rot is active before unlearning"
    );

    demo.cheat_at(pudge, Cheat::UnlearnAbilities);
    assert_eq!(
        demo.unit(pudge).abilities[usize::from(rot.0)].level,
        0,
        "Rot level was zeroed"
    );
    assert!(
        !demo.unit(pudge).abilities[usize::from(rot.0)].on,
        "Rot toggle must be off after unlearning"
    );
    assert!(
        demo.unit(pudge)
            .effects
            .iter()
            .all(|e| e.id != EffectId::ROT),
        "Rot effect must not remain after unlearning"
    );
    assert!(
        demo.view()
            .projectiles
            .iter()
            .all(|p| p.inflictor != Some(Inflictor::Ability(AbilityId::ROT))),
        "Rot mark must not remain after unlearning"
    );
    let hp_after = demo.unit(pudge).hp;
    for _ in 0..rules::TICKS_PER_SECOND {
        demo.run(1);
        let hp = demo.unit(pudge).hp;
        assert!(
            hp >= hp_after,
            "Pudge kept taking Rot burn after unlearning: hp {hp} < {hp_after}"
        );
    }
}

#[test]
fn unlearning_abilities_cancels_an_active_dismember_channel() {
    use bota_proto::{AbilityId, EffectId, EventKind, Inflictor, StatusFlags};

    let mut demo = Demo::on(HERO_DEMO_ID);
    let own = demo.me().unit.expect("own hero");
    demo.cheat(Cheat::Levels { count: 25 });
    demo.cheat(Cheat::Refresh);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    demo.cheat_at(pudge, Cheat::Levels { count: 10 });
    let dismember = ability_slot_of(demo.unit(pudge), AbilityId::DISMEMBER);
    demo.order(pudge, Order::Learn { slot: dismember });
    let own_pos = demo.unit(own).pos;
    demo.cheat_at(
        pudge,
        Cheat::Teleport {
            at: own_pos + Vec2::from_ints(120, 0),
        },
    );
    demo.order(
        pudge,
        Order::Cast {
            slot: dismember,
            target: Target::Unit(own),
        },
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        demo.run(1);
        if demo.unit(own).statuses.bits & StatusFlags::STUNNED != 0 {
            break;
        }
    }
    assert_ne!(
        demo.unit(own).statuses.bits & StatusFlags::STUNNED,
        0,
        "Dismember stunned the victim"
    );
    assert!(
        demo.unit(own)
            .effects
            .iter()
            .any(|e| e.id == EffectId::DISMEMBERED),
        "victim wears Dismembered effect"
    );
    assert_ne!(
        demo.unit(pudge).statuses.bits & StatusFlags::CHANNELLING,
        0,
        "Pudge is channelling Dismember"
    );

    demo.cheat_at(pudge, Cheat::UnlearnAbilities);
    assert_eq!(
        demo.unit(pudge).statuses.bits & StatusFlags::CHANNELLING,
        0,
        "Pudge must stop channelling Dismember after unlearning"
    );
    assert_eq!(
        demo.unit(own).statuses.bits & StatusFlags::STUNNED,
        0,
        "victim must not remain stunned after Dismember is unlearned"
    );
    assert!(
        demo.unit(own)
            .effects
            .iter()
            .all(|e| e.id != EffectId::DISMEMBERED),
        "victim must not keep Dismembered effect after unlearning"
    );
    assert!(
        demo.view()
            .projectiles
            .iter()
            .all(|p| p.inflictor != Some(Inflictor::Ability(AbilityId::DISMEMBER))),
        "Dismember mark must be cleared when unlearned"
    );
    // Run through the rest of the channel duration: no magical Dismember bites may land.
    for _ in 0..4 * rules::TICKS_PER_SECOND {
        let streams = demo.local.step(&[&[]]).expect("a step");
        for msg in streams.into_iter().flatten() {
            if let ServerMsg::Events { events, .. } = msg {
                for event in events {
                    if let EventKind::Damaged {
                        target,
                        kind: DamageKind::Magical,
                        ..
                    } = event
                    {
                        assert_ne!(
                            target, own,
                            "victim took magical Dismember bite after unlearning"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn deferred_cast_point_wait_overflow_on_a_controlled_hero_refuses_to_the_giver() {
    use bota_proto::AbilityId;

    let mut demo = Demo::on(HERO_DEMO_ID);
    let sf = demo.spawn_enemy(HeroId::SHADOW_FIEND);
    demo.cheat_at(sf, Cheat::Levels { count: 10 });
    let requiem = ability_slot_of(demo.unit(sf), AbilityId::REQUIEM);
    let raze = ability_slot_of(demo.unit(sf), AbilityId::RAZE_NEAR);
    demo.order(sf, Order::Learn { slot: requiem });
    demo.order(sf, Order::Learn { slot: raze });
    // Requiem of Souls has a 1.67 s (50-tick) cast point, long enough to overflow the
    // cast-point wait queue (32 entries) at one order per tick through the delay line.
    demo.order(
        sf,
        Order::Cast {
            slot: requiem,
            target: Target::None,
        },
    );
    assert!(
        demo.unit(sf).cast.is_some(),
        "Shadow Fiend stands in Requiem's cast point"
    );

    let cast_raze = Order::Cast {
        slot: raze,
        target: Target::None,
    };
    for _ in 0..rules::MAX_QUEUED_ORDERS {
        demo.order(sf, cast_raze);
    }
    assert!(
        demo.unit(sf).cast.is_some(),
        "still in Requiem's cast point when the 33rd cast arrives"
    );
    demo.step_naming(&[(Some(sf), cast_raze)]);
    assert_eq!(
        demo.rejected,
        [RejectReason::QueueFull],
        "the giver must receive QueueFull when overflowing a controlled hero's cast-point wait"
    );
}

#[test]
fn deferred_cast_point_wait_refusal_on_release_goes_to_the_giver() {
    use bota_proto::AbilityId;

    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    let hook = ability_slot_of(
        pair.view(1).unit(pudge).expect("pudge stands"),
        AbilityId::MEAT_HOOK,
    );
    pair.give(1, Some(pudge), Order::Learn { slot: hook });
    assert_eq!(pair.rejected[1], []);
    let aim = pair.view(1).unit(pudge).expect("pudge").pos + Vec2::from_ints(400, 0);
    let cast_hook = Order::Cast {
        slot: hook,
        target: Target::Pos(aim),
    };
    // Seat 1 casts Meat Hook on the created hero, then while in its cast point issues a second
    // cast that waits for release; once the first cast fires, Meat Hook is on cooldown and the
    // waiting cast must be refused to Seat 1 (not Seat 0 and not dropped on the created seat).
    pair.give(1, Some(pudge), cast_hook);
    assert_eq!(pair.rejected[1], []);
    assert!(
        pair.view(1).unit(pudge).expect("pudge").cast.is_some(),
        "Pudge stands in Meat Hook's cast point"
    );
    pair.give(1, Some(pudge), cast_hook);
    assert_eq!(pair.rejected[1], [], "second cast waited behind cast point");

    let mut seat1_refusals = Vec::new();
    let mut seat0_refusals = Vec::new();
    for _ in 0..rules::TICKS_PER_SECOND {
        pair.run(1);
        seat0_refusals.extend_from_slice(&pair.rejected[0]);
        seat1_refusals.extend_from_slice(&pair.rejected[1]);
        if !seat1_refusals.is_empty() {
            break;
        }
    }
    assert_eq!(
        seat1_refusals,
        [RejectReason::OnCooldown],
        "Seat 1 must hear OnCooldown when its waiting cast is released"
    );
    assert_eq!(seat0_refusals, [], "Seat 0 gave no order");
}

#[test]
fn deferred_shift_queued_order_refusal_on_take_up_goes_to_the_giver() {
    use bota_proto::AbilityId;

    let mut pair = Pair::on(HERO_DEMO_ID, true);
    let pudge = pair.dire_creates_an_ally();
    let hook = ability_slot_of(
        pair.view(1).unit(pudge).expect("pudge stands"),
        AbilityId::MEAT_HOOK,
    );
    pair.give(1, Some(pudge), Order::Learn { slot: hook });
    assert_eq!(pair.rejected[1], []);
    let from = pair.view(1).unit(pudge).expect("pudge").pos;
    let nearby = from + Vec2::from_ints(120, 0);
    // Walk a short distance, then Shift-queue Meat Hook behind the walk from Seat 1.
    pair.give(
        1,
        Some(pudge),
        Order::Move {
            target: Target::Pos(nearby),
        },
    );
    assert_eq!(pair.rejected[1], []);

    pair.seq += 1;
    let queued = [OutgoingOrder {
        seq: pair.seq,
        unit: Some(pudge),
        order: Order::Cast {
            slot: hook,
            target: Target::Pos(nearby + Vec2::from_ints(400, 0)),
        },
        seen: pair.local.tick(),
        queue: true,
    }];
    let streams = pair.local.step(&[&[], &queued]).expect("a step");
    pair.take(streams);
    assert_eq!(pair.rejected[1], [], "queued cast was admitted");

    // Empty the created hero's mana while it walks so the queued cast fails on take-up.
    pair.give(
        1,
        Some(pudge),
        Order::Cheat {
            cheat: Cheat::SetPools {
                health: 100,
                mana: 0,
            },
        },
    );
    assert_eq!(pair.rejected[1], []);

    let mut seat1_refusals = Vec::new();
    let mut seat0_refusals = Vec::new();
    for _ in 0..3 * rules::TICKS_PER_SECOND {
        pair.run(1);
        seat0_refusals.extend_from_slice(&pair.rejected[0]);
        seat1_refusals.extend_from_slice(&pair.rejected[1]);
        if !seat1_refusals.is_empty() {
            break;
        }
    }
    assert_eq!(
        seat1_refusals,
        [RejectReason::NotEnoughMana],
        "Seat 1 must hear NotEnoughMana when its Shift-queued cast fails on take-up"
    );
    assert_eq!(seat0_refusals, [], "Seat 0 gave no order");
}

#[test]
fn removing_a_created_hero_removes_ground_items_it_owned() {
    use bota_proto::ItemSlot;

    let mut demo = Demo::on(HERO_DEMO_ID);
    let hero_a = demo.spawn_enemy(HeroId::PUDGE);
    let slot = demo.row_of(hero_a).slot;
    demo.cheat_at(
        hero_a,
        Cheat::Item {
            item: ItemId::BLINK_DAGGER,
        },
    );
    demo.order(
        hero_a,
        Order::Put {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        demo.run(1);
        if demo
            .view()
            .loot
            .iter()
            .any(|loot| loot.item == ItemId::BLINK_DAGGER)
        {
            break;
        }
    }
    assert!(
        demo.view()
            .loot
            .iter()
            .any(|loot| loot.item == ItemId::BLINK_DAGGER),
        "dropped Blink Dagger lies on the ground"
    );

    demo.cheat_at(hero_a, Cheat::RemoveHero);
    assert!(
        demo.view()
            .loot
            .iter()
            .all(|loot| loot.item != ItemId::BLINK_DAGGER),
        "ground items owned by the removed hero must be removed with it"
    );

    // Spawning another hero in the reused slot finds nothing left on the ground from the old hero.
    let hero_b = demo.spawn_enemy(HeroId::SHADOW_FIEND);
    assert_eq!(demo.row_of(hero_b).slot, slot);
    assert!(demo.view().loot.is_empty());
}

#[test]
fn items_of_a_removed_hero_held_by_another_unit_become_that_holders() {
    use bota_proto::ItemSlot;

    // Sandbox map (DEMO_ID): items have prices and selling refunds gold.
    let mut demo = Demo::on(DEMO_ID);
    demo.cheat(Cheat::AllVision { on: true });
    let own = demo.me().unit.expect("own hero");
    let home = bota_core::hero_spawn_pos(map_of(DEMO_ID), Team::Radiant);
    demo.cheat(Cheat::Teleport { at: home });

    // Create an allied Radiant hero beside the player's hero at the Radiant fountain.
    demo.cheat(Cheat::CreateHero {
        hero: HeroId::PUDGE,
        enemy: false,
        at: home + Vec2::from_ints(100, 0),
    });
    let hero_a = demo
        .view()
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("ally hero A in slot 1");
    demo.cheat_at(
        hero_a,
        Cheat::Item {
            item: ItemId::BLINK_DAGGER,
        },
    );
    // Hero A hands its Blink Dagger to the player's own hero.
    demo.order(
        hero_a,
        Order::Put {
            slot: ItemSlot(0),
            target: Target::Unit(own),
        },
    );
    for _ in 0..2 * rules::TICKS_PER_SECOND {
        demo.run(1);
        if demo
            .unit(own)
            .items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::BLINK_DAGGER)
        {
            break;
        }
    }
    let blink_slot = demo
        .unit(own)
        .items
        .iter()
        .position(|slot| slot.as_ref().is_some_and(|item| item.id == ItemId::BLINK_DAGGER))
        .expect("player hero holds hero A's Blink Dagger");
    assert_eq!(
        demo.unit(own).items[blink_slot]
            .as_ref()
            .expect("item")
            .owner,
        SlotId(1),
        "before removal, hero A owns the handed item"
    );
    // Before removal, the player cannot sell hero A's item.
    demo.step(&[Order::Sell {
        slot: ItemSlot(blink_slot as u8),
    }]);
    assert_eq!(demo.rejected, [RejectReason::NotYourItem]);

    // Remove hero A, then spawn hero B reusing SlotId(1).
    demo.cheat_at(hero_a, Cheat::RemoveHero);
    let owner_after = demo.unit(own).items[blink_slot]
        .as_ref()
        .expect("player still holds Blink Dagger")
        .owner;
    assert_eq!(
        owner_after,
        SlotId(0),
        "held item must transfer ownership to its holder (and never carry a sentinel)"
    );
    assert_ne!(owner_after, SlotId(u8::MAX));

    demo.cheat(Cheat::CreateHero {
        hero: HeroId::SHADOW_FIEND,
        enemy: false,
        at: home + Vec2::from_ints(100, 0),
    });
    let hero_b_row = demo.view().player(SlotId(1)).expect("hero B reused slot 1");
    let hero_b_gold_before = hero_b_row.gold.expect("ally gold visible");

    // The player (new owner) can now sell the Blink Dagger and receives the gold; hero B gets none.
    let own_gold_before = demo.me().gold.expect("own gold");
    demo.step(&[Order::Sell {
        slot: ItemSlot(blink_slot as u8),
    }]);
    assert_eq!(demo.rejected, [], "holder can sell the transferred item");
    assert!(
        demo.me().gold.expect("own gold") > own_gold_before,
        "holder received the sale gold"
    );
    assert_eq!(
        demo.view().player(SlotId(1)).expect("hero B").gold,
        Some(hero_b_gold_before),
        "next hero in slot 1 received nothing"
    );
}

#[test]
fn aimed_body_cheats_on_a_dead_created_hero_refuse_with_hero_dead_like_own_dead_hero() {
    use bota_proto::ModifierSpec;

    let mut demo = Demo::on(HERO_DEMO_ID);
    let pudge = demo.spawn_enemy(HeroId::PUDGE);
    let spot = rules::DEMO_LANE_CORNERS[1];
    let body_cheats = [
        Cheat::Teleport { at: spot },
        Cheat::ApplyModifier {
            target: Target::None,
            spec: ModifierSpec::NOMINAL,
            ticks: 30,
        },
        Cheat::ClearModifiers {
            target: Target::None,
        },
        Cheat::SpellImmunity {
            target: Target::None,
            ticks: 30,
        },
        Cheat::SetPools {
            health: 50,
            mana: 50,
        },
        Cheat::MaxAbilities,
        Cheat::UnlearnAbilities,
    ];

    // Kill own hero first and record that each body cheat returns HeroDead.
    demo.kill_hero();
    for cheat in body_cheats {
        demo.step(&[Order::Cheat { cheat }]);
        assert_eq!(
            demo.rejected,
            [RejectReason::HeroDead],
            "own dead hero must refuse {cheat:?} with HeroDead"
        );
    }
    demo.cheat(Cheat::Respawn);

    // Kill the created hero: every body-requiring cheat must give the same HeroDead refusal.
    demo.cheat_at(pudge, Cheat::Suicide);
    assert!(demo.row_of(pudge).respawn_left > 0, "created hero lies dead");
    for cheat in body_cheats {
        demo.step_naming(&[(Some(pudge), Order::Cheat { cheat })]);
        assert_eq!(
            demo.rejected,
            [RejectReason::HeroDead],
            "dead created hero must refuse {cheat:?} with HeroDead"
        );
    }
    // Removing a dead created hero still works.
    demo.cheat_at(pudge, Cheat::RemoveHero);
    assert!(demo.view().player(SlotId(1)).is_none());
}

#[test]
fn hero_scoped_cheats_aimed_at_an_illusion_are_refused_as_not_a_hero_and_never_hit_giver() {
    use bota_proto::{RuneKind, RuneSet};

    let map = MapId(2);
    let cfg = MatchConfig {
        map,
        cheats: true,
        objectives: bota_proto::ObjectiveSet::NONE,
        runes: RuneSet::of(RuneKind::Illusion),
        ..config()
    };
    let (local, opening) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut demo = Demo {
        local,
        seq: 0,
        view: None,
        rejected: Vec::new(),
        over: false,
    };
    demo.take(opening);
    demo.cheat(Cheat::AllVision { on: true });

    let rune_spot = map_of(map).rune_spots[0].at;
    demo.cheat(Cheat::CreateHero {
        hero: HeroId::PUDGE,
        enemy: false,
        at: rune_spot + Vec2::from_ints(120, 0),
    });
    let pudge = demo
        .view()
        .player(SlotId(1))
        .and_then(|row| row.unit)
        .expect("created Pudge");
    demo.cheat(Cheat::SpawnRune {
        kind: RuneKind::Illusion,
        at: rune_spot,
    });
    let rune = demo
        .view()
        .runes
        .first()
        .expect("Illusion rune spawned")
        .id;
    demo.order(
        pudge,
        Order::Move {
            target: Target::Unit(rune),
        },
    );
    let mut illusion = None;
    for _ in 0..3 * rules::TICKS_PER_SECOND {
        demo.run(1);
        illusion = demo
            .view()
            .units
            .iter()
            .find(|u| {
                u.kind == UnitKind::Hero
                    && u.owner == Some(SlotId(1))
                    && u.id != pudge
                    && u.effects
                        .iter()
                        .any(|e| e.id == bota_proto::EffectId::ILLUSION)
            })
            .map(|u| u.id);
        if illusion.is_some() {
            break;
        }
    }
    let illusion = illusion.expect("created hero's rune illusion");

    let own_gold_before = demo.me().gold;
    let own_level_before = demo.me().level;
    for cheat in [
        Cheat::Suicide,
        Cheat::Gold { amount: 1000 },
        Cheat::Levels { count: 2 },
        Cheat::Item {
            item: ItemId::BLINK_DAGGER,
        },
        Cheat::Invulnerable { on: true },
        Cheat::ResetHero,
        Cheat::Respawn,
        Cheat::ResetBuyback,
        Cheat::RemoveHero,
        Cheat::MaxAbilities,
        Cheat::UnlearnAbilities,
    ] {
        demo.step_naming(&[(Some(illusion), Order::Cheat { cheat })]);
        assert_eq!(
            demo.rejected,
            [RejectReason::NotAHero],
            "{cheat:?} aimed at a created hero's illusion must be refused as NotAHero"
        );
    }
    assert_eq!(
        demo.me().respawn_left,
        0,
        "Suicide aimed at an illusion must never kill the giver's hero"
    );
    assert_eq!(demo.me().gold, own_gold_before, "giver's gold untouched");
    assert_eq!(demo.me().level, own_level_before, "giver's level untouched");
}
