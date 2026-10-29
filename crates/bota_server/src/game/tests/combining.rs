//! Recipes combine where their parts can reach one another, as in Dota: in
//! the stash, in the stash together with the hero's own bag while the hero is
//! in its shop, on the courier, and on the courier's arrival; a purchase
//! counts every part the seat owns anywhere.

use bota_core::rules;
use bota_proto::{AbilitySlot, ItemId, Order, Target};

use crate::game::{Entity, Inventory, World};

use super::support::*;

const PARTS: [ItemId; 3] = [ItemId::CIRCLET, ItemId::GAUNTLETS, ItemId::RECIPE_BRACER];

/// A hero with gold, in its shop or out on the lane.
fn a_hero(at_shop: bool) -> (World, Entity) {
    let (mut world, hero) = a_hero_with_gold(10_000);
    if !at_shop {
        world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
        world.step();
    }
    (world, hero)
}

fn lay(bag: &mut Inventory, at: usize, item: ItemId) {
    bag.slots[at] = Some(fresh_stack(item, 0));
}

fn ids(bag: &Inventory) -> Vec<ItemId> {
    bag.held().map(|stack| stack.id).collect()
}

#[test]
fn parts_bought_away_from_the_shop_build_in_the_stash() {
    let (mut world, hero) = a_hero(false);
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::BRACER,
        },
    );
    assert_eq!(ids(&world.seats[0].stash), [ItemId::BRACER]);
    assert!(ids(world.inventory.get(hero).expect("bag")).is_empty());
}

#[test]
fn the_stash_and_the_hero_in_its_shop_build_together_but_not_afar() {
    for at_shop in [true, false] {
        let (mut world, hero) = a_hero(at_shop);
        lay(world.inventory.get_mut(hero).expect("bag"), 2, PARTS[0]);
        lay(&mut world.seats[0].stash, 0, PARTS[1]);
        lay(&mut world.seats[0].stash, 1, PARTS[2]);
        world.step();
        let bag = world.inventory.get(hero).expect("bag");
        if at_shop {
            assert_eq!(
                ids(bag),
                [ItemId::BRACER],
                "the build lands in the hero's bag"
            );
            assert!(ids(&world.seats[0].stash).is_empty());
        } else {
            assert_eq!(ids(bag), [PARTS[0]], "away, the stash is out of reach");
            assert_eq!(ids(&world.seats[0].stash).len(), 2);
        }
    }
}

#[test]
fn parts_on_the_courier_build_on_the_courier() {
    let (mut world, _) = a_hero(true);
    let courier = the_courier(&world);
    for (at, part) in PARTS.into_iter().enumerate() {
        lay(world.inventory.get_mut(courier).expect("bag"), at, part);
    }
    world.step();
    assert_eq!(
        ids(world.inventory.get(courier).expect("bag")),
        [ItemId::BRACER]
    );
}

#[test]
fn what_the_courier_brings_builds_with_what_the_hero_holds() {
    let (mut world, hero) = a_hero(false);
    let courier = the_courier(&world);
    lay(world.inventory.get_mut(hero).expect("bag"), 0, PARTS[0]);
    lay(world.inventory.get_mut(courier).expect("bag"), 0, PARTS[1]);
    lay(world.inventory.get_mut(courier).expect("bag"), 1, PARTS[2]);
    world.step();
    assert_eq!(
        ids(world.inventory.get(courier).expect("bag")).len(),
        2,
        "on the courier alone it is two parts short of nothing"
    );
    advance_validated(
        &mut world,
        Some(courier),
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::None,
        },
    );
    run(&mut world, 900);
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::BRACER]
    );
    assert!(ids(world.inventory.get(courier).expect("bag")).is_empty());
}

#[test]
fn a_purchase_counts_the_parts_on_the_courier_and_in_the_stash() {
    let (mut world, hero) = a_hero(false);
    let courier = the_courier(&world);
    lay(world.inventory.get_mut(courier).expect("bag"), 0, PARTS[0]);
    lay(&mut world.seats[0].stash, 0, PARTS[1]);
    let gold = world.seats[0].gold;
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::BRACER,
        },
    );
    assert_eq!(
        gold - world.seats[0].gold,
        price_of(ItemId::RECIPE_BRACER),
        "only the recipe is bought"
    );
    assert_eq!(
        ids(&world.seats[0].stash),
        [ItemId::GAUNTLETS, ItemId::RECIPE_BRACER],
        "the recipe waits with its part: the courier's is a world away"
    );
    assert!(ids(world.inventory.get(hero).expect("bag")).is_empty());
}
