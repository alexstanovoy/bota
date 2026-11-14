//! What the client makes of the events of a tick: floaters, the kill feed,
//! and cues for the front end.

use bota_proto::{
    AbilityId, DamageKind, EntityId, EventKind, GoldAward, GoldAwardKind, RuneKind, SlotId, Team,
    UnitKind, UnitView,
};

use crate::ui::app::{
    App, CueKind, FeedKind, Floater, FloaterKind, Killer, MAX_TOLD, RuneAct, Victim,
};

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

    /// Takes in one event, and keeps it for the front end.
    pub(crate) fn absorb_event(&mut self, event: EventKind) {
        let before = if self.told.len() < MAX_TOLD {
            let mut referenced = event.clone();
            let mut before = Vec::new();
            referenced.visit_ids(&mut |id| {
                if before.iter().any(|unit: &UnitView| unit.id == *id) {
                    return;
                }
                if let Some(unit) = self.prev_view().and_then(|view| view.unit(*id)) {
                    before.push(unit.clone());
                }
            });
            let before: std::sync::Arc<[UnitView]> = before.into();
            self.before_told.push(before.clone());
            self.told.push(event.clone());
            Some(before)
        } else {
            None
        };
        let fed = self.feed.len();
        let sounded = self.cues.len();
        self.take_in(event.clone());
        for line in &mut self.feed[fed..] {
            line.told = Some(event.clone());
        }
        for cue in &mut self.cues[sounded..] {
            cue.told = Some(event.clone());
            cue.before = before.clone();
        }
    }

    /// What one event sets off: its floaters, its feed line, its cues.
    fn take_in(&mut self, event: EventKind) {
        match event {
            EventKind::MegaCreeps { team } => {
                let lost = match team {
                    Team::Radiant => Team::Dire,
                    Team::Dire => Team::Radiant,
                    Team::Neutral => return,
                };
                self.cue(CueKind::MegaCreeps(team), 1.0, None);
                // The installed Ancient sound-event names do not specify emitter coordinates.
                // This losing-Ancient anchor is unverified in Dota, pending an in-game check;
                // Valve 7.20 documents the horn's addition, not its placement.
                let ancient = self.statics().and_then(|statics| {
                    bota_core::map_of(statics.metadata().map).ancients[bota_core::team_index(lost)]
                });
                if let Some(at) = ancient {
                    self.cue(
                        CueKind::MegaCreepsHorn(lost),
                        1.0,
                        Some((at.x.to_f32(), at.y.to_f32())),
                    );
                }
            }
            EventKind::BoughtBack { slot } => {
                let own = self.my_slot == Some(slot);
                self.cue(CueKind::BoughtBack { own }, 1.0, None);
                if let Some(team) = self.player(slot).map(|player| player.team) {
                    self.push_feed(FeedKind::BoughtBack { slot, team });
                }
            }
            EventKind::ItemPlaced { item, team, .. } => {
                self.push_feed(FeedKind::ItemPlaced { item, team })
            }
            EventKind::ItemDestroyed { item, by, team, .. } => self.push_feed(FeedKind::Loot {
                item,
                by,
                team,
                destroyed: true,
            }),
            EventKind::ItemTaken { item, by, team } => self.push_feed(FeedKind::Loot {
                item,
                by: Some(by),
                team,
                destroyed: false,
            }),
            EventKind::BossRoared { objective, team } => {
                self.push_feed(FeedKind::BossRoared { objective, team });
                self.cue(super::CueKind::BossRoar, 1.0, None);
            }
            EventKind::BossFell { objective, team } => {
                self.push_feed(FeedKind::BossFell { objective, team });
                self.cue(super::CueKind::BossDeath(objective), 1.0, None);
            }
            EventKind::Damaged {
                source,
                target,
                amount,
                kind,
                ..
            } => self.on_damage(source, target, amount, kind),
            EventKind::Missed { source, target } => {
                if self.involves_hero(source, target) {
                    self.float(target, 0, FloaterKind::Miss);
                }
            }
            EventKind::Died {
                unit,
                killer,
                denied,
                bounty,
                assists,
                awards,
                credited_hero,
                ..
            } => self.on_death(
                unit,
                credited_hero.map(|hero| *hero).or(killer),
                denied,
                bounty.unwrap_or(0),
                assists,
                awards.as_deref().map(Vec::as_slice),
            ),
            EventKind::StructureDestroyed {
                unit,
                team,
                kind,
                lane,
                tier,
                ranged,
                ..
            } => {
                // The fall of a building is told just after its death, which names who struck it.
                let killer = match self.fallen_to.take() {
                    Some((fallen, killer)) if fallen == unit => killer,
                    _ => None,
                };
                let at = self.spot(unit);
                self.cue(CueKind::StructureFell, 1.0, at);
                self.push_feed(FeedKind::StructureFell {
                    team,
                    kind,
                    lane,
                    tier,
                    ranged,
                    killer: killer.and_then(|k| self.killer_of(k)),
                });
            }
            EventKind::LevelUp { unit, level } => {
                self.float(unit, i32::from(level), FloaterKind::Level);
                if Some(unit) == self.ears() {
                    self.cue(CueKind::LevelUp, 0.9, None);
                }
            }
            EventKind::AbilityCast { caster, ability } => self.on_cast(caster, ability),
            EventKind::ItemBought { slot, item } => {
                if Some(slot) == self.my_slot {
                    self.quick_buy.bought(item);
                    self.cue(CueKind::Buy, 0.9, None);
                }
            }
            EventKind::Healed { target, amount, .. } => {
                if amount > 0 && self.is_hero(target) {
                    self.float(target, amount, FloaterKind::Heal);
                }
            }
            // A teleport's end, a burst and a tree falling or growing back
            // are the front end's to sound and draw, from what was told; a
            // burst's blows float their own numbers.
            EventKind::Teleported { .. }
            | EventKind::AttackLaunched { .. }
            | EventKind::Landed { .. }
            | EventKind::TreeFelled { .. }
            | EventKind::TreeGrew { .. }
            | EventKind::ItemUsed { .. } => {}
            EventKind::Captured { unit, team, .. } => {
                let building = self
                    .freshest(unit)
                    .map(|seen| (seen.kind, seen.team, seen.pos));
                self.push_feed(FeedKind::Captured { building, by: team });
            }
            EventKind::ShrineActivated { team, xp, by, .. } => {
                let by = by.and_then(|hero| self.seat_of_hero(hero));
                self.push_feed(FeedKind::ShrineActivated { by, team, xp });
            }
            EventKind::RuneActivated {
                unit, kind, gold, ..
            } => self.rune_line(unit, None, kind, RuneAct::Activated { gold }),
            EventKind::RuneBottled { unit, kind } => {
                self.rune_line(unit, None, kind, RuneAct::Bottled);
            }
            EventKind::RuneDenied {
                unit, kind, team, ..
            } => self.rune_line(unit, Some(team), kind, RuneAct::Denied),
            // Dota's feed has no line for a pool growing a lotus or a hero
            // plucking one; the world shows both.
            EventKind::RuneSpawned { .. }
            | EventKind::LotusGrown { .. }
            | EventKind::LotusPlucked { .. } => {}
            // Dota's feed tells no warp: the gates glow and hum, and the unit
            // comes out at the other end.
            EventKind::GateWarped { .. } | EventKind::RetiredItemState { .. } => {}
        }
    }

    /// A feed line for a hero and a rune: the hero's seat if it is known, and
    /// its side, told or read off the hero in the view.
    fn rune_line(&mut self, hero: EntityId, told: Option<Team>, kind: RuneKind, act: RuneAct) {
        let team = told.or_else(|| {
            self.view()
                .and_then(|view| view.unit(hero))
                .map(|unit| unit.team)
        });
        let Some(team) = team else {
            return;
        };
        let by = self.seat_of_hero(hero);
        self.push_feed(FeedKind::Rune {
            by,
            team,
            kind,
            act,
        });
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
        self.float(target, amount, look);
        let ears = self.ears();
        if source.is_some() && source == ears && kind == DamageKind::Physical {
            let hero = source.and_then(|id| self.freshest(id)).and_then(|u| u.hero);
            self.cue_blow(hero, CueKind::Hit, 0.55, None, Some(target));
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
    ///
    /// A fall is told of a unit never in sight too: a hero's, which Dota's
    /// feed announces wherever it falls (named by its seat), and a kill of
    /// this side's own in the fog, whose gold is told: its last hit sounds,
    /// with no spot to draw a number at.
    fn on_death(
        &mut self,
        unit: EntityId,
        killer: Option<EntityId>,
        denied: bool,
        gold: i32,
        assists: u16,
        awards: Option<&[GoldAward]>,
    ) {
        self.fallen_to = Some((unit, killer));
        let victim = self
            .freshest(unit)
            .map(|victim| (victim.pos.x.to_f32(), victim.pos.y.to_f32(), victim.kind));
        let fallen = self
            .freshest(unit)
            .and_then(|victim| victim.hero)
            .or_else(|| {
                let view = self.view()?;
                let seat = view
                    .players
                    .iter()
                    .find(|p| self.body_of(p.slot) == Some(unit));
                seat.map(|p| p.hero)
            });
        let at = victim.map(|(x, y, _)| (x, y));
        let hero = victim.map_or_else(
            || self.seat_of_hero(unit).is_some(),
            |(_, _, kind)| kind == UnitKind::Hero,
        );
        if let Some(kind) = self.death_line(unit, killer, gold, assists) {
            self.push_feed(kind);
        }
        let ears = self.ears();
        let mine = killer.is_some() && killer == ears;
        if denied {
            self.float(unit, 0, FloaterKind::Deny);
            if mine {
                self.cue(CueKind::Deny, 0.9, None);
            }
        } else {
            self.float_death_gold(unit, mine, gold, awards);
            if gold > 0 && mine && !hero {
                self.cue(CueKind::LastHit, 0.95, at);
            }
        }
        if hero {
            if mine {
                self.cue(CueKind::HeroKill, 1.0, None);
            }
            let gain = if Some(unit) == ears { 1.0 } else { 0.6 };
            self.cue_of(fallen, CueKind::HeroDeath, gain, at);
        }
    }

    /// Render only the watched recipient's actual credit components; old aggregate-only
    /// deaths retain their existing presentation.
    fn float_death_gold(
        &mut self,
        fallen: EntityId,
        mine: bool,
        gold: i32,
        awards: Option<&[GoldAward]>,
    ) {
        let Some(awards) = awards else {
            if mine && gold > 0 && self.freshest(fallen).is_some() {
                self.float(fallen, gold, FloaterKind::Gold);
            }
            return;
        };
        let watched = self
            .my_slot
            .or_else(|| self.ears().and_then(|hero| self.seat_of_hero(hero)));
        for award in awards
            .iter()
            .filter(|award| Some(award.recipient) == watched && award.amount > 0)
        {
            let unit = match award.kind {
                GoldAwardKind::KillBounty if self.spot(fallen).is_some() => fallen,
                GoldAwardKind::KillBounty
                | GoldAwardKind::FlagbearerBonus
                | GoldAwardKind::AssistGold => self
                    .body_of(award.recipient)
                    .filter(|hero| self.freshest(*hero).is_some())
                    .unwrap_or(fallen),
            };
            self.float(unit, award.amount, FloaterKind::Gold);
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
        for p in view.projectiles.iter().filter(|p| p.inflictor.is_none()) {
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
                launched.push((hero.clone(), gain));
                if launched.len() == 4 {
                    break;
                }
            }
        }
        for (hero, gain) in launched {
            let at = (hero.pos.x.to_f32(), hero.pos.y.to_f32());
            let next = self.cues.len();
            self.cue_of(hero.hero, CueKind::Attack, gain, Some(at));
            if let Some(cue) = self.cues.get_mut(next) {
                cue.thrower = Some(hero);
            }
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
    fn float(&mut self, unit: EntityId, amount: i32, kind: FloaterKind) {
        if let Some(world) = self.spot(unit) {
            self.floaters.push(Floater {
                kind,
                amount,
                world,
                unit,
                age: 0.0,
            });
        }
    }

    /// The kill feed line of a death. Only a hero's death has one.
    fn death_line(
        &self,
        unit: EntityId,
        killer: Option<EntityId>,
        gold: i32,
        assists: u16,
    ) -> Option<FeedKind> {
        let victim = match self.freshest(unit) {
            Some(victim) if victim.kind != UnitKind::Hero => return None,
            Some(victim) => match victim.owner {
                Some(slot) => Victim::Seat(slot),
                None => Victim::Hero(victim.team),
            },
            None => Victim::Seat(self.seat_of_hero(unit)?),
        };
        // The feed only tells hero deaths, whose gold is public as in Dota;
        // the killer it names may never have been in sight, a hero the feed
        // named, known by its seat.
        let killer = killer.and_then(|k| self.killer_of(k));
        Some(FeedKind::HeroDied {
            victim,
            killer,
            gold,
            assists,
        })
    }

    /// Who struck a blow, as a feed line names it: a hero by its seat, which it may never have
    /// been in sight for, any other unit by its kind and side.
    fn killer_of(&self, id: EntityId) -> Option<Killer> {
        match self.freshest(id) {
            Some(k) => Some(match (k.kind, k.owner) {
                (UnitKind::Hero, Some(slot)) => Killer::Seat(slot),
                (kind, _) => Killer::Unit { kind, team: k.team },
            }),
            None => self.seat_of_hero(id).map(Killer::Seat),
        }
    }

    /// The seat whose hero a unit is or was, by the bodies the scoreboard
    /// named: a hero is known by its seat even when it was never in sight.
    fn seat_of_hero(&self, id: EntityId) -> Option<SlotId> {
        self.view()?
            .players
            .iter()
            .map(|player| player.slot)
            .find(|slot| self.body_of(*slot) == Some(id))
    }

    /// A unit as last seen, in view or out of it.
    pub fn freshest(&self, id: EntityId) -> Option<&UnitView> {
        self.known(id).map(|(unit, _)| unit)
    }
}
