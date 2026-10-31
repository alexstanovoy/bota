//! Dota's cheat commands as the simulation carries them out: each refused in
//! a match without cheats, and in one with them doing what Dota's does
//! through the rules' own machinery.

use bota_core::rules;
use bota_proto::{
    Cheat, CreepSide, Fixed, HeroId, ItemId, MapId, Order, RejectReason, SlotId, Team, UnitKind,
    Vec2,
};

use crate::game::tests::support::{self, an_empty_spot, run, seat_order};
use crate::game::{Entity, Event, MatchConfig, World};

/// A one-seat match on a map, with or without cheats.
fn world_on(map: u16, cheats: bool) -> World {
    let cfg = MatchConfig {
        starting_gold_lost: Vec::new(),
        economy_events: Vec::new(),
        cheats,
        map: MapId(map),
        ..support::config()
    };
    World::for_match(&cfg, cfg.rng())
}

fn cheat(world: &mut World, cheat: Cheat) -> Vec<Event> {
    seat_order(world, Order::Cheat { cheat })
}

fn hero(world: &World) -> Entity {
    world.seats[0].unit.expect("standing")
}

/// The units of the kinds `of` takes, standing now.
fn count(world: &World, of: impl Fn(UnitKind, Option<Team>) -> bool) -> usize {
    world
        .entities
        .iter()
        .filter(|&entity| {
            world
                .kind
                .get(entity)
                .is_some_and(|kind| of(*kind, world.team.get(entity).copied()))
        })
        .count()
}

fn lane_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepRanged
            | UnitKind::CreepSiege
    )
}

/// Every command but the old ones (`cheats.rs` has those), aimed sensibly.
fn every_command(at: Vec2) -> Vec<Cheat> {
    vec![
        Cheat::LevelBots { count: 1 },
        Cheat::Wtf { on: true },
        Cheat::Respawn,
        Cheat::CreateHero {
            hero: HeroId::PUDGE,
            enemy: true,
            at,
        },
        Cheat::SpawnCreeps,
        Cheat::CreepSpawning { on: false },
        Cheat::SpawnNeutrals,
        Cheat::KillCreeps {
            which: CreepSide::All,
        },
        Cheat::StartGame,
        Cheat::AllVision { on: true },
        Cheat::Invulnerable { on: true },
        Cheat::Teleport { at },
        Cheat::ResetHero,
        Cheat::TimeOfDay { night: false },
        Cheat::CreateCreeps { enemy: true, at },
    ]
}

/// Brings the seat's hero down, and runs the tick that sees it fall.
fn fell(world: &mut World) {
    let hero = hero(world);
    world.bury(vec![(hero, None)], &mut Vec::new());
    run(world, 1);
    assert!(world.seats[0].dead(), "the hero fell");
}

#[test]
fn every_command_is_refused_without_cheats_and_taken_with_them() {
    let plain = world_on(1, false);
    let cheating = world_on(1, true);
    let at = an_empty_spot(&cheating);
    for cheat in every_command(at) {
        let order = Order::Cheat { cheat };
        assert_eq!(
            plain.validate_order(SlotId(0), None, &order),
            Err(RejectReason::NoCheats),
            "{cheat:?}"
        );
        assert_eq!(
            cheating.validate_order(SlotId(0), None, &order),
            Ok(()),
            "{cheat:?}"
        );
    }
}

#[test]
fn a_dead_hero_cheats_too_except_where_it_takes_a_body() {
    let mut world = world_on(1, true);
    let at = an_empty_spot(&world);
    fell(&mut world);
    for cheat in every_command(at) {
        let wanted = match cheat {
            Cheat::Teleport { .. } => Err(RejectReason::HeroDead),
            _ => Ok(()),
        };
        let order = Order::Cheat { cheat };
        assert_eq!(
            world.validate_order(SlotId(0), None, &order),
            wanted,
            "{cheat:?}"
        );
    }
    let mut plain = world_on(1, false);
    fell(&mut plain);
    let order = Order::Cheat {
        cheat: Cheat::Respawn,
    };
    assert_eq!(
        plain.validate_order(SlotId(0), None, &order),
        Err(RejectReason::NoCheats)
    );
}

