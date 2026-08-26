//! The razes, the souls gathered from what falls, and the requiem they feed.

use bota_proto::{DamageKind, Fixed, Target};

use crate::engine::Entity;
use crate::game::{StackKind, Status, StatusKind, World, hero_def, point_along, rules};

impl World {
    /// Burns everything hostile standing where a raze lands.
    ///
    /// A raze lands at its own reach from the caster, along the line towards
    /// where it was aimed. `reach` indexes [`rules::RAZE_DISTANCE`].
    pub fn cast_raze(
        &mut self,
        caster: Entity,
        level: usize,
        reach: usize,
        target: Target,
    ) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(from) = self.transform.get(caster).map(|t| t.pos) else {
            return false;
        };
        let at = point_along(from, pos, Fixed::from_int(rules::RAZE_DISTANCE[reach]));
        if at == from {
            return false;
        }
        let radius = rules::units(rules::RAZE_RADIUS);
        let struck: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                self.hostile(caster, *other)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .collect();
        for mark in struck {
            self.push_hit(
                Some(caster),
                mark,
                rules::RAZE_DAMAGE[level],
                DamageKind::Magical,
            );
        }
        true
    }

    /// Lets the gathered souls go at once: everything hostile within
    /// [`rules::REQUIEM_RADIUS`] takes what each soul is worth and walks
    /// slower for a while. What is let go is not spent; with nothing gathered
    /// the cast still happens and nothing goes out.
    pub fn cast_requiem(&mut self, caster: Entity, level: usize) -> bool {
        let Some(at) = self.transform.get(caster).map(|t| t.pos) else {
            return false;
        };
        let held = self
            .stacks
            .get(caster)
            .map_or(0, |kept| kept.of(StackKind::Souls));
        if held == 0 {
            return true;
        }
        let damage = rules::REQUIEM_DAMAGE_PER_SOUL[level] * held as i32;
        let radius = rules::units(rules::REQUIEM_RADIUS);
        let struck: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                self.hostile(caster, *other)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .collect();
        for mark in struck {
            self.push_hit(Some(caster), mark, damage, DamageKind::Magical);
            let mut on_it = self.statuses.remove(mark).unwrap_or_default();
            on_it.put(Status {
                kind: StatusKind::Slowed {
                    pct: rules::REQUIEM_SLOW_PCT[level],
                },
                ticks_left: rules::REQUIEM_SLOW_TICKS,
            });
            self.statuses.insert(mark, on_it);
        }
        true
    }

    /// Hands the soul of what has fallen to whoever brought it down.
    ///
    /// Only a hero that gathers souls at all takes one, and only up to what
    /// its level lets it hold. A hero is worth more than anything else;
    /// a structure or a ward is worth nothing.
    pub fn feed_souls(&mut self, fallen: Entity, killer: Option<Entity>) {
        let Some(killer) = killer.filter(|killer| *killer != fallen) else {
            return;
        };
        if self
            .kind
            .get(fallen)
            .copied()
            .is_none_or(|kind| !crate::game::leaves_a_death(kind))
        {
            return;
        }
        if !self.gathers_souls(killer) {
            return;
        }
        let worth = if self.is_hero(fallen) {
            rules::SOULS_PER_HERO
        } else {
            rules::SOULS_PER_UNIT
        };
        let cap = self.soul_cap(killer);
        let mut held = self.stacks.get(killer).copied().unwrap_or_default();
        held.gather_up_to(StackKind::Souls, worth, cap);
        self.stacks.insert(killer, held);
    }

    /// Whether an entity is a hero that gathers souls at all.
    pub fn gathers_souls(&self, entity: Entity) -> bool {
        self.hero
            .get(entity)
            .and_then(|id| hero_def(*id))
            .is_some_and(|def| def.souls)
    }

    /// How many souls an entity may hold at the level it has reached.
    pub fn soul_cap(&self, entity: Entity) -> u32 {
        let level = self.level.get(entity).map_or(1, |level| u32::from(level.0));
        rules::SOUL_CAP_BASE + rules::SOUL_CAP_PER_LEVEL * level.saturating_sub(1)
    }
}
