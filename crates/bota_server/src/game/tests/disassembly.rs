//! Taking a build apart and locking items against combining, by the rules
//! `bota_core::disassembly` lists (Dota 7.41f: `items.txt`'s
//! `ItemDisassembleRule`, the localization's Combine Lock and Disassembly
//! glossary, Liquipedia's *Item Disassembling*): what may come apart and when,
//! where, what comes back, and what a lock keeps out of a build.

use bota_core::rules;
use bota_proto::{Disassembly, ItemId, ItemSlot, Order, RejectReason, SlotId, Target, Team, Vec2};

use crate::game::{BAG_SLOTS, Entity, Inventory, World, wire_id};

use super::support::*;

const BRACER_PARTS: [ItemId; 3] = [ItemId::CIRCLET, ItemId::GAUNTLETS, ItemId::RECIPE_BRACER];

/// A hero with gold, in its shop or out on the lane.
fn a_hero(at_shop: bool) -> (World, Entity) {
    let (mut world, hero) = a_hero_with_gold(10_000);
    if !at_shop {
        world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
        world.step();
    }
    (world, hero)
}

/// A hero whose bag holds a Bracer its parts just built, at slot 0.
fn with_a_bracer(at_shop: bool) -> (World, Entity) {
    let (mut world, hero) = a_hero(at_shop);
    for (at, part) in BRACER_PARTS.into_iter().enumerate() {
        lay(world.inventory.get_mut(hero).expect("bag"), at, part);
    }
    world.step();
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::BRACER]
    );
    (world, hero)
}

fn lay(bag: &mut Inventory, at: usize, item: ItemId) {
    bag.slots[at] = Some(fresh_stack(item, 0));
}

fn ids(bag: &Inventory) -> Vec<ItemId> {
    bag.held().map(|stack| stack.id).collect()
}

fn disassemble(at: usize) -> Order {
    Order::Disassemble {
        slot: ItemSlot(at as u8),
    }
}

fn lock(at: usize, locked: bool) -> Order {
    Order::SetCombineLock {
        slot: ItemSlot(at as u8),
        locked,
    }
}

fn refusal(world: &World, unit: Option<Entity>, order: &Order) -> Result<(), RejectReason> {
    world.validate_order(SlotId(0), unit.map(wire_id), order)
}

#[test]
fn a_build_comes_apart_into_its_parts_and_its_recipe_every_one_locked() {
    let (mut world, hero) = with_a_bracer(true);
    let gold = world.seats[0].gold;
    advance_validated(&mut world, None, disassemble(0));
    let bag = world.inventory.get(hero).expect("bag");
    let mut got = ids(bag);
    got.sort();
    let mut parts = BRACER_PARTS.to_vec();
    parts.sort();
    assert_eq!(got, parts, "the recipe scroll comes back too");
    assert!(
        bag.held().all(|stack| stack.locked),
        "all are combine locked"
    );
    assert_eq!(world.seats[0].gold, gold, "no gold changes hands");
    run(&mut world, 5);
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")).len(),
        3,
        "locked, they do not build again"
    );
}

#[test]
fn a_build_may_come_apart_for_ten_seconds_after_it_was_combined_and_no_longer() {
    for (wait, allowed) in [
        (rules::DISASSEMBLY_WINDOW_TICKS - 2, true),
        (rules::DISASSEMBLY_WINDOW_TICKS + 2, false),
    ] {
        let (mut world, _) = with_a_bracer(true);
        run(&mut world, wait);
        let want = if allowed {
            Ok(())
        } else {
            Err(RejectReason::NotDisassemblable)
        };
        assert_eq!(refusal(&world, None, &disassemble(0)), want, "after {wait}");
    }
}

#[test]
fn using_a_build_ends_the_window() {
    let (mut world, hero) = a_hero(true);
    for (at, part) in [ItemId::BOOTS, ItemId::GLOVES, ItemId::BELT]
        .into_iter()
        .enumerate()
    {
        lay(world.inventory.get_mut(hero).expect("bag"), at, part);
    }
    world.step();
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::POWER_TREADS]
    );
    assert_eq!(refusal(&world, None, &disassemble(0)), Ok(()));
    // The treads' toggle is a use of the item.
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert_eq!(
        refusal(&world, None, &disassemble(0)),
        Err(RejectReason::NotDisassemblable)
    );
}

