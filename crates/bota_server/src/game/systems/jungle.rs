//! The jungle: filling camps, and what a neutral does once drawn off one.

use bota_core::{Roster, jungle_upgrades, neutral_spawn_due, rosters_of, rules};
use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::game::{Entity, Landed, Purpose, Spots, UnitOrder, World};

impl World {
    /// Fills every unblocked camp on the minute mark, with a roster other
    /// than the one it last put out.
    ///
    /// A camp with anything alive that walks, or a ward, standing in its box
    /// puts nothing out.
    pub fn fill_camps(&mut self) {
        if !neutral_spawn_due(self.tick) {
            return;
        }
        let box_radius = rules::units(rules::CAMP_BOX_RADIUS);
        let upgrades = jungle_upgrades(self.tick);
        for (index, camp) in self.map.camps.iter().enumerate() {
            if self.anything_in(camp.pos, box_radius) {
                continue;
            }
            let choices: Vec<(u8, &Roster)> = rosters_of(camp.kind).collect();
            let last = self.camp_last.get(index).copied().unwrap_or(u8::MAX);
            let fresh: Vec<&(u8, &Roster)> = choices.iter().filter(|(id, _)| *id != last).collect();
            if fresh.is_empty() {
                continue;
            }
            let taken = fresh[self
                .rng
                .global(Purpose::NeutralSpawn)
                .below(fresh.len() as u32) as usize];
            while self.camp_last.len() <= index {
                self.camp_last.push(u8::MAX);
            }
            self.camp_last[index] = taken.0;
            let creeps = taken.1.creeps;
            for (slot, kind) in creeps.iter().enumerate() {
                let across =
                    (slot as i32 - (creeps.len() as i32 - 1) / 2) * rules::CAMP_SPAWN_SPACING;
                let at = camp.pos + Vec2::from_ints(across, 0);
                self.spawn_neutral(kind.def(), at, index as u8, upgrades);
            }
        }
        self.settle();
    }

    /// Whether anything alive that walks, or a ward, stands inside a circle.
    fn anything_in(&self, at: Vec2, radius: Fixed) -> bool {
        self.entities.iter().any(|entity| {
            let blocks = self
                .stats
                .get(entity)
                .is_some_and(|stats| stats.move_speed > Fixed::ZERO)
                || self.kind.get(entity) == Some(&UnitKind::Ward);
            blocks
                && self.alive(entity)
                && self
                    .transform
                    .get(entity)
                    .is_some_and(|t| t.pos.within(at, radius))
        })
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
                self.transform.get(entity).map(|t| t.pos),
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
                let Some(their_at) = self.transform.get(target).map(|t| t.pos) else {
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
        let Some(at) = self.transform.get(entity).map(|t| t.pos) else {
            return false;
        };
        let reach = rules::units(range);
        let hostile = |other: Entity| {
            self.transform
                .get(other)
                .is_some_and(|t| t.pos.within(at, reach))
                && self.hostile(entity, other)
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
            let (Some(here), Some(there)) = (
                self.transform.get(blow.target).map(|t| t.pos),
                self.transform.get(by).map(|t| t.pos),
            ) else {
                continue;
            };
            if !here.within(there, rules::units(rules::NEUTRAL_DAMAGE_AGGRO_RANGE)) {
                continue;
            }
            for beast in self.entities.iter().collect::<Vec<_>>() {
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
