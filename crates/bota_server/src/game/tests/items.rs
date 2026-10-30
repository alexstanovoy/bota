//! Consumables, active items and trees.

use bota_core::{ITEMS, MELEE_CREEP, ModifierKind, map_of, rules, tower_def};

use crate::game::{Entity, Event, ItemStack, Mana, World, tree_positions, wire_id};
use bota_proto::{
    Attribute, DamageKind, EffectId, EventKind, Fixed, HeroId, Inflictor, ItemId, ItemSlot, MapId,
    Order, SlotId, Target, Team, Vec2,
};

use super::support::*;

#[test]
fn a_drink_is_told_of_as_what_was_missing_and_a_full_hero_is_not_told_at_all() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    hand_item(&mut world, hero, ItemId::HEALING_SALVE, 1);
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
    hand_item(&mut world, hero, ItemId::HEALING_SALVE, 1);
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
fn a_salve_and_a_tango_mend_alike_and_each_names_its_own_item() {
    // Dota shows a Healing Salve's and a Tango's mend as two buffs, each with
    // its item's icon: a view and the mending told name the item.
    for item in [ItemId::HEALING_SALVE, ItemId::TANGO] {
        let (mut world, hero, tree) = a_hero_by_the_trees(item, 1);
        world.health.get_mut(hero).expect("standing").hp -= Fixed::from_int(100);
        let aim = if item == ItemId::TANGO {
            Target::Pos(tree)
        } else {
            Target::None
        };
        let mut events = Vec::new();
        assert!(
            world.use_item(hero, 0, aim, &mut events),
            "item {item:?} is used"
        );
        let told = events.iter().find_map(|event| match event.kind {
            EventKind::Healed { inflictor, .. } => Some(inflictor),
            _ => None,
        });
        assert_eq!(
            told,
            Some(Some(Inflictor::Item(item))),
            "the mending names it"
        );
        let mend = shown(&world, Some(Team::Radiant), hero)
            .effects
            .into_iter()
            .find(|effect| effect.id == EffectId::MENDING)
            .expect("the mend shows");
        assert_eq!(mend.inflictor, Some(Inflictor::Item(item)), "{item:?}");
        assert_eq!(mend.caster, Some(wire_id(hero)), "drunk by its bearer");
        assert!(mend.ticks_total.is_some(), "a mend runs out");
        assert_eq!(mend.ticks_left, mend.ticks_total, "it has only begun");
        world.step();
        let mend = shown(&world, Some(Team::Radiant), hero)
            .effects
            .into_iter()
            .find(|effect| effect.id == EffectId::MENDING)
            .expect("the mend runs on");
        assert_eq!(
            mend.ticks_left.map(|left| left + 1),
            mend.ticks_total,
            "its time runs down from the full length, which stays"
        );
    }
}

#[test]
fn a_clarity_is_told_of_by_the_mana_that_was_missing() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    world.mana.get_mut(hero).expect("has a pool").mana -= Fixed::from_int(60);
    hand_item(&mut world, hero, ItemId::CLARITY, 1);
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
    for item in [ItemId::HEALING_SALVE, ItemId::CLARITY] {
        let (mut world, hero) = a_hero_with_gold(0);
        hand_item(&mut world, hero, item, 1);
        world.step();
        // Aimed at itself by name, the way a click on one's own hero sends it.
        assert!(
            world.use_item(hero, 0, Target::Unit(wire_id(hero)), &mut Vec::new()),
            "item {} may be drunk by whoever holds it",
            item.0
        );
        assert!(
            world.modifiers.get(hero).is_some_and(|on_it| on_it
                .active()
                .any(|held| !matches!(held.kind, ModifierKind::Fountain { .. }))),
            "item {} leaves its effect on the one who drank it",
            item.0
        );
    }
}

#[test]
fn a_quelling_blade_takes_a_tree_down_and_the_tree_comes_back() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::QUELLING_BLADE, 0);
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
    // Not while the hero stands by its spot.
    world.transform.get_mut(hero).expect("hero").pos = tree + Vec2::from_ints(400, 0);
    run(&mut world, rules::TREE_REGROW_TICKS + 1);
    assert_eq!(world.trees.felled().count(), 0, "in time it comes back");
    assert!(sight_stopped_at(&world, tree), "and stops a look again");
}