#[test]
fn a_carrier_that_dies_takes_the_window_with_it_but_the_stash_keeps_its_own() {
    let (mut world, hero) = a_hero(false);
    for (at, part) in BRACER_PARTS.into_iter().enumerate() {
        lay(world.inventory.get_mut(hero).expect("bag"), at, part);
        lay(&mut world.seats[0].stash, at, part);
    }
    // Away from the shop the bag and the stash build apart.
    world.step();
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::BRACER]
    );
    assert_eq!(ids(&world.seats[0].stash), [ItemId::BRACER]);
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    world.step();
    assert_eq!(
        refusal(&world, None, &disassemble(0)),
        Err(RejectReason::NotDisassemblable),
        "what the body carried is past taking apart"
    );
    assert_eq!(
        refusal(&world, None, &disassemble(BAG_SLOTS)),
        Ok(()),
        "a hero that lies dead takes apart what its stash holds"
    );
    seat_order(&mut world, disassemble(BAG_SLOTS));
    assert_eq!(ids(&world.seats[0].stash).len(), 3);
}

#[test]
fn a_build_that_enters_the_bag_of_another_seats_unit_can_no_longer_come_apart() {
    let (mut world, hero) = with_a_bracer(true);
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    let ally = seat_hero(&mut world, 1, Team::Radiant, spot + Vec2::from_ints(100, 0));
    world.settle();
    assert!(world.put_item(hero, 0, Target::Unit(wire_id(ally))));
    run(&mut world, 5);
    let received = world
        .inventory
        .get(ally)
        .expect("bag")
        .held()
        .next()
        .copied();
    assert_eq!(received.map(|stack| stack.disassemble_until), Some(0));
}

#[test]
fn a_build_comes_apart_in_the_backpack_on_the_courier_and_in_the_stash_within_reach() {
    let (mut world, hero) = with_a_bracer(true);
    // The backpack: past the six slots a hero uses.
    world
        .inventory
        .get_mut(hero)
        .expect("bag")
        .swap_slots(0, BAG_SLOTS - 1);
    let backpack = (0..BAG_SLOTS)
        .rev()
        .find(|at| slot_of(&world, hero, *at).is_some())
        .expect("held");
    assert_eq!(refusal(&world, None, &disassemble(backpack)), Ok(()));

    let (mut world, _) = a_hero(true);
    let courier = the_courier(&world);
    for (at, part) in BRACER_PARTS.into_iter().enumerate() {
        lay(world.inventory.get_mut(courier).expect("bag"), at, part);
    }
    world.step();
    let built = ids(world.inventory.get(courier).expect("bag"));
    assert_eq!(built, [ItemId::BRACER]);
    advance_validated(&mut world, Some(courier), disassemble(0));
    assert_eq!(ids(world.inventory.get(courier).expect("bag")).len(), 3);

    for at_shop in [true, false] {
        let (mut world, _) = a_hero(at_shop);
        let mut built = fresh_stack(ItemId::BRACER, 0);
        built.disassemble_until = world.tick + rules::DISASSEMBLY_WINDOW_TICKS;
        world.seats[0].stash.slots[0] = Some(built);
        let want = if at_shop {
            Ok(())
        } else {
            Err(RejectReason::NotAtShop)
        };
        assert_eq!(refusal(&world, None, &disassemble(BAG_SLOTS)), want);
    }
}

#[test]
fn what_is_built_of_nothing_or_never_comes_apart() {
    let (mut world, hero) = a_hero(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        1,
        ItemId::WARD_DISPENSER,
    );
    for at in [0, 1] {
        assert_eq!(
            refusal(&world, None, &disassemble(at)),
            Err(RejectReason::NotDisassemblable)
        );
    }
    assert_eq!(
        refusal(&world, None, &disassemble(2)),
        Err(RejectReason::EmptySlot)
    );
}

#[test]
fn only_the_seat_that_owns_an_item_takes_it_apart_or_locks_it() {
    let (mut world, hero) = with_a_bracer(true);
    world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("held")
        .owner = SlotId(1);
    for order in [disassemble(0), lock(0, true)] {
        assert_eq!(
            refusal(&world, None, &order),
            Err(RejectReason::NotYourItem)
        );
    }
}

#[test]
fn parts_land_where_the_build_was_and_the_rest_in_the_bag_and_then_on_the_ground() {
    let (mut world, hero) = with_a_bracer(true);
    for at in 1..BAG_SLOTS - 1 {
        lay(
            world.inventory.get_mut(hero).expect("bag"),
            at,
            ItemId::TANGO,
        );
    }
    let before = on_the_ground(&world).len();
    advance_validated(&mut world, None, disassemble(0));
    let bag = world.inventory.get(hero).expect("bag");
    assert!(
        bag.slots[0].is_some_and(|stack| stack.locked),
        "the first part where it was"
    );
    assert_eq!(
        bag.held().filter(|stack| stack.locked).count(),
        2,
        "one more fits"
    );
    assert_eq!(
        on_the_ground(&world).len(),
        before + 1,
        "the last lies at the hero's feet"
    );
}

