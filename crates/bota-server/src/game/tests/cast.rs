//! Casting abilities.

use crate::game::rules;
use crate::game::{Entity, MELEE_CREEP, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn frenzy_puts_haste_on_its_caster_and_spends_the_mana() {
    let mut world = World::new();
    let hero = caster(&mut world, bota_proto::Vec2::from_ints(5000, 5000), 1);
    let full = world.mana.get(hero).expect("has mana").mana;
    let plain = world.stats.get(hero).expect("settled").attack_speed;
    world.order_cast(
        hero,
        crate::game::PendingCast::Ability {
            slot: bota_proto::AbilitySlot(1),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert!(
        world.mana.get(hero).expect("has mana").mana < full,
        "a cast costs mana"
    );
    // Stats are worked out before casts run, so what a cast puts on shows
    // from the tick after.
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").attack_speed > plain,
        "and it swings faster while the haste holds"
    );
    assert!(
        world.abilities.get(hero).expect("has a book").slots[1].cooldown > 0,
        "and waits before casting again"
    );
}

#[test]
fn a_cast_with_no_mana_behind_it_does_nothing() {
    let mut world = World::new();
    let hero = caster(&mut world, bota_proto::Vec2::from_ints(5000, 5000), 1);
    if let Some(mana) = world.mana.get_mut(hero) {
        mana.mana = Fixed::ZERO;
    }
    world.order_cast(
        hero,
        crate::game::PendingCast::Ability {
            slot: bota_proto::AbilitySlot(1),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert_eq!(
        world.abilities.get(hero).expect("has a book").slots[1].cooldown,
        0,
        "nothing was spent and nothing began"
    );
}

#[test]
fn a_multishot_strikes_everything_around_and_leaves_allies_be() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let hero = caster(&mut world, at, 3);
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5200, 5000),
    );
    let ally = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5200, 5100),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 5000),
    );
    world.settle();
    world.fill_pools(near);
    world.fill_pools(ally);
    world.fill_pools(far);
    let (was_near, was_ally, was_far) = (
        world.health.get(near).expect("standing").hp,
        world.health.get(ally).expect("standing").hp,
        world.health.get(far).expect("standing").hp,
    );
    world.order_cast(
        hero,
        crate::game::PendingCast::Ability {
            slot: bota_proto::AbilitySlot(3),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert!(
        world.health.get(near).expect("standing").hp < was_near,
        "the one in the ring is struck"
    );
    assert_eq!(
        world.health.get(ally).expect("standing").hp,
        was_ally,
        "its own side is left be"
    );
    assert_eq!(
        world.health.get(far).expect("standing").hp,
        was_far,
        "and the one outside the ring is untouched"
    );
}

#[test]
fn a_point_is_spent_only_when_there_is_one_and_the_level_allows_it() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    world.settle();
    assert!(world.learn(hero, 0), "the first point goes in");
    assert!(
        !world.learn(hero, 1),
        "and the second waits for a second level"
    );
    assert!(
        !world.learn(hero, 0),
        "one ability twice over is no different"
    );
    // Levelled up, the point is there, but the ultimate still waits.
    world.level.insert(hero, crate::game::Level(2));
    assert!(
        !world.learn(hero, 3),
        "the ultimate waits for the level it asks for"
    );
    assert!(world.learn(hero, 1), "a basic one does not");
    // A passive takes points like anything else.
    world.level.insert(hero, crate::game::Level(3));
    assert!(world.learn(hero, 2), "a passive is learned, not cast");
    // Right up to the level it opens on, the ultimate answers.
    world
        .level
        .insert(hero, crate::game::Level(rules::ULT_LEVEL_FLOORS[0]));
    assert!(world.learn(hero, 3), "and then it opens");
    // Nothing goes past its own cap, however many levels are had.
    world.level.insert(hero, crate::game::Level(50));
    for _ in 0..10 {
        world.learn(hero, 0);
    }
    assert_eq!(
        world.abilities.get(hero).expect("casts").slots[0].level,
        rules::ABILITY_MAX_LEVEL,
        "the hook stops at its own cap"
    );
}

