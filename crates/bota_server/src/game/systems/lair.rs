//! Neutral bosses read their homes and respawn window from the map's rows.
use crate::game::{Entity, Lair, Purpose, UnitOrder, World};
use bota_core::{rules, time_of_day};
use bota_proto::EntityId;

impl World {
    pub(crate) fn stand_up_lairs(&mut self) {
        for def in self.map.lairs {
            if !self.objectives.has(def.objective) {
                continue;
            }
            let unit = self.spawn_neutral_home(def.neutral, def.day, None, 0);
            self.lairs.push(Lair {
                unit: Some(unit),
                falls: 0,
                respawn_at: 0,
                home: def.day,
                travelling: false,
            });
        }
    }

    pub(crate) fn tick_lairs(&mut self) {
        for index in 0..self.lairs.len() {
            let def = self
                .map
                .lairs
                .iter()
                .filter(|def| self.objectives.has(def.objective))
                .nth(index)
                .expect("lair row");
            let clock = self.clock();
            let night = clock >= self.schedule().horn + def.move_after * rules::TICKS_PER_SECOND
                && time_of_day(clock, self.schedule()).night;
            let home = if night { def.night } else { def.day };
            if let Some(unit) = self.lairs[index].unit {
                if home != self.lairs[index].home {
                    self.lairs[index].home = home;
                    self.lairs[index].travelling = true;
                    if let Some(held) = self.camp_home.get_mut(unit) {
                        held.home = home;
                    }
                    if let Some(ai) = self.neutral_ai.get_mut(unit) {
                        ai.going_home = true;
                        ai.awake = false;
                    }
                    self.target.remove(unit);
                    self.set_order(unit, UnitOrder::Move { pos: home });
                }
                if self.pos(unit).is_some_and(|at| {
                    at.within(home, rules::units(def.neutral.neutral().home.back))
                }) {
                    self.lairs[index].travelling = false;
                }
            } else if clock >= self.lairs[index].respawn_at {
                let unit = self.spawn_neutral_home(def.neutral, home, None, 0);
                self.lairs[index].unit = Some(unit);
                self.lairs[index].home = home;
                self.lairs[index].travelling = false;
            }
        }
    }

    pub(crate) fn migrating(&self, unit: Entity) -> bool {
        self.lairs
            .iter()
            .any(|lair| lair.unit == Some(unit) && lair.travelling)
    }

    pub(crate) fn bury_lair(&mut self, unit: Entity) {
        let Some(index) = self.lairs.iter().position(|lair| lair.unit == Some(unit)) else {
            return;
        };
        let def = self
            .map
            .lairs
            .iter()
            .filter(|def| self.objectives.has(def.objective))
            .nth(index)
            .expect("lair row");
        let falls = self.lairs[index].falls.saturating_add(1);
        let mut random = self.rng.for_unit(
            Purpose::Lair,
            EntityId {
                idx: falls,
                generation: 0,
            },
            index as u8,
        );
        let delay = def.respawn[0] + random.below(def.respawn[1] - def.respawn[0] + 1);
        self.lairs[index].unit = None;
        self.lairs[index].travelling = false;
        self.lairs[index].falls = falls;
        self.lairs[index].respawn_at = self.clock().saturating_add(delay * rules::TICKS_PER_SECOND);
    }
}