#[test]
fn a_tango_eats_a_tree_and_without_one_eats_nothing() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::TANGO, 3);
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
                per_tick: Fixed::ZERO,
                breaks: false
            }
        ),
        "the one who ate it mends"
    );
}

#[test]
fn a_branch_puts_a_tree_up_and_eating_that_one_feeds_twice_as_long() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::IRON_BRANCH, 1);
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
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::IRON_BRANCH, 1);
    let spot = tree + Vec2::from_ints(240, 0);
    assert!(world.use_item(hero, 0, Target::Pos(spot), &mut Vec::new()));
    run(&mut world, planted_tree_ticks() + 1);
    assert!(world.trees.planted().is_empty(), "in time it goes");
    assert!(!sight_stopped_at(&world, spot), "and stops nothing");
}

/// Health a hero gains from one Tango beyond what an identical hero gains without it, over a
/// minute: from a tree of the map's own, or from one an Iron Branch put up.
fn health_a_tango_gives(planted: bool) -> i32 {
    let hurt = |world: &mut World, hero: Entity| {
        world.health.get_mut(hero).expect("health").hp = Fixed::from_int(100);
    };
    let (mut fed, hero, tree) = a_hero_by_the_trees(ItemId::IRON_BRANCH, 1);
    let (mut fasting, other, _) = a_hero_by_the_trees(ItemId::IRON_BRANCH, 1);
    // Both put the branch up, so that the two carry the same.
    let spot = tree + Vec2::from_ints(240, 0);
    assert!(fed.use_item(hero, 0, Target::Pos(spot), &mut Vec::new()));
    assert!(fasting.use_item(other, 0, Target::Pos(spot), &mut Vec::new()));
    let eaten = if planted { spot } else { tree };
    fed.inventory.get_mut(hero).expect("bag").slots[1] = Some(fresh_stack(ItemId::TANGO, 1));
    hurt(&mut fed, hero);
    hurt(&mut fasting, other);
    assert!(fed.use_item(hero, 1, Target::Pos(eaten), &mut Vec::new()));
    run(&mut fed, 60 * rules::TICKS_PER_SECOND);
    run(&mut fasting, 60 * rules::TICKS_PER_SECOND);
    let hp = |world: &World, who: Entity| world.health.get(who).expect("health").hp;
    (hp(&fed, hero) - hp(&fasting, other)).to_int()
}

/// Dota 7.41f `item_tango`: 7 health a second for 16 seconds from a tree of the map, 112 in
/// all; off a tree an Iron Branch put up the duration is doubled (Liquipedia's *Tango*:
/// "When cast on an Iron Branch tree the health regen duration is greatly increased", 32
/// seconds), so 224. The whole of a drink is given: within the health of one tick's mend of
/// it, not the share a rate rounded to hundredths of a point leaves (110 and 220).
#[test]
fn a_tango_gives_112_off_a_map_tree_and_224_off_a_tree_a_branch_put_up() {
    for (planted, dota) in [(false, 112), (true, 224)] {
        let gained = health_a_tango_gives(planted);
        assert!(
            (dota - 1..=dota).contains(&gained),
            "{} gave {gained}, Dota gives {dota}",
            if planted {
                "a tree an Iron Branch put up"
            } else {
                "a tree of the map"
            }
        );
    }
}

#[test]
fn a_blade_takes_only_the_tree_it_was_pointed_at_and_only_within_reach() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::QUELLING_BLADE, 0);
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
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.step();
    let bare_creep = one_swing_takes(&mut world, hero, creep);
    let bare_hero = one_swing_takes(&mut world, hero, theirs);
    hand_item(&mut world, hero, ItemId::QUELLING_BLADE, 0);
    world.step();
    // A ranged hero's: Shadow Fiend wears the blade's ranged number.
    let blade = ITEMS[usize::from(ItemId::QUELLING_BLADE.0)]
        .carried_by(true)
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

