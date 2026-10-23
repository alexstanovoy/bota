//! Missiles in the air, and what they leave behind when they arrive.

use std::collections::VecDeque;

use bota_core::{facing_towards, kind_def, move_towards, per_tick};
use bota_proto::{DamageKind, Fixed, Inflictor, Team, UnitKind, Vec2};

use crate::game::{
    Entity, EntityAllocator, Ground, Health, Hit, HitEffect, Projectile, Table, Transform,
    Visibility,
};

/// What flight reads and writes.
pub struct MissileCx<'a> {
    /// Which entities exist; a missile given up is freed here.
    pub entities: &'a mut EntityAllocator,
    pub projectile: &'a mut Table<Projectile>,
    /// Where everything stands, missiles included.
    pub transform: &'a mut Table<Transform>,
    pub team: &'a mut Table<Team>,
    /// Who sees what; a missile given up loses its row here.
    pub visibility: &'a mut Table<Visibility>,
    /// Health, for telling whether a missile still has anybody to reach.
    pub health: &'a Table<Health>,
    /// What each target is: a structure never evades an uphill attack.
    pub kind: &'a Table<UnitKind>,
    /// Elevation under a missile's source and target when it arrives.
    pub ground: &'a Ground,
    /// Where an arriving missile leaves its blow.
    pub hits: &'a mut VecDeque<Hit>,
}

/// Moves every missile along and turns the ones that arrive into blows.
///
/// A missile whose target has fallen is given up in the air. One that
/// arrives thrown up at higher ground says so on its blow, whose target
/// evades it with the uphill miss besides its own evasion.
pub fn missile_system(cx: MissileCx<'_>) {
    let MissileCx {
        entities,
        projectile,
        transform,
        team,
        visibility,
        health,
        kind,
        ground,
        hits,
    } = cx;
    for missile in entities.iter().collect::<Vec<_>>() {
        let Some(shot) = projectile.get(missile).cloned() else {
            continue;
        };
        let standing = health
            .get(shot.target)
            .is_some_and(|health| health.hp > Fixed::ZERO);
        let to = transform.get(shot.target).map(|t| t.pos);
        let (true, Some(to)) = (standing, to) else {
            give_up(missile, entities, projectile, transform, team, visibility);
            continue;
        };
        let Some(at) = transform.get_mut(missile) else {
            continue;
        };
        let next = move_towards(at.pos, to, per_tick(shot.speed));
        at.pos = next;
        at.facing = facing_towards(next, to);
        if next != to {
            continue;
        }
        let uphill = !shot.pierces && thrown_uphill(&shot, to, transform, kind, ground);
        land(&shot, uphill, hits);
        give_up(missile, entities, projectile, transform, team, visibility);
    }
}

/// Lays the blows a missile that arrived deals: its own, and the magical
/// damage of each pierce riding with it.
fn land(shot: &Projectile, uphill: bool, hits: &mut VecDeque<Hit>) {
    hits.push_back(Hit {
        source: shot.source,
        inflictor: shot.inflictor,
        target: shot.target,
        amount: shot.damage,
        kind: shot.kind,
        damage_amp_bp: shot.damage_amp_bp,
        attack: true,
        pierces: shot.pierces,
        uphill,
        effect: shot.effect,
    });
    for &(damage, item) in &shot.pierced {
        hits.push_back(Hit {
            source: shot.source,
            inflictor: Some(Inflictor::Item(item)),
            target: shot.target,
            amount: damage,
            kind: DamageKind::Magical,
            damage_amp_bp: shot.pierce_amp_bp,
            attack: false,
            pierces: false,
            uphill: false,
            effect: HitEffect::None,
        });
    }
}

/// Whether a missile arriving at `target_pos` was thrown up at higher
/// ground: its attacker, where it stands now (where it was thrown from once
/// gone), lower than its target, which is no structure, and the missile one
/// that can miss so.
fn thrown_uphill(
    shot: &Projectile,
    target_pos: Vec2,
    transform: &Table<Transform>,
    kind: &Table<UnitKind>,
    ground: &Ground,
) -> bool {
    let source_tier = shot
        .source
        .and_then(|source| transform.get(source))
        .map_or(shot.launch_tier, |source| ground.tier(source.pos));
    shot.can_miss_uphill
        && shot.source.is_some()
        && !kind
            .get(shot.target)
            .copied()
            .is_some_and(|kind| kind_def(kind).structure)
        && ground.tier(target_pos) > source_tier
}

/// Takes a missile out of the air and out of the world.
fn give_up(
    missile: Entity,
    entities: &mut EntityAllocator,
    projectile: &mut Table<Projectile>,
    transform: &mut Table<Transform>,
    team: &mut Table<Team>,
    visibility: &mut Table<Visibility>,
) {
    projectile.remove(missile);
    transform.remove(missile);
    team.remove(missile);
    visibility.remove(missile);
    entities.free(missile);
}
