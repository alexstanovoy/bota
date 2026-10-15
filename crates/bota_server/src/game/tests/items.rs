//! Consumables, active items and trees.

use bota_core::{HERO, MELEE_CREEP, ModifierKind, SHADOW_FIEND, map_of, rules};

use crate::game::{
    BLINK_RANGE, Event, ITEM_BLINK_DAGGER, ITEM_CLARITY, ITEM_HEALING_SALVE, ITEM_IRON_BRANCH,
    ITEM_PHASE_BOOTS, ITEM_POWER_TREADS, ITEM_QUELLING_BLADE, ITEM_TANGO, ITEMS, ItemStack, Mana,
    SALVE_TICKS, World, tree_positions, wire_id,
};
use bota_proto::{
    Attribute, DamageKind, EventKind, Fixed, HeroId, ItemId, MapId, SlotId, Target, Team, UnitKind,
    Vec2,
};

use super::support::*;

#[test]
fn a_salve_puts_mending_on_whoever_drinks_it_and_runs_out() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    hand_item(&mut world, hero, ITEM_HEALING_SALVE, 1);
    assert!(
        world.use_item(hero, 0, Target::None, &mut Vec::new()),
        "it drinks"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the last charge takes the stack with it"
    );
    world.step();
    let plain = HERO.hp_regen + rules::HP_REGEN_PER_STRENGTH * SHADOW_FIEND.attributes.strength;
    assert!(
        world.stats.get(hero).expect("settled").hp_regen > plain,
        "it mends faster while the salve holds"
    );
    for _ in 0..SALVE_TICKS + 1 {
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
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    hand_item(&mut world, hero, ITEM_HEALING_SALVE, 1);
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, Target::None, &mut events),
        "a full hero still drinks"
    );
    let told = |events: &[Event]| {
        events.iter().find_map(|event| match event.kind {
            EventKind::Healed { target, amount, .. } => Some((target, amount)),
            _ => None,
        })
    };
    assert_eq!(told(&events), None, "with nothing missing, nothing is told");

    world.health.get_mut(hero).expect("standing").hp -= Fixed::from_int(150);
    hand_item(&mut world, hero, ITEM_HEALING_SALVE, 1);
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, Target::None, &mut events),
        "a hurt one drinks"
    );
    assert_eq!(
        told(&events),
        Some((wire_id(hero), 150)),
        "and the mending is told at what was missing, not the whole drink"
    );
}

#[test]
fn a_clarity_is_told_of_by_the_mana_that_was_missing() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    world.mana.get_mut(hero).expect("has a pool").mana -= Fixed::from_int(60);
    hand_item(&mut world, hero, ITEM_CLARITY, 1);
    let mut events = Vec::new();
    assert!(
        world.use_item(hero, 0, Target::None, &mut events),
        "it drinks"
    );
    let told = events.iter().find_map(|event| match event.kind {
        EventKind::Healed { amount, mana, .. } => Some((amount, mana)),
        _ => None,
    });
    assert_eq!(
        told,
        Some((0, 60)),
        "the mending says the mana that was missing, and no health at all"
    );
}

#[test]
fn a_drink_may_be_aimed_at_the_one_drinking_it() {
    for item in [ITEM_HEALING_SALVE, ITEM_CLARITY] {
        let (mut world, hero) = a_hero_with_gold(0);
        hand_item(&mut world, hero, item, 1);
        world.step();
        // Aimed at itself by name, the way a click on one's own hero sends it.
        assert!(
            world.use_item(hero, 0, Target::Unit(wire_id(hero)), &mut Vec::new()),
            "item {item} may be drunk by whoever holds it"
        );
        assert!(
            world.modifiers.get(hero).is_some_and(|on_it| on_it
                .active()
                .any(|held| !matches!(held.kind, ModifierKind::Fountain { .. }))),
            "item {item} leaves its effect on the one who drank it"
        );
    }
}

#[test]
fn a_quelling_blade_takes_a_tree_down_and_the_tree_comes_back() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ITEM_QUELLING_BLADE, 0);
    assert!(sight_stopped_at(&world, tree), "the tree stops a look");
    assert!(
        world.use_item(hero, 0, Target::Pos(tree), &mut Vec::new()),
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
    let (mut world, hero, tree) = a_hero_by_the_trees(ITEM_TANGO, 3);
    let far = tree + Vec2::from_ints(4000, 0);
    assert!(
        !world.use_item(hero, 0, Target::Pos(far), &mut Vec::new()),
        "aimed where no tree stands it does nothing"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|s| s.charges),
        Some(3),
        "and spends no charge on it"
    );
    assert!(world.use_item(hero, 0, Target::Pos(tree), &mut Vec::new()));
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
            ModifierKind::Mending {
                per_tick: 0,
                breaks: false
            }
        ),
        "the one who ate it mends"
    );
}

