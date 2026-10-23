//! What the client makes of the events of a tick: floaters, the kill feed,
//! and cues for the front end.

use bota_proto::{AbilityId, DamageKind, EntityId, EventKind, UnitKind, UnitView};

use crate::ui::app::{App, CueKind, FeedLine, Floater, FloaterKind};

/// Health share under which the low-health cue comes.
const LOW_HP: f32 = 0.25;

impl App {
    /// The hero the cues and the aids are about: one's own, else the hero
    /// the camera is pinned to or that is picked.
    pub fn ears(&self) -> Option<EntityId> {
        if let Some(me) = self.my_hero() {
            return Some(me);
        }
        let hero = |id: EntityId| {
            self.known(id)
                .is_some_and(|(unit, stale)| stale == 0 && unit.kind == UnitKind::Hero)
        };
        self.pinned_unit()
            .filter(|id| hero(*id))
            .or(self.selected.filter(|id| hero(*id)))
    }

    /// Takes in one event.
    pub(crate) fn absorb_event(&mut self, event: EventKind) {
        match event {
            EventKind::Damaged {
                source,
                target,
                amount,
                kind,
            } => self.on_damage(source, target, amount, kind),
            EventKind::Missed { source, target } => {
                if self.involves_hero(source, target) {
                    self.float(target, "miss".to_string(), FloaterKind::Miss);
                }
            }
            EventKind::Died {
                unit,
                killer,
                denied,
                gold,
            } => self.on_death(unit, killer, denied, gold),
            EventKind::StructureDestroyed { team, .. } => {
                self.feed.push(FeedLine {
                    text: format!("a {team:?} structure has fallen"),
                    age: 0.0,
                });
            }
            EventKind::LevelUp { unit, level } => {
                self.float(unit, format!("level {level}"), FloaterKind::Level);
                if Some(unit) == self.ears() {
                    self.cue(CueKind::LevelUp, 0.9, None);
                }
            }
            EventKind::AbilityCast { caster, ability } => self.on_cast(caster, ability),
            EventKind::ItemBought { slot, .. } => {
                if Some(slot) == self.my_slot {
                    self.cue(CueKind::Buy, 0.9, None);
                }
            }
            EventKind::Healed { target, amount, .. } => {
                if amount > 0 && self.is_hero(target) {
                    self.float(target, format!("+{amount}"), FloaterKind::Heal);
                }
            }
        }
    }

    /// A blow landing: its number when a hero is on either side of it, and
    /// the cues of the watched hero's hit and of a tower striking it.
    fn on_damage(
        &mut self,
        source: Option<EntityId>,
        target: EntityId,
        amount: i32,
        kind: DamageKind,
    ) {
        if !self.involves_hero(source, target) {
            return;
        }
        let look = match kind {
            DamageKind::Physical => FloaterKind::Damage,
            _ => FloaterKind::Magic,
        };
        self.float(target, amount.to_string(), look);
        let ears = self.ears();
        if source.is_some() && source == ears && kind == DamageKind::Physical {
            self.cue(CueKind::Hit, 0.55, None);
        }
        let by_tower = source
            .and_then(|id| self.freshest(id))
            .is_some_and(|unit| unit.kind == UnitKind::Tower);
        if by_tower && Some(target) == ears {
            self.cue(CueKind::TowerShot, 0.8, None);
        }
    }

    /// A unit falling: the gold or the deny mark, the feed line of a hero,
    /// and the cues of it.
    fn on_death(&mut self, unit: EntityId, killer: Option<EntityId>, denied: bool, gold: i32) {
        let Some(victim) = self.freshest(unit) else {
            return;
        };
        let at = (victim.pos.x.to_f32(), victim.pos.y.to_f32());
        let hero = victim.kind == UnitKind::Hero;
        if let Some(text) = self.death_line(unit, killer, gold) {
            self.feed.push(FeedLine { text, age: 0.0 });
        }
        let ears = self.ears();
        let mine = killer.is_some() && killer == ears;
        if denied {
            self.float(unit, "!".to_string(), FloaterKind::Deny);
            if mine {
                self.cue(CueKind::Deny, 0.9, None);
            }
        } else if gold > 0 && mine {
            self.float(unit, format!("+{gold}"), FloaterKind::Gold);
            if !hero {
                self.cue(CueKind::LastHit, 0.95, Some(at));
            }
        }
        if hero {
            if mine {
                self.cue(CueKind::HeroKill, 1.0, None);
            }
            let gain = if Some(unit) == ears { 1.0 } else { 0.6 };
            self.cue(CueKind::HeroDeath, gain, Some(at));
        }
    }

