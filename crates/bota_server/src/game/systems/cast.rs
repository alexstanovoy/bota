//! Casting abilities: the checks, the cost, and the charges a cast hands
//! enemy items.

use bota_core::{NeutralAbility, ability_def, at_home_shop, item_def, rules};
use bota_proto::{AbilityId, AbilitySlot, EventKind, Fixed, Target, UnitKind};

use crate::game::{Entity, Event, EventVisibility, World, ability_hooks, wire_id};

impl World {
    /// Whether an entity stands in its own shop.
    pub fn at_shop(&self, entity: Entity) -> bool {
        let (Some(at), Some(side)) = (self.pos(entity), self.team.get(entity).copied()) else {
            return false;
        };
        at_home_shop(self.map, side, at)
    }

    /// Casts one of an entity's abilities to the moment it has gone off:
    /// the checks, the ability's own work, and the cost.
    ///
    /// False when it did not go off, and then nothing was spent: the ability
    /// must be active and learned, the cooldown run out, the mana be there,
    /// and the ability itself must have found something to do.
    pub fn begin_ability(&mut self, entity: Entity, slot: AbilitySlot, target: Target) -> bool {
        let at = usize::from(slot.0);
        let Some(ability) = self
            .abilities
            .get(entity)
            .and_then(|book| book.slots.get(at))
            .copied()
        else {
            return false;
        };
        let Some(def) = ability_def(ability.id) else {
            return false;
        };
        // An illusion casts nothing, and uses no item.
        if def.passive || ability.level == 0 || ability.cooldown > 0 || self.wears_illusion(entity)
        {
            return false;
        }
        let cost = self.ability_mana_cost(entity, ability.id, ability.level);
        if self.mana.get(entity).map_or(0, |m| m.mana.to_int()) < cost {
            return false;
        }
        let cooldown = self.ability_cooldown(entity, ability.id, ability.level);
        let gone_off = match NeutralAbility::of(ability.id) {
            Some(spell) => self.cast_spell(entity, spell, ability.level, target),
            None => {
                ability_hooks(ability.id).is_some_and(|hooks| (hooks.on_cast)(self, entity, target))
            }
        };
        if !gone_off {
            return false;
        }
        self.expose_caster(entity, target);
        self.charge_nearby_items(entity);
        if let Some(mana) = self.mana.get_mut(entity) {
            mana.mana -= Fixed::from_int(cost);
        }
        if let Some(book) = self.abilities.get_mut(entity)
            && let Some(ability) = book.slots.get_mut(at)
        {
            ability.cooldown = cooldown;
        }
        let visible_to = self.who_may_know(entity, None);
        self.events.push(Event {
            kind: EventKind::AbilityCast {
                caster: wire_id(entity),
                ability: ability.id,
            },
            visible_to,
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
        true
    }

    /// Gives every enemy item near a cast one charge of what it may hold.
    ///
    /// Only a cast by a hero is worth a charge, and only an item of a hero
    /// standing within its charge radius of it takes one.
    fn charge_nearby_items(&mut self, caster: Entity) {
        if self.kind.get(caster).copied() != Some(UnitKind::Hero) {
            return;
        }
        let (Some(at), Some(side)) = (self.pos(caster), self.team.get(caster).copied()) else {
            return;
        };
        for other in self.entities.iter() {
            if self.team.get(other).copied() == Some(side)
                || self.kind.get(other).copied() != Some(UnitKind::Hero)
            {
                continue;
            }
            let Some(from) = self.pos(other) else {
                continue;
            };
            let Some(bag) = self.inventory.get_mut(other) else {
                continue;
            };
            for stack in bag.slots.iter_mut().flatten() {
                let Some(def) = item_def(stack.id) else {
                    continue;
                };
                let reach = def.charge_radius().unwrap_or(0);
                if def.cast_charges > 0 && from.within(at, rules::units(reach)) {
                    stack.charges = stack.charges.saturating_add(1).min(def.cast_charges);
                }
            }
        }
    }

    /// Which ability sits in one of an entity's slots; `AbilityId(u16::MAX)`
    /// when there is none.
    pub fn ability_in(&self, entity: Entity, slot: AbilitySlot) -> AbilityId {
        self.abilities
            .get(entity)
            .and_then(|book| book.slots.get(usize::from(slot.0)))
            .map_or(AbilityId(u16::MAX), |held| held.id)
    }
}
