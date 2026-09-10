use bota_proto::{
    EntityId, Fixed, HeroId, ItemId, ItemSlot, Order, RejectReason, SlotId, Target, Team,
};

use crate::game::{BAG_SLOTS, Status, StatusKind, Statuses, rules, wire_id};

use super::fixtures::*;

const BUY_MANGO: Order = Order::Buy { item: MANGO };
const USE_MANGO: Order = Order::Use {
    slot: ItemSlot(0),
    target: Target::None,
};

#[test]
fn buy_order_accepts_full_storage_with_inventory_backpack_or_stash_stack_room() {
    for at in [0, rules::INVENTORY_SLOTS, BAG_SLOTS] {
        let (mut world, hero) = fixture();
        fill_storage(&mut world, hero);
        if at < BAG_SLOTS {
            world.inventory.get_mut(hero).unwrap().slots[at] = Some(mango(2));
        } else {
            world.seats[0].stash.slots[at - BAG_SLOTS] = Some(mango(2));
        }
        assert_eq!(world.validate_order(OWNER, None, &BUY_MANGO), Ok(()));
        assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
        assert_eq!(world.seats[0].gold, 935);
        assert_eq!(
            world.validate_order(OWNER, None, &BUY_MANGO),
            Err(RejectReason::InventoryFull)
        );
    }
}

#[test]
fn buy_order_rejects_truly_full_storage_as_inventory_full() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(3));
    assert_eq!(
        world.validate_order(OWNER, None, &BUY_MANGO),
        Err(RejectReason::InventoryFull)
    );
    reject_buy(&mut world, hero);
}

#[test]
fn buy_order_rejects_sixty_four_gold_before_capacity_and_accepts_sixty_five() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(2));
    world.seats[0].gold = 64;
    assert_eq!(
        world.validate_order(OWNER, None, &BUY_MANGO),
        Err(RejectReason::NotEnoughGold)
    );
    world.seats[0].gold = 65;
    assert_eq!(world.validate_order(OWNER, None, &BUY_MANGO), Ok(()));
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(world.seats[0].gold, 0);
    assert_eq!(held(&world, hero, 0).charges, 3);
}

#[test]
fn remote_buy_order_counts_only_stash_stack_room() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(2));
    world.seats[0].stash.slots[0] = Some(mango(2));
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    assert_eq!(world.validate_order(OWNER, None, &BUY_MANGO), Ok(()));
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
    assert_eq!(held(&world, hero, 0).charges, 2);
    assert_eq!(
        world.validate_order(OWNER, None, &BUY_MANGO),
        Err(RejectReason::InventoryFull)
    );
}

#[test]
fn buy_order_does_not_count_foreign_or_marked_stack_room() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    let mut foreign = mango(1);
    foreign.owner = SlotId(1);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(foreign);
    let mut marked = mango(1);
    marked.for_sale = true;
    world.seats[0].stash.slots[0] = Some(marked);
    assert_eq!(
        world.validate_order(OWNER, None, &BUY_MANGO),
        Err(RejectReason::InventoryFull)
    );
    reject_buy(&mut world, hero);
}

#[test]
fn courier_named_buy_order_does_not_treat_courier_bag_as_purchase_capacity() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.transform.get_mut(hero).unwrap().pos = AWAY;
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    assert!(world.at_shop(courier));
    assert_eq!(
        world.validate_order(OWNER, Some(wire_id(courier)), &BUY_MANGO),
        Err(RejectReason::InventoryFull)
    );
    reject_buy(&mut world, hero);
}

#[test]
fn courier_named_buy_order_counts_hero_stack_room_at_shop() {
    let (mut world, hero) = fixture();
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(2));
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    world.transform.get_mut(courier).unwrap().pos = AWAY;
    assert!(world.at_shop(hero));
    assert_eq!(
        world.validate_order(OWNER, Some(wire_id(courier)), &BUY_MANGO),
        Ok(())
    );
    assert!(world.buy(OWNER, MANGO, &mut Vec::new()));
    assert_eq!(held(&world, hero, 0).charges, 3);
}