/// Dota 7.41f, Healing Salve and Clarity: "If the unit is attacked by an
/// enemy hero or Roshan, the effect is lost."; Blink Dagger: "cannot be used
/// for %blink_damage_cooldown% seconds after taking damage from an enemy hero
/// or Roshan." A tower is neither.
#[test]
fn a_tower_shot_leaves_the_drinks_running_and_a_blink_ready() {
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(5000, 5000));
    let drinker = a_drinker(&mut world, Vec2::from_ints(5400, 5000));
    assert!(
        one_swing_takes(&mut world, tower, drinker) > 0,
        "the shot lands"
    );
    assert_eq!(drinks(&world, drinker), (true, true), "both drinks hold");
    assert_eq!(blink_wait(&world, drinker), 0, "the blink is ready");
}

#[test]
fn an_enemy_heros_swing_or_raze_puts_the_drinks_out_and_sets_a_blink_back() {
    for raze in [false, true] {
        let (mut world, fiend, _far_off) = fiend_and_a_mark(3000);
        let at = Vec2::from_ints(5000 + rules::RAZE_DISTANCE[0], 5000);
        let drinker = a_drinker(&mut world, at);
        if raze {
            let events = cast_through(&mut world, 0, rules::RAZE_CAST_POINT_MS);
            assert!(!burns_on(&events, drinker).is_empty(), "the raze lands");
            assert!(
                !events.iter().any(|event| matches!(event.kind,
                    EventKind::Damaged { target, kind: DamageKind::Physical, .. }
                        if target == wire_id(drinker))),
                "and nothing else does"
            );
        } else {
            assert!(
                one_swing_takes(&mut world, fiend, drinker) > 0,
                "the swing lands"
            );
        }
        assert_eq!(
            drinks(&world, drinker),
            (false, false),
            "raze {raze}: both drinks go out"
        );
        assert!(
            blink_wait(&world, drinker) > 0,
            "raze {raze}: the blink waits"
        );
    }
}

#[test]
fn a_creeps_swing_or_ones_own_rot_leaves_the_drinks_running() {
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    let drinker = a_drinker(&mut world, Vec2::from_ints(5100, 5000));
    assert!(
        one_swing_takes(&mut world, creep, drinker) > 0,
        "the swing lands"
    );
    assert_eq!(
        drinks(&world, drinker),
        (true, true),
        "a creep breaks nothing"
    );
    assert_eq!(blink_wait(&world, drinker), 0);

    let (mut world, pudge, _far_off) = pudge_and_a_mark(3000);
    set_level(&mut world, pudge, 1, 1);
    drink_up(&mut world, pudge);
    cast(&mut world, 1, Target::None);
    let burnt = (0..rules::BURN_PERIOD_TICKS * 2)
        .flat_map(|_| world.step())
        .any(|event| {
            matches!(event.kind, EventKind::Damaged { target, amount, .. }
                if target == wire_id(pudge) && amount > 0)
        });
    assert!(burnt, "the rot burns its owner");
    assert_eq!(
        drinks(&world, pudge),
        (true, true),
        "his own rot breaks nothing"
    );
    assert_eq!(blink_wait(&world, pudge), 0);
}

#[test]
fn a_heros_blow_never_puts_out_what_a_tree_bought() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    let hero = hero_at(&mut world, Vec2::from_ints(7000, 7000));
    let theirs = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(7000, 7000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.step();
    let tree = tree_positions(map, crate::game::terrain_of(map))
        .into_iter()
        .find(|at| at.within(Vec2::from_ints(7000, 7000), Fixed::from_int(4000)))
        .expect("the forest reaches here");
    world.transform.get_mut(hero).expect("hero").pos = tree + Vec2::from_ints(120, 0);
    hand_item(&mut world, hero, ItemId::TANGO, 1);
    assert!(world.use_item(hero, 0, Target::Pos(tree), &mut Vec::new()));
    world.push_hit(Some(theirs), hero, 10, DamageKind::Physical, None);
    world.step();
    assert_eq!(
        drinks(&world, hero),
        (true, false),
        "a tango holds through a hero's blow"
    );
}

/// A Dire hero standing at a spot with a Healing Salve and a Clarity running
/// and a Blink Dagger ready in its second slot.
fn a_drinker(world: &mut World, at: Vec2) -> Entity {
    let drinker = world.spawn_hero(Team::Dire, at, SlotId(1), HeroId::SHADOW_FIEND);
    world.settle();
    world.step();
    drink_up(world, drinker);
    drinker
}

