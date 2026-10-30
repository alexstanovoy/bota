//! The Lotus Pools at Dota 7.41f: what they grow and when, the row of plucks
//! a hero makes standing in one, and the lotuses it holds and eats.

use bota_core::rules::{self, Schedule};
use bota_core::{
    DOTA_LOTUS_POOLS, LOTUS_COOLDOWN, LOTUS_TAKE_RANGE, PLUCK_TICKS, POOL_MAX_LOTUSES, map_of,
};
use bota_proto::{
    EventKind, Fixed, HeroId, ItemId, ItemSlot, LotusTier, MapId, ObjectiveKind, ObjectiveSet,
    Order, Pick, RejectReason, RuneSet, SlotId, Target, Team, UnitKind, Vec2,
};

use crate::game::{Entity, Event, World, wire_id};

use super::runes::{fraction, hero, place};
use super::support::*;

/// The tick a game minute begins at on the Dota map, counted from its horn.
fn at_minute(minutes: u32) -> u32 {
    Schedule::of(rules::DOTA_PREGAME_TICKS).at_minute(minutes)
}

/// A match on the Dota map with the Lotus Pools alone, its seats Shadow
/// Fiends of the sides given, before any tick.
fn pool_match(sides: &[Team]) -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.runes = RuneSet::NONE;
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::LotusPools);
    cfg.picks = sides
        .iter()
        .enumerate()
        .map(|(slot, team)| Pick {
            slot: SlotId(slot as u8),
            team: *team,
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    World::for_match(&cfg, cfg.rng())
}

/// Steps the one tick whose game clock is `clock`.
fn step_at(world: &mut World, clock: u32) -> Vec<Event> {
    world.tick = clock - 1 - world.clock_shift.game;
    world.step()
}

fn pool_at(world: &World, spot: usize) -> Entity {
    let at = DOTA_LOTUS_POOLS[spot];
    world
        .entities
        .iter()
        .find(|&entity| {
            world.kind.get(entity) == Some(&UnitKind::LotusPool)
                && world.transform.get(entity).is_some_and(|t| t.pos == at)
        })
        .expect("a pool stands there")
}

fn held(world: &World, pool: Entity) -> (LotusTier, u8) {
    let pool = world.pool.get(pool).expect("a pool");
    (pool.tier, pool.lotuses)
}

fn fill(world: &mut World, pool: Entity, tier: LotusTier, lotuses: u8) {
    let pool = world.pool.get_mut(pool).expect("a pool");
    pool.tier = tier;
    pool.lotuses = lotuses;
}

/// Puts a hero in the middle of a pool.
fn stand_in(world: &mut World, slot: usize, spot: usize) -> Entity {
    let unit = hero(world, slot);
    place(world, unit, DOTA_LOTUS_POOLS[spot]);
    unit
}

/// The lotuses a hero carries: item, charges.
fn lotuses_of(world: &World, unit: Entity) -> Vec<(ItemId, u8)> {
    world
        .inventory
        .get(unit)
        .expect("a bag")
        .slots
        .iter()
        .flatten()
        .filter(|stack| {
            [
                ItemId::HEALING_LOTUS,
                ItemId::GREAT_HEALING_LOTUS,
                ItemId::GREATER_HEALING_LOTUS,
            ]
            .contains(&stack.id)
        })
        .map(|stack| (stack.id, stack.charges))
        .collect()
}

fn grown(events: &[Event]) -> Vec<LotusTier> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::LotusGrown { tier, .. } => Some(tier),
            _ => None,
        })
        .collect()
}

fn radiant_and_dire() -> [Team; 2] {
    [Team::Radiant, Team::Dire]
}

#[test]
fn two_pools_stand_at_the_dota_spots_empty_and_neutral() {
    let world = pool_match(&radiant_and_dire());
    for spot in 0..2 {
        let pool = pool_at(&world, spot);
        assert_eq!(held(&world, pool), (LotusTier::Healing, 0));
        assert_eq!(world.team.get(pool), Some(&Team::Neutral));
        assert!(world.health.get(pool).is_some(), "a building with health");
    }
    assert_eq!(map_of(MapId(0)).lotus_pools.len(), 2);
    assert!(
        map_of(MapId(0))
            .default_objectives
            .has(ObjectiveKind::LotusPools),
        "the Dota map stands them by default"
    );
    assert!(
        !map_of(MapId(2))
            .default_objectives
            .has(ObjectiveKind::LotusPools),
        "the mid-only map only when asked"
    );
}

