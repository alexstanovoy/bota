//! The shop while a seat's hero lies dead, as in Dota: buying goes to the
//! stash, with what the body carried counting towards a build; only what
//! was bought since the fall sells back; and what the body carried sells at
//! the full price no more.

use bota_core::item_def;
use bota_proto::{EventKind, ItemId, ItemSlot, Order, RejectReason, SlotId, Target};

use crate::game::{BAG_SLOTS, World};

use super::support::*;

/// A match whose one hero carries boots and has just fallen, with gold to
/// spend.
fn fallen_with_boots() -> World {
    let (mut world, hero) = match_hero();
    world.seats[0].gold = 5000;
    world.inventory.get_mut(hero).expect("a bag").slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    world.step();
    assert!(world.seats[0].unit.is_none(), "the hero lies dead");
    world
}

fn stash_items(world: &World) -> Vec<ItemId> {
    world.seats[0]
        .stash
        .slots
        .iter()
        .flatten()
        .map(|stack| stack.id)
        .collect()
}

#[test]
fn a_seat_whose_hero_lies_dead_buys_into_the_stash_with_what_the_body_carried_counting() {
    let mut world = fallen_with_boots();
    let walk = Order::Move {
        target: Target::Pos(bota_proto::Vec2::from_ints(5000, 5000)),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &walk),
        Err(RejectReason::HeroDead),
        "a body's orders wait for the body"
    );
    let treads = Order::Buy {
        item: ItemId::POWER_TREADS,
    };
    assert_eq!(world.validate_order(SlotId(0), None, &treads), Ok(()));
    let gold = world.seats[0].gold;
    let events = seat_order(&mut world, treads);
    assert!(events.iter().any(|event| event.kind
        == EventKind::ItemBought {
            slot: SlotId(0),
            item: ItemId::POWER_TREADS,
        }));
    let boots = item_def(ItemId::BOOTS).expect("boots").cost;
    let treads = item_def(ItemId::POWER_TREADS).expect("treads").cost;
    assert_eq!(
        gold - world.seats[0].gold,
        treads - boots,
        "the carried boots are not bought again"
    );
    // The parts bought build with the body's boots at once (Dota: the stash
    // combines with the inventory of a hero lying dead).
    let kept = world.seats[0]
        .kept
        .as_ref()
        .expect("the body's bag is kept");
    let carried: Vec<ItemId> = kept.bag.held().map(|stack| stack.id).collect();
    assert!(carried.contains(&ItemId::POWER_TREADS), "{carried:?}");
    assert!(!carried.contains(&ItemId::BOOTS));
    // Quick-buy is buying: a single item whole, too.
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::TANGO,
        },
    );
    assert!(stash_items(&world).contains(&ItemId::TANGO));
}

#[test]
fn while_the_hero_lies_dead_only_what_was_bought_since_it_fell_sells_back() {
    let (mut world, hero) = match_hero();
    world.seats[0].gold = 1000;
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::CIRCLET, 0));
    world.inventory.get_mut(hero).expect("a bag").slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    run(&mut world, 10);
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    world.step();
    let sell = |at: usize| Order::Sell {
        slot: ItemSlot(at as u8),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &sell(BAG_SLOTS)),
        Err(RejectReason::HeroDead),
        "the stash held it before the fall"
    );
    assert_eq!(
        world.validate_order(SlotId(0), None, &sell(0)),
        Err(RejectReason::HeroDead),
        "the body's bag waits for the body"
    );
    let before = world.seats[0].gold;
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::TANGO,
        },
    );
    let paid = before - world.seats[0].gold;
    assert!(paid > 0);
    let at = world.seats[0]
        .stash
        .slots
        .iter()
        .position(|slot| slot.is_some_and(|stack| stack.id == ItemId::TANGO))
        .expect("bought into the stash");
    let gold = world.seats[0].gold;
    assert_eq!(
        world.validate_order(SlotId(0), None, &sell(BAG_SLOTS + at)),
        Ok(())
    );
    seat_order(&mut world, sell(BAG_SLOTS + at));
    assert_eq!(
        world.seats[0].gold - gold,
        paid,
        "untouched and fresh: the whole price back"
    );
    assert!(!stash_items(&world).contains(&ItemId::TANGO));
    assert!(
        stash_items(&world).contains(&ItemId::CIRCLET),
        "the older stack stays"
    );
}

#[test]
fn what_a_body_carried_when_it_fell_sells_back_at_the_full_price_no_more() {
    let (mut world, hero) = match_hero();
    world.inventory.get_mut(hero).expect("a bag").slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    let kept = world.seats[0].kept.as_ref().expect("the fallen body's kit");
    let boots = kept.bag.slots[0].expect("the boots it carried");
    assert!(
        boots.touched,
        "Dota: the sellback timer ends when the carrier dies"
    );
    assert_eq!(kept.fell, world.tick);
}

/// Checked in the Dota 2 client (7.41, 2026-10-24): a dead hero may swap its
/// items between its slots; the stash, at a shop it does not stand in, stays
/// out of reach, and what it carried comes back where it was put.
#[test]
fn a_dead_hero_swaps_what_it_carried_between_its_slots_and_reaches_no_stash() {
    let mut world = fallen_with_boots();
    let to_backpack = Order::Swap {
        from: ItemSlot(0),
        to: ItemSlot(6),
    };
    assert_eq!(world.validate_order(SlotId(0), None, &to_backpack), Ok(()));
    seat_order(&mut world, to_backpack);
    let kept = |world: &World, at: usize| {
        world.seats[0]
            .kept
            .as_ref()
            .and_then(|kept| kept.bag.slots[at])
            .map(|stack| stack.id)
    };
    assert_eq!(
        kept(&world, 6),
        Some(ItemId::BOOTS),
        "moved to the backpack"
    );
    assert_eq!(kept(&world, 0), None);
    let to_stash = Order::Swap {
        from: ItemSlot(6),
        to: ItemSlot(BAG_SLOTS as u8),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &to_stash),
        Err(RejectReason::NotAtShop)
    );
    let back = Order::Swap {
        from: ItemSlot(6),
        to: ItemSlot(2),
    };
    seat_order(&mut world, back);
    assert_eq!(kept(&world, 2), Some(ItemId::BOOTS));
    let hero = back_on_its_feet(&mut world);
    assert_eq!(
        slot_of(&world, hero, 2).map(|stack| stack.id),
        Some(ItemId::BOOTS),
        "it stands up with its boots where it put them"
    );
}
