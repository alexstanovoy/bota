use bota_proto::{Fixed, Target};

use crate::game::{BAG_SLOTS, Errand, rules};

use super::fixtures::*;

#[test]
fn courier_fetch_and_delivery_preserve_whole_stack_metadata() {
    let (mut world, hero) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    let mut stack = mango(3);
    stack.touched = true;
    stack.mute = 12;
    stack.cooldown = 7;
    world.seats[0].stash.slots[0] = Some(stack);
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    assert!(world.courier_take_stash(courier));
    world.tick_couriers();
    assert_eq!(held(&world, courier, 0), stack);
    assert!(world.seats[0].stash.slots[0].is_none());
    world.transform.get_mut(courier).unwrap().pos = AWAY;
    world.tick_couriers();
    assert_eq!(held(&world, hero, 0), stack);
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
    assert_eq!(world.errand.get(courier), Some(&Errand::GoingHome));
}

#[test]
fn courier_return_keeps_charges_owner_and_sale_mark() {
    let (mut world, _) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    let mut stack = mango(2);
    stack.touched = true;
    stack.for_sale = true;
    put(&mut world, courier, 0, stack);
    assert!(world.courier_return_items(courier));
    world.tick_couriers();
    assert_eq!(world.seats[0].stash.slots[0], Some(stack));
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
}

#[test]
fn courier_has_no_mango_regen_and_cannot_use_without_mana_pool() {
    let (mut world, hero) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    let baseline = world.stats.get(courier).copied().unwrap();
    put(&mut world, courier, 0, mango(3));
    world.settle();
    assert_eq!(world.stats.get(courier), Some(&baseline));
    assert!(world.mana.get(courier).is_none());
    reject_use(&mut world, courier, 0, Target::None);
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 0);
}

#[test]
fn full_hero_returns_intact_stack_instead_of_losing_charges() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.seats[0].stash.slots[0] = None;
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    put(&mut world, courier, 0, mango(3));
    assert!(world.courier_deliver(courier));
    world.tick_couriers();
    assert_eq!(held(&world, courier, 0), mango(3));
    assert_eq!(world.errand.get(courier), Some(&Errand::PutBack));
    world.tick_couriers();
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
}

#[test]
fn courier_delivery_to_backpack_adds_no_hero_regen() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[rules::INVENTORY_SLOTS] = None;
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    let baseline = world.stats.get(hero).unwrap().hp_regen;
    put(&mut world, courier, 0, mango(3));
    assert!(world.courier_deliver(courier));
    world.tick_couriers();
    world.settle();
    assert_eq!(held(&world, hero, rules::INVENTORY_SLOTS).charges, 3);
    assert_eq!(world.stats.get(hero).unwrap().hp_regen, baseline);
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
}

#[test]
fn dead_owner_delivery_returns_all_charges_to_stash() {
    let (mut world, hero) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    put(&mut world, courier, 0, mango(3));
    world.health.get_mut(hero).unwrap().hp = Fixed::ZERO;
    assert!(world.courier_deliver(courier));
    world.tick_couriers();
    assert_eq!(world.errand.get(courier), Some(&Errand::PutBack));
    world.tick_couriers();
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(charges_in(world.inventory.get(courier).unwrap()), 0);
}

#[test]
fn explicit_courier_stash_merge_conserves_charges() {
    let (mut world, _) = fixture();
    world.stand_up_courier(0);
    let courier = world.seats[0].courier.unwrap();
    put(&mut world, courier, 0, mango(1));
    world.seats[0].stash.slots[0] = Some(mango(2));
    assert!(world.move_item(OWNER, courier, BAG_SLOTS, 0));
    assert_eq!(held(&world, courier, 0).charges, 3);
    assert!(world.seats[0].stash.slots[0].is_none());
}
