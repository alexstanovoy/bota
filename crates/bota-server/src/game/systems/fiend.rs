//! The razes, the souls gathered from what falls, the presence worn by the
//! enemies near him, and the requiem the souls feed.

use bota_proto::{AbilityId, Angle, DamageKind, Fixed};

use crate::engine::Entity;
use crate::game::{
    Hit, HitEffect, Modifier, ModifierKind, RequiemLine, StackKind, Transform, World, ability,
    heading_of, leaves_a_death, move_towards, per_tick, point_along, rules,
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
                attack: false,
                pierces: false,
                effect: HitEffect::Shadowraze { level: level as u8 },
            });
        }
        let which = AbilityId(ability::RAZE_NEAR.0 + reach as u16);
        self.spawn_mark(which, caster, at, rules::MARK_TICKS);
        true
    }

    /// Lets the gathered souls go as lines flying out of the caster, one to
    /// a soul up to [`rules::REQUIEM_LINES_MAX`], spread evenly round it
    /// starting along its facing. Each line burns what it crosses on its
    /// way out. What is let go is not spent; with nothing gathered the cast
    /// still happens and nothing goes out.
    pub fn cast_requiem(&mut self, caster: Entity, level: usize) -> bool {
        let (Some(from), Some(side)) = (
            self.transform.get(caster).copied(),
            self.team.get(caster).copied(),
        ) else {
            return false;
        };
        let held = self
            .stacks
            .get(caster)
            .map_or(0, |kept| kept.of(StackKind::Souls));
        let lines = held.min(rules::REQUIEM_LINES_MAX);
        for nth in 0..lines {
            let facing = Angle {
                brads: from.facing.brads.wrapping_add((nth * 65536 / lines) as u16),
            };
            let aim = point_along(
                from.pos,
                from.pos + heading_of(facing),
                Fixed::from_int(rules::REQUIEM_LINE_DISTANCE),
            );
            let line = self.spawn();
            self.transform.insert(
                line,
                Transform {
                    pos: from.pos,
                    facing,
                },
            );
            self.set_team(line, side);
            self.requiem_line.insert(
                line,
                RequiemLine {
                    owner: caster,
                    aim,
                    speed: Fixed::from_int(rules::REQUIEM_LINE_SPEED),
                    travelled: Fixed::ZERO,
                    distance: Fixed::from_int(rules::REQUIEM_LINE_DISTANCE),
                    damage: rules::REQUIEM_LINE_DAMAGE[level],
                    slow_pct: rules::REQUIEM_SLOW_PCT[level],
                    struck: Vec::new(),
                },
            );
        }
        true
    }

    /// Flies every line of a requiem one tick on.
    ///
    /// A line burns everything hostile within its width of where it now
    /// flies, once each, and adds [`rules::REQUIEM_LINE_TICKS`] of fear and
    /// slow to it, up to [`rules::REQUIEM_HOLD_MAX_TICKS`]. It widens as it
    /// goes, and is gone once it has flown its distance.
    pub fn tick_requiem_lines(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut line) = self.requiem_line.get(entity).cloned() else {
                continue;
            };
            let Some(at) = self.transform.get(entity).map(|t| t.pos) else {
                self.put_out(entity);
                continue;
            };
            let step = per_tick(line.speed);
            let next = move_towards(at, line.aim, step);
            line.travelled = (line.travelled + step).min(line.distance);
            if let Some(transform) = self.transform.get_mut(entity) {
                transform.pos = next;
            }
            let width = requiem_width(line.travelled, line.distance);
            let crossed: Vec<Entity> = self
                .entities
                .iter()
                .filter(|other| {
                    *other != entity
                        && !line.struck.contains(other)
                        && self.hostile(entity, *other)
                        && self.transform.get(*other).is_some_and(|t| {
                            let hull = self.hull.get(*other).map_or(Fixed::ZERO, |h| h.bound);
                            t.pos.within(next, width + hull)
                        })
                })
                .collect();
            for other in crossed {
                self.push_hit(Some(line.owner), other, line.damage, DamageKind::Magical);
                for kind in [
                    ModifierKind::Slowed { pct: line.slow_pct },
                    ModifierKind::Feared,
                ] {
                    self.extend_modifier(
                        other,
                        Modifier {
                            kind,
                            source: Some(line.owner),
                            ticks_left: Some(rules::REQUIEM_LINE_TICKS),
                        },
                        rules::REQUIEM_HOLD_MAX_TICKS,
                    );
                }
                line.struck.push(other);
            }
            if next == line.aim || line.travelled >= line.distance {
                self.put_out(entity);
            } else {
                self.requiem_line.insert(entity, line);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Takes a line of a requiem out of the world.
    fn put_out(&mut self, line: Entity) {
        self.requiem_line.remove(line);
        self.transform.remove(line);
        self.team.remove(line);
        self.despawn(line);
    }

    /// Lets a share of the souls go when the one holding them falls:
    /// [`rules::SOULS_LOST_ON_DEATH_PCT`] of them, rounded down.
    pub fn let_souls_go(&mut self, fallen: Entity) {
        if let Some(kept) = self.stacks.get_mut(fallen) {
            let held = kept.of(StackKind::Souls);
            kept.set(
                StackKind::Souls,
                held - held * rules::SOULS_LOST_ON_DEATH_PCT / 100,
            );
        }
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
                self.put_modifier(
                    mark,
                    Modifier {
                        kind: ModifierKind::ArmorBroken {
                            armor: rules::PRESENCE_ARMOR[usize::from(level - 1)],
                        },
                        source: Some(carrier),
                        ticks_left: Some(rules::PRESENCE_LINGER_TICKS),
                    },
                );
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

/// How wide a line of the requiem catches once it has flown so far of its
/// distance: from [`rules::REQUIEM_LINE_WIDTH_START`] out to
/// [`rules::REQUIEM_LINE_WIDTH_END`], evenly along the way.
fn requiem_width(travelled: Fixed, distance: Fixed) -> Fixed {
    let start = Fixed::from_int(rules::REQUIEM_LINE_WIDTH_START);
    let growth = i64::from(rules::REQUIEM_LINE_WIDTH_END - rules::REQUIEM_LINE_WIDTH_START);
    if distance.raw <= 0 {
        return start;
    }
    let grown = growth * i64::from(travelled.raw) / i64::from(distance.raw);
    start + Fixed::from_int(grown as i32)
}
