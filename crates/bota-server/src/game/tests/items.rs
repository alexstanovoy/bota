//! Consumables, active items and trees.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, Mana, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_salve_puts_mending_on_whoever_drinks_it_and_runs_out() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.step();
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE),
            charges: 1,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
        "it drinks"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the last charge takes the stack with it"
    );
    world.step();
    let plain = crate::game::HERO.hp_regen
        + rules::HP_REGEN_PER_STRENGTH * crate::game::HERO.attributes.strength;
    assert!(
        world.stats.get(hero).expect("settled").hp_regen > plain,
        "it mends faster while the salve holds"
    );
    for _ in 0..salve_ticks() + 1 {
        world.step();
    }
    assert_eq!(
        world.stats.get(hero).map(|s| s.hp_regen),
        Some(plain),
        "and back to its own once it runs out"
    );
}

#[test]
fn a_drink_is_told_of_as_what_was_missing_and_a_full_hero_is_not_told_at_all() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.step();
    let salve = crate::game::ItemStack {
        id: bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE),
        charges: 1,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    };
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(salve);
    }
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut events),
        "a full hero still drinks"
    );
    let told = |events: &[crate::game::Event]| {
        events.iter().find_map(|event| match event.kind {
            bota_proto::EventKind::Healed { target, amount, .. } => Some((target, amount)),
            _ => None,
        })
    };
    assert_eq!(told(&events), None, "with nothing missing, nothing is told");

    if let Some(health) = world.health.get_mut(hero) {
        health.hp -= Fixed::from_int(150);
    }
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(salve);
    }
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut events),
        "a hurt one drinks"
    );
    assert_eq!(
        told(&events),
        Some((crate::game::wire_id(hero), 150)),
        "and the mending is told at what was missing, not the whole drink"
    );
}

#[test]
fn a_clarity_is_told_of_by_the_mana_that_was_missing() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.step();
    if let Some(pool) = world.mana.get_mut(hero) {
        pool.mana -= Fixed::from_int(60);
    }
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_CLARITY),
            charges: 1,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut events),
        "it drinks"
    );
    let told = events.iter().find_map(|event| match event.kind {
        bota_proto::EventKind::Healed { amount, mana, .. } => Some((amount, mana)),
        _ => None,
    });
    assert_eq!(
        told,
        Some((0, 60)),
        "the mending says the mana that was missing, and no health at all"
    );
}

#[test]
fn a_scroll_carries_its_user_once_the_channel_runs_out() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let from = world.transform.get(hero).expect("standing").pos;
    // Told to walk somewhere before it read the scroll: the order stays
    // behind with the spot it was given in.
    world.set_order(
        hero,
        crate::game::UnitOrder::Move {
            pos: bota_proto::Vec2::from_ints(8000, 12000),
        },
    );
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(to), &mut Vec::new()),
        "beside a building of its own it may go"
    );
    assert!(world.is_channelling(hero), "and stands through the channel");
    let projected = world
        .view(bota_proto::Team::Radiant)
        .units
        .into_iter()
        .find(|unit| unit.id == crate::game::wire_id(hero))
        .expect("channelled hero is projected");
    assert_ne!(
        projected.statuses.bits & bota_proto::StatusFlags::CHANNELLING,
        0,
        "channel state crosses the seat-visible protocol"
    );
    world.step();
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(from),
        "still where it was while it channels"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and the scroll is not spent yet"
    );
    for _ in 0..88 {
        world.step();
    }
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(from),
        "and one tick short of the channel it has not moved"
    );
    world.step();
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(to),
        "then it is there"
    );
    for _ in 0..30 {
        world.step();
    }
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(to),
        "and stays there rather than walking back to what it was told"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "and the scroll went with it"
    );
}

#[test]
fn a_scroll_aimed_where_nothing_of_its_own_stands_does_nothing() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let nowhere = bota_proto::Vec2::from_ints(14000, 9216);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(nowhere), &mut Vec::new()),
        "the middle of the map is nothing to go to"
    );
    assert!(!world.is_channelling(hero));
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on it"
    );
}

#[test]
fn an_order_takes_a_channel_away_and_leaves_the_scroll() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to), &mut Vec::new()));
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert!(!world.is_channelling(hero), "the order took it away");
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and the scroll is still there to use again"
    );
}

