//! What a body does: the attack cycle, and the abilities and items it casts.
//!
//! A body does one thing at a time. Who to attack is put in [`Target`] by
//! acquisition, by a creep's own mind or by an order; what to cast is put in
//! `Orders.pending` by an order. Coming round to face either is movement's
//! business. A swing begins only once the target can be seen, is in reach,
//! and the attacker is looking near enough at it; a cast begins once what it
//! is aimed at is in reach.

use bota_proto::{DamageKind, Fixed, ItemSlot, Team, Vec2};

use crate::game::{
    Action, ActionPhase, ActionState, BEATS_PER_TICK, Chance, Entity, Hit, HitEffect, PendingCast,
    Projectile, Purpose, Ratio, Stats, Target, Visibility, World, ability, attack_gain, beats,
    cross, is_creep, is_structure, rules, wire_id,
};
use crate::game::{facing_gap, facing_towards};

/// What one swing deals, rolled when it lands.
#[derive(Clone, Copy)]
struct Swing {
    /// Its physical damage.
    damage: i32,
    /// Whether it is a critical strike.
    crit: bool,
    /// The magical damage piercing with it, when it pierces.
    pierce: Option<i32>,
    /// Physical amplification, in basis points.
    physical_amp_bp: i32,
    /// Magical amplification, in basis points.
    magical_amp_bp: i32,
}

/// How long an ability or an item holds the body: `point` and `backswing` in
/// milliseconds, `duration` in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    point: u32,
    backswing: u32,
    duration: u32,
}

impl Timing {
    /// Whether it holds the body at all.
    fn instant(self) -> bool {
        self.point == 0 && self.backswing == 0 && self.duration == 0
    }
}

impl World {
    /// Whether an entity swings at all: it acts, and a swing of its is worth
    /// something.
    pub fn can_attack(&self, entity: Entity) -> bool {
        self.action.contains(entity) && self.stats.get(entity).is_some_and(|stats| stats.damage > 0)
    }

    /// Whether an entity is standing through an ability or an item that
    /// runs on.
    pub fn is_channelling(&self, entity: Entity) -> bool {
        matches!(
            self.action.get(entity).map(|action| action.state),
            Some(
                ActionState::CastAbility {
                    phase: ActionPhase::During { .. },
                    ..
                } | ActionState::UseItem {
                    phase: ActionPhase::During { .. },
                    ..
                }
            )
        )
    }

    /// Points an entity at the cast it was ordered to make.
    pub fn order_cast(&mut self, entity: Entity, pending: PendingCast) {
        if let Some(orders) = self.orders.get_mut(entity) {
            orders.pending = Some(pending);
        }
    }

    /// The cast an entity has been ordered to make and has not begun.
    pub fn pending_cast(&self, entity: Entity) -> Option<PendingCast> {
        self.orders.get(entity).and_then(|orders| orders.pending)
    }

    /// Where a cast is aimed, if it is aimed anywhere at all.
    ///
    /// A cast at a unit is aimed where the unit stands while it stands;
    /// one at nothing in particular is aimed nowhere.
    pub fn cast_spot(&self, pending: PendingCast) -> Option<Vec2> {
        match pending.target() {
            bota_proto::Target::Pos(pos) => Some(pos),
            bota_proto::Target::Unit(target) => {
                let on = self.of_wire(target)?;
                self.alive(on)
                    .then(|| self.transform.get(on).map(|t| t.pos))
                    .flatten()
            }
            bota_proto::Target::None => None,
        }
    }

    /// How far a pending cast is walked into. Zero for one that is never
    /// walked into: an ability without a reach, an item at a spot, at a
    /// building or at its user.
    pub fn cast_reach(&self, entity: Entity, pending: PendingCast) -> i32 {
        match pending {
            PendingCast::Ability { slot, .. } => {
                crate::game::ability_def(self.ability_in(entity, slot)).map_or(0, |def| def.range)
            }
            PendingCast::Item { slot, .. } => self
                .item_def_in(entity, usize::from(slot.0))
                .filter(|def| {
                    matches!(def.aim, Some(bota_proto::Aim::Unit | bota_proto::Aim::Tree))
                })
                .map_or(0, |def| def.range),
        }
    }

