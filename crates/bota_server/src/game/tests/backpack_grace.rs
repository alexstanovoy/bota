//! Backpack timing from Valve's 7.38 and 7.38b notes:
//! https://www.dota2.com/datafeed/patchnotes?version=7.38&language=english
//! https://www.dota2.com/datafeed/patchnotes?version=7.38b&language=english
//! First entry grants five seconds, once per item per hero.

use super::support::*;
use crate::game::{Entity, World};
use bota_core::{fountain_pos, rules};
use bota_proto::{ItemId, ItemSlot, Order, Team, Vec2};

fn lane_item() -> (World, Entity) {
    let (mut world, hero) = match_hero();
    world.transform.get_mut(hero).expect("hero").pos = Vec2::from_ints(8000, 8000);
    hand_item(&mut world, hero, ItemId::CIRCLET, 0);
    world.settle();
    assert!(!world.at_any_shop(hero));
    (world, hero)
}

fn swap(world: &mut World, from: u8, to: u8) {
    advance_validated(
        world,
        None,
        Order::Swap {
            from: ItemSlot(from),
            to: ItemSlot(to),
        },
    );
}

#[test]
fn the_first_backpack_visit_returns_without_downtime_within_five_seconds() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    for _ in 0..147 {
        world.step();
    }
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned").mute,
        0,
        "7.38b's five-second grace still applies before its boundary"
    );
}

#[test]
fn five_seconds_in_the_backpack_exhausts_the_first_visit_grace() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    for _ in 0..rules::BACKPACK_GRACE_TICKS - 1 {
        world.step();
    }
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned").mute,
        rules::BACKPACK_MUTE_TICKS - 1
    );
}

#[test]
fn a_second_backpack_visit_does_not_renew_the_same_items_grace() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("first return").mute,
        0,
        "first visit uses grace"
    );
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("second return").mute,
        rules::BACKPACK_MUTE_TICKS - 1,
        "7.38: only once per item per hero"
    );
}

#[test]
fn entering_the_home_shop_clears_an_existing_backpack_inactivity_timer() {
    let (mut world, hero) = lane_item();
    world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("item")
        .mute = 100;
    world.transform.get_mut(hero).expect("hero").pos = fountain_pos(world.map, Team::Radiant);
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).expect("item").mute,
        0,
        "walking into shop range clears backpack cooldowns (https://www.dota2.com/springcleaning2018/)"
    );
}

#[test]
fn the_secret_shop_uses_the_same_backpack_exemption_and_clears_existing_inactivity() {
    let mut cfg = config();
    cfg.map = bota_proto::MapId(0);
    cfg.objectives = bota_core::map_of(cfg.map).default_objectives;
    cfg.runes = bota_core::map_of(cfg.map).default_runes;
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("hero");
    hand_item(&mut world, hero, ItemId::CIRCLET, 0);
    world.transform.get_mut(hero).expect("hero").pos = Vec2::from_ints(4260, 10950);
    assert!(world.at_any_shop(hero));
    assert!(
        !world.at_shop(hero),
        "secret shop cannot give access to the stash"
    );
    world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("item")
        .mute = 100;
    world.step();
    assert_eq!(slot_of(&world, hero, 0).expect("item").mute, 0);
    swap(&mut world, 0, 6);
    for _ in 0..151 {
        world.step();
    }
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned").mute,
        0,
        "any shop, even after first-entry grace expires"
    );
}

#[test]
fn backpack_grace_information_is_own_side_only() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    let own = world.view(Team::Radiant);
    assert!(
        own.players[0]
            .backpack
            .is_some_and(|b| b.used & (1 << 6) != 0 && b.grace_left[0] > 0)
    );
    world
        .visibility
        .get_mut(hero)
        .expect("visibility")
        .add(Team::Dire);
    let enemy = world.view(Team::Dire);
    assert!(
        enemy.players[0].backpack.is_none(),
        "the other side never learns grace history or its clock"
    );
}

#[test]
fn dropping_and_retrieving_an_item_does_not_renew_its_consumed_first_entry_grace() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(slot_of(&world, hero, 0).expect("first return").mute, 0);
    assert!(world.put_item(hero, 0, bota_proto::Target::None));
    world.step();
    let ground = on_the_ground(&world);
    assert_eq!(ground.len(), 1);
    assert!(world.take_item(hero, crate::game::wire_id(ground[0])));
    world.step();
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0)
            .expect("second visit after ground")
            .mute,
        rules::BACKPACK_MUTE_TICKS - 1,
        "per-item history follows the stack, not its inventory slot"
    );
}