#[test]
fn business_with_the_bag_and_the_shop_takes_no_channel_away() {
    let (mut world, hero) = a_hero_with_a_scroll();
    world.seats[0].gold = 5000;
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to), &mut Vec::new()));
    let asks = [
        bota_proto::Order::Buy {
            item: bota_proto::ItemId(crate::game::ITEM_BOOTS),
        },
        bota_proto::Order::Learn {
            slot: bota_proto::AbilitySlot(0),
        },
        bota_proto::Order::Sell {
            slot: bota_proto::ItemSlot(9),
        },
    ];
    for order in asks {
        world.advance(&[crate::game::Command {
            slot: bota_proto::SlotId(0),
            unit: None,
            order,
        }]);
        assert!(
            world.is_channelling(hero),
            "the channel stands through {order:?}"
        );
    }
}

#[test]
fn a_drink_may_be_aimed_at_the_one_drinking_it() {
    for item in [crate::game::ITEM_HEALING_SALVE, crate::game::ITEM_CLARITY] {
        let mut world = World::for_match(&config(), config().rng());
        let hero = world.seats[0].unit.expect("stood up");
        if let Some(bag) = world.inventory.get_mut(hero) {
            bag.slots[0] = Some(crate::game::ItemStack {
                id: bota_proto::ItemId(item),
                charges: 1,
                cooldown: 0,
                mute: 0,
                mode: None,
                bought_tick: 0,
                touched: false,
                owner: bota_proto::SlotId(0),
                for_sale: false,
            });
        }
        world.step();
        // Aimed at itself by name, the way a click on one's own hero sends it.
        assert!(
            world.use_item(
                hero,
                0,
                bota_proto::Target::Unit(crate::game::wire_id(hero)),
                &mut Vec::new()
            ),
            "item {item} may be drunk by whoever holds it"
        );
        assert!(
            world.modifiers.get(hero).is_some_and(|on_it| on_it
                .active()
                .any(|held| !matches!(held.kind, crate::game::ModifierKind::Fountain { .. }))),
            "item {item} leaves its effect on the one who drank it"
        );
    }
}

#[test]
fn a_quelling_blade_takes_a_tree_down_and_the_tree_comes_back() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    assert!(sight_stopped_at(&world, tree), "the tree stops a look");
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(tree), &mut Vec::new()),
        "the blade reaches it"
    );
    assert_eq!(world.trees.felled().count(), 1, "one tree is down");
    assert!(
        !sight_stopped_at(&world, tree),
        "and what it stopped is stopped no longer"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "the blade itself is not spent on it"
    );
    for _ in 0..rules::TREE_REGROW_TICKS + 1 {
        world.step();
    }
    assert_eq!(world.trees.felled().count(), 0, "in time it comes back");
    assert!(sight_stopped_at(&world, tree), "and stops a look again");
}

#[test]
fn a_tango_eats_a_tree_and_without_one_eats_nothing() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_TANGO, 3);
    let far = tree + bota_proto::Vec2::from_ints(4000, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(far), &mut Vec::new()),
        "aimed where no tree stands it does nothing"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|s| s.charges),
        Some(3),
        "and spends no charge on it"
    );
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(tree), &mut Vec::new()));
    assert_eq!(world.trees.felled().count(), 1, "the tree it ate is gone");
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|s| s.charges),
        Some(2),
        "and one charge with it"
    );
    assert!(
        carries(
            &world,
            hero,
            crate::game::ModifierKind::Mending {
                per_tick: 0,
                breaks: false
            }
        ),
        "the one who ate it mends"
    );
}

#[test]
fn a_branch_puts_a_tree_up_and_eating_that_one_feeds_twice_as_long() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_IRON_BRANCH, 1);
    let spot = tree + bota_proto::Vec2::from_ints(240, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(spot), &mut Vec::new()),
        "the branch goes into open ground"
    );
    assert_eq!(world.trees.planted().len(), 1, "and a tree stands there");
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the branch is spent on it"
    );
    assert!(
        sight_stopped_at(&world, spot),
        "what it put up stops a look"
    );
    // The same tango, eaten off the map's own tree and off a planted one.
    let plain = tango_ticks(&mut world, hero, tree);
    let put_up = tango_ticks(&mut world, hero, spot);
    assert_eq!(put_up, plain * 2, "a tree put up feeds twice as long");
    // What is left of it goes on its own.
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_IRON_BRANCH, 1);
    let spot = tree + bota_proto::Vec2::from_ints(240, 0);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(spot), &mut Vec::new()));
    for _ in 0..rules::PLANTED_TREE_TICKS + 1 {
        world.step();
    }
    assert!(world.trees.planted().is_empty(), "in time it goes");
    assert!(!sight_stopped_at(&world, spot), "and stops nothing");
}

