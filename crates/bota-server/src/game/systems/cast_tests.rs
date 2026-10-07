//! Casting through the body: what goes off at once, what holds the body,
//! and how what holds it ends.

use crate::game::{
    ActionPhase, ActionState, Command, Entity, Level, MELEE_CREEP, Modifier, ModifierKind,
    PendingCast, Seat, World, rules, wire_id,
};
use bota_proto::{AbilitySlot, DamageKind, HeroId, Order, SlotId, Target, Team, Vec2};

use super::test_support::ANVIL;

/// Pudge in a seat with the dismember learned, and a mark 100 off.
fn pudge_and_a_mark() -> (World, Entity, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId(1),
    );
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    world.seats.push(Seat::new(
        SlotId(0),
        Team::Radiant,
        HeroId(1),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(pudge);
    world.settle();
    world.fill_pools(pudge);
    world.step();
    (world, pudge, mark)
}

/// Sylla with the frenzy learned, set on an anvil in reach and two ticks
/// into a swing at it.
fn sylla_mid_swing() -> (World, Entity) {
    let mut world = World::new();
    let sylla = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId(0),
    );
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.level.insert(sylla, Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(sylla);
    assert!(world.learn(sylla, 1), "the frenzy is learned");
    world.set_target(sylla, anvil);
    world.step();
    world.step();
    assert!(
        matches!(
            world.action.get(sylla).map(|action| action.state),
            Some(ActionState::Attack {
                phase: ActionPhase::Before { .. },
                ..
            })
        ),
        "two ticks in, the swing is under way"
    );
    (world, sylla)
}

/// Orders the seat's Pudge to dismember the mark.
fn dismember(world: &mut World, mark: Entity) {
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cast {
            slot: AbilitySlot(3),
            target: Target::Unit(wire_id(mark)),
        },
    }]);
}

/// Whether an entity carries a modifier of a kind put there by somebody.
fn put_on(world: &World, entity: Entity, kind: ModifierKind, by: Entity) -> bool {
    let same = std::mem::discriminant(&kind);
    world.modifiers.get(entity).is_some_and(|on_it| {
        on_it
            .active()
            .any(|held| std::mem::discriminant(&held.kind) == same && held.source == Some(by))
    })
}

/// Which of the stun, the burn and the mending a dismember puts on are on.
fn dismember_parts(world: &World, pudge: Entity, mark: Entity) -> [bool; 3] {
    let burn = ModifierKind::Burning {
        amount: 0,
        kind: DamageKind::Pure,
        lethal: true,
    };
    let mend = ModifierKind::Mending {
        per_tick: 0,
        breaks: false,
    };
    [
        put_on(world, mark, ModifierKind::Stunned, pudge),
        put_on(world, mark, burn, pudge),
        put_on(world, pudge, mend, pudge),
    ]
}

#[test]
fn a_cast_that_holds_the_body_for_no_time_goes_off_mid_swing_on_the_tick_it_is_ordered() {
    let (mut world, sylla) = sylla_mid_swing();
    let full = world.mana.get(sylla).expect("has mana").mana;
    world.order_cast(
        sylla,
        PendingCast::Ability {
            slot: AbilitySlot(1),
            target: Target::None,
        },
    );
    world.step();
    assert!(
        world.mana.get(sylla).expect("has mana").mana < full,
        "it went off that very tick"
    );
    assert_eq!(world.pending_cast(sylla), None, "and is not owed any more");
    assert!(
        matches!(
            world.action.get(sylla).map(|action| action.state),
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
        "it takes hold on the tick it is ordered"
    );
    assert_eq!(
        dismember_parts(&world, pudge, mark),
        [true; 3],
        "and puts on all of it"
    );
    for tick in 1..rules::DISMEMBER_TICKS {
        world.step();
        assert!(world.is_channelling(pudge), "still holding {tick} ticks in");
        assert_eq!(
            dismember_parts(&world, pudge, mark),
            [true; 3],
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
        [false; 3],
        "and takes off everything it put on"
    );
}

/// Something done to a world holding a dismember, given Pudge and the mark.
type BreakOff = fn(&mut World, Entity, Entity);

#[test]
fn a_stun_on_the_one_holding_or_a_mark_out_of_reach_breaks_a_dismember_off() {
    let breaks: [(&str, BreakOff); 2] = [
        ("stunned", |world, pudge, _| {
            let stun = Modifier {
                kind: ModifierKind::Stunned,
                source: None,
                ticks_left: Some(5),
            };
            world.put_modifier(pudge, stun);
        }),
        ("out of reach", |world, _, mark| {
            world.transform.get_mut(mark).expect("standing").pos = Vec2::from_ints(7000, 5000);
        }),
    ];
    for (case, break_off) in breaks {
        let (mut world, pudge, mark) = pudge_and_a_mark();
        dismember(&mut world, mark);
        for _ in 0..10 {
            world.step();
        }
        assert!(world.is_channelling(pudge), "{case}: it holds until then");
        break_off(&mut world, pudge, mark);
        world.step();
        assert!(!world.is_channelling(pudge), "{case}: it lets go");
        assert_eq!(
            dismember_parts(&world, pudge, mark),
            [false; 3],
            "{case}: and takes off everything it put on"
        );
    }
}