#[test]
fn a_branch_puts_a_tree_up_and_eating_that_one_feeds_twice_as_long() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ITEM_IRON_BRANCH, 1);
    let spot = tree + Vec2::from_ints(240, 0);
    assert!(
        world.use_item(hero, 0, Target::Pos(spot), &mut Vec::new()),
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
    let (mut world, hero, tree) = a_hero_by_the_trees(ITEM_IRON_BRANCH, 1);
    let spot = tree + Vec2::from_ints(240, 0);
    assert!(world.use_item(hero, 0, Target::Pos(spot), &mut Vec::new()));
    for _ in 0..rules::PLANTED_TREE_TICKS + 1 {
        world.step();
    }
    assert!(world.trees.planted().is_empty(), "in time it goes");
    assert!(!sight_stopped_at(&world, spot), "and stops nothing");
}

#[test]
fn a_blade_takes_only_the_tree_it_was_pointed_at_and_only_within_reach() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ITEM_QUELLING_BLADE, 0);
    let stand = world.transform.get(hero).expect("hero").pos;
    // Open ground a step off the trunk, still well inside the blade's reach.
    let beside = tree + Vec2::from_ints(rules::TREE_RADIUS + 20, 0);
    assert!(
        !world.use_item(hero, 0, Target::Pos(beside), &mut Vec::new()),
        "pointed at ground beside a tree it takes nothing"
    );
    world.transform.get_mut(hero).expect("hero").pos = tree + Vec2::from_ints(2000, 0);
    assert!(
        !world.use_item(hero, 0, Target::Pos(tree), &mut Vec::new()),
        "the tree is the one pointed at, but it is out of reach"
    );
    assert_eq!(world.trees.felled().count(), 0);
    world.transform.get_mut(hero).expect("hero").pos = stand;
    let on_it = tree + Vec2::from_ints(rules::TREE_RADIUS - 10, 0);
    assert!(
        world.use_item(hero, 0, Target::Pos(on_it), &mut Vec::new()),
        "pointed at the trunk it takes that tree"
    );
    assert_eq!(world.trees.felled().count(), 1);
}

#[test]
fn a_quelling_blade_is_worth_something_against_a_creep_and_nothing_against_a_hero() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    let theirs = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5000, 5100),
        SlotId(1),
        HeroId(1),
    );
    world.settle();
    world.step();
    let bare_creep = one_swing_takes(&mut world, hero, creep);
    let bare_hero = one_swing_takes(&mut world, hero, theirs);
    hand_item(&mut world, hero, ITEM_QUELLING_BLADE, 0);
    world.step();
    let blade = ITEMS[usize::from(ITEM_QUELLING_BLADE)]
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
fn a_hero_hit_loses_its_drink_but_never_what_a_tree_bought() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    let hero = hero_at(&mut world, Vec2::from_ints(7000, 7000));
    let theirs = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(7000, 7000),
        SlotId(1),
        HeroId(1),
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(7000, 7000));
    world.settle();
    world.step();
    let salve = ModifierKind::Mending {
        per_tick: 0,
        breaks: false,
    };
    // A creep may hit all day and the drink holds.
    hand_item(&mut world, hero, ITEM_HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    world.push_hit(Some(creep), hero, 10, DamageKind::Physical);
    world.step();
    assert!(carries(&world, hero, salve), "a creep does not break it");
    // A hero's blow puts it out.
    world.push_hit(Some(theirs), hero, 10, DamageKind::Physical);
    world.step();
    assert!(!carries(&world, hero, salve), "a hero does");
    // What a tree bought is broken by nothing.
    let tree = tree_positions(map, crate::game::terrain_of(map))
        .into_iter()
        .find(|at| at.within(Vec2::from_ints(7000, 7000), Fixed::from_int(4000)))
        .expect("the forest reaches here");
    world.transform.get_mut(hero).expect("hero").pos = tree + Vec2::from_ints(120, 0);
    hand_item(&mut world, hero, ITEM_TANGO, 1);
    assert!(world.use_item(hero, 0, Target::Pos(tree), &mut Vec::new()));
    world.push_hit(Some(theirs), hero, 10, DamageKind::Physical);
    world.step();
    assert!(
        carries(&world, hero, salve),
        "a tango holds through a hero's blow"
    );
}

