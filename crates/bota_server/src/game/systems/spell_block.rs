//! One absorption path for cast-time and projectile-impact spell blocks.
use super::applied::cooldown_after;
use crate::game::{
    AbilityBook, Entity, Event, EventVisibility, Modifiers, Stats, Table, World, wire_id,
};
use bota_core::{ModifierKind, NeutralAbility, Spell, ability_cooldown, ability_def, rules};
use bota_proto::{AbilityId, EventKind, Target, Team};

#[derive(Clone, Copy, Debug)]
pub struct BlockActivation {
    pub unit: Entity,
    pub ability: AbilityId,
}

/// Called only for a living, non-invulnerable target and an eligible spell.
/// Allegiance belongs to the spell's launch, independently of the source's lifetime.
pub(crate) fn absorb(
    target: Entity,
    from: Team,
    abilities: &mut Table<AbilityBook>,
    stats: &Table<Stats>,
    modifiers: &Table<Modifiers>,
    teams: &Table<Team>,
    free: bool,
) -> Option<BlockActivation> {
    if teams.get(target).is_none_or(|side| *side == from) {
        return None;
    }
    let broken = modifiers
        .get(target)
        .is_some_and(|mods| mods.active().any(|m| m.kind == ModifierKind::Broken));
    let book = abilities.get_mut(target)?;
    for state in &mut book.slots {
        if state.level == 0 || state.cooldown > 0 {
            continue;
        }
        let Some(ability) = NeutralAbility::of(state.id) else {
            continue;
        };
        let Spell::Block { breakable, .. } = ability.spell() else {
            continue;
        };
        if *breakable && broken {
            continue;
        }
        let rate = stats
            .get(target)
            .map_or(rules::NOMINAL_BP, |s| s.cooldown_rate_bp);
        state.cooldown = if free {
            0
        } else {
            cooldown_after(ability_cooldown(state.id, state.level), rate)
        };
        return Some(BlockActivation {
            unit: target,
            ability: state.id,
        });
    }
    None
}

impl World {
    /// None means the target is no longer eligible at its cast point.
    pub(crate) fn block_cast_target(
        &mut self,
        caster: Entity,
        ability: AbilityId,
        target: Target,
    ) -> Option<bool> {
        if !self.alive(caster) {
            return None;
        }
        let Target::Unit(id) = target else {
            return None;
        };
        let on = self.of_wire(id).filter(|on| self.alive(*on))?;
        let def = ability_def(ability)?;
        let (Some(side), Some(kind)) = (self.team.get(caster).copied(), self.kind.get(on).copied())
        else {
            return None;
        };
        let own = self.team.get(on) == Some(&side);
        if own
            || !self.can_see_of(caster, on)
            || !def.targets.takes(kind, own)
            || !self.spell_reaches(ability, on)
            || self.stats.get(on).is_none_or(|s| s.invulnerable)
            || !self.within_reach(caster, on, rules::units(self.reach_of(caster, def.range)))
        {
            return None;
        }
        let Some(activation) = absorb(
            on,
            side,
            &mut self.abilities,
            &self.stats,
            &self.modifiers,
            &self.team,
            self.wtf,
        ) else {
            return Some(false);
        };
        self.tell_ability_triggered(activation.unit, activation.ability);
        Some(true)
    }

    pub(crate) fn tell_ability_triggered(&mut self, caster: Entity, ability: AbilityId) {
        self.events.push(Event::Fact {
            kind: EventKind::AbilityCast {
                caster: wire_id(caster),
                ability,
            },
            visible_to: self.who_may_know(caster, None),
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }
}
