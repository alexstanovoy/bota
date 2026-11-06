//! What is on a unit: putting it on, doing its work each tick, taking it off.

use bota_core::{
    ModifierKind, Ms, Stat, breaks_regen, dismember_bite_tick, item_def, kind_def, rules,
    spell_buff,
};
use bota_proto::{AbilityId, DamageKind, Fixed, Inflictor, ItemId, Team};

use crate::game::{Landed, Modifier, Stats, Table, World, cooldown_after, disabled_in};

use super::grenade::grenade_of;
use crate::engine::Entity;

/// Whether a modifier is a disable or slow that status resistance shortens.
/// A neutral's spell counts when it slows the bearer. Dota shortens roots,
/// disarms and breaks like stuns.
fn shortened_by_resistance(kind: ModifierKind) -> bool {
    match kind {
        ModifierKind::Control { .. } => true,
        ModifierKind::Spell { ability, level, .. } => {
            spell_buff(ability).is_some_and(|buff| buff.get(Stat::MovePct, level) < 0)
        }
        _ => matches!(
            kind,
            ModifierKind::Stunned
                | ModifierKind::Dismembered { .. }
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

/// What a rot of a level burns each [`rules::ROT_TICK_TICKS`].
fn rot_beat(level: u8) -> i32 {
    rules::ROT_DAMAGE_PER_SECOND[usize::from(level)] * rules::ROT_TICK_TICKS as i32
        / rules::TICKS_PER_SECOND as i32
}

impl World {
    pub(crate) fn dispel(
        &mut self,
        on: Entity,
        polarity: crate::game::Dispel,
        power: crate::game::DispelPower,
    ) {
        if let Some(mods) = self.modifiers.get_mut(on) {
            mods.0.retain(|m| {
                !bota_core::effect_def(m.kind.effect_id()).is_some_and(|d| {
                    d.harmful == matches!(polarity, crate::game::Dispel::Debuffs)
                        && (d.dispellable
                            || (power == crate::game::DispelPower::Strong && d.strong_dispellable))
                })
            });
        }
    }

    pub fn controlled(&self, entity: Entity, control: bota_core::Control) -> bool {
        self.modifiers
            .get(entity)
            .is_some_and(|mods| mods.active().any(|m| m.kind.controls(control)))
    }
    /// Whether an entity is held: it neither walks, nor turns, nor swings,
    /// nor casts until it lifts.
    pub fn silenced(&self, entity: Entity) -> bool {
        self.controlled(entity, bota_core::Control::Silence)
    }

    pub fn held(&self, entity: Entity) -> bool {
        self.controlled(entity, bota_core::Control::Stun)
            || self.push.get(entity).is_some_and(|push| push.carries())
            || self.landing_hold.binary_search(&entity).is_ok()
    }

    /// Whether an entity is rooted: it cannot walk, nor turn to walk, but
    /// still swings and casts.
    pub fn rooted(&self, entity: Entity) -> bool {
        self.controlled(entity, bota_core::Control::Root)
    }

    /// Whether an entity is disarmed: it cannot begin a swing. A neutral
    /// that is takes nothing on, stands still and casts nothing. An ethereal
    /// one is disarmed as well (Dota's Ghost Form: "unable to attack").
    pub fn disarmed(&self, entity: Entity) -> bool {
        self.controlled(entity, bota_core::Control::Disarm) || self.attack_immune(entity)
    }

    /// The world units an AoE bonus adds to an entity's radii that it widens: what it carries.
    pub(crate) fn aoe_bonus_of(&self, entity: Entity) -> i32 {
        self.stats.get(entity).map_or(0, |stats| stats.aoe_bonus)
    }

    /// One of an entity's radii, `radius` world units, as its AoE bonus widens it: a radius its
    /// script marks `affected_by_aoe_increase` ([`bota_core::AOE_ABILITY_VALUES`], an item's
    /// [`ItemData::aoe`](bota_core::ItemData::aoe)).
    pub(crate) fn widened_of(&self, entity: Entity, radius: i32) -> Fixed {
        rules::units(bota_core::widened(radius, self.aoe_bonus_of(entity)))
    }

    /// Whether no attack reaches an entity: an ethereal one (Dota's Ghost
    /// Form: "cannot attack or be attacked"), whether the attack is an
    /// enemy's or an ally's.
    pub fn attack_immune(&self, entity: Entity) -> bool {
        self.stats.get(entity).is_some_and(|stats| stats.ethereal)
    }

    /// Whether an entity is muted: it uses no item.
    pub fn muted(&self, entity: Entity) -> bool {
        self.wears_state(entity, ModifierKind::mutes)
    }

    /// Whether an entity wears an active modifier of a kind `state` holds for.
    fn wears_state(&self, entity: Entity, state: fn(ModifierKind) -> bool) -> bool {
        self.modifiers
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|held| state(held.kind)))
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

    /// Whether an entity wears what keeps it from taking anybody on of its
    /// own accord ([`ModifierKind::disables_auto_attack`]).
    pub fn auto_attack_disabled(&self, entity: Entity) -> bool {
        self.modifiers
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|held| held.kind.disables_auto_attack()))
    }

    /// Whoever a feared entity runs from, while that one is still known.
    pub fn flees_from(&self, entity: Entity) -> Option<Entity> {
        self.active_modifier(entity, ModifierKind::Feared)?.source
    }

    /// Whether an entity wears an active modifier of exactly this kind.
    pub(crate) fn wears(&self, entity: Entity, kind: ModifierKind) -> bool {
        self.modifiers
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|held| held.kind == kind))
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
        self.store_modifier(on, modifier);
    }

    /// Stores an already resisted modifier, recording effective harmful participation.
    /// Auras pass here too without resisting their linger a second time.
    pub(super) fn store_modifier(&mut self, on: Entity, modifier: Modifier) {
        if self.modifiers.get(on).is_none() {
            return;
        }
        if hostile(modifier.kind) && !disabled_in(&self.modifiers, &modifier) {
            self.note_hurt_by(modifier.source, on);
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
                        if !on_beat || amount <= 0 {
                            continue;
                        }
                        if lethal && self.alive(entity) {
                            self.push_hit(
                                modifier.source,
                                entity,
                                amount,
                                kind,
                                modifier.inflictor,
                            );
                        } else if !lethal && self.above_last_point(entity) {
                            self.push_sparing(
                                modifier.source,
                                entity,
                                amount,
                                kind,
                                modifier.inflictor,
                                false,
                            );
                        }
                    }
                    ModifierKind::Rot { level, since } => self.rot(entity, level, since),
                    ModifierKind::Spell {
                        ability,
                        level,
                        amount,
                    } => self.spell_pulse(entity, modifier, (ability, level, amount)),
                    ModifierKind::Dismembered {
                        level,
                        bites,
                        channel_ms,
                    } => {
                        // The bites share the channel as long as it holds: a
                        // channel status resistance shortened, shortened alike.
                        let (bites, channel) = (u32::from(bites), Ms(u32::from(channel_ms)));
                        let duration = modifier.ticks_total.unwrap_or(channel.ticks());
                        let held_for = duration.saturating_sub(modifier.ticks_left.unwrap_or(0));
                        let lands = |bite| {
                            (dismember_bite_tick(bite, bites, channel) * duration)
                                .div_ceil(channel.ticks().max(1))
                                == held_for
                        };
                        if (1..bites).any(lands)
                            && let Some(holder) = modifier.source
                            && self.alive(entity)
                        {
                            self.dismember_bites(
                                holder,
                                entity,
                                (usize::from(level), bites, channel),
                            );
                        }
                    }
                    ModifierKind::Laid { item, since } => {
                        self.laid_beat(entity, modifier, item, since)
                    }
                    ModifierKind::BloodGrenade { since } => {
                        let Some(spec) = grenade_of(modifier.inflictor) else {
                            continue;
                        };
                        let since_then = self.tick.saturating_sub(since);
                        if since_then > 0
                            && since_then.is_multiple_of(rules::TICKS_PER_SECOND)
                            && self.alive(entity)
                        {
                            self.push_hit(
                                modifier.source,
                                entity,
                                spec.damage_per_second,
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

    /// One tick of a rot switched on at `since`: it burns its owner, never to
    /// death, at `since` and every [`rules::ROT_TICK_TICKS`] after, and lays
    /// itself on every enemy hero and basic unit standing in it, burning each
    /// from the tick it was laid on, on the same rhythm, while it stands there
    /// (Liquipedia, *Pudge*: "starting immediately upon debuff application").
    ///
    /// What it lays is handed out afresh every tick for its linger, so what
    /// walks out of it, or is left by its switching off, stays slowed that
    /// long, unburned. A rot whose owner is dead goes out.
    pub(crate) fn rot(&mut self, owner: Entity, level: u8, since: u32) {
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
        // What the owner's talents make of the level's slow, as Dota's rot
        // writes it: a change of speed.
        let slow = -self
            .talented(owner, rules::ROT_SLOW_VALUE, usize::from(level))
            .int();
        let slow_pct = u8::try_from(slow.clamp(0, 100)).expect("a percent");
        if (self.tick - since).is_multiple_of(rules::ROT_TICK_TICKS) && self.above_last_point(owner)
        {
            self.push_sparing(
                None,
                owner,
                rot_beat(level),
                DamageKind::Magical,
                with,
                false,
            );
        }
        let reach = self.widened_of(owner, rules::ROT_RADIUS);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if other == owner || !self.alive(other) || self.hull.get(other).is_none() {
                continue;
            }
            // Dota's heroes and basic units: never a building or a ward.
            if !self
                .kind
                .get(other)
                .is_some_and(|&kind| kind_def(kind).struck_by_area_spells)
            {
                continue;
            }
            if !self.pos(other).is_some_and(|pos| pos.within(at, reach)) {
                continue;
            }
            if self.team.get(other).copied() != Some(side)
                && self.spell_reaches(AbilityId::ROT, other)
            {
                // Still rotting from this rot, it keeps the tick it was laid on.
                let laid = self.modifiers.get(other).and_then(|on_it| {
                    on_it.active().find_map(|held| match held.kind {
                        ModifierKind::Rotting { since, .. } if held.source == Some(owner) => {
                            Some(since)
                        }
                        _ => None,
                    })
                });
                let laid = laid.unwrap_or(self.tick);
                let rotting = ModifierKind::Rotting {
                    level,
                    slow_pct,
                    since: laid,
                };
                let lingers = Some(rules::ROT_LINGER_TICKS);
                self.put_modifier(other, Modifier::new(rotting, Some(owner), with, lingers));
                if (self.tick - laid).is_multiple_of(rules::ROT_TICK_TICKS) {
                    self.push_hit(
                        Some(owner),
                        other,
                        rot_beat(level),
                        DamageKind::Magical,
                        with,
                    );
                }
            }
        }
    }

    /// The beat of what an item laid that removes health: its `dps` over
    /// each `beat_ticks` since it was laid, a magical HP removal that never
    /// takes the last point (Dota 7.07: Poison Attack "is no longer lethal
    /// and is now flagged as HP Removal").
    fn laid_beat(&mut self, on: Entity, modifier: Modifier, item: ItemId, since: u32) {
        let Some(laid) = item_def(item).and_then(|def| def.on_attack) else {
            return;
        };
        let since_then = self.tick.saturating_sub(since);
        if laid.dps <= 0 || since_then == 0 || !since_then.is_multiple_of(laid.beat_ticks) {
            return;
        }
        if self.above_last_point(on) {
            let amount = laid.dps * laid.beat_ticks as i32 / rules::TICKS_PER_SECOND as i32;
            self.push_sparing(
                modifier.source,
                on,
                amount,
                DamageKind::Magical,
                modifier.inflictor,
                true,
            );
        }
    }

    /// Whether an entity has health above its last point, for a blow that
    /// spares it to take.
    fn above_last_point(&self, entity: Entity) -> bool {
        self.health
            .get(entity)
            .is_some_and(|health| health.hp.to_int() > 1)
    }

    /// Puts out every drink a blow is enough to break, and sets back every
    /// item that answers to one: the blows [`breaks_regen`] names. A drink
    /// that does not break and an item with no break cooldown are left alone.
    pub fn break_on_blows(&mut self, felt: &[Landed]) {
        for blow in felt.iter().filter(|blow| !blow.removal) {
            if blow.amount > 0
                && blow.source.is_some_and(|source| {
                    source != blow.target
                        && (self.owner.get(source).is_some()
                            || self.kind.get(source) == Some(&bota_proto::UnitKind::Roshan))
                })
                && let Some(on_it) = self.modifiers.get_mut(blow.target)
            {
                on_it
                    .0
                    .retain(|held| !matches!(held.kind, ModifierKind::Recovery { .. }));
            }
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
                            | ModifierKind::BottleRegeneration { .. }
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

/// Whether a modifier is a debuff its bearer's enemies put on it, which credits them an assist
/// should the bearer fall soon after (Dota's wiki, *Gold*: "dealt damage or applied a debuff"):
/// the effect it shows is a harmful one.
fn hostile(kind: ModifierKind) -> bool {
    bota_core::effect_def(kind.effect_id()).is_some_and(|def| def.harmful)
}
