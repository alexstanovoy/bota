//! Missiles in the air, and what they leave behind when they arrive.

use std::collections::VecDeque;

use bota_core::{facing_towards, kind_def, move_towards, per_tick};
use bota_proto::{DamageKind, Fixed, Team, UnitKind, Vec2};

use crate::game::{
    Entity, EntityAllocator, Ground, Health, Hit, HitEffect, MatchRng, Missed, Projectile,
    PseudoRandom25, Purpose, Table, Transform, Visibility, wire_id,
};

use super::hitting::entity_slot;

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
    /// Hidden streams used when an attacker first needs an uphill sequence.
    pub rng: &'a MatchRng,
    /// Uphill miss sequence per attacker slot.
    pub uphill_miss: &'a mut Vec<Option<PseudoRandom25>>,
    /// Where an arriving missile leaves its blow.
    pub hits: &'a mut VecDeque<Hit>,
    /// Where one that missed uphill is left for whatever answers to it.
    pub missed: &'a mut VecDeque<Missed>,
}

/// Moves every missile along and turns the ones that arrive into blows.
///
/// A missile whose target has fallen is given up in the air. One that misses
/// uphill is given up and left on `missed`.
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
        rng,
        uphill_miss,
        hits,
        missed,
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
        if !shot.pierces && misses_uphill(&shot, to, transform, kind, ground, rng, uphill_miss) {
            missed.push_back(Missed {
                source: shot.source,
                target: shot.target,
                at: to,
                side: team.get(shot.target).copied().unwrap_or(Team::Neutral),
            });
        } else {
            land(&shot, hits);
        }
        give_up(missile, entities, projectile, transform, team, visibility);
    }
}

/// Lays the blows a missile that arrived deals: its own, and the magical
/// pierce riding with it.
fn land(shot: &Projectile, hits: &mut VecDeque<Hit>) {
    hits.push_back(Hit {
        source: shot.source,
        target: shot.target,
        amount: shot.damage,
        kind: shot.kind,
        damage_amp_bp: shot.damage_amp_bp,
        attack: true,
        pierces: shot.pierces,
        effect: shot.effect,
    });
    if shot.pierce_damage > 0 {
        hits.push_back(Hit {
            source: shot.source,
            target: shot.target,
            amount: shot.pierce_damage,
            kind: DamageKind::Magical,
            damage_amp_bp: shot.pierce_amp_bp,
            attack: false,
            pierces: false,
            effect: HitEffect::None,
        });
    }
}

fn misses_uphill(
    shot: &Projectile,
    target_pos: Vec2,
    transform: &Table<Transform>,
    kind: &Table<UnitKind>,
    ground: &Ground,
    rng: &MatchRng,
    uphill_miss: &mut Vec<Option<PseudoRandom25>>,
) -> bool {
    let source_tier = shot
        .source
        .and_then(|source| transform.get(source))
        .map_or(shot.launch_tier, |source| ground.tier(source.pos));
    if !shot.can_miss_uphill
        || kind
            .get(shot.target)
            .copied()
            .is_some_and(|kind| kind_def(kind).structure)
        || ground.tier(target_pos) <= source_tier
    {
        return false;
    }
    let Some(source) = shot.source else {
        return false;
    };
    let chance = entity_slot(uphill_miss, source, || {
        PseudoRandom25::new(rng.for_unit(Purpose::Evasion, wire_id(source), 0))
    });
    chance.roll()
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
