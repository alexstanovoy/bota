use bota_proto::{
    AbilitySlot, EventKind, Fixed, HeroId, ItemId, ItemSlot, Order, SlotId, Target, Team,
};

use crate::game::{Command, Seat, rules};

use super::fixtures::*;

#[test]
fn mango_mana_enables_a_validated_raze_without_changing_any_strategy() {
    let (mut world, caster, target) = fixture();
    let mut seat = Seat::new(SlotId(0), Team::Radiant, HeroId(2), 65, rules::STASH_SLOTS);
    seat.unit = Some(caster);
    world.seats.push(seat);
    world.transform.get_mut(caster).unwrap().pos =
        crate::game::fountain_pos(world.map, Team::Radiant);
    let buy = Order::Buy { item: ItemId(42) };
    assert_eq!(world.validate_order(SlotId(0), None, &buy), Ok(()));
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: buy,
    }]);
    world.transform.get_mut(caster).unwrap().pos = ORIGIN;
    world.mana.get_mut(caster).unwrap().mana = Fixed::ZERO;
    let use_mango = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    assert_eq!(world.validate_order(SlotId(0), None, &use_mango), Ok(()));
    let consumed = world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: use_mango,
    }]);
    assert_eq!(consumed.len(), 1);
    assert_eq!(
        consumed[0].kind,
        EventKind::Healed {
            source: Some(crate::game::wire_id(caster)),
            target: crate::game::wire_id(caster),
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
    assert_eq!(world.validate_order(SlotId(0), None, &cast), Ok(()));
    let events = world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: cast,
    }]);
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
    let mut seat = Seat::new(SlotId(0), Team::Radiant, HeroId(2), 0, rules::STASH_SLOTS);
    seat.unit = Some(caster);
    world.seats.push(seat);
    for (slot, expected) in [(0, 90), (1, 140), (2, 190)] {
        reveal(&mut world, target);
        let order = Order::Cast {
            slot: AbilitySlot(slot),
            target: Target::None,
        };
        assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
        let events = world.advance(&[Command {
            slot: SlotId(0),
            unit: None,
            order,
        }]);
        assert_eq!(damage(&events, target), vec![expected]);
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
