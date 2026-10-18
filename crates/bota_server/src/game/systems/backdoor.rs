//! Backdoor protection: a structure far from enemy lane creeps takes less
//! damage and mends what the blows took.

use bota_core::{kind_def, rules};
use bota_proto::{Fixed, Team, UnitKind};

use crate::engine::Entity;
use crate::game::{Backdoor, Landed, World};

impl World {
    /// Gives a new structure its protection: held from the first tick.
    pub(crate) fn give_backdoor(&mut self, entity: Entity) {
        self.backdoor.insert(
            entity,
            Backdoor {
                quiet_ticks: rules::BACKDOOR_REACTIVATION_TICKS,
                owed: Fixed::ZERO,
            },
        );
    }

    /// How far a structure looks for enemy lane creeps, if it looks at all:
    /// a tier two tower for itself, an Ancient for its whole side's base.
    fn backdoor_watch(&self, entity: Entity) -> Option<i32> {
        match self.kind.get(entity)? {
            UnitKind::Ancient => Some(rules::BACKDOOR_BASE_RADIUS),
            UnitKind::Tower if self.tier.get(entity).map(|tier| tier.0) == Some(2) => {
                Some(rules::BACKDOOR_TOWER_RADIUS)
            }
            _ => None,
        }
    }

    /// The structure whose quiet a protected structure holds by.
    fn backdoor_source(&self, entity: Entity) -> Option<Entity> {
        if self.backdoor_watch(entity) == Some(rules::BACKDOOR_TOWER_RADIUS) {
            return Some(entity);
        }
        let side = self.team.get(entity).copied()?;
        self.entities.iter().find(|&other| {
            self.kind.get(other) == Some(&UnitKind::Ancient)
                && self.team.get(other).copied() == Some(side)
                && self.alive(other)
        })
    }

    /// Whether backdoor protection holds on a structure now.
    pub(crate) fn backdoor_holds(&self, entity: Entity) -> bool {
        self.backdoor.get(entity).is_some()
            && self.alive(entity)
            && self
                .backdoor_source(entity)
                .and_then(|source| self.backdoor.get(source))
                .is_some_and(|source| source.quiet_ticks >= rules::BACKDOOR_REACTIVATION_TICKS)
    }

    /// Counts the quiet of every watching structure and mends what protected
    /// structures are owed.
    pub fn tick_backdoor(&mut self) {
        let creeps: Vec<(bota_proto::Vec2, Team)> = self
            .entities
            .iter()
            .filter(|&entity| {
                self.kind
                    .get(entity)
                    .is_some_and(|k| kind_def(*k).lane_creep)
                    && self.alive(entity)
            })
            .filter_map(|entity| Some((self.transform.get(entity)?.pos, *self.team.get(entity)?)))
            .collect();
        let held: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.backdoor.get(entity).is_some())
            .collect();
        for &entity in &held {
            let (Some(radius), Some(side), Some(from)) = (
                self.backdoor_watch(entity),
                self.team.get(entity).copied(),
                self.transform.get(entity).map(|at| at.pos),
            ) else {
                continue;
            };
            let reach = rules::units(radius);
            let seen = creeps
                .iter()
                .any(|&(at, team)| team != side && at.within(from, reach));
            if let Some(state) = self.backdoor.get_mut(entity) {
                state.quiet_ticks = if seen {
                    0
                } else {
                    (state.quiet_ticks + 1).min(rules::BACKDOOR_REACTIVATION_TICKS)
                };
            }
        }
        let heal = Fixed::from_ratio(
            rules::BACKDOOR_HEAL_PER_SECOND,
            rules::TICKS_PER_SECOND as i32,
        );
        for entity in held {
            let holds = self.backdoor_holds(entity);
            let Some(state) = self.backdoor.get_mut(entity) else {
                continue;
            };
            if !holds {
                state.owed = Fixed::ZERO;
                continue;
            }
            let mend = state.owed.min(heal);
            state.owed -= mend;
            let max = self.stats.get(entity).map_or(Fixed::ZERO, |s| s.max_hp);
            if let Some(pool) = self.health.get_mut(entity) {
                pool.hp = (pool.hp + mend).min(max);
            }
        }
    }

    /// Wears the protection as [`Stats::backdoor`](crate::game::Stats::backdoor).
    pub(crate) fn mark_backdoor(&mut self) {
        let held: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.backdoor.get(entity).is_some())
            .collect();
        for entity in held {
            let holds = self.backdoor_holds(entity);
            if let Some(stats) = self.stats.get_mut(entity) {
                stats.backdoor = holds;
            }
        }
    }

    /// Notes the health enemy blows took from protected structures: it is
    /// theirs to mend.
    pub(crate) fn owe_backdoor(&mut self, felt: &[Landed]) {
        for blow in felt {
            let protected = !blow.fatal && self.stats.get(blow.target).is_some_and(|s| s.backdoor);
            let enemy = blow
                .source
                .and_then(|source| self.team.get(source).copied())
                .is_some_and(|team| team != blow.side);
            if protected
                && enemy
                && let Some(state) = self.backdoor.get_mut(blow.target)
            {
                state.owed += Fixed::from_int(blow.amount);
            }
        }
    }
}
