//! What a neutral's spell does once it goes off: one mechanic to each kind
//! of [`Act`], read from the spell's data.
//!
//! An area is the foes whose centre lies within it. A hit lands in the tick
//! of the cast, after the modifiers it comes with; a disable is shortened by
//! the status resistance of what wears it. The mechanics that strike lie in
//! [`spell_strike`](super::spell_strike), those that call something in
//! [`spell_call`](super::spell_call).

use bota_core::{
    Act, ModifierKind, NeutralAbility, Shared, Spell, Then, at_level, effect_def, rules,
    ticks_of_ms,
};
use bota_proto::{Inflictor, Target};

use crate::game::{Entity, Modifier, World};

/// Who casts what, at which level.
#[derive(Clone, Copy)]
pub(super) struct Cast {
    pub caster: Entity,
    pub ability: NeutralAbility,
    pub level: u8,
}

impl Cast {
    /// The spell, as what it does names it.
    pub fn inflictor(self) -> Option<Inflictor> {
        Some(Inflictor::Ability(self.ability.id()))
    }
}

impl World {
    /// Lets one neutral spell go off from `caster` at `target`, at `level`.
    /// False when it found nothing to do, and then nothing is spent.
    pub fn cast_spell(
        &mut self,
        caster: Entity,
        ability: NeutralAbility,
        level: u8,
        target: Target,
    ) -> bool {
        let Spell::Cast { act, shared, .. } = *ability.spell() else {
            return false;
        };
        let cast = Cast {
            caster,
            ability,
            level,
        };
        let went = match act {
            Act::Stomp { .. } => self.stomp(cast, act),
            Act::Bolt { .. } => self.bolt(cast, act, target),
            Act::Chain { .. } => self.chain_lightning(cast, act, target),
            Act::Summon { .. } => self.summon(cast, act),
            Act::Enrage { ms, .. } => self.wear(cast, caster, ms, 0),
            Act::Smash { .. } => self.smash(cast, act, target),
            Act::Spikes { .. } => self.spikes(cast, act, target),
            Act::Bubble { .. } => self.bubble(cast, act, target),
            Act::Push { .. } => self.push_away(cast, act, target),
            Act::Purge { .. } => self.purge(cast, act, target),
            Act::Tornado { .. } => self.tornado(cast, act, target),
        };
        if went && let Some(shared) = shared {
            self.hold_others(caster, shared);
        }
        went
    }

    /// Puts the spell's own modifier on an entity for `ms`, with `amount`
    /// the spell's own count.
    pub(super) fn wear(&mut self, cast: Cast, on: Entity, ms: i32, amount: u16) -> bool {
        let kind = ModifierKind::Spell {
            ability: cast.ability,
            level: cast.level,
            amount,
        };
        let ticks = Some(ticks_of_ms(ms));
        let held = Modifier::new(kind, Some(cast.caster), cast.inflictor(), ticks);
        self.put_modifier(on, held);
        true
    }

    /// Stuns an entity for `ms`.
    pub(super) fn stun_for(&mut self, cast: Cast, on: Entity, ms: i32) {
        let ticks = Some(ticks_of_ms(ms));
        let kind = ModifierKind::Stunned;
        let held = Modifier::new(kind, Some(cast.caster), cast.inflictor(), ticks);
        self.put_modifier(on, held);
    }

    /// The unit a cast is aimed at, when it is a foe.
    pub(super) fn foe_aimed_at(&self, cast: Cast, target: Target) -> Option<Entity> {
        let on = self.unit_aimed_at(target)?;
        self.is_foe(cast.caster, on).then_some(on)
    }

    /// The living unit a cast is aimed at.
    pub(super) fn unit_aimed_at(&self, target: Target) -> Option<Entity> {
        let Target::Unit(id) = target else {
            return None;
        };
        self.of_wire(id).filter(|on| self.alive(*on))
    }

    /// Every foe within the radius stomps take: damage, then a stun or the
    /// spell's own modifier, for as long as the spell holds a hero and a
    /// creep.
    fn stomp(&mut self, cast: Cast, act: Act) -> bool {
        let Act::Stomp {
            radius,
            damage,
            kind,
            hero_ms,
            other_ms,
            then,
        } = act
        else {
            return false;
        };
        let Some(at) = self.pos(cast.caster) else {
            return false;
        };
        let reach = rules::units(at_level(&radius, cast.level));
        for foe in self.foes_within(cast.caster, at, reach) {
            let ms = if self.is_hero(foe) { hero_ms } else { other_ms };
            let ms = at_level(&ms, cast.level);
            match then {
                Then::Stun => self.stun_for(cast, foe, ms),
                Then::Wear { .. } => {
                    self.wear(cast, foe, ms, 0);
                }
            }
            let damage = at_level(&damage, cast.level);
            if damage > 0 {
                self.push_hit(Some(cast.caster), foe, damage, kind, cast.inflictor());
            }
        }
        true
    }

    /// A barrier on an ally. One bubble replaces another, whichever of the
    /// Water Bubbles raised it.
    fn bubble(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Bubble { absorb, ms, .. } = act else {
            return false;
        };
        let Some(ally) = self.unit_aimed_at(target) else {
            return false;
        };
        if self.team.get(ally) != self.team.get(cast.caster) {
            return false;
        }
        let soak = u16::try_from(at_level(&absorb, cast.level)).unwrap_or(u16::MAX);
        self.wear(cast, ally, ms, soak)
    }

    /// Takes the buffs a basic dispel takes off the target, and slows it,
    /// the slow wearing off in [`World::tick_purges`].
    fn purge(&mut self, cast: Cast, act: Act, target: Target) -> bool {
        let Act::Purge { start_pct, ms } = act else {
            return false;
        };
        let Some(on) = self.foe_aimed_at(cast, target) else {
            return false;
        };
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.0.retain(|held| {
                !effect_def(held.kind.effect_id()).is_some_and(|def| def.dispellable)
            });
        }
        self.wear(cast, on, ms, u16::try_from(start_pct).unwrap_or(0))
    }

    /// Puts every other neutral within a hold's radius that carries a spell
    /// of its family on the hold's cooldown, unless it waits longer already.
    fn hold_others(&mut self, caster: Entity, hold: Shared) {
        let Some(at) = self.pos(caster) else {
            return;
        };
        let reach = rules::units(hold.radius);
        let ticks = ticks_of_ms(hold.ms);
        let others: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                *other != caster
                    && self.neutral.get(*other).is_some()
                    && self.pos(*other).is_some_and(|pos| pos.within(at, reach))
            })
            .collect();
        for other in others {
            let Some(book) = self.abilities.get_mut(other) else {
                continue;
            };
            for state in book.slots.iter_mut() {
                let family =
                    NeutralAbility::of(state.id).and_then(|ability| match ability.spell() {
                        Spell::Cast { shared, .. } => shared.map(|shared| shared.family),
                        _ => None,
                    });
                if family == Some(hold.family) {
                    state.cooldown = state.cooldown.max(ticks);
                }
            }
        }
    }
}
