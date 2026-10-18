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

/// Owned units out of view: at most one a seat and kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BodyMemory {
    /// Sorted by id.
    bodies: Vec<Remembered>,
}

impl BodyMemory {
    /// Takes in a snapshot: refreshes every owned unit in view and drops the
    /// remembered bodies their seat has replaced.
    pub(crate) fn fold(&mut self, view: &WorldView) {
        let owned = || view.units.iter().filter(|unit| unit.owner.is_some());
        let mut shown: Vec<(Option<SlotId>, UnitKind)> =
            owned().map(|unit| (unit.owner, unit.kind)).collect();
        shown.sort_unstable();
        shown.dedup();
        self.bodies.retain(|held| {
            view.unit(held.unit.id).is_some()
                || shown
                    .binary_search(&(held.unit.owner, held.unit.kind))
                    .is_err()
        });
        // Sorted by id, as the view's units are, so each is found by search.
        self.bodies.sort_unstable_by_key(|held| held.unit.id);
        let known = self.bodies.len();
        for unit in owned() {
            match self.bodies[..known].binary_search_by_key(&unit.id, |held| held.unit.id) {
                Ok(at) => {
                    self.bodies[at].tick = view.tick;
                    copy_unit_view(&mut self.bodies[at].unit, unit);
                }
                Err(_) => self.bodies.push(Remembered {
                    tick: view.tick,
                    unit: unit.clone(),
                }),
            }
        }
        self.bodies.sort_unstable_by_key(|held| held.unit.id);
        self.keep_latest_unseen(view);
    }

    /// Of the bodies out of view, keeps one a seat and kind: the one seen
    /// last. A stream that shows a seat many bodies of a kind, then none,
    /// would otherwise leave every one of them remembered.
    fn keep_latest_unseen(&mut self, view: &WorldView) {
        let mut unseen: Vec<usize> = (0..self.bodies.len())
            .filter(|&at| view.unit(self.bodies[at].unit.id).is_none())
            .collect();
        let key = |at: &usize| {
            let held = &self.bodies[*at];
            (held.unit.owner, held.unit.kind, held.tick, held.unit.id)
        };
        unseen.sort_unstable_by_key(key);
        let mut stale = vec![false; self.bodies.len()];
        for pair in unseen.windows(2) {
            let (older, newer) = (&self.bodies[pair[0]].unit, &self.bodies[pair[1]].unit);
            stale[pair[0]] = older.owner == newer.owner && older.kind == newer.kind;
        }
        let mut at = 0;
        self.bodies.retain(|_| {
            at += 1;
            !stale[at - 1]
        });
    }

    pub(crate) fn get(&self, id: EntityId) -> Option<&Remembered> {
        let at = self
            .bodies
            .binary_search_by_key(&id, |held| held.unit.id)
            .ok()?;
        self.bodies.get(at)
    }

    /// The hero body last kept for a seat.
    pub(crate) fn hero_of(&self, slot: SlotId) -> Option<EntityId> {
        self.bodies
            .iter()
            .find(|held| held.unit.owner == Some(slot) && held.unit.kind == UnitKind::Hero)
            .map(|held| held.unit.id)
    }
}
