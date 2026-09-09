//! Channelling a scroll, and what happens when it runs out.

use bota_proto::{Target, UnitKind, Vec2};

use crate::game::rules;
use crate::game::{Entity, Teleport, World};

impl World {
    /// Runs every channelled teleport one tick on.
    ///
    /// One that runs out carries whoever was channelling it, leaves it
    /// standing where it lands, and spends the scroll that paid for it. One
    /// whose channeller has fallen or been held is dropped, and the scroll
    /// stays unspent.
    pub fn tick_teleports(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut going) = self.teleport.get(entity).copied() else {
                continue;
            };
            if !self.alive(entity) || self.held(entity) {
                self.teleport.remove(entity);
                continue;
            }
            going.ticks_left = going.ticks_left.saturating_sub(1);
            if going.ticks_left > 0 {
                self.teleport.insert(entity, going);
                continue;
            }
            self.teleport.remove(entity);
            if let Some(at) = self.transform.get_mut(entity) {
                at.pos = going.to;
            }
            // Whatever it was told to do before is left behind with the spot
            // it was told it in: it arrives standing.
            self.route.remove(entity);
            self.set_order(entity, crate::game::UnitOrder::Idle);
            self.spend_charge(entity, going.slot);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Whether a spot may be teleported to: walkable ground within reach of a
    /// building of one's own side that still stands.
    pub fn teleport_spot(&self, side: bota_proto::Team, to: Vec2, range: i32) -> bool {
        if !self.grid.walkable(to) {
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

    /// Starts a channel, if the spot aimed at is one that may be reached.
    pub fn begin_teleport(
        &mut self,
        entity: Entity,
        target: Target,
        channel: u32,
        range: i32,
        slot: usize,
    ) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(side) = self.team.get(entity).copied() else {
            return false;
        };
        if !self.teleport_spot(side, pos, range) {
            return false;
        }
        self.teleport.insert(
            entity,
            Teleport {
                ticks_left: channel.max(1),
                to: pos,
                slot,
            },
        );
        true
    }

    /// Whether an entity is standing through a channel right now.
    pub fn is_channelling(&self, entity: Entity) -> bool {
        self.teleport.get(entity).is_some() || self.dismember.get(entity).is_some()
    }
}
