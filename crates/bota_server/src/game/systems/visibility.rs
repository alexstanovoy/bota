//! Working out who sees what.

use bota_core::{CellGrid, SightBlock, kind_def, rules};
use bota_proto::{Fixed, Target, Team, UnitKind, Vec2};

use crate::game::{
    Entity, EntityAllocator, EventVisibility, Exposure, ExposureCell, Ground, Spots, Stats, Table,
    Transform, Visibility, World,
};

/// What working out sight reads and writes.
pub struct SightCx<'a> {
    pub entities: &'a EntityAllocator,
    pub transform: &'a Table<Transform>,
    pub team: &'a Table<Team>,
    pub kind: &'a Table<UnitKind>,
    /// How far each entity sees.
    pub stats: &'a Table<Stats>,
    pub ground: &'a Ground,
    /// Which cells stop a sight line.
    pub sight_block: &'a SightBlock,
    /// Sight strikes showed the sides struck at.
    pub exposure: &'a Table<Exposure>,
    /// Where a Town Portal Scroll shows its side the ground it lands on.
    pub scroll_sights: &'a [(Team, Vec2)],
    /// The tick being worked out, for which of that sight still shows.
    pub tick: u32,
    /// Where the answer goes.
    pub visibility: &'a mut Table<Visibility>,
    /// Reused buffers the tables are read into.
    pub sight: &'a mut SightScratch,
}

/// The scratch a pass of sight works in, kept between passes.
#[derive(Clone, Default)]
pub struct SightScratch {
    /// One row per live entity.
    rows: Vec<SightRow>,
    /// Every entity whose side and ordinary sight are worth asking about.
    viewers: Vec<SightViewer>,
    /// Every entity whose side and true sight are worth asking about.
    true_viewers: Vec<SightViewer>,
    /// Every row that stands somewhere and has sight to write, by where.
    spots: Spots<u32>,
    /// Which row of [`SightScratch::rows`] each entity slot was read into,
    /// `u32::MAX` for none: the rows are looked up by entity on every view.
    row_at: Vec<u32>,
    /// What every entity that moved or came into the world since the pass
    /// was judged to be seen by where it stood at the tick's end, by entity
    /// slot: worked out once a tick rather than on every view.
    judged: Vec<Option<Judged>>,
}

/// One entity judged afresh against the eyes of the last pass.
#[derive(Clone, Copy)]
struct Judged {
    entity: Entity,
    at: Option<Vec2>,
    team: Option<Team>,
    seen: Visibility,
}

impl SightScratch {
    pub fn new() -> SightScratch {
        SightScratch::default()
    }
}

/// One entity as the sight system reads it: what it is, where it stands and
/// which sides see it.
#[derive(Clone)]
struct SightRow {
    entity: Entity,
    team: Option<Team>,
    at: Option<Vec2>,
    vision: Fixed,
    true_sight: Fixed,
    kind: Option<UnitKind>,
    /// Whether true sight alone finds it.
    hides: bool,
    /// Whether it flies, and sees past height and trees as a flyer does.
    flying: bool,
    tier: u8,
    /// Which sides see it, as the table has it.
    seen: Option<Visibility>,
}

/// One entity that can see: where from, how far, and how far true sight
/// reaches.
#[derive(Clone, Copy)]
struct SightViewer {
    side: Team,
    from: Vec2,
    vision: Fixed,
    true_sight: Fixed,
    tier: u8,
    /// Whether it sees past height and trees.
    flying: bool,
}

