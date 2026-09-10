//! The razes, the souls gathered from what falls, the presence worn by the
//! enemies near him, and the requiem the souls feed.

use bota_proto::{AbilityId, DamageKind, Fixed};

use crate::engine::Entity;
use crate::game::{
    Hit, HitEffect, StackKind, Status, StatusKind, World, ability, heading_of, leaves_a_death,
    point_along, rules,
};

impl World {
    /// Burns everything hostile standing where a raze lands.
    ///
    /// A raze takes no aim: it lands at its own reach from the caster, along
    /// the line the caster faces. `reach` indexes [`rules::RAZE_DISTANCE`].
    pub fn cast_raze(&mut self, caster: Entity, level: usize, reach: usize) -> bool {
        assert!(level < rules::RAZE_DAMAGE.len());
        assert!(reach < rules::RAZE_DISTANCE.len());
        if !self.alive(caster) {
            return false;
        }
        let Some(from) = self.transform.get(caster).copied() else {
            return false;
        };
        let ahead = from.pos + heading_of(from.facing);
        let at = point_along(
            from.pos,
            ahead,
            Fixed::from_int(rules::RAZE_DISTANCE[reach]),
        );
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
            self.hits.push_back(Hit {
                source: Some(caster),
                target: mark,
                amount: rules::RAZE_DAMAGE[level],
                kind: DamageKind::Magical,
                crit: false,
                effect: HitEffect::Shadowraze { level: level as u8 },
            });
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

    /// Lays the presence on everything hostile standing near its carriers.
    ///
    /// Standing in it puts the armor break on afresh every tick; walking out
    /// leaves it to run out on its own. A structure or a ward wears none.
    pub fn spread_presence(&mut self) {
        let carriers: Vec<(Entity, u8)> = self
            .entities
            .iter()
            .filter_map(|entity| {
                let level = self.carried_level(entity, ability::PRESENCE);
                (level > 0).then_some((entity, level))
            })
            .collect();
        for (carrier, level) in carriers {
            let Some(from) = self.transform.get(carrier).map(|t| t.pos) else {
                continue;
            };
            let reach = rules::units(rules::PRESENCE_RADIUS);
            let struck: Vec<Entity> = self
                .entities
                .iter()
                .filter(|other| {
                    self.hostile(carrier, *other)
                        && self
                            .kind
                            .get(*other)
                            .is_some_and(|kind| leaves_a_death(*kind))
                        && self
                            .transform
                            .get(*other)
                            .is_some_and(|t| t.pos.within(from, reach))
                })
                .collect();
            for mark in struck {
                let mut on_it = self.statuses.remove(mark).unwrap_or_default();
                on_it.put(Status {
                    kind: StatusKind::ArmorBroken {
                        armor: rules::PRESENCE_ARMOR[usize::from(level - 1)],
                    },
                    ticks_left: rules::PRESENCE_LINGER_TICKS,
                });
                self.statuses.insert(mark, on_it);
            }
        }
    }

    /// Hands the soul of what has fallen to whoever brought it down.
    ///
    /// Only a hero with the necromastery learned takes one, and only up to
    /// what its level lets it hold. A hero is worth more than anything else;
    /// a structure or a ward is worth nothing.
    pub fn feed_souls(&mut self, fallen: Entity, killer: Option<Entity>) {
        let Some(killer) = killer.filter(|killer| *killer != fallen) else {
            return;
        };
        if self
            .kind
            .get(fallen)
            .copied()
            .is_none_or(|kind| !leaves_a_death(kind))
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

    /// Whether an entity gathers souls at all: the necromastery is learned.
    pub fn gathers_souls(&self, entity: Entity) -> bool {
        self.carried_level(entity, ability::NECROMASTERY) > 0
    }

    /// How many souls an entity may hold at the necromastery level it has
    /// learned. Zero while it is unlearned.
    pub fn soul_cap(&self, entity: Entity) -> u32 {
        match self.carried_level(entity, ability::NECROMASTERY) {
            0 => 0,
            level => rules::NECRO_SOUL_CAP[usize::from(level - 1)],
        }
    }

    /// Which level of an ability an entity has learned. Zero for one that
    /// does not carry it at all.
    pub fn carried_level(&self, entity: Entity, id: AbilityId) -> u8 {
        self.abilities.get(entity).map_or(0, |book| {
            book.slots
                .iter()
                .find(|slot| slot.id == id)
                .map_or(0, |slot| slot.level)
        })
    }
}
