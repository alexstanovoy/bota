//! The map's objectives: standing them up, an order at one turned into the
//! hidden channel that works it, the Outposts captured and the Watchers lit
//! and put out.

use bota_core::{
    AbilityDef, CAPTURE_TICKS, LAMP_LIT_TICKS, LAMP_OFF_TICKS, ModifierKind,
    OBJECTIVES_OPEN_AFTER_HORN, OUTPOST, SHRINE, SHRINE_COUNTDOWN_TICKS, SHRINE_RADIUS, WATCHER,
    WATCHER_DAY_VISION, WATCHER_NIGHT_VISION, ability_def, kind_def, objective_unit, rules,
};
use bota_proto::{
    AbilityId, AbilitySlot, EntityId, EventKind, Inflictor, ObjectiveKind, ObjectiveSet, Target,
    Team, UnitKind,
};

use crate::game::{
    ActionPhase, ActionState, Capture, Entity, Event, EventVisibility, Modifier, Outpost, World,
    Zone, wire_id,
};

impl World {
    /// Stands up the objectives the match's rules ask for, at the places
    /// the map holds: each Outpost with its home side, each Watcher and each
    /// Shrine of the neutral team.
    pub(crate) fn stand_up_objectives(&mut self, objectives: ObjectiveSet) {
        self.objectives = objectives;
        let map = self.map;
        if objectives.has(ObjectiveKind::Outposts) {
            for outpost in map.outposts {
                let entity = self.spawn_objective(&OUTPOST, outpost.home, outpost.at);
                self.outpost.insert(entity, Outpost { home: outpost.home });
            }
        }
        if objectives.has(ObjectiveKind::Watchers) {
            for at in map.watchers {
                self.spawn_objective(&WATCHER, Team::Neutral, *at);
            }
        }
        if objectives.has(ObjectiveKind::Shrines) {
            for at in map.shrines {
                let entity = self.spawn_objective(&SHRINE, Team::Neutral, *at);
                self.zone.insert(
                    entity,
                    Zone {
                        radius: rules::units(SHRINE_RADIUS),
                        ticks: SHRINE_COUNTDOWN_TICKS,
                        side: None,
                        progress: 0,
                    },
                );
            }
        }
        if !objectives.is_empty() {
            self.lay_passability();
        }
    }

