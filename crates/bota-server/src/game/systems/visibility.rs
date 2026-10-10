//! Working out who sees what.

use bota_proto::{Fixed, Target, Team, UnitKind, Vec2};

use crate::game::{
    CellGrid, Entity, EntityAllocator, EventVisibility, Exposure, ExposureCell, Ground, Spots,
    Stats, Table, Transform, Visibility, World, is_structure, rules, sight_clear,
};

/// What working out sight reads and writes.
pub struct SightCx<'a> {
    pub entities: &'a EntityAllocator,
    pub transform: &'a Table<Transform>,
    pub team: &'a Table<Team>,
    pub kind: &'a Table<UnitKind>,
    /// How far each entity sees.
    pub stats: &'a Table<Stats>,
    /// The height of the ground.
    pub ground: &'a Ground,
    /// Which cells stop a sight line.
    pub sight_block: &'a CellGrid,
    /// Sight strikes showed the sides struck at.
    pub exposure: &'a Table<Exposure>,
    /// The tick being worked out, for which of that sight still shows.
    pub tick: u32,
    /// Where the answer goes.
    pub visibility: &'a mut Table<Visibility>,
    /// Reused buffers the tables are read into.
    pub sight: &'a mut SightScratch,
}

/// The scratch a pass of sight works in, kept between passes.
#[derive(Default)]
pub struct SightScratch {
    /// One row per live entity.
    rows: Vec<SightRow>,
    /// Every entity whose side and ordinary sight are worth asking about.
    viewers: Vec<SightViewer>,
    /// Every entity whose side and true sight are worth asking about.
    true_viewers: Vec<SightViewer>,
    /// Every row that stands somewhere and has sight to write, by where.
    spots: Spots<u32>,
}

impl SightScratch {
    /// Scratch that has worked out nothing.
    pub fn new() -> SightScratch {
        SightScratch::default()
    }
}

/// One entity as the sight system reads it: what it is, where it stands and
/// which sides see it.
struct SightRow {
    /// Which entity this is.
    entity: Entity,
    /// Which side it is on.
    team: Option<Team>,
    /// Where it stands.
    at: Option<Vec2>,
    /// How far it sees.
    vision: Fixed,
    /// How far true sight reaches.
    true_sight: Fixed,
    /// What kind of thing it is.
    kind: Option<UnitKind>,
    /// Whether true sight alone finds it.
    hides: bool,
    /// The height it stands at.
    tier: u8,
    /// Which sides see it, as the table has it.
    seen: Option<Visibility>,
}

/// One entity that can see: where from, how far, and how far true sight
/// reaches.
#[derive(Clone, Copy)]
struct SightViewer {
    /// Which side it is on.
    side: Team,
    /// Where it stands.
    from: Vec2,
    /// How far it sees.
    vision: Fixed,
    /// How far true sight reaches.
    true_sight: Fixed,
    /// The height it stands at.
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
            tier: at.map_or(0, |at| ground.tier(at)),
            seen: visibility.get(entity).copied(),
        });
    }
    sight.gather_viewers();
    sight.gather_exposures(exposure, tick, ground);
    sight.reset_rows();
    sight.trace(ground, sight_block);
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
                flying: false,
            };
            if row.vision > Fixed::ZERO {
                self.viewers.push(viewer);
            }
            if row.true_sight > Fixed::ZERO {
                self.true_viewers.push(viewer);
            }
        }
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
                    self.viewers.push(exposure_viewer(side, cell, ground));
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
            if row.kind.is_some_and(is_structure) {
                seen.add(Team::Radiant);
                seen.add(Team::Dire);
            }
        }
    }

    /// Adds each viewer's side to every row it sees, walking only the rows
    /// within the square its vision spans.
    fn trace(&mut self, ground: &Ground, sight_block: &CellGrid) {
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
                if viewer.flying || sight_clear(ground, sight_block, viewer.from, viewer.tier, at) {
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

/// The viewer a strike's circle stands for.
fn exposure_viewer(side: Team, cell: ExposureCell, ground: &Ground) -> SightViewer {
    SightViewer {
        side,
        from: cell.at,
        vision: rules::units(rules::STRIKE_REVEAL_RADIUS),
        true_sight: Fixed::ZERO,
        tier: ground.tier(cell.at),
        flying: cell.flying,
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
        let Some(at) = self.transform.get(striker).map(|t| t.pos) else {
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
            cell.flying
                || (tier >= target_tier
                    && sight_clear(&self.ground, &self.sight_block, cell.at, tier, at))
        });
        exposed
            || self.entities.iter().any(|entity| {
                if self.team.get(entity) != Some(&team) {
                    return false;
                }
                let (Some(from), Some(radius)) = (
                    self.transform.get(entity).map(|t| t.pos),
                    self.stats.get(entity).map(|s| s.vision),
                ) else {
                    return false;
                };
                if radius <= Fixed::ZERO || !from.within(at, radius) {
                    return false;
                }
                let viewer_tier = self.ground.tier(from);
                viewer_tier >= target_tier
                    && sight_clear(&self.ground, &self.sight_block, from, viewer_tier, at)
            })
    }

    /// Whether a side sees an entity, from what was worked out this tick.
    pub fn can_see(&self, team: Team, entity: Entity) -> bool {
        self.visibility
            .get(entity)
            .is_some_and(|seen| seen.by(team))
    }

    /// Who may learn of something that happened at a point, when one side is
    /// party to it.
    ///
    /// A side is told of what it can see, and always of what involves it.
    pub fn who_may_know(&self, at: Vec2, involved: Team) -> EventVisibility {
        let radiant = involved == Team::Radiant || self.can_see_point(Team::Radiant, at);
        let dire = involved == Team::Dire || self.can_see_point(Team::Dire, at);
        match (radiant, dire) {
            (true, true) => EventVisibility::Everyone,
            (true, false) => EventVisibility::OneTeam(Team::Radiant),
            (false, true) => EventVisibility::OneTeam(Team::Dire),
            (false, false) => EventVisibility::OneTeam(involved),
        }
    }
}