#[test]
fn a_blade_takes_the_tree_it_was_pointed_at_and_no_other() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    // Open ground a step off the trunk, still well inside the blade's reach.
    let beside = tree + bota_proto::Vec2::from_ints(rules::TREE_RADIUS + 20, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(beside), &mut Vec::new()),
        "pointed at ground beside a tree it takes nothing"
    );
    assert_eq!(world.trees.felled().count(), 0);
    let on_it = tree + bota_proto::Vec2::from_ints(rules::TREE_RADIUS - 10, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(on_it), &mut Vec::new()),
        "pointed at the trunk it takes that tree"
    );
    assert_eq!(world.trees.felled().count(), 1);
}

#[test]
fn a_blade_cannot_reach_a_tree_it_was_pointed_at_from_far_off() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    world.transform.get_mut(hero).expect("hero").pos = tree + bota_proto::Vec2::from_ints(2000, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(tree), &mut Vec::new()),
        "the tree is the one pointed at, but it is out of reach"
    );
    assert_eq!(world.trees.felled().count(), 0);
}

#[test]
fn a_quelling_blade_is_worth_something_against_a_creep_and_nothing_against_a_hero() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000, 5100),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    let bare_creep = one_swing_takes(&mut world, hero, creep);
    let bare_hero = one_swing_takes(&mut world, hero, theirs);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_QUELLING_BLADE),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    world.step();
    let blade = crate::game::ITEMS[usize::from(crate::game::ITEM_QUELLING_BLADE)]
        .carried
        .damage_to_creeps;
    assert_eq!(
        world.stats.get(hero).map(|s| s.damage_to_creeps),
        Some(blade),
        "what it carries against creeps is worked out"
    );
    let with_creep = one_swing_takes(&mut world, hero, creep);
    let with_hero = one_swing_takes(&mut world, hero, theirs);
    // Armor takes its share of the blade as it does of everything else, so
    // what lands is somewhere between one point and the whole of it.
    let felt = with_creep - bare_creep;
    assert!(
        (1..=blade).contains(&felt),
        "the creep feels the blade: {bare_creep} then {with_creep}, of {blade} carried"
    );
    // A hero mends while it is being measured, so what it took can read a
    // point light; what it must not do is read heavier.
    assert!(
        with_hero <= bare_hero,
        "the hero feels none of it: {bare_hero} then {with_hero}"
    );
}

#[test]
fn a_scroll_read_is_owed_by_the_hero_and_not_by_the_scroll() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to), &mut Vec::new()));
    for _ in 0..91 {
        world.step();
    }
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the scroll went with the teleport"
    );
    // A fresh scroll, bought after the first was spent, is still on the wait.
    hand_item(&mut world, hero, crate::game::ITEM_TOWN_PORTAL_SCROLL, 1);
    let there = beside_own_tower(&world);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(there), &mut Vec::new()),
        "a new scroll does not buy a new wait"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on the attempt"
    );
    for _ in 0..rules::SCROLL_WAIT_TICKS {
        world.step();
    }
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(there), &mut Vec::new()),
        "once the wait is out it reads again"
    );
}

#[test]
fn a_hero_hit_loses_its_drink_but_never_what_a_tree_bought() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(7000, 7000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 7000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 7000),
    );
    world.settle();
    world.step();
    let salve = crate::game::ModifierKind::Mending {
        per_tick: 0,
        breaks: false,
    };
    // A creep may hit all day and the drink holds.
    hand_item(&mut world, hero, crate::game::ITEM_HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()));
    world.push_hit(Some(creep), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(carries(&world, hero, salve), "a creep does not break it");
    // A hero's blow puts it out.
    world.push_hit(Some(theirs), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(!carries(&world, hero, salve), "a hero does");
    // What a tree bought is broken by nothing.
    let tree = crate::game::tree_positions(map)
        .into_iter()
        .find(|at| {
            at.within(
                bota_proto::Vec2::from_ints(7000, 7000),
                bota_proto::Fixed::from_int(4000),
            )
        })
        .expect("the forest reaches here");
    world.transform.get_mut(hero).expect("hero").pos = tree + bota_proto::Vec2::from_ints(120, 0);
    hand_item(&mut world, hero, crate::game::ITEM_TANGO, 1);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(tree), &mut Vec::new()));
    world.push_hit(Some(theirs), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(
        carries(&world, hero, salve),
        "a tango holds through a hero's blow"
    );
}

#[test]
fn what_an_item_is_set_to_is_worth_points_of_that_attribute() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought"
    );
    world.step();
    let bonus =
        Fixed::from_int(crate::game::ITEMS[usize::from(crate::game::ITEM_POWER_TREADS)].mode_bonus);
    let on_strength = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Strength),
        "they come set to strength"
    );
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
        "and using them sets them over"
    );
    world.step();
    let on_agility = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Agility),
        "to the attribute after the one they were on"
    );
    assert_eq!(
        on_agility.attributes.strength + bonus,
        on_strength.attributes.strength,
        "what they were worth in strength is gone"
    );
    assert_eq!(
        on_agility.attributes.agility,
        on_strength.attributes.agility + bonus,
        "and worth the same in agility instead"
    );
}

