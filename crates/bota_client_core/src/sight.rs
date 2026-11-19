//! What a side sees, worked out as the server works it out.
//!
//! The server tells a client only what its side sees; this lays the same
//! rule over the ground to say which cells those are, to shade the rest.
//! The rule is [`bota_core::SightBlock`]: the ground, the map's fog blocker
//! walls and the trees standing now, which a snapshot tells by the map's
//! felled trees and the planted ones.

use std::sync::Arc;

use bota_core::{CellGrid, SightBlock, rules};
use bota_proto::{Fixed, Team, Vec2, WorldView};

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
    /// Refresh a received-forest cache during view preparation, not inside script callbacks.
    /// O(grid cells + trees) on a changed forest, O(forest changes) otherwise; no unit or hidden world state.
    pub fn refresh_forest(
        cache: &mut Option<Self>,
        statics: &Arc<MatchStatic>,
        felled: &[u32],
        planted: &[Vec2],
    ) {
        if cache.as_ref().is_none_or(|ground| {
            !Arc::ptr_eq(&ground.statics, statics) || !ground.fits_forest(felled, planted)
        }) {
            *cache = Self::lay_forest(statics, felled, planted);
        }
    }

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
        Self::lay_forest(statics, &felled, &planted)
    }

    /// O(GRID_CELLS² + static trees + planted trees); build once per received forest.
    /// Only the receiving seat's forest changes belong in these slices.
    pub fn lay_forest(
        statics: &Arc<MatchStatic>,
        felled: &[u32],
        planted: &[Vec2],
    ) -> Option<Self> {
        if statics.metadata().terrain_cells as usize != rules::GRID_CELLS
            || felled.len() > bota_proto::limits::MAX_STATIC_TREES
            || planted.len() > bota_proto::limits::MAX_PLANTED_TREES
        {
            return None;
        }
        let mut walls = CellGrid::open();
        for &(x, y) in statics.opaque_cells() {
            walls.close_cell(usize::from(x), usize::from(y));
        }
        let mut down = vec![false; statics.trees().len()];
        for &index in felled {
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
            felled: felled.to_vec(),
            planted: planted.to_vec(),
        })
    }

    /// Whether the forest laid is the one `view` tells.
    pub fn fits(&self, view: &WorldView) -> bool {
        self.fits_forest(&view.felled_trees, &view.planted_trees)
    }
    pub fn fits_forest(&self, felled: &[u32], planted: &[Vec2]) -> bool {
        self.felled == felled && self.planted == planted
    }

    /// Marks in `lit`, one entry per cell row-major from the south-west,
    /// every cell `team` sees through units and grants in `view`. Leaves `lit` as it is
    /// when it is not one entry per cell.
    pub fn lit(&self, view: &WorldView, team: Team, lit: &mut [bool]) {
        if lit.len() != rules::GRID_CELLS * rules::GRID_CELLS {
            return;
        }
        lit.fill(false);
        for (at, radius) in viewers(view, team, bota_core::map_of(self.statics.metadata().map)) {
            let Some((cx, cy)) = CellGrid::cell_of(at) else {
                continue;
            };
            let reach = (radius.to_int() / rules::GRID_CELL_SIZE + 1) as usize;
            let tier = self.statics.ground_tier(at);
            let last = rules::GRID_CELLS - 1;
            for y in cy.saturating_sub(reach)..=(cy + reach).min(last) {
                for x in cx.saturating_sub(reach)..=(cx + reach).min(last) {
                    let slot = &mut lit[y * rules::GRID_CELLS + x];
                    if !*slot {
                        *slot = self.sees_from(at, radius, tier, CellGrid::cell_center((x, y)));
                    }
                }
            }
        }
    }

    /// Whether `team` sees a point through units and grants in `view`.
    pub fn sees(&self, view: &WorldView, team: Team, at: Vec2) -> bool {
        self.sees_units(view.units.iter(), team, at)
    }
    /// O(visible units × maximum sight ray length), bounded by the view and grid limits.
    pub fn sees_units<'a>(
        &self,
        units: impl Iterator<Item = &'a bota_proto::UnitView> + 'a,
        team: Team,
        at: Vec2,
    ) -> bool {
        unit_viewers(units, team, bota_core::map_of(self.statics.metadata().map))
            .any(|(from, radius)| self.sees_from(from, radius, self.statics.ground_tier(from), at))
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
fn viewers<'a>(
    view: &'a WorldView,
    team: Team,
    map: &'a bota_core::MapDef,
) -> impl Iterator<Item = (Vec2, Fixed)> + 'a {
    unit_viewers(view.units.iter(), team, map)
}
fn unit_viewers<'a>(
    units: impl Iterator<Item = &'a bota_proto::UnitView> + 'a,
    team: Team,
    map: &'a bota_core::MapDef,
) -> impl Iterator<Item = (Vec2, Fixed)> + 'a {
    units
        .take(bota_proto::limits::MAX_VIEW_UNITS)
        .filter_map(move |unit| {
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
            (radius > Fixed::ZERO).then_some((unit.pos, radius))
        })
        .chain(
            map.base_blockers
                .iter()
                .filter(move |blocker| blocker.team == team)
                .map(|blocker| (blocker.at, Fixed::from_int(bota_core::BASE_BLOCKER_VISION))),
        )
}
