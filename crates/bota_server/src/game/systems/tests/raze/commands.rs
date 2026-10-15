use bota_core::rules;

use crate::game::{Action, ActionState, Entity, Event, World, fountain_pos, wire_id};
use bota_proto::{AbilitySlot, EventKind, Fixed, ItemId, ItemSlot, Order, SlotId, Target, Team};

use super::fixtures::*;

/// Ticks a raze stands in its cast point past the tick it is ordered on.
const CAST_POINT_TICKS: u32 = 17;

/// Orders seat 0's hero to raze from a slot and runs the world until the
/// raze has gone off: every event on the way.
fn raze_from(world: &mut World, caster: Entity, slot: u8) -> Vec<Event> {
    if world.action.get(caster).is_none() {
        world.action.insert(
            caster,
            Action {
                state: ActionState::Ready,
                attack_cooldown: 0,
            },
        );
    }
    let cast = Order::Cast {
        slot: AbilitySlot(slot),
        target: Target::None,
    };
    let mut events = order(world, cast);
    for _ in 0..CAST_POINT_TICKS {
        assert!(world.is_casting(caster), "still in the cast point");
        events.extend(world.step());
    }
    assert!(!world.is_casting(caster), "and it has gone off");
    events
}

#[test]
fn mango_mana_enables_a_validated_raze_without_changing_any_strategy() {
    let (mut world, caster, target) = fixture();
    seat(&mut world, SlotId(0), Team::Radiant, caster, 65);
    world.transform.get_mut(caster).unwrap().pos = fountain_pos(world.map, Team::Radiant);
    order(&mut world, Order::Buy { item: ItemId(42) });
    world.transform.get_mut(caster).unwrap().pos = ORIGIN;
    world.mana.get_mut(caster).unwrap().mana = Fixed::ZERO;
    let use_mango = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    let consumed = order(&mut world, use_mango);
    assert_eq!(consumed.len(), 1);
    assert_eq!(
        consumed[0].kind,
        EventKind::Healed {
            source: Some(wire_id(caster)),
            target: wire_id(caster),
            amount: 0,
            mana: 100,
        }
    );
    assert_eq!(world.mana.get(caster).unwrap().mana, Fixed::from_int(100));
    reveal(&mut world, target);
    let events = raze_from(&mut world, caster, 1);
    assert_eq!(damage(&events, target), vec![85]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::Healed { .. }))
    );
    assert_eq!(world.mana.get(caster).unwrap().mana, Fixed::from_int(25));
    assert_eq!(effects(&world, target), vec![effect(1, 180)]);
    assert!(world.inventory.get(caster).unwrap().slots[0].is_none());
}

#[test]
fn raze_validated_reach_orders_share_stacks_but_keep_independent_cooldowns() {
    let (mut world, caster, target) = fixture();
    seat(&mut world, SlotId(0), Team::Radiant, caster, 0);
    for (slot, expected) in [(0, 85), (1, 120), (2, 155)] {
        reveal(&mut world, target);
        assert_eq!(
            damage(&raze_from(&mut world, caster, slot), target),
            vec![expected]
        );
    }
    let book = world.abilities.get(caster).unwrap();
    let between = CAST_POINT_TICKS + 1;
    let full = rules::RAZE_COOLDOWN[0];
    assert_eq!(
        book.slots[..3]
            .iter()
            .map(|slot| slot.cooldown)
            .collect::<Vec<_>>(),
        vec![full - 2 * between, full - between, full],
        "each reach keeps a cooldown of its own"
    );
    assert_eq!(effects(&world, target), vec![effect(3, 180)]);
}
