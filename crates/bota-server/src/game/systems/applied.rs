//! Applied stat changes: their countdown and the rates they leave.

use bota_proto::{AbilityId, DamageKind, ItemId};

use crate::game::rules;
use crate::game::{
    Entity, MAX_COMBINED_MODIFIER_SCALE, Stats, World, ability_cooldown, ability_mana_cost,
    item_def,
};

impl World {
    /// Runs every applied stat change one tick down and drops what has run
    /// out.
    ///
    /// A world where nothing is applied pays nothing for the pass.
    pub fn tick_applied(&mut self) {
        if self.applied.is_empty() {
            return;
        }
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let drop_row = match self.applied.get_mut(entity) {
                Some(applied) => {
                    applied.retain(|held| !held.tick());
                    applied.is_empty()
                }
                None => false,
            };
            if drop_row {
                self.applied.remove(entity);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// A bounty after the killing unit's gold income scale.
    ///
    /// Modifier deltas add first and the composed bounty is scaled once, so
    /// the result is `base * (nominal + sum(deltas)) / nominal`.
    pub fn bounty_after(&self, killer: Option<Entity>, base: i32) -> i32 {
        let scale = killer
            .and_then(|unit| self.applied.get(unit))
            .map_or(rules::NOMINAL_BP, |applied| {
                applied.iter().fold(rules::NOMINAL_BP, |sum, held| {
                    sum.saturating_add(held.spec.gold_income - rules::NOMINAL_BP)
                })
            })
            .clamp(0, MAX_COMBINED_MODIFIER_SCALE);
        (i64::from(base) * i64::from(scale) / i64::from(rules::NOMINAL_BP))
            .clamp(0, i64::from(i32::MAX)) as i32
    }

    /// Outgoing damage amplification captured while a source is still live.
    /// A delayed carrier keeps this value if the source later falls or its
    /// slot changes hands.
    pub fn outgoing_damage_amp_bp(&self, source: Option<Entity>, kind: DamageKind) -> i32 {
        source
            .filter(|source| self.entities.contains(*source))
            .and_then(|source| self.stats.get(source))
            .map_or(rules::NOMINAL_BP, |stats| stats.damage_amp_bp(kind))
    }

    /// Mana one cast of an ability costs an entity, after its mana cost rate.
    pub fn ability_mana_cost(&self, entity: Entity, id: AbilityId, level: u8) -> i32 {
        self.cost_after_rate(entity, ability_mana_cost(id, level))
    }

    /// Ticks one cast of an ability puts on the clock, after its cooldown
    /// rate.
    pub fn ability_cooldown(&self, entity: Entity, id: AbilityId, level: u8) -> u32 {
        self.cooldown_after_rate(entity, ability_cooldown(id, level))
    }

    /// Mana one use of an item costs an entity, after its mana cost rate.
    pub fn item_mana_cost(&self, entity: Entity, id: ItemId) -> i32 {
        self.cost_after_rate(entity, item_def(id).map_or(0, |def| def.mana_cost))
    }

    /// Ticks one use of an item puts on the clock, after its cooldown rate.
    pub fn item_cooldown(&self, entity: Entity, id: ItemId) -> u32 {
        self.cooldown_after_rate(entity, item_def(id).map_or(0, |def| def.cooldown))
    }

    /// A base mana cost after the entity's mana cost rate.
    pub fn cost_after_rate(&self, entity: Entity, base: i32) -> i32 {
        let rate = self.mana_cost_rate_of(Some(entity));
        cost_after(base, rate)
    }

    /// A base cooldown after the entity's cooldown rate.
    pub fn cooldown_after_rate(&self, entity: Entity, base: u32) -> u32 {
        let rate = self.rate_of(Some(entity), |stats| stats.cooldown_rate_bp);
        cooldown_after(base, rate)
    }

    /// The mana cost rate a unit's projections and charges carry. Nominal
    /// when there is no body or it has no stats.
    pub(crate) fn mana_cost_rate_of(&self, entity: Option<Entity>) -> i32 {
        self.rate_of(entity, |stats| stats.mana_cost_rate_bp)
    }

    /// One rate stat of an optional entity, nominal when it has no stats.
    fn rate_of(&self, entity: Option<Entity>, pick: fn(&Stats) -> i32) -> i32 {
        entity
            .and_then(|entity| self.stats.get(entity))
            .map_or(rules::NOMINAL_BP, pick)
            .max(1)
    }
}

/// A mana cost after a rate in basis points.
pub fn cost_after(base: i32, rate_bp: i32) -> i32 {
    (i64::from(base) * i64::from(rate_bp) / i64::from(rules::NOMINAL_BP))
        .clamp(0, i64::from(i32::MAX)) as i32
}

/// Ticks a cooldown comes to at a rate in basis points, never zero for a
/// cooldown that was set at all.
pub fn cooldown_after(base: u32, rate_bp: i32) -> u32 {
    if base == 0 {
        return 0;
    }
    (i64::from(base) * i64::from(rate_bp) / i64::from(rules::NOMINAL_BP))
        .clamp(1, i64::from(u32::MAX)) as u32
}