/// Drinks a Healing Salve and a Clarity and lays a Blink Dagger in the
/// second slot.
fn drink_up(world: &mut World, hero: Entity) {
    for item in [ItemId::HEALING_SALVE, ItemId::CLARITY] {
        hand_item(world, hero, item, 1);
        assert!(
            world.use_item(hero, 0, Target::None, &mut Vec::new()),
            "drunk"
        );
    }
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(fresh_stack(ItemId::BLINK_DAGGER, 0));
    }
    assert_eq!(drinks(world, hero), (true, true));
}

/// Whether a unit mends health and whether it mends mana right now.
fn drinks(world: &World, unit: Entity) -> (bool, bool) {
    let health = ModifierKind::Mending {
        per_tick: Fixed::ZERO,
        breaks: false,
    };
    let mana = ModifierKind::Clarity {
        per_tick: Fixed::ZERO,
        breaks: false,
    };
    (carries(world, unit, health), carries(world, unit, mana))
}

/// What is left of the wait on the Blink Dagger in a hero's second slot.
fn blink_wait(world: &World, hero: Entity) -> u32 {
    slot_of(world, hero, 1).expect("a blink dagger").cooldown
}

#[test]
fn what_an_item_is_set_to_is_worth_points_of_that_attribute() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = ItemId::POWER_TREADS;
    let mut events = Vec::new();
    assert!(world.buy(SlotId(0), treads, &mut events), "bought");
    world.step();
    let bonus = Fixed::from_int(ITEMS[usize::from(ItemId::POWER_TREADS.0)].mode_bonus);
    let before = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Agility),
        "bought whole they come of the Band, set to the hero's agility"
    );
    assert!(
        world.use_item(hero, 0, Target::None, &mut Vec::new()),
        "and using them sets them over"
    );
    world.step();
    let on_strength = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Strength),
        "to the attribute after the one they were on: Dota's wheel goes strength, \
         intelligence, agility, and from agility back to strength"
    );
    assert_eq!(
        on_strength.attributes.agility + bonus,
        before.attributes.agility,
        "what they were worth in agility is gone"
    );
    assert_eq!(
        on_strength.attributes.strength,
        before.attributes.strength + bonus,
        "and worth the same in strength instead"
    );
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Intelligence),
        "then to intelligence"
    );
}

#[test]
fn switching_treads_mid_swing_breaks_nothing_off() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(ItemStack {
        mode: Some(Attribute::Strength),
        ..fresh_stack(ItemId::POWER_TREADS, 0)
    });
    world.settle();
    attack_click(&mut world, theirs);
    for _ in 0..60 {
        if swinging(&world, hero).is_some() {
            break;
        }
        world.advance(&[]);
    }
    assert_eq!(swinging(&world, hero), Some(theirs), "mid-swing");
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(Attribute::Intelligence),
        "the switch went off at once"
    );
    assert_eq!(
        swinging(&world, hero),
        Some(theirs),
        "beside the swing, which goes on: Dota's immediate item"
    );
}

#[test]
fn treads_switched_round_the_wheel_mend_nothing() {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(ItemStack {
        mode: Some(Attribute::Strength),
        ..fresh_stack(ItemId::POWER_TREADS, 0)
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
    let bonus = Fixed::from_int(ITEMS[usize::from(ItemId::POWER_TREADS.0)].mode_bonus);
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
fn a_blink_aimed_at_closed_ground_steps_back_to_open() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, ItemId::BLINK_DAGGER, 0);
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
fn phase_walks_a_body_through_another() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, ItemId::PHASE_BOOTS, 0);
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
    let (mut world, hero) = match_hero();
    let regen = |world: &World| {
        let unit = shown(world, Some(Team::Radiant), hero);
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
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    let (plain_hp, plain_mana) = regen(&world);
    assert_eq!((plain_hp.raw, plain_mana.raw), served(&world));
    assert!(fountain_hp > plain_hp && fountain_mana > plain_mana);

    hand_item(&mut world, hero, ItemId::HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    world.step();
    let (salved, _) = regen(&world);
    assert_eq!(salved.raw, served(&world).0);
    assert!(salved > plain_hp, "the salve shows in the view");
}