#[test]
fn respawn_brings_a_fallen_hero_back_and_moves_a_standing_one_home() {
    let mut world = world_on(1, true);
    fell(&mut world);
    cheat(&mut world, Cheat::Respawn);
    let back = hero(&world);
    let home = bota_core::hero_spawn_pos(world.map, Team::Radiant);
    assert!(
        world
            .pos(back)
            .is_some_and(|at| at.within(home, rules::units(400)))
    );
    let away = an_empty_spot(&world);
    cheat(&mut world, Cheat::Teleport { at: away });
    assert!(
        world
            .pos(back)
            .is_some_and(|at| at.within(away, rules::units(400)))
    );
    world.health.get_mut(back).expect("standing").hp = Fixed::from_int(10);
    cheat(&mut world, Cheat::Respawn);
    assert_eq!(
        hero(&world),
        back,
        "a standing hero is moved, not made anew"
    );
    assert!(
        world
            .pos(back)
            .is_some_and(|at| at.within(home, rules::units(400)))
    );
    let full = world.stats.get(back).expect("stats").max_hp;
    assert_eq!(world.health.get(back).map(|h| h.hp), Some(full));
}

#[test]
fn a_created_hero_stands_where_told_on_a_seat_of_its_own_and_levels_with_the_bots() {
    let mut world = world_on(1, true);
    let at = an_empty_spot(&world);
    cheat(
        &mut world,
        Cheat::CreateHero {
            hero: HeroId::PUDGE,
            enemy: true,
            at,
        },
    );
    assert_eq!(world.seats.len(), 2);
    let created = &world.seats[1];
    assert!(created.created);
    assert_eq!(
        (created.slot, created.team, created.hero),
        (SlotId(1), Team::Dire, HeroId::PUDGE)
    );
    let body = created.unit.expect("standing");
    assert!(
        world
            .pos(body)
            .is_some_and(|spot| spot.within(at, rules::units(400)))
    );
    assert!(
        world.view(Team::Radiant).player(SlotId(1)).is_some(),
        "told among the players"
    );
    cheat(&mut world, Cheat::LevelBots { count: 4 });
    assert_eq!(world.seats[1].level, 5, "the created hero went up");
    assert_eq!(world.seats[0].level, 1, "the player's did not");
    // A created hero drives nothing a player could order.
    let order = Order::Cheat {
        cheat: Cheat::Gold { amount: 1 },
    };
    assert_eq!(
        world.validate_order(SlotId(1), None, &order),
        Err(RejectReason::NotYourUnit)
    );
}

#[test]
fn free_spells_cost_nothing_and_leave_no_cooldown_until_they_are_turned_off() {
    let mut world = world_on(1, true);
    let hero = hero(&world);
    let raze = world.abilities.get(hero).expect("a kit").slots[0].id;
    let (cost, wait) = (
        world.ability_mana_cost(hero, raze, 1),
        world.ability_cooldown(hero, raze, 1),
    );
    assert!(cost > 0 && wait > 0);
    world.abilities.get_mut(hero).expect("a kit").slots[0].cooldown = 99;
    cheat(&mut world, Cheat::Wtf { on: true });
    assert_eq!(
        world.abilities.get(hero).expect("a kit").slots[0].cooldown,
        0,
        "and a refresh"
    );
    assert_eq!(world.ability_mana_cost(hero, raze, 1), 0);
    assert_eq!(world.ability_cooldown(hero, raze, 1), 0);
    cheat(&mut world, Cheat::Wtf { on: false });
    assert_eq!(world.ability_mana_cost(hero, raze, 1), cost);
    assert_eq!(world.ability_cooldown(hero, raze, 1), wait);
}

