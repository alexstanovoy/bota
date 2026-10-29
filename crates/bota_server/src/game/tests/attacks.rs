//! The attack cycle and uphill misses.

use bota_core::{MELEE_CREEP, NeutralKind, attack_gain, facing_towards, rules};

use crate::game::{
    ActionPhase, ActionState, Entity, Ground, Health, MatchRng, UnitOrder, World, wire_id,
};
use bota_proto::{Angle, EventKind, Fixed, HeroId, Order, SlotId, Target, Team, Vec2};

use super::support::*;

#[test]
fn a_swing_waits_on_the_angle_it_is_looking_at() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    // Turned right away from it: in reach, seen, but not looked at.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = Angle { brads: 32768 };
    }
    world.run_actions();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "nothing begins while it is looking the other way"
    );
    // Looking straight at it.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = Angle { brads: 0 };
    }
    world.run_actions();
    assert!(
        swinging(&world, attacker).is_some(),
        "and begins once it is"
    );
}

#[test]
fn a_swing_waits_on_being_able_to_see_at_all() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    world.run_actions();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "what a side has no eyes on it does not swing at"
    );
}

#[test]
fn a_swing_waits_on_reach() {
    let (mut world, attacker, mark) = duel(600);
    world.set_target(attacker, mark);
    world.run_actions();
    assert_eq!(swinging(&world, attacker), None, "too far to touch");
}

#[test]
fn a_swing_lands_on_whoever_it_began_against() {
    let (mut world, attacker, mark) = duel(100);
    let other = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some(), "the swing began");
    // Set on somebody else halfway through.
    world.set_target(attacker, other);
    let was = world.health.get(mark).expect("standing").hp;
    run(&mut world, ticks_of(rules::MELEE_CREEP_ATTACK_POINT) + 1);
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "and landed on the one it began against"
    );
}

#[test]
fn a_swing_that_began_still_connects_a_little_past_reach() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        swinging(&world, attacker).is_some(),
        "the swing began in reach"
    );
    // Backs off by less than the leeway while the swing is under way.
    let just_out = rules::MELEE_CREEP_ATTACK_RANGE + 60;
    let was = world.health.get(mark).expect("standing").hp;
    for _ in 0..ticks_of(rules::MELEE_CREEP_ATTACK_POINT) + 1 {
        if let Some(at) = world.transform.get_mut(mark) {
            at.pos = Vec2::from_ints(5000 + just_out, 5000);
        }
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "a step past reach does not shake it off"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_gets_away() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        swinging(&world, attacker).is_some(),
        "the swing began in reach"
    );
    let far = rules::MELEE_CREEP_ATTACK_RANGE + rules::ATTACK_RANGE_LEEWAY + 200;
    let was = world.health.get(mark).expect("standing").hp;
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = Vec2::from_ints(5000 + far, 5000);
    }
    world.step();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "the swing is given up the moment it gets away"
    );
    assert_eq!(
        world.action.get(attacker).map(|a| a.attack_cooldown),
        Some(0),
        "and costs nothing, so the next one may start at once"
    );
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        was,
        "nothing was struck"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_falls() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some());
    world.health.insert(mark, Health { hp: Fixed::ZERO });
    world.step();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "there is nothing left to strike"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_is_lost_from_sight() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some());
    // Blinded to it, the way stepping into fog would.
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    world.run_actions();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "it does not finish a swing at what it can no longer see"
    );
}

#[test]
fn a_swing_costs_the_swinger_the_ground_it_stands_on() {
    let (mut world, hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Nothing else for it to fight, so it takes the hero and keeps after it.
    world.despawn(ours);
    world.transform.get_mut(hero).expect("hero").pos = Vec2::from_ints(5280, 5000);
    attack_click(&mut world, theirs);
    world.advance(&[]);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the creep takes the one in front of it"
    );
    let before = gap_along_lane(&world, hero, theirs);
    for _ in 0..60 {
        seat_order(
            &mut world,
            Order::Move {
                target: Target::Pos(Vec2::from_ints(3000, 5000)),
            },
        );
    }
    let after = gap_along_lane(&world, hero, theirs);
    // The creep is the faster of the two: only the ticks it spends rooted in
    // its swings let the hero pull away at all.
    assert!(
        after > before + 200,
        "the hero should be pulling away: {before} then {after}"
    );
}

