//! When each projectile and each item on the ground was first seen.

use bota_proto::{EntityId, LootView, ProjectileView, Vec2};

/// A projectile in flight, with where and when it was before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissileTrack {
    pub id: EntityId,
    pub first_tick: u32,
    pub last_tick: u32,
    /// The snapshot before this one that held it; absent on its first.
    pub previous_tick: Option<u32>,
    pub position: Vec2,
    pub previous_position: Vec2,
}

/// An item lying on the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootTrack {
    pub id: EntityId,
    pub first_tick: u32,
    pub last_tick: u32,
}

/// Live projectiles and loot, sorted by handle, with a spare for the next
/// snapshot so no tick allocates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Missiles {
    missiles: Vec<MissileTrack>,
    loot: Vec<LootTrack>,
    next_missiles: Vec<MissileTrack>,
    next_loot: Vec<LootTrack>,
}

impl Missiles {
    pub(crate) fn missiles(&self) -> &[MissileTrack] {
        &self.missiles
    }

    pub(crate) fn loot(&self) -> &[LootTrack] {
        &self.loot
    }

    /// Replaces both lists with what the snapshot holds. The views need not
    /// be sorted: a client may have reordered them.
    pub(crate) fn fold(&mut self, tick: u32, projectiles: &[ProjectileView], loot: &[LootView]) {
        self.next_missiles.clear();
        for projectile in projectiles {
            let prior = self
                .missiles
                .binary_search_by_key(&projectile.id, |entry| entry.id)
                .ok()
                .map(|index| self.missiles[index]);
            self.next_missiles.push(match prior {
                Some(entry) => MissileTrack {
                    id: projectile.id,
                    first_tick: entry.first_tick,
                    last_tick: tick,
                    previous_tick: Some(entry.last_tick),
                    position: projectile.pos,
                    previous_position: entry.position,
                },
                None => MissileTrack {
                    id: projectile.id,
                    first_tick: tick,
                    last_tick: tick,
                    previous_tick: None,
                    position: projectile.pos,
                    previous_position: projectile.pos,
                },
            });
        }
        self.next_missiles.sort_unstable_by_key(|entry| entry.id);
        std::mem::swap(&mut self.missiles, &mut self.next_missiles);
        self.next_loot.clear();
        for item in loot {
            let first_tick = self
                .loot
                .binary_search_by_key(&item.id, |entry| entry.id)
                .map_or(tick, |index| self.loot[index].first_tick);
            self.next_loot.push(LootTrack {
                id: item.id,
                first_tick,
                last_tick: tick,
            });
        }
        self.next_loot.sort_unstable_by_key(|entry| entry.id);
        std::mem::swap(&mut self.loot, &mut self.next_loot);
    }
}
