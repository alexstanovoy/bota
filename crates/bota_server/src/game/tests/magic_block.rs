//! Carried passives that block a share of a large magical blow, a charge a block: Dota's Infused
//! Raindrops, every number from the install's `items.txt` through the catalogue.

use bota_core::{item_def, mitigate, rules};
use bota_proto::{DamageKind, EventKind, Fixed, ItemId, Order, SlotId, Team, Vec2};

use crate::game::{Entity, World, wire_id};

use super::item_numbers::foe_hero;
use super::support::*;

/// A hero of seat 0 carrying `charges` of Infused Raindrops in its first slot, out of the
/// fountain, an enemy hero beside it and one of its own side: the world, the carrier, the enemy
/// and the ally.
fn carrier(charges: u8) -> (World, Entity, Entity, Entity) {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, ItemId::INFUSED_RAINDROP, charges);
    world.transform.get_mut(hero).expect("standing").pos = Vec2::from_ints(9000, 9216);
    let at = Vec2::from_ints(9000, 9216);
    let enemy = foe_hero(&mut world, at + Vec2::from_ints(3000, 0));
    let ally = world.spawn_hero(
        Team::Radiant,
        at + Vec2::from_ints(-3000, 0),
        SlotId(2),
        bota_proto::HeroId::PUDGE,
    );
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    (world, hero, enemy, ally)
}

/// What `on` takes of a magical blow of `amount` from `by`, laid this tick.
fn magic_blow(world: &mut World, by: Entity, on: Entity, amount: i32) -> i32 {
    world.push_hit(Some(by), on, amount, DamageKind::Magical, None);
    world
        .step()
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Damaged { target, amount, .. } if target == wire_id(on) => Some(amount),
            _ => None,
        })
        .expect("the blow is told")
}

/// What `on`'s magic resistance leaves of `amount`.
fn after_resistance(world: &World, on: Entity, amount: i32) -> i32 {
    let resist = world.stats.get(on).expect("derived").magic_resist_bp;
    mitigate(amount, DamageKind::Magical, Fixed::ZERO, resist)
}

/// The Raindrops' charges in the carrier's first slot, none once they are gone.
fn charges(world: &World, hero: Entity) -> Option<u8> {
    slot_of(world, hero, 0)
        .filter(|held| held.id == ItemId::INFUSED_RAINDROP)
        .map(|held| held.charges)
}

/// Infused Raindrops block 120 of an enemy's magical blow of 75 or more before reductions, after
/// the carrier's magic resistance, spending one of the six charges they are bought with and
/// their 7 seconds (`magic_damage_block`, `min_damage`, `initial_charges`, `AbilityCooldown`;
/// Dota: "Consumes a charge to block 120 magic damage from damage instances over 75 damage").
#[test]
fn infused_raindrops_block_120_of_an_enemys_large_magical_blow_for_a_charge() {
    let def = item_def(ItemId::INFUSED_RAINDROP).expect("in the catalogue");
    let guard = def.magic_block.expect("a magic block");
    assert_eq!((guard.amount, guard.min_damage), (120, 75));
    assert_eq!(def.charges, 6);
    assert_eq!(def.cooldown, 7 * rules::TICKS_PER_SECOND);
    let (mut world, hero, enemy, _) = carrier(6);
    let full = after_resistance(&world, hero, 300);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(charges(&world, hero), Some(5), "a charge spent");
    let wait = slot_of(&world, hero, 0).expect("carried").cooldown;
    assert!(
        wait > 7 * rules::TICKS_PER_SECOND - 3 && wait <= 7 * rules::TICKS_PER_SECOND,
        "its cooldown runs: {wait}"
    );
}

/// "Neither consumes charges nor blocks any damage while on cooldown": the next blow lands whole,
/// and once the 7 seconds are over a blow is blocked again.
#[test]
fn infused_raindrops_block_nothing_while_their_cooldown_runs() {
    let (mut world, hero, enemy, _) = carrier(6);
    let full = after_resistance(&world, hero, 300);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full);
    assert_eq!(charges(&world, hero), Some(5));
    run(&mut world, 7 * rules::TICKS_PER_SECOND);
    world.fill_pools(hero);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(charges(&world, hero), Some(4));
}

/// The least damage is counted before reductions (Liquipedia: "Only triggers on magical damage
/// equal or greater than 75 (before reductions)"): a blow of 74 lands whole and spends nothing;
/// one of 75, cut below 75 by resistance, is blocked whole.
#[test]
fn infused_raindrops_answer_to_a_blow_of_75_before_reductions_and_not_74() {
    let (mut world, hero, enemy, _) = carrier(6);
    assert_eq!(
        magic_blow(&mut world, enemy, hero, 74),
        after_resistance(&world, hero, 74)
    );
    assert_eq!(charges(&world, hero), Some(6), "nothing spent");
    assert_eq!(magic_blow(&mut world, enemy, hero, 75), 0);
    assert_eq!(charges(&world, hero), Some(5));
}

/// Only an enemy's magical blow is blocked (Liquipedia: "excluding self-inflicted damage and
/// damage dealt by allies"; the block is magical): the carrier's own, an ally's and an enemy's
/// physical blow land whole and spend nothing.
#[test]
fn infused_raindrops_block_no_physical_blow_and_none_of_their_side() {
    let (mut world, hero, enemy, ally) = carrier(6);
    let full = after_resistance(&world, hero, 100);
    assert_eq!(magic_blow(&mut world, hero, hero, 100), full, "its own");
    world.fill_pools(hero);
    assert_eq!(magic_blow(&mut world, ally, hero, 100), full, "an ally's");
    world.fill_pools(hero);
    world.push_hit(Some(enemy), hero, 100, DamageKind::Physical, None);
    let armor = world.stats.get(hero).expect("derived").armor;
    let physical = world.step().iter().find_map(|event| match event.kind() {
        EventKind::Damaged { target, amount, .. } if target == wire_id(hero) => Some(amount),
        _ => None,
    });
    assert_eq!(
        physical,
        Some(mitigate(100, DamageKind::Physical, armor, 0)),
        "a physical blow"
    );
    assert_eq!(charges(&world, hero), Some(6), "nothing spent");
}

