use bota_proto::{Fixed, SlotId, Target};

use crate::game::{BAG_SLOTS, carried_bonus, rules};

use super::fixtures::*;

#[test]
fn explicit_move_merges_into_stack_and_clears_source() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    put(&mut world, hero, 1, mango(1));
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert!(held(&world, hero, 0).touched);
    assert!(world.inventory.get(hero).unwrap().slots[1].is_none());
}

#[test]
fn explicit_move_caps_destination_and_retains_source_remainder() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    put(&mut world, hero, 1, mango(3));
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(held(&world, hero, 1).charges, 2);
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 5);
    assert!(held(&world, hero, 1).touched);
}

#[test]
fn every_merge_charge_pair_preserves_cap_count_and_source_metadata() {
    let (mut world, hero) = fixture();
    for target_charges in 1..3 {
        for source_charges in 1..=3 {
            let mut source = mango(source_charges);
            source.bought_tick = 20;
            source.cooldown = 11;
            let mut target = mango(target_charges);
            target.bought_tick = 10;
            target.mute = 7;
            world.inventory.get_mut(hero).unwrap().slots[0] = Some(target);
            world.inventory.get_mut(hero).unwrap().slots[1] = Some(source);
            assert!(world.move_item(OWNER, hero, 1, 0));
            let total = target_charges + source_charges;
            target.charges = total.min(3);
            target.cooldown = 11;
            target.touched = true;
            source.charges = total.saturating_sub(3);
            source.touched = true;
            assert_eq!(held(&world, hero, 0), target);
            assert_eq!(
                world.inventory.get(hero).unwrap().slots[1],
                (source.charges > 0).then_some(source)
            );
            assert_eq!(
                charges_in(world.inventory.get(hero).unwrap()),
                u32::from(total)
            );
        }
    }
}

#[test]
fn full_merge_target_keeps_existing_swap_behavior_and_both_stacks() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    put(&mut world, hero, 1, mango(1));
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert_eq!(held(&world, hero, 0).charges, 1);
    assert_eq!(held(&world, hero, 1).charges, 3);
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 4);
}

#[test]
fn merge_keeps_oldest_purchase_and_strongest_mute_and_cooldown() {
    let (mut world, hero) = fixture();
    let mut source = mango(1);
    source.bought_tick = 10;
    source.mute = 8;
    source.cooldown = 11;
    let mut target = mango(1);
    target.bought_tick = 20;
    target.mute = 12;
    target.cooldown = 7;
    put(&mut world, hero, 0, target);
    put(&mut world, hero, 1, source);
    assert!(world.move_item(OWNER, hero, 1, 0));
    target.charges = 2;
    target.bought_tick = 10;
    target.cooldown = 11;
    target.touched = true;
    assert_eq!(held(&world, hero, 0), target);
    assert!(world.inventory.get(hero).unwrap().slots[1].is_none());
}

#[test]
fn different_owners_swap_intact_instead_of_merging() {
    let (mut world, hero) = fixture();
    let mut foreign = mango(2);
    foreign.owner = SlotId(1);
    put(&mut world, hero, 0, mango(1));
    put(&mut world, hero, 1, foreign);
    assert!(world.move_item(OWNER, hero, 1, 0));
    foreign.touched = true;
    assert_eq!(held(&world, hero, 0), foreign);
    assert_eq!(held(&world, hero, 1).owner, OWNER);
    assert_eq!(held(&world, hero, 1).charges, 1);
}

#[test]
fn different_sale_marks_swap_without_clearing_or_propagating_marks() {
    let (mut world, hero) = fixture();
    let mut marked = mango(2);
    marked.for_sale = true;
    put(&mut world, hero, 0, mango(1));
    put(&mut world, hero, 1, marked);
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert!(held(&world, hero, 0).for_sale);
    assert!(!held(&world, hero, 1).for_sale);
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 3);
}

#[test]
fn different_modes_swap_intact_instead_of_merging() {
    let (mut world, hero) = fixture();
    let mut different = mango(2);
    different.mode = Some(bota_proto::Attribute::Strength);
    put(&mut world, hero, 0, mango(1));
    put(&mut world, hero, 1, different);
    assert!(world.move_item(OWNER, hero, 1, 0));
    different.touched = true;
    assert_eq!(held(&world, hero, 0), different);
    assert_eq!(held(&world, hero, 1).mode, None);
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 3);
}

