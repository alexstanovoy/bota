//! The neutral AI's casting: which spell a neutral lets go, and at whom.
//!
//! Every tick a neutral that is awake, set on a target and free to act reads
//! its spells in slot order and orders the first that is off cooldown and
//! paid for and whose [`bota_core::CastRule`] picks something (the decider is
//! `cast_rule.rs`). Ordering is all it does: the cast point,
//! the cost, the cooldown and the backswing are the action machinery's, the
//! same as a hero's. A cast is only ordered at something already in its
//! reach, since a neutral walking in to cast would drop the target it is
//! fighting; Dota's neutrals walk in. Areas are counted by the distance to
//! the centre, as the game's own searches of a radius do for these spells.

use bota_core::{ModifierKind, NeutralAbility, NeutralKind, Spell, When, kind_def, rules};
use bota_proto::{AbilitySlot, Target, Team, Vec2};

use crate::game::{AbilityState, ActionState, Entity, PendingCast, Purpose, World};

impl World {
    /// Runs everything a neutral's spells leave going, then lets each
    /// neutral order the spell it wants.
    pub fn tick_neutral_spells(&mut self) {
        self.tick_chains();
        self.tick_pushes();
        self.tick_purges();
        let entities = self.take_entity_snapshot();
        for caster in entities.iter().copied() {
            if self.neutral.get(caster).is_none() {
                continue;
            }
            if let Some((slot, target)) = self.wanted_cast(caster) {
                let slot = AbilitySlot(slot as u8);
                self.order_cast(caster, PendingCast::Ability { slot, target });
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Whether a neutral may begin a spell at all: awake and not walking
    /// home, and neither held, disarmed nor already casting.
    fn free_to_cast(&self, caster: Entity) -> bool {
        let awake = self
            .neutral_ai
            .get(caster)
            .is_some_and(|ai| ai.awake && !ai.going_home);
        let busy = matches!(
            self.action.get(caster).map(|action| action.state),
            None | Some(ActionState::CastAbility { .. } | ActionState::UseItem { .. })
        );
        let disarmed = self.modifiers.get(caster).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| held.kind == ModifierKind::Disarmed)
        });
        awake
            && !busy
            && !disarmed
            && self.alive(caster)
            && self.pending_cast(caster).is_none()
            && !self.held(caster)
            && !self.feared(caster)
    }

    /// The first spell a neutral wants to cast now, and at what.
    fn wanted_cast(&self, caster: Entity) -> Option<(usize, Target)> {
        let held = self.target_of(caster).filter(|held| self.alive(*held))?;
        if !self.free_to_cast(caster) {
            return None;
        }
        let book = self.abilities.get(caster)?;
        for (slot, state) in book.slots.iter().enumerate() {
            let Some(ability) = NeutralAbility::of(state.id) else {
                continue;
            };
            let Spell::Cast {
                when: When::Think(rule),
                ..
            } = *ability.spell()
            else {
                continue;
            };
            if !self.castable(caster, state) {
                continue;
            }
            if let Some(target) = self.cast_by_rule(caster, ability, &rule, held) {
                return Some((slot, target));
            }
        }
        None
    }

    /// Whether a spell is learned, off cooldown and paid for.
    fn castable(&self, caster: Entity, state: &AbilityState) -> bool {
        let cost = self.ability_mana_cost(caster, state.id, state.level);
        state.level > 0
            && state.cooldown == 0
            && self.mana.get(caster).map_or(0, |m| m.mana.to_int()) >= cost
    }

    /// Whether `other` is something a neutral's spells strike: not a
    /// building, a ward, a courier or anything damage passes by, and not of
    /// its side.
    pub(super) fn is_foe(&self, caster: Entity, other: Entity) -> bool {
        self.hostile_unseen(caster, other)
            && self.kind.get(other).is_some_and(|kind| {
                let def = kind_def(*kind);
                !def.structure && !def.fixed
            })
    }

    /// Every foe whose centre lies within a radius of a spot, in slot order.
    pub(super) fn foes_within(
        &self,
        caster: Entity,
        at: Vec2,
        radius: bota_proto::Fixed,
    ) -> Vec<Entity> {
        self.foes_near(caster, at, radius).collect()
    }

    /// Lets the neutrals of a camp answer the fall of one of their own: each
    /// that carries a spell for it and is free to cast it rolls its chance on
    /// the match's dice, and casts at whoever struck the blow.
    pub fn avenge(&mut self, fallen: &[(Entity, Option<Entity>)]) {
        for &(dead, killer) in fallen {
            let (Some(kind), Some(home), Some(killer)) = (
                self.neutral.get(dead).copied(),
                self.camp_home.get(dead).copied(),
                killer,
            ) else {
                continue;
            };
            let slain_by_a_side = self
                .team
                .get(killer)
                .is_some_and(|side| *side != Team::Neutral);
            if !slain_by_a_side || !self.alive(killer) {
                continue;
            }
            let mates: Vec<Entity> = self
                .entities
                .iter()
                .filter(|mate| {
                    *mate != dead
                        && self
                            .camp_home
                            .get(*mate)
                            .is_some_and(|h| h.camp == home.camp)
                        && self.neutral.get(*mate).is_some()
                })
                .collect();
            for mate in mates {
                self.avenge_with(mate, kind, killer);
            }
        }
    }

    /// One neutral's answer to the fall of a neutral of a kind.
    fn avenge_with(&mut self, mate: Entity, fallen: NeutralKind, killer: Entity) {
        if !self.free_to_cast(mate) {
            return;
        }
        let Some((slot, chance, ability)) = self.avenging_slot(mate, fallen) else {
            return;
        };
        if self.rng.global(Purpose::NeutralCast).below(100) as i32 >= chance {
            return;
        }
        let (Some(from), Some(there)) = (self.pos(mate), self.pos(killer)) else {
            return;
        };
        let range = rules::units(ability.entry().def.range);
        let spot = if from.within(there, range) {
            there
        } else {
            bota_core::point_along(from, there, range)
        };
        let slot = AbilitySlot(slot as u8);
        let target = Target::Pos(spot);
        self.order_cast(mate, PendingCast::Ability { slot, target });
    }

    /// The slot of the first spell a neutral holds for the fall of a kind
    /// that is learned, off cooldown and paid for, with its chance.
    fn avenging_slot(
        &self,
        mate: Entity,
        fallen: NeutralKind,
    ) -> Option<(usize, i32, NeutralAbility)> {
        let book = self.abilities.get(mate)?;
        book.slots.iter().enumerate().find_map(|(slot, state)| {
            let ability = NeutralAbility::of(state.id)?;
            let Spell::Cast {
                when: When::MateFalls { kind, chance_pct },
                ..
            } = *ability.spell()
            else {
                return None;
            };
            (kind == fallen && self.castable(mate, state)).then_some((slot, chance_pct, ability))
        })
    }
}
