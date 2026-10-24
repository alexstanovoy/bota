//! What is on a unit: putting it on, doing its work each tick, taking it off.

use bota_core::{
    ModifierKind, Stat, breaks_regen, dismember_bite_tick, item_def, rules, spell_buff,
};
use bota_proto::{AbilityId, DamageKind, Fixed, Inflictor, Team};

use crate::game::{Landed, Modifier, Stats, Table, World, cooldown_after, disabled_in};

use crate::engine::Entity;

/// Whether a modifier is a disable or slow that status resistance shortens.
/// A neutral's spell counts when it slows the bearer. Dota shortens roots,
/// disarms and breaks like stuns.
fn shortened_by_resistance(kind: ModifierKind) -> bool {
    match kind {
        ModifierKind::Spell { ability, level, .. } => {
            spell_buff(ability).is_some_and(|buff| buff.get(Stat::MovePct, level) < 0)
        }
        _ => matches!(
            kind,
            ModifierKind::Stunned
                | ModifierKind::Feared
                | ModifierKind::Slowed { .. }
                | ModifierKind::Rotting { .. }
                | ModifierKind::MagicBroken { .. }
                | ModifierKind::BloodGrenade { .. }
                | ModifierKind::Rooted
                | ModifierKind::Disarmed
                | ModifierKind::Broken
        ),
    }
}

/// How long a disable or slow holds on an entity after its status
/// resistance. Every other kind is left as it is.
///
/// Every timed disable and slow passes here, whatever hands it out: an
/// ability, an item or an aura.
pub fn resisted_ticks(stats: &Table<Stats>, on: Entity, kind: ModifierKind, ticks: u32) -> u32 {
    if !shortened_by_resistance(kind) {
        return ticks;
    }
    let resist = stats
        .get(on)
        .map_or(0, |stats| stats.status_resist_bp)
        .clamp(0, rules::NOMINAL_BP - 1);
    let kept =
        i64::from(ticks) * i64::from(rules::NOMINAL_BP - resist) / i64::from(rules::NOMINAL_BP);
    kept.max(1) as u32
}

/// What a rot of a level burns on one beat of [`rules::BURN_PERIOD_TICKS`].
fn rot_beat(level: u8) -> i32 {
    rules::ROT_DAMAGE_PER_SECOND[usize::from(level)] * rules::BURN_PERIOD_TICKS as i32
        / rules::TICKS_PER_SECOND as i32
}

impl World {
    /// Whether an entity is held: it neither walks, nor turns, nor swings,
    /// nor casts until it lifts.
    pub fn held(&self, entity: Entity) -> bool {
        self.active_modifier(entity, ModifierKind::Stunned)
            .is_some()
    }

    /// Whether an entity is rooted: it cannot walk, nor turn to walk, but
    /// still swings and casts.
    pub fn rooted(&self, entity: Entity) -> bool {
        self.active_modifier(entity, ModifierKind::Rooted).is_some()
    }

    /// Whether an entity is disarmed: it cannot begin a swing. A neutral
    /// that is takes nothing on, stands still and casts nothing.
    pub fn disarmed(&self, entity: Entity) -> bool {
        self.active_modifier(entity, ModifierKind::Disarmed)
            .is_some()
    }

    /// Whether an entity is broken: its own passives do nothing. What the
    /// auras a Break silences put on stays on and does nothing
    /// ([`World::disabled`]); the blows it lays strikes on and the throes of
    /// its death stop; a building's aura and what others put on it work as
    /// before.
    pub fn broken(&self, entity: Entity) -> bool {
        self.active_modifier(entity, ModifierKind::Broken).is_some()
    }

    /// Whether a modifier on a unit is disabled: [`disabled_in`].
    pub fn disabled(&self, held: &Modifier) -> bool {
        disabled_in(&self.modifiers, held)
    }

    /// Whether an entity is feared: it runs from whoever put the fear on,
    /// and neither swings nor casts until it lifts.
    pub fn feared(&self, entity: Entity) -> bool {
        self.active_modifier(entity, ModifierKind::Feared).is_some()
    }

    /// Whoever a feared entity runs from, while that one is still known.
    pub fn flees_from(&self, entity: Entity) -> Option<Entity> {
        self.active_modifier(entity, ModifierKind::Feared)?.source
    }

    /// The first active modifier of exactly this kind on an entity.
    fn active_modifier(&self, entity: Entity, kind: ModifierKind) -> Option<&Modifier> {
        self.modifiers
            .get(entity)?
            .active()
            .find(|held| held.kind == kind)
    }