#[test]
fn creeps_spawn_now_hold_off_and_come_back_and_are_taken_away_by_side() {
    let mut world = world_on(1, true);
    cheat(&mut world, Cheat::SpawnCreeps);
    let radiant = count(&world, |kind, team| {
        lane_creep(kind) && team == Some(Team::Radiant)
    });
    let dire = count(&world, |kind, team| {
        lane_creep(kind) && team == Some(Team::Dire)
    });
    assert!(radiant > 0 && dire > 0, "a wave for both sides");
    cheat(
        &mut world,
        Cheat::KillCreeps {
            which: CreepSide::Radiant,
        },
    );
    assert_eq!(
        count(&world, |kind, team| lane_creep(kind)
            && team == Some(Team::Radiant)),
        0
    );
    assert_eq!(
        count(&world, |kind, team| lane_creep(kind)
            && team == Some(Team::Dire)),
        dire
    );
    cheat(
        &mut world,
        Cheat::KillCreeps {
            which: CreepSide::All,
        },
    );
    cheat(&mut world, Cheat::CreepSpawning { on: false });
    let to_first_wave = world.schedule().first_wave() + 5 - world.tick;
    run(&mut world, to_first_wave);
    assert_eq!(
        count(&world, |kind, _| lane_creep(kind)),
        0,
        "no wave on the clock"
    );
    cheat(&mut world, Cheat::CreepSpawning { on: true });
    run(&mut world, rules::WAVE_PERIOD_TICKS);
    assert!(
        count(&world, |kind, _| lane_creep(kind)) > 0,
        "the next wave comes"
    );
}

#[test]
fn neutrals_spawn_now_and_are_taken_away() {
    let mut world = world_on(0, true);
    assert_eq!(count(&world, |kind, _| kind == UnitKind::CreepNeutral), 0);
    cheat(&mut world, Cheat::SpawnNeutrals);
    let neutrals = count(&world, |kind, _| kind == UnitKind::CreepNeutral);
    assert!(neutrals > 0, "the camps filled before their time");
    cheat(
        &mut world,
        Cheat::KillCreeps {
            which: CreepSide::Neutral,
        },
    );
    assert_eq!(count(&world, |kind, _| kind == UnitKind::CreepNeutral), 0);
}

#[test]
fn start_game_moves_the_game_clock_to_the_horn_and_the_first_wave_comes() {
    let mut world = world_on(1, true);
    run(&mut world, 10);
    assert!(
        world.view(Team::Radiant).clock.is_none(),
        "nothing moved yet"
    );
    cheat(&mut world, Cheat::StartGame);
    assert_eq!(world.clock(), world.schedule().horn, "the horn, this tick");
    assert!(
        count(&world, |kind, _| lane_creep(kind)) > 0,
        "and the first wave with it"
    );
    let view = world.view(Team::Radiant);
    assert_eq!(view.game_tick(), world.schedule().horn);
    assert!(!world.time_of_day().night, "day breaks at the horn");
    let shift = world.clock_shift.game;
    cheat(&mut world, Cheat::StartGame);
    assert_eq!(world.clock_shift.game, shift, "once begun, nothing more");
}

#[test]
fn the_night_toggle_turns_the_time_of_day_and_the_cycle_goes_on() {
    let mut world = world_on(1, true);
    cheat(&mut world, Cheat::StartGame);
    assert!(!world.time_of_day().night);
    cheat(&mut world, Cheat::TimeOfDay { night: true });
    let night = world.time_of_day();
    assert!(night.night, "night falls now");
    assert!(world.view(Team::Radiant).clock.is_some_and(|c| c.day > 0));
    cheat(&mut world, Cheat::TimeOfDay { night: true });
    assert!(world.time_of_day().night, "asking again changes nothing");
    run(&mut world, night.ticks_left);
    assert!(
        !world.time_of_day().night,
        "and day breaks when the night is out"
    );
}