#[test]
fn a_pool_grows_a_lotus_every_three_minutes_from_3_00_up_to_six() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    assert!(grown(&step_at(&mut world, at_minute(3) - 1)).is_empty());
    for (round, minute) in [3, 6, 9, 12, 15, 18].into_iter().enumerate() {
        let events = step_at(&mut world, at_minute(minute));
        assert_eq!(
            grown(&events),
            vec![LotusTier::Healing; 2],
            "one in each pool at {minute}:00"
        );
        assert_eq!(held(&world, pool), (LotusTier::Healing, round as u8 + 1));
        assert!(
            grown(&step_at(&mut world, at_minute(minute) + 30)).is_empty(),
            "none between the marks"
        );
    }
    let full = step_at(&mut world, at_minute(21));
    assert!(grown(&full).is_empty(), "a full pool grows no more");
    assert_eq!(held(&world, pool).1, POOL_MAX_LOTUSES);
}

#[test]
fn both_sides_are_told_what_both_pools_hold_wherever_their_heroes_are() {
    let mut world = pool_match(&radiant_and_dire());
    step_at(&mut world, at_minute(3));
    for team in [Team::Radiant, Team::Dire] {
        let view = world.view(team);
        assert_eq!(view.pools.len(), 2, "{team:?}");
        assert!(view.pools.iter().all(|pool| pool.lotuses == 1
            && pool.tier == LotusTier::Healing
            && pool.next_in == Some(180 * rules::TICKS_PER_SECOND)));
        for pool in &view.pools {
            assert!(
                view.units.iter().any(|unit| unit.id == pool.id),
                "the pool itself is in the view of a side with no hero near"
            );
        }
    }
}

#[test]
fn great_lotuses_grow_from_35_00_and_greater_from_60_00_rounding_up_what_a_pool_holds() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    fill(&mut world, pool, LotusTier::Healing, 4);
    step_at(&mut world, at_minute(35));
    assert_eq!(
        held(&world, pool),
        (LotusTier::Great, 2),
        "four healing lotuses round up to two great ones"
    );
    let events = step_at(&mut world, at_minute(36));
    assert_eq!(grown(&events), vec![LotusTier::Great; 2]);
    assert_eq!(held(&world, pool), (LotusTier::Great, 3));
    step_at(&mut world, at_minute(60));
    assert_eq!(
        held(&world, pool),
        (LotusTier::Greater, 3),
        "three great ones make two greater ones, and the mark grows one more"
    );
    let events = step_at(&mut world, at_minute(63));
    assert_eq!(grown(&events), vec![LotusTier::Greater; 2]);
}

#[test]
fn a_hero_in_a_pool_plucks_a_lotus_in_one_and_a_half_seconds_and_the_next_in_less() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    fill(&mut world, pool, LotusTier::Healing, 6);
    let plucker = stand_in(&mut world, 0, 0);
    let mut times = Vec::new();
    let mut since = 0;
    for _ in 0..200 {
        world.step();
        since += 1;
        let now = held(&world, pool).1;
        if usize::from(POOL_MAX_LOTUSES - now) > times.len() {
            times.push(since);
            since = 0;
        }
    }
    assert_eq!(
        times,
        PLUCK_TICKS.to_vec(),
        "1.5, 1.05, 0.74, 0.51, 0.36 and 0.3 seconds"
    );
    // Six healing lotuses fold, three and three, into two great ones that
    // fold into a greater one.
    assert_eq!(
        lotuses_of(&world, plucker),
        vec![(ItemId::GREATER_HEALING_LOTUS, 1)]
    );
}

#[test]
fn leaving_a_pool_ends_the_row_so_the_next_lotus_takes_as_long_as_the_first() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    fill(&mut world, pool, LotusTier::Healing, 6);
    let plucker = stand_in(&mut world, 0, 0);
    for _ in 0..(PLUCK_TICKS[0] + PLUCK_TICKS[1] + 6) {
        world.step();
    }
    assert!(held(&world, pool).1 <= 4, "two plucked in a row");
    place(&mut world, plucker, Vec2::from_ints(9000, 9000));
    run(&mut world, 3);
    let before = held(&world, pool).1;
    stand_in(&mut world, 0, 0);
    run(&mut world, PLUCK_TICKS[1] + 3);
    assert_eq!(
        held(&world, pool).1,
        before,
        "back, it starts the row again"
    );
    run(&mut world, PLUCK_TICKS[0] - PLUCK_TICKS[1] + 2);
    assert_eq!(
        held(&world, pool).1,
        before - 1,
        "one after the first row's time"
    );
}

