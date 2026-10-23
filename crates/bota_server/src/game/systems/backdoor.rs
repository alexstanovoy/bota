//! Backdoor protection: a structure far from enemy lane creeps takes less
//! damage and mends what the blows took. Which structures have it, and where
//! each looks, is their kind of unit's [`BackdoorWatch`]; while it holds, the
//! structure wears it as a modifier, as Dota's buildings wear
//! `modifier_backdoor_protection_active`.

use bota_core::{BackdoorWatch, ModifierKind, kind_def, rules};
use bota_proto::{Fixed, Team};

use crate::engine::Entity;
use crate::game::{Backdoor, Def, Landed, Modifier, World};

impl World {
    /// Gives a new structure the protection its kind of unit carries, held
    /// from the first tick.
    pub(crate) fn give_backdoor(&mut self, entity: Entity) {
        if self.backdoor_of(entity).is_none() {
            return;
        }
        self.backdoor.insert(
            entity,
            Backdoor {
                quiet_ticks: rules::BACKDOOR_REACTIVATION_TICKS,
                owed: Fixed::ZERO,
            },
        );
    }

    /// Where an entity's protection looks for enemy lane creeps, if it has
    /// any.
    fn backdoor_of(&self, entity: Entity) -> Option<BackdoorWatch> {
        self.def.get(entity).and_then(|Def(def)| def.backdoor)
    }

    /// How far a structure looks for enemy lane creeps, if it looks at all:
    /// a tier two tower for itself, an Ancient for its whole side's base.
    fn backdoor_watch(&self, entity: Entity) -> Option<i32> {
        match self.backdoor_of(entity)? {
            BackdoorWatch::Own { radius } | BackdoorWatch::Base { radius } => Some(radius),
            BackdoorWatch::InBase => None,
        }
    }

    /// The structure whose quiet a protected structure holds by: itself when
    /// it looks, else its side's standing base watch.
    fn backdoor_source(&self, entity: Entity) -> Option<Entity> {
        if self.backdoor_watch(entity).is_some() {
            return Some(entity);
        }
        let side = self.team.get(entity).copied()?;
        self.entities.iter().find(|&other| {
            matches!(self.backdoor_of(other), Some(BackdoorWatch::Base { .. }))
                && self.team.get(other).copied() == Some(side)
                && self.alive(other)
        })
    }

    /// Whether a structure wears its protection right now.
    pub(crate) fn backdoor_on(&self, entity: Entity) -> bool {
        self.modifiers.get(entity).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| held.kind == ModifierKind::BackdoorProtection)
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

    /// Puts the protection on every structure it holds on now, untimed, and
    /// takes it off every one where it has lapsed.
    pub(crate) fn wear_backdoor(&mut self) {
        let held: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.backdoor.get(entity).is_some())
            .collect();
        for entity in held {
            match (self.backdoor_holds(entity), self.backdoor_on(entity)) {
                (true, false) => {
                    self.put_modifier(
                        entity,
                        Modifier::new(ModifierKind::BackdoorProtection, None, None),
                    );
                }
                (false, true) => self.take_modifier(entity, ModifierKind::BackdoorProtection, None),
                _ => {}
            }
        }
    }

    /// Notes the health enemy blows took from protected structures: it is
    /// theirs to mend.
    pub(crate) fn owe_backdoor(&mut self, felt: &[Landed]) {
        for blow in felt {
            let protected = !blow.fatal && self.backdoor_on(blow.target);
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
