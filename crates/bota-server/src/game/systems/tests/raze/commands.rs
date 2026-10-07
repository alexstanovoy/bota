use crate::game::{fountain_pos, wire_id};
use bota_proto::{AbilitySlot, EventKind, Fixed, ItemId, ItemSlot, Order, SlotId, Target, Team};

use super::fixtures::*;

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
    let cast = Order::Cast {
        slot: AbilitySlot(1),
        target: Target::None,
    };
    let events = order(&mut world, cast);
    assert_eq!(damage(&events, target), vec![90]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::Healed { .. }))
    );
    assert_eq!(world.mana.get(caster).unwrap().mana, Fixed::from_int(25));
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
    assert!(world.inventory.get(caster).unwrap().slots[0].is_none());
}

#[test]
fn raze_validated_reach_orders_share_stacks_but_keep_independent_cooldowns() {
    let (mut world, caster, target) = fixture();
    seat(&mut world, SlotId(0), Team::Radiant, caster, 0);
    for (slot, expected) in [(0, 90), (1, 140), (2, 190)] {
        reveal(&mut world, target);
        let cast = Order::Cast {
            slot: AbilitySlot(slot),
            target: Target::None,
        };
        assert_eq!(damage(&order(&mut world, cast), target), vec![expected]);
    }
    let book = world.abilities.get(caster).unwrap();
    assert_eq!(
        book.slots[..3]
            .iter()
            .map(|slot| slot.cooldown)
            .collect::<Vec<_>>(),
        vec![298, 299, 300]
    );
    assert_eq!(effects(&world, target), vec![effect(3, 240)]);
}