#[test]
fn an_enemy_hero_in_the_pool_runs_the_countdown_back_until_it_leaves() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    fill(&mut world, pool, LotusTier::Healing, 3);
    stand_in(&mut world, 0, 0);
    let enemy = stand_in(&mut world, 1, 0);
    run(&mut world, 4 * PLUCK_TICKS[0]);
    assert_eq!(
        held(&world, pool).1,
        3,
        "nobody plucks while both stand there"
    );
    place(&mut world, enemy, Vec2::from_ints(9000, 9000));
    run(&mut world, PLUCK_TICKS[0] + 3);
    assert_eq!(held(&world, pool).1, 2, "it goes on once the enemy is gone");
}

#[test]
fn a_hero_with_no_room_for_a_lotus_does_not_pluck_and_does_not_hold_the_pool_against_an_ally() {
    let mut world = pool_match(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let pool = pool_at(&world, 0);
    fill(&mut world, pool, LotusTier::Healing, 3);
    let full = stand_in(&mut world, 0, 0);
    for slot in 0..9 {
        world.inventory.get_mut(full).expect("a bag").slots[slot] =
            Some(fresh_stack(ItemId::IRON_BRANCH, 1));
    }
    run(&mut world, 2 * PLUCK_TICKS[0]);
    assert_eq!(held(&world, pool).1, 3, "a full bag takes none");
    let roomy = stand_in(&mut world, 1, 0);
    run(&mut world, PLUCK_TICKS[0] + 3);
    assert_eq!(held(&world, pool).1, 2);
    assert_eq!(lotuses_of(&world, roomy), vec![(ItemId::HEALING_LOTUS, 1)]);
    assert!(lotuses_of(&world, full).is_empty());
}

#[test]
fn a_lotus_goes_to_one_of_the_allies_standing_in_the_pool_at_random() {
    let mut got = [0u32; 2];
    for seed in 0..12u8 {
        let mut cfg_world = pool_match(&[Team::Radiant, Team::Radiant]);
        cfg_world.rng = crate::game::MatchRng::new(&[seed; 32], 1);
        let pool = pool_at(&cfg_world, 0);
        fill(&mut cfg_world, pool, LotusTier::Healing, 1);
        let ones = [
            stand_in(&mut cfg_world, 0, 0),
            stand_in(&mut cfg_world, 1, 0),
        ];
        run(&mut cfg_world, PLUCK_TICKS[0] + 3);
        for (at, one) in ones.into_iter().enumerate() {
            got[at] += lotuses_of(&cfg_world, one).len() as u32;
        }
    }
    assert_eq!(got[0] + got[1], 12, "one lotus each time");
    assert!(got[0] > 0 && got[1] > 0, "both get some: {got:?}");
}

#[test]
fn a_hero_eats_a_lotus_at_once_for_125_health_and_mana_and_waits_five_seconds_for_the_next() {
    let (mut world, taker) = match_hero();
    hand_item(&mut world, taker, ItemId::HEALING_LOTUS, 2);
    let stats = *world.stats.get(taker).expect("stats");
    world.health.get_mut(taker).expect("hp").hp = fraction(stats.max_hp, 1, 4);
    world.mana.get_mut(taker).expect("mana").mana = Fixed::ZERO;
    let (hp, mana) = (
        world.health.get(taker).expect("hp").hp,
        world.mana.get(taker).expect("mana").mana,
    );
    let eat = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    advance_validated(&mut world, None, eat);
    let gained = |world: &World| {
        (
            (world.health.get(taker).expect("hp").hp - hp).to_int(),
            (world.mana.get(taker).expect("mana").mana - mana).to_int(),
        )
    };
    let (got_hp, got_mana) = gained(&world);
    assert!((125..=130).contains(&got_hp), "125 health: {got_hp}");
    assert!((125..=130).contains(&got_mana), "125 mana: {got_mana}");
    assert_eq!(slot_of(&world, taker, 0).expect("one left").charges, 1);
    assert_eq!(
        world.validate_order(SlotId(0), None, &eat),
        Err(RejectReason::OnCooldown)
    );
    run(&mut world, LOTUS_COOLDOWN);
    assert_eq!(world.validate_order(SlotId(0), None, &eat), Ok(()));
}

#[test]
fn the_wait_after_a_lotus_is_shared_by_all_three_kinds() {
    let (mut world, taker) = match_hero();
    hand_item(&mut world, taker, ItemId::HEALING_LOTUS, 1);
    world.inventory.get_mut(taker).expect("a bag").slots[1] =
        Some(fresh_stack(ItemId::GREAT_HEALING_LOTUS, 1));
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert_eq!(
        world.validate_order(
            SlotId(0),
            None,
            &Order::Use {
                slot: ItemSlot(1),
                target: Target::None
            }
        ),
        Err(RejectReason::OnCooldown),
        "the great one waits for the healing one's five seconds"
    );
}

#[test]
fn a_great_lotus_mends_400_and_may_be_eaten_by_an_ally_within_reach() {
    let mut world = pool_match(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let (one, ally) = (hero(&world, 0), hero(&world, 1));
    place(&mut world, one, Vec2::from_ints(7000, 9000));
    place(&mut world, ally, Vec2::from_ints(7350, 9000));
    run(&mut world, 3);
    world.inventory.get_mut(one).expect("a bag").slots[0] =
        Some(fresh_stack(ItemId::GREAT_HEALING_LOTUS, 1));
    world.health.get_mut(ally).expect("hp").hp = Fixed::from_int(10);
    world.mana.get_mut(ally).expect("mana").mana = Fixed::ZERO;
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Unit(wire_id(ally)),
        },
    );
    let hp = world.health.get(ally).expect("hp").hp.to_int();
    let mana = world.mana.get(ally).expect("mana").mana.to_int();
    let max = world.stats.get(ally).expect("stats").max_mana.to_int();
    assert!((400..=440).contains(&hp), "400 health: {hp}");
    assert_eq!(mana, 400.min(max), "400 mana, to the pool's top");
}

