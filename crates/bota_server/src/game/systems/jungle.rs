//! The jungle: filling camps, and what a neutral does once drawn off one.

use bota_core::{
    CampDef, FLOODED_CREEPS, NeutralKind, Roster, SpawnBox, flooded_creeps, flooded_evolutions,
    jungle_upgrades, neutral_spawn_due, rosters_of, rules,
};
use bota_proto::{Team, UnitKind, Vec2};

use crate::game::{Entity, Landed, Purpose, Spots, UnitOrder, World};

impl World {
    /// Fills every unblocked camp on the minute mark: a dry camp with a
    /// roster other than the one it last put out, a flooded one with its
    /// amphibians as far as they have evolved. On the mark of an evolution a
    /// flooded camp that nothing holds trades the creeps it has for the
    /// evolved ones.
    ///
    /// A camp with a unit standing in its box puts nothing out.
    pub fn fill_camps(&mut self) {
        self.raise_neutrals();
        if !neutral_spawn_due(self.tick) {
            return;
        }
        let upgrades = jungle_upgrades(self.tick);
        for (index, camp) in self.map.camps.iter().enumerate() {
            let index = u8::try_from(index).expect("a map holds fewer than 256 camps");
            if camp.flooded && self.evolves_now(camp) {
                self.clear_flooded(index, camp.spawn_box);
            }
            if self.anything_in(camp.spawn_box) {
                continue;
            }
            if camp.flooded {
                let creeps = self.flooded_spawn(index, camp);
                self.stand_camp(index, camp.pos, &creeps, upgrades);
            } else if let Some(roster) = self.draw_roster(index, camp) {
                self.stand_camp(index, camp.pos, roster.creeps, upgrades);
            }
        }
        self.settle();
    }

    /// A roster for a dry camp other than the one it last put out.
    fn draw_roster(&mut self, index: u8, camp: &CampDef) -> Option<&'static Roster> {
        let at = usize::from(index);
        let last = self.camp_last.get(at).copied().unwrap_or(u8::MAX);
        let fresh: Vec<(u8, &'static Roster)> =
            rosters_of(camp).filter(|(id, _)| *id != last).collect();
        if fresh.is_empty() {
            return None;
        }
        let pick = self
            .rng
            .global(Purpose::NeutralSpawn)
            .below(fresh.len() as u32);
        let (id, roster) = fresh[pick as usize];
        if self.camp_last.len() <= at {
            self.camp_last.resize(at + 1, u8::MAX);
        }
        self.camp_last[at] = id;
        Some(roster)
    }

    /// Stands creeps in a row across a camp's spot.
    fn stand_camp(&mut self, camp: u8, pos: Vec2, creeps: &[NeutralKind], upgrades: u32) {
        for (slot, kind) in creeps.iter().enumerate() {
            let across = (slot as i32 - (creeps.len() as i32 - 1) / 2) * rules::CAMP_SPAWN_SPACING;
            self.spawn_neutral(*kind, pos + Vec2::from_ints(across, 0), camp, upgrades);
        }
    }

    /// Whether this tick evolves a flooded camp.
    fn evolves_now(&self, camp: &CampDef) -> bool {
        self.tick > 0
            && flooded_evolutions(camp, self.tick) != flooded_evolutions(camp, self.tick - 1)
    }

    /// Takes a flooded camp's creeps off the map for its evolution, unless
    /// one of them is awake or walking home, or anything but its own stands
    /// in its box: then the evolved creeps wait for its next fill.
    fn clear_flooded(&mut self, camp: u8, spawn_box: SpawnBox) {
        let own: Vec<Entity> = self
            .entities
            .iter()
            .filter(|beast| {
                self.camp_home
                    .get(*beast)
                    .is_some_and(|home| home.camp == camp)
            })
            .collect();
        let held = own.iter().any(|beast| {
            self.neutral_ai
                .get(*beast)
                .is_some_and(|ai| ai.awake || ai.going_home)
        });
        let blocked = self.entities.iter().any(|other| {
            !own.contains(&other) && self.alive(other) && self.blocks_camp(other, spawn_box)
        });
        if held || blocked {
            return;
        }
        for beast in own {
            self.despawn(beast);
        }
    }

