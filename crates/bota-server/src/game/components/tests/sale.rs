use bota_proto::{ItemId, SlotId, Target};

use crate::game::{BAG_SLOTS, rules};

use super::fixtures::*;

#[test]
fn fresh_stack_refunds_sixty_five_per_remaining_charge() {
    let (mut world, hero) = fixture();
    for charges in 1..=3 {
        put(&mut world, hero, 0, mango(charges));
        let before = world.seats[0].gold;
        assert!(world.sell_item(OWNER, hero, 0));
        assert_eq!(world.seats[0].gold - before, 65 * i32::from(charges));
        assert!(world.inventory.get(hero).unwrap().slots[0].is_none());
    }
}

#[test]
fn touched_stack_sells_half_total_remaining_value_rounded_down() {
    let (mut world, hero) = fixture();
    for (charges, price) in [(1, 32), (2, 65), (3, 97)] {
        let mut stack = mango(charges);
        stack.touched = true;
        put(&mut world, hero, 0, stack);
        let before = world.seats[0].gold;
        assert!(world.sell_item(OWNER, hero, 0));
        assert_eq!(world.seats[0].gold - before, price);
    }
}

#[test]
fn consumed_charge_is_never_refunded() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    let before = world.seats[0].gold;
    assert!(world.sell_item(OWNER, hero, 0));
    assert_eq!(world.seats[0].gold - before, 65);
}

#[test]
fn refund_window_is_inclusive_and_late_sales_pay_half() {
    let (mut world, hero) = fixture();
    for (tick, price) in [
        (rules::SELL_REFUND_TICKS, 195),
        (rules::SELL_REFUND_TICKS + 1, 97),
    ] {
        world.tick = tick;
        put(&mut world, hero, 0, mango(3));
        let before = world.seats[0].gold;
        assert!(world.sell_item(OWNER, hero, 0));
        assert_eq!(world.seats[0].gold - before, price);
    }
}

#[test]
fn buying_into_old_stack_does_not_refresh_refund_window() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(1));
    world.tick = rules::SELL_REFUND_TICKS + 1;
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(held(&world, hero, 0).charges, 2);
    assert_eq!(held(&world, hero, 0).bought_tick, 0);
    let before = world.seats[0].gold;
    assert!(world.sell_item(OWNER, hero, 0));
    assert_eq!(world.seats[0].gold - before, 65);
}

#[test]
fn stash_sells_remaining_charges_from_away() {
    let (mut world, hero) = fixture();
    world.seats[0].stash.slots[0] = Some(mango(2));
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    let before = world.seats[0].gold;
    assert!(world.sell_item(OWNER, hero, BAG_SLOTS));
    assert_eq!(world.seats[0].gold - before, 130);
    assert!(world.seats[0].stash.slots[0].is_none());
}

#[test]
fn non_owner_cannot_sell_or_mark_mango() {
    let (mut world, hero) = fixture();
    let mut stack = mango(3);
    stack.owner = SlotId(1);
    put(&mut world, hero, 0, stack);
    let gold = world.seats[0].gold;
    assert!(!world.sell_item(OWNER, hero, 0));
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    assert!(!world.sell_item(OWNER, hero, 0));
    assert_eq!(held(&world, hero, 0), stack);
    assert_eq!(world.seats[0].gold, gold);
}

#[test]
fn courier_collects_marked_remaining_charges_and_settles_once() {
    let (mut world, hero) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    let home = world.transform.get(courier).unwrap().pos;
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    world.transform.get_mut(courier).unwrap().pos = AWAY;
    let mut stack = mango(2);
    stack.touched = true;
    put(&mut world, hero, 0, stack);
    let gold = world.seats[0].gold;
    assert!(world.sell_item(OWNER, hero, 0));
    assert_eq!(world.seats[0].gold, gold);
    assert!(world.courier_deliver(courier));
    world.tick_couriers();
    assert_eq!(held(&world, courier, 0).charges, 2);
    assert!(held(&world, courier, 0).for_sale);
    world.transform.get_mut(courier).unwrap().pos = home;
    world.settle_sales();
    assert_eq!(world.seats[0].gold - gold, 65);
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
    world.settle_sales();
    assert_eq!(world.seats[0].gold - gold, 65);
}

#[test]
fn courier_sale_pays_item_owner_not_courier_owner() {
    let (mut world, _) = fixture();
    let other = SlotId(1);
    world.seats.push(crate::game::Seat::new(
        other,
        bota_proto::Team::Radiant,
        bota_proto::HeroId(2),
        200,
        rules::STASH_SLOTS,
    ));
    world.stand_up_courier(0);
    let courier = world.seats[0].courier.unwrap();
    let mut stack = mango(2);
    stack.owner = other;
    stack.for_sale = true;
    stack.touched = true;
    put(&mut world, courier, 0, stack);
    let holder_gold = world.seats[0].gold;
    world.settle_sales();
    assert_eq!(world.seats[1].gold, 265);
    assert_eq!(world.seats[0].gold, holder_gold);
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
}

#[test]
fn tango_sale_value_is_not_multiplied_by_charges() {
    let (mut world, hero) = fixture();
    let mut tango = mango(3);
    tango.id = ItemId(crate::game::ITEM_TANGO);
    put(&mut world, hero, 0, tango);
    let before = world.seats[0].gold;
    assert!(world.sell_item(OWNER, hero, 0));
    assert_eq!(world.seats[0].gold - before, 90);
}
