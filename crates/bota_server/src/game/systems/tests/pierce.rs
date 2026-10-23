//! Pierce: the chance an attacker's swing goes through evasion and lands
//! its bonus, rolled with Dota's pseudo-random distribution, every pierce
//! carried on its own; a structure is never pierced.

use bota_core::{Odds, UnitDef, carried_pierces, rules};

use crate::game::{Entity, Event, UnitOrder, World, wire_id};
use bota_proto::{DamageKind, EventKind, Fixed, HeroId, ItemId, SlotId, Team, UnitKind, Vec2};

use super::support::{ANVIL, assert_dota_runs, hold, opened_rolls, step_until, swing_at};

/// The anvil, standing as a building.
const KEEP: UnitDef = UnitDef {
    kind: UnitKind::Tower,
    move_speed: 0,
    ..ANVIL
};

/// A hero of a kind on the Radiant side, holding what it is given and
/// swinging as fast as it may.
fn hero_holding(world: &mut World, hero: HeroId, held: &[ItemId]) -> Entity {
    let entity = world.spawn_hero(Team::Radiant, Vec2::from_ints(5000, 5000), SlotId(0), hero);
    for (slot, item) in held.iter().enumerate() {
        hold(world, entity, slot, *item);
    }
    swing_at(world, entity, rules::MAX_ATTACK_SPEED);
    entity
}

/// What one tick told of one entity's attacks at another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Told {
    /// Physical blows that landed.
    landed: usize,
    /// Attacks that missed.
    missed: usize,
    /// What each magical bonus a pierce landed took off.
    bonuses: Vec<i32>,
}

/// What a tick's events told of one entity's attacks at another.
fn told_in(events: &[Event], from: Entity, on: Entity) -> Told {
    let mut told = Told::default();
    for event in events {
        match event.kind {
            EventKind::Damaged {
                source,
                target,
                amount,
                kind,
                ..
            } if source == Some(wire_id(from)) && target == wire_id(on) => match kind {
                DamageKind::Physical => told.landed += 1,
                DamageKind::Magical => told.bonuses.push(amount),
                DamageKind::Pure => {}
            },
            EventKind::Missed { source, target }
                if source == Some(wire_id(from)) && target == wire_id(on) =>
            {
                told.missed += 1;
            }
            _ => {}
        }
    }
    told
}

/// What a tick told of one entity's attacks at another, if it told of any.
fn swing_in(events: &[Event], from: Entity, on: Entity) -> Vec<Told> {
    let told = told_in(events, from, on);
    if told == Told::default() {
        Vec::new()
    } else {
        vec![told]
    }
}

/// The first `wanted` ticks that told of a swing.
fn swings(world: &mut World, from: Entity, on: Entity, wanted: usize) -> Vec<Told> {
    let mut seen = Vec::new();
    step_until(world, &mut seen, wanted, |events| {
        swing_in(events, from, on)
    });
    seen
}

#[test]
fn a_monkey_king_bar_pierces_its_share_in_dotas_runs_and_lands_its_bonus() {
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId::PUDGE, &[ItemId::MONKEY_KING_BAR]);
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(pudge, anvil);
    let seen = swings(&mut world, pudge, anvil, 200);
    // Dota's 80% steps three quarters: a swing after one that did not
    // pierce always does.
    assert_dota_runs(
        &seen,
        Odds::percent(80),
        |told| !told.bonuses.is_empty(),
        "the bar",
    );
    for told in &seen {
        assert_eq!(told.landed, 1, "every swing landed: {told:?}");
        for bonus in &told.bonuses {
            assert_eq!(*bonus, 70, "the bonus is the bar's, against no resistance");
        }
    }
}