#[test]
fn all_vision_shows_each_side_what_the_other_sees() {
    let mut world = world_on(1, true);
    let at = an_empty_spot(&world);
    cheat(
        &mut world,
        Cheat::CreateHero {
            hero: HeroId::PUDGE,
            enemy: true,
            at,
        },
    );
    let far = world.seats[1].unit.expect("standing");
    run(&mut world, 1);
    assert!(!world.can_see(Team::Radiant, far), "out of Radiant's sight");
    cheat(&mut world, Cheat::AllVision { on: true });
    run(&mut world, 1);
    assert!(
        world.can_see(Team::Radiant, far),
        "seen through Dire's eyes"
    );
    assert!(world.can_see_point(Team::Radiant, at));
    cheat(&mut world, Cheat::AllVision { on: false });
    run(&mut world, 1);
    assert!(!world.can_see(Team::Radiant, far));
}

#[test]
fn an_invulnerable_hero_stays_so_through_death_and_back() {
    let mut world = world_on(1, true);
    cheat(&mut world, Cheat::Invulnerable { on: true });
    run(&mut world, 1);
    let hero = hero(&world);
    assert!(world.stats.get(hero).is_some_and(|s| s.invulnerable));
    fell(&mut world);
    cheat(&mut world, Cheat::Respawn);
    run(&mut world, 1);
    let back = self::hero(&world);
    assert!(world.stats.get(back).is_some_and(|s| s.invulnerable));
    cheat(&mut world, Cheat::Invulnerable { on: false });
    run(&mut world, 1);
    assert!(
        world.stats.get(back).is_some_and(|s| s.invulnerable),
        "risen at its fountain it stays untouchable until given a command"
    );
    seat_order(
        &mut world,
        bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    );
    run(&mut world, 1);
    assert!(!world.stats.get(back).is_some_and(|s| s.invulnerable));
}

#[test]
fn reset_hero_starts_the_hero_over_at_its_fountain() {
    let mut world = world_on(1, true);
    cheat(&mut world, Cheat::Levels { count: 5 });
    cheat(&mut world, Cheat::Gold { amount: 3000 });
    cheat(
        &mut world,
        Cheat::Item {
            item: ItemId::BUTTERFLY,
        },
    );
    cheat(&mut world, Cheat::ResetHero);
    let seat = &world.seats[0];
    assert_eq!(
        (seat.level, seat.xp, seat.gold),
        (1, 0, rules::STARTING_GOLD)
    );
    let hero = hero(&world);
    assert!(
        !world
            .inventory
            .get(hero)
            .expect("a bag")
            .held()
            .any(|stack| stack.id == ItemId::BUTTERFLY)
    );
    let home = bota_core::hero_spawn_pos(world.map, Team::Radiant);
    assert!(
        world
            .pos(hero)
            .is_some_and(|at| at.within(home, rules::units(400)))
    );
}

#[test]
fn creeps_created_at_a_spot_stand_there_on_their_side() {
    let mut world = world_on(1, true);
    let at = an_empty_spot(&world);
    cheat(&mut world, Cheat::CreateCreeps { enemy: true, at });
    let near = world
        .entities
        .iter()
        .filter(|&entity| {
            world.kind.get(entity).is_some_and(|kind| lane_creep(*kind))
                && world.team.get(entity) == Some(&Team::Dire)
                && world
                    .pos(entity)
                    .is_some_and(|spot| spot.within(at, rules::units(500)))
        })
        .count();
    assert!(near >= 3, "a wave of Dire creeps where asked, {near}");
}

#[test]
fn the_match_tells_whether_cheats_are_on() {
    for cheats in [false, true] {
        let cfg = MatchConfig {
            cheats,
            ..support::config()
        };
        assert_eq!(cfg.info().cheats, cheats);
    }
}
