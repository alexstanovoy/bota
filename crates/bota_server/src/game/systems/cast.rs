//! Casting abilities: the checks, the cost, and the charges a cast hands
//! enemy items.

use bota_core::{NeutralAbility, ability_def, at_home_shop, item_def, rules};
use bota_proto::{AbilityId, AbilitySlot, Fixed, RejectReason, Target, UnitKind};

use crate::game::{Entity, World, ability_hooks};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CastOutcome {
    Refused,
    Applied,
    Blocked,
}
impl CastOutcome {
    pub fn went(self) -> bool {
        self != Self::Refused
    }
}

impl World {
    /// Whether an entity stands in its own shop.
    pub fn at_shop(&self, entity: Entity) -> bool {
        let (Some(at), Some(side)) = (self.pos(entity), self.team.get(entity).copied()) else {
            return false;
        };
        at_home_shop(self.map, side, at)
    }

    /// Any shop location, for selling and backpack inactivity.
    pub fn at_any_shop(&self, entity: Entity) -> bool {
        self.pos(entity)
            .zip(self.hull.get(entity))
            .is_some_and(|(pos, hull)| bota_core::at_any_shop(self.map, pos, hull.bound))
    }

    /// Casts one of an entity's abilities to the moment it has gone off:
    /// the checks, the ability's own work, and the cost.
    ///
    /// False when it did not go off, and then nothing was spent: the ability
    /// must be active and learned, the cooldown run out, the mana be there,
    /// and the ability itself must have found something to do.
    pub fn begin_ability(&mut self, entity: Entity, slot: AbilitySlot, target: Target) -> bool {
        self.begin_ability_outcome(entity, slot, target).went()
    }

    pub(crate) fn begin_ability_outcome(
        &mut self,
        entity: Entity,
        slot: AbilitySlot,
        target: Target,
    ) -> CastOutcome {
        let at = usize::from(slot.0);
        let Some(ability) = self
            .abilities
            .get(entity)
            .and_then(|book| book.slots.get(at))
            .copied()
        else {
            return CastOutcome::Refused;
        };
        let Some(def) = ability_def(ability.id) else {
            return CastOutcome::Refused;
        };
        if self.cast_blocked(entity, ability, def).is_some()
            || self.short_of_mana_for_cast(entity, ability)
        {
            return CastOutcome::Refused;
        }
        if self.silenced(entity) && !def.cast_while_silenced {
            return CastOutcome::Refused;
        }
        let cost = self.ability_mana_cost(entity, ability.id, ability.level);
        let cooldown = self.ability_cooldown(entity, ability.id, ability.level);
        let blocked = if matches!(def.block, bota_core::BlockTiming::Cast) {
            let Some(blocked) = self.block_cast_target(entity, ability.id, target) else {
                return CastOutcome::Refused;
            };
            blocked
        } else {
            false
        };
        let gone_off = blocked
            || match NeutralAbility::of(ability.id) {
                Some(spell) => self.cast_spell(entity, spell, ability.level, target),
                None => ability_hooks(ability.id)
                    .is_some_and(|hooks| (hooks.on_cast)(self, entity, target)),
            };
        if !gone_off {
            return CastOutcome::Refused;
        }
        self.pay_for_ability(entity, at, ability.id, cost, cooldown, target);
        if blocked {
            CastOutcome::Blocked
        } else {
            CastOutcome::Applied
        }
    }

    fn pay_for_ability(
        &mut self,
        entity: Entity,
        at: usize,
        ability: AbilityId,
        cost: i32,
        cooldown: u32,
        target: Target,
    ) {
        self.expose_caster(entity, target);
        if ability_def(ability).is_some_and(|def| def.procs_magic_stick) {
            self.charge_nearby_items(entity);
        }
        if let Some(mana) = self.mana.get_mut(entity) {
            mana.mana -= Fixed::from_int(cost);
        }
        if let Some(book) = self.abilities.get_mut(entity)
            && let Some(ability) = book.slots.get_mut(at)
        {
            ability.cooldown = cooldown;
        }
        self.tell_ability_triggered(entity, ability);
    }

    /// What keeps an ability in a slot from going off at all right now,
    /// whoever asks: an order taken for it, or the cast itself. A passive one,
    /// or any one of an illusion (which casts nothing, and uses no item), one
    /// not learned or still cooling down, any one of a stunned or feared
    /// caster but what may be cast while stunned, and one Dota marks
    /// `ROOT_DISABLES` while its caster is rooted.
    /// Mana is asked last, by [`World::short_of_mana_for_cast`].
    pub(crate) fn cast_blocked(
        &self,
        entity: Entity,
        ability: crate::game::AbilityState,
        def: &bota_core::AbilityDef,
    ) -> Option<RejectReason> {
        if def.passive || self.wears_illusion(entity) {
            return Some(RejectReason::NotCastable);
        }
        if ability.level == 0 {
            return Some(RejectReason::NotLearned);
        }
        if ability.cooldown > 0 {
            return Some(RejectReason::OnCooldown);
        }
        // A channel is no bar: an order breaks it off, as in Dota. What Dota
        // lets begin despite a stun (Roshan's roar) begins despite a fear too.
        if !def.cast_while_stunned && (self.held(entity) || self.feared(entity)) {
            return Some(RejectReason::Disabled);
        }
        if def.root_disables && self.rooted(entity) {
            return Some(RejectReason::Disabled);
        }
        None
    }

    /// Whether an entity has less mana than a cast of the ability in a slot
    /// costs.
    pub(crate) fn short_of_mana_for_cast(
        &self,
        entity: Entity,
        ability: crate::game::AbilityState,
    ) -> bool {
        let cost = self.ability_mana_cost(entity, ability.id, ability.level);
        self.mana.get(entity).map_or(0, |m| m.mana.to_int()) < cost
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
            let bonus = self.aoe_bonus_of(other);
            let Some(bag) = self.inventory.get_mut(other) else {
                continue;
            };
            for stack in bag.slots.iter_mut().flatten() {
                let Some(def) = item_def(stack.id) else {
                    continue;
                };
                // The stick's radius is its carrier's, widened by the carrier's AoE bonus.
                let reach = bota_core::widened(def.charge_radius().unwrap_or(0), bonus);
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