#[test]
fn a_cast_aimed_out_of_reach_walks_the_caster_in_and_then_goes_off() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(4000);
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(rules::HOOK_RANGE + 900, 0);
    throw_hook(&mut world, far);
    assert!(
        world.pending_cast(pudge).is_some(),
        "out of reach the cast is held rather than dropped"
    );
    let mut thrown = false;
    for _ in 0..400 {
        world.step();
        if world.entities.iter().any(|e| world.hook.get(e).is_some()) {
            thrown = true;
            break;
        }
    }
    assert!(thrown, "and goes off once the caster has walked in");
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert!(
        stood.x.to_int() > from.x.to_int(),
        "the caster walked at it: {} then {}",
        from.x.to_int(),
        stood.x.to_int()
    );
    assert!(
        stood.within(far, bota_proto::Fixed::from_int(rules::HOOK_RANGE)),
        "and no further than it had to"
    );
}

#[test]
fn a_later_order_calls_a_held_cast_off() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(4000);
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(rules::HOOK_RANGE + 900, 0);
    throw_hook(&mut world, far);
    assert!(
        world.pending_cast(pudge).is_some(),
        "out of reach the cast is held rather than dropped"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(4200, 5000)),
        },
    }]);
    assert_eq!(
        world.pending_cast(pudge),
        None,
        "the later order took the held cast away"
    );
    for _ in 0..120 {
        world.step();
        assert!(
            !world.entities.iter().any(|e| world.hook.get(e).is_some()),
            "and it never goes off"
        );
    }
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert!(
        stood.x < from.x,
        "the body answers the order it was given instead: {stood:?}"
    );
}

#[test]
fn a_cast_walked_towards_a_target_that_fell_is_given_up_and_costs_nothing() {
    let (mut world, pudge, mark) = pudge_and_a_mark(4000);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    world.mana.insert(
        pudge,
        crate::game::Mana {
            mana: Fixed::from_int(110),
        },
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(3),
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    assert!(
        world.pending_cast(pudge).is_some(),
        "out of reach the cast is held while the caster walks in"
    );
    world.push_hit(None, mark, 10_000, bota_proto::DamageKind::Pure);
    for _ in 0..3 {
        world.step();
        assert!(
            !world.is_channelling(pudge),
            "a cast at what has fallen never goes off"
        );
    }
    assert_eq!(
        world.pending_cast(pudge),
        None,
        "the held cast is given up with its target"
    );
    assert_eq!(
        world
            .abilities
            .get(pudge)
            .map(|book| book.slots[3].cooldown),
        Some(0),
        "and nothing was spent on it"
    );
}

#[test]
fn an_aimed_cast_takes_the_bodys_order_over() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    assert!(
        matches!(
            world.orders.get(pudge).map(|o| o.current),
            Some(crate::game::UnitOrder::Attack { .. })
        ),
        "the attack order stands"
    );
    let from = world.transform.get(pudge).expect("standing").pos;
    throw_hook(&mut world, from + bota_proto::Vec2::from_ints(400, 0));
    assert!(
        matches!(
            world.orders.get(pudge).map(|o| o.current),
            Some(crate::game::UnitOrder::Idle)
        ),
        "the cast took the order's place and the attack is not returned to"
    );
}

#[test]
fn a_body_walking_into_a_cast_swings_at_nothing_on_the_way() {
    let (mut world, pudge, mark) = pudge_and_a_mark(300);
    let full = world.health.get(mark).expect("standing").hp;
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(0, rules::HOOK_RANGE + 900);
    throw_hook(&mut world, far);
    for _ in 0..30 {
        world.step();
        assert_eq!(
            world.target_of(pudge),
            None,
            "the body is the cast's until it goes off"
        );
    }
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "nothing was swung at on the way"
    );
}

