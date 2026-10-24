//! What a match tells once, at its start, and never changes.

use std::sync::Arc;

use bota_core::{CellGrid, Recipe, cell_tier, missing_parts_with, rules};
use bota_proto::{
    Attribute, Fixed, ItemId, MapId, MatchInfo, Pick, ShopEntry, SlotId, Team, TickMode, Vec2,
};

use crate::limits::{
    MAX_OPAQUE_CELLS, MAX_RECIPE_TREE, MAX_SEATS, MAX_SHOP_COMPONENTS, MAX_SHOP_ITEMS,
    MAX_SHOP_RECIPES, MAX_STATIC_TREES, MAX_TERRAIN_AXIS, MAX_TERRAIN_RUNS,
};
use crate::session::SessionError;

/// The public terms of a match, without its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchMetadata {
    pub map: MapId,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    /// Ticks before game time zero.
    pub pregame_ticks: u32,
    /// Number of terrain cells on each map axis.
    pub terrain_cells: u32,
    pub mode: TickMode,
    pub seats: u8,
    /// Whether the match honours cheat orders.
    pub cheats: bool,
}

/// The validated, immutable inputs of a match: shared by every session of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchStatic {
    metadata: MatchMetadata,
    /// Sorted by slot.
    roster: Vec<Pick>,
    trees: Vec<Vec2>,
    terrain_rle: Vec<(u16, u8)>,
    opaque_cells: Vec<(u16, u16)>,
    terrain: Vec<u8>,
    shop: Vec<ShopEntry>,
    fountains: [Vec2; 2],
    shop_range: i32,
}

impl MatchStatic {
    /// Validates a match description and keeps what a client reads from it.
    pub fn new(info: &MatchInfo) -> Result<Arc<Self>, SessionError> {
        if info.tick_rate == 0 {
            return Err(SessionError::ZeroTickRate);
        }
        if info.picks.is_empty() || info.picks.len() > MAX_SEATS {
            return Err(SessionError::SeatCount(info.picks.len()));
        }
        validate_picks(&info.picks)?;
        validate_static_inputs(info)?;
        let mut roster = info.picks.clone();
        roster.sort_by_key(|pick| pick.slot);
        Ok(Arc::new(Self {
            metadata: MatchMetadata {
                map: info.map,
                tick_rate: info.tick_rate,
                pregame_ticks: info.pregame_ticks,
                terrain_cells: info.terrain_cells,
                mode: info.mode,
                seats: info.picks.len() as u8,
                cheats: info.cheats,
            },
            roster,
            trees: info.trees.clone(),
            terrain_rle: info.terrain_rle.clone(),
            opaque_cells: info.opaque_cells.clone(),
            terrain: decode_terrain(info),
            shop: info.shop.clone(),
            fountains: info.fountains,
            shop_range: info.shop_range,
        }))
    }

    /// The match's public terms.
    #[inline]
    pub const fn metadata(&self) -> MatchMetadata {
        self.metadata
    }

    /// Every seat and its hero, sorted by slot.
    #[inline]
    pub fn roster(&self) -> &[Pick] {
        &self.roster
    }

    /// The pick of one seat.
    pub fn pick(&self, slot: SlotId) -> Option<&Pick> {
        self.roster
            .binary_search_by_key(&slot, |pick| pick.slot)
            .ok()
            .map(|index| &self.roster[index])
    }

    /// Every tree the map starts with.
    #[inline]
    pub fn trees(&self) -> &[Vec2] {
        &self.trees
    }

    /// Run-length encoded terrain, as the server sent it.
    #[inline]
    pub fn terrain_rle(&self) -> &[(u16, u8)] {
        &self.terrain_rle
    }

    /// The terrain cells blocking sight regardless of elevation.
    #[inline]
    pub fn opaque_cells(&self) -> &[(u16, u16)] {
        &self.opaque_cells
    }

    /// The decoded terrain, row-major from the south-west corner.
    pub fn terrain(&self) -> &[u8] {
        &self.terrain
    }

    /// Everything the shop sells.
    pub fn shop(&self) -> &[ShopEntry] {
        &self.shop
    }

    /// What the shop sells under `id`.
    pub fn entry(&self, id: ItemId) -> Option<&ShopEntry> {
        self.shop.iter().find(|entry| entry.id == id)
    }

    /// The parts of `item` a seat holding `held` still has to buy at this
    /// match's shop, by [`missing_parts_with`]: its cheapest recipe, the one
    /// set to `primary` first among equals.
    pub fn missing_parts(
        &self,
        item: ItemId,
        held: &[ItemId],
        primary: Option<Attribute>,
    ) -> Vec<ItemId> {
        let recipes_of = |id: ItemId| {
            let entry = self.entry(id);
            let first = entry
                .filter(|entry| !entry.components.is_empty())
                .map(|entry| &entry.components[..]);
            let others = entry.into_iter().flat_map(|entry| &entry.alternatives);
            first
                .into_iter()
                .chain(others.map(|parts| &parts[..]))
                .map(|parts| Recipe { parts, mode: None })
        };
        missing_parts_with(item, held, primary, recipes_of, |id| {
            self.entry(id).map_or(0, |entry| entry.cost)
        })
    }

