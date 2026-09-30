//! The forest as it stands right now: what is down, and what has been put up.
//!
//! The map's own trees are named by their place in [`tree_positions`], which
//! never changes; a tree put up during a match is named by its place in
//! [`Forest::planted`], which is compacted only when one goes.
//!
//! [`tree_positions`]: crate::game::tree_positions

use bota_proto::Vec2;

use crate::game::rules;

/// One tree, whichever kind it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tree {
    /// One of the map's own, by its place in the map's list.
    Rooted(usize),
    /// One put up during the match, by its place in the standing list.
    Planted(usize),
}

/// A tree put up during the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Planted {
    /// Where it stands.
    pub at: Vec2,
    /// The tick it goes on its own.
    pub until: u32,
}

/// Every tree of a match and what has become of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forest {
    /// The tick each of the map's trees comes back. Zero for one standing.
    back: Vec<u32>,
    /// Every one of the map's trees that is down, by index, ascending.
    down: Vec<u32>,
    /// What has been put up and not yet gone.
    planted: Vec<Planted>,
}

impl Forest {
    /// A map's forest with every tree of it standing.
    pub fn of(map: &'static crate::game::MapDef) -> Forest {
        Forest {
            back: vec![0; crate::game::tree_positions(map).len()],
            down: Vec::new(),
            planted: Vec::new(),
        }
    }

    /// Whether one of the map's trees is standing.
    pub fn rooted_stands(&self, index: usize) -> bool {
        self.back.get(index).is_some_and(|back| *back == 0)
    }

    /// Every one of the map's trees that is down, in order.
    pub fn felled(&self) -> impl Iterator<Item = u32> + '_ {
        self.down.iter().copied()
    }

    /// Every tree put up and still standing, in order.
    pub fn planted(&self) -> &[Planted] {
        &self.planted
    }

    /// Where a tree stands, or nothing if that tree does not.
    pub fn spot(&self, map: &'static crate::game::MapDef, tree: Tree) -> Option<Vec2> {
        match tree {
            Tree::Rooted(index) => self
                .rooted_stands(index)
                .then(|| crate::game::tree_positions(map).get(index).copied())
                .flatten(),
            Tree::Planted(index) => self.planted.get(index).map(|tree| tree.at),
        }
    }

    /// The standing tree nearest a spot, within a reach of it. One put up is
    /// taken before one of the map's own at the same distance.
    pub fn nearest(
        &self,
        map: &'static crate::game::MapDef,
        at: Vec2,
        reach: bota_proto::Fixed,
    ) -> Option<Tree> {
        let mut best: Option<(i64, Tree)> = None;
        for (index, tree) in self.planted.iter().enumerate() {
            let far = at.distance_squared(tree.at);
            if tree.at.within(at, reach) && best.is_none_or(|(b, _)| far < b) {
                best = Some((far, Tree::Planted(index)));
            }
        }
        for (index, spot) in crate::game::tree_positions(map).into_iter().enumerate() {
            if !self.rooted_stands(index) || !spot.within(at, reach) {
                continue;
            }
            let far = at.distance_squared(spot);
            if best.is_none_or(|(b, _)| far < b) {
                best = Some((far, Tree::Rooted(index)));
            }
        }
        best.map(|(_, tree)| tree)
    }

    /// Takes a tree down. One of the map's own comes back in its own time; one
    /// put up does not come back at all.
    pub fn fell(&mut self, tree: Tree, now: u32) {
        match tree {
            Tree::Rooted(index) => {
                if let Some(back) = self.back.get_mut(index) {
                    *back = now + rules::TREE_REGROW_TICKS;
                    if let Err(at) = self.down.binary_search(&(index as u32)) {
                        self.down.insert(at, index as u32);
                    }
                }
            }
            Tree::Planted(index) => {
                if index < self.planted.len() {
                    self.planted.remove(index);
                }
            }
        }
    }

    /// Puts a tree up at a spot until a tick.
    pub fn plant(&mut self, at: Vec2, until: u32) {
        self.planted.push(Planted { at, until });
    }

    /// Brings back what has waited out its time and takes away what has run
    /// out of it. Answers whether anything changed.
    pub fn tick(&mut self, now: u32) -> bool {
        let before = self.down.len();
        let back = &mut self.back;
        self.down.retain(|&index| {
            let back = &mut back[index as usize];
            if now >= *back {
                *back = 0;
            }
            *back > 0
        });
        let moved = self.down.len() != before;
        let before = self.planted.len();
        self.planted.retain(|tree| now < tree.until);
        moved || self.planted.len() != before
    }
}
