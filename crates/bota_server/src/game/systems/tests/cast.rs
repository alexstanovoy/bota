//! Casting through the body: what goes off at once, what holds the body,
//! and how what holds it ends.

use bota_core::{MELEE_CREEP, ModifierKind, rules};

use crate::game::{ActionPhase, ActionState, Entity, Level, PendingCast, World, wire_id};
use bota_proto::{AbilitySlot, HeroId, SlotId, Target, Team, Vec2};

use super::support::ANVIL;
use crate::game::tests::support::{cast, put_on, run, seated_hero, set_level};

/// Pudge in a seat with the dismember learned, and a mark 100 off.
fn pudge_and_a_mark() -> (World, Entity, Entity) {
    let mut world = World::new();
    let pudge = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId::PUDGE);
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    set_level(&mut world, pudge, 3, 1);
    world.settle();
    world.fill_pools(pudge);
    world.step();
    (world, pudge, mark)
}

/// Shadow Fiend with the feast learned, set on an anvil in reach and two
/// ticks into a swing at it.
fn fiend_mid_swing() -> (World, Entity) {
    let mut world = World::new();
    let fiend = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.level.insert(fiend, Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(fiend);
    assert!(world.learn(fiend, 3), "the feast is learned");
    world.set_target(fiend, anvil);
    world.step();
    world.step();
    assert!(
        matches!(
            world.action.get(fiend).map(|action| action.state),
            Some(ActionState::Attack {
                phase: ActionPhase::Before { .. },
                ..
            })
        ),
        "two ticks in, the swing is under way"
    );
    (world, fiend)
}

/// Orders a dismember and steps until its cast point is over and it holds.
fn dismember(world: &mut World, mark: Entity) {
    cast(world, 3, Target::Unit(wire_id(mark)));
    for _ in 0..rules::DISMEMBER_CAST_POINT_MS / 30 + 2 {
        if dismember_parts(world, world.seats[0].unit.expect("seated"), mark)[0] {
            return;
        }
        world.step();
    }
}

/// Whether an entity carries a modifier of a kind put there by somebody.
fn carries_from(world: &World, entity: Entity, kind: ModifierKind, by: Entity) -> bool {
    let same = std::mem::discriminant(&kind);
    world.modifiers.get(entity).is_some_and(|on_it| {
        on_it
            .active()
            .any(|held| std::mem::discriminant(&held.kind) == same && held.source == Some(by))
    })
}

/// Which of the stun and the hold a dismember puts on the mark are on.
fn dismember_parts(world: &World, pudge: Entity, mark: Entity) -> [bool; 2] {
    [
        carries_from(world, mark, ModifierKind::Stunned, pudge),
        carries_from(world, mark, ModifierKind::Dismembered { level: 0 }, pudge),
    ]
}

#[test]
fn a_cast_that_holds_the_body_for_no_time_goes_off_mid_swing_on_the_tick_it_is_ordered() {
    let (mut world, fiend) = fiend_mid_swing();
    let full = world.mana.get(fiend).expect("has mana").mana;
    world.order_cast(
        fiend,
        PendingCast::Ability {
            slot: AbilitySlot(3),
            target: Target::None,
        },
    );
    world.step();
    assert!(
        world.mana.get(fiend).expect("has mana").mana < full,
        "it went off that very tick"
    );
    assert_eq!(world.pending_cast(fiend), None, "and is not owed any more");
    assert!(
        matches!(
            world.action.get(fiend).map(|action| action.state),
            Some(ActionState::Attack { .. })
        ),
        "while the swing goes on as if nothing happened"
    );
}

#[test]
fn a_dismember_holds_for_exactly_its_ticks_and_then_lets_go() {
    let (mut world, pudge, mark) = pudge_and_a_mark();
    dismember(&mut world, mark);
    assert!(
        world.is_channelling(pudge),
        "it takes hold once its cast point is over"
    );
    assert_eq!(
        dismember_parts(&world, pudge, mark),
        [true; 2],
        "and puts on all of it"
    );
    for tick in 1..rules::DISMEMBER_TICKS {
        world.step();
        assert!(world.is_channelling(pudge), "still holding {tick} ticks in");
        assert_eq!(
            dismember_parts(&world, pudge, mark),
            [true; 2],
            "{tick} ticks in"
        );
    }
    world.step();
    assert!(
        !world.is_channelling(pudge),
        "it lets go once its ticks have run"
    );
    assert_eq!(
        dismember_parts(&world, pudge, mark),
        [false; 2],
        "and takes off everything it put on"
    );
}

/// Something done to a world holding a dismember, given Pudge and the mark.
type BreakOff = fn(&mut World, Entity, Entity);

#[test]
fn a_stun_on_the_one_holding_or_a_mark_out_of_reach_breaks_a_dismember_off() {
    let breaks: [(&str, BreakOff); 2] = [
        ("stunned", |world, pudge, _| {
            put_on(world, pudge, ModifierKind::Stunned, 5);
        }),
        ("out of reach", |world, _, mark| {
            world.transform.get_mut(mark).expect("standing").pos = Vec2::from_ints(7000, 5000);
        }),
    ];
    for (case, break_off) in breaks {
        let (mut world, pudge, mark) = pudge_and_a_mark();
        dismember(&mut world, mark);
        run(&mut world, 10);
        assert!(world.is_channelling(pudge), "{case}: it holds until then");
        break_off(&mut world, pudge, mark);
        world.step();
        assert!(!world.is_channelling(pudge), "{case}: it lets go");
        assert_eq!(
            dismember_parts(&world, pudge, mark),
            [false; 2],
            "{case}: and takes off everything it put on"
        );
    }
}