#[test]
fn an_order_to_break_off_gives_up_a_swing_that_has_not_landed() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..60 {
        if swinging(&world, hero).is_some() {
            break;
        }
        world.advance(&[]);
    }
    assert!(
        swinging(&world, hero).is_some(),
        "the hero should be mid-swing by now"
    );
    stop_click(&mut world);
    let action = world.action.get(hero).copied().expect("acting");
    assert_eq!(action.state, ActionState::Ready, "the swing is given up");
    assert_eq!(action.attack_cooldown, 0, "and nothing was spent on it");
}

#[test]
fn an_order_after_a_swing_lands_does_not_hurry_the_next_one() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..120 {
        if recovering(&world, hero) {
            break;
        }
        world.advance(&[]);
    }
    let before = world
        .action
        .get(hero)
        .copied()
        .expect("acting")
        .attack_cooldown;
    assert!(
        before > 0,
        "the swing that landed spent the wait for the next"
    );
    stop_click(&mut world);
    assert!(!recovering(&world, hero), "the recovery is cancelled");
    let gain = attack_gain(world.stats.get(hero).expect("settled").attack_speed);
    assert_eq!(
        world.action.get(hero).expect("acting").attack_cooldown,
        before - gain,
        "but the wait for the next swing runs on"
    );
}

#[test]
fn ranged_attacks_miss_uphill_and_say_so_but_never_on_level_ground() {
    let ground = Ground::of(&crate::game::DOTA_TERRAIN);
    let (source, level, uphill) = nearby_elevations(&ground);
    let level_damage =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    let uphill_damage =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    assert!(level_damage > 0, "the level-ground control must attack");
    assert!(uphill_damage < level_damage, "uphill attacks must miss");
    assert!(
        uphill_damage > level_damage / 2,
        "the uphill miss rate must stay near one quarter"
    );
    let (level_landed, level_missed) =
        ranged_swings_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    let (uphill_landed, uphill_missed) =
        ranged_swings_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    assert!(level_landed > 0, "the level-ground control lands");
    assert_eq!(level_missed, 0, "and nothing misses on level ground");
    assert!(uphill_missed > 0, "uphill, misses are told of");
    assert!(
        uphill_missed < uphill_landed,
        "and they are the smaller part: {uphill_missed} of {}",
        uphill_landed + uphill_missed
    );
}

#[test]
fn buildings_never_evade_uphill_but_flying_attackers_miss_too() {
    let ground = Ground::of(&crate::game::DOTA_TERRAIN);
    let (source, level, uphill) = nearby_elevations(&ground);
    let building_level =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let building_uphill =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let flying_level = ranged_damage_after_ticks(
        source,
        level,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );
    let flying_uphill = ranged_damage_after_ticks(
        source,
        uphill,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );
    assert_eq!(building_uphill, building_level, "buildings do not evade");
    // 7.41: "Units with free movement now can miss their attacks when
    // attacking uphill targets."
    assert!(
        flying_uphill < flying_level,
        "flying attacks miss uphill too"
    );
}

#[test]
fn uphill_eligibility_uses_attacker_and_target_elevation_at_impact() {
    let ground = Ground::of(&crate::game::DOTA_TERRAIN);
    let (low, _, high) = nearby_elevations(&ground);
    let low_tier = ground.tier(low);
    let high_tier = ground.tier(high);
    let moved_up = manual_projectile_damage(high, high, low_tier, false);
    let moved_down = manual_projectile_damage(low, high, high_tier, false);
    let pierced_down = manual_projectile_damage(low, high, high_tier, true);
    assert_eq!(
        moved_up, 512,
        "an attacker now level with its target does not miss"
    );
    assert!(
        moved_down < moved_up,
        "an attacker now below its target can miss"
    );
    assert!(
        moved_down > moved_up / 2,
        "the miss rate stays near one quarter"
    );
    assert_eq!(pierced_down, 512, "a shot that pierces never misses uphill");
}

