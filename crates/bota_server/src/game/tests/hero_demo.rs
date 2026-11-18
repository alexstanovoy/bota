//! The hero demo game (Dota's Hero Demo on its own small map) as its one player meets it: an
//! in-process match of one seat with cheats on, driven by orders and cheats through the order
//! path, every check read from what the seat was sent.

use bota_core::{DEMO_ID, HERO_DEMO_ID, attackable_from, map_of, rules};
use bota_proto::{
    Cheat, DamageKind, EntityId, HeroId, MapId, Order, OutgoingOrder, PlayerView, RejectReason,
    ServerMsg, SlotId, Team, UnitKind, UnitView, Vec2, WorldView,
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
        let seen = self.local.tick();
        let out: Vec<OutgoingOrder> = orders
            .iter()
            .map(|&order| {
                self.seq += 1;
                OutgoingOrder {
                    seq: self.seq,
                    unit: None,
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

    fn unit(&self, id: EntityId) -> &UnitView {
        self.view().unit(id).expect("the unit is seen")
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
