//! Keeping guarded structures untouchable until what shields them falls.

use bota_proto::{Team, UnitKind};

use crate::engine::Entity;
use crate::game::{StructureId, Until, World, is_structure};

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

    /// Marks every structure the map still guards untouchable.
    ///
    /// Runs right after stats are derived: the guard is worn as
    /// invulnerability, so targeting, orders and blows all refuse it the
    /// same way they refuse anything invulnerable.
    pub fn guard_structures(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(kind) = self.kind.get(entity).copied() else {
                continue;
            };
            if !is_structure(kind) || self.structure_open(entity) {
                continue;
            }
            if let Some(stats) = self.stats.get_mut(entity) {
                stats.invulnerable = true;
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