#[test]
fn treads_switched_round_the_wheel_mend_nothing() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_POWER_TREADS),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: Some(bota_proto::Attribute::Strength),
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    world.settle();
    world.step();
    world.health.insert(
        hero,
        Health {
            hp: Fixed::from_int(100),
        },
    );
    world.mana.insert(
        hero,
        Mana {
            mana: Fixed::from_int(30),
        },
    );
    let switches = 12;
    for _ in 0..switches {
        assert!(
            world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
            "switched"
        );
        world.step();
    }
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Strength),
        "four full turns of the wheel end where they began"
    );
    // What the pools may gain over the wheel is what regeneration mends and
    // not a drop more, bounded by the highest rate any mode pays.
    let bonus =
        Fixed::from_int(crate::game::ITEMS[usize::from(crate::game::ITEM_POWER_TREADS)].mode_bonus);
    let ticks = Fixed::from_int(switches);
    let mended = (rules::HERO_HP_REGEN
        + rules::HP_REGEN_PER_STRENGTH * (rules::HERO_ATTRIBUTES.strength + bonus))
        * ticks;
    assert!(
        world.health.get(hero).expect("standing").hp <= Fixed::from_int(100) + mended,
        "no health is minted"
    );
    let cleared = (rules::HERO_MANA_REGEN
        + rules::MANA_REGEN_PER_INTELLIGENCE * (rules::HERO_ATTRIBUTES.intelligence + bonus))
        * ticks;
    assert!(
        world.mana.get(hero).expect("has a pool").mana <= Fixed::from_int(30) + cleared,
        "and no mana"
    );
}

#[test]
fn a_blink_carries_no_further_than_it_reaches() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    let range = crate::game::BLINK_RANGE;
    let far = bota_proto::Vec2 {
        x: from.x + rules::units(range * 4),
        y: from.y,
    };
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(far), &mut Vec::new()),
        "it goes"
    );
    let landed = world.transform.get(hero).expect("stands somewhere").pos;
    assert!(landed != from, "and it carried");
    assert!(
        landed.within(from, rules::units(range)),
        "no further than it reaches"
    );
}

#[test]
fn a_blink_aimed_at_closed_ground_steps_back_to_open() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    let aim = bota_proto::Vec2 {
        x: from.x + rules::units(600),
        y: from.y,
    };
    stand_a_wall(&mut world, aim, rules::units(rules::BLINK_STEP_BACK * 2));
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(aim), &mut Vec::new()),
        "it goes"
    );
    let landed = world.transform.get(hero).expect("stands somewhere").pos;
    assert!(
        world.clearance.walkable(landed),
        "and it lands on open ground"
    );
    assert!(landed != aim, "short of what it was aimed at");
}

#[test]
fn a_blow_from_a_hero_sets_a_blink_back() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.cooldown),
        Some(0),
        "it is ready"
    );
    let hitter = world.spawn();
    world.kind.insert(hitter, bota_proto::UnitKind::Hero);
    world.push_hit(Some(hitter), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    let wait = crate::game::ITEMS[usize::from(crate::game::ITEM_BLINK_DAGGER)].breaks_on_damage;
    assert!(
        slot_of(&world, hero, 0)
            .map(|stack| stack.cooldown)
            .is_some_and(|left| left > 0 && left <= wait),
        "and a blow from a hero sets it back"
    );
}

#[test]
fn a_magic_stick_gains_charges_and_is_kept_when_it_spends_them() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_MAGIC_STICK, 0);
    world.step();
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
        "with no charge there is nothing to spend"
    );
    if let Some(bag) = world.inventory.get_mut(hero)
        && let Some(stack) = bag.slots[0].as_mut()
    {
        stack.charges = 4;
    }
    if let Some(pool) = world.health.get_mut(hero) {
        pool.hp = Fixed::from_int(100);
    }
    world.step();
    let before = world.health.get(hero).expect("has health").hp;
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
        "with charges it mends"
    );
    assert!(
        world.health.get(hero).expect("has health").hp > before,
        "and health comes back"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.charges == 0),
        "every charge goes at once, and the stack stays"
    );
}

#[test]
fn phase_walks_a_body_through_another() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_PHASE_BOOTS, 0);
    world.step();
    assert!(
        !world.stats.get(hero).expect("settled").phased,
        "it walks round what is in the way until the boots are used"
    );
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()),
        "it goes"
    );
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").phased,
        "and then walks through it"
    );
}
