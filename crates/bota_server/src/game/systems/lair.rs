//! Neutral bosses read their homes and respawn window from the map's rows.
use crate::game::{Entity, Lair, Purpose, UnitOrder, World};
use bota_core::rules;
use bota_proto::EntityId;

impl World {
    pub(crate) fn stand_up_lairs(&mut self) {
        assert!(
            self.map.lairs.len() <= bota_proto::limits::MAX_LAIRS,
            "map lair definitions exceed the wire bound"
        );
        for def in self.map.lairs {
            if !self.objectives.has(def.objective) {
                continue;
            }
            let unit = self.spawn_neutral_home(def.neutral, def.day, None, 0);
            self.lairs.push(Lair {
                unit: Some(unit),
                falls: 0,
                last_killer: None,
                respawn_at: 0,
                home: def.day,
                travelling: false,
                journey: None,
                fell_at: None,
                known_alive: [true, true],
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
            let home = def.home_at(clock, self.schedule());
            if let Some(unit) = self.lairs[index].unit {
                if home != self.lairs[index].home {
                    self.lairs[index].home = home;
                    self.lairs[index].travelling = true;
                    self.begin_journey(unit, def.moving);
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
                if self.lairs[index].travelling
                    && self.pos(unit).is_some_and(|at| {
                        at.within(home, rules::units(def.neutral.neutral().home.back))
                    })
                {
                    self.lairs[index].travelling = false;
                    self.end_journey(unit);
                }
            } else if clock >= self.lairs[index].respawn_at {
                self.reclaim_lair_items(def);
                let unit = self.spawn_neutral_home(def.neutral, home, None, 0);
                self.lairs[index].unit = Some(unit);
                self.lairs[index].home = home;
                self.lairs[index].travelling = false;
                self.lairs[index].journey = None;
            }
        }
    }

    pub(crate) fn migrating(&self, unit: Entity) -> bool {
        self.lairs
            .iter()
            .any(|lair| lair.unit == Some(unit) && lair.travelling)
    }

    pub(crate) fn bury_lair(&mut self, unit: Entity, killer: Option<Entity>) {
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
        let team = killer
            .and_then(|killer| self.team.get(killer).copied())
            .filter(|team| *team != bota_proto::Team::Neutral);
        self.lairs[index].last_killer = team;
        self.lairs[index].fell_at = Some(self.clock());
        self.lairs[index].known_alive = [false, false];
        self.events.push(crate::game::Event::Fact {
            kind: bota_proto::EventKind::BossFell {
                objective: def.objective,
                team,
            },
            visible_to: crate::game::EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: crate::game::EventVisibility::Everyone,
        });
        let falls = self.lairs[index].falls.saturating_add(1);
        if let Some(at) = self.pos(unit) {
            for drop in def.drops {
                if falls >= drop.from
                    && self.catalogue.holds(drop.item)
                    && let Some(mut stack) = crate::game::ItemStack::dropped(drop.item)
                {
                    stack.level = (falls - drop.from + 1).min(u32::from(drop.max_level)) as u8;
                    self.lay_loot(stack, at);
                }
            }
        }
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
        self.lairs[index].journey = None;
        self.lairs[index].falls = falls;
        self.lairs[index].respawn_at = self.clock().saturating_add(delay * rules::TICKS_PER_SECOND);
    }
    fn reclaim_lair_items(&mut self, def: &bota_core::LairDef) {
        let mut recovered = Vec::new();
        for unit in self.entities.iter() {
            if let Some(bag) = self.inventory.get_mut(unit) {
                for slot in &mut bag.slots {
                    if let Some(stack) = *slot
                        && def
                            .drops
                            .iter()
                            .any(|drop| drop.reclaimed && drop.item == stack.id)
                    {
                        recovered.push((unit, stack.id));
                        *slot = None;
                    }
                }
            }
        }
        for seat in &mut self.seats {
            seat.stash.slots.iter_mut().for_each(|slot| {
                if slot.is_some_and(|stack| {
                    def.drops
                        .iter()
                        .any(|drop| drop.reclaimed && drop.item == stack.id)
                }) {
                    *slot = None;
                }
            });
        }
        let ground: Vec<_> = self
            .entities
            .iter()
            .filter(|unit| {
                self.loot.get(*unit).is_some_and(|loot| {
                    def.drops
                        .iter()
                        .any(|drop| drop.reclaimed && drop.item == loot.0.id)
                })
            })
            .collect();
        for unit in ground {
            self.despawn(unit);
        }
        for (unit, item) in recovered {
            self.recover_expired_item(unit, item);
        }
    }
}

impl World {
    /// Sightings are observed for both teams every tick, independent of requested views.
    pub(crate) fn note_lair_sightings(&mut self) {
        for index in 0..self.lairs.len() {
            let Some(unit) = self.lairs[index].unit.filter(|u| self.alive(*u)) else {
                continue;
            };
            let observed = [
                self.shows(bota_proto::Team::Radiant, unit),
                self.shows(bota_proto::Team::Dire, unit),
            ];
            for (known, seen) in self.lairs[index].known_alive.iter_mut().zip(observed) {
                *known |= seen;
            }
        }
    }
    pub(crate) fn project_lairs(
        &self,
        viewer: Option<bota_proto::Team>,
    ) -> Vec<bota_proto::LairView> {
        let definitions = self
            .map
            .lairs
            .iter()
            .enumerate()
            .filter(|(_, def)| self.objectives.has(def.objective));
        debug_assert_eq!(
            self.lairs.len(),
            definitions.clone().count(),
            "enabled lairs match their definitions"
        );
        assert!(self.lairs.len() <= bota_proto::limits::MAX_LAIRS);
        definitions
            .zip(&self.lairs)
            .map(|((index, _), lair)| bota_proto::LairView {
                lair: u8::try_from(index).expect("bounded map lair index"),
                fell_at: lair.fell_at,
                alive_known_by: bota_proto::SeenBy {
                    radiant: lair.known_alive[0]
                        && viewer.is_none_or(|t| t == bota_proto::Team::Radiant),
                    dire: lair.known_alive[1] && viewer.is_none_or(|t| t == bota_proto::Team::Dire),
                },
            })
            .collect()
    }
}
