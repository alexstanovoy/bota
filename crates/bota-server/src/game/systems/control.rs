//! What holds a unit still, and what burns it while it stands there.

use bota_proto::Fixed;

use crate::engine::Entity;
use bota_proto::UnitKind;

use crate::game::{StatusKind, World, rules};

impl World {
    /// Whether an entity is held: it neither walks, nor turns, nor swings,
    /// nor casts until it lifts.
    pub fn held(&self, entity: Entity) -> bool {
        self.statuses
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|s| s.kind == StatusKind::Stunned))
    }

    /// Takes health off everything that is burning, on the beat.
    ///
    /// A burn that may not take the last point stops one short of it, so what
    /// its own owner carries never kills that owner.
    pub fn tick_burning(&mut self) {
        if !self.tick.is_multiple_of(rules::BURN_PERIOD_TICKS) {
            return;
        }
        let mut struck = Vec::new();
        for entity in self.entities.iter() {
            let Some(on_it) = self.statuses.get(entity) else {
                continue;
            };
            let left = self
                .health
                .get(entity)
                .map_or(0, |health| health.hp.to_int());
            for status in on_it.active() {
                let StatusKind::Burning {
                    amount,
                    kind,
                    from,
                    lethal,
                } = status.kind
                else {
                    continue;
                };
                let amount = if lethal { amount } else { amount.min(left - 1) };
                if amount > 0 {
                    struck.push((from, entity, amount, kind));
                }
            }
        }
        for (from, on, amount, kind) in struck {
            if self.health.get(on).is_some_and(|h| h.hp > Fixed::ZERO) {
                self.push_hit(from, on, amount, kind);
            }
        }
    }
}

impl World {
    /// Puts out every drink a blow is enough to break, and sets back every
    /// item that answers to one.
    ///
    /// Only a hero or a tower breaks anything; a creep may hit all day
    /// without it. What was drunk with nothing to break it is left alone, and
    /// so is an item that answers to no blow.
    pub fn break_on_blows(&mut self, felt: &[crate::game::Landed]) {
        for blow in felt {
            let Some(from) = blow.source else {
                continue;
            };
            if !self
                .kind
                .get(from)
                .copied()
                .is_some_and(|kind| matches!(kind, UnitKind::Hero | UnitKind::Tower))
            {
                continue;
            }
            if let Some(on_it) = self.statuses.get_mut(blow.target) {
                on_it.0.retain(|status| {
                    !matches!(
                        status.kind,
                        StatusKind::Mending { breaks: true, .. }
                            | StatusKind::Clarity { breaks: true, .. }
                    )
                });
            }
            if let Some(bag) = self.inventory.get_mut(blow.target) {
                for stack in bag.slots.iter_mut().flatten() {
                    let Some(def) = crate::game::item_def(stack.id) else {
                        continue;
                    };
                    if def.breaks_on_damage > 0 {
                        stack.cooldown = stack.cooldown.max(def.breaks_on_damage);
                    }
                }
            }
        }
    }
}