    /// An ability taking effect: the cue of a raze or a requiem.
    fn on_cast(&mut self, caster: EntityId, ability: AbilityId) {
        let at = self.spot(caster);
        if matches!(
            ability,
            AbilityId::RAZE_NEAR | AbilityId::RAZE_MID | AbilityId::RAZE_FAR
        ) {
            self.cue(CueKind::Raze, 0.9, at);
        } else if ability == AbilityId::REQUIEM {
            self.cue(CueKind::Requiem, 1.0, at);
        }
    }

    /// A hero letting an attack go: a plain missile that has just appeared,
    /// thrown by a hero.
    pub(crate) fn launch_cues(&mut self) {
        let Some(view) = self.view() else {
            return;
        };
        let prev = self.prev_view();
        let ears = self.ears();
        let mut launched = Vec::new();
        for p in view.projectiles.iter().filter(|p| p.ability.is_none()) {
            let old = prev.is_some_and(|prev| {
                prev.projectiles
                    .binary_search_by_key(&p.id, |q| q.id)
                    .is_ok()
            });
            if old {
                continue;
            }
            let thrower = p.source.and_then(|id| view.unit(id));
            if let Some(hero) = thrower.filter(|unit| unit.kind == UnitKind::Hero) {
                let gain = if Some(hero.id) == ears { 0.7 } else { 0.4 };
                launched.push((gain, (hero.pos.x.to_f32(), hero.pos.y.to_f32())));
            }
        }
        for (gain, at) in launched.into_iter().take(4) {
            self.cue(CueKind::Attack, gain, Some(at));
        }
    }

    /// The low-health cue, at most every second or so while the watched
    /// hero is under a quarter of its health.
    pub(crate) fn heartbeat_cue(&mut self, dt: f32) {
        self.heartbeat = (self.heartbeat - dt).max(0.0);
        let Some(hero) = self.ears().and_then(|id| self.known(id)).map(|(u, _)| u) else {
            return;
        };
        let share = hero.hp.max(0) as f32 / hero.max_hp.max(1) as f32;
        if hero.hp > 0 && share < LOW_HP && self.heartbeat <= 0.0 {
            self.heartbeat = if share < LOW_HP / 2.0 { 0.75 } else { 1.1 };
            self.cue(CueKind::LowHp, 0.8, None);
        }
    }

    /// Whether either side of a blow is a hero.
    fn involves_hero(&self, source: Option<EntityId>, target: EntityId) -> bool {
        self.is_hero(target) || source.is_some_and(|id| self.is_hero(id))
    }

    fn is_hero(&self, id: EntityId) -> bool {
        self.freshest(id)
            .is_some_and(|unit| unit.kind == UnitKind::Hero)
    }

    /// Where a unit stands in the freshest view that still has it.
    fn spot(&self, id: EntityId) -> Option<(f32, f32)> {
        self.freshest(id)
            .map(|u| (u.pos.x.to_f32(), u.pos.y.to_f32()))
    }

    /// Sets a floater off a unit, if it is still to be found.
    fn float(&mut self, unit: EntityId, text: String, kind: FloaterKind) {
        if let Some(world) = self.spot(unit) {
            self.floaters.push(Floater {
                text,
                kind,
                world,
                unit,
                age: 0.0,
            });
        }
    }

    /// The kill feed line of a death. Only a hero's death has one.
    fn death_line(&self, unit: EntityId, killer: Option<EntityId>, gold: i32) -> Option<String> {
        let victim = self.freshest(unit)?;
        if victim.kind != UnitKind::Hero {
            return None;
        }
        let victim = match victim.owner {
            Some(slot) => self.seat_name(slot),
            None => format!("a {:?} hero", victim.team),
        };
        let killer_unit = killer.and_then(|k| self.freshest(k));
        // The feed only tells hero deaths, whose gold is public as in Dota.
        let by = killer_unit.map(|k| match (k.kind, k.owner) {
            (UnitKind::Hero, Some(slot)) => self.seat_name(slot),
            (UnitKind::Tower, _) => format!("the {:?} tower", k.team),
            (UnitKind::Fountain, _) => format!("the {:?} fountain", k.team),
            (kind, _) => format!("a {:?} {}", k.team, bota_core::kind_def(kind).name),
        });
        Some(match by {
            Some(by) if gold > 0 => format!("{victim} was slain by {by} for {gold} gold"),
            Some(by) => format!("{victim} was slain by {by}"),
            None => format!("{victim} died"),
        })
    }

    /// A unit as last seen, in view or out of it.
    pub fn freshest(&self, id: EntityId) -> Option<&UnitView> {
        self.known(id).map(|(unit, _)| unit)
    }
}