#[test]
fn a_build_that_gathered_charges_gives_them_back_to_the_part_that_gathers_them() {
    let (mut world, hero) = a_hero(true);
    for (at, part) in [
        ItemId::MAGIC_STICK,
        ItemId::IRON_BRANCH,
        ItemId::IRON_BRANCH,
        ItemId::RECIPE_MAGIC_WAND,
    ]
    .into_iter()
    .enumerate()
    {
        lay(world.inventory.get_mut(hero).expect("bag"), at, part);
    }
    world.step();
    let wand = world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("built");
    assert_eq!(wand.id, ItemId::MAGIC_WAND);
    wand.charges = 14;
    advance_validated(&mut world, None, disassemble(0));
    let stick = world
        .inventory
        .get(hero)
        .expect("bag")
        .held()
        .find(|stack| stack.id == ItemId::MAGIC_STICK)
        .copied()
        .expect("the stick");
    assert_eq!(stick.charges, 10, "up to what a stick holds");
    assert_eq!(stick.cooldown, 0);
}

#[test]
fn a_build_comes_apart_by_the_recipe_its_attribute_names() {
    let (mut world, hero) = a_hero(true);
    for (at, part) in [ItemId::BOOTS, ItemId::GLOVES, ItemId::ROBE]
        .into_iter()
        .enumerate()
    {
        lay(world.inventory.get_mut(hero).expect("bag"), at, part);
    }
    world.step();
    advance_validated(&mut world, None, disassemble(0));
    let mut got = ids(world.inventory.get(hero).expect("bag"));
    got.sort();
    let mut want = vec![ItemId::BOOTS, ItemId::GLOVES, ItemId::ROBE];
    want.sort();
    assert_eq!(got, want, "treads of intelligence come of the robe");
}

#[test]
fn a_locked_part_is_no_part_of_a_build_in_the_bag_with_the_stash() {
    let (mut world, hero) = a_hero(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    advance_validated(&mut world, None, lock(0, true));
    lay(&mut world.seats[0].stash, 0, ItemId::GAUNTLETS);
    lay(&mut world.seats[0].stash, 1, ItemId::RECIPE_BRACER);
    run(&mut world, 3);
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::CIRCLET],
        "the lock holds the circlet out of the build its stash would complete"
    );
    assert_eq!(ids(&world.seats[0].stash).len(), 2);
    advance_validated(&mut world, None, lock(0, false));
    run(&mut world, 3);
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::BRACER]
    );
}

#[test]
fn a_locked_part_is_no_part_of_a_build_in_the_stash_or_on_the_courier() {
    let (mut world, _) = a_hero(false);
    lay(&mut world.seats[0].stash, 0, ItemId::CIRCLET);
    lay(&mut world.seats[0].stash, 1, ItemId::GAUNTLETS);
    lay(&mut world.seats[0].stash, 2, ItemId::RECIPE_BRACER);
    world.seats[0].stash.slots[0].as_mut().expect("held").locked = true;
    let courier = the_courier(&world);
    for (at, part) in BRACER_PARTS.into_iter().enumerate() {
        lay(world.inventory.get_mut(courier).expect("bag"), at, part);
    }
    world.inventory.get_mut(courier).expect("bag").slots[0]
        .as_mut()
        .expect("held")
        .locked = true;
    run(&mut world, 3);
    assert_eq!(
        ids(&world.seats[0].stash).len(),
        3,
        "not built in the stash"
    );
    assert_eq!(
        ids(world.inventory.get(courier).expect("bag")).len(),
        3,
        "not built on the courier"
    );
    advance_validated(&mut world, None, lock(BAG_SLOTS, false));
    advance_validated(&mut world, Some(courier), lock(0, false));
    run(&mut world, 3);
    assert_eq!(ids(&world.seats[0].stash), [ItemId::BRACER]);
    assert_eq!(
        ids(world.inventory.get(courier).expect("bag")),
        [ItemId::BRACER]
    );
}