/// Rewrites who sees each entity that has a row of sight.
///
/// A side always sees its own, and both sides see every building. Past
/// that, a side sees what one of its viewers has in range, on ground no
/// higher than its own, along an open sight line. A strike shows the side
/// struck at a circle around the striker the same way, see
/// [`World::expose`]. What hides is seen by the other side only where that
/// side's true sight also reaches it.
pub fn visibility_system(cx: SightCx<'_>) {
    let SightCx {
        entities,
        transform,
        team,
        kind,
        stats,
        ground,
        sight_block,
        exposure,
        scroll_sights,
        tick,
        visibility,
        sight,
    } = cx;
    sight.rows.clear();
    for entity in entities.iter() {
        let at = transform.get(entity).map(|t| t.pos);
        let stats = stats.get(entity);
        sight.rows.push(SightRow {
            entity,
            team: team.get(entity).copied(),
            at,
            vision: stats.map_or(Fixed::ZERO, |s| s.vision),
            true_sight: stats.map_or(Fixed::ZERO, |s| s.true_sight),
            kind: kind.get(entity).copied(),
            hides: stats.is_some_and(|s| s.hides),
            flying: stats.is_some_and(|s| s.flies),
            tier: at.map_or(0, |at| ground.tier(at)),
            seen: visibility.get(entity).copied(),
        });
    }
    sight.index_rows();
    sight.judged.clear();
    sight.gather_viewers();
    sight.gather_exposures(exposure, tick, ground);
    for &(side, at) in scroll_sights {
        let radius = rules::SCROLL_SIGHT_RADIUS;
        sight.viewers.push(reveal(side, at, radius, true, ground));
    }
    sight.reset_rows();
    sight.trace(sight_block);
    sight.hide();
    for row in &sight.rows {
        let (Some(seen), Some(slot)) = (row.seen, visibility.get_mut(row.entity)) else {
            continue;
        };
        *slot = seen;
    }
}

impl SightScratch {
    /// Every row that can see, by ordinary sight and by true sight.
    fn gather_viewers(&mut self) {
        self.viewers.clear();
        self.true_viewers.clear();
        for row in &self.rows {
            let (Some(side), Some(from)) = (row.team, row.at) else {
                continue;
            };
            let viewer = SightViewer {
                side,
                from,
                vision: row.vision,
                true_sight: row.true_sight,
                tier: row.tier,
                flying: row.flying,
            };
            if row.vision > Fixed::ZERO {
                self.viewers.push(viewer);
            }
            if row.true_sight > Fixed::ZERO {
                self.true_viewers.push(viewer);
            }
        }
    }

    /// Whether a side's sight takes in a cell: one of its viewers reaches the
    /// cell's centre, from ground no lower, over an open line.
    pub(crate) fn sees_cell(
        &self,
        side: Team,
        cell: (usize, usize),
        ground: &Ground,
        sight_block: &SightBlock,
    ) -> bool {
        let at = CellGrid::cell_center(cell);
        let tier = ground.tier(at);
        self.viewers.iter().any(|viewer| {
            viewer.side == side
                && viewer.from.within(at, viewer.vision)
                && (viewer.flying
                    || (viewer.tier >= tier && sight_block.clear(viewer.from, viewer.tier, at)))
        })
    }

    /// The row the last pass read an entity into, if it read one.
    fn row_of(&self, entity: Entity) -> Option<&SightRow> {
        let at = *self.row_at.get(entity.index().0 as usize)?;
        self.rows
            .get(at as usize)
            .filter(|row| row.entity == entity)
    }

    /// Indexes the rows by entity slot, for [`SightScratch::row_of`].
    fn index_rows(&mut self) {
        self.row_at.clear();
        for (at, row) in self.rows.iter().enumerate() {
            let slot = row.entity.index().0 as usize;
            if self.row_at.len() <= slot {
                self.row_at.resize(slot + 1, u32::MAX);
            }
            self.row_at[slot] = at as u32;
        }
    }

    /// Whether one side sees a row, judged against the viewers of the last
    /// pass as [`visibility_system`] judges every row: its own side always,
    /// both sides a building, otherwise a viewer in reach over an open line,
    /// and for what hides, true sight as well.
    fn sees_row(&self, team: Team, row: &SightRow, sight_block: &SightBlock) -> bool {
        if row.team == Some(team) {
            return true;
        }
        if team != Team::Neutral && row.kind.is_some_and(|kind| kind_def(kind).always_seen) {
            return true;
        }
        let Some(at) = row.at else {
            return false;
        };
        let seen = self.viewers.iter().any(|viewer| {
            viewer.side == team
                && viewer.from.within(at, viewer.vision)
                && (viewer.flying
                    || (viewer.tier >= row.tier && sight_block.clear(viewer.from, viewer.tier, at)))
        });
        seen && (!row.hides
            || row.team.is_none()
            || self
                .true_viewers
                .iter()
                .any(|viewer| viewer.side == team && viewer.from.within(at, viewer.true_sight)))
    }

