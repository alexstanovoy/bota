//! Keeping guarded structures untouchable until what shields them falls.

use bota_core::{ModifierKind, StructureId, Until, kind_def};
use bota_proto::{Team, UnitKind};

use crate::game::{Modifier, World};

use crate::engine::Entity;

impl World {
    /// Whether an entity is one a structure id names.
    fn names_structure(&self, id: StructureId, entity: Entity) -> bool {
        match id {
            StructureId::Tower { lane, tier } => {
                self.kind.get(entity) == Some(&UnitKind::Tower)
                    && self.lane.get(entity).map(|l| l.0) == Some(lane)
                    && self.tier.get(entity).map(|t| t.0) == Some(tier)
            }
            StructureId::Barracks { lane, ranged } => {
                self.kind.get(entity) == Some(&UnitKind::Barracks)
                    && self.lane.get(entity).map(|l| l.0) == Some(lane)
                    && self.rax.get(entity).map(|r| r.ranged) == Some(ranged)
            }
            StructureId::Filler => self.kind.get(entity) == Some(&UnitKind::Filler),
            StructureId::Ancient => self.kind.get(entity) == Some(&UnitKind::Ancient),
        }
    }

    /// Whether nothing a side has left standing answers to an id.
    pub fn all_down(&self, side: Team, id: StructureId) -> bool {
        !self.entities.iter().any(|entity| {
            self.team.get(entity).copied() == Some(side)
                && self.alive(entity)
                && self.names_structure(id, entity)
        })
    }

    /// Whether a side has lost enough for a condition to hold.
    fn condition_met(&self, side: Team, until: &Until) -> bool {
        match until {
            Until::Down(id) => self.all_down(side, *id),
            Until::AnyOf(each) => each.iter().any(|one| self.condition_met(side, one)),
            Until::AllOf(each) => each.iter().all(|one| self.condition_met(side, one)),
            Until::Never => false,
        }
    }

    /// Whether a structure may be struck yet.
    ///
    /// Every rule naming it must be satisfied; one no rule names is open
    /// from the first tick.
    pub fn structure_open(&self, entity: Entity) -> bool {
        let Some(side) = self.team.get(entity).copied() else {
            return true;
        };
        self.map
            .protection
            .iter()
            .filter(|rule| self.names_structure(rule.guarded, entity))
            .all(|rule| self.condition_met(side, &rule.until))
    }

    /// Puts on each Outpost its capture guard while it holds, and takes it
    /// off once it no longer does: while its home side holds it and every
    /// tier-two tower of that side stands ([`bota_core::OUTPOST_OPENS`]).
    pub(crate) fn wear_capture_guards(&mut self) {
        if !self.objectives.has(bota_proto::ObjectiveKind::Outposts) {
            return;
        }
        let outposts: Vec<(Entity, Team)> = self
            .entities
            .iter()
            .filter_map(|entity| Some((entity, self.outpost.get(entity)?.home)))
            .collect();
        for (outpost, home) in outposts {
            let holds = self.team.get(outpost) == Some(&home)
                && !self.condition_met(home, &bota_core::OUTPOST_OPENS);
            let worn = self.modifiers.get(outpost).is_some_and(|on_it| {
                on_it
                    .active()
                    .any(|held| held.kind == ModifierKind::CaptureGuard)
            });
            if holds && !worn {
                self.put_modifier(
                    outpost,
                    Modifier::new(ModifierKind::CaptureGuard, None, None, None),
                );
            } else if !holds && worn {
                self.take_modifier(outpost, ModifierKind::CaptureGuard, None);
            }
        }
    }

    /// Marks every structure the map still guards untouchable.
    ///
    /// Runs right after stats are derived; the guard is worn as
    /// [`Stats::invulnerable`](crate::game::Stats::invulnerable).
    pub fn guard_structures(&mut self) {
        for entity in self.entities.iter() {
            let Some(kind) = self.kind.get(entity).copied() else {
                continue;
            };
            if !kind_def(kind).structure || self.structure_open(entity) {
                continue;
            }
            if let Some(stats) = self.stats.get_mut(entity) {
                stats.invulnerable = true;
            }
        }
    }

    /// Makes untouchable every hero whose seat a cheat made invulnerable.
    pub fn guard_cheated_heroes(&mut self) {
        for index in 0..self.seats.len() {
            let seat = &self.seats[index];
            if !seat.invulnerable {
                continue;
            }
            if let Some(stats) = seat.unit.and_then(|unit| self.stats.get_mut(unit)) {
                stats.invulnerable = true;
            }
        }
    }
}
