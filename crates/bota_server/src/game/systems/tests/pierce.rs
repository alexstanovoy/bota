//! Pierce: the share of an attacker's swings that go through evasion and
//! land their bonus, held exactly; a structure is never pierced.

use bota_core::{Ratio, UnitDef, rules};

use crate::game::{
    Entity, Event, ITEM_BUTTERFLY, ITEM_JAVELIN, ITEM_MONKEY_KING_BAR, UnitOrder, World, wire_id,
};
use bota_proto::{DamageKind, EventKind, Fixed, HeroId, SlotId, Team, UnitKind, Vec2};

use super::support::{
    ANVIL, assert_exact_blocks, hold, opened, step_until, swinging_at, until_opened,
};

/// The anvil, standing as a building.
const KEEP: UnitDef = UnitDef {
    kind: UnitKind::Tower,
    move_speed: 0,
    ..ANVIL
};

/// A hero of a kind on the Radiant side, holding what it is given and
/// swinging as fast as it may.
fn hero_holding(world: &mut World, hero: HeroId, held: &[u16]) -> Entity {
    let entity = world.spawn_hero(Team::Radiant, Vec2::from_ints(5000, 5000), SlotId(0), hero);
    for (slot, item) in held.iter().enumerate() {
        hold(world, entity, slot, *item);
    }
    world
        .modifiers
        .insert(entity, swinging_at(rules::MAX_ATTACK_SPEED));
    entity
}

/// What one tick told of one entity's attacks at another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Told {
    /// Physical blows that landed.
    landed: usize,
    /// Attacks that missed.
    missed: usize,
    /// What the magical bonus a pierce landed took off, if one did.
    bonus: Option<i32>,
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
                DamageKind::Magical => told.bonus = Some(amount),
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
fn a_monkey_king_bar_pierces_exactly_its_share_and_lands_its_bonus() {
    let ratio = Ratio::new(4, 5);
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId(0), &[ITEM_MONKEY_KING_BAR]);
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(pudge, anvil);
    let told = |events: &[Event]| swing_in(events, pudge, anvil);
    let (mut seen, first) =
        until_opened(&mut world, 40, told, |world| opened(&world.pierce, pudge));
    assert_eq!(
        seen.len(),
        1,
        "the first swing was told of the tick it rolled"
    );
    step_until(
        &mut world,
        &mut seen,
        first + 5 * usize::from(ratio.den()),
        told,
    );
    assert_exact_blocks(&seen, first, ratio, |told| told.bonus.is_some(), "the bar");
    for told in &seen {
        assert_eq!(told.landed, 1, "every swing landed: {told:?}");
        if let Some(bonus) = told.bonus {
            assert_eq!(bonus, 70, "the bonus is the bar's, against no resistance");
        }
    }
}

#[test]
fn a_pierced_swing_is_never_evaded() {
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId(0), &[ITEM_MONKEY_KING_BAR]);
    let dodger = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(1),
        HeroId(1),
    );
    hold(&mut world, dodger, 0, ITEM_BUTTERFLY);
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
        if told.bonus.is_some() {
            assert_eq!(
                (told.landed, told.missed),
                (1, 0),
                "a swing that pierced landed: {told:?}"
            );
        }
    }
    let unpierced = seen.iter().filter(|told| told.bonus.is_none()).count();
    let missed = seen.iter().filter(|told| told.missed > 0).count();
    assert!(
        missed <= unpierced,
        "only a swing that did not pierce may miss: {missed} of {unpierced}"
    );
}

#[test]
fn a_structure_is_never_pierced() {
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId(0), &[ITEM_MONKEY_KING_BAR]);
    let keep = world.spawn_unit(&KEEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(pudge, keep);
    let seen = swings(&mut world, pudge, keep, 20);
    assert_eq!(seen.len(), 20, "twenty swings were told of");
    assert!(
        seen.iter().all(|told| told.bonus.is_none()),
        "none of them pierced"
    );
    assert!(
        opened(&world.pierce, pudge).is_none(),
        "and no stream was opened"
    );
}

#[test]
fn of_two_pierces_carried_the_better_counts_with_its_own_damage() {
    let mut world = World::new();
    let both = hero_holding(&mut world, HeroId(0), &[ITEM_JAVELIN, ITEM_MONKEY_KING_BAR]);
    world.settle();
    let stats = *world.stats.get(both).expect("settled");
    assert_eq!((stats.pierce, stats.pierce_damage), (Ratio::new(4, 5), 70));
    let mut world = World::new();
    let javelin = hero_holding(&mut world, HeroId(0), &[ITEM_JAVELIN]);
    world.settle();
    let stats = *world.stats.get(javelin).expect("settled");
    assert_eq!((stats.pierce, stats.pierce_damage), (Ratio::new(1, 4), 60));
}

#[test]
fn a_monkey_king_bar_reaches_further_in_melee_hands_only() {
    let mut world = World::new();
    let bare = hero_holding(&mut world, HeroId(0), &[]);
    let armed = hero_holding(&mut world, HeroId(0), &[ITEM_MONKEY_KING_BAR]);
    let archer = hero_holding(&mut world, HeroId(1), &[]);
    let armed_archer = hero_holding(&mut world, HeroId(1), &[ITEM_MONKEY_KING_BAR]);
    world.settle();
    let reach = |entity: Entity| world.stats.get(entity).expect("settled").attack_range;
    assert_eq!(reach(armed) - reach(bare), Fixed::from_int(50));
    assert_eq!(reach(armed_archer), reach(archer));
}
