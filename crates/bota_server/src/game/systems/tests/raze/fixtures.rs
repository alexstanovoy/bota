use bota_core::rules;

use crate::game::tests::support;
pub(super) use crate::game::tests::support::{advance_validated, burns_on, shown};
use crate::game::{Entity, Seat, Stats, UnitOrder, World, wire_id};
use bota_proto::{Angle, EffectId, EffectView, Fixed, HeroId, SlotId, Team, Vec2};

pub(super) const RAZE_EFFECT: EffectId = EffectId::SHADOWRAZE;
pub(super) const ORIGIN: Vec2 = Vec2::from_ints(5000, 5000);
pub(super) const TARGET: Vec2 = Vec2::from_ints(5450, 5000);

pub(super) fn fixture() -> (World, Entity, Entity) {
    let mut world = World::new();
    let caster = hero(&mut world, Team::Radiant, ORIGIN, SlotId(0));
    let target = hero(&mut world, Team::Dire, TARGET, SlotId(1));
    // Sight worked out once from where they stand, as the views read it,
    // and both shown to every side.
    world.settle();
    reveal(&mut world, caster);
    reveal(&mut world, target);
    assert!(world.alive(caster));
    assert!(world.alive(target));
    (world, caster, target)
}

pub(super) fn hero(world: &mut World, side: Team, pos: Vec2, slot: SlotId) -> Entity {
    let entity = world.spawn_hero(side, pos, slot, HeroId::SHADOW_FIEND);
    prepare(world, entity, pos);
    entity
}

pub(super) fn prepare(world: &mut World, entity: Entity, pos: Vec2) {
    world.def.remove(entity);
    world.action.remove(entity);
    world.hull.remove(entity);
    world.stats.insert(entity, stats());
    world.fill_pools(entity);
    world.set_order(entity, UnitOrder::Stand);
    let transform = world.transform.get_mut(entity).unwrap();
    transform.pos = pos;
    transform.facing = Angle { brads: 0 };
    for slot in &mut world.abilities.get_mut(entity).unwrap().slots[..3] {
        slot.level = 1;
    }
    reveal(world, entity);
    assert!(world.alive(entity));
    assert_eq!(world.health.get(entity).unwrap().hp, Fixed::from_int(30000));
}

fn stats() -> Stats {
    Stats {
        max_hp: Fixed::from_int(30000),
        max_mana: Fixed::from_int(30000),
        vision: Fixed::from_int(2000),
        ..support::stats()
    }
}

/// Puts `unit` in a new seat at `slot` holding `gold`.
pub(super) fn seat(world: &mut World, slot: SlotId, team: Team, unit: Entity, gold: i32) {
    let mut seat = Seat::new(slot, team, HeroId::SHADOW_FIEND, gold, rules::STASH_SLOTS);
    seat.unit = Some(unit);
    world.seats.push(seat);
}

pub(super) fn reveal(world: &mut World, entity: Entity) {
    let seen = world.visibility.get_mut(entity).unwrap();
    seen.add(Team::Radiant);
    seen.add(Team::Dire);
    seen.add(Team::Neutral);
}

pub(super) fn land(world: &mut World, caster: Entity, target: Entity, level: usize) -> i32 {
    reveal(world, target);
    assert!(world.cast_raze(caster, level, 1));
    let amounts = burns_on(&world.step(), target);
    assert_eq!(amounts.len(), 1);
    amounts[0]
}

/// The raze records on a unit as these tests count them: stacks and time
/// left, without who put them on and with what.
pub(super) fn effects(world: &World, target: Entity) -> Vec<EffectView> {
    world
        .view_full()
        .units
        .iter()
        .find(|unit| unit.id == wire_id(target))
        .map_or_else(Vec::new, |unit| {
            unit.effects
                .iter()
                .filter(|effect| effect.id == RAZE_EFFECT)
                .map(|effect| EffectView {
                    caster: None,
                    inflictor: None,
                    ticks_total: None,
                    ..*effect
                })
                .collect()
        })
}

/// A raze record as [`effects`] counts it.
pub(super) fn effect(stacks: u32, ticks: u32) -> EffectView {
    EffectView {
        id: RAZE_EFFECT,
        ticks_left: Some(ticks),
        stacks: Some(stacks),
        caster: None,
        inflictor: None,
        ticks_total: None,
    }
}
