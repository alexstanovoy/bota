//! Wards bought, carried, planted and handed on: they stack, and an Observer
//! Ward and a Sentry Ward held by one seat share the one slot of a Ward
//! Dispenser.

use crate::game::{BAG_SLOTS, Entity, ItemStack, World, wire_id};
use bota_core::rules;
use bota_proto::{
    DispenserView, ItemId, ItemSlot, Order, RejectReason, SlotId, Target, Team, UnitKind, Vec2,
};

use super::support::*;

fn buy(world: &mut World, item: ItemId) -> bool {
    world.buy(SlotId(0), item, &mut Vec::new())
}

fn held(world: &World, hero: Entity) -> Vec<ItemStack> {
    world
        .inventory
        .get(hero)
        .expect("bag")
        .held()
        .copied()
        .collect()
}

fn dispenser_view(world: &World, hero: Entity, at: usize) -> Option<DispenserView> {
    let stack = slot_of(world, hero, at)?;
    crate::game::item_view(&stack, rules::NOMINAL_BP, &[]).dispenser
}

fn standing(world: &World, kind: UnitKind) -> usize {
    world
        .entities
        .iter()
        .filter(|entity| world.kind.get(*entity) == Some(&kind))
        .count()
}

#[test]
fn several_of_a_ward_bought_are_one_stack() {
    let (mut world, hero) = match_hero();
    for _ in 0..2 {
        assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    }
    let wards = held(&world, hero);
    assert_eq!(wards.len(), 1, "two observers take one slot");
    assert_eq!(
        (wards[0].id, wards[0].charges),
        (ItemId::OBSERVER_WARD, 2),
        "with a charge to each"
    );
    assert!(
        !buy(&mut world, ItemId::OBSERVER_WARD),
        "and the shop's stock of two is spent"
    );
    let gold = world.seats[0].gold;
    for _ in 0..3 {
        assert!(buy(&mut world, ItemId::SENTRY_WARD));
    }
    assert_eq!(
        world.seats[0].gold,
        gold - 3 * price_of(ItemId::SENTRY_WARD)
    );
}

#[test]
fn both_kinds_bought_share_one_slot_with_a_count_each() {
    let (mut world, hero) = match_hero();
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    let wards = held(&world, hero);
    assert_eq!(wards.len(), 1, "one slot holds the lot");
    assert_eq!(wards[0].id, ItemId::WARD_DISPENSER);
    let view = crate::game::item_view(&wards[0], rules::NOMINAL_BP, &[]);
    assert_eq!(
        view.dispenser,
        Some(DispenserView {
            observers: 2,
            sentries: 2,
            sentry_active: false
        }),
        "it starts on the Observer Ward"
    );
    assert_eq!(view.charges, Some(2), "and shows the active kind's count");
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert_eq!(dispenser_view(&world, hero, 0).map(|d| d.sentries), Some(3));
    assert_eq!(held(&world, hero).len(), 1);
}

#[test]
fn a_full_bag_still_takes_a_ward_into_its_dispenser() {
    let (mut world, hero) = match_hero();
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    let bag = world.inventory.get_mut(hero).expect("bag");
    for slot in bag.slots.iter_mut().skip(1) {
        *slot = Some(a_stack_of(ItemId::BOOTS, 0));
    }
    assert!(
        buy(&mut world, ItemId::SENTRY_WARD),
        "it fits the dispenser"
    );
    assert_eq!(dispenser_view(&world, hero, 0).map(|d| d.sentries), Some(2));
    assert!(
        world.seats[0].stash.slots.iter().all(Option::is_none),
        "nothing went to the stash"
    );
}

#[test]
fn a_switch_picks_the_kind_a_use_plants_and_a_spent_kind_leaves_the_other_plain() {
    let (mut world, hero) = match_hero();
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    world.step();
    seat_order(&mut world, Order::SwitchWard { slot: ItemSlot(0) });
    let view = dispenser_view(&world, hero, 0).expect("a dispenser");
    assert!(view.sentry_active, "the double tap switched it");
    let stack = slot_of(&world, hero, 0).expect("held");
    assert_eq!(stack.cooldown, 0, "and costs no wait");
    let at = spot + Vec2::from_ints(200, 0);
    assert!(world.use_item(hero, 0, Target::Pos(at), &mut Vec::new()));
    assert_eq!(standing(&world, UnitKind::Ward), 1);
    let ward = the_ward(&world);
    assert_eq!(
        world.def.get(ward).map(|def| def.0.true_sight),
        Some(bota_core::SENTRY_WARD.true_sight),
        "a Sentry Ward stood"
    );
    let left = slot_of(&world, hero, 0).expect("still held");
    assert_eq!(
        (left.id, left.charges, left.sentries),
        (ItemId::OBSERVER_WARD, 2, 0),
        "the last sentry gone, the observers stay as a plain stack"
    );
    assert!(left.cooldown > 0, "and the plant starts the wait");
}

