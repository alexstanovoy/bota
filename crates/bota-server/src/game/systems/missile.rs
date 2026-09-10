//! Missiles in the air, and what they leave behind when they arrive.

use std::collections::VecDeque;

use bota_proto::{Fixed, Team, UnitKind};

use crate::game::{
    Entity, EntityAllocator, Ground, Health, Hit, MatchRng, Projectile, PseudoRandom25, Purpose,
    Table, Transform, Visibility, World, is_structure, rules, wire_id,
};
use crate::game::{facing_towards, move_towards, per_tick};

/// What flight reads and writes.
pub struct MissileCx<'a> {
    /// Which entities exist, where a missile is given up and a blow is made.
    pub entities: &'a mut EntityAllocator,
    /// The missiles themselves.
    pub projectile: &'a mut Table<Projectile>,
    /// Where everything stands, missiles included.
    pub transform: &'a mut Table<Transform>,
    /// Which side each entity is on.
    pub team: &'a mut Table<Team>,
    /// Who sees what, so a new blow carries nothing to see.
    pub visibility: &'a mut Table<Visibility>,
    /// Health, for telling whether a missile still has anybody to reach.
    pub health: &'a Table<Health>,
    /// What each target is, so buildings never evade uphill attacks.
    pub kind: &'a Table<UnitKind>,
    /// Elevation under a missile's target when it arrives.
    pub ground: &'a Ground,
    /// Hidden streams used when an attacker first needs an uphill sequence.
    pub rng: &'a MatchRng,
    /// Uphill miss sequence per attacker slot.
    pub uphill_miss: &'a mut Vec<Option<PseudoRandom25>>,
    /// Where an arriving missile leaves its blow.
    pub hits: &'a mut VecDeque<Hit>,
    /// Where one that still has a bounce in it is left to be sent on.
    pub bounced: &'a mut VecDeque<(Entity, Entity)>,
}

/// Moves every missile along and turns the ones that arrive into blows.
///
/// A missile whose target has fallen is given up in the air: nothing it was
/// aimed at is left to reach. One that lands with a bounce still in it is
/// left on the bounce queue rather than given up.
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
        bounced,
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
        if misses_uphill(&shot, to, transform, kind, ground, rng, uphill_miss) {
            give_up(missile, entities, projectile, transform, team, visibility);
            continue;
        }
        hits.push_back(Hit {
            source: shot.source,
            target: shot.target,
            amount: shot.damage,
            kind: shot.kind,
            crit: shot.crit,
            effect: crate::game::HitEffect::None,
        });
        // One with bounces left is kept where it landed: where it goes next
        // is settled once it is known what stands there.
        if shot.bounces_left > 0 {
            bounced.push_back((missile, shot.target));
            continue;
        }
        give_up(missile, entities, projectile, transform, team, visibility);
    }
}

fn misses_uphill(
    shot: &Projectile,
    target_pos: bota_proto::Vec2,
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
        || kind.get(shot.target).copied().is_some_and(is_structure)
        || ground.tier(target_pos) <= source_tier
    {
        return false;
    }
    let Some(source) = shot.source else {
        return false;
    };
    let index = source.index().0 as usize;
    if uphill_miss.len() <= index {
        uphill_miss.resize_with(index + 1, || None);
    }
    let chance = uphill_miss[index].get_or_insert_with(|| {
        PseudoRandom25::new(rng.for_unit(Purpose::Evasion, wire_id(source), 0))
    });
    chance.roll()
}

/// Takes a missile out of the air and out of the world.
fn give_up(
    missile: crate::game::Entity,
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

impl World {
    /// Sends every missile that landed on to its next mark, and gives up the
    /// ones with nowhere left to go.
    pub fn bounce_missiles(&mut self) {
        for (missile, from) in std::mem::take(&mut self.bounced) {
            if !self.bounce_on(missile, from) {
                self.give_up_missile(missile);
            }
        }
    }

    /// Sends a missile on to the next enemy near where it landed.
    ///
    /// It never strikes the same one twice, and stops once its bounces are
    /// spent or there is nobody left to go to.
    pub fn bounce_on(&mut self, missile: Entity, from: Entity) -> bool {
        let Some(mut shot) = self.projectile.get(missile).cloned() else {
            return false;
        };
        if shot.bounces_left == 0 {
            return false;
        }
        let Some(at) = self.transform.get(from).map(|t| t.pos) else {
            return false;
        };
        let radius = rules::units(shot.bounce_range);
        let Some(source) = shot.source else {
            return false;
        };
        let next = self
            .entities
            .iter()
            .filter(|other| {
                !shot.bounced.contains(other)
                    && self.hostile(source, *other)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .min_by_key(|other| {
                self.transform
                    .get(*other)
                    .map_or(i64::MAX, |t| t.pos.distance_squared(at))
            });
        let Some(next) = next else {
            return false;
        };
        shot.bounces_left -= 1;
        shot.bounced.push(next);
        shot.target = next;
        self.projectile.insert(missile, shot);
        true
    }

    /// Takes a missile out of the air and out of the world.
    pub fn give_up_missile(&mut self, missile: Entity) {
        self.projectile.remove(missile);
        self.transform.remove(missile);
        self.team.remove(missile);
        self.despawn(missile);
    }
}