#[test]
fn another_hero_has_its_own_first_entry_grace_for_the_same_item() {
    let mut cfg = config();
    cfg.picks.push(bota_proto::Pick {
        slot: bota_proto::SlotId(1),
        team: Team::Radiant,
        hero: bota_proto::HeroId::PUDGE,
    });
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("first hero");
    let other = world.seats[1].unit.expect("second hero");
    for unit in [hero, other] {
        world.transform.get_mut(unit).expect("hero").pos = Vec2::from_ints(8000, 8000);
    }
    hand_item(&mut world, hero, ItemId::CIRCLET, 0);
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert!(world.put_item(
        hero,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(other))
    ));
    world.step();
    assert!(slot_of(&world, hero, 0).is_none());
    assert!(world.move_item(bota_proto::SlotId(1), other, 0, 6));
    assert!(world.move_item(bota_proto::SlotId(1), other, 6, 0));
    assert_eq!(
        slot_of(&world, other, 0)
            .expect("first return on second hero")
            .mute,
        0,
        "7.38 grants one grace per item per hero, not only the buyer"
    );
}

#[test]
fn the_enemy_base_shop_also_clears_backpack_inactivity() {
    // https://liquipedia.net/dota2/Portal:Items: the enemy fountain shop is usable too.
    let (mut world, hero) = lane_item();
    world.inventory.get_mut(hero).expect("bag").slots[0]
        .as_mut()
        .expect("item")
        .mute = 100;
    world.transform.get_mut(hero).expect("hero").pos = fountain_pos(world.map, Team::Dire);
    world.step();
    assert_eq!(slot_of(&world, hero, 0).expect("item").mute, 0);
}

fn shop_mangos(backpack: u8, stash: u8) -> (World, Entity) {
    let (mut world, hero) = match_hero();
    let mut item = a_stack_of(ItemId::MANGO, 0);
    item.charges = backpack;
    world.inventory.get_mut(hero).expect("bag").slots[6] = Some(item);
    item.charges = stash;
    world.seats[0].stash.slots[0] = Some(item);
    world.settle();
    (world, hero)
}

#[test]
fn a_partial_stash_merge_preserves_the_resident_backpacks_grace_deadline() {
    let (mut world, hero) = shop_mangos(2, 3);
    let before = slot_of(&world, hero, 6)
        .expect("resident Mango")
        .backpack
        .left(world.tick);
    assert!(before > 0);
    assert!(world.move_item(bota_proto::SlotId(0), hero, 9, 6));
    assert_eq!(slot_of(&world, hero, 6).expect("merged Mango").charges, 3);
    assert_eq!(world.seats[0].stash.slots[0].expect("remainder").charges, 2);
    assert_eq!(
        slot_of(&world, hero, 6)
            .expect("resident Mango")
            .backpack
            .left(world.tick),
        before,
        "the destination never left its backpack slot"
    );
}

#[test]
fn a_partial_backpack_merge_into_stash_preserves_the_remainders_grace_deadline() {
    let (mut world, hero) = shop_mangos(3, 2);
    let before = slot_of(&world, hero, 6)
        .expect("Mango")
        .backpack
        .left(world.tick);
    assert!(before > 0);
    assert!(world.move_item(bota_proto::SlotId(0), hero, 6, 9));
    assert_eq!(slot_of(&world, hero, 6).expect("remainder").charges, 2);
    assert_eq!(
        slot_of(&world, hero, 6)
            .expect("remainder")
            .backpack
            .left(world.tick),
        before,
        "the unmoved charges remain in the same backpack visit"
    );
}

#[test]
fn a_same_upkeep_delivery_then_drop_consumes_the_heros_first_backpack_visit() {
    let mut cfg = config();
    cfg.picks.push(bota_proto::Pick {
        slot: bota_proto::SlotId(1),
        team: Team::Radiant,
        hero: bota_proto::HeroId::PUDGE,
    });
    let mut world = World::for_match(&cfg, cfg.rng());
    let sender = world.seats[0].unit.expect("sender");
    let hero = world.seats[1].unit.expect("receiver");
    for unit in [sender, hero] {
        world.transform.get_mut(unit).expect("standing").pos = Vec2::from_ints(8000, 8000);
    }
    for slot in 0..6 {
        world.inventory.get_mut(hero).expect("bag").slots[slot] =
            Some(fresh_stack(ItemId::CIRCLET, 0));
    }
    hand_item(&mut world, sender, ItemId::BOOTS, 0);
    assert!(world.put_item(
        sender,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(hero))
    ));
    // Both errands run in entity order in the same handling phase. The
    // receiver's slot fills before its already queued drop is carried out.
    world.handling.insert(
        hero,
        crate::game::Handling::PutAt {
            slot: 6,
            pos: world.pos(hero).expect("position"),
        },
    );
    world.step();
    let ground = on_the_ground(&world);
    assert_eq!(
        ground.len(),
        1,
        "delivery and drop both completed this upkeep"
    );
    let stack = world.loot.get(ground[0]).expect("dropped stack").0;
    assert!(
        stack.backpack.used(bota_proto::SlotId(1)),
        "a visit lasting less than one upkeep still consumes grace"
    );
    assert_eq!(
        stack.backpack.left(world.tick),
        0,
        "the dropped item is no longer resident"
    );
}

