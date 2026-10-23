//! Shadow Fiend's feast of souls: the haste it puts on him, the souls it
//! takes from the enemies near him, and what is left of them once it ends.
//!
//! The feast runs as Dota's does, as a modifier on its caster:
//! `ModifierKind::Feasting` for its run, then `ModifierKind::FeastLinger`.
//! Whatever takes the modifier off, running out or a basic dispel, ends that
//! phase on the next tick.

use bota_core::{ModifierKind, Talent, kind_def, rules};
use bota_proto::UnitKind;

use crate::game::{Feast, FeastPhase, Fed, Modifier, World};

use crate::engine::Entity;

impl World {
    /// Starts a feast on the caster at a level counted from zero. A feast
    /// still running or lingering on him ends first.
    pub fn cast_feast(&mut self, caster: Entity, level: usize) -> bool {
        assert!(level < rules::FEAST_TARGETS.len());
        if !self.alive(caster) {
            return false;
        }
        self.end_feast(caster);
        let running = ModifierKind::Feasting {
            level: level as u8,
            souls: 0,
        };
        self.put_modifier(
            caster,
            Modifier::new(running, Some(caster), Some(rules::FEAST_TICKS)),
        );
        self.feast.insert(
            caster,
            Feast {
                phase: FeastPhase::Feasting,
                fed: Vec::with_capacity(rules::FEAST_TARGETS[level]),
            },
        );
        true
    }

    /// The level and the ticks left of the feast running on an entity, while
    /// its modifier is on.
    fn feast_running(&self, entity: Entity) -> Option<(u8, u32)> {
        self.modifiers
            .get(entity)?
            .active()
            .find_map(|held| match held.kind {
                ModifierKind::Feasting { level, .. } => Some((level, held.ticks_left?)),
                _ => None,
            })
    }

    /// Whether what a feast left still lingers on an entity.
    fn feast_lingers(&self, entity: Entity) -> bool {
        self.modifiers.get(entity).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| matches!(held.kind, ModifierKind::FeastLinger { .. }))
        })
    }

    /// Shows on the feast's modifier how many souls it holds.
    fn show_feast_souls(&mut self, entity: Entity, souls: u32) {
        let souls = u16::try_from(souls).unwrap_or(u16::MAX);
        let Some(on_it) = self.modifiers.get_mut(entity) else {
            return;
        };
        for held in on_it.0.iter_mut() {
            match &mut held.kind {
                ModifierKind::Feasting { souls: shown, .. }
                | ModifierKind::FeastLinger { souls: shown } => *shown = souls,
                _ => {}
            }
        }
    }

    /// Runs every feast one tick on: a gathering every
    /// [`rules::FEAST_PERIOD_TICKS`] while its modifier runs, and the end of
    /// a phase whose modifier has lifted.
    pub fn tick_feasts(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(feast) = self.feast.get(entity).cloned() else {
                continue;
            };
            match feast.phase {
                FeastPhase::Feasting => match self.feast_running(entity) {
                    None => self.stop_feasting(entity, feast),
                    Some((level, left)) => {
                        let ran = rules::FEAST_TICKS.saturating_sub(left);
                        if ran.is_multiple_of(rules::FEAST_PERIOD_TICKS) {
                            self.gather_feast(entity, feast, usize::from(level));
                        }
                    }
                },
                FeastPhase::Lingering if !self.feast_lingers(entity) => {
                    self.stop_lingering(entity);
                }
                FeastPhase::Lingering => {}
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Ends whatever feast an entity has on it at once, as running out would:
    /// the souls of the enemies still standing go, and so does whatever is
    /// held past the cap.
    pub fn end_feast(&mut self, entity: Entity) {
        let Some(feast) = self.feast.get(entity).cloned() else {
            return;
        };
        if feast.phase == FeastPhase::Feasting {
            self.stop_feasting(entity, feast);
        }
        self.stop_lingering(entity);
    }

    /// Takes souls from up to [`rules::FEAST_PER_PERIOD`] enemies not fed on
    /// yet, heroes first and the nearest first among them, while the feast
    /// has room for more.
    fn gather_feast(&mut self, caster: Entity, mut feast: Feast, level: usize) {
        let room = rules::FEAST_TARGETS[level].saturating_sub(feast.fed.len());
        let Some(at) = self.pos(caster) else {
            return;
        };
        let radius = rules::units(rules::FEAST_RADIUS);
        let mut near: Vec<(bool, i64, Entity)> = self
            .entities
            .iter()
            .filter(|other| {
                self.hostile(caster, *other)
                    && feast.fed.iter().all(|fed| fed.from != *other)
                    && self
                        .kind
                        .get(*other)
                        .is_some_and(|kind| *kind == UnitKind::Hero || kind_def(*kind).creep)
            })
            .filter_map(|other| {
                let pos = self.pos(other)?;
                pos.within(at, radius)
                    .then(|| (!self.is_hero(other), pos.distance_squared(at), other))
            })
            .collect();
        near.sort_by_key(|(creep, apart, _)| (*creep, *apart));
        let per_hero = rules::FEAST_SOULS_PER_HERO
            + if self.has_talent(caster, Talent::FeastSoulsPerHero) {
                rules::TALENT_FEAST_SOULS_PER_HERO
            } else {
                0
            };
        let mut held = self.souls_of(caster);
        for (creep, _, other) in near.into_iter().take(room.min(rules::FEAST_PER_PERIOD)) {
            let souls = if creep {
                rules::FEAST_SOULS_PER_UNIT
            } else {
                per_hero
            };
            held = held.saturating_add(souls);
            feast.fed.push(Fed { from: other, souls });
        }
        assert!(feast.fed.len() <= rules::FEAST_TARGETS[level]);
        self.set_souls(caster, held);
        self.show_feast_souls(caster, feast.souls());
        self.feast.insert(caster, feast);
    }

    /// The feast's haste is over: the souls of the enemies still standing
    /// go, and those of the fallen linger for [`rules::FEAST_LINGER_TICKS`].
    fn stop_feasting(&mut self, caster: Entity, mut feast: Feast) {
        let standing: u32 = feast
            .fed
            .iter()
            .filter(|fed| self.alive(fed.from))
            .map(|fed| fed.souls)
            .sum();
        feast.fed.retain(|fed| !self.alive(fed.from));
        let left = self.souls_of(caster).saturating_sub(standing);
        self.set_souls(caster, left);
        self.take_modifier(
            caster,
            ModifierKind::Feasting { level: 0, souls: 0 },
            Some(caster),
        );
        if feast.fed.is_empty() {
            self.stop_lingering(caster);
            return;
        }
        let souls = u16::try_from(feast.souls()).unwrap_or(u16::MAX);
        self.put_modifier(
            caster,
            Modifier::new(
                ModifierKind::FeastLinger { souls },
                Some(caster),
                Some(rules::FEAST_LINGER_TICKS),
            ),
        );
        feast.phase = FeastPhase::Lingering;
        self.feast.insert(caster, feast);
    }

    /// What lingered is over: no more souls are held than the cap allows.
    fn stop_lingering(&mut self, caster: Entity) {
        self.feast.remove(caster);
        self.take_modifier(caster, ModifierKind::FeastLinger { souls: 0 }, Some(caster));
        let kept = self.souls_of(caster).min(self.soul_cap(caster));
        self.set_souls(caster, kept);
    }
}
