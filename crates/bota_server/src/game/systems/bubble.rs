//! Water bubbles: the barrier a Marshmage puts on an ally, and the mend it
//! bursts into.
//!
//! The cast that puts one up is [`World::cast_spell`]'s: it lays a
//! `ModifierKind::Spell` of the bubble's ability on the ally, the damage it
//! can still soak as its `amount` and its life as its time.

use bota_core::{Act, Lv, ModifierKind, NeutralAbility, Spell, at_level, kind_def, rules};
use bota_proto::{Fixed, UnitKind};

use crate::game::{Entity, Modifiers, World};

/// The bubble an ability puts up, if it puts one up.
fn bubble_of(ability: NeutralAbility) -> Option<(Lv, i32, i32)> {
    match *ability.spell() {
        Spell::Cast {
            act:
                Act::Bubble {
                    absorb,
                    heal_pct,
                    radius,
                    ..
                },
            ..
        } => Some((absorb, heal_pct, radius)),
        _ => None,
    }
}

/// Takes magical damage `taken` off the bubbles on a unit, the first put on
/// first, and answers what is left of it for the unit's health.
pub fn soak(on_it: &mut Modifiers, mut taken: i32) -> i32 {
    for held in on_it.0.iter_mut() {
        if taken == 0 {
            break;
        }
        if held.ticks_left == Some(0) {
            continue;
        }
        let ModifierKind::Spell {
            ability, amount, ..
        } = &mut held.kind
        else {
            continue;
        };
        if bubble_of(*ability).is_none() {
            continue;
        }
        let soaked = taken.min(i32::from(*amount));
        *amount -= soaked as u16;
        taken -= soaked;
    }
    taken
}

impl World {
    /// Bursts every bubble that is used up or in its last tick: it comes
    /// off, and the one it was on mends `heal_pct` percent of the bubble's
    /// whole barrier, and so does every ally within its radius.
    ///
    /// A bubble that lapses is seen in the tick before it would go, since
    /// the clock takes it off the unit first.
    pub fn burst_bubbles(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(on_it) = self.modifiers.get(entity) else {
                continue;
            };
            let spent = on_it.active().find_map(|held| match held.kind {
                ModifierKind::Spell {
                    ability,
                    level,
                    amount,
                } if bubble_of(ability).is_some()
                    && (amount == 0 || held.ticks_left.is_some_and(|left| left <= 1)) =>
                {
                    Some((ability, level))
                }
                _ => None,
            });
            if let Some((ability, level)) = spent {
                self.burst_bubble(entity, ability, level);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Takes one bubble off a unit and mends with it.
    fn burst_bubble(&mut self, on: Entity, ability: NeutralAbility, level: u8) {
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.0.retain(|held| {
                !matches!(held.kind, ModifierKind::Spell { ability: it, .. } if it == ability)
            });
        }
        let Some((absorb, heal_pct, radius)) = bubble_of(ability) else {
            return;
        };
        let heal = at_level(&absorb, level) * heal_pct / 100;
        if heal == 0 {
            return;
        }
        let (Some(side), Some(at)) = (self.team.get(on).copied(), self.pos(on)) else {
            return;
        };
        let reach = rules::units(radius);
        for ally in self.entities.iter().collect::<Vec<_>>() {
            let near =
                ally == on || (radius > 0 && self.pos(ally).is_some_and(|p| p.within(at, reach)));
            if near && self.team.get(ally).copied() == Some(side) && self.living_unit(ally) {
                self.mend(ally, heal);
            }
        }
    }

    /// Whether a unit is alive and not a building, a ward or a missile.
    pub(crate) fn living_unit(&self, entity: Entity) -> bool {
        self.alive(entity)
            && self
                .kind
                .get(entity)
                .is_some_and(|kind| !kind_def(*kind).structure && *kind != UnitKind::Ward)
    }

    /// Mends a unit by `amount`, up to its maximum.
    pub(crate) fn mend(&mut self, entity: Entity, amount: i32) {
        let Some(max) = self.stats.get(entity).map(|stats| stats.max_hp) else {
            return;
        };
        if let Some(health) = self.health.get_mut(entity) {
            health.hp = (health.hp + Fixed::from_int(amount)).min(max);
        }
    }
}