#[test]
fn assembly_and_disassembly_inherit_consumed_backpack_history() {
    let (mut world, hero) = lane_item();
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    world.inventory.get_mut(hero).expect("bag").slots[1] = Some(fresh_stack(ItemId::GAUNTLETS, 0));
    world.inventory.get_mut(hero).expect("bag").slots[2] =
        Some(fresh_stack(ItemId::RECIPE_BRACER, 0));
    world.step();
    let built = slot_of(&world, hero, 0).expect("assembled");
    assert_eq!(built.id, ItemId::BRACER);
    assert!(
        built.backpack.used(bota_proto::SlotId(0)),
        "assembly inherits the parts' used history"
    );
    advance_validated(&mut world, None, Order::Disassemble { slot: ItemSlot(0) });
    let mut actual: Vec<_> = world
        .inventory
        .get(hero)
        .expect("bag")
        .held()
        .map(|part| part.id)
        .collect();
    actual.sort();
    let mut expected = [ItemId::CIRCLET, ItemId::GAUNTLETS, ItemId::RECIPE_BRACER];
    expected.sort();
    assert_eq!(
        actual, expected,
        "the Bracer really disassembled into its three components"
    );
    assert!(
        world
            .inventory
            .get(hero)
            .expect("bag")
            .held()
            .all(|part| part.backpack.used(bota_proto::SlotId(0))),
        "every disassembled part inherits the build's history"
    );
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned part").mute,
        rules::BACKPACK_MUTE_TICKS - 1,
        "combine/disassemble cannot renew grace"
    );
}

#[test]
fn the_enemy_fountain_shop_sells_bag_items_and_town_portal_scrolls() {
    // https://liquipedia.net/dota2/Portal:Items: selling works at the enemy fountain shop.
    let (mut world, hero) = lane_item();
    world.transform.get_mut(hero).expect("standing").pos = fountain_pos(world.map, Team::Dire);
    let scroll =
        crate::game::ItemStack::bought(ItemId::TOWN_PORTAL_SCROLL, bota_proto::SlotId(0), 0)
            .expect("scroll");
    world.inventory.get_mut(hero).expect("bag").town_portal = Some(Some(scroll));
    let gold = world.seats[0].gold;
    advance_validated(&mut world, None, Order::Sell { slot: ItemSlot(0) });
    assert!(slot_of(&world, hero, 0).is_none());
    assert!(
        world.seats[0].gold > gold,
        "enemy shop sells rather than marking"
    );
    let gold = world.seats[0].gold;
    advance_validated(
        &mut world,
        None,
        Order::Sell {
            slot: ItemSlot(rules::TOWN_PORTAL_SLOT as u8),
        },
    );
    assert!(world.inventory.get(hero).expect("bag").town_portal == Some(None));
    assert!(
        world.seats[0].gold > gold,
        "the special TP slot uses the same sale predicate"
    );
}

#[test]
fn a_disassembled_part_on_the_ground_keeps_history_but_leaves_its_backpack_residence() {
    let (mut world, hero) = lane_item();
    let mut build = fresh_stack(ItemId::BRACER, 0);
    build.disassemble_until = world.tick + rules::DISASSEMBLY_WINDOW_TICKS;
    // A freshly received build can be disassembled before the next upkeep.
    for slot in 0..9 {
        world.inventory.get_mut(hero).expect("full bag").slots[slot] =
            Some(fresh_stack(ItemId::CIRCLET, 0));
    }
    world.inventory.get_mut(hero).expect("bag").slots[6] = Some(build);
    advance_validated(&mut world, None, Order::Disassemble { slot: ItemSlot(6) });
    let ground = on_the_ground(&world);
    assert_eq!(ground.len(), 2, "two components spill from the full bag");
    assert!(
        !world
            .inventory
            .get(hero)
            .expect("bag")
            .held()
            .any(|stack| stack.id == ItemId::BRACER)
    );
    for entity in ground {
        let part = world.loot.get(entity).expect("component on ground").0;
        assert!(
            part.backpack.used(bota_proto::SlotId(0)),
            "consumed first-entry grant follows every component"
        );
        assert_eq!(
            part.backpack.left(world.tick),
            0,
            "a ground item has no backpack residence"
        );
    }
}