#[test]
fn three_healing_lotuses_fold_into_a_great_one_wherever_they_come_from() {
    let (mut world, taker) = match_hero();
    for _ in 0..3 {
        let stack = crate::game::ItemStack::granted(ItemId::HEALING_LOTUS, SlotId(0), 0, 1);
        assert!(
            world
                .inventory
                .get_mut(taker)
                .expect("a bag")
                .receive(stack)
        );
    }
    world.step();
    assert_eq!(
        lotuses_of(&world, taker),
        vec![(ItemId::GREAT_HEALING_LOTUS, 1)]
    );
}

#[test]
fn a_lotus_on_the_ground_is_taken_from_275_units() {
    let (mut world, taker) = match_hero();
    let spot = an_empty_spot(&world);
    let stack = crate::game::ItemStack::granted(ItemId::HEALING_LOTUS, SlotId(0), 0, 1);
    let lying = world.lay_loot(stack, spot);
    // A hero 260 away is within a lotus's reach and not within an ordinary
    // item's.
    let away = Vec2::from_ints(spot.x.to_int() - 260, spot.y.to_int());
    place(&mut world, taker, away);
    advance_validated(
        &mut world,
        None,
        Order::Take {
            target: Target::Unit(wire_id(lying)),
        },
    );
    run(&mut world, 3);
    assert_eq!(lotuses_of(&world, taker), vec![(ItemId::HEALING_LOTUS, 1)]);
    const { assert!(LOTUS_TAKE_RANGE > rules::TAKE_ITEM_RANGE) };
    assert_eq!(world.pos(taker), Some(away), "it did not walk");
}

/// Lays `charges` of an item in the first free slot of a hero's bag, as a
/// stack of its own that is not merged into another.
fn lay(world: &mut World, unit: Entity, item: ItemId, charges: u8) {
    let bag = world.inventory.get_mut(unit).expect("a bag");
    let at = bag.slots.iter().position(Option::is_none).expect("room");
    bag.slots[at] = Some(crate::game::ItemStack::granted(item, SlotId(0), 0, charges));
}

