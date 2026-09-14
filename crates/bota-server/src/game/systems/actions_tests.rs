//! The attack cycle: exact counts at every speed, and the old ticks at the
//! base one.

use bota_proto::{Fixed, Team, Vec2};

use crate::game::{
    Entity, MELEE_CREEP, Modifier, ModifierKind, Modifiers, UnitDef, World, attack_gain, beats,
    rules,
};

/// What takes a strike and never falls: no armor, no mending, no swing back.
const ANVIL: UnitDef = UnitDef {
    max_hp: 30_000,
    armor: 0,
    damage: 0,
    hp_regen: Fixed::ZERO,
    ..MELEE_CREEP
};

/// A melee attacker worth one point a strike, at a cycle of its own.
const fn pace(attack_time: u32, attack_point: u32, attack_backswing: u32) -> UnitDef {
    UnitDef {
        attack_time,
        attack_point,
        attack_backswing,
        damage: 1,
        ..MELEE_CREEP
    }
}

const HERO_PACE: UnitDef = pace(2100, 300, 400);
const CREEP_PACE: UnitDef = pace(1000, 466, 500);
const FOUNTAIN_PACE: UnitDef = pace(166, 33, 66);
const FAST_PACE: UnitDef = pace(150, 50, 50);
const TOWER_PACE: UnitDef = pace(966, 200, 133);
const SIEGE_PACE: UnitDef = pace(3000, 700, 500);

const PACES: [&UnitDef; 6] = [
    &HERO_PACE,
    &CREEP_PACE,
    &FOUNTAIN_PACE,
    &FAST_PACE,
    &TOWER_PACE,
    &SIEGE_PACE,
];

const SPEEDS: [i32; 19] = [
    20, 21, 22, 33, 57, 99, 100, 101, 133, 150, 200, 299, 333, 400, 499, 500, 600, 699, 700,
];

/// Stands an attacker of a kind at a speed against an anvil in reach, set
/// on it and looking at it.
fn duel(attacker: &'static UnitDef, speed: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let from = world.spawn_unit(attacker, Team::Radiant, at);
    let on = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.modifiers.insert(
        from,
        Modifiers(vec![Modifier {
            kind: ModifierKind::Haste {
                speed: speed - rules::BASE_ATTACK_SPEED,
            },
            source: None,
            ticks_left: Some(u32::MAX / 2),
        }]),
    );
    world.settle();
    world.set_target(from, on);
    (world, from, on)
}

/// Points taken off an entity since it stood up full.
fn taken(world: &World, on: Entity) -> i32 {
    let stats = world.stats.get(on).expect("settled");
    let health = world.health.get(on).expect("standing");
    (stats.max_hp - health.hp).to_int()
}

/// Strikes a cycle lands by the end of so many steps, the windup beginning
/// on the first step and adding nothing that step.
fn closed_form(steps: u32, gain: u32, hit_at: u32, len: u32) -> i32 {
    let elapsed = (steps - 1) * gain;
    if elapsed < hit_at {
        0
    } else {
        ((elapsed - hit_at) / len + 1) as i32
    }
}

#[test]
fn strikes_match_the_closed_form_at_every_speed() {
    for def in PACES {
        for speed in SPEEDS {
            let (mut world, from, on) = duel(def, speed);
            let stats = *world.stats.get(from).expect("settled");
            assert_eq!(stats.attack_speed, speed);
            let gain = attack_gain(speed);
            let hit_at = beats(def.attack_point);
            let len = beats(def.attack_time);
            for step in 1..=400 {
                world.step();
                assert_eq!(
                    taken(&world, on),
                    closed_form(step, gain, hit_at, len),
                    "attack_time {} at speed {speed}, step {step}",
                    def.attack_time
                );
            }
        }
    }
}

#[test]
fn a_fast_cycle_lands_two_strikes_in_one_tick() {
    let (mut world, _from, on) = duel(&FAST_PACE, 700);
    let mut best = 0;
    let mut before = 0;
    for _ in 0..60 {
        world.step();
        let now = taken(&world, on);
        best = best.max(now - before);
        before = now;
    }
    assert_eq!(best, 2, "at 1.55 strikes a tick some tick carries two");
}

#[test]
fn the_base_speed_keeps_the_old_ticks() {
    let (mut world, _from, on) = duel(&HERO_PACE, 100);
    let mut landed = Vec::new();
    let mut before = 0;
    for step in 1..=200 {
        world.step();
        let now = taken(&world, on);
        if now > before {
            landed.push(step);
        }
        before = now;
    }
    assert_eq!(
        landed,
        vec![10, 73, 136, 199],
        "nine ticks to the first hit, sixty-three between"
    );
}

#[test]
fn a_swing_that_lands_holds_the_body_for_the_backswing_and_no_longer() {
    let (mut world, from, _on) = duel(&HERO_PACE, 100);
    let mut rooted = Vec::new();
    for step in 1..=80 {
        world.step();
        let held = matches!(
            world.action.get(from).map(|a| a.state),
            Some(crate::game::ActionState::Attack { .. })
        );
        if held {
            rooted.push(step);
        }
    }
    // The windup runs from the first step to the hit on the tenth, the
    // backswing twelve ticks past it; the next windup begins on the
    // sixty-fourth.
    let expected: Vec<u32> = (1..=21).chain(64..=80).collect();
    assert_eq!(rooted, expected);
}