/// A race set up the same for both sides, two heroes striking a neutral
/// creep that stands as far from either: the creep turns on one of them, and
/// one blow is felt first. Neither is settled by which side was spawned into
/// the lower slots, so over matches each side takes its share of the kills.
#[test]
fn neither_side_takes_every_kill_of_an_even_race() {
    let mut kills = [0u32; 2];
    for seed in 0..40 {
        let mut world = World::new();
        world.rng = MatchRng::new(&[7; 32], seed);
        let at = Vec2::from_ints(5000, 5000);
        let mark = world.spawn_unit(&MELEE_CREEP, Team::Neutral, at);
        let sides = [
            (Team::Radiant, at - Vec2::from_ints(300, 0)),
            (Team::Dire, at + Vec2::from_ints(300, 0)),
        ]
        .map(|(team, from)| {
            let hero = world.spawn_hero(team, from, SlotId(0), HeroId::SHADOW_FIEND);
            if let Some(transform) = world.transform.get_mut(hero) {
                transform.facing = facing_towards(from, at);
            }
            hero
        });
        world.settle();
        world.health.insert(
            mark,
            Health {
                hp: Fixed::from_int(1),
            },
        );
        for hero in sides {
            world.set_order(
                hero,
                UnitOrder::Attack {
                    target: mark,
                    last_seen: at,
                },
            );
        }
        let killer = (0..90)
            .flat_map(|_| world.step())
            .find_map(|event| match event.kind {
                EventKind::Died { unit, killer, .. } if unit == wire_id(mark) => killer,
                _ => None,
            })
            .expect("the mark falls");
        let side = sides
            .iter()
            .position(|hero| wire_id(*hero) == killer)
            .expect("one of the two struck it");
        kills[side] += 1;
    }
    assert!(
        kills.iter().all(|won| *won >= 10),
        "each side takes a fair share of the kills: {kills:?}"
    );
}

/// What one tick of an attacker's cycle shows: `B` in the windup, `A` in
/// the recovery, `.` ready; with `*` when a missile of its left this tick
/// and `!` when its target lost health.
fn cycle_marks(world: &mut World, attacker: Entity, mark: Entity, ticks: u32) -> String {
    let shots = |world: &World| {
        world
            .entities
            .iter()
            .filter(|e| {
                world
                    .projectile
                    .get(*e)
                    .is_some_and(|shot| shot.source == Some(attacker))
            })
            .count()
    };
    let mut marks = String::new();
    for _ in 0..ticks {
        let (hp, flying) = (world.health.get(mark).map(|h| h.hp), shots(world));
        world.step();
        marks.push(match world.action.get(attacker).map(|a| a.state) {
            Some(ActionState::Attack {
                phase: ActionPhase::Before { .. },
                ..
            }) => 'B',
            Some(ActionState::Attack { .. }) => 'A',
            _ => '.',
        });
        if shots(world) > flying {
            marks.push('*');
        }
        if world.health.get(mark).map(|h| h.hp) < hp {
            marks.push('!');
        }
    }
    marks
}

/// The first tick in `marks` that carries `mark`, counted from zero.
fn first_tick_with(marks: &str, mark: char) -> Option<usize> {
    let at = marks.find(mark)?;
    Some(marks[..at].matches(['B', 'A', '.']).count() - 1)
}

/// Each run of one phase in `marks`, as (what, ticks), dropping the shot and
/// hit marks into the run they fell in.
fn phase_runs(marks: &str) -> Vec<(char, usize)> {
    let mut runs: Vec<(char, usize)> = Vec::new();
    for phase in marks.chars().filter(|c| matches!(c, 'B' | 'A' | '.')) {
        match runs.last_mut() {
            Some((last, len)) if *last == phase => *len += 1,
            _ => runs.push((phase, 1)),
        }
    }
    runs
}