    /// Whether an entity has a cast to make that it still has to walk in
    /// for: one aimed somewhere further off than the cast reaches.
    pub fn cast_out_of_reach(&self, entity: Entity) -> bool {
        let Some(pending) = self.pending_cast(entity) else {
            return false;
        };
        let reach = self.cast_reach(entity, pending);
        let (Some(spot), Some(at)) = (
            self.cast_spot(pending),
            self.transform.get(entity).map(|t| t.pos),
        ) else {
            return false;
        };
        reach > 0 && !at.within(spot, rules::units(reach))
    }

    /// Breaks off what the body is doing, the way an order does.
    ///
    /// A swing under way is left to the attack cycle, which gives it up on
    /// its own once the target is gone; the recovery after one ends here.
    /// An ability or item that runs on is broken off and told so.
    pub fn cancel_action(&mut self, entity: Entity) {
        let Some(action) = self.action.get(entity).copied() else {
            return;
        };
        let cleared = match action.state {
            ActionState::Ready
            | ActionState::Attack {
                phase: ActionPhase::Before { .. } | ActionPhase::During { .. },
                ..
            } => return,
            ActionState::Attack {
                phase: ActionPhase::After { .. },
                ..
            } => true,
            ActionState::CastAbility { phase, .. } | ActionState::UseItem { phase, .. } => {
                if let ActionPhase::During { .. } = phase
                    && let Some(what) = what_of(action.state)
                {
                    self.call_cancel(entity, what);
                }
                true
            }
        };
        if cleared && let Some(action) = self.action.get_mut(entity) {
            action.state = ActionState::Ready;
        }
    }

    /// Gives the body over to an ability or item: a swing under way is given
    /// up at no cost, a recovery is cut short.
    fn occupy(&mut self, entity: Entity, state: ActionState) {
        let Some(action) = self.action.get_mut(entity) else {
            return;
        };
        if let ActionState::Attack {
            phase: ActionPhase::Before { .. },
            ..
        } = action.state
        {
            action.attack_cooldown = 0;
        }
        action.state = state;
    }