/// What a hero carries of the lotuses, Cheese and Royale with Cheese: item,
/// charges, in slot order.
fn held_items(world: &World, unit: Entity) -> Vec<(ItemId, u8)> {
    world
        .inventory
        .get(unit)
        .expect("a bag")
        .slots
        .iter()
        .flatten()
        .map(|stack| (stack.id, stack.charges))
        .collect()
}

/// Lays the stacks given, steps one tick, and answers what the bag holds.
fn assembled(stacks: &[(ItemId, u8)]) -> Vec<(ItemId, u8)> {
    let (mut world, unit) = match_hero();
    for &(item, charges) in stacks {
        lay(&mut world, unit, item, charges);
    }
    world.step();
    held_items(&world, unit)
}

const HEALING: ItemId = ItemId::HEALING_LOTUS;
const GREAT: ItemId = ItemId::GREAT_HEALING_LOTUS;
const GREATER: ItemId = ItemId::GREATER_HEALING_LOTUS;

/// Lets the pools act on the tick whose game clock is `clock`, and nothing
/// else of the world: a long game's marks are reached without playing it.
fn pools_act_at(world: &mut World, clock: u32) {
    world.tick = clock - world.clock_shift.game;
    world.tick_pools();
}

#[test]
fn a_pool_never_holds_more_than_six_lotuses_whatever_the_time() {
    let mut world = pool_match(&radiant_and_dire());
    let pool = pool_at(&world, 0);
    let mut tier = LotusTier::Healing;
    // Every growth mark of three hours.
    for minute in (3..=180).step_by(3) {
        pools_act_at(&mut world, at_minute(minute));
        let (now, lotuses) = held(&world, pool);
        assert!(
            lotuses <= POOL_MAX_LOTUSES,
            "{lotuses} lotuses at {minute}:00"
        );
        assert!(now >= tier, "a pool's tier never goes back at {minute}:00");
        tier = now;
    }
    assert_eq!(held(&world, pool), (LotusTier::Greater, POOL_MAX_LOTUSES));
    // A full pool of each tier, across the marks that round it up and add
    // one, and at the marks after.
    for from in [LotusTier::Healing, LotusTier::Great, LotusTier::Greater] {
        for minute in [33, 36, 57, 60, 63, 120] {
            let mut world = pool_match(&radiant_and_dire());
            let pool = pool_at(&world, 1);
            fill(&mut world, pool, from, POOL_MAX_LOTUSES);
            pools_act_at(&mut world, at_minute(minute));
            assert!(
                held(&world, pool).1 <= POOL_MAX_LOTUSES,
                "{from:?} full at {minute}:00"
            );
        }
    }
}

#[test]
fn three_healing_lotuses_in_any_stacks_make_a_great_one_and_the_rest_stay() {
    // Three in one stack, as a pool hands them over one at a time.
    assert_eq!(assembled(&[(HEALING, 3)]), vec![(GREAT, 1)]);
    // Two stacks that add to three charges.
    assert_eq!(assembled(&[(HEALING, 2), (HEALING, 1)]), vec![(GREAT, 1)]);
    // Two short of a build wait.
    assert_eq!(assembled(&[(HEALING, 2)]), vec![(HEALING, 2)]);
    // Four: a Great one is built and one Healing Lotus is left.
    let four = assembled(&[(HEALING, 3), (HEALING, 1)]);
    assert!(
        four.contains(&(GREAT, 1)) && four.contains(&(HEALING, 1)),
        "{four:?}"
    );
    assert_eq!(four.len(), 2);
}

#[test]
fn two_great_lotuses_make_a_greater_one_in_one_stack_or_two() {
    assert_eq!(assembled(&[(GREAT, 2)]), vec![(GREATER, 1)]);
    assert_eq!(assembled(&[(GREAT, 1), (GREAT, 1)]), vec![(GREATER, 1)]);
    assert_eq!(assembled(&[(GREAT, 1)]), vec![(GREAT, 1)]);
    // A third waits.
    let three = assembled(&[(GREAT, 2), (GREAT, 1)]);
    assert!(
        three.contains(&(GREATER, 1)) && three.contains(&(GREAT, 1)),
        "{three:?}"
    );
}

