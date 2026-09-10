use bota_proto::{
    Angle, DamageKind, EffectId, EffectView, EventKind, Fixed, HeroId, SlotId, Team, Vec2,
};

use crate::game::{Entity, Event, Stats, UnitOrder, World, wire_id};

pub(super) const RAZE_EFFECT: EffectId = EffectId(15);
pub(super) const ORIGIN: Vec2 = Vec2::from_ints(5000, 5000);
pub(super) const TARGET: Vec2 = Vec2::from_ints(5450, 5000);

pub(super) fn fixture() -> (World, Entity, Entity) {
    let mut world = World::new();
    let caster = hero(&mut world, Team::Radiant, ORIGIN, SlotId(0));
    let target = hero(&mut world, Team::Dire, TARGET, SlotId(1));
    assert!(world.alive(caster));
    assert!(world.alive(target));
    (world, caster, target)
}

pub(super) fn hero(world: &mut World, side: Team, pos: Vec2, slot: SlotId) -> Entity {
    let entity = world.spawn_hero(side, pos, slot, HeroId(2));
    prepare(world, entity, pos);
    entity
}

pub(super) fn prepare(world: &mut World, entity: Entity, pos: Vec2) {
    world.def.remove(entity);
    world.attacking.remove(entity);
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
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        damage: 0,
        attack_range: Fixed::ZERO,
        acquisition: Fixed::ZERO,
        attack_interval: 30,
        attack_speed: 100,
        attributes: bota_proto::Attributes::ZERO,
        primary: None,
        attack_point: 0,
        attack_backswing: 0,
        projectile_speed: None,
        armor: Fixed::ZERO,
        magic_resist_pct: 0,
        move_speed: Fixed::ZERO,
        turn_rate: 0,
        damage_to_creeps: 0,
        vision: Fixed::from_int(2000),
        true_sight: Fixed::ZERO,
        hides: false,
        flies: false,
        phased: false,
        invulnerable: false,
    }
}

pub(super) fn reveal(world: &mut World, entity: Entity) {
    let seen = world.visibility.get_mut(entity).unwrap();
    seen.add(Team::Radiant);
    seen.add(Team::Dire);
    seen.add(Team::Neutral);
}

pub(super) fn damage(events: &[Event], target: Entity) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged {
                target: on,
                amount,
                kind: DamageKind::Magical,
                ..
            } if on == wire_id(target) => Some(amount),
            _ => None,
        })
        .collect()
}

pub(super) fn land(world: &mut World, caster: Entity, target: Entity, level: usize) -> i32 {
    reveal(world, target);
    assert!(world.cast_raze(caster, level, 1));
    let amounts = damage(&world.step(), target);
    assert_eq!(amounts.len(), 1);
    amounts[0]
}

pub(super) fn effects(world: &World, target: Entity) -> Vec<EffectView> {
    world
        .view_full()
        .units
        .iter()
        .find(|unit| unit.id == wire_id(target))
        .map_or_else(Vec::new, |unit| {
            unit.effects
                .iter()
                .copied()
                .filter(|effect| effect.id == RAZE_EFFECT)
                .collect()
        })
}

pub(super) fn effect(stacks: u32, ticks: u32) -> EffectView {
    EffectView {
        id: RAZE_EFFECT,
        ticks_left: Some(ticks),
        stacks: Some(stacks),
    }
}
