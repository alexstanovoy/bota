//! The forest as it stands right now: what is down, and what has been put up.
//!
//! The map's own trees are named by their place in [`tree_positions`], which
//! never changes; a tree put up during a match is named by its place in
//! [`Forest::planted`], which is compacted only when one goes.
//!
//! Each side also keeps the forest as it last saw it: since 2014 Dota shows
//! no tree falling, growing back or put up where a side has no sight, so a
//! side's view of the forest changes only where it looks.
//!
//! [`tree_positions`]: crate::game::tree_positions

use std::sync::Arc;

use bota_core::rules;
use bota_proto::{Team, Vec2};

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
    pub at: Vec2,
    /// The tick it goes on its own.
    pub until: u32,
}

/// Every tree of a match and what has become of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Forest {
    /// Where each of the map's trees stands, by index.
    spots: Arc<[Vec2]>,
    /// The tick each of the map's trees comes back. Zero for one standing.
    back: Vec<u32>,
    /// Every one of the map's trees that is down, by index, ascending.
    down: Vec<u32>,
    /// What has been put up and not yet gone.
    planted: Vec<Planted>,
    /// The forest as each side last saw it: Radiant's, then Dire's.
    known: [KnownForest; 2],
}

/// The forest as one side last saw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownForest {
    /// The map's trees it saw down and has not seen stand again, by index,
    /// ascending.
    pub down: Vec<u32>,
    /// Where the trees it saw put up stand, until it sees them gone, in the
    /// order it first saw them.
    pub planted: Vec<Vec2>,
}

impl Forest {
    /// A map's forest with every tree of it standing, as both sides know.
    pub fn of(map: &bota_core::MapDef, terrain: &crate::game::MapTerrain) -> Forest {
        let spots: Arc<[Vec2]> = crate::game::tree_positions(map, terrain).into();
        Forest {
            back: vec![0; spots.len()],
            spots,
            down: Vec::new(),
            planted: Vec::new(),
            known: Default::default(),
        }
    }

    /// Where each of the map's own trees stands, by index.
    pub fn spots(&self) -> &[Vec2] {
        &self.spots
    }

    /// The forest as a side last saw it; nothing for the jungle, which is
    /// shown no forest.
    pub fn known(&self, team: Team) -> Option<&KnownForest> {
        bota_core::side_index(team).map(|side| &self.known[side])
    }

    /// Brings what a side knows of the forest in line with what stands,
    /// for every tree whose ground it sees: `sees_rooted` asks of a map
    /// tree's spot, `sees_planted` of a planted one's.
    pub fn learn(
        &mut self,
        team: Team,
        sees_rooted: impl Fn(Vec2) -> bool,
        sees_planted: impl Fn(Vec2) -> bool,
    ) {
        let Some(side) = bota_core::side_index(team) else {
            return;
        };
        let changed = differing(&self.down, &self.known[side].down)
            .filter(|&(index, _)| sees_rooted(self.spots[index as usize]))
            .collect::<Vec<_>>();
        let known = &mut self.known[side];
        for (index, down) in changed {
            match (known.down.binary_search(&index), down) {
                (Err(at), true) => known.down.insert(at, index),
                (Ok(at), false) => {
                    known.down.remove(at);
                }
                _ => {}
            }
        }
        let planted = &self.planted;
        known
            .planted
            .retain(|at| planted.iter().any(|tree| tree.at == *at) || !sees_planted(*at));
        for tree in planted {
            if !known.planted.contains(&tree.at) && sees_planted(tree.at) {
                known.planted.push(tree.at);
            }
        }
    }

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
    pub fn spot(&self, tree: Tree) -> Option<Vec2> {
        match tree {
            Tree::Rooted(index) => self.rooted_stands(index).then(|| self.spots[index]),
            Tree::Planted(index) => self.planted.get(index).map(|tree| tree.at),
        }
    }

    /// The standing tree nearest a spot, within a reach of it. One put up is
    /// taken before one of the map's own at the same distance.
    pub fn nearest(&self, at: Vec2, reach: bota_proto::Fixed) -> Option<Tree> {
        let mut best: Option<(i64, Tree)> = None;
        for (index, tree) in self.planted.iter().enumerate() {
            let far = at.distance_squared(tree.at);
            if tree.at.within(at, reach) && best.is_none_or(|(b, _)| far < b) {
                best = Some((far, Tree::Planted(index)));
            }
        }
        for (index, &spot) in self.spots.iter().enumerate() {
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

/// Every map tree on which what stands and what a side knows differ, with
/// whether it is down: the indices in one of two ascending lists and not
/// the other, ascending.
fn differing<'a>(truth: &'a [u32], known: &'a [u32]) -> impl Iterator<Item = (u32, bool)> + 'a {
    let (mut t, mut k) = (0, 0);
    std::iter::from_fn(move || {
        loop {
            match (truth.get(t), known.get(k)) {
                (Some(&a), Some(&b)) if a == b => {
                    t += 1;
                    k += 1;
                }
                (Some(&a), Some(&b)) if a < b => {
                    t += 1;
                    return Some((a, true));
                }
                (Some(_), Some(&b)) | (None, Some(&b)) => {
                    k += 1;
                    return Some((b, false));
                }
                (Some(&a), None) => {
                    t += 1;
                    return Some((a, true));
                }
                (None, None) => return None,
            }
        }
    })
}