#[test]
fn a_pierced_swing_is_never_evaded() {
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId::PUDGE, &[ItemId::MONKEY_KING_BAR]);
    let dodger = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    hold(&mut world, dodger, 0, ItemId::BUTTERFLY);
    world.settle();
    world.set_order(dodger, UnitOrder::Stand);
    world.set_target(pudge, dodger);
    // Mended to full before every tick, so the bar never brings her down.
    let mut seen = Vec::new();
    for _ in 0..800 {
        if seen.len() >= 40 {
            break;
        }
        world.fill_pools(dodger);
        seen.extend(swing_in(&world.step(), pudge, dodger));
    }
    assert_eq!(seen.len(), 40, "forty swings were told of");
    for told in &seen {
        if !told.bonuses.is_empty() {
            assert_eq!(
                (told.landed, told.missed),
                (1, 0),
                "a swing that pierced landed: {told:?}"
            );
        }
    }
    let unpierced = seen.iter().filter(|told| told.bonuses.is_empty()).count();
    let missed = seen.iter().filter(|told| told.missed > 0).count();
    assert!(
        missed <= unpierced,
        "only a swing that did not pierce may miss: {missed} of {unpierced}"
    );
}

#[test]
fn a_structure_is_never_pierced() {
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId::PUDGE, &[ItemId::MONKEY_KING_BAR]);
    let keep = world.spawn_unit(&KEEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(pudge, keep);
    let seen = swings(&mut world, pudge, keep, 20);
    assert_eq!(seen.len(), 20, "twenty swings were told of");
    assert!(
        seen.iter().all(|told| told.bonuses.is_empty()),
        "none of them pierced"
    );
    assert!(
        opened_rolls(&world.pierce, pudge).is_none(),
        "and no stream was opened"
    );
}

#[test]
fn every_pierce_carried_rolls_its_own_and_several_bars_pierce_as_one() {
    // Dota: Javelin-based items stack, and several may land on one attack,
    // but several Monkey King Bars pierce as one.
    let rolled = |items: &[ItemId]| {
        carried_pierces(items.iter().copied().enumerate(), false)
            .map(|(at, pierce)| (at, pierce.item, pierce.damage))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rolled(&[ItemId::JAVELIN, ItemId::MONKEY_KING_BAR]),
        [(0, ItemId::JAVELIN, 60), (1, ItemId::MONKEY_KING_BAR, 70)]
    );
    assert_eq!(
        rolled(&[ItemId::JAVELIN, ItemId::JAVELIN]),
        [(0, ItemId::JAVELIN, 60), (1, ItemId::JAVELIN, 60)]
    );
    assert_eq!(
        rolled(&[ItemId::MONKEY_KING_BAR, ItemId::MONKEY_KING_BAR]),
        [(0, ItemId::MONKEY_KING_BAR, 70)]
    );
    let mut world = World::new();
    let both = hero_holding(
        &mut world,
        HeroId::PUDGE,
        &[ItemId::JAVELIN, ItemId::MONKEY_KING_BAR],
    );
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(both, anvil);
    let seen = swings(&mut world, both, anvil, 200);
    assert!(
        seen.iter()
            .any(|told| told.bonuses.contains(&60) && told.bonuses.contains(&70)),
        "the Javelin and the bar land together on some swing"
    );
    assert_dota_runs(
        &seen,
        Odds::percent(25),
        |told| told.bonuses.contains(&60),
        "the javelin",
    );
    assert_dota_runs(
        &seen,
        Odds::percent(80),
        |told| told.bonuses.contains(&70),
        "the bar",
    );
}

#[test]
fn a_monkey_king_bar_reaches_further_in_melee_hands_only() {
    let mut world = World::new();
    let bare = hero_holding(&mut world, HeroId::PUDGE, &[]);
    let armed = hero_holding(&mut world, HeroId::PUDGE, &[ItemId::MONKEY_KING_BAR]);
    let archer = hero_holding(&mut world, HeroId::SHADOW_FIEND, &[]);
    let armed_archer = hero_holding(&mut world, HeroId::SHADOW_FIEND, &[ItemId::MONKEY_KING_BAR]);
    world.settle();
    let reach = |entity: Entity| world.stats.get(entity).expect("settled").attack_range;
    assert_eq!(reach(armed) - reach(bare), Fixed::from_int(50));
    assert_eq!(reach(armed_archer), reach(archer));
}