    /// Fountain of each team, Radiant then Dire.
    pub const fn fountains(&self) -> [Vec2; 2] {
        self.fountains
    }

    /// How close to its own fountain a hero counts as standing in the home shop.
    pub const fn shop_range(&self) -> i32 {
        self.shop_range
    }

    /// The terrain cell under a position, if it is on the map.
    #[inline]
    pub fn cell_of(&self, position: Vec2) -> Option<(usize, usize)> {
        cell_xy(self.metadata.terrain_cells as usize, position)
    }

    /// Elevation tier of the ground under a position, as the simulator ranks
    /// it for uphill misses; zero off the map.
    #[inline]
    pub fn ground_tier(&self, position: Vec2) -> u8 {
        let axis = self.metadata.terrain_cells as usize;
        cell_xy(axis, position)
            .and_then(|(x, y)| self.terrain.get(y * axis + x))
            .map_or(0, |&cell| cell_tier(cell))
    }
}

/// The cell a position falls into, if it is on an `axis` by `axis` field.
pub fn cell_xy(axis: usize, position: Vec2) -> Option<(usize, usize)> {
    CellGrid::cell_of(position).filter(|&(x, y)| x < axis && y < axis)
}

/// Inclusive raw fixed-point bounds shared by both axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawExtent {
    minimum: i64,
    maximum: i64,
}

impl RawExtent {
    pub(crate) fn map(cells: u32) -> Self {
        Self {
            minimum: 0,
            maximum: map_maximum_raw(cells),
        }
    }

    pub(crate) fn widened(self, units: i32) -> Self {
        let margin = i64::from(units) * i64::from(Fixed::ONE.raw);
        Self {
            minimum: self.minimum - margin,
            maximum: self.maximum + margin,
        }
    }

    fn contains(self, raw: i32) -> bool {
        (self.minimum..=self.maximum).contains(&i64::from(raw))
    }

    pub const fn minimum(self) -> i64 {
        self.minimum
    }

    pub const fn maximum(self) -> i64 {
        self.maximum
    }
}

/// The largest raw coordinate on a map of `cells` terrain cells a side.
#[inline]
pub fn map_maximum_raw(cells: u32) -> i64 {
    i64::from(cells)
        .saturating_mul(i64::from(rules::GRID_CELL_SIZE))
        .saturating_mul(i64::from(Fixed::ONE.raw))
        .saturating_sub(1)
        .min(i64::from(i32::MAX))
}

pub(crate) fn validate_positions(
    field: &'static str,
    positions: impl IntoIterator<Item = Vec2>,
    extent: RawExtent,
) -> Result<(), SessionError> {
    for (index, position) in positions.into_iter().enumerate() {
        if !extent.contains(position.x.raw) || !extent.contains(position.y.raw) {
            return Err(SessionError::PositionOutOfMap {
                field,
                index,
                x_raw: position.x.raw,
                y_raw: position.y.raw,
                extent,
            });
        }
    }
    Ok(())
}

/// Fails when `actual` of a bounded field exceeds its `limit`.
pub fn check_limit(field: &'static str, actual: usize, limit: usize) -> Result<(), SessionError> {
    if actual > limit {
        return Err(SessionError::LimitExceeded {
            field,
            actual,
            limit,
        });
    }
    Ok(())
}

/// The own seat's team, from the roster: the seat must exist and be playable.
pub(crate) fn seat_team(
    statics: &MatchStatic,
    slot: SlotId,
    hero: Option<bota_proto::HeroId>,
) -> Result<Team, SessionError> {
    if usize::from(slot.0) >= MAX_SEATS {
        return Err(SessionError::OwnSlotOutOfRange(slot.0));
    }
    let own = statics
        .pick(slot)
        .ok_or(SessionError::MissingOwnPick(slot.0))?;
    if let Some(expected) = hero
        && own.hero != expected
    {
        return Err(SessionError::WrongOwnHero {
            actual: own.hero,
            expected,
        });
    }
    if !matches!(own.team, Team::Radiant | Team::Dire) {
        return Err(SessionError::InvalidOwnTeam(own.team));
    }
    Ok(own.team)
}

fn validate_picks(picks: &[Pick]) -> Result<(), SessionError> {
    for (index, pick) in picks.iter().enumerate() {
        if usize::from(pick.slot.0) >= MAX_SEATS {
            return Err(SessionError::PickSlotOutOfRange(pick.slot.0));
        }
        if picks[..index].iter().any(|other| other.slot == pick.slot) {
            return Err(SessionError::DuplicatePickSlot(pick.slot.0));
        }
    }
    Ok(())
}