#[test]
fn a_locked_part_stays_out_of_the_build_when_the_courier_brings_the_rest() {
    let (mut world, hero) = a_hero(false);
    let courier = the_courier(&world);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    lay(
        world.inventory.get_mut(courier).expect("bag"),
        0,
        ItemId::GAUNTLETS,
    );
    lay(
        world.inventory.get_mut(courier).expect("bag"),
        1,
        ItemId::RECIPE_BRACER,
    );
    advance_validated(&mut world, None, lock(0, true));
    advance_validated(
        &mut world,
        Some(courier),
        Order::Cast {
            slot: bota_proto::AbilitySlot(3),
            target: Target::None,
        },
    );
    run(&mut world, 900);
    let mut got = ids(world.inventory.get(hero).expect("bag"));
    got.sort();
    let mut want = vec![ItemId::CIRCLET, ItemId::GAUNTLETS, ItemId::RECIPE_BRACER];
    want.sort();
    assert_eq!(got, want, "delivered, and not built");
    let circlet = (0..BAG_SLOTS)
        .find(|at| slot_of(&world, hero, *at).is_some_and(|stack| stack.id == ItemId::CIRCLET))
        .expect("held");
    advance_validated(&mut world, None, lock(circlet, false));
    run(&mut world, 3);
    assert_eq!(
        ids(world.inventory.get(hero).expect("bag")),
        [ItemId::BRACER]
    );
}

#[test]
fn a_lock_is_set_not_flipped_and_only_an_item_that_is_a_part_of_a_recipe_takes_one() {
    let (mut world, hero) = a_hero(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        1,
        ItemId::TANGO,
    );
    for _ in 0..2 {
        advance_validated(&mut world, None, lock(0, true));
        assert!(slot_of(&world, hero, 0).expect("held").locked);
    }
    advance_validated(&mut world, None, lock(0, false));
    assert!(!slot_of(&world, hero, 0).expect("held").locked);
    assert_eq!(
        refusal(&world, None, &lock(1, true)),
        Err(RejectReason::CannotLockCombining)
    );
    assert_eq!(
        refusal(&world, None, &lock(3, true)),
        Err(RejectReason::EmptySlot)
    );
}

#[test]
fn a_locked_item_sells_for_what_it_always_does() {
    let (mut world, hero) = a_hero(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("held")
        .touched = true;
    advance_validated(&mut world, None, lock(0, true));
    let gold = world.seats[0].gold;
    advance_validated(&mut world, None, Order::Sell { slot: ItemSlot(0) });
    assert_eq!(world.seats[0].gold - gold, price_of(ItemId::CIRCLET) / 2);
    assert!(slot_of(&world, hero, 0).is_none());
}

#[test]
fn a_purchase_does_not_count_a_locked_part_as_held() {
    let (mut world, hero) = a_hero(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        0,
        ItemId::CIRCLET,
    );
    advance_validated(&mut world, None, lock(0, true));
    let gold = world.seats[0].gold;
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::BRACER,
        },
    );
    assert_eq!(
        gold - world.seats[0].gold,
        price_of(ItemId::BRACER),
        "all of it is bought: the locked circlet is not spent on the build"
    );
    let bag = world.inventory.get(hero).expect("bag");
    assert!(ids(bag).contains(&ItemId::BRACER));
    assert!(
        bag.held()
            .any(|stack| stack.id == ItemId::CIRCLET && stack.locked),
        "and the locked one stays"
    );
}

#[test]
fn an_order_to_take_apart_or_lock_leaves_what_the_body_is_doing_alone() {
    let (mut world, hero) = with_a_bracer(true);
    let from = world.transform.get(hero).expect("standing").pos;
    let far = rules::DEMO_LANE_CORNERS[0];
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Pos(far),
        },
    );
    run(&mut world, 10);
    advance_validated(&mut world, None, disassemble(0));
    advance_validated(&mut world, None, lock(0, false));
    let mid = world.transform.get(hero).expect("standing").pos;
    run(&mut world, 30);
    let later = world.transform.get(hero).expect("standing").pos;
    assert!(
        mid.within(from, rules::units(3000)) && later != mid,
        "it walks on"
    );
}

#[test]
fn a_view_shows_its_own_side_what_is_locked_and_what_may_come_apart_and_the_enemy_neither() {
    let (mut world, hero) = with_a_bracer(true);
    lay(
        world.inventory.get_mut(hero).expect("bag"),
        1,
        ItemId::CIRCLET,
    );
    advance_validated(&mut world, None, lock(1, true));
    let own = shown(&world, Some(Team::Radiant), hero);
    let build = own.items[0].expect("shown");
    assert_eq!(build.locked, Some(false));
    assert!(
        matches!(build.disassembly, Some(Disassembly::Window { ticks_left }) if ticks_left > 0)
    );
    let part = own.items[1].expect("shown");
    assert_eq!((part.locked, part.disassembly), (Some(true), None));
    let here = world.transform.get(hero).expect("standing").pos;
    seat_hero(&mut world, 1, Team::Dire, here + Vec2::from_ints(100, 0));
    world.settle();
    let theirs = shown(&world, Some(Team::Dire), hero);
    for item in theirs.items.iter().flatten() {
        assert_eq!((item.locked, item.disassembly), (None, None));
    }
}
