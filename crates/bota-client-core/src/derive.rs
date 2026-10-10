//! What successive snapshots tell that one alone does not: when a cast
//! point began, how long a wait of a slot was when it began, and how much
//! health a unit had a moment ago.

use bota_proto::{EntityId, StatusFlags, WorldView};

/// Units tracked at most per table; past it the oldest entries give way.
const MAX_TRACKED: usize = 1024;

/// One timed wait of a unit's slot, as first seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wait {
    unit: EntityId,
    /// The slot: an ability below 64, an item at 64 and above.
    slot: u8,
    /// Ticks the wait was at when first seen.
    total: u32,
    /// Ticks it was at last time.
    last: u32,
}

/// The facts derived from successive snapshots.
#[derive(Clone, Debug, Default)]
pub struct Derived {
    /// When each unit in the cast point of something began it, by tick.
    casts: Vec<(EntityId, u32)>,
    /// The full length of every wait of a slot under way.
    waits: Vec<Wait>,
    /// Health each unit is drawn as having a moment ago: it trails the real
    /// health down.
    ghosts: Vec<(EntityId, f32)>,
}

/// The slot number [`Derived::wait_left`] takes for item slot `slot`.
pub const ITEM_WAIT_BASE: u8 = 64;

impl Derived {
    /// Takes in a new snapshot.
    pub fn snapshot(&mut self, view: &WorldView) {
        self.note_casts(view);
        self.note_waits(view);
    }

    fn note_casts(&mut self, view: &WorldView) {
        let casting = |id: &EntityId| {
            view.units
                .binary_search_by_key(id, |u| u.id)
                .is_ok_and(|at| view.units[at].statuses.bits & StatusFlags::CASTING != 0)
        };
        self.casts.retain(|(id, _)| casting(id));
        for unit in &view.units {
            if unit.statuses.bits & StatusFlags::CASTING != 0
                && !self.casts.iter().any(|(id, _)| *id == unit.id)
                && self.casts.len() < MAX_TRACKED
            {
                self.casts.push((unit.id, view.tick));
            }
        }
    }

    fn note_waits(&mut self, view: &WorldView) {
        let mut seen = Vec::with_capacity(self.waits.len());
        for unit in view.units.iter().filter(|u| !u.abilities.is_empty()) {
            let abilities = unit.abilities.iter().map(|a| a.cooldown_left).enumerate();
            let items = unit
                .items
                .iter()
                .map(|item| item.map_or(0, |item| item.cooldown_left))
                .enumerate()
                .map(|(at, left)| (usize::from(ITEM_WAIT_BASE) + at, left));
            for (slot, left) in abilities.chain(items).take(128) {
                if left == 0 || seen.len() >= MAX_TRACKED {
                    continue;
                }
                let slot = slot as u8;
                let old = self
                    .waits
                    .iter()
                    .find(|w| w.unit == unit.id && w.slot == slot);
                let total = match old {
                    Some(w) if left <= w.last => w.total,
                    _ => left,
                };
                seen.push(Wait {
                    unit: unit.id,
                    slot,
                    total,
                    last: left,
                });
            }
        }
        self.waits = seen;
    }

    /// The tick a unit began the cast point it is in, if it is in one.
    pub fn cast_began(&self, unit: EntityId) -> Option<u32> {
        self.casts
            .iter()
            .find(|(id, _)| *id == unit)
            .map(|(_, t)| *t)
    }

    /// How much of a slot's wait is still to run, 0 to 1, given the ticks
    /// `left` on it now. `slot` counts ability slots from 0 and item slots
    /// from [`ITEM_WAIT_BASE`].
    pub fn wait_left(&self, unit: EntityId, slot: u8, left: u32) -> f32 {
        if left == 0 {
            return 0.0;
        }
        let total = self
            .waits
            .iter()
            .find(|w| w.unit == unit && w.slot == slot)
            .map_or(left, |w| w.total.max(left));
        left as f32 / total.max(1) as f32
    }

    /// Moves every trailing health towards the real one, `dt` seconds on.
    pub fn trail(&mut self, view: &WorldView, dt: f32) {
        self.ghosts
            .retain(|(id, _)| view.units.binary_search_by_key(id, |u| u.id).is_ok());
        for unit in &view.units {
            let hp = unit.hp.max(0) as f32;
            let full = self.ghosts.len() >= MAX_TRACKED;
            match self.ghosts.iter_mut().find(|(id, _)| *id == unit.id) {
                Some((_, ghost)) if *ghost > hp => {
                    *ghost += (hp - *ghost) * (dt * 3.5).min(1.0);
                }
                Some((_, ghost)) => *ghost = hp,
                None if !full => self.ghosts.push((unit.id, hp)),
                None => {}
            }
        }
    }

    /// The trailing health of a unit, at least its real health.
    pub fn ghost(&self, unit: EntityId, hp: i32) -> f32 {
        self.ghosts
            .iter()
            .find(|(id, _)| *id == unit)
            .map_or(hp as f32, |(_, g)| g.max(hp as f32))
    }
}