fn validate_static_inputs(info: &MatchInfo) -> Result<(), SessionError> {
    check_limit("MatchInfo.trees", info.trees.len(), MAX_STATIC_TREES)?;
    check_limit(
        "MatchInfo.terrain_rle",
        info.terrain_rle.len(),
        MAX_TERRAIN_RUNS,
    )?;
    check_limit(
        "MatchInfo.opaque_cells",
        info.opaque_cells.len(),
        MAX_OPAQUE_CELLS,
    )?;
    check_limit("MatchInfo.shop", info.shop.len(), MAX_SHOP_ITEMS)?;
    if info.terrain_cells == 0 || info.terrain_cells > MAX_TERRAIN_AXIS {
        return Err(SessionError::TerrainAxis(info.terrain_cells));
    }
    if let Some(index) = info.terrain_rle.iter().position(|(run, _)| *run == 0) {
        return Err(SessionError::ZeroTerrainRun(index));
    }
    let expected = u64::from(info.terrain_cells) * u64::from(info.terrain_cells);
    let actual = info
        .terrain_rle
        .iter()
        .fold(0u64, |sum, (run, _)| sum.saturating_add(u64::from(*run)));
    if actual != expected {
        return Err(SessionError::TerrainCoverage { actual, expected });
    }
    for &(x, y) in &info.opaque_cells {
        if u32::from(x) >= info.terrain_cells || u32::from(y) >= info.terrain_cells {
            return Err(SessionError::OpaqueCellOutOfRange {
                x,
                y,
                cells: info.terrain_cells,
            });
        }
    }
    validate_positions(
        "MatchInfo.trees",
        info.trees.iter().copied(),
        RawExtent::map(info.terrain_cells),
    )?;
    for entry in &info.shop {
        check_limit(
            "ShopEntry.components",
            entry.components.len(),
            MAX_SHOP_COMPONENTS,
        )?;
        check_limit(
            "ShopEntry.alternatives",
            entry.alternatives.len(),
            MAX_SHOP_RECIPES,
        )?;
        for parts in &entry.alternatives {
            check_limit(
                "ShopEntry.alternatives parts",
                parts.len(),
                MAX_SHOP_COMPONENTS,
            )?;
        }
    }
    validate_recipes(&info.shop)
}

/// Every item once, every part of a recipe sold, no item built from itself,
/// and no recipe tree past [`MAX_RECIPE_TREE`]: a client lays a tree out
/// whole, and a shop of items each made of many of the next would make that
/// exponential.
fn validate_recipes(shop: &[ShopEntry]) -> Result<(), SessionError> {
    if (1..shop.len()).any(|at| shop[..at].iter().any(|other| other.id == shop[at].id)) {
        return Err(SessionError::InvalidShop("repeats an item"));
    }
    let mut colors = [0u8; MAX_SHOP_ITEMS];
    let mut trees = [0usize; MAX_SHOP_ITEMS];
    (0..shop.len()).try_for_each(|root| visit_recipes(shop, root, &mut colors, &mut trees))
}

/// Depth first over recipe parts, `colors` 1 on the path and 2 once done; the
/// depth is at most the shop's size. `trees` holds, once an item is done,
/// the items its first recipe lays out, itself included.
fn visit_recipes(
    shop: &[ShopEntry],
    at: usize,
    colors: &mut [u8],
    trees: &mut [usize],
) -> Result<(), SessionError> {
    match colors[at] {
        1 => return Err(SessionError::InvalidShop("has a cyclic recipe")),
        2 => return Ok(()),
        _ => {}
    }
    colors[at] = 1;
    let entry = &shop[at];
    let mut tree = 1;
    for (index, part) in entry
        .components
        .iter()
        .chain(entry.alternatives.iter().flatten())
        .enumerate()
    {
        let next = shop
            .iter()
            .position(|other| other.id == *part)
            .ok_or(SessionError::InvalidShop("names a part it does not sell"))?;
        visit_recipes(shop, next, colors, trees)?;
        if index < entry.components.len() {
            tree += trees[next];
        }
        if tree > MAX_RECIPE_TREE {
            return Err(SessionError::InvalidShop("has a recipe tree too large"));
        }
    }
    trees[at] = tree;
    colors[at] = 2;
    Ok(())
}

fn decode_terrain(info: &MatchInfo) -> Vec<u8> {
    let axis = info.terrain_cells as usize;
    let mut terrain = Vec::with_capacity(axis * axis);
    for &(run, cell) in &info.terrain_rle {
        terrain.resize(terrain.len() + usize::from(run), cell);
    }
    debug_assert_eq!(terrain.len(), axis * axis);
    terrain
}

#[cfg(test)]
mod tests;