#[test]
fn what_an_item_is_set_to_is_worth_points_of_that_attribute() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = ItemId(ITEM_POWER_TREADS);
    let mut events = Vec::new();
    assert!(world.buy(SlotId(0), treads, &mut events), "bought");
    world.step();
    let bonus = Fixed::from_int(ITEMS[usize::from(ITEM_POWER_TREADS)].mode_bonus);
    let on_strength = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Strength),
        "they come set to strength"
    );
    assert!(
        world.use_item(hero, 0, Target::None, &mut Vec::new()),
        "and using them sets them over"
    );
    world.step();
    let on_agility = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Agility),
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
    let hero = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId(1));
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(ItemStack {
        mode: Some(Attribute::Strength),
        ..fresh_stack(ITEM_POWER_TREADS, 0)
    });
    world.settle();
    world.step();
    world.health.insert(hero, health(100));
    world.mana.insert(
        hero,
        Mana {
            mana: Fixed::from_int(30),
        },
    );
    let switches = 12;
    for _ in 0..switches {
        assert!(
            world.use_item(hero, 0, Target::None, &mut Vec::new()),
            "switched"
        );
        world.step();
    }
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Strength),
        "four full turns of the wheel end where they began"
    );
    // What the pools may gain over the wheel is what regeneration mends and
    // not a drop more, bounded by the highest rate any mode pays.
    let bonus = Fixed::from_int(ITEMS[usize::from(ITEM_POWER_TREADS)].mode_bonus);
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
    hand_item(&mut world, hero, ITEM_BLINK_DAGGER, 0);
    world.step();
    let far = from + Vec2::from_ints(BLINK_RANGE * 4, 0);
    assert!(
        world.use_item(hero, 0, Target::Pos(far), &mut Vec::new()),
        "it goes"
    );
    let landed = world.transform.get(hero).expect("stands somewhere").pos;
    assert!(landed != from, "and it carried");
    assert!(
        landed.within(from, rules::units(BLINK_RANGE)),
        "no further than it reaches"
    );
}

#[test]
fn a_blink_aimed_at_closed_ground_steps_back_to_open() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, ITEM_BLINK_DAGGER, 0);
    world.step();
    let aim = from + Vec2::from_ints(600, 0);
    stand_a_wall(&mut world, aim, rules::units(rules::BLINK_STEP_BACK * 2));
    assert!(
        world.use_item(hero, 0, Target::Pos(aim), &mut Vec::new()),
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
    hand_item(&mut world, hero, ITEM_BLINK_DAGGER, 0);
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.cooldown),
        Some(0),
        "it is ready"
    );
    let hitter = world.spawn();
    world.kind.insert(hitter, UnitKind::Hero);
    world.push_hit(Some(hitter), hero, 10, DamageKind::Physical);
    world.step();
    let wait = ITEMS[usize::from(ITEM_BLINK_DAGGER)].breaks_on_damage;
    assert!(
        slot_of(&world, hero, 0)
            .map(|stack| stack.cooldown)
            .is_some_and(|left| left > 0 && left <= wait),
        "and a blow from a hero sets it back"
    );
}

#[test]
fn phase_walks_a_body_through_another() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, ITEM_PHASE_BOOTS, 0);
    world.step();
    assert!(
        !world.stats.get(hero).expect("settled").phased,
        "it walks round what is in the way until the boots are used"
    );
    assert!(
        world.use_item(hero, 0, Target::None, &mut Vec::new()),
        "it goes"
    );
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").phased,
        "and then walks through it"
    );
}

#[test]
fn the_view_reports_the_regeneration_the_server_applies() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let regen = |world: &World| {
        let unit = world
            .view(Team::Radiant)
            .units
            .into_iter()
            .find(|seen| seen.id == wire_id(hero))
            .expect("in sight");
        (unit.hp_regen, unit.mana_regen)
    };
    let served = |world: &World| {
        let stats = world.stats.get(hero).expect("settled");
        let per_second = |per_tick: Fixed| per_tick.raw * rules::TICKS_PER_SECOND as i32;
        (per_second(stats.hp_regen), per_second(stats.mana_regen))
    };
    world.step();
    let (fountain_hp, fountain_mana) = regen(&world);
    assert_eq!((fountain_hp.raw, fountain_mana.raw), served(&world));

    world.transform.get_mut(hero).expect("hero").pos = Vec2::from_ints(5000, 5000);
    for _ in 0..=rules::TICKS_PER_SECOND {
        world.step();
    }
    let (plain_hp, plain_mana) = regen(&world);
    assert_eq!((plain_hp.raw, plain_mana.raw), served(&world));
    assert!(fountain_hp > plain_hp && fountain_mana > plain_mana);

    hand_item(&mut world, hero, ITEM_HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    world.step();
    let (salved, _) = regen(&world);
    assert_eq!(salved.raw, served(&world).0);
    assert!(salved > plain_hp, "the salve shows in the view");
}
