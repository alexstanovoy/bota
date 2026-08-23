//! Which lane is the bot's, and where along it things are.
//!
//! A role is only a number until it says where to stand. Both sides always see
//! every building, so the two fountains are enough to lay out the three lanes:
//! one straight between them and one round each corner. Which of the three
//! belongs to a role depends on the side, because the safe lane of one is the
//! hard lane of the other — that mirroring is the whole reason a role has to
//! be told apart from a lane.

use bota_proto::{Team, Vec2};

/// What a seat is there to do.
///
/// Numbered as they are spoken of. What each one means here is only which lane
/// it belongs in; everything else a role implies is for the model to find out.
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

    /// The role a name spells.
    pub fn named(name: &str) -> Option<Role> {
        Some(match name {
            "carry" => Role::Carry,
            "mid" => Role::Mid,
            "offlane" => Role::Offlane,
            "roamer" => Role::Roamer,
            "support" => Role::Support,
            _ => return None,
        })
    }

    /// How a plan spells it.
    pub fn spelling(self) -> &'static str {
        match self {
            Role::Carry => "carry",
            Role::Mid => "mid",
            Role::Offlane => "offlane",
            Role::Roamer => "roamer",
            Role::Support => "support",
        }
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

    /// Its place in a row of flags, counting from nought.
    pub fn at(self) -> usize {
        usize::from(self.number() - 1)
    }

    /// Which lane it belongs in, for a side.
    ///
    /// The safe lane of one side is the hard lane of the other, which is why
    /// this takes both.
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

/// A lane as a line, from one fountain to the other.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// The corners it runs through, the bot's own end first.
    pub route: Vec<Vec2>,
}

impl Lane {
    /// The lane a side walks, laid out from where the fountains stand.
    ///
    /// The corners are the same two spots whichever side is asking; what
    /// changes is which end the route starts at.
    pub fn of(which: Which, team: Team, radiant: Vec2, dire: Vec2) -> Lane {
        let corner = match which {
            Which::Top => Some(Vec2 {
                x: radiant.x,
                y: dire.y,
            }),
            Which::Bottom => Some(Vec2 {
                x: dire.x,
                y: radiant.y,
            }),
            Which::Mid => None,
        };
        let (home, away) = if team == Team::Radiant {
            (radiant, dire)
        } else {
            (dire, radiant)
        };
        Lane {
            route: match corner {
                None => vec![home, away],
                Some(corner) => vec![home, corner, away],
            },
        }
    }

    /// How long the whole lane is.
    pub fn length(&self) -> f32 {
        self.route
            .windows(2)
            .map(|leg| crate::span(leg[0], leg[1]))
            .sum()
    }

    /// How far off the line a spot lies.
    pub fn off_the_line(&self, at: Vec2) -> f32 {
        self.route
            .windows(2)
            .map(|leg| onto(leg[0], leg[1], at).1)
            .fold(f32::MAX, f32::min)
    }

    /// The spot a share of the way along the lane, from the bot's own end.
    ///
    /// A share of zero is its own fountain and one is the other side's.
    pub fn spot_along(&self, share: f32) -> Vec2 {
        let want = self.length() * share.clamp(0.0, 1.0);
        let mut walked = 0.0;
        for leg in self.route.windows(2) {
            let length = crate::span(leg[0], leg[1]);
            if walked + length >= want || length <= 0.0 {
                let part = if length > 0.0 {
                    (want - walked) / length
                } else {
                    0.0
                };
                let (ax, ay) = (leg[0].x.to_f32(), leg[0].y.to_f32());
                let (bx, by) = (leg[1].x.to_f32(), leg[1].y.to_f32());
                return Vec2::from_ints(
                    (ax + (bx - ax) * part).round() as i32,
                    (ay + (by - ay) * part).round() as i32,
                );
            }
            walked += length;
        }
        self.route.last().copied().unwrap_or(Vec2::from_ints(0, 0))
    }

    /// Where the two waves will meet, halfway along.
    pub fn where_they_meet(&self) -> Vec2 {
        self.spot_along(0.5)
    }

    /// How far along the lane a spot falls, from the bot's own end.
    pub fn how_far_along(&self, at: Vec2) -> f32 {
        let mut walked = 0.0;
        let mut best = (f32::MAX, 0.0);
        for leg in self.route.windows(2) {
            let (part, off) = onto(leg[0], leg[1], at);
            let length = crate::span(leg[0], leg[1]);
            if off < best.0 {
                best = (off, walked + length * part);
            }
            walked += length;
        }
        best.1
    }
}

/// Where along a segment a spot falls, and how far off the line it is.
fn onto(one: Vec2, other: Vec2, at: Vec2) -> (f32, f32) {
    let (ax, ay) = (one.x.to_f32(), one.y.to_f32());
    let (bx, by) = (other.x.to_f32(), other.y.to_f32());
    let (px, py) = (at.x.to_f32(), at.y.to_f32());
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    if length <= f32::EPSILON {
        return (0.0, crate::span(one, at));
    }
    let part = (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0);
    let (nx, ny) = (ax + dx * part, ay + dy * part);
    (part, ((px - nx) * (px - nx) + (py - ny) * (py - ny)).sqrt())
}