    /// Every circle a strike still shows, as a viewer of the side struck at.
    fn gather_exposures(&mut self, exposure: &Table<Exposure>, tick: u32, ground: &Ground) {
        for row in &self.rows {
            let Some(Exposure(cells)) = exposure.get(row.entity) else {
                continue;
            };
            for (side, cell) in [Team::Radiant, Team::Dire, Team::Neutral]
                .into_iter()
                .zip(cells)
            {
                if let Some(cell) = cell.filter(|cell| tick <= cell.until) {
                    let radius = rules::STRIKE_REVEAL_RADIUS;
                    self.viewers
                        .push(reveal(side, cell.at, radius, cell.flying, ground));
                }
            }
        }
    }

    /// Every row seen by its own side alone, and a building by both.
    fn reset_rows(&mut self) {
        for row in &mut self.rows {
            let Some(seen) = row.seen.as_mut() else {
                continue;
            };
            *seen = Visibility::NONE;
            if let Some(side) = row.team {
                seen.add(side);
            }
            if row.kind.is_some_and(|kind| kind_def(kind).always_seen) {
                seen.add(Team::Radiant);
                seen.add(Team::Dire);
            }
        }
    }

    /// Adds each viewer's side to every row it sees, walking only the rows
    /// within the square its vision spans.
    fn trace(&mut self, sight_block: &SightBlock) {
        self.spots.clear();
        for (index, row) in self.rows.iter().enumerate() {
            if let (Some(at), Some(_)) = (row.at, row.seen) {
                self.spots.push(at, index as u32);
            }
        }
        self.spots.sort();
        for viewer in &self.viewers {
            let reach = i64::from(viewer.vision.raw).abs();
            for index in self.spots.around(viewer.from, reach) {
                let row = &mut self.rows[index as usize];
                let (Some(at), Some(seen)) = (row.at, row.seen.as_mut()) else {
                    continue;
                };
                if seen.by(viewer.side)
                    || !viewer.from.within(at, viewer.vision)
                    || (!viewer.flying && viewer.tier < row.tier)
                {
                    continue;
                }
                if viewer.flying || sight_block.clear(viewer.from, viewer.tier, at) {
                    seen.add(viewer.side);
                }
            }
        }
    }

    /// Takes what hides away from every side whose true sight does not
    /// reach it.
    fn hide(&mut self) {
        for row in &mut self.rows {
            if !row.hides {
                continue;
            }
            let (Some(side), Some(at), Some(already)) = (row.team, row.at, row.seen) else {
                continue;
            };
            let mut seen = Visibility::NONE;
            seen.add(side);
            for viewer in &self.true_viewers {
                if viewer.side != side
                    && already.by(viewer.side)
                    && viewer.from.within(at, viewer.true_sight)
                {
                    seen.add(viewer.side);
                }
            }
            row.seen = Some(seen);
        }
    }
}

/// The viewer a circle of sight shown to a side stands for: a scroll's
/// landing, which flies, or a strike's circle.
fn reveal(side: Team, at: Vec2, radius: i32, flying: bool, ground: &Ground) -> SightViewer {
    SightViewer {
        side,
        from: at,
        vision: rules::units(radius),
        true_sight: Fixed::ZERO,
        tier: ground.tier(at),
        flying,
    }
}