#[test]
fn matching_sale_marks_merge_and_remain_marked() {
    let (mut world, hero) = fixture();
    let mut marked = mango(1);
    marked.for_sale = true;
    put(&mut world, hero, 0, marked);
    put(&mut world, hero, 1, marked);
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert_eq!(held(&world, hero, 0).charges, 2);
    assert!(held(&world, hero, 0).for_sale);
    assert!(world.inventory.get(hero).unwrap().slots[1].is_none());
}

#[test]
fn backpack_merge_mutes_whole_destination_until_exact_boundary() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(1));
    put(&mut world, hero, rules::INVENTORY_SLOTS, mango(2));
    assert!(world.move_item(OWNER, hero, rules::INVENTORY_SLOTS, 0));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(held(&world, hero, 0).mute, rules::BACKPACK_MUTE_TICKS);
    for _ in 0..rules::BACKPACK_MUTE_TICKS - 1 {
        world.tick_gear();
    }
    assert_eq!(held(&world, hero, 0).mute, 1);
    assert_eq!(
        carried_bonus(world.inventory.get(hero).unwrap()).hp_regen,
        Fixed::ZERO
    );
    reject_use(&mut world, hero, 0, Target::None);
    world.tick_gear();
    assert_eq!(held(&world, hero, 0).mute, 0);
    assert_eq!(
        carried_bonus(world.inventory.get(hero).unwrap())
            .hp_regen
            .raw,
        2619
    );
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    assert_eq!(held(&world, hero, 0).charges, 2);
}

#[test]
fn partial_backpack_merge_does_not_mute_unmoved_remainder() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    put(&mut world, hero, rules::INVENTORY_SLOTS, mango(3));
    assert!(world.move_item(OWNER, hero, rules::INVENTORY_SLOTS, 0));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(held(&world, hero, 0).mute, rules::BACKPACK_MUTE_TICKS);
    assert_eq!(held(&world, hero, rules::INVENTORY_SLOTS).charges, 2);
    assert_eq!(held(&world, hero, rules::INVENTORY_SLOTS).mute, 0);
}

#[test]
fn stash_moves_require_shop_and_conserve_charges_both_ways() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    world.seats[0].stash.slots[0] = Some(mango(1));
    let home = world.transform.get(hero).unwrap().pos;
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    let before = world.hash();
    assert!(!world.move_item(OWNER, hero, BAG_SLOTS, 0));
    assert_eq!(world.hash(), before);
    world.transform.get_mut(hero).unwrap().pos = home;
    assert!(world.move_item(OWNER, hero, BAG_SLOTS, 0));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert!(world.seats[0].stash.slots[0].is_none());
    assert!(world.move_item(OWNER, hero, 0, BAG_SLOTS));
    assert_eq!(world.seats[0].stash.slots[0].unwrap().charges, 3);
    assert!(world.inventory.get(hero).unwrap().slots[0].is_none());
}

#[test]
fn existing_mute_survives_stash_round_trip_and_merge() {
    let (mut world, hero) = fixture();
    let mut stack = mango(2);
    stack.mute = rules::BACKPACK_MUTE_TICKS;
    put(&mut world, hero, 0, stack);
    put(&mut world, hero, 1, mango(1));
    assert!(world.move_item(OWNER, hero, 0, BAG_SLOTS));
    assert!(world.move_item(OWNER, hero, BAG_SLOTS, 1));
    assert_eq!(held(&world, hero, 1).charges, 3);
    assert_eq!(held(&world, hero, 1).mute, rules::BACKPACK_MUTE_TICKS);
    reject_use(&mut world, hero, 1, Target::None);
}

#[test]
fn invalid_courier_bag_destination_cannot_destroy_stash_stack() {
    let (mut world, _) = fixture();
    world.stand_up_courier(0);
    let courier = world.seats[0].courier.unwrap();
    world.seats[0].stash.slots[0] = Some(mango(3));
    let before = world.hash();
    assert!(!world.move_item(OWNER, courier, BAG_SLOTS, rules::INVENTORY_SLOTS));
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(world.hash(), before);
}

#[test]
fn charges_change_hash_and_repeated_use_is_deterministic() {
    let (mut first, hero) = fixture();
    let (mut second, other) = fixture();
    put(&mut first, hero, 0, mango(3));
    put(&mut second, other, 0, mango(2));
    assert_ne!(first.hash(), second.hash());
    second.inventory.get_mut(other).unwrap().slots[0] = Some(mango(3));
    assert_eq!(first.hash(), second.hash());
    for _ in 0..3 {
        assert!(first.use_item(hero, 0, Target::None, &mut Vec::new()));
        assert!(second.use_item(other, 0, Target::None, &mut Vec::new()));
        assert_eq!(first.hash(), second.hash());
    }
}