#[test]
fn a_charge_recipe_inherits_the_consumed_parts_backpack_history() {
    let (mut world, hero) = lane_item();
    let mut lotuses = crate::game::ItemStack::granted(
        ItemId::HEALING_LOTUS,
        bota_proto::SlotId(0),
        world.tick,
        3,
    );
    lotuses.backpack.enter(bota_proto::SlotId(0), world.tick);
    lotuses.backpack.leave();
    world.inventory.get_mut(hero).expect("bag").slots[0] = Some(lotuses);
    world.step();
    let built = slot_of(&world, hero, 0).expect("combined lotus");
    assert_eq!(built.id, ItemId::GREAT_HEALING_LOTUS);
    assert!(
        built.backpack.used(bota_proto::SlotId(0)),
        "consuming charges also inherits first-entry history"
    );
    swap(&mut world, 0, 6);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned lotus").mute,
        rules::BACKPACK_MUTE_TICKS - 1
    );
}

#[test]
fn a_failed_give_to_a_full_bag_keeps_the_original_backpack_grace_deadline() {
    let mut cfg = config();
    cfg.picks.push(bota_proto::Pick {
        slot: bota_proto::SlotId(1),
        team: Team::Radiant,
        hero: bota_proto::HeroId::PUDGE,
    });
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("sender");
    let ally = world.seats[1].unit.expect("full recipient");
    for unit in [hero, ally] {
        world.transform.get_mut(unit).expect("standing").pos = Vec2::from_ints(8000, 8000);
    }
    for slot in 0..9 {
        world.inventory.get_mut(ally).expect("full bag").slots[slot] =
            Some(fresh_stack(ItemId::CIRCLET, 0));
    }
    hand_item(&mut world, hero, ItemId::BOOTS, 0);
    swap(&mut world, 0, 6);
    let before = slot_of(&world, hero, 6)
        .expect("backpack item")
        .backpack
        .left(world.tick);
    assert!(before > 1, "first-entry grace active");
    assert!(world.put_item(
        hero,
        6,
        bota_proto::Target::Unit(crate::game::wire_id(ally))
    ));
    world.step();
    let held = slot_of(&world, hero, 6).expect("failed give leaves the item in place");
    assert_eq!(held.id, ItemId::BOOTS);
    assert_eq!(
        held.backpack.left(world.tick),
        before - 1,
        "a failed give does not end residence"
    );
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned").mute,
        0,
        "the original grace still avoids inactivity"
    );
}

#[test]
fn a_fresh_component_cannot_renew_another_components_exhausted_backpack_grace() {
    let (mut world, hero) = lane_item();
    world.inventory.get_mut(hero).expect("bag").slots[0] = None;
    for (slot, id) in [
        (6, ItemId::CIRCLET),
        (7, ItemId::GAUNTLETS),
        (8, ItemId::RECIPE_BRACER),
    ] {
        let mut part = fresh_stack(id, 0);
        if slot == 7 {
            part.backpack.enter(bota_proto::SlotId(0), world.tick);
            part.backpack.leave();
        }
        world.inventory.get_mut(hero).expect("bag").slots[slot] = Some(part);
    }
    world.settle();
    assert!(
        slot_of(&world, hero, 6)
            .expect("fresh component")
            .backpack
            .left(world.tick)
            > 0
    );
    assert_eq!(
        slot_of(&world, hero, 7)
            .expect("used component")
            .backpack
            .left(world.tick),
        0
    );
    world.step();
    let build = slot_of(&world, hero, 6).expect("build in lowest consumed slot");
    assert_eq!(build.id, ItemId::BRACER);
    assert_eq!(
        build.backpack.left(world.tick),
        0,
        "a new build inherits the exhausted input's restriction"
    );
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned build").mute,
        rules::BACKPACK_MUTE_TICKS - 1
    );
}

