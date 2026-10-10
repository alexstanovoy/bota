//! Shadow Fiend's feast of souls: the haste it puts on him, the souls it
//! takes from the enemies near him, and what is left of them once it ends.

use bota_proto::UnitKind;

use crate::game::{Feast, FeastPhase, Fed, StackKind, Talent, World, is_creep, rules};

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
        self.feast.insert(
            caster,
            Feast {
                level: level as u8,
                phase: FeastPhase::Feasting,
                ticks_left: rules::FEAST_TICKS,
                fed: Vec::with_capacity(rules::FEAST_TARGETS[level]),
            },
        );
        true
    }

    /// Runs every feast one tick on: a gathering every
    /// [`rules::FEAST_PERIOD_TICKS`] while it runs, and what lingers counted
    /// down.
    pub fn tick_feasts(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut feast) = self.feast.get(entity).cloned() else {
                continue;
            };
            assert!(feast.ticks_left > 0);
            feast.ticks_left -= 1;
            match feast.phase {
                FeastPhase::Feasting => {
                    let ran = rules::FEAST_TICKS - feast.ticks_left;
                    if feast.ticks_left == 0 {
                        self.stop_feasting(entity, feast);
                    } else {
                        if ran.is_multiple_of(rules::FEAST_PERIOD_TICKS) {
                            self.gather_feast(entity, &mut feast);
                        }
                        self.feast.insert(entity, feast);
                    }
                }
                FeastPhase::Lingering => {
                    if feast.ticks_left == 0 {
                        self.stop_lingering(entity);
                    } else {
                        self.feast.insert(entity, feast);
                    }
                }
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
    fn gather_feast(&mut self, caster: Entity, feast: &mut Feast) {
        let room = rules::FEAST_TARGETS[usize::from(feast.level)].saturating_sub(feast.fed.len());
        let Some(at) = self.transform.get(caster).map(|t| t.pos) else {
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
                        .is_some_and(|kind| *kind == UnitKind::Hero || is_creep(*kind))
            })
            .filter_map(|other| {
                let pos = self.transform.get(other)?.pos;
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
        let mut held = self.stacks.get(caster).copied().unwrap_or_default();
        for (creep, _, other) in near.into_iter().take(room.min(rules::FEAST_PER_PERIOD)) {
            let souls = if creep {
                rules::FEAST_SOULS_PER_UNIT
            } else {
                per_hero
            };
            held.gather(StackKind::Souls, souls);
            feast.fed.push(Fed { from: other, souls });
        }
        assert!(feast.fed.len() <= rules::FEAST_TARGETS[usize::from(feast.level)]);
        self.stacks.insert(caster, held);
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
        if let Some(held) = self.stacks.get_mut(caster) {
            let left = held.of(StackKind::Souls).saturating_sub(standing);
            held.set(StackKind::Souls, left);
        }
        if feast.fed.is_empty() {
            self.stop_lingering(caster);
            return;
        }
        feast.phase = FeastPhase::Lingering;
        feast.ticks_left = rules::FEAST_LINGER_TICKS;
        self.feast.insert(caster, feast);
    }

    /// What lingered is over: no more souls are held than the cap allows.
    fn stop_lingering(&mut self, caster: Entity) {
        self.feast.remove(caster);
        let cap = self.soul_cap(caster);
        if let Some(held) = self.stacks.get_mut(caster) {
            let kept = held.of(StackKind::Souls).min(cap);
            held.set(StackKind::Souls, kept);
        }
    }
}
