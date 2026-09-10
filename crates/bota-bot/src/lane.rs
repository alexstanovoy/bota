//! Which lane a seat holds, and where along it things are.
//!
//! The lane is read off the snapshot rather than carried as a table: both
//! sides always see every building, so the fountains and the towers are
//! enough to lay out where a lane runs on whichever map is being played.

use bota_proto::{Team, UnitKind, Vec2, WorldView};

use crate::span;

/// What a seat is there to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The one the safe lane is farmed for.
    Carry,
    /// The middle.
    Mid,
    /// The hard lane.
    Offlane,
    /// With the hard lane.
    Roamer,
    /// With the safe lane.
    Support,
}

/// How many roles there are.
pub const ROLES: usize = 5;

impl Role {
    /// The role a number names, counting from one as they are spoken of.
    pub fn of(number: u8) -> Option<Role> {
        Some(match number {
            1 => Role::Carry,
            2 => Role::Mid,
            3 => Role::Offlane,
            4 => Role::Roamer,
            5 => Role::Support,
            _ => return None,
        })
    }

    /// Its number, counting from one.
    pub fn number(self) -> u8 {
        match self {
            Role::Carry => 1,
            Role::Mid => 2,
            Role::Offlane => 3,
            Role::Roamer => 4,
            Role::Support => 5,
        }
    }

    /// Which lane it belongs in, for a side.
    pub fn lane(self, team: Team) -> Which {
        let bottom_is_safe = team == Team::Radiant;
        match self {
            Role::Mid => Which::Mid,
            Role::Carry | Role::Support => {
                if bottom_is_safe {
                    Which::Bottom
                } else {
                    Which::Top
                }
            }
            Role::Offlane | Role::Roamer => {
                if bottom_is_safe {
                    Which::Top
                } else {
                    Which::Bottom
                }
            }
        }
    }
}

/// One of the three lanes, named as the map has them rather than as a side
/// sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// Up the one edge and along the other.
    Top,
    /// Straight between the fountains.
    Mid,
    /// Round the other corner.
    Bottom,
}

/// Every lane there is.
pub const LANES: [Which; 3] = [Which::Top, Which::Mid, Which::Bottom];

/// A lane as a line, from one side's fountain to the other's.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// Which lane this is.
    pub which: Which,
    /// The spots it runs through, the reading side's own end first.
    pub route: Vec<Vec2>,
    /// The frontmost standing tower of the reading side on this lane.
    pub mine: Option<Vec2>,
    /// The frontmost standing tower of the other side on this lane.
    pub theirs: Option<Vec2>,
}

impl Lane {
    /// Lays out a lane from the buildings a snapshot shows.
    ///
    /// `None` when the snapshot shows no fountains, which is the state
    /// before a match has started.
    pub fn read(view: &WorldView, which: Which, team: Team) -> Option<Lane> {
        let radiant = fountain(view, Team::Radiant)?;
        let dire = fountain(view, Team::Dire)?;
        let coarse = coarse_line(which, radiant, dire);
        let mine: Vec<Vec2> = towers_of(view, which, team, radiant, dire);
        let theirs: Vec<Vec2> = towers_of(view, which, other_side(team), radiant, dire);
        let (home, away) = if team == Team::Radiant {
            (radiant, dire)
        } else {
            (dire, radiant)
        };
        let along = |at: Vec2| walked_to(&coarse, at);
        let mut mine = mine;
        let mut theirs = theirs;
        mine.sort_by(|one, other| order_by(along(*one), along(*other)));
        theirs.sort_by(|one, other| order_by(along(*one), along(*other)));
        if team == Team::Dire {
            mine.reverse();
            theirs.reverse();
        }
        let mut route = vec![home];
        route.extend(mine.iter().copied());
        if let Some(corner) = corner_of(
            which,
            mine.last().copied().unwrap_or(home),
            theirs.first().copied().unwrap_or(away),
            radiant,
            dire,
        ) {
            route.push(corner);
        }
        route.extend(theirs.iter().copied());
        route.push(away);
        Some(Lane {
            which,
            route,
            mine: mine.last().copied(),
            theirs: theirs.first().copied(),
        })
    }

    /// Where the two sides' waves meet: halfway between the frontmost tower
    /// each side still holds.
    pub fn where_they_meet(&self) -> Vec2 {
        match (self.mine, self.theirs) {
            (Some(mine), Some(theirs)) => between(mine, theirs, 0.5),
            _ => self.spot_along(0.5),
        }
    }

    /// How long the whole lane is.
    pub fn length(&self) -> f32 {
        self.route.windows(2).map(|leg| span(leg[0], leg[1])).sum()
    }

    /// The spot a share of the way along, from the reading side's own end.
    ///
    /// A share of zero is its own fountain and one is the other side's.
    pub fn spot_along(&self, share: f32) -> Vec2 {
        let want = self.length() * share.clamp(0.0, 1.0);
        let mut walked = 0.0;
        for leg in self.route.windows(2) {
            let length = span(leg[0], leg[1]);
            if walked + length >= want || length <= 0.0 {
                let part = if length > 0.0 {
                    (want - walked) / length
                } else {
                    0.0
                };
                return between(leg[0], leg[1], part);
            }
            walked += length;
        }
        self.route.last().copied().unwrap_or(Vec2::ZERO)
    }

