use bota_core::ModifierKind;

use crate::game::{HitEffect, Inventory, ItemStack, wire_id};
use bota_proto::{Attribute, ItemId, SlotId, Team};

use super::fixtures::*;

#[test]
fn mechanics_hash_distinguishes_a_pending_raze_from_a_plain_hit_and_other_stack_bonuses() {
    for (effect, case) in [
        (HitEffect::None, "an ordinary magical hit"),
        (
            HitEffect::Shadowraze { stack_damage: 50 },
            "another stack bonus",
        ),
    ] {
        let (mut world, caster, _) = fixture();
        assert!(world.cast_raze(caster, 0, 1));
        let before = world.hash();
        world.hits.front_mut().unwrap().effect = effect;
        assert_ne!(world.hash(), before, "{case}");
    }
}

#[test]
fn mechanics_hash_distinguishes_stack_sources_but_public_effects_do_not() {
    let (mut world, caster, target) = fixture();
    let other = hero(&mut world, Team::Radiant, ORIGIN, SlotId(2));
    land(&mut world, caster, target, 0);
    let before = world.hash();
    let public = effects(&world, target);
    let held = &mut world.modifiers.get_mut(target).unwrap().0[0];
    held.kind = ModifierKind::Shadowraze { stacks: 1 };
    held.source = Some(other);
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
    for (from, stacks, ticks) in [(returned, 1, 180), (returned, 2, 180), (returned, 2, 179)] {
        let held = &mut world.modifiers.get_mut(target).unwrap().0[0];
        held.kind = ModifierKind::Shadowraze { stacks };
        held.source = Some(from);
        held.ticks_left = Some(ticks);
        let after = world.hash();
        assert_ne!(after, before);
        before = after;
    }
}

/// Where a stack may be kept: in the caster's bag, in the courier's kept bag
/// or on the ground.
#[derive(Clone, Copy, Debug)]
enum Kept {
    Bag,
    Courier,
    Ground,
}

/// The hash of a world keeping `stack` at `kept`.
fn hash_keeping(kept: Kept, stack: ItemStack) -> u64 {
    let (mut world, caster, _) = fixture();
    match kept {
        Kept::Bag => world.inventory.get_mut(caster).unwrap().slots[0] = Some(stack),
        Kept::Courier => {
            seat(&mut world, SlotId(0), Team::Radiant, caster, 0);
            let mut bag = Inventory::empty(6);
            bag.slots[0] = Some(stack);
            world.seats[0].courier_kept = Some(bag);
        }
        Kept::Ground => {
            world.lay_loot(stack, ORIGIN);
        }
    }
    world.hash()
}

/// One named change to an item stack.
type StackChange = (&'static str, fn(&mut ItemStack));

#[test]
fn mechanics_hash_includes_every_merge_field_of_a_stack_wherever_it_is_kept() {
    let changes: [StackChange; 4] = [
        ("charges", |stack| stack.charges = 2),
        ("owner", |stack| stack.owner = SlotId(1)),
        ("sale mark", |stack| stack.for_sale = true),
        ("mode", |stack| stack.mode = Some(Attribute::Agility)),
    ];
    for kept in [Kept::Bag, Kept::Courier, Kept::Ground] {
        for item in [ItemId::MANGO, ItemId::POWER_TREADS] {
            let base = ItemStack::bought(item, SlotId(0), 0).unwrap();
            for (name, change) in changes {
                let mut changed = base;
                change(&mut changed);
                assert_ne!(
                    hash_keeping(kept, changed),
                    hash_keeping(kept, base),
                    "{name} of item {} kept in {kept:?}",
                    item.0
                );
            }
        }
    }
}

#[test]
fn a_dropped_mango_is_picked_up_with_its_charges_and_ownership() {
    let (mut world, caster, _) = fixture();
    let mut stack = ItemStack::bought(ItemId::MANGO, SlotId(0), 0).unwrap();
    (stack.charges, stack.owner, stack.for_sale) = (3, SlotId(1), true);
    let loot = world.lay_loot(stack, ORIGIN);
    let laid = world.loot.get(loot).unwrap().0;
    assert_eq!(
        laid,
        ItemStack {
            touched: true,
            ..stack
        }
    );
    reveal(&mut world, loot);
    assert!(world.take_item(caster, wire_id(loot)));
    world.tick_handling();
    assert_eq!(world.inventory.get(caster).unwrap().slots[0], Some(laid));
    assert!(!world.entities.contains(loot));
}
