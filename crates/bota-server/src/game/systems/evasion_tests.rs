//! Evasion: the share of attacks at a carrier that miss, held exactly and
//! told of; what is not an attack never misses.

use bota_proto::{DamageKind, EventKind, Fixed, HeroId, ItemId, SlotId, Team, Vec2};

use crate::game::{
    Entity, Event, ITEM_BUTTERFLY, ITEM_TALISMAN_OF_EVASION, ItemStack, MELEE_CREEP, Modifier,
    ModifierKind, Modifiers, RANGED_CREEP, Ratio, UnitDef, World, rules, wire_id,
};

/// A melee attacker worth a point a strike on a short cycle, never landing
/// two strikes in one tick, never falling and never bringing a hero down.
const STRIKER: UnitDef = UnitDef {
    max_hp: 30_000,
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 1,
    ..MELEE_CREEP
};

/// One that never swings at all.
const BYSTANDER: UnitDef = UnitDef {
    damage: 0,
    ..STRIKER
};

/// The same, throwing its strikes.
const ARCHER: UnitDef = UnitDef {
    max_hp: 30_000,
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 1,
    projectile_speed: Some(6_000),
    ..RANGED_CREEP
};

/// Swinging as fast as a body may.
fn fastest() -> Modifiers {
    Modifiers(vec![Modifier {
        kind: ModifierKind::Haste {
            speed: rules::MAX_ATTACK_SPEED - rules::BASE_ATTACK_SPEED,
        },
        source: None,
        ticks_left: Some(u32::MAX / 2),
    }])
}

/// Puts an item in one of a hero's slots.
fn hold(world: &mut World, hero: Entity, slot: usize, item: u16) {
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[slot] = Some(ItemStack {
            id: ItemId(item),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: SlotId(0),
            for_sale: false,
        });
    }
}

/// An attacker of a kind set on a hero in reach, the hero holding what it
/// is given.
fn attacker_at_a_hero(attacker: &'static UnitDef, held: &[u16]) -> (World, Entity, Entity) {
    let mut world = World::new();
    let from = world.spawn_unit(attacker, Team::Radiant, Vec2::from_ints(5000, 5000));
    let mark = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(0),
        HeroId(1),
    );
    for (slot, item) in held.iter().enumerate() {
        hold(&mut world, mark, slot, *item);
    }
    world.modifiers.insert(from, fastest());
    world.settle();
    world.fill_pools(mark);
    world.set_target(from, mark);
    (world, from, mark)
}

/// What one entity's attacks at another came to among a tick's events, in
/// order: true for a blow that landed, false for one that missed.
fn swings_in(events: &[Event], from: Entity, on: Entity) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged { source, target, .. }
                if source == Some(wire_id(from)) && target == wire_id(on) =>
            {
                Some(true)
            }
            EventKind::Missed { source, target }
                if source == Some(wire_id(from)) && target == wire_id(on) =>
            {
                Some(false)
            }
            _ => None,
        })
        .collect()
}

/// Steps until so many swings have been told of, adding them to what was
/// seen.
fn swing(world: &mut World, from: Entity, on: Entity, seen: &mut Vec<bool>, wanted: usize) {
    for _ in 0..800 {
        if seen.len() >= wanted {
            return;
        }
        seen.extend(swings_in(&world.step(), from, on));
    }
}

/// The stream of evasion rolls a target has opened, if it has.
fn evasion_stream(world: &World, target: Entity) -> Option<&crate::game::Chance> {
    world
        .evasion
        .get(target.index().0 as usize)
        .and_then(|chance| chance.as_ref())
}

/// How many rolls of the first block the first roll left, itself included.
fn first_block_len(world: &World, target: Entity, den: usize) -> usize {
    let chance = evasion_stream(world, target).expect("the first swing opened the stream");
    match usize::from(chance.block_position()) {
        0 => 1,
        pos => den - (pos - 1),
    }
}