    /// What a flooded camp puts out now: its creeps as far as they have
    /// evolved, save that a creep evolved part way through a size is not
    /// put out again while the one already out still stands.
    fn flooded_spawn(&self, camp: u8, def: &CampDef) -> [NeutralKind; FLOODED_CREEPS] {
        let done = flooded_evolutions(def, self.tick);
        let mut creeps = flooded_creeps(def, done);
        let whole = flooded_creeps(def, done - done % FLOODED_CREEPS as u32);
        let mut standing: Vec<NeutralKind> = self
            .entities
            .iter()
            .filter(|beast| {
                self.alive(*beast)
                    && self
                        .camp_home
                        .get(*beast)
                        .is_some_and(|home| home.camp == camp)
            })
            .filter_map(|beast| self.neutral.get(beast).copied())
            .collect();
        for (creep, plain) in creeps.iter_mut().zip(whole) {
            if *creep == plain {
                continue;
            }
            if let Some(at) = standing.iter().position(|kind| kind == creep) {
                standing.swap_remove(at);
                *creep = plain;
            }
        }
        creeps
    }

    /// Raises every neutral standing to the upgrades the clock has reached:
    /// Dota's upgrades reach the living, not only those spawned after.
    fn raise_neutrals(&mut self) {
        let now = jungle_upgrades(self.tick);
        if self.tick == 0 || now == jungle_upgrades(self.tick - 1) {
            return;
        }
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            self.set_neutral_upgrades(entity, now);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Whether anything blocks a camp's spawn box.
    fn anything_in(&self, spawn_box: SpawnBox) -> bool {
        self.entities
            .iter()
            .any(|entity| self.blocks_camp(entity, spawn_box))
    }

    /// Whether a unit keeps a camp from spawning: anything alive standing in
    /// its box, whether it walks or not, wards included, however hidden or
    /// invulnerable it is. Dota lets the courier through; what the map has
    /// that no such unit stands for (items, trees, the dead) does not count.
    fn blocks_camp(&self, entity: Entity, spawn_box: SpawnBox) -> bool {
        self.alive(entity)
            && self.kind.get(entity) != Some(&UnitKind::Courier)
            && self.pos(entity).is_some_and(|pos| spawn_box.contains(pos))
    }

    /// Runs down every neutral's patience and sends home the ones led too far.
    ///
    /// A neutral gives up when it has been led past its guard distance for
    /// longer than its window, and takes nothing on again until it is back and
    /// its block has run out. At night it sleeps: nothing coming near wakes
    /// it (Liquipedia, Time of Day: a neutral's aggro range is 0 by night).
    pub fn tick_jungle(&mut self) {
        let guard = rules::units(rules::NEUTRAL_GUARD_DISTANCE);
        let back = rules::units(rules::NEUTRAL_RETURN);
        let night = bota_core::time_of_day(self.tick).night;
        let entities = self.take_entity_snapshot();
        let mut wakers = std::mem::take(&mut self.jungle_spots);
        let mut laid = false;
        for entity in entities.iter().copied() {
            let (Some(mut ai), Some(home), Some(at)) = (
                self.neutral_ai.get(entity).copied(),
                self.camp_home.get(entity).copied(),
                self.pos(entity),
            ) else {
                continue;
            };
            ai.reaggro_block = ai.reaggro_block.saturating_sub(1);
            if ai.going_home {
                ai.awake = false;
                if at.within(home.home, back) {
                    ai.going_home = false;
                    ai.leash_left = ai.next_window;
                    self.set_order(entity, UnitOrder::Idle);
                } else {
                    self.set_order(entity, UnitOrder::Move { pos: home.home });
                }
                self.neutral_ai.insert(entity, ai);
                continue;
            }
            if at.within(home.home, guard) {
                ai.leash_left = ai.next_window;
            } else {
                ai.leash_left = ai.leash_left.saturating_sub(1);
                if ai.leash_left == 0 {
                    ai.going_home = true;
                    ai.reaggro_block = rules::NEUTRAL_REAGGRO_BLOCK;
                    ai.next_window = rules::NEUTRAL_SHORT_WINDOW;
                }
            }
            // Asleep it wakes by day for what comes right up to it; from
            // further off, and at night from anywhere, nothing but a blow
            // wakes it.
            if !ai.awake && !ai.going_home && ai.reaggro_block == 0 && !night {
                if !laid {
                    self.lay_wakers(&mut wakers);
                    laid = true;
                }
                ai.awake = self.anything_hostile_near(entity, rules::NEUTRAL_AGGRO_RANGE, &wakers);
            }
            if !ai.going_home
                && let Some(target) = self.target_of(entity)
                && self.alive(target)
            {
                let Some(their_at) = self.pos(target) else {
                    continue;
                };
                self.set_order(entity, UnitOrder::AttackMove { pos: their_at });
            }
            self.neutral_ai.insert(entity, ai);
        }
        wakers.clear();
        self.jungle_spots = wakers;
        self.recycle_entity_snapshot(entities);
    }

    /// Lays out every live, placed entity of a side other than the neutrals':
    /// the only ones a neutral would fight. Nothing tick_jungle does moves,
    /// kills or turns them, so one layout serves the whole pass.
    fn lay_wakers(&self, wakers: &mut Spots<Entity>) {
        wakers.clear();
        for entity in self.entities.iter() {
            let side = self.team.get(entity).copied();
            if side.is_none() || side == Some(Team::Neutral) || !self.alive(entity) {
                continue;
            }
            if let Some(transform) = self.transform.get(entity) {
                wakers.push(transform.pos, entity);
            }
        }
        wakers.sort();
    }

    /// Whether anything this entity would fight stands within a reach of it.
    ///
    /// A neutral fights nothing of its own side and nothing dead, so it asks
    /// only `wakers` within the square about it; anything else asks every
    /// entity. The answer is the same either way: a pure test over a set.
    fn anything_hostile_near(&self, entity: Entity, range: i32, wakers: &Spots<Entity>) -> bool {
        let Some(at) = self.pos(entity) else {
            return false;
        };
        let reach = rules::units(range);
        let hostile = |other: Entity| {
            self.pos(other).is_some_and(|pos| pos.within(at, reach)) && self.hostile(entity, other)
        };
        if self.team.get(entity) == Some(&Team::Neutral) {
            wakers.around(at, i64::from(reach.raw).abs()).any(hostile)
        } else {
            self.entities.iter().any(hostile)
        }
    }

    /// Wakes every neutral of a camp onto whoever struck one of them.
    ///
    /// A camp answers as one: what is struck does not answer alone, and what
    /// is walking home or still blocked from waking is not called back by it.
    /// A blow wakes them from as far as [`rules::NEUTRAL_DAMAGE_AGGRO_RANGE`],
    /// seen or not.
    pub fn rouse_camps(&mut self, felt: &[Landed]) {
        for blow in felt {
            let (Some(by), Some(struck)) = (blow.source, self.camp_home.get(blow.target).copied())
            else {
                continue;
            };
            if self.neutral_ai.get(blow.target).is_none() || !self.alive(by) {
                continue;
            }
            let (Some(here), Some(there)) = (self.pos(blow.target), self.pos(by)) else {
                continue;
            };
            if !here.within(there, rules::units(rules::NEUTRAL_DAMAGE_AGGRO_RANGE)) {
                continue;
            }
            for beast in self.entities.iter() {
                if self.camp_home.get(beast).map(|home| home.camp) != Some(struck.camp) {
                    continue;
                }
                let Some(mut ai) = self.neutral_ai.get(beast).copied() else {
                    continue;
                };
                if ai.going_home || ai.reaggro_block > 0 || !self.alive(beast) {
                    continue;
                }
                ai.roused_by = Some(by);
                ai.awake = true;
                ai.leash_left = ai.next_window;
                self.neutral_ai.insert(beast, ai);
            }
        }
    }
}
