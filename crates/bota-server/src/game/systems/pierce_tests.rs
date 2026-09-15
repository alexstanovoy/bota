//! Pierce: the share of an attacker's swings that go through evasion and
//! land their bonus, held exactly; a structure is never pierced.

use bota_proto::{DamageKind, EventKind, Fixed, HeroId, ItemId, SlotId, Team, UnitKind, Vec2};

use crate::game::{
    Entity, Event, ITEM_BUTTERFLY, ITEM_JAVELIN, ITEM_MONKEY_KING_BAR, ItemStack, MELEE_CREEP,
    Modifier, ModifierKind, Modifiers, Ratio, UnitDef, UnitOrder, World, rules, wire_id,
};

/// What takes a strike and never falls: no armor, no mending, no swing back.
const ANVIL: UnitDef = UnitDef {
    max_hp: 30_000,
    armor: 0,
    damage: 0,
    hp_regen: Fixed::ZERO,
    ..MELEE_CREEP
};

/// The same, standing as a building.
const KEEP: UnitDef = UnitDef {
    kind: UnitKind::Tower,
    move_speed: 0,
    ..ANVIL
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

/// A hero of a kind on the Radiant side, holding what it is given and
/// swinging as fast as it may.
fn hero_holding(world: &mut World, hero: HeroId, held: &[u16]) -> Entity {
    let entity = world.spawn_hero(Team::Radiant, Vec2::from_ints(5000, 5000), SlotId(0), hero);
    for (slot, item) in held.iter().enumerate() {
        hold(world, entity, slot, *item);
    }
    world.modifiers.insert(entity, fastest());
    entity
}

/// What one tick told of one entity's attacks at another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Told {
    /// Physical blows that landed.
    landed: usize,
    /// Attacks that missed.
    missed: usize,
    /// Magical bonuses landed by a pierce, and what each took off.
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

/// Steps until so many ticks have told of a swing, keeping what each told.
fn swings(world: &mut World, from: Entity, on: Entity, wanted: usize) -> Vec<Told> {
    let mut seen = Vec::new();
    for _ in 0..800 {
        if seen.len() >= wanted {
            break;
        }
        let told = told_in(&world.step(), from, on);
        if told != Told::default() {
            seen.push(told);
        }
    }
    seen
}

/// The stream of pierce rolls an attacker has opened, if it has.
fn pierce_stream(world: &World, attacker: Entity) -> Option<&crate::game::Chance> {
    world
        .pierce
        .get(attacker.index().0 as usize)
        .and_then(|chance| chance.as_ref())
}

#[test]
fn a_monkey_king_bar_pierces_exactly_its_share_and_lands_its_bonus() {
    let ratio = Ratio::new(4, 5);
    let (num, den) = (usize::from(ratio.num()), usize::from(ratio.den()));
    let mut world = World::new();
    let pudge = hero_holding(&mut world, HeroId(1), &[ITEM_MONKEY_KING_BAR]);
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_target(pudge, anvil);
    // Step to the first swing, which opens the stream and says where in
    // its block it opened.
    let mut seen = Vec::new();
    let mut first = None;
    for _ in 0..40 {
        let told = told_in(&world.step(), pudge, anvil);
        if told != Told::default() {
            seen.push(told);
        }
        if let Some(chance) = pierce_stream(&world, pudge) {
            first = Some(match usize::from(chance.block_position()) {
                0 => 1,
                pos => den - (pos - 1),
            });
            break;
        }
    }
    let first = first.expect("a swing came within forty ticks");
    assert_eq!(
        seen.len(),
        1,
        "the first swing was told of the tick it rolled"
    );
    seen.extend(swings(&mut world, pudge, anvil, first + den * 5));
    let full = seen[first..].chunks_exact(den);
    assert!(full.len() >= 5, "five full blocks of swings were told of");
    for (nth, block) in full.enumerate() {
        let pierced = block.iter().filter(|told| told.bonus.is_some()).count();
        assert_eq!(pierced, num, "block {nth}: {block:?}");
    }
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
    let pudge = hero_holding(&mut world, HeroId(1), &[ITEM_MONKEY_KING_BAR]);
    let sylla = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(1),
        HeroId(0),
    );
    hold(&mut world, sylla, 0, ITEM_BUTTERFLY);
    world.settle();
    world.set_order(sylla, UnitOrder::Stand);
    world.set_target(pudge, sylla);
    // Mended to full before every tick, so the bar never brings her down.
    let mut seen = Vec::new();
    for _ in 0..800 {
        if seen.len() >= 40 {
            break;
        }
        world.fill_pools(sylla);
        let told = told_in(&world.step(), pudge, sylla);
        if told != Told::default() {
            seen.push(told);
        }
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
    let pudge = hero_holding(&mut world, HeroId(1), &[ITEM_MONKEY_KING_BAR]);
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
        pierce_stream(&world, pudge).is_none(),
        "and no stream was opened"
    );
}

#[test]
fn of_two_pierces_carried_the_better_counts_with_its_own_damage() {
    let mut world = World::new();
    let both = hero_holding(&mut world, HeroId(1), &[ITEM_JAVELIN, ITEM_MONKEY_KING_BAR]);
    world.settle();
    let stats = *world.stats.get(both).expect("settled");
    assert_eq!((stats.pierce, stats.pierce_damage), (Ratio::new(4, 5), 70));
    let mut world = World::new();
    let javelin = hero_holding(&mut world, HeroId(1), &[ITEM_JAVELIN]);
    world.settle();
    let stats = *world.stats.get(javelin).expect("settled");
    assert_eq!((stats.pierce, stats.pierce_damage), (Ratio::new(1, 4), 60));
}

#[test]
fn a_monkey_king_bar_reaches_further_in_melee_hands_only() {
    let mut world = World::new();
    let bare = hero_holding(&mut world, HeroId(1), &[]);
    let armed = hero_holding(&mut world, HeroId(1), &[ITEM_MONKEY_KING_BAR]);
    let archer = hero_holding(&mut world, HeroId(0), &[]);
    let armed_archer = hero_holding(&mut world, HeroId(0), &[ITEM_MONKEY_KING_BAR]);
    world.settle();
    let reach = |entity: Entity| world.stats.get(entity).expect("settled").attack_range;
    assert_eq!(reach(armed) - reach(bare), Fixed::from_int(50));
    assert_eq!(reach(armed_archer), reach(archer));
}