impl World {
    /// Shows the side a strike was aimed at a circle of
    /// [`rules::STRIKE_REVEAL_RADIUS`] around where the striker stands, for
    /// [`rules::STRIKE_REVEAL_TICKS`] from the next sight pass on.
    ///
    /// A striker shows each side one circle at most: striking again moves
    /// it, starts its time afresh, and, while the last one still showed,
    /// makes it see past height and trees. A strike at one's own side shows
    /// nothing.
    pub fn expose(&mut self, striker: Entity, on: Entity) {
        let (Some(mine), Some(theirs)) =
            (self.team.get(striker).copied(), self.team.get(on).copied())
        else {
            return;
        };
        let Some(at) = self.pos(striker) else {
            return;
        };
        if mine == theirs {
            return;
        }
        let tick = self.tick;
        let mut exposure = self.exposure.get(striker).copied().unwrap_or_default();
        let cell = &mut exposure.0[theirs as usize];
        let flying = cell.is_some_and(|cell| tick <= cell.until);
        *cell = Some(ExposureCell {
            at,
            until: tick + rules::STRIKE_REVEAL_TICKS,
            flying,
        });
        self.exposure.insert(striker, exposure);
    }

    /// Shows a caster to the side of the unit its spell went off at, when
    /// that unit stands within [`rules::CAST_REVEAL_RANGE`] of it.
    pub fn expose_caster(&mut self, caster: Entity, target: Target) {
        let Target::Unit(id) = target else {
            return;
        };
        let Some(mark) = self.of_wire(id) else {
            return;
        };
        let (Some(from), Some(to)) = (self.transform.get(caster), self.transform.get(mark)) else {
            return;
        };
        if from
            .pos
            .within(to.pos, rules::units(rules::CAST_REVEAL_RANGE))
        {
            self.expose(caster, mark);
        }
    }

    /// Whether a side sees a point on the map, worked out live from its
    /// viewers' ordinary sight and the circles strikes showed it.
    pub fn can_see_point(&self, team: Team, at: Vec2) -> bool {
        let target_tier = self.ground.tier(at);
        let exposed = self.entities.iter().any(|entity| {
            let Some(Some(cell)) = self.exposure.get(entity).map(|e| e.0[team as usize]) else {
                return false;
            };
            if self.tick > cell.until
                || !cell
                    .at
                    .within(at, rules::units(rules::STRIKE_REVEAL_RADIUS))
            {
                return false;
            }
            let tier = self.ground.tier(cell.at);
            cell.flying || (tier >= target_tier && self.sight_block.clear(cell.at, tier, at))
        });
        let landing = self.teleport_sights().any(|(side, from)| {
            side == team && from.within(at, rules::units(rules::SCROLL_SIGHT_RADIUS))
        });
        exposed
            || landing
            || self.entities.iter().any(|entity| {
                if self.team.get(entity) != Some(&team) {
                    return false;
                }
                let (Some(from), Some(stats)) = (self.pos(entity), self.stats.get(entity)) else {
                    return false;
                };
                if stats.vision <= Fixed::ZERO || !from.within(at, stats.vision) {
                    return false;
                }
                let viewer_tier = self.ground.tier(from);
                stats.flies
                    || (viewer_tier >= target_tier && self.sight_block.clear(from, viewer_tier, at))
            })
    }

    /// Whether a side's views show an entity: what the tick's sight found,
    /// for an entity that stands where that sight found it. One that moved
    /// since (blinked, was carried, flew) or was put into the world since (a
    /// missile let go, what a fall leaves) is judged where it now stands,
    /// against the same eyes: a view shows what stands where its side has
    /// sight, wherever it came from, and never a unit where its side could
    /// not see it.
    pub fn shows(&self, team: Team, entity: Entity) -> bool {
        let Some(seen) = self.visibility.get(entity) else {
            return false;
        };
        let at = self.pos(entity);
        let side = self.team.get(entity).copied();
        let sight = &self.sight_scratch;
        if sight
            .row_of(entity)
            .is_some_and(|row| row.at == at && row.team == side)
        {
            return seen.by(team);
        }
        let judged = sight
            .judged
            .get(entity.index().0 as usize)
            .copied()
            .flatten()
            .filter(|judged| judged.entity == entity && judged.at == at && judged.team == side);
        match judged {
            Some(judged) => judged.seen.by(team),
            None => self.judge(entity, at, side).by(team),
        }
    }

