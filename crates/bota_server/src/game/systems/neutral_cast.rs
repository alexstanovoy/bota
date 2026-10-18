//! The neutral AI's casting: which spell a neutral lets go, and at whom.
//!
//! Every tick a neutral that is awake, set on a target and free to act reads
//! its spells in slot order and orders the first whose [`When`] holds and
//! that is off cooldown and paid for. Ordering is all it does: the cast point,
//! the cost, the cooldown and the backswing are the action machinery's, the
//! same as a hero's. A cast is only ordered at something already in its
//! reach, since a neutral walking in to cast would drop the target it is
//! fighting; Dota's neutrals walk in. Areas are counted by the distance to
//! the centre, as the game's own searches of a radius do for these spells.

use bota_core::{
    ModifierKind, NeutralAbility, NeutralKind, Spell, WATER_BUBBLE_EFFECT, When, effect_def,
    kind_def, rules, spell_effect,
};
use bota_proto::{AbilitySlot, Aim, Target, Team, UnitKind, Vec2};

use crate::game::{
    AbilityState, ActionState, Entity, PendingCast, Purpose, UnitOrder, World, wire_id,
};

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
            let Spell::Cast { when, .. } = *ability.spell() else {
                continue;
            };
            if !self.castable(caster, state) {
                continue;
            }
            if let Some(target) = self.target_for(caster, ability, when, held) {
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

    /// What a spell would be cast at, if its condition holds. `held` is who
    /// the neutral is set on.
    fn target_for(
        &self,
        caster: Entity,
        ability: NeutralAbility,
        when: When,
        held: Entity,
    ) -> Option<Target> {
        let at = self.pos(caster)?;
        let def = &ability.entry().def;
        match when {
            When::Crowd { radius, at_least } => {
                let near = self.foes_within(caster, at, rules::units(radius));
                (near.len() >= usize::from(at_least)).then_some(Target::None)
            }
            When::Beset { radius, at_least } => self.beset(caster, at, radius, at_least),
            When::Hurt { below_pct, radius } => {
                let foe = self.hurt_foe(caster, at, below_pct, radius, held)?;
                self.aimed(def.aim, foe)
            }
            When::Weak { below_hp, attacker } => {
                self.weak_foe(caster, at, def.range, below_hp, attacker)
            }
            When::Fighting => self
                .in_reach(caster, held)
                .then(|| Target::Unit(wire_id(caster))),
            When::AllyHurt { below_pct } => self.hurt_ally(caster, at, def.range, below_pct),
            When::Duel { radius } => self.duel(caster, at, def.range, radius, held),
            When::Chaser { radius } => self.chaser(caster, at, def.range, radius, held),
            When::Buffed { radius } => self.buffed_hero(caster, at, def.range.min(radius)),
            When::Target => {
                let near = self.pos(held)?.within(at, rules::units(def.range));
                near.then(|| self.aimed(def.aim, held)).flatten()
            }
            When::Avenge { .. } => None,
        }
    }

    /// How a spell of an aim is pointed at a unit.
    fn aimed(&self, aim: Aim, at: Entity) -> Option<Target> {
        match aim {
            Aim::Unit => Some(Target::Unit(wire_id(at))),
            Aim::Point => self.pos(at).map(Target::Pos),
            _ => Some(Target::None),
        }
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
        self.entities
            .iter()
            .filter(|other| self.is_foe(caster, *other))
            .filter(|other| self.pos(*other).is_some_and(|pos| pos.within(at, radius)))
            .collect()
    }

    /// While it swings, whether enough foes within a radius are set on it.
    fn beset(&self, caster: Entity, at: Vec2, radius: i32, at_least: u8) -> Option<Target> {
        let swinging = matches!(
            self.action.get(caster).map(|action| action.state),
            Some(ActionState::Attack { .. })
        );
        let on_it = self
            .foes_within(caster, at, rules::units(radius))
            .into_iter()
            .filter(|foe| self.target_of(*foe) == Some(caster))
            .count();
        (swinging && on_it >= usize::from(at_least)).then_some(Target::None)
    }

    /// The foe a hurt neutral casts at: with the neutral below a share of its
    /// health, the nearest within a radius, or whoever it is set on for a
    /// radius of zero.
    fn hurt_foe(
        &self,
        caster: Entity,
        at: Vec2,
        below_pct: i32,
        radius: i32,
        held: Entity,
    ) -> Option<Entity> {
        let hp = i64::from(self.health.get(caster)?.hp.to_int());
        let most = i64::from(self.stats.get(caster)?.max_hp.to_int());
        if hp * 100 >= i64::from(below_pct) * most {
            return None;
        }
        if radius == 0 {
            return Some(held);
        }
        self.foes_within(caster, at, rules::units(radius))
            .into_iter()
            .min_by_key(|foe| {
                self.pos(*foe)
                    .map_or(i64::MAX, |pos| pos.distance_squared(at))
            })
    }

    /// The foe in reach with the least health under a bound, when the spell
    /// is cast at the units that attack its caster only, one of those.
    fn weak_foe(
        &self,
        caster: Entity,
        at: Vec2,
        range: i32,
        below_hp: i32,
        attacker: bool,
    ) -> Option<Target> {
        let weakest = self
            .foes_within(caster, at, rules::units(range))
            .into_iter()
            .filter(|foe| !attacker || self.target_of(*foe) == Some(caster))
            .filter_map(|foe| Some((self.health.get(foe)?.hp.to_int(), foe)))
            .filter(|(hp, _)| *hp < below_hp)
            .min_by_key(|(hp, _)| *hp)?;
        Some(Target::Unit(wire_id(weakest.1)))
    }

    /// The most hurt ally in reach below a share of its health, one that
    /// wears no bubble yet. The caster counts.
    fn hurt_ally(&self, caster: Entity, at: Vec2, range: i32, below_pct: i32) -> Option<Target> {
        let side = self.team.get(caster).copied()?;
        let reach = rules::units(range);
        let share = |ally: Entity| -> Option<i64> {
            let hp = i64::from(self.health.get(ally)?.hp.to_int());
            let most = i64::from(self.stats.get(ally)?.max_hp.to_int()).max(1);
            Some(hp * 100 / most)
        };
        let ally = self
            .entities
            .iter()
            .filter(|ally| {
                self.alive(*ally)
                    && self.team.get(*ally) == Some(&side)
                    && self.kind.get(*ally) == Some(&UnitKind::CreepNeutral)
                    && !self
                        .stats
                        .get(*ally)
                        .is_some_and(|stats| stats.invulnerable)
                    && self.pos(*ally).is_some_and(|pos| pos.within(at, reach))
                    && !self.wears_bubble(*ally)
            })
            .filter_map(|ally| Some((share(ally)?, ally)))
            .filter(|(share, _)| *share < i64::from(below_pct))
            .min_by_key(|(share, _)| *share)?;
        Some(Target::Unit(wire_id(ally.1)))
    }

    /// Whether a water bubble is up on an entity.
    fn wears_bubble(&self, entity: Entity) -> bool {
        self.modifiers.get(entity).is_some_and(|on_it| {
            on_it.active().any(|held| match held.kind {
                ModifierKind::Spell { ability, .. } => {
                    spell_effect(ability) == Some(WATER_BUBBLE_EFFECT)
                }
                _ => false,
            })
        })
    }

    /// The hero it is set on, in reach, while a hero of the other side
    /// stands within a radius of the neutral.
    fn duel(
        &self,
        caster: Entity,
        at: Vec2,
        range: i32,
        radius: i32,
        held: Entity,
    ) -> Option<Target> {
        let side = self.team.get(held).copied()?;
        if !self.is_hero(held) || !self.pos(held)?.within(at, rules::units(range)) {
            return None;
        }
        let rival = self.entities.iter().any(|other| {
            other != caster
                && self.is_hero(other)
                && self.alive(other)
                && self
                    .team
                    .get(other)
                    .is_some_and(|team| *team != side && *team != Team::Neutral)
                && self
                    .pos(other)
                    .is_some_and(|pos| pos.within(at, rules::units(radius)))
        });
        rival.then(|| Target::Unit(wire_id(held)))
    }

    /// The hero it is set on, in reach, when that hero is after a hero of
    /// the other side standing within a radius of the neutral.
    fn chaser(
        &self,
        caster: Entity,
        at: Vec2,
        range: i32,
        radius: i32,
        held: Entity,
    ) -> Option<Target> {
        let side = self.team.get(held).copied()?;
        if !self.is_hero(held) || !self.pos(held)?.within(at, rules::units(range)) {
            return None;
        }
        let chased = match self.orders.get(held).map(|orders| orders.current) {
            Some(UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }) => {
                Some(target)
            }
            _ => self.target_of(held),
        }?;
        let victim = chased != caster
            && self.is_hero(chased)
            && self.alive(chased)
            && self
                .team
                .get(chased)
                .is_some_and(|team| *team != side && *team != Team::Neutral)
            && self.pos(chased)?.within(at, rules::units(radius));
        victim.then(|| Target::Unit(wire_id(held)))
    }

    /// The nearest enemy hero within a radius wearing a buff a purge takes
    /// off.
    fn buffed_hero(&self, caster: Entity, at: Vec2, radius: i32) -> Option<Target> {
        let buffed = |hero: Entity| {
            self.modifiers.get(hero).is_some_and(|on_it| {
                on_it.active().any(|held| {
                    effect_def(held.kind.effect_id()).is_some_and(|def| def.dispellable)
                })
            })
        };
        let hero = self
            .foes_within(caster, at, rules::units(radius))
            .into_iter()
            .filter(|foe| self.is_hero(*foe) && buffed(*foe))
            .min_by_key(|foe| {
                self.pos(*foe)
                    .map_or(i64::MAX, |pos| pos.distance_squared(at))
            })?;
        Some(Target::Unit(wire_id(hero)))
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
                when: When::Avenge { kind, chance_pct },
                ..
            } = *ability.spell()
            else {
                return None;
            };
            (kind == fallen && self.castable(mate, state)).then_some((slot, chance_pct, ability))
        })
    }
}
