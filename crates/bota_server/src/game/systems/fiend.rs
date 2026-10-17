//! The razes, the souls gathered from what falls, the presence worn by the
//! enemies near him, and the requiem the souls feed.

use bota_core::{
    ModifierKind, Talent, attack_class_pct, hero_def, kind_def, move_towards, per_tick,
    point_ahead, presence_armor, raze_damage, raze_landing, raze_stack_damage, requiem_death_lines,
    requiem_fear_ticks, requiem_line_damage, requiem_lines, requiem_width, rules, soul_cap,
};
use bota_proto::{AbilityId, Angle, DamageKind, Fixed, Team};

use crate::game::{Fear, Hit, HitEffect, Modifier, RequiemLine, StackKind, World};

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
        let at = raze_landing(from.pos, from.facing, reach);
        let radius = rules::units(rules::RAZE_RADIUS);
        let struck: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                self.burns(caster, *other)
                    && self.pos(*other).is_some_and(|pos| pos.within(at, radius))
            })
            .collect();
        let amount = raze_damage(level, self.souls_of(caster));
        let damage_amp_bp = self.outgoing_damage_amp_bp(Some(caster), DamageKind::Magical);
        let stack_damage =
            raze_stack_damage(level, self.has_talent(caster, Talent::RazeStackDamage));
        let swings = self.has_talent(caster, Talent::RazeAttackDamage);
        for mark in struck {
            self.hits.push_back(Hit {
                source: Some(caster),
                target: mark,
                amount,
                kind: DamageKind::Magical,
                damage_amp_bp,
                attack: false,
                pierces: false,
                effect: HitEffect::Shadowraze { stack_damage },
            });
            if swings {
                self.raze_strikes(caster, mark);
            }
        }
        let which = AbilityId(AbilityId::RAZE_NEAR.0 + reach as u16);
        self.spawn_mark(which, caster, at, rules::MARK_TICKS);
        true
    }

    /// The attack damage a raze with the level-25 talent deals besides its
    /// burn: a swing's damage, rolled as a swing rolls it and weighed by the
    /// attack classes, as physical damage that never misses. No attack
    /// modifier rides with it: no crit, no pierce, nothing against creeps
    /// only.
    fn raze_strikes(&mut self, caster: Entity, mark: Entity) {
        let Some(stats) = self.stats.get(caster).copied() else {
            return;
        };
        let damage = stats.damage + self.roll_damage(stats.damage_spread);
        let class_pct = match (self.kind.get(caster), self.kind.get(mark)) {
            (Some(&from), Some(&to)) => attack_class_pct(from, to),
            _ => 100,
        };
        self.hits.push_back(Hit {
            source: Some(caster),
            target: mark,
            amount: damage * class_pct / 100,
            kind: DamageKind::Physical,
            damage_amp_bp: stats.damage_amp_bp(DamageKind::Physical),
            attack: false,
            pierces: false,
            effect: HitEffect::None,
        });
    }

    /// Lets the gathered souls go as lines flying out of the caster, one to
    /// a soul up to [`rules::REQUIEM_LINES_MAX`]. What is let go is not
    /// spent; with nothing gathered the cast still happens and nothing goes
    /// out.
    pub fn cast_requiem(&mut self, caster: Entity, level: usize) -> bool {
        let lines = requiem_lines(self.souls_of(caster));
        self.release_lines(caster, level, lines, true)
    }

    /// The requiem a fallen Shadow Fiend lets go where he fell, once his
    /// death has taken its share of the souls: a line for every
    /// [`rules::REQUIEM_DEATH_LINES`] of what is left, whatever the cooldown
    /// and the mana say. Its lines only burn. Nothing goes out while the
    /// requiem is unlearned.
    pub fn requiem_on_death(&mut self, fallen: Entity) {
        let level = self.carried_level(fallen, AbilityId::REQUIEM);
        if level == 0 {
            return;
        }
        let lines = requiem_death_lines(self.souls_of(fallen));
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
        let (line, most) = requiem_fear_ticks(self.has_talent(caster, Talent::RequiemFear));
        let fear = Fear { line, most };
        for nth in 0..lines {
            let facing = Angle {
                brads: (nth * 65536 / lines) as u16,
            };
            let aim = point_ahead(
                from.pos,
                facing,
                Fixed::from_int(rules::REQUIEM_LINE_DISTANCE),
            );
            let line = self.spawn_placed(from.pos, facing, side);
            self.requiem_line.insert(
                line,
                RequiemLine {
                    owner: caster,
                    aim,
                    speed: Fixed::from_int(rules::REQUIEM_LINE_SPEED),
                    travelled: Fixed::ZERO,
                    distance: Fixed::from_int(rules::REQUIEM_LINE_DISTANCE),
                    damage: requiem_line_damage(level),
                    damage_amp_bp,
                    level: level as u8,
                    frightens,
                    fear,
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
            let Some(at) = self.pos(entity) else {
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
            attack: false,
            pierces: false,
            effect: if line.frightens {
                HitEffect::Requiem {
                    level: line.level,
                    fear: line.fear,
                }
            } else {
                HitEffect::None
            },
        });
    }

    /// Whether a raze or a requiem line of one side lands on another entity:
    /// anything hostile to it but a structure.
    fn burns(&self, from: Entity, other: Entity) -> bool {
        self.hostile(from, other)
            && !self
                .kind
                .get(other)
                .copied()
                .is_some_and(|kind| kind_def(kind).structure)
    }

    /// Takes a line of a requiem out of the world.
    fn put_out(&mut self, line: Entity) {
        self.requiem_line.remove(line);
        self.discard(line);
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
                let level = self.carried_level(entity, AbilityId::PRESENCE);
                (level > 0).then_some((entity, level))
            })
            .collect();
        for (carrier, level) in carriers {
            let (Some(from), Some(side)) = (self.pos(carrier), self.team.get(carrier).copied())
            else {
                continue;
            };
            let reach = rules::units(rules::PRESENCE_RADIUS);
            let armor = presence_armor(level, self.has_talent(carrier, Talent::PresenceArmor));
            let struck: Vec<Entity> = self
                .entities
                .iter()
                .filter(|other| {
                    self.pos(*other).is_some_and(|pos| pos.within(from, reach))
                        && self.wears_presence(carrier, side, *other)
                })
                .collect();
            for mark in struck {
                self.put_modifier(
                    mark,
                    Modifier::new(
                        ModifierKind::ArmorBroken { armor },
                        Some(carrier),
                        Some(rules::PRESENCE_LINGER_TICKS),
                    ),
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
                .is_some_and(|kind| kind_def(*kind).leaves_a_death)
            && !self
                .stats
                .get(other)
                .is_some_and(|stats| stats.invulnerable)
            && self.can_see(theirs, carrier)
    }

    /// Hands the soul of what has fallen to whoever brought it down.
    ///
    /// Only a killer that gathers souls takes any, and only up to
    /// [`World::soul_cap`]; a count a feast has carried past it stays
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
            .is_none_or(|kind| !kind_def(kind).leaves_a_death)
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
            .is_some_and(|def| def.innate == Some(AbilityId::NECROMASTERY))
    }

    /// How many souls an entity may gather from kills, its talent included.
    /// Zero for one that gathers none.
    pub fn soul_cap(&self, entity: Entity) -> u32 {
        if self.gathers_souls(entity) {
            soul_cap(self.has_talent(entity, Talent::NecroSoulCap))
        } else {
            0
        }
    }

    /// Whether an entity has learned a talent. A fallen hero's book is on
    /// its seat and answers nothing here.
    pub fn has_talent(&self, entity: Entity, talent: Talent) -> bool {
        self.abilities
            .get(entity)
            .is_some_and(|book| book.has_talent(talent))
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