    /// How far along the lane a spot falls, from the reading side's own end.
    pub fn how_far_along(&self, at: Vec2) -> f32 {
        walked_to(&self.route, at)
    }

    /// How far off the centerline a spot lies.
    pub fn off_the_line(&self, at: Vec2) -> f32 {
        self.route
            .windows(2)
            .map(|leg| onto(leg[0], leg[1], at).1)
            .fold(f32::MAX, f32::min)
    }

    /// The spot that many world units along the lane from a spot on it,
    /// towards the other side when the count is positive.
    pub fn spot_from(&self, at: Vec2, forward: f32) -> Vec2 {
        let length = self.length().max(1.0);
        self.spot_along((self.how_far_along(at) + forward) / length)
    }
}

/// The side that is not this one.
pub fn other_side(team: Team) -> Team {
    match team {
        Team::Radiant => Team::Dire,
        Team::Dire => Team::Radiant,
        Team::Neutral => Team::Neutral,
    }
}

/// Where a side's fountain stands.
pub fn fountain(view: &WorldView, team: Team) -> Option<Vec2> {
    view.units
        .iter()
        .find(|unit| unit.team == team && unit.kind == UnitKind::Fountain)
        .map(|unit| unit.pos)
}

/// The rough shape of a lane, from the fountains alone.
///
/// Enough to tell one lane's towers from another's, and to put them in
/// order along it.
fn coarse_line(which: Which, radiant: Vec2, dire: Vec2) -> Vec<Vec2> {
    match which {
        Which::Mid => vec![radiant, dire],
        Which::Top => vec![
            radiant,
            Vec2 {
                x: radiant.x,
                y: dire.y,
            },
            dire,
        ],
        Which::Bottom => vec![
            radiant,
            Vec2 {
                x: dire.x,
                y: radiant.y,
            },
            dire,
        ],
    }
}

/// One side's standing towers on one lane.
///
/// A tower belongs to the lane whose rough shape it stands nearest.
fn towers_of(view: &WorldView, which: Which, team: Team, radiant: Vec2, dire: Vec2) -> Vec<Vec2> {
    let lines: Vec<(Which, Vec<Vec2>)> = LANES
        .iter()
        .map(|lane| (*lane, coarse_line(*lane, radiant, dire)))
        .collect();
    view.units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Tower && unit.team == team && unit.hp > 0)
        .filter(|unit| {
            lines
                .iter()
                .min_by(|one, other| {
                    order_by(off_line(&one.1, unit.pos), off_line(&other.1, unit.pos))
                })
                .is_some_and(|(nearest, _)| *nearest == which)
        })
        .map(|unit| unit.pos)
        .collect()
}

/// Where a lane bends between the two frontmost towers, for a lane that
/// bends at all.
///
/// The bend is one of the two spots where the legs out of those towers would
/// meet, and it is the one further from the middle of the map.
fn corner_of(which: Which, mine: Vec2, theirs: Vec2, radiant: Vec2, dire: Vec2) -> Option<Vec2> {
    if which == Which::Mid {
        return None;
    }
    let middle = between(radiant, dire, 0.5);
    let one = Vec2 {
        x: mine.x,
        y: theirs.y,
    };
    let other = Vec2 {
        x: theirs.x,
        y: mine.y,
    };
    Some(if span(one, middle) >= span(other, middle) {
        one
    } else {
        other
    })
}

/// How far along a polyline a spot falls, measured from its first end.
fn walked_to(line: &[Vec2], at: Vec2) -> f32 {
    let mut walked = 0.0;
    let mut best = (f32::MAX, 0.0);
    for leg in line.windows(2) {
        let (part, off) = onto(leg[0], leg[1], at);
        let length = span(leg[0], leg[1]);
        if off < best.0 {
            best = (off, walked + length * part);
        }
        walked += length;
    }
    best.1
}

/// How far off a polyline a spot lies.
fn off_line(line: &[Vec2], at: Vec2) -> f32 {
    line.windows(2)
        .map(|leg| onto(leg[0], leg[1], at).1)
        .fold(f32::MAX, f32::min)
}

/// Where along a segment a spot falls, and how far off the line it is.
fn onto(one: Vec2, other: Vec2, at: Vec2) -> (f32, f32) {
    let (ax, ay) = (one.x.to_f32(), one.y.to_f32());
    let (bx, by) = (other.x.to_f32(), other.y.to_f32());
    let (px, py) = (at.x.to_f32(), at.y.to_f32());
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    if length <= f32::EPSILON {
        return (0.0, span(one, at));
    }
    let part = (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0);
    let (nx, ny) = (ax + dx * part, ay + dy * part);
    (part, ((px - nx) * (px - nx) + (py - ny) * (py - ny)).sqrt())
}

/// The spot a share of the way from one place to another.
pub fn between(one: Vec2, other: Vec2, share: f32) -> Vec2 {
    let (ax, ay) = (one.x.to_f32(), one.y.to_f32());
    let (bx, by) = (other.x.to_f32(), other.y.to_f32());
    Vec2::from_ints(
        (ax + (bx - ax) * share).round() as i32,
        (ay + (by - ay) * share).round() as i32,
    )
}

/// Two lengths in order, with equal ones left as they are.
pub fn order_by(one: f32, other: f32) -> std::cmp::Ordering {
    one.partial_cmp(&other).unwrap_or(std::cmp::Ordering::Equal)
}
