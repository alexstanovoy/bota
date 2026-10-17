//! What is on a unit: putting it on, doing its work each tick, taking it off.

use bota_core::{ModifierKind, breaks_regen, item_def, rules};
use bota_proto::{AbilityId, DamageKind, Fixed, Team};

use crate::game::{Landed, Modifier, Stats, Table, World, cooldown_after};

use crate::engine::Entity;

/// How long a disable or slow holds on an entity after its status
/// resistance. Every other kind is left as it is.
///
/// Every timed disable and slow passes here, whatever hands it out: an
/// ability, an item or an aura.
pub fn resisted_ticks(stats: &Table<Stats>, on: Entity, kind: ModifierKind, ticks: u32) -> u32 {
    if !matches!(
        kind,
        ModifierKind::Stunned
            | ModifierKind::Feared
            | ModifierKind::Slowed { .. }
            | ModifierKind::MagicBroken { .. }
            | ModifierKind::BloodGrenade { .. }
    ) {
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

    /// Whether an entity is feared: it runs from whoever put the fear on,
    /// and neither swings nor casts until it lifts.
    pub fn feared(&self, entity: Entity) -> bool {
        self.modifiers
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|held| held.kind == ModifierKind::Feared))
    }

    /// Whoever a feared entity runs from, while that one is still known.
    pub fn flees_from(&self, entity: Entity) -> Option<Entity> {
        self.modifiers.get(entity).and_then(|on_it| {
            on_it
                .active()
                .find(|held| held.kind == ModifierKind::Feared)
                .and_then(|held| held.source)
        })
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

    /// Takes off every modifier of one kind from one source.
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
                            self.push_hit(modifier.source, entity, amount, kind);
                        }
                    }
                    ModifierKind::Rot { level } => self.rot(entity, level),
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

    /// Burns and slows every unit of another side with a hull standing in
    /// one entity's rot. Its owner burns by the same amount, never to death,
    /// and is not slowed.
    ///
    /// What it does is handed out afresh every tick for two ticks, so walking
    /// out of it or switching it off lifts it. A rot whose owner is dead goes
    /// out.
    fn rot(&mut self, owner: Entity, level: u8) {
        if !self.alive(owner) {
            self.rot_goes_out(owner);
            return;
        }
        let (Some(at), Some(side)) = (
            self.transform.get(owner).map(|t| t.pos),
            self.team.get(owner).copied(),
        ) else {
            return;
        };
        // The cloud shown for it stands where its owner now stands.
        if let Some(shown) = self.mark_of(owner, AbilityId::ROT)
            && let Some(transform) = self.transform.get_mut(shown)
        {
            transform.pos = at;
        }
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
            if self.team.get(other).copied() != Some(side) {
                self.rot_on(owner, other, amount, level);
            }
        }
    }

    /// Burns `other` by `amount` and slows it, for two ticks, from `owner`'s
    /// rot.
    fn rot_on(&mut self, owner: Entity, other: Entity, amount: i32, level: u8) {
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