/// Dota's attack timings, tick for tick at 30 ticks a second: the windup of
/// the attack point, the hit or the missile at its end, the backswing, and
/// the attack time between two starts, all at the attacker's attack speed.
/// Lane creeps (100 attack speed): melee 0.467 + 0.533 every 1 s, ranged
/// 0.5 + 0.3 every 1 s, siege 0.7 + 1.3 every 3 s. Shadow Fiend at level one
/// (125 attack speed): 0.5 + 0.54 every 1.6 s, each divided by 1.25. A giant wolf
/// (likewise 125): 0.33 + 0.64 every 2 s.
#[test]
fn the_attack_cycle_keeps_dotas_timings() {
    use bota_core::{RANGED_CREEP, SIEGE_CREEP, UnitDef};
    let creep = |def: &'static UnitDef, gap: i32| {
        let mut world = World::new();
        let at = Vec2::from_ints(5000, 5000);
        let attacker = world.spawn_unit(def, Team::Radiant, at);
        let mark = world.spawn_unit(&SIEGE_CREEP, Team::Dire, at + Vec2::from_ints(gap, 0));
        world.settle();
        world.set_target(attacker, mark);
        let marks = cycle_marks(&mut world, attacker, mark, 181);
        (
            marks,
            world.stats.get(attacker).expect("settled").attack_speed,
        )
    };
    for (def, gap, windup, backswing, every, launch) in [
        (&MELEE_CREEP, 100, 14, 16, 30, '!'),
        (&RANGED_CREEP, 400, 15, 9, 30, '*'),
        (&SIEGE_CREEP, 400, 21, 39, 90, '*'),
        (NeutralKind::GiantWolf.def(), 100, 8, 16, 48, '!'),
    ] {
        let (marks, speed) = creep(def, gap);
        assert_eq!(speed, def.attack_speed);
        let runs = phase_runs(&marks);
        assert_eq!(runs[0], ('B', windup), "{:?} windup: {marks}", def.kind);
        assert_eq!(
            runs[1],
            ('A', backswing),
            "{:?} backswing: {marks}",
            def.kind
        );
        assert_eq!(
            first_tick_with(&marks, launch),
            Some(windup),
            "{:?} lets go at the point, the windup's ticks after it began: {marks}",
            def.kind
        );
        let starts: Vec<usize> = phase_runs(&marks)
            .iter()
            .scan(0, |at, (phase, len)| {
                let start = *at;
                *at += len;
                Some((*phase == 'B').then_some(start))
            })
            .flatten()
            .collect();
        assert!(
            starts.windows(2).all(|pair| pair[1] - pair[0] == every),
            "{:?} attacks every {every} ticks: {marks}",
            def.kind
        );
    }
    // Shadow Fiend, ordered to deny a creep of his own in reach that he
    // faces: nothing else draws him, so the order alone starts the swing.
    let mut world = World::new();
    let fiend = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5400, 5000));
    world.settle();
    world.health.insert(
        mark,
        Health {
            hp: Fixed::from_int(250),
        },
    );
    world.step();
    assert_eq!(swinging(&world, fiend), None, "idle until ordered");
    attack_click(&mut world, mark);
    assert!(
        swinging(&world, fiend).is_some(),
        "the windup begins on the order's tick"
    );
    assert_eq!(world.stats.get(fiend).expect("settled").attack_speed, 125);
    let marks = cycle_marks(&mut world, fiend, mark, 120);
    let runs = phase_runs(&marks);
    assert_eq!(runs[0], ('B', 11), "the rest of a 12-tick windup: {marks}");
    assert_eq!(
        first_tick_with(&marks, '*').map(|tick| tick + 1),
        Some(12),
        "the missile leaves 12 ticks after the order: {marks}"
    );
    assert_eq!(runs[1], ('A', 13), "a 13-tick backswing: {marks}");
    let cycle: Vec<usize> = runs.iter().map(|(_, len)| *len).collect();
    // Windup + backswing + idle per swing: 1.28 s is 38.4 ticks, so the
    // starts come 38 or 39 ticks apart.
    let periods: Vec<usize> = cycle[1..].chunks(3).map(|c| c.iter().sum()).collect();
    assert!(
        periods.iter().take(2).all(|p| *p == 38 || *p == 39),
        "Shadow Fiend swings every 1.28 s: {marks}"
    );
}

/// Pudge swings by hand at Liquipedia's 0.5 + 1.17: the windup and the
/// backswing, tick for tick at his attack speed, and the strike between them.
#[test]
fn pudge_recovers_from_a_swing_for_1_17_seconds() {
    let mut world = World::new();
    let pudge = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId::PUDGE);
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5150, 5000));
    world.settle();
    world.health.insert(
        mark,
        Health {
            hp: Fixed::from_int(250),
        },
    );
    world.step();
    attack_click(&mut world, mark);
    assert_eq!(world.stats.get(pudge).expect("settled").attack_speed, 113);
    let marks = cycle_marks(&mut world, pudge, mark, 120);
    let runs = phase_runs(&marks);
    // At 113 attack speed: 0.5 s is a 13-tick windup before the blow, 1.17 s
    // a backswing of 31 ticks from it (it was 400 ms, 11 ticks).
    assert_eq!(runs[0], ('B', 13), "the windup: {marks}");
    assert_eq!(runs[1], ('A', 31), "the backswing: {marks}");
}