#[test]
fn a_cast_with_no_mana_is_named_and_refused() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    let aim = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(0),
        target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(5600, 5000)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &aim),
        Ok(()),
        "with the mana for it, it is allowed"
    );
    world
        .mana
        .insert(pudge, crate::game::Mana { mana: Fixed::ZERO });
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &aim),
        Err(bota_proto::RejectReason::NotEnoughMana),
        "and without it the seat is told why"
    );
    // An unlearned slot and a wrongly aimed one are named too.
    let unlearned = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(3),
        target: bota_proto::Target::Unit(crate::game::wire_id(pudge)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &unlearned),
        Err(bota_proto::RejectReason::NotLearned),
        "a slot with no points in it says so"
    );
    // A passive is not a slot with nothing in it: it is one that is never
    // cast at all.
    let passive = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(2),
        target: bota_proto::Target::None,
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &passive),
        Err(bota_proto::RejectReason::NotCastable),
        "and a passive says that instead"
    );
    let wrongly_aimed = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(0),
        target: bota_proto::Target::None,
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &wrongly_aimed),
        Err(bota_proto::RejectReason::WrongTargetKind)
    );
}

#[test]
fn a_bolt_goes_on_to_the_next_and_never_back_to_the_same_one() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let hero = caster(&mut world, at, 2);
    // Three enemies in a row, each within a bounce of the last.
    let marks: Vec<Entity> = (0..3)
        .map(|step| {
            world.spawn_unit(
                &MELEE_CREEP,
                bota_proto::Team::Dire,
                at + bota_proto::Vec2::from_ints(300 + 300 * step, 0),
            )
        })
        .collect();
    world.settle();
    world.step();
    let full: Vec<i32> = marks
        .iter()
        .map(|mark| world.health.get(*mark).expect("standing").hp.to_int())
        .collect();
    world.order_cast(
        hero,
        crate::game::PendingCast::Ability {
            slot: bota_proto::AbilitySlot(2),
            target: bota_proto::Target::Unit(crate::game::wire_id(marks[0])),
        },
    );
    for _ in 0..120 {
        world.step();
    }
    for (mark, was) in marks.iter().zip(full) {
        assert!(
            world.health.get(*mark).expect("standing").hp.to_int() < was,
            "the bolt reached every one of them"
        );
    }
    assert!(
        world
            .entities
            .iter()
            .all(|entity| world.projectile.get(entity).is_none()),
        "and is gone once it runs out of places to go"
    );
}

#[test]
fn a_spell_aimed_at_what_it_cannot_take_is_named_and_refused() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world
        .level
        .insert(hero, crate::game::Level(rules::HERO_MAX_LEVEL));
    assert!(world.learn(hero, 2), "the bolt is learned");
    let ally = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        world.transform.get(hero).expect("standing").pos,
    );
    world.settle();
    world.step();
    let at_an_ally = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(2),
        target: bota_proto::Target::Unit(crate::game::wire_id(ally)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &at_an_ally),
        Err(bota_proto::RejectReason::WrongTargetKind),
        "a bolt at one of your own is refused and named"
    );
}

#[test]
fn one_point_levels_every_raze_at_once_and_costs_one() {
    let mut world = World::new();
    let fiend = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(2),
    );
    world.settle();
    assert!(world.learn(fiend, 2), "the point goes in");
    let levels: Vec<u8> = world
        .abilities
        .get(fiend)
        .expect("casts")
        .slots
        .iter()
        .map(|held| held.level)
        .collect();
    assert_eq!(
        levels,
        vec![1, 1, 1, 0, 0, 0],
        "every raze stands at one, and nothing else moved"
    );
    assert_eq!(
        world.points_spent(fiend),
        1,
        "the three cost one point together"
    );
    assert!(!world.learn(fiend, 3), "and there is nothing left to spend");
    // The trio waits for hero levels the same as any one ability would.
    world.level.insert(fiend, crate::game::Level(2));
    assert!(
        !world.learn(fiend, 0),
        "the second raze level waits for hero level three"
    );
    assert!(
        world.learn(fiend, 3),
        "while the necromastery is open to the spare point"
    );
}

#[test]
fn learning_an_ability_tells_of_no_cast() {
    let (mut world, hero) = a_hero_with_gold(0);

    let events = advance_validated(
        &mut world,
        Some(hero),
        bota_proto::Order::Learn {
            slot: bota_proto::AbilitySlot(0),
        },
    );

    assert_eq!(world.abilities.get(hero).expect("book").slots[0].level, 1);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, bota_proto::EventKind::AbilityCast { .. })),
        "a skill point is not a cast"
    );
}
