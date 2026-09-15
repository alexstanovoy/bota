//! Teleporting: where a scroll may carry, and carrying.

use bota_proto::{Target, UnitKind, Vec2};

use crate::game::rules;
use crate::game::{Entity, World};

impl World {
    /// Whether a spot may be teleported to: walkable ground within reach of a
    /// building of one's own side that still stands.
    pub fn teleport_spot(&self, side: bota_proto::Team, to: Vec2, range: i32) -> bool {
        if !self.grid.stands_clear(to) {
            return false;
        }
        let reach = rules::units(range);
        self.entities.iter().any(|entity| {
            matches!(
                self.kind.get(entity),
                Some(UnitKind::Tower | UnitKind::Ancient | UnitKind::Fountain)
            ) && self.team.get(entity).copied() == Some(side)
                && self.alive(entity)
                && self
                    .transform
                    .get(entity)
                    .is_some_and(|at| at.pos.within(to, reach))
        })
    }

    /// Whether an entity may read a scroll at a spot.
    pub fn may_teleport(&self, entity: Entity, target: Target, range: i32) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(side) = self.team.get(entity).copied() else {
            return false;
        };
        self.teleport_spot(side, pos, range)
    }

    /// Carries an entity to the spot its scroll was read at, leaves it
    /// standing there, and spends the scroll in the slot.
    ///
    /// Whatever it was told to do before is left behind with the spot it was
    /// told it in: it arrives standing.
    pub fn teleport_to(&mut self, entity: Entity, slot: usize, target: Target) {
        let Target::Pos(pos) = target else {
            return;
        };
        if let Some(at) = self.transform.get_mut(entity) {
            at.pos = pos;
        }
        self.route.remove(entity);
        self.set_order(entity, crate::game::UnitOrder::Idle);
        self.spend_charge(entity, slot);
    }
}