    /// The hidden abilities the match's objectives call for, which every
    /// hero holds: what an order at each kind it stands up casts.
    pub(crate) fn interactions(&self) -> impl Iterator<Item = AbilityId> + '_ {
        self.objectives
            .iter()
            .filter_map(objective_unit)
            .filter_map(|kind| kind_def(kind).interact)
    }

    /// Whether an ability may be aimed at a unit by its filter: the unit's
    /// kind, and its side against the caster's.
    pub(crate) fn aimable(&self, caster: Entity, def: &AbilityDef, mark: Entity) -> bool {
        let Some(kind) = self.kind.get(mark).copied() else {
            return def.targets.any_kind();
        };
        let own = self.team.get(caster).is_some() && self.team.get(caster) == self.team.get(mark);
        def.targets.takes(kind, own)
    }

    /// The slot of the hidden ability an order at a unit casts instead of
    /// attacking or following it: the one its kind names, held by the unit
    /// ordered and aimable at it. Nothing for any other unit.
    pub(crate) fn interaction_at(&self, unit: Entity, target: EntityId) -> Option<AbilitySlot> {
        let mark = self.of_wire(target)?;
        let id = kind_def(*self.kind.get(mark)?).interact?;
        let at = self
            .abilities
            .get(unit)?
            .slots
            .iter()
            .position(|held| held.id == id)?;
        let def = ability_def(id)?;
        self.aimable(unit, def, mark)
            .then(|| AbilitySlot(u8::try_from(at).unwrap_or(u8::MAX)))
    }

    /// Hands a unit over to another side, as Dota's `SetTeam`: every aim at
    /// it is called off, the match setup's modifiers are put on it afresh
    /// for its new side, and both sides are told who holds it now, the hero
    /// that took it only to its own.
    pub(crate) fn change_team(&mut self, entity: Entity, side: Team, by: Option<Entity>) {
        self.set_team(entity, side);
        self.call_off_aims(entity, |_| true);
        self.apply_spawn_modifiers(entity);
        self.events.push(Event {
            kind: EventKind::Captured {
                unit: wire_id(entity),
                team: side,
                by: by.map(wire_id),
            },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::OneTeam(side),
        });
    }

    /// The unit a hidden channel is aimed at, while it stands.
    fn channel_mark(&self, target: Target) -> Option<Entity> {
        let Target::Unit(id) = target else {
            return None;
        };
        self.of_wire(id).filter(|mark| self.alive(*mark))
    }

    /// Whether a hero's side may start or go on capturing an Outpost: one of
    /// the other side, unguarded, and captured by nobody else.
    fn may_capture(&self, caster: Entity, outpost: Entity) -> bool {
        let Some(side) = self.team.get(caster).copied() else {
            return false;
        };
        self.clock() >= self.schedule().horn + OBJECTIVES_OPEN_AFTER_HORN
            && self.outpost.get(outpost).is_some()
            && self.team.get(outpost) != Some(&side)
            && !self.wears(outpost, ModifierKind::CaptureGuard)
            && self
                .capture
                .get(outpost)
                .is_none_or(|held| held.team == side)
    }

    /// `ability_capture` goes off: the channel begins, unless it is before
    /// the horn or the Outpost is the caster's side's own, guarded, or being
    /// captured by the other side ("Outpost already being captured by enemy
    /// team.").
    pub(crate) fn capture_begins(&mut self, caster: Entity, target: Target) -> bool {
        let Some(outpost) = self.channel_mark(target) else {
            return false;
        };
        if !self.may_capture(caster, outpost) {
            return false;
        }
        let Some(side) = self.team.get(caster).copied() else {
            return false;
        };
        if self.capture.get(outpost).is_none() {
            self.capture.insert(
                outpost,
                Capture {
                    team: side,
                    progress: 0,
                },
            );
        }
        true
    }

    /// One tick of a capture's channel: a tick more towards it, and the
    /// Outpost taken once its side has channelled [`CAPTURE_TICKS`] in all.
    /// It ends when the Outpost is taken or may no longer be.
    pub(crate) fn capture_goes_on(&mut self, caster: Entity, target: Target) -> bool {
        let Some(outpost) = self.channel_mark(target) else {
            return false;
        };
        if !self.may_capture(caster, outpost) {
            return false;
        }
        let Some(held) = self.capture.get_mut(outpost) else {
            return false;
        };
        held.progress += 1;
        if held.progress < CAPTURE_TICKS {
            return true;
        }
        self.capture_completes(caster, target);
        false
    }

    /// A capture's channel ran its whole length: the Outpost is its side's.
    pub(crate) fn capture_completes(&mut self, caster: Entity, target: Target) {
        let (Some(outpost), Some(side)) =
            (self.channel_mark(target), self.team.get(caster).copied())
        else {
            return;
        };
        if !self.may_capture(caster, outpost) {
            return;
        }
        self.capture.remove(outpost);
        self.change_team(outpost, side, Some(caster));
    }

    /// A capture's channel was broken off: it does nothing, and what its
    /// side gathered is lost once none of its heroes channels on.
    pub(crate) fn capture_breaks_off(&mut self, caster: Entity, target: Target) {
        let Some(outpost) = self.channel_mark(target) else {
            return;
        };
        let named = wire_id(outpost);
        let goes_on = self.entities.iter().any(|unit| {
            unit != caster
                && matches!(
                    self.action.get(unit).map(|action| action.state),
                    Some(ActionState::CastAbility {
                        target: Target::Unit(at),
                        slot,
                        phase: ActionPhase::During { .. },
                    }) if at == named && self.ability_in(unit, slot) == AbilityId::CAPTURE
                )
        });
        if !goes_on {
            self.capture.remove(outpost);
        }
    }

    /// The side a Watcher is lit for, while it is.
    fn lit_for(&self, watcher: Entity) -> Option<Team> {
        self.modifiers
            .get(watcher)?
            .active()
            .find_map(|held| match held.kind {
                ModifierKind::VisionGrant { side, .. } => Some(side),
                _ => None,
            })
    }

    /// `ability_lamp_use` goes off: the channel begins, from the horn on, at
    /// a Watcher that is not out and not already lit for the caster's side.
    pub(crate) fn lamp_begins(&mut self, caster: Entity, target: Target) -> bool {
        let Some(watcher) = self.channel_mark(target) else {
            return false;
        };
        self.clock() >= self.schedule().horn + OBJECTIVES_OPEN_AFTER_HORN
            && self.kind.get(watcher) == Some(&UnitKind::Watcher)
            && !self.wears(watcher, ModifierKind::LampOff)
            && self.lit_for(watcher) != self.team.get(caster).copied()
    }

    /// A lamp's channel ran its whole length: a Watcher at rest is lit for
    /// the caster's side for [`LAMP_LIT_TICKS`]; one lit for the other side
    /// is put out for [`LAMP_OFF_TICKS`], and is at rest again after.
    pub(crate) fn lamp_completes(&mut self, caster: Entity, target: Target) {
        let (Some(watcher), Some(side)) =
            (self.channel_mark(target), self.team.get(caster).copied())
        else {
            return;
        };
        if self.wears(watcher, ModifierKind::LampOff) {
            return;
        }
        let inflictor = Some(Inflictor::Ability(AbilityId::LAMP_USE));
        match self.lit_for(watcher) {
            Some(lit) if lit == side => {}
            Some(lit) => {
                self.take_modifier(watcher, lit_by(lit), None);
                self.put_modifier(
                    watcher,
                    Modifier::new(ModifierKind::LampOff, None, inflictor, Some(LAMP_OFF_TICKS)),
                );
            }
            None => self.put_modifier(
                watcher,
                Modifier::new(lit_by(side), None, inflictor, Some(LAMP_LIT_TICKS)),
            ),
        }
    }
}

/// The sight a Watcher lit for a side gives it.
const fn lit_by(side: Team) -> ModifierKind {
    ModifierKind::VisionGrant {
        side,
        day: WATCHER_DAY_VISION,
        night: WATCHER_NIGHT_VISION,
    }
}
