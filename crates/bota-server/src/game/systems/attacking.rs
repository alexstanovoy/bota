//! The attack cycle: waiting, winding up, and the moment a swing comes due.
//!
//! What this owns is the cycle itself. Who to attack is put in [`Target`] by
//! acquisition, by a creep's own mind or by an order; coming round to face
//! that target is movement's business. Here a swing begins only once three
//! things hold at the same time: the target can be seen, it is in reach, and
//! the attacker is looking near enough at it.

use std::collections::VecDeque;

use bota_proto::{DamageKind, Fixed, Team, UnitKind};

use crate::game::{
    Attacking, Entity, EntityAllocator, Ground, Health, Hit, Hull, Projectile, Stats, StatusKind,
    Statuses, Table, Target, Transform, Visibility, Windup, is_creep,
};
use crate::game::{facing_gap, facing_towards, rules};

/// What the attack cycle reads and writes.
pub struct AttackCx<'a> {
    /// Which entities exist, and where a swing that comes due makes one.
    pub entities: &'a mut EntityAllocator,
    /// Where each entity stands and which way it looks; a missile is placed
    /// here when one takes to the air.
    pub transform: &'a mut Table<Transform>,
    /// The room each entity takes.
    pub hull: &'a Table<Hull>,
    /// Elevation under a ranged attacker when its missile leaves.
    pub ground: &'a Ground,
    /// What kind of thing each entity is, for what a blow is worth against it.
    pub kind: &'a Table<UnitKind>,
    /// Which side each entity is on.
    pub team: &'a mut Table<Team>,
    /// Health, for telling the standing from the fallen.
    pub health: &'a Table<Health>,
    /// Reach, timing and damage.
    pub stats: &'a Table<Stats>,
    /// Who sees what, worked out this tick.
    pub visibility: &'a mut Table<Visibility>,
    /// Who each entity is set on.
    pub target: &'a Table<Target>,
    /// What is on each entity, for what holds it still.
    pub statuses: &'a Table<Statuses>,
    /// The cycle itself.
    pub attacking: &'a mut Table<Attacking>,
    /// Where a melee swing leaves its blow.
    pub hits: &'a mut VecDeque<Hit>,
    /// Where a ranged swing leaves its missile.
    pub projectile: &'a mut Table<Projectile>,
}

/// Runs every attack cycle one tick on.
///
/// A swing that comes due leaves something behind: a blow where the target
/// stands, or a missile on its way to it. Which of the two is decided here,
/// from the attacker's own reach.
pub fn attacking_system(cx: AttackCx<'_>) {
    let AttackCx {
        entities,
        transform,
        hull,
        ground,
        kind,
        team,
        health,
        stats,
        visibility,
        target,
        statuses,
        attacking,
        hits,
        projectile,
    } = cx;
    for entity in entities.iter().collect::<Vec<_>>() {
        let gates = Gates {
            transform,
            hull,
            team,
            health,
            visibility,
        };
        let Some(mut state) = attacking.get(entity).copied() else {
            continue;
        };
        let held = statuses
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|s| s.kind == StatusKind::Stunned));
        state.cooldown = state.cooldown.saturating_sub(1);
        state.recovering = state.recovering.saturating_sub(1);
        let reach = stats.get(entity).map(|s| s.attack_range);
        match state.windup {
            // The swing is given up the moment what it was aimed at stops
            // being something to aim at, and the moment the entity is set on
            // nobody at all: an order to stop or to walk away takes the swing
            // with it. Nothing was struck, so nothing was spent: the wait for
            // the next one starts over at zero.
            Some(windup)
                if held
                    || target.get(entity).is_none()
                    || reach.is_none_or(|reach| {
                        !still_worth_swinging_at(&gates, entity, windup.target, reach)
                    }) =>
            {
                state.windup = None;
                state.cooldown = 0;
            }
            Some(windup) if windup.ticks_left > 1 => {
                state.windup = Some(Windup {
                    target: windup.target,
                    ticks_left: windup.ticks_left - 1,
                });
            }
            // A swing that has run its course lands on whoever it began
            // against, whatever the entity is set on now.
            Some(windup) => {
                state.windup = None;
                state.recovering = stats.get(entity).map_or(0, |s| s.attack_backswing);
                if let Some(stat) = stats.get(entity).copied() {
                    let against = kind.get(windup.target).copied();
                    strike(
                        entity,
                        windup.target,
                        &stat,
                        against,
                        entities,
                        transform,
                        ground,
                        team,
                        visibility,
                        hits,
                        projectile,
                    );
                }
            }
            None => {
                if !held
                    && state.cooldown == 0
                    && let Some(Target(on)) = target.get(entity).copied()
                    && let Some(stats) = stats.get(entity)
                    && stats.damage > 0
                    && may_swing(&gates, entity, on, stats)
                {
                    state.windup = Some(Windup {
                        target: on,
                        ticks_left: stats.attack_point.max(1),
                    });
                    state.cooldown = stats.attack_interval;
                }
            }
        }
        attacking.insert(entity, state);
    }
}