    /// Puts a modifier on an entity. One that is not a unit takes nothing.
    ///
    /// A timed disable or slow is shortened first by the target's status
    /// resistance, down to one tick.
    pub fn put_modifier(&mut self, on: Entity, mut modifier: Modifier) {
        if let Some(ticks) = modifier.ticks_left {
            modifier.ticks_left = Some(resisted_ticks(&self.stats, on, modifier.kind, ticks));
        }
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.put(modifier);
        }
    }

    pub fn take_modifier(&mut self, on: Entity, like: ModifierKind, source: Option<Entity>) {
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.take(like, source);
        }
    }

    /// Does every modifier's work for the tick: what burns takes health on
    /// the beat, a Blood Grenade's debuff on every whole second of its own,
    /// and the rot puts its burn and slow on everything standing in it.
    /// Their time runs down in [`World::tick_gear`].
    ///
    /// A burn that is not lethal stops one short of the last point.
    pub fn tick_modifiers(&mut self) {
        let on_beat = self.tick.is_multiple_of(rules::BURN_PERIOD_TICKS);
        let entities = self.take_entity_snapshot();
        // One buffer for every entity this tick: `active()` borrows the
        // modifier table while the effects below edit other tables.
        let mut held = std::mem::take(&mut self.modifier_scratch);
        held.clear();
        for entity in entities.iter().copied() {
            let Some(on_it) = self.modifiers.get(entity) else {
                continue;
            };
            held.extend(on_it.active().copied());
            for modifier in held.drain(..) {
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
                            self.push_hit(
                                modifier.source,
                                entity,
                                amount,
                                kind,
                                modifier.inflictor,
                            );
                        }
                    }
                    ModifierKind::Rot { level } => self.rot(entity, level, on_beat),
                    ModifierKind::Rotting { level } => {
                        if on_beat && self.alive(entity) {
                            self.push_hit(
                                modifier.source,
                                entity,
                                rot_beat(level),
                                DamageKind::Magical,
                                modifier.inflictor,
                            );
                        }
                    }
                    ModifierKind::Spell {
                        ability,
                        level,
                        amount,
                    } => self.spell_pulse(entity, modifier, (ability, level, amount)),
                    ModifierKind::Dismembered { level } => {
                        let held_for = rules::DISMEMBER_TICKS - modifier.ticks_left.unwrap_or(0);
                        let bites = 1..rules::DISMEMBER_BITES;
                        if bites.map(dismember_bite_tick).any(|tick| tick == held_for)
                            && let Some(holder) = modifier.source
                            && self.alive(entity)
                        {
                            self.dismember_bites(holder, entity, usize::from(level));
                        }
                    }
                    ModifierKind::BloodGrenade { since } => {
                        let since_then = self.tick.saturating_sub(since);
                        if since_then > 0
                            && since_then.is_multiple_of(rules::TICKS_PER_SECOND)
                            && self.alive(entity)
                        {
                            self.push_hit(
                                modifier.source,
                                entity,
                                rules::GRENADE_DAMAGE_PER_SECOND,
                                DamageKind::Magical,
                                modifier.inflictor,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        assert!(held.is_empty());
        self.modifier_scratch = held;
        self.recycle_entity_snapshot(entities);
    }

    /// Burns its owner on the beat, never to death, and lays the rot on
    /// every unit of another side with a hull standing in it.
    ///
    /// What it lays is handed out afresh every tick for two ticks, so walking
    /// out of it or switching it off lifts it. A rot whose owner is dead goes
    /// out.
    fn rot(&mut self, owner: Entity, level: u8, on_beat: bool) {
        if !self.alive(owner) {
            self.rot_goes_out(owner);
            return;
        }
        let (Some(at), Some(side)) = (self.pos(owner), self.team.get(owner).copied()) else {
            return;
        };
        // The cloud shown for it stands where its owner now stands.
        if let Some(shown) = self.mark_of(owner, AbilityId::ROT)
            && let Some(transform) = self.transform.get_mut(shown)
        {
            transform.pos = at;
        }
        let with = Some(Inflictor::Ability(AbilityId::ROT));
        let left = self
            .health
            .get(owner)
            .map_or(0, |health| health.hp.to_int());
        let own = rot_beat(level).min(left - 1);
        if on_beat && own > 0 {
            self.push_hit(None, owner, own, DamageKind::Magical, with);
        }
        let reach = rules::units(rules::ROT_RADIUS);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if other == owner || !self.alive(other) || self.hull.get(other).is_none() {
                continue;
            }
            if !self.pos(other).is_some_and(|pos| pos.within(at, reach)) {
                continue;
            }
            if self.team.get(other).copied() != Some(side)
                && self.spell_reaches(AbilityId::ROT, other)
            {
                let rotting = ModifierKind::Rotting { level };
                self.put_modifier(other, Modifier::new(rotting, Some(owner), with, Some(2)));
            }
        }
    }

    /// Puts out every drink a blow is enough to break, and sets back every
    /// item that answers to one: the blows [`breaks_regen`] names. A drink
    /// that does not break and an item with no break cooldown are left alone.
    pub fn break_on_blows(&mut self, felt: &[Landed]) {
        for blow in felt {
            let breaks = blow.source.is_some_and(|from| {
                self.kind.get(from).is_some_and(|&kind| {
                    let side = self.team.get(from).copied().unwrap_or(Team::Neutral);
                    breaks_regen(kind, side, blow.side)
                })
            });
            if !breaks {
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
            let rate = self
                .stats
                .get(blow.target)
                .map_or(rules::NOMINAL_BP, |stats| stats.cooldown_rate_bp);
            if let Some(bag) = self.inventory.get_mut(blow.target) {
                for stack in bag.slots.iter_mut().flatten() {
                    let Some(def) = item_def(stack.id) else {
                        continue;
                    };
                    if def.breaks_on_damage > 0 {
                        stack.cooldown = stack
                            .cooldown
                            .max(cooldown_after(def.breaks_on_damage, rate));
                    }
                }
            }
        }
    }
}
