use bota_proto::{Attribute, HeroId, ItemId, SlotId, Team, WorldView};

use crate::game::{HitEffect, Inventory, ItemStack, Seat, StatusKind, rules};

use super::fixtures::*;

#[test]
fn mechanics_hash_distinguishes_pending_raze_from_an_ordinary_magical_hit() {
    let (mut world, caster, _) = fixture();
    assert!(world.cast_raze(caster, 0, 1));
    let before = world.hash();
    world.hits.front_mut().unwrap().effect = HitEffect::None;
    assert_ne!(world.hash(), before);
}

#[test]
fn mechanics_hash_distinguishes_pending_raze_levels_before_damage_is_resolved() {
    let (mut world, caster, _) = fixture();
    assert!(world.cast_raze(caster, 0, 1));
    let before = world.hash();
    world.hits.front_mut().unwrap().effect = HitEffect::Shadowraze { level: 1 };
    assert_ne!(world.hash(), before);
}

#[test]
fn mechanics_hash_distinguishes_stack_sources_but_public_effects_do_not() {
    let (mut world, caster, target) = fixture();
    let other = hero(&mut world, Team::Radiant, ORIGIN, SlotId(2));
    land(&mut world, caster, target, 0);
    let before = world.hash();
    let public = effects(&world, target);
    world.statuses.get_mut(target).unwrap().0[0].kind = StatusKind::Shadowraze {
        from: other,
        stacks: 1,
    };
    assert_ne!(world.hash(), before);
    assert_eq!(effects(&world, target), public);
}

#[test]
fn mechanics_hash_includes_caster_generation_stack_count_and_ticks() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    assert!(world.despawn(caster));
    let returned = hero(&mut world, Team::Radiant, ORIGIN, SlotId(0));
    assert_eq!(returned.index(), caster.index());
    let mut before = world.hash();
    for (from, stacks, ticks) in [(returned, 1, 240), (returned, 2, 240), (returned, 2, 239)] {
        let status = &mut world.statuses.get_mut(target).unwrap().0[0];
        status.kind = StatusKind::Shadowraze { from, stacks };
        status.ticks_left = ticks;
        let after = world.hash();
        assert_ne!(after, before);
        before = after;
    }
}

#[test]
fn mechanics_hash_includes_mango_merge_owner_and_sale_mark() {
    let (mut world, caster, _) = fixture();
    world.inventory.get_mut(caster).unwrap().slots[0] = ItemStack::bought(ItemId(42), SlotId(0), 0);
    let before = world.hash();
    world.inventory.get_mut(caster).unwrap().slots[0]
        .as_mut()
        .unwrap()
        .owner = SlotId(1);
    let changed_owner = world.hash();
    assert_ne!(changed_owner, before);
    world.inventory.get_mut(caster).unwrap().slots[0]
        .as_mut()
        .unwrap()
        .for_sale = true;
    assert_ne!(world.hash(), changed_owner);
}

#[test]
fn mechanics_hash_includes_item_mode_used_by_merge_compatibility() {
    let (mut world, caster, _) = fixture();
    world.inventory.get_mut(caster).unwrap().slots[0] = ItemStack::bought(ItemId(29), SlotId(0), 0);
    let before = world.hash();
    world.inventory.get_mut(caster).unwrap().slots[0]
        .as_mut()
        .unwrap()
        .mode = Some(Attribute::Agility);
    assert_ne!(world.hash(), before);
}

#[test]
fn mechanics_hash_includes_mango_charges_kept_while_courier_is_dead() {
    let (mut world, _, _) = fixture();
    let mut seat = Seat::new(SlotId(0), Team::Radiant, HeroId(2), 0, rules::STASH_SLOTS);
    let mut bag = Inventory::empty(6);
    bag.slots[0] = ItemStack::bought(ItemId(42), SlotId(0), 0);
    seat.courier_kept = Some(bag);
    world.seats.push(seat);
    let before = world.hash();
    world.seats[0].courier_kept.as_mut().unwrap().slots[0]
        .as_mut()
        .unwrap()
        .charges = 2;
    assert_ne!(world.hash(), before);
}

#[test]
fn mechanics_hash_keeps_mango_charges_and_ownership_observable_after_dropping() {
    let (mut world, caster, _) = fixture();
    let stack = ItemStack::bought(ItemId(42), SlotId(0), 0).unwrap();
    let loot = world.lay_loot(stack, ORIGIN);
    let before = world.hash();
    world.loot.get_mut(loot).unwrap().0.charges = 3;
    let changed_charges = world.hash();
    assert_ne!(changed_charges, before);
    world.loot.get_mut(loot).unwrap().0.owner = SlotId(1);
    let changed_owner = world.hash();
    assert_ne!(changed_owner, changed_charges);
    world.loot.get_mut(loot).unwrap().0.for_sale = true;
    assert_ne!(world.hash(), changed_owner);
    let expected = world.loot.get(loot).unwrap().0;
    reveal(&mut world, loot);
    assert!(world.take_item(caster, crate::game::wire_id(loot)));
    world.tick_handling();
    assert_eq!(
        world.inventory.get(caster).unwrap().slots[0],
        Some(expected)
    );
    assert!(!world.entities.contains(loot));
}

#[test]
fn raze_ticks_and_counts_round_trip_through_the_existing_snapshot_codec() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    land(&mut world, caster, target, 0);
    let view = world.view_full();
    let encoded = bota_proto::encode_frame_to_vec(&view).unwrap();
    let decoded: WorldView =
        bota_proto::decode_payload(&encoded[bota_proto::LEN_PREFIX..]).unwrap();
    assert_eq!(decoded, view);
    assert_eq!(effects(&world, target), vec![effect(2, 240)]);
}
