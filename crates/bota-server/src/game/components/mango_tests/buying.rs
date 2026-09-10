use bota_proto::{EventKind, ItemId, SlotId};

use crate::game::{EventVisibility, rules};

use super::fixtures::*;

#[test]
fn sixty_four_gold_rejects_atomically() {
    let (mut world, hero) = fixture();
    world.seats[0].gold = 64;
    reject_buy(&mut world, hero);
}

#[test]
fn sixty_five_gold_buys_one_charge_and_emits_one_event() {
    let (mut world, hero) = fixture();
    world.seats[0].gold = 65;
    world.tick = 17;
    let mut events = Vec::new();
    assert!(world.buy(OWNER, MANGO, &mut events));
    let mut expected = mango(1);
    expected.bought_tick = 17;
    assert_eq!(held(&world, hero, 0), expected);
    assert_eq!(world.seats[0].gold, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].kind,
        EventKind::ItemBought {
            slot: OWNER,
            item: MANGO
        }
    );
    assert_eq!(
        events[0].visible_to,
        EventVisibility::OneTeam(bota_proto::Team::Radiant)
    );
}

#[test]
fn purchases_fill_three_charges_then_use_next_slot() {
    let (mut world, hero) = fixture();
    let mut events = Vec::new();
    for _ in 0..4 {
        assert!(world.buy(OWNER, MANGO, &mut events));
    }
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(held(&world, hero, 1).charges, 1);
    assert_eq!(world.inventory.get(hero).unwrap().held().count(), 2);
    assert_eq!(world.seats[0].gold, 740);
    assert_eq!(events.len(), 4);
}

#[test]
fn full_storage_can_buy_into_remaining_stack_room() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(2));
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(world.seats[0].gold, 935);
    reject_buy(&mut world, hero);
}

#[test]
fn full_storage_can_buy_into_backpack_stack_room() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[rules::INVENTORY_SLOTS] = Some(mango(2));
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(held(&world, hero, rules::INVENTORY_SLOTS).charges, 3);
    assert_eq!(world.seats[0].gold, 935);
}

#[test]
fn remote_purchase_merges_only_in_stash() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(1));
    world.seats[0].stash.slots[0] = Some(mango(2));
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(world.seats[0].stash.slots[0].unwrap().charges, 3);
    assert_eq!(held(&world, hero, 0).charges, 1);
    reject_buy(&mut world, hero);
}

#[test]
fn full_bag_falls_back_to_stash_stack_room_at_shop() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.seats[0].stash.slots[0] = Some(mango(2));
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(world.seats[0].gold, 935);
}

#[test]
fn fourth_remote_purchase_uses_next_stash_slot() {
    let (mut world, hero) = fixture();
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    for _ in 0..4 {
        assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    }
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(world.seats[0].stash.slots[1], Some(mango(1)));
    assert_eq!(charges_in(world.inventory.get(hero).unwrap()), 0);
    assert_eq!(world.seats[0].gold, 740);
}

#[test]
fn full_stack_without_free_slot_rejects_atomically() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(3));
    reject_buy(&mut world, hero);
}

#[test]
fn unaffordable_merge_keeps_gold_charges_metadata_and_events() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    world.seats[0].gold = 64;
    reject_buy(&mut world, hero);
}

#[test]
fn purchase_merge_preserves_old_touched_mute_and_cooldown() {
    let (mut world, hero) = fixture();
    let mut old = mango(1);
    old.touched = true;
    old.mute = 9;
    old.cooldown = 7;
    put(&mut world, hero, 0, old);
    world.tick = 50;
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    old.charges = 2;
    assert_eq!(held(&world, hero, 0), old);
    assert_eq!(world.inventory.get(hero).unwrap().held().count(), 1);
}

#[test]
fn purchases_do_not_merge_foreign_owned_or_marked_stacks() {
    let (mut world, hero) = fixture();
    let mut foreign = mango(1);
    foreign.owner = SlotId(1);
    let mut marked = mango(1);
    marked.for_sale = true;
    put(&mut world, hero, 0, foreign);
    put(&mut world, hero, 1, marked);
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(held(&world, hero, 0), foreign);
    assert_eq!(held(&world, hero, 1), marked);
    assert_eq!(held(&world, hero, 2), mango(1));
}

#[test]
fn incompatible_stack_room_does_not_bypass_full_storage() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    let mut foreign = mango(1);
    foreign.owner = SlotId(1);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(foreign);
    let mut marked = mango(1);
    marked.for_sale = true;
    world.seats[0].stash.slots[0] = Some(marked);
    reject_buy(&mut world, hero);
}

#[test]
fn non_mango_consumable_purchases_still_use_separate_slots() {
    let (mut world, hero) = fixture();
    let tango = ItemId(crate::game::ITEM_TANGO);
    for _ in 0..2 {
        assert!(world.buy(OWNER, tango, &mut Vec::new()));
    }
    assert_eq!(held(&world, hero, 0).charges, 3);
    assert_eq!(held(&world, hero, 1).charges, 3);
    assert_eq!(world.inventory.get(hero).unwrap().held().count(), 2);
}
