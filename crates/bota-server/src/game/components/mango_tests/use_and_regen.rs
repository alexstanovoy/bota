use bota_proto::{Aim, EntityId, Fixed, ItemSlot, Order, RejectReason, Target, Team};

use crate::game::{BAG_SLOTS, carried_bonus, item_def, item_views, regenerate, rules, wire_id};

use super::fixtures::*;

#[test]
fn catalog_appends_numeric_mango_with_one_charge_and_self_aim() {
    assert_eq!(crate::game::ITEMS.len(), 43);
    let def = item_def(MANGO).expect("Mango must exist at item id 42");
    assert_eq!(def.cost, 65);
    assert_eq!(def.charges, 1);
    assert_eq!(def.cast_charges, 0);
    assert_eq!(def.cooldown, 0);
    assert_eq!(def.mana_cost, 0);
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let view = item_views(world.inventory.get(hero).unwrap())[0].unwrap();
    assert_eq!(view.charges, Some(3));
    assert_eq!(view.aim, Some(Aim::Own));
    assert_eq!(view.range, 0);
    assert_eq!(crate::game::shop_entries().last().unwrap().id, MANGO);
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
    assert!(world.statuses.get(hero).is_none());
    assert!(world.seats[0].item_clocks.is_empty());
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn explicit_self_target_succeeds() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(2));
    assert!(world.use_item(hero, 0, Target::Unit(wire_id(hero)), &mut Vec::new()));
    assert_eq!(world.mana.get(hero).unwrap().mana, Fixed::from_int(100));
    assert_eq!(held(&world, hero, 0).charges, 1);
    assert!(held(&world, hero, 0).touched);
}

#[test]
fn every_positive_deficit_through_one_hundred_succeeds_and_clamps() {
    let (mut world, hero) = fixture();
    let maximum = world.stats.get(hero).unwrap().max_mana;
    for deficit in [
        Fixed::EPSILON,
        Fixed::from_ratio(1, 2),
        Fixed::ONE,
        Fixed::from_int(99),
        Fixed::from_int(100),
    ] {
        world.mana.get_mut(hero).unwrap().mana = maximum - deficit;
        put(&mut world, hero, 0, mango(1));
        assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
        assert_eq!(world.mana.get(hero).unwrap().mana, maximum);
        assert!(world.inventory.get(hero).unwrap().slots[0].is_none());
    }
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
fn full_and_overfull_mana_reject_without_mutation() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    for extra in [Fixed::ZERO, Fixed::EPSILON] {
        world.mana.get_mut(hero).unwrap().mana = world.stats.get(hero).unwrap().max_mana + extra;
        reject_use(&mut world, hero, 0, Target::None);
    }
}

#[test]
fn missing_mana_pool_zero_capacity_and_missing_stats_reject() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.stats.get_mut(hero).unwrap().max_mana = Fixed::ZERO;
    reject_use(&mut world, hero, 0, Target::None);
    world.stats.get_mut(hero).unwrap().max_mana = Fixed::from_int(500);
    let pool = world.mana.remove(hero).unwrap();
    reject_use(&mut world, hero, 0, Target::None);
    world.mana.insert(hero, pool);
    world.stats.remove(hero);
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn invalid_position_ally_enemy_and_stale_targets_reject() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let ally = world.spawn_unit(&crate::game::HERO, Team::Radiant, AWAY);
    let enemy = world.spawn_unit(&crate::game::HERO, Team::Dire, AWAY);
    for target in [
        Target::Pos(AWAY),
        Target::Unit(wire_id(ally)),
        Target::Unit(wire_id(enemy)),
        Target::Unit(EntityId {
            idx: u32::MAX,
            generation: u32::MAX,
        }),
    ] {
        reject_use(&mut world, hero, 0, target);
    }
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
fn order_errors_report_backpack_mute_and_death_exactly() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let order = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    world.inventory.get_mut(hero).unwrap().slots[0]
        .as_mut()
        .unwrap()
        .mute = 1;
    assert_eq!(
        world.validate_order(OWNER, None, &order),
        Err(RejectReason::NotReady)
    );
    let backpack = Order::Use {
        slot: ItemSlot(6),
        target: Target::None,
    };
    assert_eq!(
        world.validate_order(OWNER, None, &backpack),
        Err(RejectReason::WrongTargetKind)
    );
    world.health.get_mut(hero).unwrap().hp = Fixed::ZERO;
    assert_eq!(
        world.validate_order(OWNER, None, &order),
        Err(RejectReason::HeroDead)
    );
}

#[test]
fn regen_quantizes_each_charge_before_multiplication() {
    let (mut world, hero) = fixture();
    for charges in 1..=3 {
        world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(charges));
        let bonus = carried_bonus(world.inventory.get(hero).unwrap());
        assert_eq!(bonus.hp_regen.raw, 873 * i32::from(charges));
        assert_eq!(bonus.mana_regen, Fixed::ZERO);
    }
    assert_eq!(rules::TICKS_PER_SECOND, 30);
    assert_eq!(873 * rules::TICKS_PER_SECOND, 26190);
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
    stick.id = bota_proto::ItemId(crate::game::ITEM_MAGIC_STICK);
    put(&mut world, hero, 0, stick);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    assert_eq!(world.mana.get(hero).unwrap().mana, Fixed::from_int(45));
    assert_eq!(held(&world, hero, 0).charges, 0);
    assert_eq!(held(&world, hero, 0).cooldown, 390);
}
