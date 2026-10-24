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
    let bought = stash_items(&world);
    assert!(
        !bought.is_empty() && !bought.contains(&ItemId::BOOTS),
        "{bought:?}"
    );
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