/// Leaves what a swing that came due turns into.
#[allow(clippy::too_many_arguments)]
fn strike(
    attacker: Entity,
    on: Entity,
    stats: &Stats,
    against: Option<UnitKind>,
    entities: &mut EntityAllocator,
    transform: &mut Table<Transform>,
    ground: &Ground,
    team: &mut Table<Team>,
    visibility: &mut Table<Visibility>,
    hits: &mut VecDeque<Hit>,
    projectile: &mut Table<Projectile>,
) {
    let side = team.get(attacker).copied().unwrap_or(Team::Neutral);
    // What is carried against creeps is worth nothing against anything else.
    let damage = stats.damage
        + if against.is_some_and(is_creep) {
            stats.damage_to_creeps
        } else {
            0
        };
    match stats.projectile_speed {
        None => hits.push_back(Hit {
            source: Some(attacker),
            target: on,
            amount: damage,
            kind: DamageKind::Physical,
            crit: false,
        }),
        Some(speed) => {
            let Some(at) = transform.get(attacker).copied() else {
                return;
            };
            let missile = entities.alloc();
            transform.insert(missile, at);
            team.insert(missile, side);
            let mut seen = Visibility::NONE;
            seen.add(side);
            visibility.insert(missile, seen);
            projectile.insert(
                missile,
                Projectile {
                    speed,
                    source: Some(attacker),
                    target: on,
                    damage,
                    kind: DamageKind::Physical,
                    ability: None,
                    launch_tier: ground.tier(at.pos),
                    can_miss_uphill: !stats.flies,
                    crit: false,
                    bounces_left: 0,
                    bounce_range: 0,
                    bounced: Vec::new(),
                },
            );
        }
    }
}

/// The tables the three gates are answered from.
struct Gates<'a> {
    transform: &'a Table<Transform>,
    hull: &'a Table<Hull>,
    team: &'a Table<Team>,
    health: &'a Table<Health>,
    visibility: &'a Table<Visibility>,
}

/// Whether a swing may begin: the target is standing, seen, in reach, and
/// looked at.
fn may_swing(gates: &Gates<'_>, attacker: Entity, on: Entity, stats: &Stats) -> bool {
    if !worth_swinging_at(gates, attacker, on, stats.attack_range) {
        return false;
    }
    let (Some(from), Some(at)) = (gates.transform.get(attacker), gates.transform.get(on)) else {
        return false;
    };
    // Looked at, within the angle a swing allows. Only the start waits on
    // this: once under way the attacker keeps turning with its target.
    let wanted = facing_towards(from.pos, at.pos);
    facing_gap(from.facing, wanted) <= rules::ATTACK_ANGLE_BRADS
}

/// Whether a swing already under way is still worth finishing.
///
/// The same three things a swing begins on, less the angle, and with the
/// leeway a started swing carries: the target has to leave by more than that
/// to shake it off.
fn still_worth_swinging_at(gates: &Gates<'_>, attacker: Entity, on: Entity, reach: Fixed) -> bool {
    worth_swinging_at(
        gates,
        attacker,
        on,
        reach + rules::units(rules::ATTACK_RANGE_LEEWAY),
    )
}

/// Whether the target is standing, seen by the attacker's side, and within
/// `reach` edge to edge.
fn worth_swinging_at(gates: &Gates<'_>, attacker: Entity, on: Entity, reach: Fixed) -> bool {
    let standing = gates
        .health
        .get(on)
        .is_some_and(|health| health.hp > Fixed::ZERO);
    if !standing {
        return false;
    }
    // Seen: a side does not swing at what it has no eyes on.
    let Some(side) = gates.team.get(attacker).copied() else {
        return false;
    };
    if !gates.visibility.get(on).is_some_and(|seen| seen.by(side)) {
        return false;
    }
    let (Some(from), Some(at)) = (gates.transform.get(attacker), gates.transform.get(on)) else {
        return false;
    };
    let hulls = gates.hull.get(attacker).map_or(Fixed::ZERO, |h| h.radius)
        + gates.hull.get(on).map_or(Fixed::ZERO, |h| h.radius);
    from.pos.within(at.pos, reach + hulls)
}
