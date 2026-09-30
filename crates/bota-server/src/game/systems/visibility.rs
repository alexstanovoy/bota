//! Working out who sees what.

use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::game::{CellGrid, EventVisibility, Ground, Spots, sight_clear};
use crate::game::{
    Entity, EntityAllocator, Stats, Table, Transform, Visibility, World, is_structure,
};

/// What working out sight reads and writes.
pub struct SightCx<'a> {
    /// Which entities exist.
    pub entities: &'a EntityAllocator,
    /// Where each entity stands.
    pub transform: &'a Table<Transform>,
    /// Which side each entity is on.
    pub team: &'a Table<Team>,
    /// What kind of thing each entity is.
    pub kind: &'a Table<UnitKind>,
    /// How far each entity sees.
    pub stats: &'a Table<Stats>,
    /// The height of the ground.
    pub ground: &'a Ground,
    /// Which cells stop a sight line.
    pub sight_block: &'a CellGrid,
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
}

/// Rewrites who sees each entity that has a row of sight.
///
/// A side always sees its own, and both sides see every building. Past
/// that, a side sees what one of its viewers has in range, on ground no
/// higher than its own, along an open sight line. What hides is seen by the
/// other side only where that side's true sight also reaches it.
pub fn visibility_system(cx: SightCx<'_>) {
    let SightCx {
        entities,
        transform,
        team,
        kind,
        stats,
        ground,
        sight_block,
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
            };
            if row.vision > Fixed::ZERO {
                self.viewers.push(viewer);
            }
            if row.true_sight > Fixed::ZERO {
                self.true_viewers.push(viewer);
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
                    || viewer.tier < row.tier
                {
                    continue;
                }
                if sight_clear(ground, sight_block, viewer.from, viewer.tier, at) {
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

impl World {
    /// Whether a side sees a point on the map, worked out live from its
    /// viewers' ordinary sight.
    pub fn can_see_point(&self, team: Team, at: Vec2) -> bool {
        let target_tier = self.ground.tier(at);
        self.entities.iter().any(|entity| {
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
