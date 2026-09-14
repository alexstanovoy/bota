//! What is on a unit: putting it on, running it one tick on, taking it off.

use bota_proto::{DamageKind, Fixed, UnitKind};

use crate::engine::Entity;
use crate::game::{Modifier, ModifierKind, World, rules};

impl World {
    /// Whether an entity is held: it neither walks, nor turns, nor swings,
    /// nor casts until it lifts.
    pub fn held(&self, entity: Entity) -> bool {
        self.modifiers.get(entity).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| held.kind == ModifierKind::Stunned)
        })
    }

    /// Puts a modifier on an entity. One that is not a unit takes nothing.
    pub fn put_modifier(&mut self, on: Entity, modifier: Modifier) {
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.put(modifier);
        }
    }

    /// Takes off every modifier of one kind from one source.
    pub fn take_modifier(&mut self, on: Entity, like: ModifierKind, source: Option<Entity>) {
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.take(like, source);
        }
    }

    /// Runs every modifier one tick on: what burns takes health on the beat,
    /// and the rot puts its burn and slow on everything standing in it.
    ///
    /// A burn that may not take the last point stops one short of it, so
    /// what its own owner carries never kills that owner.
    pub fn tick_modifiers(&mut self) {
        let on_beat = self.tick.is_multiple_of(rules::BURN_PERIOD_TICKS);
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(on_it) = self.modifiers.get(entity) else {
                continue;
            };
            let held: Vec<Modifier> = on_it.active().copied().collect();
            for modifier in held {
                match modifier.kind {
                    ModifierKind::Burning {
                        amount,
                        kind,
                        lethal,
                    } => {
                        if !on_beat {
                            continue;
                        }
                        let left = self
                            .health
                            .get(entity)
                            .map_or(0, |health| health.hp.to_int());
                        let amount = if lethal { amount } else { amount.min(left - 1) };
                        if amount > 0 && self.health.get(entity).is_some_and(|h| h.hp > Fixed::ZERO)
                        {
                            self.push_hit(modifier.source, entity, amount, kind);
                        }
                    }
                    ModifierKind::Rot { level } => self.rot(entity, level),
                    _ => {}
                }
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Burns and slows everything standing in one entity's rot, its owner
    /// included.
    ///
    /// What it does is handed out afresh every tick, so walking out of it
    /// lifts it, and switching it off lifts it everywhere at once. Its owner
    /// burns by the same amount but never to death.
    fn rot(&mut self, owner: Entity, level: u8) {
        if !self.alive(owner) {
            if let Some(on_it) = self.modifiers.get_mut(owner) {
                on_it
                    .0
                    .retain(|held| !matches!(held.kind, ModifierKind::Rot { .. }));
            }
            return;
        }
        let (Some(at), Some(side)) = (
            self.transform.get(owner).map(|t| t.pos),
            self.team.get(owner).copied(),
        ) else {
            return;
        };
        let amount = rules::ROT_DAMAGE_PER_SECOND[usize::from(level)]
            * rules::BURN_PERIOD_TICKS as i32
            / rules::TICKS_PER_SECOND as i32;
        let reach = rules::units(rules::ROT_RADIUS);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if !self.alive(other) || self.hull.get(other).is_none() {
                continue;
            }
            if !self
                .transform
                .get(other)
                .is_some_and(|t| t.pos.within(at, reach))
            {
                continue;
            }
            if other == owner {
                self.put_modifier(
                    owner,
                    Modifier {
                        kind: ModifierKind::Burning {
                            amount,
                            kind: DamageKind::Magical,
                            lethal: false,
                        },
                        source: None,
                        ticks_left: Some(2),
                    },
                );
                continue;
            }
            if self.team.get(other).copied() == Some(side) {
                continue;
            }
            self.put_modifier(
                other,
                Modifier {
                    kind: ModifierKind::Burning {
                        amount,
                        kind: DamageKind::Magical,
                        lethal: true,
                    },
                    source: Some(owner),
                    ticks_left: Some(2),
                },
            );
            self.put_modifier(
                other,
                Modifier {
                    kind: ModifierKind::Slowed {
                        pct: rules::ROT_SLOW_PCT[usize::from(level)],
                    },
                    source: Some(owner),
                    ticks_left: Some(2),
                },
            );
        }
    }

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
            if let Some(on_it) = self.modifiers.get_mut(blow.target) {
                on_it.0.retain(|held| {
                    !matches!(
                        held.kind,
                        ModifierKind::Mending { breaks: true, .. }
                            | ModifierKind::Clarity { breaks: true, .. }
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
