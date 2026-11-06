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
        self.events.push(Event::Fact {
            kind: EventKind::TreeFelled {
                at,
                by: Some(wire_id(by)),
            },
            visible_to: self.who_sees_spot(at, side),
            gold_hidden_from: None,
            source_told_to: self.who_may_name(Some(by)),
        });
    }

    /// Fells a capsule of trees in one passability/vision update.
    pub(crate) fn fell_trees_along(&mut self, from: Vec2, to: Vec2, radius: i32, by: Entity) {
        let middle = Vec2 {
            x: bota_proto::Fixed {
                raw: ((i64::from(from.x.raw) + i64::from(to.x.raw)) / 2) as i32,
            },
            y: bota_proto::Fixed {
                raw: ((i64::from(from.y.raw) + i64::from(to.y.raw)) / 2) as i32,
            },
        };
        let pad = i64::from(rules::units(radius).raw);
        let reach = pad
            + (i64::from(to.x.raw) - i64::from(from.x.raw))
                .abs()
                .max((i64::from(to.y.raw) - i64::from(from.y.raw)).abs())
                / 2
            + 1;
        let struck = |at| bota_core::segment_distance_squared(at, from, to) <= pad * pad;
        let mut trees: Vec<(Tree, Vec2)> = self
            .trees
            .rooted_near(middle, reach)
            .filter_map(|i| {
                let at = self.trees.spots()[i];
                struck(at).then_some((Tree::Rooted(i), at))
            })
            .collect();
        trees.extend(
            self.trees
                .planted()
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, tree)| struck(tree.at).then_some((Tree::Planted(i), tree.at))),
        );
        if trees.is_empty() {
            return;
        }
        let side = self.team.get(by).copied().unwrap_or(Team::Neutral);
        for (tree, at) in trees {
            self.trees.fell(tree, self.tick);
            self.events.push(Event::Fact {
                kind: EventKind::TreeFelled {
                    at,
                    by: Some(wire_id(by)),
                },
                visible_to: self.who_sees_spot(at, side),
                gold_hidden_from: None,
                source_told_to: self.who_may_name(Some(by)),
            });
        }
        self.lay_sight_block();
        self.lay_passability();
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
            self.events.push(Event::Fact {
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