#[test]
fn a_dispenser_that_is_not_one_cannot_be_switched_and_none_is_sold() {
    let (mut world, hero) = match_hero();
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert_eq!(
        world.check_order(
            SlotId(0),
            None,
            &Order::SwitchWard { slot: ItemSlot(0) },
            false
        ),
        Err(RejectReason::NotCastable)
    );
    assert_eq!(
        world.check_order(
            SlotId(0),
            None,
            &Order::SwitchWard { slot: ItemSlot(3) },
            false
        ),
        Err(RejectReason::EmptySlot)
    );
    assert_eq!(
        world.check_order(
            SlotId(0),
            None,
            &Order::Buy {
                item: ItemId::WARD_DISPENSER
            },
            false
        ),
        Err(RejectReason::UnknownItem)
    );
    assert!(!buy(&mut world, ItemId::WARD_DISPENSER));
    assert_eq!(held(&world, hero).len(), 1);
}

#[test]
fn a_dispenser_sells_for_its_sentries_alone() {
    let (mut world, hero) = match_hero();
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    let gold = world.seats[0].gold;
    assert!(world.sell_item(SlotId(0), hero, 0));
    assert_eq!(
        world.seats[0].gold - gold,
        2 * price_of(ItemId::SENTRY_WARD),
        "bought a moment ago, the sentries refund in full and the free observer nothing"
    );
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    run(&mut world, rules::SELL_REFUND_TICKS + 1);
    let gold = world.seats[0].gold;
    assert!(world.sell_item(SlotId(0), hero, 0));
    assert_eq!(
        world.seats[0].gold - gold,
        price_of(ItemId::SENTRY_WARD) * rules::SELL_PCT / 100,
        "later it pays the usual share"
    );
}

#[test]
fn away_from_the_shop_wards_wait_apart_in_the_stash_and_join_in_the_bag() {
    let (mut world, hero) = match_hero();
    let away = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = away;
    world.step();
    assert!(buy(&mut world, ItemId::OBSERVER_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    assert!(buy(&mut world, ItemId::SENTRY_WARD));
    let stash: Vec<ItemStack> = world.seats[0]
        .stash
        .slots
        .iter()
        .flatten()
        .copied()
        .collect();
    assert_eq!(
        stash.iter().map(|s| (s.id, s.charges)).collect::<Vec<_>>(),
        vec![(ItemId::OBSERVER_WARD, 1), (ItemId::SENTRY_WARD, 2)],
        "the stash builds nothing, so each kind stacks in a slot of its own"
    );
    // Fetched by the courier, they come together in the courier's hands.
    let courier = the_courier(&world);
    assert!(world.courier_take_stash(courier));
    world.step();
    let carried = held(&world, courier);
    assert_eq!(carried.len(), 1, "one slot of the courier's");
    assert_eq!(
        (carried[0].id, carried[0].charges, carried[0].sentries),
        (ItemId::WARD_DISPENSER, 1, 2)
    );
    assert!(world.seats[0].stash.slots.iter().all(Option::is_none));
}

#[test]
fn a_dispenser_goes_whole_to_an_ally_the_ground_and_the_stash() {
    let (mut world, hero) = match_hero();
    for item in [
        ItemId::OBSERVER_WARD,
        ItemId::SENTRY_WARD,
        ItemId::SENTRY_WARD,
    ] {
        assert!(buy(&mut world, item));
    }
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    let ally = seat_hero(&mut world, 1, Team::Radiant, spot + Vec2::from_ints(100, 0));
    world.settle();
    seat_order(&mut world, Order::SwitchWard { slot: ItemSlot(0) });
    assert!(world.put_item(hero, 0, Target::Unit(wire_id(ally))));
    run(&mut world, 5);
    let given = held(&world, ally);
    assert_eq!(given.len(), 1, "it arrived in one slot");
    assert_eq!(
        (
            given[0].id,
            given[0].charges,
            given[0].sentries,
            given[0].sentry_active
        ),
        (ItemId::WARD_DISPENSER, 1, 2, true),
        "with its counts and the kind it was switched to"
    );
    assert!(held(&world, hero).is_empty());
    // Dropped and taken up again it is the same.
    assert!(world.put_item(ally, 0, Target::None));
    run(&mut world, 5);
    assert!(held(&world, ally).is_empty(), "it lies on the ground");
    let lying = on_the_ground(&world)[0];
    assert!(world.take_item(ally, wire_id(lying)));
    run(&mut world, 5);
    let back = held(&world, ally);
    assert_eq!(
        (back[0].id, back[0].charges, back[0].sentries),
        (ItemId::WARD_DISPENSER, 1, 2)
    );
}

#[test]
fn a_dispenser_moves_to_the_stash_whole_and_wards_given_by_an_ally_do_not_join() {
    let (mut world, hero) = match_hero();
    for item in [ItemId::OBSERVER_WARD, ItemId::SENTRY_WARD] {
        assert!(buy(&mut world, item));
    }
    assert!(world.move_item(SlotId(0), hero, 0, BAG_SLOTS));
    let stashed = world.seats[0].stash.slots[0].expect("in the stash");
    assert_eq!(
        (stashed.id, stashed.charges, stashed.sentries),
        (ItemId::WARD_DISPENSER, 1, 1)
    );
    // Another seat's wards stay a stack of their own beside the dispenser.
    let mut theirs = a_stack_of(ItemId::OBSERVER_WARD, 1);
    theirs.charges = 1;
    assert!(world.inventory.get_mut(hero).expect("bag").receive(theirs));
    assert!(world.move_item(SlotId(0), hero, BAG_SLOTS, 1));
    world.step();
    assert_eq!(held(&world, hero).len(), 2);
}
