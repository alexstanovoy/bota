use bota_core::{item_def, rules};
use bota_proto::{Aim, Fixed, Target};

use crate::game::{BAG_SLOTS, carried_bonus, item_views, regenerate};

use super::fixtures::*;

#[test]
fn catalog_appends_numeric_mango_with_one_charge_and_self_aim() {
    assert_eq!(bota_core::ITEMS.len(), 54);
    let def = item_def(MANGO).expect("Mango must exist at item id 42");
    assert_eq!(def.cost, 65);
    assert_eq!(def.charges, 1);
    assert_eq!(def.cast_charges, 0);
    assert_eq!(def.cooldown, 0);
    assert_eq!(def.mana_cost, 0);
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let view = item_views(world.inventory.get(hero).unwrap(), rules::NOMINAL_BP, &[])[0].unwrap();
    assert_eq!(view.charges, Some(3));
    assert_eq!(view.aim, Some(Aim::Own));
    assert_eq!(view.range, 0);
    assert_eq!(bota_core::shop_entries()[42].id, MANGO);
}

#[test]
fn use_restores_exactly_one_hundred_mana_and_spends_only_one_charge() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let health = world.health.get(hero).unwrap().hp;
    let tick = world.tick;
    for left in (0..3).rev() {
        assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
        assert_eq!(
            world.mana.get(hero).unwrap().mana,
            Fixed::from_int((3 - left) * 100)
        );
        assert_eq!(
            world.inventory.get(hero).unwrap().slots[0].map(|s| s.charges),
            (left > 0).then_some(left as u8)
        );
    }
    assert_eq!(world.tick, tick);
    assert_eq!(world.health.get(hero).unwrap().hp, health);
    assert!(
        world
            .modifiers
            .get(hero)
            .is_none_or(|on_it| on_it.0.is_empty())
    );
    assert!(world.seats[0].item_clocks.is_empty());
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn near_fixed_maximum_restoration_does_not_overflow() {
    let (mut world, hero) = fixture();
    world.stats.get_mut(hero).unwrap().max_mana = Fixed::MAX;
    world.mana.get_mut(hero).unwrap().mana = Fixed::MAX - Fixed::EPSILON;
    put(&mut world, hero, 0, mango(1));
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    assert_eq!(world.mana.get(hero).unwrap().mana, Fixed::MAX);
    assert!(world.inventory.get(hero).unwrap().slots[0].is_none());
}

#[test]
fn dead_and_despawned_users_reject_without_spending() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.health.get_mut(hero).unwrap().hp = Fixed::ZERO;
    reject_use(&mut world, hero, 0, Target::None);
    world.health.get_mut(hero).unwrap().hp = Fixed::ONE;
    assert!(world.despawn(hero));
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn backpack_stash_and_out_of_bounds_slots_reject() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, rules::INVENTORY_SLOTS, mango(3));
    world.seats[0].stash.slots[0] = Some(mango(3));
    for at in [rules::INVENTORY_SLOTS, BAG_SLOTS, usize::MAX] {
        reject_use(&mut world, hero, at, Target::None);
    }
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
}

#[test]
fn mute_cooldown_and_zero_charges_reject() {
    let (mut world, hero) = fixture();
    for (mute, cooldown, charges) in [(1, 0, 3), (0, 1, 3), (0, 0, 0)] {
        let mut stack = mango(3);
        stack.mute = mute;
        stack.cooldown = cooldown;
        stack.charges = charges;
        world.inventory.get_mut(hero).unwrap().slots[0] = Some(stack);
        reject_use(&mut world, hero, 0, Target::None);
    }
}

#[test]
fn separate_stacks_add_the_same_regen_as_combined_charges() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    put(&mut world, hero, 1, mango(1));
    let before = carried_bonus(world.inventory.get(hero).unwrap()).hp_regen;
    assert_eq!(before.raw, 2619);
    assert!(world.move_item(OWNER, hero, 1, 0));
    assert_eq!(
        carried_bonus(world.inventory.get(hero).unwrap()).hp_regen,
        before
    );
}

#[test]
fn regen_decreases_per_use_and_disappears_after_last_charge() {
    let (mut world, hero) = fixture();
    let base = world.stats.get(hero).unwrap().hp_regen;
    put(&mut world, hero, 0, mango(3));
    for left in (0..=3).rev() {
        world.settle();
        assert_eq!(
            world.stats.get(hero).unwrap().hp_regen.raw - base.raw,
            873 * left
        );
        if left > 0 {
            world.mana.get_mut(hero).unwrap().mana = Fixed::ZERO;
            assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
        }
    }
}

#[test]
fn regen_accumulates_exact_raw_health_for_thirty_ticks() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.settle();
    let base = crate::game::carried_bonus(world.inventory.get(hero).unwrap()).hp_regen;
    assert_eq!(base.raw, 2619);
    world.stats.get_mut(hero).unwrap().hp_regen = base;
    world.health.get_mut(hero).unwrap().hp = Fixed::ONE;
    for _ in 0..30 {
        regenerate(
            &world.entities,
            &world.stats,
            &mut world.health,
            &mut world.mana,
        );
    }
    assert_eq!(
        world.health.get(hero).unwrap().hp.raw,
        Fixed::ONE.raw + 78570
    );
}

#[test]
fn stash_backpack_and_muted_stacks_add_no_passive() {
    let (mut world, hero) = fixture();
    let base = world.stats.get(hero).unwrap().hp_regen;
    let mut muted = mango(3);
    muted.mute = 1;
    put(&mut world, hero, 0, muted);
    put(&mut world, hero, rules::INVENTORY_SLOTS, mango(3));
    world.seats[0].stash.slots[0] = Some(mango(3));
    world.settle();
    assert_eq!(world.stats.get(hero).unwrap().hp_regen, base);
    assert_eq!(
        carried_bonus(world.inventory.get(hero).unwrap()).hp_regen,
        Fixed::ZERO
    );
}

#[test]
fn stick_still_spends_all_charges_and_keeps_its_slot() {
    let (mut world, hero) = fixture();
    let mut stick = mango(3);
    stick.id = bota_proto::ItemId::MAGIC_STICK;
    put(&mut world, hero, 0, stick);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    assert_eq!(world.mana.get(hero).unwrap().mana, Fixed::from_int(45));
    assert_eq!(held(&world, hero, 0).charges, 0);
    assert_eq!(held(&world, hero, 0).cooldown, 390);
}
