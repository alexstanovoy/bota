//! Shortcuts round the rules, for trying things out in a match that allows
//! them: Dota's cheats (dota2.fandom.com/wiki/Cheats) and its Demo Hero's
//! panel. Each is carried out here as an ordinary effect of the tick it is
//! taken on, through the rules' own machinery where there is one (levels as
//! experience, a respawn as a respawn, waves and camps as the spawners lay
//! them), so a match with cheats stays as deterministic as one without.

use bota_core::{hero_def, hero_spawn_pos, rules};
use bota_proto::{
    Cheat, CreepSide, EventKind, ItemId, RejectReason, SlotId, Target, Team, UnitKind, Vec2,
    modifier_ticks_bounded,
};

use crate::game::{
    AppliedModifier, AppliedOrigin, Entity, Event, EventVisibility, ItemStack, Seat, UnitOrder,
    World,
};

/// The seats a match may hold at most, cheats' heroes included: as many as
/// a client takes in a view.
pub const MAX_SEATS_WITH_CHEATS: usize = 10;

impl World {
    /// Carries out a cheat for a seat, through the hero it drives when one
    /// stands.
    pub fn cheat(
        &mut self,
        slot: SlotId,
        unit: Option<Entity>,
        cheat: Cheat,
        events: &mut Vec<Event>,
    ) {
        if self.check_cheat(slot, unit, &cheat).is_err() {
            return;
        }
        let Some(seat) = self.seats.iter().position(|seat| seat.slot == slot) else {
            return;
        };
        match cheat {
            Cheat::Gold { amount } => {
                let gold = &mut self.seats[seat].gold;
                *gold = gold.saturating_add(amount).max(0);
            }
            Cheat::Levels { count } => self.raise_levels(seat, count, events),
            Cheat::Refresh => self.refresh_all(),
            Cheat::Item { item } => self.hand_out(seat, item, events),
            Cheat::ApplyModifier {
                target,
                spec,
                ticks,
            } => {
                if let Some(mark) = unit.and_then(|unit| self.cheat_target(unit, target).ok()) {
                    // Replaces only what a cheat put there before.
                    let mut applied = self.applied.remove(mark).unwrap_or_default();
                    applied.retain(|held| held.origin != AppliedOrigin::Cheat);
                    applied.push(AppliedModifier {
                        spec,
                        ticks_left: Some(ticks),
                        origin: AppliedOrigin::Cheat,
                    });
                    self.applied.insert(mark, applied);
                }
            }
            Cheat::ClearModifiers { target } => {
                if let Some(mark) = unit.and_then(|unit| self.cheat_target(unit, target).ok()) {
                    let empty = self.applied.get_mut(mark).is_some_and(|applied| {
                        applied.retain(|held| held.origin != AppliedOrigin::Cheat);
                        applied.is_empty()
                    });
                    if empty {
                        self.applied.remove(mark);
                    }
                }
            }
            Cheat::LevelBots { count } => {
                for index in 0..self.seats.len() {
                    if self.seats[index].created {
                        self.raise_levels(index, count, events);
                    }
                }
            }
            Cheat::Wtf { on } => {
                self.wtf = on;
                if on {
                    self.refresh_all();
                }
            }
            Cheat::Respawn => self.respawn_now(seat),
            Cheat::CreateHero { hero, enemy, at } => {
                let team = self.cheat_side(seat, enemy);
                if let Some(at) = self.open_landing(at) {
                    self.create_hero(hero, team, at);
                }
            }
            Cheat::SpawnCreeps => {
                let wave = bota_core::latest_wave(self.clock()).unwrap_or(1);
                self.spawn_wave(wave);
            }
            Cheat::CreepSpawning { on } => self.creeps_halted = !on,
            Cheat::SpawnNeutrals => self.fill_camps_now(),
            Cheat::KillCreeps { which } => self.kill_creeps(which),
            Cheat::StartGame => self.start_game(),
            Cheat::AllVision { on } => self.all_vision = on,
            Cheat::Invulnerable { on } => self.seats[seat].invulnerable = on,
            Cheat::Teleport { at } => {
                if let (Some(unit), Some(at)) = (unit, self.open_landing(at)) {
                    self.move_unit_to(unit, at);
                }
            }
            Cheat::ResetHero => self.reset_hero(seat),
            Cheat::TimeOfDay { night } => {
                let now = self.time_of_day();
                if now.night != night {
                    self.clock_shift.day = self.clock_shift.day.saturating_add(now.ticks_left);
                }
            }
            Cheat::CreateCreeps { enemy, at } => {
                let team = self.cheat_side(seat, enemy);
                if let Some(at) = self.open_landing(at) {
                    self.spawn_wave_at(team, at);
                }
            }
        }
    }

