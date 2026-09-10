//! Where the trees stand.
//!
//! The map's own forest is sent once, when the match begins; which of those
//! trees are down and which have been put up during the match ride in every
//! snapshot. Both are needed to eat a tango, which asks for a tree by the
//! ground it stands on.

use bota_proto::{MatchInfo, Vec2, WorldView};

use crate::{order_by, span};

/// The map's forest, as the match began.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forest {
    /// Every tree the map put up, in the order the match named them.
    pub trees: Vec<Vec2>,
}

impl Forest {
    /// The forest a match started with.
    pub fn of(info: &MatchInfo) -> Forest {
        Forest {
            trees: info.trees.clone(),
        }
    }

    /// The nearest tree still standing within a reach of a spot.
    pub fn nearest_standing(&self, view: &WorldView, to: Vec2, within: f32) -> Option<Vec2> {
        self.nearest_kept(view, to, within, |_| true)
    }

    /// The same, of the trees a filter keeps.
    ///
    /// Used to leave out the ones a hero would have to walk the wrong way up
    /// the lane to reach.
    pub fn nearest_kept(
        &self,
        view: &WorldView,
        to: Vec2,
        within: f32,
        keep: impl Fn(Vec2) -> bool,
    ) -> Option<Vec2> {
        self.trees
            .iter()
            .enumerate()
            .filter(|(at, _)| !view.felled_trees.contains(&(*at as u32)))
            .map(|(_, pos)| *pos)
            .chain(view.planted_trees.iter().copied())
            .filter(|pos| span(*pos, to) <= within)
            .filter(|pos| keep(*pos))
            .min_by(|one, other| order_by(span(*one, to), span(*other, to)))
    }
}