#[test]
fn non_mango_buy_order_keeps_catalog_price_and_empty_slot_validation() {
    let (mut world, hero) = fixture();
    let mut stick = mango(1);
    stick.id = ItemId(crate::game::ITEM_MAGIC_STICK);
    stick.charges = 0;
    put(&mut world, hero, 0, stick);
    world.seats[0].gold = 250;
    let wand = Order::Buy {
        item: ItemId(crate::game::ITEM_MAGIC_WAND),
    };
    assert_eq!(
        world.validate_order(OWNER, None, &wand),
        Err(RejectReason::NotEnoughGold)
    );
    fill_storage(&mut world, hero);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(mango(1));
    let tango = Order::Buy {
        item: ItemId(crate::game::ITEM_TANGO),
    };
    assert_eq!(
        world.validate_order(OWNER, None, &tango),
        Err(RejectReason::InventoryFull)
    );
}

#[test]
fn use_order_accepts_none_and_explicit_self_for_every_positive_deficit_boundary() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let maximum = world.stats.get(hero).unwrap().max_mana;
    for deficit in [
        Fixed::EPSILON,
        Fixed::from_ratio(1, 2),
        Fixed::ONE,
        Fixed::from_int(99),
        Fixed::from_int(100),
        Fixed::from_int(500),
    ] {
        world.mana.get_mut(hero).unwrap().mana = maximum - deficit;
        for target in [Target::None, Target::Unit(wire_id(hero))] {
            let order = Order::Use {
                slot: ItemSlot(0),
                target,
            };
            assert_eq!(world.validate_order(OWNER, None, &order), Ok(()));
        }
        assert_eq!(held(&world, hero, 0).charges, 3);
        assert_eq!(world.mana.get(hero).unwrap().mana, maximum - deficit);
    }
}

#[test]
fn use_order_rejects_full_and_overfull_mana_as_not_ready() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let maximum = world.stats.get(hero).unwrap().max_mana;
    for extra in [Fixed::ZERO, Fixed::EPSILON] {
        world.mana.get_mut(hero).unwrap().mana = maximum + extra;
        for target in [Target::None, Target::Unit(wire_id(hero))] {
            let order = Order::Use {
                slot: ItemSlot(0),
                target,
            };
            assert_eq!(
                world.validate_order(OWNER, None, &order),
                Err(RejectReason::NotReady)
            );
        }
    }
    assert_eq!(held(&world, hero, 0), mango(3));
}

#[test]
fn use_order_rejects_absent_mana_pool_as_not_ready() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.mana.remove(hero);
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::NotReady)
    );
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn use_order_rejects_zero_mana_capacity_as_not_ready() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.stats.get_mut(hero).unwrap().max_mana = Fixed::ZERO;
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::NotReady)
    );
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn use_order_rejects_missing_stats_as_not_ready() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.stats.remove(hero);
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::NotReady)
    );
    reject_use(&mut world, hero, 0, Target::None);
}

#[test]
fn use_order_rejects_other_units_positions_and_stale_targets_as_wrong_target_kind() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    let ally = world.spawn_hero(Team::Radiant, AWAY, SlotId(1), HeroId(2));
    let enemy = world.spawn_hero(Team::Dire, AWAY, SlotId(2), HeroId(2));
    world.settle();
    let own_position = world.transform.get(hero).unwrap().pos;
    for target in [
        Target::Unit(wire_id(ally)),
        Target::Unit(wire_id(enemy)),
        Target::Pos(own_position),
        Target::Pos(AWAY),
        Target::Unit(EntityId {
            idx: u32::MAX,
            generation: u32::MAX,
        }),
    ] {
        let order = Order::Use {
            slot: ItemSlot(0),
            target,
        };
        assert_eq!(
            world.validate_order(OWNER, None, &order),
            Err(RejectReason::WrongTargetKind)
        );
        reject_use(&mut world, hero, 0, target);
    }
}