    /// The sides that see an entity standing at `at`, judged against the
    /// eyes of the last pass, see [`World::shows`].
    fn judge(&self, entity: Entity, at: Option<Vec2>, side: Option<Team>) -> Visibility {
        let stats = self.stats.get(entity);
        let row = SightRow {
            entity,
            team: side,
            at,
            vision: Fixed::ZERO,
            true_sight: Fixed::ZERO,
            kind: self.kind.get(entity).copied(),
            hides: stats.is_some_and(|stats| stats.hides),
            flying: stats.is_some_and(|stats| stats.flies),
            tier: at.map_or(0, |at| self.ground.tier(at)),
            seen: None,
        };
        let mut seen = Visibility::NONE;
        for team in [Team::Radiant, Team::Dire, Team::Neutral] {
            if self.sight_scratch.sees_row(team, &row, &self.sight_block) {
                seen.add(team);
            }
        }
        seen
    }

    /// Judges once every entity that moved or came into the world since the
    /// tick's pass of sight, for the views and the orders given on them.
    pub(crate) fn judge_what_moved(&mut self) {
        let mut judged = std::mem::take(&mut self.sight_scratch.judged);
        judged.clear();
        for entity in self.entities.iter() {
            if self.visibility.get(entity).is_none() {
                continue;
            }
            let at = self.pos(entity);
            let side = self.team.get(entity).copied();
            let unmoved = self
                .sight_scratch
                .row_of(entity)
                .is_some_and(|row| row.at == at && row.team == side);
            if unmoved {
                continue;
            }
            let slot = entity.index().0 as usize;
            if judged.len() <= slot {
                judged.resize(slot + 1, None);
            }
            judged[slot] = Some(Judged {
                entity,
                at,
                team: side,
                seen: self.judge(entity, at, side),
            });
        }
        self.sight_scratch.judged = judged;
    }

    /// Whether a side sees an entity, from what was worked out this tick.
    pub fn can_see(&self, team: Team, entity: Entity) -> bool {
        self.visibility
            .get(entity)
            .is_some_and(|seen| seen.by(team))
    }

    /// Whether a side sees a unit as something befalls it: what its view
    /// would show of it now, see [`World::shows`]. A unit gone from the
    /// world is known to its own side alone.
    pub fn sees_now(&self, team: Team, entity: Entity) -> bool {
        self.team.get(entity) == Some(&team) || self.shows(team, entity)
    }

    /// Who may learn of what befell a unit: its own side, the side `party`
    /// when one is party to it (the one that brought it down), and each side
    /// that sees it as it happens. Seeing the ground it stands on is not
    /// seeing it: a ward hidden there is told of to nobody else.
    pub fn who_may_know(&self, subject: Entity, party: Option<Team>) -> EventVisibility {
        let told = |team| party == Some(team) || self.sees_now(team, subject);
        EventVisibility::of(told(Team::Radiant), told(Team::Dire))
    }

    /// Who may learn of what happened at a spot rather than to a unit, as a
    /// grenade bursting: the sides that see the spot, and always `party`.
    pub fn who_sees_spot(&self, at: Vec2, party: Team) -> EventVisibility {
        let told = |team| party == team || self.can_see_point(team, at);
        EventVisibility::of(told(Team::Radiant), told(Team::Dire))
    }

    /// Who may be told which unit dealt something: its own side and each
    /// side that sees it as it happens. Everyone when no unit dealt it.
    pub fn who_may_name(&self, dealer: Option<Entity>) -> EventVisibility {
        let Some(dealer) = dealer else {
            return EventVisibility::Everyone;
        };
        EventVisibility::of(
            self.sees_now(Team::Radiant, dealer),
            self.sees_now(Team::Dire, dealer),
        )
    }
}