/// "When the charges are gone, the item disappears": the last charge blocks, and the slot is
/// empty after.
#[test]
fn infused_raindrops_are_gone_with_their_last_charge() {
    let (mut world, hero, enemy, _) = carrier(1);
    let full = after_resistance(&world, hero, 300);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(slot_of(&world, hero, 0), None);
}

/// Several carried block one at a time, each on its own charges and cooldown (Dota 7.41e:
/// "Multiple instances of Infused Raindrops no longer lose Magical Damage Block charges
/// simultaneously"): the first blocks and spends a charge, the second keeps its six; the next
/// blow, the first waiting, the second blocks.
#[test]
fn two_infused_raindrops_block_one_at_a_time() {
    let (mut world, hero, enemy, _) = carrier(6);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(fresh_stack(ItemId::INFUSED_RAINDROP, 6));
    }
    let charges_of = |world: &World| -> Vec<_> {
        (0..2)
            .map(|at| slot_of(world, hero, at).map(|held| held.charges))
            .collect()
    };
    let full = after_resistance(&world, hero, 300);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(charges_of(&world), vec![Some(5), Some(6)], "one spends");
    world.fill_pools(hero);
    assert_eq!(magic_blow(&mut world, enemy, hero, 300), full - 120);
    assert_eq!(charges_of(&world), vec![Some(5), Some(5)], "then the other");
}

/// An illusion's Raindrops block nothing (Dota's item: `illusionuse` No): an illusion of the
/// carrier takes an enemy's blow whole, and its copy keeps its charges.
#[test]
fn an_illusions_infused_raindrops_block_nothing() {
    let (mut world, hero, enemy, _) = carrier(6);
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&unit| world.illusion.get(unit).is_some_and(|made| made.of == hero))
        .expect("an illusion");
    world.step();
    assert_eq!(charges(&world, illusion), Some(6), "it carries a copy");
    let taken_bp = world.stats.get(illusion).expect("derived").damage_taken_bp;
    let whole = (i64::from(after_resistance(&world, illusion, 100)) * i64::from(taken_bp)
        / i64::from(rules::NOMINAL_BP)) as i32;
    assert_eq!(magic_blow(&mut world, enemy, illusion, 100), whole);
    assert_eq!(charges(&world, illusion), Some(6));
}

/// The shop has none until 270 seconds into the match, counted from the pregame's start as Dota
/// counts `ItemInitialStockTime` (its 990 s for Aghanim's Shard is the 15:00 of a clock that
/// starts at -1:30), then one, and one more a second after each is bought (`ItemStockInitial` 0,
/// `ItemStockMax` 1, `ItemStockTime` 1).
#[test]
fn infused_raindrops_come_into_the_shop_270_seconds_into_the_match_one_at_a_time() {
    let stock = item_def(ItemId::INFUSED_RAINDROP)
        .and_then(|def| def.stock)
        .expect("a stocked item");
    assert_eq!(stock.opens_ticks, 270 * rules::TICKS_PER_SECOND);
    let (mut world, hero) = a_hero_with_gold(10_000);
    let buy = |world: &mut World| {
        seat_order(
            world,
            Order::Buy {
                item: ItemId::INFUSED_RAINDROP,
            },
        )
        .iter()
        .any(|event| matches!(event.kind(), EventKind::ItemBought { item, .. } if item == ItemId::INFUSED_RAINDROP))
    };
    assert!(!buy(&mut world), "none at the start");
    let now = world.tick;
    run(&mut world, stock.opens_ticks - 2 - now);
    assert!(!world.in_stock(Team::Radiant, ItemId::INFUSED_RAINDROP));
    run(&mut world, 2);
    assert!(world.in_stock(Team::Radiant, ItemId::INFUSED_RAINDROP));
    assert!(buy(&mut world), "one once the shelf opens");
    assert!(!buy(&mut world), "and only one");
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(buy(&mut world), "another a second later");
    assert_eq!(slot_of(&world, hero, 0).map(|held| held.charges), Some(6));
}

/// While carried they add 0.6 mana regeneration a second (`mana_regen`; Dota 7.41f: "Mana Regen
/// bonus decreased from +0.8 to +0.6").
#[test]
fn infused_raindrops_add_their_mana_regeneration() {
    let carried = item_def(ItemId::INFUSED_RAINDROP)
        .expect("in the catalogue")
        .carried;
    let per_second = carried.mana_regen * Fixed::from_int(rules::TICKS_PER_SECOND as i32);
    // A tick's worth, counted in whole steps of the fixed point, misses a second's by less
    // than a step a tick.
    let off = (per_second - Fixed::from_ratio(6, 10)).raw.abs();
    assert!(off <= rules::TICKS_PER_SECOND as i32, "{per_second:?}");
    let (mut world, hero, _, _) = carrier(6);
    let with = world.stats.get(hero).expect("derived").mana_regen;
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = None;
    }
    world.step();
    let gained = with - world.stats.get(hero).expect("derived").mana_regen;
    assert!((gained - carried.mana_regen).raw.abs() <= 1, "{gained:?}");
}