#[test]
fn use_order_rejects_mute_as_not_ready_and_accepts_exact_expiry() {
    let (mut world, hero) = fixture();
    let mut stack = mango(3);
    stack.mute = 1;
    put(&mut world, hero, 0, stack);
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::NotReady)
    );
    world.tick_gear();
    assert_eq!(world.validate_order(OWNER, None, &USE_MANGO), Ok(()));
    assert_eq!(held(&world, hero, 0).charges, 3);
}

#[test]
fn use_order_rejects_backpack_and_stash_as_wrong_target_kind() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, rules::INVENTORY_SLOTS, mango(3));
    world.seats[0].stash.slots[0] = Some(mango(3));
    for at in [rules::INVENTORY_SLOTS, BAG_SLOTS] {
        for target in [Target::None, Target::Unit(wire_id(hero))] {
            let order = Order::Use {
                slot: ItemSlot(at as u8),
                target,
            };
            assert_eq!(
                world.validate_order(OWNER, None, &order),
                Err(RejectReason::WrongTargetKind)
            );
        }
    }
    assert_eq!(world.seats[0].stash.slots[0], Some(mango(3)));
}

#[test]
fn use_order_rejects_dead_or_missing_hero_as_hero_dead() {
    let (mut world, hero) = fixture();
    put(&mut world, hero, 0, mango(3));
    world.health.get_mut(hero).unwrap().hp = Fixed::ZERO;
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::HeroDead)
    );
    world.seats[0].unit = None;
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::HeroDead)
    );
}

#[test]
fn use_order_rejects_empty_charges_as_no_charges_even_at_full_mana() {
    let (mut world, hero) = fixture();
    let mut stack = mango(1);
    stack.charges = 0;
    put(&mut world, hero, 0, stack);
    world.mana.get_mut(hero).unwrap().mana = world.stats.get(hero).unwrap().max_mana;
    for target in [Target::None, Target::Unit(wire_id(hero))] {
        let order = Order::Use {
            slot: ItemSlot(0),
            target,
        };
        assert_eq!(
            world.validate_order(OWNER, None, &order),
            Err(RejectReason::NoCharges)
        );
    }
    assert_eq!(held(&world, hero, 0), stack);
}

#[test]
fn use_order_keeps_cooldown_and_disabled_errors_before_mana_readiness() {
    let (mut world, hero) = fixture();
    let mut stack = mango(3);
    stack.cooldown = 1;
    put(&mut world, hero, 0, stack);
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::OnCooldown)
    );
    world.tick_gear();
    world.statuses.insert(
        hero,
        Statuses(vec![Status {
            kind: StatusKind::Stunned,
            ticks_left: 1,
        }]),
    );
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::Disabled)
    );
    world.mana.get_mut(hero).unwrap().mana = world.stats.get(hero).unwrap().max_mana;
    assert_eq!(
        world.validate_order(OWNER, None, &USE_MANGO),
        Err(RejectReason::Disabled)
    );
}

#[test]
fn courier_use_order_without_mana_pool_rejects_as_not_ready() {
    let (mut world, _) = fixture();
    world.stand_up_courier(0);
    world.settle();
    let courier = world.seats[0].courier.unwrap();
    put(&mut world, courier, 0, mango(3));
    assert_eq!(
        world.validate_order(OWNER, Some(wire_id(courier)), &USE_MANGO),
        Err(RejectReason::NotReady)
    );
    reject_use(&mut world, courier, 0, Target::None);
}

#[test]
fn non_mango_restore_keeps_none_only_targeting_and_no_deficit_requirement() {
    let (mut world, hero) = fixture();
    let mut stick = mango(3);
    stick.id = ItemId(crate::game::ITEM_MAGIC_STICK);
    put(&mut world, hero, 0, stick);
    world.mana.get_mut(hero).unwrap().mana = world.stats.get(hero).unwrap().max_mana;
    assert_eq!(world.validate_order(OWNER, None, &USE_MANGO), Ok(()));
    let explicit_self = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(wire_id(hero)),
    };
    assert_eq!(
        world.validate_order(OWNER, None, &explicit_self),
        Err(RejectReason::WrongTargetKind)
    );
    world.mana.remove(hero);
    assert_eq!(world.validate_order(OWNER, None, &USE_MANGO), Ok(()));
}
