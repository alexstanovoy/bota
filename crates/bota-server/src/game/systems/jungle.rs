//! The jungle: filling camps, and what a neutral does once drawn off one.

use bota_proto::{Team, Vec2};

use crate::game::{CampHome, Entity, NeutralAi, UnitOrder, Upgrades, World, rosters_of};
use crate::game::{Purpose, rules};

impl World {
    /// Fills every empty, unblocked camp on the minute mark.
    ///
    /// A camp with anything standing in its box puts nothing out, which is
    /// what makes blocking a camp work: anything that walks, and a ward,
    /// which walks nowhere but is put there for exactly this.
    pub fn fill_camps(&mut self) {
        if self.tick < rules::FIRST_NEUTRAL_TICK
            || !(self.tick - rules::FIRST_NEUTRAL_TICK)
                .is_multiple_of(rules::NEUTRAL_SPAWN_PERIOD_TICKS)
        {
            return;
        }
        let box_radius = rules::units(rules::CAMP_BOX_RADIUS);
        let upgrades = self.jungle_upgrades();
        for (index, camp) in self.map.camps.iter().enumerate() {
            if self.anything_in(camp.pos, box_radius) {
                continue;
            }
            let choices: Vec<(u8, &crate::game::Roster)> = rosters_of(camp.kind).collect();
            let last = self.camp_last.get(index).copied().unwrap_or(u8::MAX);
            let fresh: Vec<&(u8, &crate::game::Roster)> =
                choices.iter().filter(|(id, _)| *id != last).collect();
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
                let beast = self.spawn_unit(kind.def(), Team::Neutral, at);
                self.upgrades.insert(beast, Upgrades(upgrades));
                self.camp_home.insert(
                    beast,
                    CampHome {
                        camp: index as u8,
                        home: at,
                    },
                );
                self.neutral_ai.insert(
                    beast,
                    NeutralAi {
                        leash_left: rules::NEUTRAL_AGGRO_WINDOW,
                        reaggro_block: 0,
                        next_window: rules::NEUTRAL_AGGRO_WINDOW,
                        going_home: false,
                        roused_by: None,
                        awake: false,
                    },
                );
            }
        }
        self.settle();
    }

    /// How many upgrade intervals the jungle carries by now, capped.
    fn jungle_upgrades(&self) -> u32 {
        let periods = self.tick / rules::NEUTRAL_UPGRADE_PERIOD_TICKS;
        periods.min(rules::NEUTRAL_UPGRADE_CAP as u32)
    }

    /// Whether anything that walks stands inside a circle.
    fn anything_in(&self, at: Vec2, radius: bota_proto::Fixed) -> bool {
        self.entities.iter().any(|entity| {
            let blocks = self
                .stats
                .get(entity)
                .is_some_and(|stats| stats.move_speed > bota_proto::Fixed::ZERO)
                || self.kind.get(entity) == Some(&bota_proto::UnitKind::Ward);
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
    /// its block has run out.
    pub fn tick_jungle(&mut self) {
        let guard = rules::units(rules::NEUTRAL_GUARD_DISTANCE);
        let back = rules::units(rules::NEUTRAL_RETURN);
        let entities = self.take_entity_snapshot();
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
            // Asleep it wakes for what comes right up to it; from further
            // off nothing but a blow wakes it.
            if !ai.awake && !ai.going_home && ai.reaggro_block == 0 {
                ai.awake = self.anything_hostile_near(entity, rules::NEUTRAL_AGGRO_RANGE);
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
        self.recycle_entity_snapshot(entities);
    }
}

impl World {
    /// Whether anything this entity would fight stands within a reach of it.
    fn anything_hostile_near(&self, entity: Entity, range: i32) -> bool {
        let Some(at) = self.transform.get(entity).map(|t| t.pos) else {
            return false;
        };
        let reach = rules::units(range);
        self.entities.iter().any(|other| {
            self.hostile(entity, other)
                && self
                    .transform
                    .get(other)
                    .is_some_and(|t| t.pos.within(at, reach))
        })
    }

    /// Wakes every neutral of a camp onto whoever struck one of them.
    ///
    /// A camp answers as one: what is struck does not answer alone, and what
    /// is walking home is not called back by it. A blow carries further than
    /// eyes do and wakes them whether they can see who threw it or not.
    pub fn rouse_camps(&mut self, felt: &[crate::game::Landed]) {
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