/// Runs an attacker at a hero holding a Butterfly and checks that every
/// full block of swings carries exactly the Butterfly's share of misses.
fn misses_hold_exactly(attacker: &'static UnitDef) {
    let ratio = Ratio::new(7, 20);
    let (num, den) = (usize::from(ratio.num()), usize::from(ratio.den()));
    let (mut world, from, mark) = attacker_at_a_hero(attacker, &[ITEM_BUTTERFLY]);
    assert_eq!(
        world.stats.get(mark).map(|stats| stats.evasion),
        Some(ratio),
        "the Butterfly is worth its evasion"
    );
    // Step to the first swing that arrives, which opens the stream.
    let mut swings = Vec::new();
    let mut first = None;
    for _ in 0..40 {
        swings.extend(swings_in(&world.step(), from, mark));
        if evasion_stream(&world, mark).is_some() {
            first = Some(first_block_len(&world, mark, den));
            break;
        }
    }
    let first = first.expect("a swing arrived within forty ticks");
    assert_eq!(
        swings.len(),
        1,
        "the first swing was told of the tick it rolled"
    );
    swing(&mut world, from, mark, &mut swings, first + den * 5);
    let full = swings[first..].chunks_exact(den);
    assert!(full.len() >= 5, "five full blocks of swings were told of");
    for (nth, block) in full.enumerate() {
        let missed = block.iter().filter(|landed| !**landed).count();
        assert_eq!(missed, num, "block {nth}: {block:?}");
    }
}

#[test]
fn a_butterfly_makes_exactly_its_share_of_melee_swings_miss() {
    misses_hold_exactly(&STRIKER);
}

#[test]
fn a_butterfly_makes_exactly_its_share_of_thrown_swings_miss() {
    misses_hold_exactly(&ARCHER);
}

#[test]
fn what_is_not_an_attack_never_misses() {
    let (mut world, from, mark) = attacker_at_a_hero(&BYSTANDER, &[ITEM_BUTTERFLY]);
    let mut blows = Vec::new();
    for _ in 0..40 {
        world.push_hit(Some(from), mark, 10, DamageKind::Magical);
        blows.extend(swings_in(&world.step(), from, mark));
    }
    assert_eq!(blows.len(), 40, "every blow of the spell was told of");
    assert!(
        blows.iter().all(|landed| *landed),
        "and none of them missed"
    );
}

#[test]
fn without_evasion_nothing_is_rolled_and_nothing_misses() {
    let (mut world, from, mark) = attacker_at_a_hero(&STRIKER, &[]);
    let mut swings = Vec::new();
    swing(&mut world, from, mark, &mut swings, 30);
    assert_eq!(swings.len(), 30, "thirty swings were told of");
    assert!(swings.iter().all(|landed| *landed), "all of them landed");
    assert!(
        evasion_stream(&world, mark).is_none(),
        "and no stream was opened"
    );
}

#[test]
fn of_two_evasions_carried_the_better_counts() {
    let (world, _from, mark) =
        attacker_at_a_hero(&STRIKER, &[ITEM_TALISMAN_OF_EVASION, ITEM_BUTTERFLY]);
    assert_eq!(
        world.stats.get(mark).map(|stats| stats.evasion),
        Some(Ratio::new(7, 20))
    );
    let (world, _from, mark) = attacker_at_a_hero(&STRIKER, &[ITEM_TALISMAN_OF_EVASION]);
    assert_eq!(
        world.stats.get(mark).map(|stats| stats.evasion),
        Some(Ratio::new(3, 20))
    );
}

#[test]
fn a_butterfly_is_worth_its_agility_damage_and_share_of_the_base_pace() {
    let (world, _from, plain) = attacker_at_a_hero(&STRIKER, &[]);
    let bare = *world.stats.get(plain).expect("settled");
    let (world, _from, mark) = attacker_at_a_hero(&STRIKER, &[ITEM_BUTTERFLY]);
    let with = *world.stats.get(mark).expect("settled");
    assert_eq!(
        with.attributes.agility - bare.attributes.agility,
        Fixed::from_int(30)
    );
    // Thirty of its own; Pudge's primary is strength, so agility pays no
    // damage.
    assert_eq!(with.damage - bare.damage, 30);
    let agility_pace =
        (with.attributes.agility * Fixed::from_int(rules::ATTACK_SPEED_PER_AGILITY)).to_int();
    let base = rules::BASE_ATTACK_SPEED + agility_pace;
    assert_eq!(
        with.attack_speed,
        base + base * 20 / 100,
        "a fifth of the base pace and of what agility adds, and nothing else"
    );
}
