//! What a side sees, worked out as the server works it out.
//!
//! The server tells a client only what its side sees; this lays the same
//! rule over the ground to say which cells those are, to shade the rest.
//! The rule is [`bota_core::SightBlock`]: the ground, the map's fog blocker
//! walls and the trees standing now, which a snapshot tells by the map's
//! felled trees and the planted ones.

use std::sync::Arc;

use bota_core::{CellGrid, SightBlock, rules};
use bota_proto::{Fixed, Team, UnitView, Vec2, WorldView};

use crate::match_static::MatchStatic;

/// The ground and the forest sight is worked over, as of one snapshot.
///
/// Sight lines of the stream's own side only: the flight of a scroll's
/// landing and the circle a strike shows are not on the wire.
#[derive(Clone, Debug)]
pub struct SightGround {
    statics: Arc<MatchStatic>,
    block: SightBlock,
    /// The map trees felled when the forest was laid, by index.
    felled: Vec<u32>,
    /// The trees put up by items when the forest was laid.
    planted: Vec<Vec2>,
}

impl SightGround {
    /// The ground with the forest as `view` tells it; the map's own forest
    /// untouched without a view. Absent for a match on a grid of another
    /// size than the sight rule's.
    pub fn lay(statics: &Arc<MatchStatic>, view: Option<&WorldView>) -> Option<Self> {
        if statics.metadata().terrain_cells as usize != rules::GRID_CELLS {
            return None;
        }
        let (felled, planted) = view.map_or((Vec::new(), Vec::new()), |view| {
            (view.felled_trees.clone(), view.planted_trees.clone())
        });
        let mut walls = CellGrid::open();
        for &(x, y) in statics.opaque_cells() {
            walls.close_cell(usize::from(x), usize::from(y));
        }
        let mut down = vec![false; statics.trees().len()];
        for &index in &felled {
            if let Some(down) = down.get_mut(index as usize) {
                *down = true;
            }
        }
        let standing = statics
            .trees()
            .iter()
            .zip(&down)
            .filter(|(_, down)| !**down)
            .map(|(at, _)| *at);
        let block = SightBlock::lay(
            &|at| statics.ground_tier(at),
            &walls,
            standing,
            planted.iter().copied(),
        );
        Some(Self {
            statics: Arc::clone(statics),
            block,
            felled,
            planted,
        })
    }

    /// Whether the forest laid is the one `view` tells.
    pub fn fits(&self, view: &WorldView) -> bool {
        self.felled == view.felled_trees && self.planted == view.planted_trees
    }

    /// Marks in `lit`, one entry per cell row-major from the south-west,
    /// every cell `team` sees through units and grants in `view`. Leaves `lit` as it is
    /// when it is not one entry per cell.
    pub fn lit(&self, view: &WorldView, team: Team, lit: &mut [bool]) {
        if lit.len() != rules::GRID_CELLS * rules::GRID_CELLS {
            return;
        }
        lit.fill(false);
        for (unit, radius) in viewers(view, team) {
            let Some((cx, cy)) = CellGrid::cell_of(unit.pos) else {
                continue;
            };
            let reach = (radius.to_int() / rules::GRID_CELL_SIZE + 1) as usize;
            let tier = self.statics.ground_tier(unit.pos);
            let last = rules::GRID_CELLS - 1;
            for y in cy.saturating_sub(reach)..=(cy + reach).min(last) {
                for x in cx.saturating_sub(reach)..=(cx + reach).min(last) {
                    let slot = &mut lit[y * rules::GRID_CELLS + x];
                    if !*slot {
                        *slot =
                            self.sees_from(unit.pos, radius, tier, CellGrid::cell_center((x, y)));
                    }
                }
            }
        }
    }

    /// Whether `team` sees a point through units and grants in `view`.
    pub fn sees(&self, view: &WorldView, team: Team, at: Vec2) -> bool {
        viewers(view, team).any(|(unit, radius)| {
            self.sees_from(unit.pos, radius, self.statics.ground_tier(unit.pos), at)
        })
    }

    /// The server's rule for one viewer and one point: inside its vision,
    /// on ground no higher than its own, along a line nothing stops.
    fn sees_from(&self, from: Vec2, radius: Fixed, tier: u8, at: Vec2) -> bool {
        from.within(at, radius)
            && bota_core::pit_sight_clear(
                bota_core::map_of(self.statics.metadata().map).boss_zones,
                from,
                at,
            )
            && self.statics.ground_tier(at) <= tier
            && self.block.clear(from, tier, at)
    }
}

/// A grant contributes sight only to the team it names. No unit of the game sees farther than
/// [`bota_core::MAX_VISION`], so neither does one here, whatever a view says: a view of thousands
/// claiming to would have every cell worked out for each of them.
fn viewers(view: &WorldView, team: Team) -> impl Iterator<Item = (&UnitView, Fixed)> {
    view.units.iter().filter_map(move |unit| {
        let own = if unit.team == team {
            unit.vision_radius
        } else {
            Fixed::ZERO
        };
        let grant = unit
            .vision_grant
            .filter(|(side, _)| *side == team)
            .map_or(Fixed::ZERO, |(_, radius)| radius);
        let radius = own.max(grant).min(Fixed::from_int(bota_core::MAX_VISION));
        (radius > Fixed::ZERO).then_some((unit, radius))
    })
}