#[test]
fn lotuses_build_up_through_every_tier_as_they_arrive() {
    // Nine Healing Lotuses: three Great ones, two of which are a Greater.
    let nine = assembled(&[(HEALING, 3), (HEALING, 3), (HEALING, 3)]);
    assert!(
        nine.contains(&(GREATER, 1)) && nine.contains(&(GREAT, 1)),
        "{nine:?}"
    );
    assert_eq!(nine.len(), 2);
    // Eighteen: three Greater Lotuses, a stack of them.
    let eighteen = assembled(&[(HEALING, 3); 6]);
    assert_eq!(eighteen, vec![(GREATER, 3)]);
}

#[test]
fn three_greater_lotuses_and_a_cheese_make_royale_with_cheese() {
    assert_eq!(
        assembled(&[(GREATER, 3), (ItemId::CHEESE, 1)]),
        vec![(ItemId::ROYALE_WITH_CHEESE, 1)]
    );
    // The parts may sit in stacks of any size and order.
    assert_eq!(
        assembled(&[(ItemId::CHEESE, 1), (GREATER, 1), (GREATER, 2)]),
        vec![(ItemId::ROYALE_WITH_CHEESE, 1)]
    );
    // Two Greater Lotuses, or no Cheese, build nothing.
    assert_eq!(
        assembled(&[(GREATER, 2), (ItemId::CHEESE, 1)]),
        vec![(GREATER, 2), (ItemId::CHEESE, 1)]
    );
    assert_eq!(assembled(&[(GREATER, 3)]), vec![(GREATER, 3)]);
    // A Great Lotus and a Cheese are not enough either.
    assert_eq!(
        assembled(&[(GREAT, 1), (ItemId::CHEESE, 1)]),
        vec![(GREAT, 1), (ItemId::CHEESE, 1)]
    );
}

#[test]
fn royale_with_cheese_takes_the_charges_of_its_recipe_and_leaves_the_rest() {
    let left = assembled(&[(GREATER, 3), (ItemId::CHEESE, 2)]);
    assert!(left.contains(&(ItemId::ROYALE_WITH_CHEESE, 1)), "{left:?}");
    assert!(left.contains(&(ItemId::CHEESE, 1)), "{left:?}");
    assert_eq!(left.len(), 2);
}

#[test]
fn eighteen_healing_lotuses_and_a_cheese_build_all_the_way_up_to_royale_with_cheese() {
    let mut stacks = vec![(HEALING, 3); 6];
    stacks.push((ItemId::CHEESE, 1));
    assert_eq!(assembled(&stacks), vec![(ItemId::ROYALE_WITH_CHEESE, 1)]);
}

#[test]
fn a_cheese_a_cheat_hands_out_combines_with_the_lotuses_a_hero_holds() {
    let mut cfg = config();
    cfg.cheats = true;
    let mut world = World::for_match(&cfg, cfg.rng());
    let unit = world.seats[0].unit.expect("stood up");
    lay(&mut world, unit, GREATER, 3);
    advance_validated(
        &mut world,
        None,
        Order::Cheat {
            cheat: bota_proto::Cheat::Item {
                item: ItemId::CHEESE,
            },
        },
    );
    run(&mut world, 2);
    assert_eq!(
        held_items(&world, unit),
        vec![(ItemId::ROYALE_WITH_CHEESE, 1)]
    );
}

#[test]
fn cheese_is_eaten_for_2500_health_and_1500_mana_on_a_wait_of_its_own() {
    let (mut world, eater) = match_hero();
    hand_item(&mut world, eater, ItemId::CHEESE, 2);
    world.inventory.get_mut(eater).expect("a bag").slots[1] = Some(fresh_stack(HEALING, 1));
    let stats = *world.stats.get(eater).expect("stats");
    world.health.get_mut(eater).expect("hp").hp = Fixed::from_int(1);
    world.mana.get_mut(eater).expect("mana").mana = Fixed::ZERO;
    let eat = Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    };
    advance_validated(&mut world, None, eat);
    let hp = world.health.get(eater).expect("hp").hp.to_int();
    let mana = world.mana.get(eater).expect("mana").mana.to_int();
    assert_eq!(hp, (1 + 2500).min(stats.max_hp.to_int()), "2500 health");
    assert_eq!(mana, 1500.min(stats.max_mana.to_int()), "1500 mana");
    assert_eq!(slot_of(&world, eater, 0).expect("one left").charges, 1);
    assert_eq!(
        world.validate_order(SlotId(0), None, &eat),
        Err(RejectReason::OnCooldown)
    );
    // Its wait is not the lotuses': a lotus may be eaten at once.
    let lotus = Order::Use {
        slot: ItemSlot(1),
        target: Target::None,
    };
    assert_eq!(world.validate_order(SlotId(0), None, &lotus), Ok(()));
    run(&mut world, 40 * rules::TICKS_PER_SECOND);
    assert_eq!(world.validate_order(SlotId(0), None, &eat), Ok(()));
}

