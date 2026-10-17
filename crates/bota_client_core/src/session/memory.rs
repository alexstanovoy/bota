//! The last body of every seat's hero and courier, kept for as long as the
//! seat has no other of the kind in view.

use bota_proto::{EntityId, SlotId, UnitKind, UnitView, WorldView};

use crate::session::tracks::copy_unit_view;

/// A unit as last seen, and the tick it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Remembered {
    pub tick: u32,
    pub unit: UnitView,
}

/// Owned units out of view: at most a hero and a courier a seat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BodyMemory {
    bodies: Vec<Remembered>,
}

impl BodyMemory {
    /// Takes in a snapshot: refreshes every owned unit in view and drops the
    /// remembered bodies their seat has replaced.
    pub(crate) fn fold(&mut self, view: &WorldView) {
        for unit in view.units.iter().filter(|unit| unit.owner.is_some()) {
            match self.bodies.iter_mut().find(|held| held.unit.id == unit.id) {
                Some(held) => {
                    held.tick = view.tick;
                    copy_unit_view(&mut held.unit, unit);
                }
                None => self.bodies.push(Remembered {
                    tick: view.tick,
                    unit: unit.clone(),
                }),
            }
        }
        self.bodies.retain(|held| {
            view.unit(held.unit.id).is_some()
                || !view.units.iter().any(|unit| {
                    unit.owner == held.unit.owner
                        && unit.kind == held.unit.kind
                        && unit.id != held.unit.id
                })
        });
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<&Remembered> {
        self.bodies.iter().find(|held| held.unit.id == id)
    }

    /// The hero body last kept for a seat.
    pub(crate) fn hero_of(&self, slot: SlotId) -> Option<EntityId> {
        self.bodies
            .iter()
            .find(|held| held.unit.owner == Some(slot) && held.unit.kind == UnitKind::Hero)
            .map(|held| held.unit.id)
    }
}
