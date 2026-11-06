//! Turning: a body comes round at its hero's turn rate, whatever orders arrive while it does.

use bota_core::{ModifierKind, angle_delta, facing_gap, rules};

use crate::game::{Entity, World};
use bota_proto::{HeroId, ItemId, ItemSlot, Order, Target, Team, UnitKind, Vec2};

use super::support::*;

/// Brads per tick each hero turns at, as Liquipedia's *Turn Rate* reads a
/// `MovementTurnRate`: radians per 0.03 s, taken at the 30 Hz tick.
fn rate_of(world: &World, hero: Entity) -> i32 {
    i32::from(world.stats.get(hero).expect("has stats").turn_rate)
}

fn facing_of(world: &World, hero: Entity) -> u16 {
    world.transform.get(hero).expect("alive").facing.brads
}

#[test]
fn measured_quarter_and_half_turns_match_dotas_formula_before_attacking() {
    // Valve's MovementTurnRate and https://liquipedia.net/dota2/Turn_Rate:
    // time = 0.03 * angle / rate, rounded up to whole simulation ticks.
    for (hero_id, dota_rate) in [(HeroId::SHADOW_FIEND, 0.9_f64), (HeroId::PUDGE, 0.7)] {
        for (degrees, offset) in [
            (90, Vec2::from_ints(0, 120)),
            (180, Vec2::from_ints(-120, 0)),
        ] {
            let mut world = World::new();
            let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
            let enemy = seat_hero(
                &mut world,
                1,
                Team::Dire,
                Vec2::from_ints(6000, 6000) + offset,
            );
            world.settle();
            let target = bota_core::facing_towards(
                world.pos(hero).expect("hero"),
                world.pos(enemy).expect("enemy"),
            );
            let seconds = 0.03 * f64::from(degrees).to_radians() / dota_rate;
            let expected = (seconds * 30.0).ceil() as u32;
            let mut trace = Vec::new();
            let mut previous = world.transform.get(hero).expect("hero").facing;
            let mut began = None;
            let mut first_permitted = None;
            let mut completed = None;
            for tick in 1..=12 {
                if tick == 1 {
                    attack_click(&mut world, enemy);
                } else {
                    world.step();
                }
                let now = world.transform.get(hero).expect("hero").facing;
                let step = bota_core::angle_delta(previous, now).abs();
                assert!(
                    step <= rate_of(&world, hero),
                    "one tick respects the hero's rate"
                );
                trace.push(now.brads);
                if facing_gap(now, target) <= rules::ATTACK_ANGLE_BRADS {
                    first_permitted.get_or_insert(tick);
                }
                if swinging(&world, hero).is_some() {
                    assert!(
                        facing_gap(now, target) <= rules::ATTACK_ANGLE_BRADS,
                        "windup waits for the allowed angle"
                    );
                    began.get_or_insert(tick);
                }
                if facing_gap(now, target) == 0 {
                    completed = Some(tick);
                    break;
                }
                previous = now;
            }
            eprintln!(
                "TURN {hero_id:?} {degrees}deg formula={seconds:.6}s expected={expected}ticks measured={completed:?} windup={began:?} facing_brads={trace:?}"
            );
            assert_eq!(
                completed,
                Some(expected),
                "full turn matches the formula at 30 Hz"
            );
            assert!(began.is_some(), "windup starts once facing permits");
            assert_eq!(
                began, first_permitted,
                "{hero_id:?} {degrees} degrees: windup begins on the first permitted tick"
            );
        }
    }
}

fn spot_beside(world: &World, hero: Entity, dx: i32, dy: i32) -> Target {
    let at = world.transform.get(hero).expect("alive").pos;
    Target::Pos(at + Vec2::from_ints(dx, dy))
}