#[test]
fn royale_with_cheese_is_eaten_for_3000_health_and_2000_mana() {
    let (mut world, eater) = match_hero();
    hand_item(&mut world, eater, ItemId::ROYALE_WITH_CHEESE, 1);
    let stats = *world.stats.get(eater).expect("stats");
    world.health.get_mut(eater).expect("hp").hp = Fixed::from_int(1);
    world.mana.get_mut(eater).expect("mana").mana = Fixed::ZERO;
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    let hp = world.health.get(eater).expect("hp").hp.to_int();
    let mana = world.mana.get(eater).expect("mana").mana.to_int();
    assert_eq!(hp, (1 + 3000).min(stats.max_hp.to_int()), "3000 health");
    assert_eq!(mana, 2000.min(stats.max_mana.to_int()), "2000 mana");
    assert!(
        slot_of(&world, eater, 0).is_none(),
        "the one charge is spent"
    );
}

#[test]
fn a_cheese_may_not_be_aimed_at_an_ally_as_a_lotus_may() {
    let mut cfg = config();
    cfg.picks = vec![
        Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::SHADOW_FIEND,
        },
        Pick {
            slot: SlotId(1),
            team: Team::Radiant,
            hero: HeroId::SHADOW_FIEND,
        },
    ];
    let mut world = World::for_match(&cfg, cfg.rng());
    let (user, ally) = (hero(&world, 0), hero(&world, 1));
    hand_item(&mut world, user, ItemId::CHEESE, 1);
    let aimed = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(wire_id(ally)),
    };
    assert!(world.validate_order(SlotId(0), None, &aimed).is_err());
    hand_item(&mut world, user, HEALING, 1);
    assert_eq!(world.validate_order(SlotId(0), None, &aimed), Ok(()));
}

/// Orders the lock against combining of the stack in a slot, as a player does.
fn lock(world: &mut World, slot: u8, locked: bool) {
    advance_validated(
        world,
        None,
        Order::SetCombineLock {
            slot: ItemSlot(slot),
            locked,
        },
    );
}

#[test]
fn a_lotus_locked_against_combining_takes_no_part_in_a_build_until_it_is_unlocked() {
    let (mut world, unit) = match_hero();
    lay(&mut world, unit, HEALING, 2);
    // A lotus is part of a recipe, so it takes the lock.
    lock(&mut world, 0, true);
    lay(&mut world, unit, HEALING, 1);
    run(&mut world, 3);
    assert_eq!(
        held_items(&world, unit),
        vec![(HEALING, 2), (HEALING, 1)],
        "two of the three charges are locked: nothing is built"
    );
    lock(&mut world, 0, false);
    run(&mut world, 3);
    assert_eq!(held_items(&world, unit), vec![(GREAT, 1)]);
}

#[test]
fn a_cheese_or_greater_lotuses_locked_against_combining_keep_royale_from_being_built() {
    for locked_slot in [0, 1] {
        let (mut world, unit) = match_hero();
        lay(&mut world, unit, GREATER, 2);
        lay(&mut world, unit, ItemId::CHEESE, 1);
        lock(&mut world, locked_slot, true);
        lay(&mut world, unit, GREATER, 1);
        run(&mut world, 3);
        assert!(
            !held_items(&world, unit)
                .iter()
                .any(|(item, _)| *item == ItemId::ROYALE_WITH_CHEESE),
            "slot {locked_slot} locked: no Royale"
        );
        lock(&mut world, locked_slot, false);
        run(&mut world, 3);
        assert_eq!(
            held_items(&world, unit),
            vec![(ItemId::ROYALE_WITH_CHEESE, 1)],
            "slot {locked_slot} unlocked: built"
        );
    }
}