    /// Whether a seat may issue a cheat now: cheats on, what it names real
    /// and in bounds, and a hero standing for what works through one.
    pub fn check_cheat(
        &self,
        slot: SlotId,
        unit: Option<Entity>,
        cheat: &Cheat,
    ) -> Result<(), RejectReason> {
        if !self.cheats {
            return Err(RejectReason::NoCheats);
        }
        let seat = self.seat(slot).ok_or(RejectReason::NotYourUnit)?;
        if seat.created {
            return Err(RejectReason::NotYourUnit);
        }
        match cheat {
            Cheat::ApplyModifier {
                target,
                spec,
                ticks,
            } => {
                if !spec.is_bounded() || !modifier_ticks_bounded(*ticks) {
                    return Err(RejectReason::BadCheat);
                }
                self.cheat_target(unit.ok_or(RejectReason::HeroDead)?, *target)?;
            }
            Cheat::ClearModifiers { target } => {
                self.cheat_target(unit.ok_or(RejectReason::HeroDead)?, *target)?;
            }
            Cheat::Item { item } => {
                bota_core::item_def(*item).ok_or(RejectReason::BadCheat)?;
            }
            Cheat::CreateHero { hero, at, .. } => {
                hero_def(*hero).ok_or(RejectReason::BadCheat)?;
                if self.seats.len() >= MAX_SEATS_WITH_CHEATS {
                    return Err(RejectReason::BadCheat);
                }
                self.open_landing(*at).ok_or(RejectReason::ClosedGround)?;
            }
            Cheat::CreateCreeps { at, .. } => {
                self.open_landing(*at).ok_or(RejectReason::ClosedGround)?;
            }
            Cheat::Teleport { at } => {
                unit.ok_or(RejectReason::HeroDead)?;
                self.open_landing(*at).ok_or(RejectReason::ClosedGround)?;
            }
            Cheat::Gold { .. }
            | Cheat::Levels { .. }
            | Cheat::Refresh
            | Cheat::LevelBots { .. }
            | Cheat::Wtf { .. }
            | Cheat::Respawn
            | Cheat::SpawnCreeps
            | Cheat::CreepSpawning { .. }
            | Cheat::SpawnNeutrals
            | Cheat::KillCreeps { .. }
            | Cheat::StartGame
            | Cheat::AllVision { .. }
            | Cheat::Invulnerable { .. }
            | Cheat::ResetHero
            | Cheat::TimeOfDay { .. } => {}
        }
        Ok(())
    }

    /// The side a cheat puts what it creates on: the seat's, or the other.
    fn cheat_side(&self, seat: usize, enemy: bool) -> Team {
        let own = self.seats[seat].team;
        match (own, enemy) {
            (own, false) => own,
            (Team::Radiant, true) => Team::Dire,
            (_, true) => Team::Radiant,
        }
    }

    /// Takes a seat's hero up so many levels, to the cap at most, by
    /// granting the experience the levels are worth.
    fn raise_levels(&mut self, seat: usize, count: u8, events: &mut Vec<Event>) {
        let level = self.seats[seat].level;
        let wanted = level.saturating_add(count).min(rules::HERO_MAX_LEVEL);
        if wanted <= level {
            return;
        }
        let needed = rules::XP_THRESHOLDS[usize::from(wanted - 1)] - self.seats[seat].xp;
        self.grant_xp(seat, needed.max(0), events);
    }

