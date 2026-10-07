//! The razes, the souls gathered from what falls, the presence worn by the
//! enemies near him, and the requiem the souls feed.

use bota_proto::{AbilityId, Angle, DamageKind, Fixed, Team};

use crate::game::{
    Hit, HitEffect, Modifier, ModifierKind, RequiemLine, StackKind, Transform, World, ability,
    heading_of, hero_def, is_structure, leaves_a_death, move_towards, per_tick, point_along, rules,
};

use crate::engine::Entity;

impl World {
    /// Burns everything hostile but a structure standing where a raze lands.
    ///
    /// A raze takes no aim: it lands at its own reach from the caster, along
    /// the line the caster faces. `reach` indexes [`rules::RAZE_DISTANCE`].
    /// Every soul the caster holds as it goes off adds
    /// [`rules::RAZE_DAMAGE_PER_SOUL`].
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
                self.burns(caster, *other)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .collect();
        let souls = self.souls_of(caster).min(rules::SOULS_HELD_MAX);
        let amount = rules::RAZE_DAMAGE[level] + rules::RAZE_DAMAGE_PER_SOUL * souls as i32;
        let damage_amp_bp = self.outgoing_damage_amp_bp(Some(caster), DamageKind::Magical);
        for mark in struck {
            self.hits.push_back(Hit {
                source: Some(caster),
                target: mark,
                amount,
                kind: DamageKind::Magical,
                damage_amp_bp,
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
    /// a soul up to [`rules::REQUIEM_LINES_MAX`]. What is let go is not
    /// spent; with nothing gathered the cast still happens and nothing goes
    /// out.
    pub fn cast_requiem(&mut self, caster: Entity, level: usize) -> bool {
        let lines = self.souls_of(caster).min(rules::REQUIEM_LINES_MAX);
        self.release_lines(caster, level, lines, true)
    }

    /// The requiem a fallen Shadow Fiend lets go where he fell, once his
    /// death has taken its share of the souls: a line for every
    /// [`rules::REQUIEM_DEATH_LINES`] of what is left, whatever the cooldown
    /// and the mana say. Its lines only burn. Nothing goes out while the
    /// requiem is unlearned.
    pub fn requiem_on_death(&mut self, fallen: Entity) {
        let level = self.carried_level(fallen, ability::REQUIEM);
        if level == 0 {
            return;
        }
        let share = rules::REQUIEM_DEATH_LINES;
        let lines = (self.souls_of(fallen) * u32::from(share.num()) / u32::from(share.den()))
            .min(rules::REQUIEM_LINES_MAX);
        let level = usize::from(level - 1).min(rules::REQUIEM_LINE_DAMAGE.len() - 1);
        self.release_lines(fallen, level, lines, false);
    }

    /// Sends `lines` lines of a requiem out of the caster, spread evenly round
    /// it, the first due east whichever way the caster faces.
    fn release_lines(&mut self, caster: Entity, level: usize, lines: u32, frightens: bool) -> bool {
        assert!(level < rules::REQUIEM_LINE_DAMAGE.len());
        assert!(lines <= rules::REQUIEM_LINES_MAX);
        let (Some(from), Some(side)) = (
            self.transform.get(caster).copied(),
            self.team.get(caster).copied(),
        ) else {
            return false;
        };
        let damage_amp_bp = self.outgoing_damage_amp_bp(Some(caster), DamageKind::Magical);
        for nth in 0..lines {
            let facing = Angle {
                brads: (nth * 65536 / lines) as u16,
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
                    damage_amp_bp,
                    level: level as u8,
                    frightens,
                    struck: Vec::new(),
                },
            );
        }
        true
    }

    /// Flies every line of a requiem one tick on.
    ///
    /// A line burns everything hostile but a structure within its width of
    /// where it now flies, once each, frightening it as
    /// [`HitEffect::Requiem`] says when the line frightens. It widens as it
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
                        && self.burns(entity, *other)
                        && self.transform.get(*other).is_some_and(|t| {
                            let hull = self.hull.get(*other).map_or(Fixed::ZERO, |h| h.bound);
                            t.pos.within(next, width + hull)
                        })
                })
                .collect();
            for other in crossed {
                self.cross(&line, other);
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

    /// What a line of a requiem does to one it crosses: a burn that, from a
    /// line that frightens, frightens what it does not kill.
    fn cross(&mut self, line: &RequiemLine, other: Entity) {
        self.hits.push_back(Hit {
            source: Some(line.owner),
            target: other,
            amount: line.damage,
            kind: DamageKind::Magical,
            damage_amp_bp: line.damage_amp_bp,
            crit: false,
            attack: false,
            pierces: false,
            effect: if line.frightens {
                HitEffect::Requiem { level: line.level }
            } else {
                HitEffect::None
            },
        });
    }

    /// Whether a raze or a requiem line of one side lands on another entity:
    /// anything hostile to it but a structure.
    fn burns(&self, from: Entity, other: Entity) -> bool {
        self.hostile(from, other) && !self.kind.get(other).copied().is_some_and(is_structure)
    }

    /// Takes a line of a requiem out of the world.
    fn put_out(&mut self, line: Entity) {
        self.requiem_line.remove(line);
        self.transform.remove(line);
        self.team.remove(line);
        self.despawn(line);
    }

    /// Lets a share of the souls go when the one holding them falls: a
    /// feast under way ends first, then [`rules::SOULS_LOST_ON_DEATH_PCT`]
    /// of what is left goes, rounded down.
    pub fn let_souls_go(&mut self, fallen: Entity) {
        self.end_feast(fallen);
        if let Some(kept) = self.stacks.get_mut(fallen) {
            let held = kept.of(StackKind::Souls);
            kept.set(
                StackKind::Souls,
                held - held * rules::SOULS_LOST_ON_DEATH_PCT / 100,
            );
        }
    }

    /// Lays the presence on every enemy standing near its carriers that
    /// sees the carrier.
    ///
    /// Standing in it puts the armor break on afresh every tick; walking out
    /// leaves it to run out on its own. A structure, a ward or what damage
    /// passes by wears none.
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
            let (Some(from), Some(side)) = (
                self.transform.get(carrier).map(|t| t.pos),
                self.team.get(carrier).copied(),
            ) else {
                continue;
            };
            let reach = rules::units(rules::PRESENCE_RADIUS);
            let struck: Vec<Entity> = self
                .entities
                .iter()
                .filter(|other| {
                    self.transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(from, reach))
                        && self.wears_presence(carrier, side, *other)
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

    /// Whether one entity wears the presence of a carrier of a side: a
    /// standing enemy that leaves a death, that damage reaches, and whose
    /// side sees the carrier.
    fn wears_presence(&self, carrier: Entity, side: Team, other: Entity) -> bool {
        let Some(theirs) = self.team.get(other).copied() else {
            return false;
        };
        theirs != side
            && self.alive(other)
            && self
                .kind
                .get(other)
                .is_some_and(|kind| leaves_a_death(*kind))
            && !self
                .stats
                .get(other)
                .is_some_and(|stats| stats.invulnerable)
            && self.can_see(theirs, carrier)
    }

    /// Hands the soul of what has fallen to whoever brought it down.
    ///
    /// Only a killer that gathers souls takes any, and only up to
    /// [`rules::NECRO_SOUL_CAP`]; a count a feast has carried past it stays
    /// where it is. A hero is worth [`rules::SOULS_PER_HERO`], anything else
    /// [`rules::SOULS_PER_UNIT`]; a structure or a ward is worth nothing.
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
        let mut held = self.stacks.get(killer).copied().unwrap_or_default();
        held.gather_up_to(StackKind::Souls, worth, self.soul_cap(killer));
        self.stacks.insert(killer, held);
    }

    /// Whether an entity gathers souls at all: it is a hero whose innate is
    /// the necromastery.
    pub fn gathers_souls(&self, entity: Entity) -> bool {
        self.hero
            .get(entity)
            .and_then(|hero| hero_def(*hero))
            .is_some_and(|def| def.innate == Some(ability::NECROMASTERY))
    }

    /// How many souls an entity may gather from kills. Zero for one that
    /// gathers none.
    pub fn soul_cap(&self, entity: Entity) -> u32 {
        if self.gathers_souls(entity) {
            rules::NECRO_SOUL_CAP
        } else {
            0
        }
    }

    /// Souls an entity holds right now, those a feast has taken included.
    pub fn souls_of(&self, entity: Entity) -> u32 {
        self.stacks
            .get(entity)
            .map_or(0, |kept| kept.of(StackKind::Souls))
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
