//! What a neutral's auras and storms hand out: the spells whose reach is a
//! circle about their bearer.

use bota_core::{
    AuraSpell, ModifierKind, NeutralAbility, Spell, Whom, isqrt64, rules, ticks_of_ms,
};
use bota_proto::{DamageKind, Inflictor, Team};

use crate::game::{Entity, Modifier, Spots, World};

use super::modifiers::resisted_ticks;

/// A storm's shape, from [`Spell::Storm`], and the spell it is.
#[derive(Clone, Copy)]
struct Storm {
    ability: NeutralAbility,
    radius: i32,
    near_radius: i32,
    near_dps: i32,
    far_dps: i32,
    period_ms: i32,
}

impl World {
    /// Hands out every neutral aura and storm for the tick, on the layout
    /// of where everything stands that [`aura_system`](crate::game::aura_system)
    /// just laid.
    ///
    /// Each spell puts its modifier on afresh every tick and lingers
    /// [`rules::AURA_LINGER_TICKS`] after the unit leaves. A broken bearer
    /// hands out nothing, and one in a river-only aura's case hands it out
    /// only while it stands in the water.
    pub fn tick_spell_auras(&mut self) {
        let spots = std::mem::take(&mut self.aura_spots);
        let mut near = Vec::new();
        let entities = self.take_entity_snapshot();
        for source in entities.iter().copied() {
            if self.neutral.get(source).is_some() && !self.broken(source) {
                self.hand_out_spells(source, &spots, &mut near);
            }
        }
        self.recycle_entity_snapshot(entities);
        self.aura_spots = spots;
    }

    /// One source's auras and storms, spell by spell.
    fn hand_out_spells(&mut self, source: Entity, spots: &Spots<Entity>, near: &mut Vec<Entity>) {
        let (Some(side), Some(from)) = (self.team.get(source).copied(), self.pos(source)) else {
            return;
        };
        let held = self
            .abilities
            .get(source)
            .map_or(0, |book| book.slots.len());
        for at in 0..held {
            let Some(state) = self
                .abilities
                .get(source)
                .and_then(|book| book.slots.get(at))
            else {
                continue;
            };
            let (level, id) = (state.level, state.id);
            let Some(ability) = NeutralAbility::of(id) else {
                continue;
            };
            let (whom, radius, storm) = match *ability.spell() {
                Spell::Aura(AuraSpell { water: true, .. }) if !self.ground.water(from) => continue,
                Spell::Aura(aura) => (aura.whom, aura.radius, None),
                Spell::Storm {
                    radius,
                    near_radius,
                    near_dps,
                    far_dps,
                    period_ms,
                    ..
                } => (
                    Whom::Enemies,
                    radius,
                    Some(Storm {
                        ability,
                        radius,
                        near_radius,
                        near_dps,
                        far_dps,
                        period_ms,
                    }),
                ),
                _ => continue,
            };
            let radius = if side == Team::Neutral {
                radius.min(rules::NEUTRAL_AURA_RADIUS)
            } else {
                radius
            };
            let reach = rules::units(radius);
            near.clear();
            near.extend(spots.around(from, i64::from(reach.raw).abs()));
            for target in near.iter().copied() {
                let wanted = match whom {
                    Whom::Allies => self.team.get(target).copied() == Some(side),
                    Whom::Enemies => self.team.get(target).is_some_and(|theirs| *theirs != side),
                };
                let there = self.pos(target).filter(|pos| pos.within(from, reach));
                if let (true, Some(there)) = (wanted && self.aura_reaches(target), there) {
                    self.hand_spell(source, target, (ability, level));
                    if let Some(storm) = storm {
                        self.rain_on(source, target, storm, isqrt64(from.distance_squared(there)));
                    }
                }
            }
        }
    }

    /// Whether an aura reaches a unit: alive, not a building or a ward, not
    /// invulnerable.
    fn aura_reaches(&self, target: Entity) -> bool {
        self.living_unit(target)
            && !self
                .stats
                .get(target)
                .is_some_and(|stats| stats.invulnerable)
    }

    /// Puts one spell's modifier on a unit. One that does not stack keeps
    /// the stronger level when several sources reach it in the same tick.
    fn hand_spell(
        &mut self,
        source: Entity,
        target: Entity,
        (ability, level): (NeutralAbility, u8),
    ) {
        let kind = ModifierKind::Spell {
            ability,
            level,
            amount: 0,
        };
        let ticks = resisted_ticks(&self.stats, target, kind, rules::AURA_LINGER_TICKS);
        let Some(on_it) = self.modifiers.get_mut(target) else {
            return;
        };
        let fresher = on_it.spell(ability, Some(source)).is_some_and(|held| {
            matches!(held.kind, ModifierKind::Spell { level: had, .. } if had > level)
                && held.ticks_left >= Some(ticks)
        });
        if !fresher {
            let with = Some(Inflictor::Ability(ability.id()));
            on_it.put(Modifier::new(kind, Some(source), with, Some(ticks)));
        }
    }

    /// A storm's damage on a unit `distance` (in raw units) from its eye,
    /// on the beat: `far_dps` a second at the rim, rising evenly to
    /// `near_dps` within `near_radius`. The 0.25 s beat is eight ticks.
    fn rain_on(&mut self, source: Entity, target: Entity, storm: Storm, distance: i64) {
        let beat = ticks_of_ms(storm.period_ms);
        if !self.tick.is_multiple_of(beat) {
            return;
        }
        let near = i64::from(rules::units(storm.near_radius).raw);
        let rim = i64::from(rules::units(storm.radius).raw);
        let (low, high) = (i64::from(storm.far_dps), i64::from(storm.near_dps));
        let dps = if distance <= near {
            high
        } else {
            low + (high - low) * (rim - distance).max(0) / (rim - near)
        };
        let amount = dps * i64::from(beat) / i64::from(rules::TICKS_PER_SECOND);
        self.push_hit(
            Some(source),
            target,
            amount as i32,
            DamageKind::Magical,
            Some(Inflictor::Ability(storm.ability.id())),
        );
    }
}