/// Ticks a body turning at `rate` brads a tick takes to come round from
/// facing straight away to within the angle a swing or a cast allows.
fn half_turn_ticks(rate: i32) -> u32 {
    let to_turn = 32768 - i32::from(rules::ATTACK_ANGLE_BRADS);
    (to_turn as u32).div_ceil(rate as u32)
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
        assert!(
            ticks >= half_turn_ticks(rate),
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

/// Steps the world until `done` holds, and answers the ticks that took,
/// counting the tick an order was given in as the first; no tick on the
/// way turns the hero by more than its rate.
fn ticks_turning_until(world: &mut World, hero: Entity, done: impl Fn(&World) -> bool) -> u32 {
    let rate = rate_of(world, hero);
    let mut before = facing_of(world, hero);
    let mut ticks = 1;
    loop {
        let now = facing_of(world, hero);
        let step = angle_delta(
            bota_proto::Angle { brads: before },
            bota_proto::Angle { brads: now },
        )
        .abs();
        assert!(
            step <= rate,
            "tick {ticks} turned {step} brads, more than the rate {rate}"
        );
        before = now;
        if done(world) {
            return ticks;
        }
        world.step();
        ticks += 1;
        assert!(ticks < 60, "never done");
    }
}

#[test]
fn an_attack_ordered_from_behind_in_reach_swings_only_once_come_round_at_the_rate() {
    // Liquipedia, *Turn Rate*: a unit attacking a target first turns until
    // it faces it within 11.5 degrees, then begins its attack point.
    for hero_id in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
        let enemy = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5880, 6000));
        world.settle();
        let away = facing_gap(
            world.transform.get(hero).expect("alive").facing,
            bota_core::facing_towards(
                world.pos(hero).expect("alive"),
                world.pos(enemy).expect("alive"),
            ),
        );
        assert_eq!(away, 32768, "{hero_id:?} starts with its back to the enemy");
        let rate = rate_of(&world, hero);
        attack_click(&mut world, enemy);
        let ticks = ticks_turning_until(&mut world, hero, |world| swinging(world, hero).is_some());
        assert_eq!(
            ticks,
            half_turn_ticks(rate),
            "{hero_id:?} began its attack point after {ticks} ticks; its turn rate takes {}",
            half_turn_ticks(rate)
        );
        let gap = facing_gap(
            world.transform.get(hero).expect("alive").facing,
            bota_core::facing_towards(
                world.pos(hero).expect("alive"),
                world.pos(enemy).expect("alive"),
            ),
        );
        assert!(
            gap <= rules::ATTACK_ANGLE_BRADS,
            "{hero_id:?} swung {gap} brads off its target"
        );
    }
}

#[test]
fn a_blink_aimed_behind_goes_off_only_once_the_hero_has_come_round() {
    // Dota turns a hero to the blink's direction before it blinks (Blink
    // Dagger changelog, 18 Jan 2011: "Heroes using the item's ability now
    // turn to face the blink direction"), though Blink has no cast point;
    // only an immediate item (a Healing Salve, a Clarity) goes off unturned.
    for hero_id in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
        hand_item(&mut world, hero, ItemId::BLINK_DAGGER, 0);
        world.settle();
        let start = world.pos(hero).expect("alive");
        let aim = start - Vec2::from_ints(600, 0);
        let rate = rate_of(&world, hero);
        seat_order(
            &mut world,
            Order::Use {
                slot: ItemSlot(0),
                target: Target::Pos(aim),
            },
        );
        let ticks = ticks_turning_until(&mut world, hero, |world| world.pos(hero) != Some(start));
        assert_eq!(
            ticks,
            half_turn_ticks(rate),
            "{hero_id:?} blinked after {ticks} ticks; its turn rate takes {}",
            half_turn_ticks(rate)
        );
        let gap = facing_gap(
            world.transform.get(hero).expect("alive").facing,
            bota_core::facing_towards(start, aim),
        );
        assert!(
            gap <= rules::ATTACK_ANGLE_BRADS,
            "{hero_id:?} blinked {gap} brads off its aim"
        );
    }
}

#[test]
fn a_rooted_hero_comes_round_to_a_ward_placed_behind_and_places_it() {
    // A root stops the walk, not the turn: the hero turns where it stands
    // and the ward goes down once it faces the spot.
    for hero_id in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(6000, 6000), hero_id);
        hand_item(&mut world, hero, ItemId::OBSERVER_WARD, 1);
        world.settle();
        put_on(&mut world, hero, ModifierKind::Rooted, 300);
        let spot = world.pos(hero).expect("alive") - Vec2::from_ints(300, 0);
        let rate = rate_of(&world, hero);
        seat_order(
            &mut world,
            Order::Use {
                slot: ItemSlot(0),
                target: Target::Pos(spot),
            },
        );
        let ward_stands = |world: &World| {
            world
                .entities
                .iter()
                .any(|entity| world.kind.get(entity) == Some(&UnitKind::Ward))
        };
        let ticks = ticks_turning_until(&mut world, hero, ward_stands);
        assert_eq!(
            ticks,
            half_turn_ticks(rate),
            "{hero_id:?} placed its ward after {ticks} ticks; its turn rate takes {}",
            half_turn_ticks(rate)
        );
        assert!(world.rooted(hero), "{hero_id:?} was rooted all the while");
    }
}