    /// Dota's `-refresh`: every hero and illusion of both sides with full
    /// pools and nothing on cooldown, every seat's item waits gone, and both
    /// Glyphs ready. Buyback stays as it is.
    fn refresh_all(&mut self) {
        let heroes: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.kind.get(entity) == Some(&UnitKind::Hero))
            .collect();
        for unit in heroes {
            self.fill_pools(unit);
            if let Some(book) = self.abilities.get_mut(unit) {
                for slot in book.slots.iter_mut() {
                    slot.cooldown = 0;
                }
            }
            if let Some(bag) = self.inventory.get_mut(unit) {
                for stack in bag.every_stack_mut() {
                    stack.cooldown = 0;
                    stack.mute = 0;
                }
            }
        }
        for seat in &mut self.seats {
            seat.item_clocks.clear();
        }
        let now = self.tick;
        for glyph in &mut self.glyphs {
            glyph.ready_at = glyph.ready_at.min(now);
        }
    }

    /// Puts an item into the hero's bag for nothing, or into the stash when
    /// the bag has no room. Told of as a purchase.
    fn hand_out(&mut self, seat: usize, item: ItemId, events: &mut Vec<Event>) {
        let owner = self.seats[seat].slot;
        let Some(stack) = ItemStack::bought(item, owner, self.tick) else {
            return;
        };
        let in_hand = self.seats[seat]
            .unit
            .and_then(|unit| self.inventory.get_mut(unit))
            .is_some_and(|bag| bag.receive(stack));
        if !in_hand && !self.seats[seat].stash.receive(stack) {
            return;
        }
        events.push(Event {
            kind: EventKind::ItemBought { slot: owner, item },
            visible_to: EventVisibility::OneTeam(self.seats[seat].team),
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }

    /// Dota's `-respawn`: a fallen hero stands up at its fountain now; a
    /// standing one is moved there with its pools full.
    fn respawn_now(&mut self, seat: usize) {
        match self.seats[seat].unit {
            None => self.respawn(seat),
            Some(unit) => {
                let fountain = hero_spawn_pos(self.map, self.seats[seat].team);
                let at = self.open_landing(fountain).unwrap_or(fountain);
                self.move_unit_to(unit, at);
                self.fill_pools(unit);
            }
        }
    }

    /// Moves a unit to a spot at once, its walk and its orders let go.
    fn move_unit_to(&mut self, unit: Entity, at: Vec2) {
        if let Some(place) = self.transform.get_mut(unit) {
            place.pos = at;
        }
        self.forget_walk(unit);
        self.queue.remove(unit);
        self.waiting.remove(unit);
        self.set_order(unit, UnitOrder::Idle);
        self.settle();
    }

    /// A hero no player drives, on a seat of its own made for it: it stands
    /// where it is put, at level one, and earns as a seat does. It has no
    /// courier.
    fn create_hero(&mut self, hero: bota_proto::HeroId, team: Team, at: Vec2) {
        let next = self
            .seats
            .iter()
            .map(|seat| seat.slot.0)
            .max()
            .map_or(0, |top| top + 1);
        let slot = SlotId(next);
        let mut seat = Seat::new(slot, team, hero, 0, rules::STASH_SLOTS);
        seat.created = true;
        let unit = self.spawn_hero(team, at, slot, hero);
        seat.unit = Some(unit);
        self.seats.push(seat);
        self.settle();
    }

    /// Dota's `-killcreeps`: the creeps of a side, the neutrals or all of
    /// them taken off the map, with no bounty and nothing done on their
    /// death.
    fn kill_creeps(&mut self, which: CreepSide) {
        let doomed: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| {
                let lane = matches!(
                    self.kind.get(entity),
                    Some(
                        UnitKind::CreepMelee
                            | UnitKind::CreepFlagbearer
                            | UnitKind::CreepRanged
                            | UnitKind::CreepSiege
                    )
                );
                let neutral = self.kind.get(entity) == Some(&UnitKind::CreepNeutral);
                let team = self.team.get(entity).copied();
                match which {
                    CreepSide::Radiant => lane && team == Some(Team::Radiant),
                    CreepSide::Dire => lane && team == Some(Team::Dire),
                    CreepSide::Neutral => neutral,
                    CreepSide::All => lane || neutral,
                }
            })
            .collect();
        for creep in doomed {
            self.despawn(creep);
        }
        self.settle();
    }

    /// Dota's `-startgame`: the pregame ends now, the game clock moved on to
    /// the horn on the next tick, and what waits on the game clock (the
    /// Glyph's first use, the seats' first scroll) moved on with it.
    fn start_game(&mut self) {
        let next = self.clock().saturating_add(1);
        if next >= rules::PREGAME_TICKS {
            return;
        }
        let ahead = rules::PREGAME_TICKS - next;
        self.clock_shift.game = self.clock_shift.game.saturating_add(ahead);
        for glyph in &mut self.glyphs {
            glyph.ready_at = glyph.ready_at.saturating_sub(ahead);
        }
        for seat in &mut self.seats {
            for (_, left) in &mut seat.item_clocks {
                *left = left.saturating_sub(ahead);
            }
        }
    }

    /// Demo Hero's Reset Hero: the seat's hero starts over at its fountain,
    /// at level one with the starting gold, nothing carried or stashed and
    /// nothing learned.
    fn reset_hero(&mut self, seat: usize) {
        if let Some(unit) = self.seats[seat].unit.take() {
            self.despawn(unit);
        }
        let (slot, team, hero) = (
            self.seats[seat].slot,
            self.seats[seat].team,
            self.seats[seat].hero,
        );
        let mut fresh = Seat::new(slot, team, hero, rules::STARTING_GOLD, rules::STASH_SLOTS);
        let old = &self.seats[seat];
        fresh.created = old.created;
        fresh.invulnerable = old.invulnerable;
        fresh.courier = old.courier;
        fresh.courier_left = old.courier_left;
        fresh.courier_kept = old.courier_kept.clone();
        fresh.kills = old.kills;
        fresh.deaths = old.deaths;
        fresh.assists = old.assists;
        fresh.last_hits = old.last_hits;
        fresh.denies = old.denies;
        fresh.hero_damage = old.hero_damage;
        fresh.structure_damage = old.structure_damage;
        self.seats[seat] = fresh;
        let at = hero_spawn_pos(self.map, team);
        let unit = self.spawn_hero(team, at, slot, hero);
        self.seats[seat].unit = Some(unit);
        self.settle();
    }

    /// The unit a modifier cheat lands on: the one it names, else the one
    /// the order is for.
    pub fn cheat_target(&self, unit: Entity, target: Target) -> Result<Entity, RejectReason> {
        match target {
            Target::None => Ok(unit),
            Target::Unit(id) => {
                let mark = self.of_wire(id).ok_or(RejectReason::UnknownTarget)?;
                if self.def.get(mark).is_none() {
                    return Err(RejectReason::UnknownTarget);
                }
                Ok(mark)
            }
            Target::Pos(_) => Err(RejectReason::WrongTargetKind),
        }
    }
}
