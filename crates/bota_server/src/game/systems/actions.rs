//! What a body does: the attack cycle, and the abilities and items it casts.
//!
//! A body does one thing at a time. Who to attack is put in [`Target`] by
//! acquisition, by a creep's own mind or by an order; what to cast is put in
//! `Orders.pending` by an order. Coming round to face either is movement's
//! business. A swing begins only once the target can be seen, is in reach,
//! and the attacker is looking near enough at it; a cast begins once what it
//! is aimed at is in reach.

use bota_core::{
    BEATS_PER_TICK, ModifierKind, Striker, Talent, ability_def, attack_gain, beats,
    carried_pierces, cast_point_ms, cross, facing_gap, facing_towards, kind_def, rules,
};
use bota_proto::{Aim, DamageKind, Fixed, Inflictor, ItemId, ItemSlot, Team, Vec2};

use super::hitting::entity_slot;

use crate::game::{
    Action, ActionPhase, ActionState, Entity, Event, Hit, HitEffect, PendingCast, Projectile,
    PseudoRandom, Purpose, Rolls, Stats, Target, World, ability_hooks, wire_id,
};

/// What one swing deals, rolled when it lands.
#[derive(Clone)]
struct Swing {
    /// Who swings it, for health counted in hits.
    striker: Striker,
    /// Its physical damage.
    damage: i32,
    /// Whether it goes through evasion for an accuracy aura's sake.
    accurate: bool,
    /// The magical damage each pierce that landed adds, and the item that
    /// pierced. Empty for none.
    pierced: Vec<(i32, ItemId)>,
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
        matches!(self.cast_phase(entity), Some(ActionPhase::During { .. }))
    }

    /// Whether an entity stands in the cast point of an ability or an item.
    pub fn is_casting(&self, entity: Entity) -> bool {
        matches!(self.cast_phase(entity), Some(ActionPhase::Before { .. }))
    }

    /// Where the ability or item an entity is casting stands, if any.
    fn cast_phase(&self, entity: Entity) -> Option<ActionPhase> {
        match self.action.get(entity)?.state {
            ActionState::CastAbility { phase, .. } | ActionState::UseItem { phase, .. } => {
                Some(phase)
            }
            ActionState::Ready | ActionState::Attack { .. } => None,
        }
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
                self.alive(on).then(|| self.pos(on)).flatten()
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
                ability_def(self.ability_in(entity, slot)).map_or(0, |def| def.range)
            }
            PendingCast::Item { slot, .. } => self
                .item_def_in(entity, usize::from(slot.0))
                .filter(|def| matches!(def.aim, Some(Aim::Unit | Aim::Tree)))
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
        let (Some(spot), Some(at)) = (self.cast_spot(pending), self.pos(entity)) else {
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
        match action.state {
            ActionState::Ready
            | ActionState::Attack {
                phase: ActionPhase::Before { .. } | ActionPhase::During { .. },
                ..
            } => return,
            ActionState::Attack {
                phase: ActionPhase::After { .. },
                ..
            } => {}
            ActionState::CastAbility { phase, .. } | ActionState::UseItem { phase, .. } => {
                if let ActionPhase::During { .. } = phase
                    && let Some(what) = what_of(action.state)
                {
                    self.call_cancel(entity, what);
                }
            }
        }
        if let Some(action) = self.action.get_mut(entity) {
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
        self.try_immediate(entity);
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
        action.state = cast_state(what, phase);
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
                        && !self.disarmed(entity)
                        && stats.damage > 0
                        && let Some(Target(on)) = self.target.get(entity).copied()
                        && self.may_swing(entity, on, stats)
                    {
                        action.state = ActionState::Attack {
                            target: on,
                            phase: ActionPhase::Before { progress: carry },
                        };
                        self.rouse_by_swing(entity, on);
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
                    cast_state(what, ActionPhase::During { progress: over })
                } else {
                    ActionState::Ready
                };
            }
            ActionPhase::During { progress } => {
                let Some(over) = cross(progress, timing.duration * BEATS_PER_TICK) else {
                    return false;
                };
                self.call_complete(entity, what);
                action.state = cast_state(what, ActionPhase::After { progress: over });
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
            cast_state(pending, ActionPhase::Before { progress: 0 }),
        );
        true
    }

    /// Sets off the immediate cast an entity was ordered to make this tick,
    /// beside whatever it is doing. One whose caster is held or feared is
    /// dropped.
    fn try_immediate(&mut self, entity: Entity) {
        let Some(immediate) = self
            .orders
            .get_mut(entity)
            .and_then(|orders| orders.immediate.take())
        else {
            return;
        };
        if !self.held(entity) && !self.feared(entity) {
            self.begin(entity, immediate);
        }
    }

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
        events: &mut Vec<Event>,
    ) -> bool {
        let duration = self.item_def_in(entity, slot).map_or(0, |def| def.duration);
        let went = self.begin_item(entity, slot, target);
        if went && duration > 0 {
            let what = PendingCast::Item {
                slot: ItemSlot(slot as u8),
                target,
            };
            self.occupy(
                entity,
                cast_state(what, ActionPhase::During { progress: 0 }),
            );
        }
        events.append(&mut self.events);
        went
    }

    /// How long a cast holds the body. Absent when the slot holds nothing.
    fn timing_of(&self, entity: Entity, what: PendingCast) -> Option<Timing> {
        let (point, backswing, duration) = match what {
            PendingCast::Ability { slot, .. } => {
                let def = ability_def(self.ability_in(entity, slot))?;
                (def.point, def.backswing, def.duration)
            }
            PendingCast::Item { slot, .. } => {
                let def = self.item_def_in(entity, usize::from(slot.0))?;
                (def.point, def.backswing, def.duration)
            }
        };
        Some(Timing {
            point: self.cast_point_of(entity, point),
            backswing,
            duration,
        })
    }

    /// Milliseconds a cast point lasts for an entity: its own length, or
    /// [`rules::TALENT_FEAST_CAST_POINT_PCT`] of it while a feast runs on one
    /// that has learned the level-25 talent. Read every tick of the point, so
    /// a feast ending inside it lengthens what is left.
    pub(crate) fn cast_point_of(&self, entity: Entity, point: u32) -> u32 {
        let feasting = self.modifiers.get(entity).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| matches!(held.kind, ModifierKind::Feasting { .. }))
        });
        cast_point_ms(
            point,
            feasting && self.has_talent(entity, Talent::FeastCastSpeed),
        )
    }

    /// Whether what runs on still has something to do.
    fn call_during(&mut self, entity: Entity, what: PendingCast) -> bool {
        match what {
            PendingCast::Ability { slot, target } => {
                let Some(hooks) = ability_hooks(self.ability_in(entity, slot)) else {
                    return false;
                };
                (hooks.on_during)(self, entity, target)
            }
            PendingCast::Item { slot, .. } => {
                let at = usize::from(slot.0);
                let Some(def) = self.item_def_in(entity, at) else {
                    return false;
                };
                self.use_goes_on(entity, def)
            }
        }
    }

    /// Tells what runs on that it was broken off.
    fn call_cancel(&mut self, entity: Entity, what: PendingCast) {
        match what {
            PendingCast::Ability { slot, target } => {
                if let Some(hooks) = ability_hooks(self.ability_in(entity, slot)) {
                    (hooks.on_cancel)(self, entity, target);
                }
            }
            PendingCast::Item { slot, .. } => {
                let at = usize::from(slot.0);
                if let Some(def) = self.item_def_in(entity, at) {
                    self.use_breaks_off(entity, def);
                }
            }
        }
    }

    /// Tells what ran on that it ran to the end.
    fn call_complete(&mut self, entity: Entity, what: PendingCast) {
        match what {
            PendingCast::Ability { slot, target } => {
                if let Some(hooks) = ability_hooks(self.ability_in(entity, slot)) {
                    (hooks.on_complete)(self, entity, target);
                }
            }
            PendingCast::Item { slot, .. } => {
                let at = usize::from(slot.0);
                if let Some(def) = self.item_def_in(entity, at) {
                    self.use_completes(entity, at, def);
                }
            }
        }
    }

    /// Leaves what a swing that came due turns into.
    fn strike(&mut self, attacker: Entity, on: Entity, stats: &Stats) {
        self.expose(attacker, on);
        // What is carried against creeps is worth nothing against anything else.
        let damage = stats.damage
            + self.roll_damage(stats.damage_spread)
            + if self
                .kind
                .get(on)
                .copied()
                .is_some_and(|kind| kind_def(kind).creep)
            {
                stats.damage_to_creeps
            } else {
                0
            };
        let class_pct = self.class_pct_of(attacker, on);
        let boost = self.boost_of(attacker, stats);
        let striker = self.kind.get(attacker).map_or(Striker::Other, |kind| {
            Striker::of(*kind, stats.projectile_speed.is_some())
        });
        let swing = Swing {
            striker,
            damage: damage * boost.crit_pct / 100 * class_pct / 100,
            accurate: boost.accurate,
            pierced: self.roll_pierces(attacker, on, stats, class_pct),
            physical_amp_bp: stats.damage_amp_bp(DamageKind::Physical),
            magical_amp_bp: stats.damage_amp_bp(DamageKind::Magical),
        };
        match stats.projectile_speed {
            None => self.strike_home(attacker, on, swing),
            Some(speed) => {
                self.throw_at(attacker, on, swing.clone(), speed);
                for extra in self.multishot_targets(attacker, on, stats) {
                    let share = self.class_pct_on(attacker, extra);
                    let shot = Swing {
                        damage: damage * share / 100,
                        ..swing.clone()
                    };
                    self.throw_at(attacker, extra, shot, speed);
                }
            }
        }
    }

    /// A melee swing's blows, laid where the target stands: the swing
    /// itself, and the magical damage of each pierce riding with it.
    fn strike_home(&mut self, attacker: Entity, on: Entity, swing: Swing) {
        self.hits.push_back(Hit {
            source: Some(attacker),
            inflictor: None,
            target: on,
            amount: swing.damage,
            kind: DamageKind::Physical,
            damage_amp_bp: swing.physical_amp_bp,
            attack: true,
            striker: Some(swing.striker),
            pierces: !swing.pierced.is_empty() || swing.accurate,
            uphill: false,
            effect: HitEffect::None,
        });
        for (bonus, item) in swing.pierced {
            self.hits.push_back(Hit {
                source: Some(attacker),
                inflictor: Some(Inflictor::Item(item)),
                target: on,
                amount: bonus,
                kind: DamageKind::Magical,
                damage_amp_bp: swing.magical_amp_bp,
                attack: false,
                striker: None,
                pierces: false,
                uphill: false,
                effect: HitEffect::None,
            });
        }
    }

    /// A ranged swing's missile, thrown from where the attacker stands and
    /// seen by its side from the throw.
    fn throw_at(&mut self, attacker: Entity, on: Entity, swing: Swing, speed: Fixed) {
        let Some(at) = self.transform.get(attacker).copied() else {
            return;
        };
        let side = self.team.get(attacker).copied().unwrap_or(Team::Neutral);
        let missile = self.spawn();
        self.transform.insert(missile, at);
        self.set_team(missile, side);
        self.projectile.insert(
            missile,
            Projectile {
                speed,
                source: Some(attacker),
                inflictor: None,
                target: on,
                damage: swing.damage,
                kind: DamageKind::Physical,
                damage_amp_bp: swing.physical_amp_bp,
                launch_tier: self.ground.tier(at.pos),
                can_miss_uphill: true,
                pierces: !swing.pierced.is_empty() || swing.accurate,
                pierced: swing.pierced,
                pierce_amp_bp: swing.magical_amp_bp,
                effect: HitEffect::None,
                striker: Some(swing.striker),
            },
        );
    }

    /// What a swing adds to its least damage: a whole number drawn evenly
    /// from `0..=spread` on the match's damage stream. A spread of zero draws
    /// nothing.
    pub(crate) fn roll_damage(&mut self, spread: i32) -> i32 {
        if spread <= 0 {
            return 0;
        }
        let span = u32::try_from(spread + 1).expect("a damage spread fits a u32");
        self.rng.global(Purpose::DamageRoll).below(span) as i32
    }

    /// The pierces that land on a swing at `on`, each with the magical
    /// damage it adds by the swing's damage class and the item that pierced:
    /// every pierce the attacker carries ([`carried_pierces`]) rolls on a
    /// pseudo-random sequence of its own, its item's slot's, and a
    /// structure is never pierced.
    fn roll_pierces(
        &mut self,
        attacker: Entity,
        on: Entity,
        stats: &Stats,
        class_pct: i32,
    ) -> Vec<(i32, ItemId)> {
        let structure = self
            .kind
            .get(on)
            .copied()
            .is_some_and(|kind| kind_def(kind).structure);
        let Some(bag) = self.inventory.get(attacker).filter(|_| !structure) else {
            return Vec::new();
        };
        let carried: Vec<(usize, ItemId)> = bag
            .slots
            .iter()
            .take(rules::INVENTORY_SLOTS)
            .enumerate()
            .filter_map(|(at, held)| {
                held.filter(|stack| stack.mute == 0)
                    .map(|stack| (at, stack.id))
            })
            .collect();
        let ranged = stats.projectile_speed.is_some();
        let mut landed = Vec::new();
        for (at, pierce) in carried_pierces(carried, ranged) {
            let rng = &self.rng;
            let rolls = entity_slot(&mut self.pierce, attacker, Rolls::default);
            let source = rolls.at(at, || {
                PseudoRandom::new(rng.for_unit(Purpose::Pierce, wire_id(attacker), at as u8))
            });
            if source.roll(pierce.chance) {
                landed.push((pierce.damage * class_pct / 100, pierce.item));
            }
        }
        landed
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
        self.health
            .get(on)
            .is_some_and(|health| health.hp > Fixed::ZERO)
            && self.can_see_of(attacker, on)
            && self.within_reach(attacker, on, reach)
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

/// The state a cast holds the body in, at a phase.
fn cast_state(what: PendingCast, phase: ActionPhase) -> ActionState {
    match what {
        PendingCast::Ability { slot, target } => ActionState::CastAbility {
            target,
            slot,
            phase,
        },
        PendingCast::Item { slot, target } => ActionState::UseItem {
            target,
            slot,
            phase,
        },
    }
}