#[test]
fn a_build_keeps_the_earliest_active_grace_deadline_of_its_components() {
    let (mut world, hero) = lane_item();
    for slot in 0..6 {
        world.inventory.get_mut(hero).expect("bag").slots[slot] =
            Some(fresh_stack(ItemId::BOOTS, 0));
    }
    world.inventory.get_mut(hero).expect("bag").slots[7] = Some(fresh_stack(ItemId::GAUNTLETS, 0));
    world.inventory.get_mut(hero).expect("bag").slots[8] =
        Some(fresh_stack(ItemId::RECIPE_BRACER, 0));
    world.settle();
    for _ in 0..10 {
        world.step();
    }
    let first = slot_of(&world, hero, 7)
        .expect("older backpack component")
        .backpack
        .left(world.tick);
    assert!(first > 2);
    world.transform.get_mut(hero).expect("standing").pos = fountain_pos(world.map, Team::Radiant);
    // The newly bought part lands in free slot6 and combines in the same
    // upkeep, before refresh_stats can observe it as a separate stack.
    advance_validated(
        &mut world,
        None,
        Order::Buy {
            item: ItemId::CIRCLET,
        },
    );
    let build = slot_of(&world, hero, 6).expect("assembled");
    assert_eq!(build.id, ItemId::BRACER);
    assert_eq!(
        build.backpack.left(world.tick),
        first - 1,
        "assembly preserves valid grace without extending its oldest deadline"
    );
    world.transform.get_mut(hero).expect("standing").pos = Vec2::from_ints(8000, 8000);
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned build").mute,
        0,
        "unexpired inherited grace remains usable"
    );
}

#[test]
fn a_new_ward_dispenser_cannot_regain_grace_from_a_fresh_component() {
    let (mut world, hero) = lane_item();
    let mut sentry = fresh_stack(ItemId::SENTRY_WARD, 1);
    sentry.backpack.enter(bota_proto::SlotId(0), world.tick);
    sentry.backpack.leave();
    world.inventory.get_mut(hero).expect("bag").slots[6] =
        Some(fresh_stack(ItemId::OBSERVER_WARD, 1));
    world.inventory.get_mut(hero).expect("bag").slots[7] = Some(sentry);
    world.settle();
    assert!(
        slot_of(&world, hero, 6)
            .expect("fresh Observer")
            .backpack
            .left(world.tick)
            > 0
    );
    world.step();
    let dispenser = slot_of(&world, hero, 6).expect("combined wards");
    assert_eq!(dispenser.id, ItemId::WARD_DISPENSER);
    assert_eq!((dispenser.charges, dispenser.sentries), (1, 1));
    assert!(slot_of(&world, hero, 7).is_none());
    assert_eq!(
        dispenser.backpack.left(world.tick),
        0,
        "changing kind inherits the exhausted input's restriction"
    );
    swap(&mut world, 6, 0);
    assert_eq!(
        slot_of(&world, hero, 0).expect("returned dispenser").mute,
        rules::BACKPACK_MUTE_TICKS - 1
    );
}

#[test]
fn adding_ward_charges_preserves_a_fresh_residents_first_entry_grace() {
    let (mut world, hero) = lane_item();
    let mut sentry = fresh_stack(ItemId::SENTRY_WARD, 1);
    sentry.backpack.enter(bota_proto::SlotId(0), world.tick);
    sentry.backpack.leave();
    world.inventory.get_mut(hero).expect("bag").slots[6] =
        Some(fresh_stack(ItemId::WARD_DISPENSER, 1));
    world.inventory.get_mut(hero).expect("bag").slots[7] = Some(sentry);
    // A fresh arrival must be observed before automatic ward combination
    // folds the incoming stack's history into the unchanged resident.
    assert!(
        !slot_of(&world, hero, 6)
            .expect("fresh resident")
            .backpack
            .used(bota_proto::SlotId(0))
    );
    world.step();
    let dispenser = slot_of(&world, hero, 6).expect("same dispenser");
    assert_eq!(dispenser.id, ItemId::WARD_DISPENSER);
    assert_eq!((dispenser.charges, dispenser.sentries), (1, 1));
    assert_eq!(
        dispenser.backpack.left(world.tick),
        rules::BACKPACK_GRACE_TICKS,
        "the fresh arrival's first visit begins in this upkeep"
    );
    world.step();
    assert_eq!(
        slot_of(&world, hero, 6)
            .expect("unchanged resident")
            .backpack
            .left(world.tick),
        rules::BACKPACK_GRACE_TICKS - 1,
        "a later upkeep never restarts residence"
    );
    swap(&mut world, 6, 0);
    assert_eq!(slot_of(&world, hero, 0).expect("returned resident").mute, 0);
}
