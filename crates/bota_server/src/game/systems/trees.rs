//! The forest through a match: a tree brought down and one grown back, each
//! told to the sides that see its spot.

use bota_core::rules;
use bota_proto::{EventKind, Team, Vec2};

use crate::game::{
    BodyIndex, Entity, Event, EventVisibility, Table, Transform, Tree, World, wire_id,
};

impl World {
    /// Brings a standing tree down by a unit's hand: what it stopped is
    /// stopped no longer, and the sides that see its spot are told.
    pub fn fell_tree(&mut self, tree: Tree, by: Entity) {
        let Some(at) = self.trees.spot(tree) else {
            return;
        };
        self.trees.fell(tree, self.tick);
        self.lay_sight_block();
        self.lay_passability();
        let side = self.team.get(by).copied().unwrap_or(Team::Neutral);
        self.events.push(Event {
            kind: EventKind::TreeFelled {
                at,
                by: Some(wire_id(by)),
            },
            visible_to: self.who_sees_spot(at, side),
            gold_hidden_from: None,
            source_told_to: self.who_may_name(Some(by)),
        });
    }

    /// The forest's tick: the map's trees whose time is up grow back where
    /// no unit stands within [`rules::TREE_REGROW_CLEAR`], and the trees put
    /// up whose time is up go, each told to the sides that see its spot.
    pub(crate) fn tick_forest(&mut self) {
        let (bodies, transform) = (&self.bodies, &self.transform);
        let change = self
            .trees
            .tick(self.tick, |at| someone_stands_by(bodies, transform, at));
        if !change.any() {
            return;
        }
        self.lay_sight_block();
        self.lay_passability();
        let spots = self.trees.spots();
        let told = change
            .grown
            .iter()
            .map(|&index| EventKind::TreeGrew {
                at: spots[index as usize],
            })
            .chain(
                change
                    .gone
                    .iter()
                    .map(|&at| EventKind::TreeFelled { at, by: None }),
            )
            .collect::<Vec<_>>();
        for kind in told {
            let (EventKind::TreeGrew { at } | EventKind::TreeFelled { at, .. }) = kind else {
                continue;
            };
            self.events.push(Event {
                visible_to: self.who_sees_spot(at, Team::Neutral),
                kind,
                gold_hidden_from: None,
                source_told_to: EventVisibility::Everyone,
            });
        }
    }
}

/// Whether a unit stands within [`rules::TREE_REGROW_CLEAR`] of a spot.
fn someone_stands_by(bodies: &BodyIndex, transform: &Table<Transform>, at: Vec2) -> bool {
    let reach = rules::units(rules::TREE_REGROW_CLEAR);
    let mut found = false;
    bodies.near(at, reach, |body| {
        found |= transform
            .get(body.entity)
            .is_some_and(|placed| placed.pos.within(at, reach));
    });
    found
}
