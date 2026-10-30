//! Turning: a body comes round at its hero's turn rate, whatever orders arrive while it does.

use bota_core::{angle_delta, facing_gap, rules};

use crate::game::{Entity, World};
use bota_proto::{HeroId, Order, Target, Vec2};

use super::support::*;

/// Brads per tick each hero turns at, as Liquipedia's *Turn Rate* reads a
/// `MovementTurnRate`: radians per 0.03 s, taken at the 30 Hz tick.
fn rate_of(world: &World, hero: Entity) -> i32 {
    i32::from(world.stats.get(hero).expect("has stats").turn_rate)
}

fn facing_of(world: &World, hero: Entity) -> u16 {
    world.transform.get(hero).expect("alive").facing.brads
}

fn spot_beside(world: &World, hero: Entity, dx: i32, dy: i32) -> Target {
    let at = world.transform.get(hero).expect("alive").pos;
    Target::Pos(at + Vec2::from_ints(dx, dy))
}

/// Tens of orders, sides alternating every `period` ticks: no tick turns the
/// body by more than its rate.
fn spam_orders(hero_id: HeroId, period: u32, order_of: impl Fn(Target) -> Order) {
    let mut world = World::new();
    let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
    world.settle();
    let rate = rate_of(&world, hero);
    let mut before = facing_of(&world, hero);
    let mut turned = 0;
    for tick in 0..240u32 {
        if tick % period == 0 {
            let side = if (tick / period).is_multiple_of(2) {
                -1
            } else {
                1
            };
            let target = spot_beside(&world, hero, -side * 1500, side * 1500);
            seat_order(&mut world, order_of(target));
        } else {
            world.step();
        }
        let now = facing_of(&world, hero);
        let step = angle_delta(
            bota_proto::Angle { brads: before },
            bota_proto::Angle { brads: now },
        )
        .abs();
        assert!(
            step <= rate,
            "{hero_id:?}: tick {tick} turned {step} brads, more than its rate {rate}"
        );
        turned += step;
        before = now;
    }
    assert!(turned > 0, "the body did turn");
}

#[test]
fn move_orders_every_tick_never_snap_the_facing() {
    for hero in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        for period in [1, 2, 3, 5] {
            spam_orders(hero, period, |target| Order::Move { target });
        }
    }
}

#[test]
fn attack_move_and_stop_orders_never_snap_the_facing() {
    for hero in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        for period in [1, 2, 3] {
            spam_orders(hero, period, |target| Order::Attack { target });
        }
    }
}

#[test]
fn a_hero_sent_the_other_way_comes_round_in_dotas_time_and_walks_off_after() {
    // Liquipedia, Turn Rate: a unit turns `MovementTurnRate` radians every
    // 0.03 s, so at the 30 Hz tick it turns 1/0.9 of that a tick; it sets
    // off once within 11.5 degrees (TURN_TOLERANCE) of its way.
    for (hero_id, dota_rate) in [(HeroId::SHADOW_FIEND, 0.9_f64), (HeroId::PUDGE, 0.7)] {
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
        world.settle();
        let start = world.transform.get(hero).expect("alive").pos;
        let radians_a_tick = dota_rate * (1.0 / 30.0) / 0.03;
        let to_walk = (180.0_f64.to_radians() - 11.5_f64.to_radians()) / radians_a_tick;
        let expected = to_walk.ceil() as u32;
        let mut stood = 0u32;
        let target = spot_beside(&world, hero, -2000, 0);
        seat_order(&mut world, Order::Move { target });
        while world.transform.get(hero).expect("alive").pos == start {
            world.step();
            stood += 1;
            assert!(stood < 60, "{hero_id:?} never set off");
        }
        // The order's own tick and the tick of the first step bracket it.
        assert!(
            (stood + 1).abs_diff(expected) <= 1,
            "{hero_id:?} stood {stood} ticks, Dota's turn takes {expected}"
        );
    }
}

#[test]
fn stop_and_move_orders_alternating_never_snap_the_facing() {
    for hero in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let mut world = World::new();
        let id = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero);
        world.settle();
        let rate = rate_of(&world, id);
        let mut before = facing_of(&world, id);
        for tick in 0..120i32 {
            let order = match tick % 3 {
                0 => Order::Move {
                    target: spot_beside(&world, id, -1500 * (1 - tick % 2 * 2), 700),
                },
                1 => Order::Move {
                    target: Target::None,
                },
                _ => Order::Attack {
                    target: Target::None,
                },
            };
            seat_order(&mut world, order);
            let now = facing_of(&world, id);
            let step = angle_delta(
                bota_proto::Angle { brads: before },
                bota_proto::Angle { brads: now },
            )
            .abs();
            assert!(step <= rate, "{hero:?}: tick {tick} turned {step}");
            before = now;
        }
    }
}

/// Casts slot 0 at a spot straight behind the caster and returns the ticks
/// that passed before the cast point began, and how far off the aim it
/// faced then.
fn turn_before_a_cast(hero_id: HeroId, spot: Vec2) -> (u32, u16, i32) {
    let mut world = World::new();
    let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
    set_level(&mut world, hero, 0, 1);
    world.settle();
    world.fill_pools(hero);
    let rate = rate_of(&world, hero);
    let mut before = facing_of(&world, hero);
    cast(&mut world, 0, Target::Pos(spot));
    let mut ticks = 1;
    while !world.is_casting(hero) {
        let now = facing_of(&world, hero);
        let step = angle_delta(
            bota_proto::Angle { brads: before },
            bota_proto::Angle { brads: now },
        )
        .abs();
        assert!(step <= rate, "{hero_id:?} turned {step} in a tick");
        before = now;
        world.step();
        ticks += 1;
        assert!(ticks < 60, "{hero_id:?} never began its cast");
    }
    let wanted = bota_core::facing_towards(world.pos(hero).expect("alive"), spot);
    let gap = facing_gap(world.transform.get(hero).expect("alive").facing, wanted);
    (ticks, gap, rate)
}

#[test]
fn a_cast_aimed_behind_the_caster_begins_once_it_has_come_round() {
    // Dota turns a caster to its aim before the cast point starts.
    for hero in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let (ticks, gap, rate) = turn_before_a_cast(hero, Vec2::from_ints(5500, 6000));
        assert!(
            gap <= rules::ATTACK_ANGLE_BRADS,
            "{hero:?} began its cast {gap} brads off its aim"
        );
        let half_turn = 32768 - i32::from(rules::ATTACK_ANGLE_BRADS);
        assert!(
            ticks as i32 >= half_turn / rate,
            "{hero:?} began its cast after {ticks} ticks, before it could have come round"
        );
    }
}

#[test]
fn attack_orders_on_creeps_either_side_never_snap_the_facing() {
    use bota_core::MELEE_CREEP;
    use bota_proto::Team;
    for hero_id in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
        let east = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(6200, 6000));
        let west = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5800, 6000));
        world.settle();
        let rate = rate_of(&world, hero);
        let mut before = facing_of(&world, hero);
        for tick in 0..90u32 {
            let on = if (tick / 2).is_multiple_of(2) {
                west
            } else {
                east
            };
            attack_click(&mut world, on);
            let now = facing_of(&world, hero);
            let step = angle_delta(
                bota_proto::Angle { brads: before },
                bota_proto::Angle { brads: now },
            )
            .abs();
            assert!(step <= rate, "{hero_id:?}: tick {tick} turned {step}");
            before = now;
        }
    }
}