    /// Runs every action one tick on.
    ///
    /// A pending cast goes off the moment its target is in reach; one that
    /// holds the body takes it over first. A swing that comes due leaves
    /// something behind: a blow where the target stands, or a missile on its
    /// way to it.
    pub fn run_actions(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            self.run_action(entity);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Runs one entity's action one tick on: the phase the tick began in is
    /// broken off or carried on, and what comes due after it is done.
    fn run_action(&mut self, entity: Entity) {
        let took = self.try_pending(entity);
        let Some(mut action) = self.action.get(entity).copied() else {
            return;
        };
        let Some(stats) = self.stats.get(entity).copied() else {
            return;
        };
        let gain = attack_gain(stats.attack_speed);
        let carry = if 0 < action.attack_cooldown && action.attack_cooldown <= gain {
            gain - action.attack_cooldown
        } else {
            0
        };
        action.attack_cooldown = action.attack_cooldown.saturating_sub(gain);
        let held = self.held(entity) || self.feared(entity);
        // A cast that took the body this tick has run for none of it.
        let cast_gain = if took { 0 } else { BEATS_PER_TICK };
        let broken = match action.state {
            ActionState::Ready => false,
            ActionState::Attack { target, phase } => {
                self.carry_swing_on(entity, &mut action, target, phase, &stats, held)
            }
            ActionState::CastAbility { phase, .. } | ActionState::UseItem { phase, .. } => {
                self.carry_cast_on(entity, &mut action, phase, cast_gain, held)
            }
        };
        if !broken {
            self.come_due(entity, &mut action, &stats, carry, held);
        }
        self.action.insert(entity, action);
    }

    /// Carries a swing one tick on, or breaks it off before it lands when
    /// the swinger is held, set on nobody, or no longer worth swinging at
    /// what it began on. A swing given up costs nothing. Answers whether it
    /// was broken off.
    fn carry_swing_on(
        &mut self,
        entity: Entity,
        action: &mut Action,
        target: Entity,
        phase: ActionPhase,
        stats: &Stats,
        held: bool,
    ) -> bool {
        let gain = attack_gain(stats.attack_speed);
        let phase = match phase {
            ActionPhase::Before { progress } => {
                if held
                    || self.target.get(entity).is_none()
                    || !self.still_worth_swinging_at(entity, target, stats.attack_range)
                {
                    action.state = ActionState::Ready;
                    action.attack_cooldown = 0;
                    return true;
                }
                ActionPhase::Before {
                    progress: progress + gain,
                }
            }
            ActionPhase::During { progress } => ActionPhase::During {
                progress: progress + gain,
            },
            ActionPhase::After { progress } => ActionPhase::After {
                progress: progress + gain,
            },
        };
        action.state = ActionState::Attack { target, phase };
        false
    }

    /// Carries a cast one tick on, or breaks it off: before it goes off when
    /// the caster is held, while it runs on when the caster is held or the
    /// ability says so, which the ability is told. Answers whether it was
    /// broken off.
    fn carry_cast_on(
        &mut self,
        entity: Entity,
        action: &mut Action,
        phase: ActionPhase,
        cast_gain: u32,
        held: bool,
    ) -> bool {
        let what = what_of(action.state).expect("a cast names what it casts");
        let phase = match phase {
            ActionPhase::Before { progress } => {
                if held {
                    action.state = ActionState::Ready;
                    return true;
                }
                ActionPhase::Before {
                    progress: progress + cast_gain,
                }
            }
            ActionPhase::During { progress } => {
                if held || !self.call_during(entity, what) {
                    self.call_cancel(entity, what);
                    action.state = ActionState::Ready;
                    return true;
                }
                ActionPhase::During {
                    progress: progress + cast_gain,
                }
            }
            ActionPhase::After { progress } => ActionPhase::After {
                progress: progress + cast_gain,
            },
        };
        action.state = with_phase(action.state, phase);
        false
    }

    /// Does whatever of an action has come due this tick: a swing begun
    /// when ready and set on something to swing at, a blow struck at the
    /// attack point, a cast gone off at its cast point and completed after
    /// its duration, and a recovery run out.
    fn come_due(
        &mut self,
        entity: Entity,
        action: &mut Action,
        stats: &Stats,
        mut carry: u32,
        held: bool,
    ) {
        let len = beats(stats.attack_time);
        loop {
            match action.state {
                ActionState::Ready => {
                    if action.attack_cooldown == 0
                        && !held
                        && stats.damage > 0
                        && let Some(Target(on)) = self.target.get(entity).copied()
                        && self.may_swing(entity, on, stats)
                    {
                        action.state = ActionState::Attack {
                            target: on,
                            phase: ActionPhase::Before { progress: carry },
                        };
                        action.attack_cooldown = len.saturating_sub(carry);
                        carry = carry.saturating_sub(len);
                        continue;
                    }
                    return;
                }
                ActionState::Attack { target, phase } => {
                    if !self.swing_due(entity, action, target, phase, stats) {
                        return;
                    }
                }
                ActionState::CastAbility { phase, .. } | ActionState::UseItem { phase, .. } => {
                    if !self.cast_due(entity, action, phase) {
                        return;
                    }
                }
            }
        }
    }

    /// Moves a swing on to its next phase if it has come due, striking at
    /// the attack point. Answers whether it moved on.
    fn swing_due(
        &mut self,
        entity: Entity,
        action: &mut Action,
        target: Entity,
        phase: ActionPhase,
        stats: &Stats,
    ) -> bool {
        match phase {
            ActionPhase::Before { progress } => {
                let Some(over) = cross(progress, beats(stats.attack_point)) else {
                    return false;
                };
                self.strike(entity, target, stats);
                action.state = ActionState::Attack {
                    target,
                    phase: ActionPhase::During { progress: over },
                };
            }
            ActionPhase::During { progress } => {
                action.state = ActionState::Attack {
                    target,
                    phase: ActionPhase::After { progress },
                };
            }
            ActionPhase::After { progress } => {
                if cross(progress, beats(stats.attack_backswing)).is_none() {
                    return false;
                }
                action.state = ActionState::Ready;
            }
        }
        true
    }

    /// Moves a cast on to its next phase if it has come due: going off at
    /// its cast point, completing after its duration, readying after its
    /// recovery. One that has no timing, or fails to go off, is given up.
    /// Answers whether it moved on.
    fn cast_due(&mut self, entity: Entity, action: &mut Action, phase: ActionPhase) -> bool {
        let what = what_of(action.state).expect("a cast names what it casts");
        let Some(timing) = self.timing_of(entity, what) else {
            action.state = ActionState::Ready;
            return true;
        };
        match phase {
            ActionPhase::Before { progress } => {
                let Some(over) = cross(progress, beats(timing.point)) else {
                    return false;
                };
                action.state = if self.begin(entity, what) {
                    with_phase(action.state, ActionPhase::During { progress: over })
                } else {
                    ActionState::Ready
                };
            }
            ActionPhase::During { progress } => {
                let Some(over) = cross(progress, timing.duration * BEATS_PER_TICK) else {
                    return false;
                };
                self.call_complete(entity, what);
                action.state = with_phase(action.state, ActionPhase::After { progress: over });
            }
            ActionPhase::After { progress } => {
                if cross(progress, beats(timing.backswing)).is_none() {
                    return false;
                }
                action.state = ActionState::Ready;
            }
        }
        true
    }

    /// Starts the cast an entity was ordered to make, once it is in reach.
    ///
    /// One that holds the body for no time at all goes off from any state
    /// and leaves the body to what it was doing; one that does takes the
    /// body over, giving up a swing under way, and waits while an earlier
    /// cast still holds it. A cast that cannot be made is dropped; one whose
    /// caster is held waits.
    ///
    /// True when the body was taken over this tick.
    fn try_pending(&mut self, entity: Entity) -> bool {
        let Some(pending) = self.pending_cast(entity) else {
            return false;
        };
        let spot = self.cast_spot(pending);
        if let bota_proto::Target::Unit(_) = pending.target()
            && spot.is_none()
        {
            self.order_cast_off(entity);
            return false;
        }
        if self.cast_out_of_reach(entity) || self.held(entity) || self.feared(entity) {
            return false;
        }
        let Some(timing) = self.timing_of(entity, pending) else {
            self.order_cast_off(entity);
            return false;
        };
        if timing.instant() {
            self.order_cast_off(entity);
            self.begin(entity, pending);
            return false;
        }
        if matches!(
            self.action.get(entity).map(|action| action.state),
            Some(ActionState::CastAbility { .. } | ActionState::UseItem { .. })
        ) {
            return false;
        }
        self.order_cast_off(entity);
        self.occupy(
            entity,
            with_phase(state_of(pending), ActionPhase::Before { progress: 0 }),
        );
        true
    }

    /// Takes away the cast an entity was ordered to make.
    fn order_cast_off(&mut self, entity: Entity) {
        if let Some(orders) = self.orders.get_mut(entity) {
            orders.pending = None;
        }
    }

    /// The moment a cast goes off: the ability's or item's own work, and its
    /// cost. False when it did not go off, and then nothing was spent.
    fn begin(&mut self, entity: Entity, what: PendingCast) -> bool {
        match what {
            PendingCast::Ability { slot, target } => self.begin_ability(entity, slot, target),
            PendingCast::Item { slot, target } => {
                self.begin_item(entity, usize::from(slot.0), target)
            }
        }
    }

    /// Uses an item now, outside the order flow: it goes off at once, and one
    /// that runs on takes the body over.
    pub fn use_item(
        &mut self,
        entity: Entity,
        slot: usize,
        target: bota_proto::Target,
        events: &mut Vec<crate::game::Event>,
    ) -> bool {
        let duration = self.item_def_in(entity, slot).map_or(0, |def| def.duration);
        let went = self.begin_item(entity, slot, target);
        if went && duration > 0 {
            self.occupy(
                entity,
                ActionState::UseItem {
                    target,
                    slot: ItemSlot(slot as u8),
                    phase: ActionPhase::During { progress: 0 },
                },
            );
        }
        events.append(&mut self.events);
        went
    }

    /// How long a cast holds the body. Absent when the slot holds nothing.
    fn timing_of(&self, entity: Entity, what: PendingCast) -> Option<Timing> {
        match what {
            PendingCast::Ability { slot, .. } => {
                let def = crate::game::ability_def(self.ability_in(entity, slot))?;
                Some(Timing {
                    point: def.point,
                    backswing: def.backswing,
                    duration: def.duration,
                })
            }
            PendingCast::Item { slot, .. } => {
                let def = self.item_def_in(entity, usize::from(slot.0))?;
                Some(Timing {
                    point: def.point,
                    backswing: def.backswing,
                    duration: def.duration,
                })
            }
        }
    }

    /// Whether what runs on still has something to do.
    fn call_during(&mut self, entity: Entity, what: PendingCast) -> bool {
        match what {
            PendingCast::Ability { slot, target } => {
                let Some(def) = crate::game::ability_def(self.ability_in(entity, slot)) else {
                    return false;
                };
                (def.on_during)(self, entity, target)
            }
            PendingCast::Item { slot, target } => {
                let at = usize::from(slot.0);
                let Some(def) = self.item_def_in(entity, at) else {
                    return false;
                };
                (def.on_during)(self, entity, at, target)
            }
        }
    }

    /// Tells what runs on that it was broken off.
    fn call_cancel(&mut self, entity: Entity, what: PendingCast) {
        match what {
            PendingCast::Ability { slot, target } => {
                if let Some(def) = crate::game::ability_def(self.ability_in(entity, slot)) {
                    (def.on_cancel)(self, entity, target);
                }
            }
            PendingCast::Item { slot, target } => {
                let at = usize::from(slot.0);
                if let Some(def) = self.item_def_in(entity, at) {
                    (def.on_cancel)(self, entity, at, target);
                }
            }
        }
    }

    /// Tells what ran on that it ran to the end.
    fn call_complete(&mut self, entity: Entity, what: PendingCast) {
        match what {
            PendingCast::Ability { slot, target } => {
                if let Some(def) = crate::game::ability_def(self.ability_in(entity, slot)) {
                    (def.on_complete)(self, entity, target);
                }
            }
            PendingCast::Item { slot, target } => {
                let at = usize::from(slot.0);
                if let Some(def) = self.item_def_in(entity, at) {
                    (def.on_complete)(self, entity, at, target);
                }
            }
        }
    }

    /// Leaves what a swing that came due turns into.
    fn strike(&mut self, attacker: Entity, on: Entity, stats: &Stats) {
        // What is carried against creeps is worth nothing against anything else.
        let damage = stats.damage
            + if self.kind.get(on).copied().is_some_and(is_creep) {
                stats.damage_to_creeps
            } else {
                0
            };
        let (damage, crit) = match self.roll_crit(attacker) {
            Some(pct) => (damage * pct / 100, true),
            None => (damage, false),
        };
        let swing = Swing {
            damage,
            crit,
            pierce: self.roll_pierce(attacker, on, stats),
            physical_amp_bp: stats.damage_amp_bp(DamageKind::Physical),
            magical_amp_bp: stats.damage_amp_bp(DamageKind::Magical),
        };
        match stats.projectile_speed {
            None => self.strike_home(attacker, on, swing),
            Some(speed) => self.throw_at(attacker, on, swing, speed, !stats.flies),
        }
    }

    /// A melee swing's blows, laid where the target stands: the swing
    /// itself, and the magical pierce riding with it.
    fn strike_home(&mut self, attacker: Entity, on: Entity, swing: Swing) {
        self.hits.push_back(Hit {
            source: Some(attacker),
            target: on,
            amount: swing.damage,
            kind: DamageKind::Physical,
            damage_amp_bp: swing.physical_amp_bp,
            crit: swing.crit,
            attack: true,
            pierces: swing.pierce.is_some(),
            effect: HitEffect::None,
        });
        if let Some(bonus) = swing.pierce {
            self.hits.push_back(Hit {
                source: Some(attacker),
                target: on,
                amount: bonus,
                kind: DamageKind::Magical,
                damage_amp_bp: swing.magical_amp_bp,
                crit: false,
                attack: false,
                pierces: false,
                effect: HitEffect::None,
            });
        }
    }

    /// A ranged swing's missile, thrown from where the attacker stands and
    /// seen by its side from the throw.
    fn throw_at(
        &mut self,
        attacker: Entity,
        on: Entity,
        swing: Swing,
        speed: Fixed,
        can_miss_uphill: bool,
    ) {
        let Some(at) = self.transform.get(attacker).copied() else {
            return;
        };
        let side = self.team.get(attacker).copied().unwrap_or(Team::Neutral);
        let missile = self.spawn();
        self.transform.insert(missile, at);
        self.team.insert(missile, side);
        let mut seen = Visibility::NONE;
        seen.add(side);
        self.visibility.insert(missile, seen);
        self.projectile.insert(
            missile,
            Projectile {
                speed,
                source: Some(attacker),
                target: on,
                damage: swing.damage,
                kind: DamageKind::Physical,
                damage_amp_bp: swing.physical_amp_bp,
                ability: None,
                launch_tier: self.ground.tier(at.pos),
                can_miss_uphill,
                crit: swing.crit,
                pierces: swing.pierce.is_some(),
                pierce_damage: swing.pierce.unwrap_or(0),
                pierce_amp_bp: swing.magical_amp_bp,
                bounces_left: 0,
                bounce_range: 0,
                bounced: Vec::new(),
            },
        );
    }

    /// Whether a swing is a critical strike, and then what it is worth as a
    /// percentage of a plain one. Absent for a swing that is not one.
    ///
    /// Only an attacker that has learned the crit ever rolls; the rate is
    /// the learned level's and is held exactly over every block of rolls.
    fn roll_crit(&mut self, attacker: Entity) -> Option<i32> {
        let level = self.carried_level(attacker, ability::CRIT);
        if level == 0 {
            return None;
        }
        let at = usize::from(level - 1);
        let ratio = rules::SYLLA_CRIT_CHANCE[at];
        let index = attacker.index().0 as usize;
        if self.crit.len() <= index {
            self.crit.resize_with(index + 1, || None);
        }
        if self.crit[index].is_none() {
            let stream = self.rng.for_unit(Purpose::Crit, wire_id(attacker), 0);
            self.crit[index] = Some(Chance::new(stream, ratio));
        }
        let chance = self.crit[index].as_mut().expect("opened above");
        chance.roll(ratio).then_some(rules::SYLLA_CRIT_MULT_PCT[at])
    }

    /// Whether a swing pierces, and then the magical damage it lands
    /// alongside. Absent for a swing that does not.
    ///
    /// Only an attacker carrying a pierce ever rolls, and a structure is
    /// never pierced; the share is held exactly over every block of rolls.
    fn roll_pierce(&mut self, attacker: Entity, on: Entity, stats: &Stats) -> Option<i32> {
        if stats.pierce == Ratio::NEVER || self.kind.get(on).copied().is_some_and(is_structure) {
            return None;
        }
        let index = attacker.index().0 as usize;
        if self.pierce.len() <= index {
            self.pierce.resize_with(index + 1, || None);
        }
        if self.pierce[index].is_none() {
            let stream = self.rng.for_unit(Purpose::Pierce, wire_id(attacker), 0);
            self.pierce[index] = Some(Chance::new(stream, stats.pierce));
        }
        let chance = self.pierce[index].as_mut().expect("opened above");
        chance.roll(stats.pierce).then_some(stats.pierce_damage)
    }

    /// Whether a swing may begin: the target is standing, seen, in reach, and
    /// looked at.
    fn may_swing(&self, attacker: Entity, on: Entity, stats: &Stats) -> bool {
        if !self.worth_swinging_at(attacker, on, stats.attack_range) {
            return false;
        }
        let (Some(from), Some(at)) = (self.transform.get(attacker), self.transform.get(on)) else {
            return false;
        };
        // Looked at, within the angle a swing allows. Only the start waits on
        // this: once under way the attacker keeps turning with its target.
        let wanted = facing_towards(from.pos, at.pos);
        facing_gap(from.facing, wanted) <= rules::ATTACK_ANGLE_BRADS
    }

    /// Whether a swing already under way is still worth finishing.
    ///
    /// The same three things a swing begins on, less the angle, and with the
    /// leeway a started swing carries: the target has to leave by more than
    /// that to shake it off.
    fn still_worth_swinging_at(&self, attacker: Entity, on: Entity, reach: Fixed) -> bool {
        self.worth_swinging_at(
            attacker,
            on,
            reach + rules::units(rules::ATTACK_RANGE_LEEWAY),
        )
    }

    /// Whether the target is standing, seen by the attacker's side, and
    /// within `reach` edge to edge.
    fn worth_swinging_at(&self, attacker: Entity, on: Entity, reach: Fixed) -> bool {
        let standing = self
            .health
            .get(on)
            .is_some_and(|health| health.hp > Fixed::ZERO);
        if !standing {
            return false;
        }
        let Some(side) = self.team.get(attacker).copied() else {
            return false;
        };
        if !self.visibility.get(on).is_some_and(|seen| seen.by(side)) {
            return false;
        }
        let (Some(from), Some(at)) = (self.transform.get(attacker), self.transform.get(on)) else {
            return false;
        };
        let hulls = self.hull.get(attacker).map_or(Fixed::ZERO, |h| h.bound)
            + self.hull.get(on).map_or(Fixed::ZERO, |h| h.bound);
        from.pos.within(at.pos, reach + hulls)
    }
}

/// What a cast in progress is casting.
fn what_of(state: ActionState) -> Option<PendingCast> {
    match state {
        ActionState::CastAbility { target, slot, .. } => {
            Some(PendingCast::Ability { slot, target })
        }
        ActionState::UseItem { target, slot, .. } => Some(PendingCast::Item { slot, target }),
        ActionState::Ready | ActionState::Attack { .. } => None,
    }
}

/// The state a pending cast takes the body into, at the start of its cast
/// point.
fn state_of(pending: PendingCast) -> ActionState {
    match pending {
        PendingCast::Ability { slot, target } => ActionState::CastAbility {
            target,
            slot,
            phase: ActionPhase::Before { progress: 0 },
        },
        PendingCast::Item { slot, target } => ActionState::UseItem {
            target,
            slot,
            phase: ActionPhase::Before { progress: 0 },
        },
    }
}

/// The same action in another phase.
fn with_phase(state: ActionState, phase: ActionPhase) -> ActionState {
    match state {
        ActionState::Attack { target, .. } => ActionState::Attack { target, phase },
        ActionState::CastAbility { target, slot, .. } => ActionState::CastAbility {
            target,
            slot,
            phase,
        },
        ActionState::UseItem { target, slot, .. } => ActionState::UseItem {
            target,
            slot,
            phase,
        },
        ActionState::Ready => ActionState::Ready,
    }
}
