//! Working out who sees what.

use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::game::{CellGrid, Event, EventVisibility, Ground, sight_clear};
use crate::game::{
    Entity, EntityAllocator, Stats, Table, Transform, Visibility, World, is_structure,
};

/// Rewrites who sees each entity.
///
/// The row itself belongs to the entity: it is made when the entity is and
/// given up when it goes, so this only ever rewrites what is already there.
///
/// A side always sees its own, and both sides always see a building. What
/// hides asks for two things at once from any other side: ordinary sight of
/// where it stands, and true sight reaching it. True sight takes nothing off
/// what hides in ground nobody is looking at. Beyond
/// that, everything that can see is walked
/// in turn against everything inside its sight radius, and the line is traced
/// only for what its side cannot already see: once one pair of eyes has an
/// entity, no other pair of the same side pays for it again.
/// What working out sight reads and writes.
///
/// The set runs past what is readable named one by one at a call site;
/// gathered here, the access a system takes is still declared, and still
/// checked when [`World::step`] hands the tables over.
///
/// [`World::step`]: crate::game::World::step
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
        sight.rows.push(SightRow {
            entity,
            team: team.get(entity).copied(),
            at: transform.get(entity).map(|t| t.pos),
            vision: stats.get(entity).map_or(Fixed::ZERO, |s| s.vision),
            true_sight: stats.get(entity).map_or(Fixed::ZERO, |s| s.true_sight),
            kind: kind.get(entity).copied(),
            hides: stats.get(entity).is_some_and(|s| s.hides),
            tier: transform.get(entity).map_or(0, |t| ground.tier(t.pos)),
            seen: visibility.get(entity).copied(),
        });
    }
    sight.viewers.clear();
    sight.true_viewers.clear();
    for row in &sight.rows {
        let (Some(side), Some(from)) = (row.team, row.at) else {
            continue;
        };
        if row.vision > Fixed::ZERO {
            sight.viewers.push(SightViewer {
                side,
                from,
                vision: row.vision,
                true_sight: row.true_sight,
                tier: row.tier,
            });
        }
        if row.true_sight > Fixed::ZERO {
            sight.true_viewers.push(SightViewer {
                side,
                from,
                vision: row.vision,
                true_sight: row.true_sight,
                tier: row.tier,
            });
        }
    }
    for row in &mut sight.rows {
        let Some(seen) = row.seen.as_mut() else {
            continue;
        };
        *seen = Visibility::NONE;
        if let Some(side) = row.team {
            seen.add(side);
        }
        // A building is on every map both sides look at, ward and unit alike
        // are not.
        if row.kind.is_some_and(is_structure) {
            seen.add(Team::Radiant);
            seen.add(Team::Dire);
        }
    }
    for viewer in &sight.viewers {
        for row in sight.rows.iter_mut() {
            if row.seen.as_ref().is_some_and(|seen| seen.by(viewer.side)) {
                continue;
            }
            let Some(at) = row.at else {
                continue;
            };
            if !viewer.from.within(at, viewer.vision) || viewer.tier < row.tier {
                continue;
            }
            if sight_clear(ground, sight_block, viewer.from, viewer.tier, at)
                && let Some(seen) = row.seen.as_mut()
            {
                seen.add(viewer.side);
            }
        }
    }
    // What hides is not given away by standing in the open: for the other
    // side it exists only where true sight reaches it.
    for row in &mut sight.rows {
        if !row.hides {
            continue;
        }
        let (Some(side), Some(at)) = (row.team, row.at) else {
            continue;
        };
        let Some(already) = row.seen else {
            continue;
        };
        let mut seen = Visibility::NONE;
        seen.add(side);
        for viewer in &sight.true_viewers {
            if viewer.side != side
                && already.by(viewer.side)
                && viewer.from.within(at, viewer.true_sight)
            {
                seen.add(viewer.side);
            }
        }
        row.seen = Some(seen);
    }
    for row in &sight.rows {
        let (Some(seen), Some(slot)) = (row.seen, visibility.get_mut(row.entity)) else {
            continue;
        };
        *slot = seen;
    }
}

impl World {
    /// Whether a side sees a point on the map.
    ///
    /// A point has no row of its own, so this is asked live: for an event at a
    /// spot, or for an order at somewhere nobody stands.
    pub fn can_see_point(&self, team: Team, at: Vec2) -> bool {
        let target_tier = self.ground.tier(at);
        self.entities.iter().any(|entity| {
            let (Some(side), Some(from), Some(radius)) = (
                self.team.get(entity).copied(),
                self.transform.get(entity).map(|t| t.pos),
                self.stats.get(entity).map(|s| s.vision),
            ) else {
                return false;
            };
            let viewer_tier = self.ground.tier(from);
            side == team
                && radius > Fixed::ZERO
                && from.within(at, radius)
                && viewer_tier >= target_tier
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

    /// Keeps from a side what it had no way of seeing.
    pub fn hide_unseen(&self, events: &mut [Event], places: &[(usize, Vec2, Team)]) {
        for &(index, at, involved) in places {
            if let Some(event) = events.get_mut(index) {
                event.visible_to = self.who_may_know(at, involved);
            }
        }
    }
}
