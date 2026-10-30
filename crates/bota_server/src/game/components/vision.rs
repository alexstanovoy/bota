//! Who can see what.

use bota_proto::{Team, Vec2};

use crate::engine::state_hash;

/// Which sides see an entity right now, one bit a side. Worked out afresh by
/// [`visibility_system`](crate::game::visibility_system); a value put in by
/// hand is gone the next time it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visibility(u8);

impl Visibility {
    /// Seen by nobody.
    pub const NONE: Visibility = Visibility(0);

    const fn bit(team: Team) -> u8 {
        1 << (team as u8)
    }

    pub fn by(self, team: Team) -> bool {
        self.0 & Visibility::bit(team) != 0
    }

    /// Records that a side sees the entity. Naming one twice changes nothing.
    pub fn add(&mut self, team: Team) {
        self.0 |= Visibility::bit(team);
    }

    pub fn clear(&mut self) {
        self.0 = 0;
    }

    /// The bits themselves, for the world fingerprint.
    pub fn bits(self) -> u8 {
        self.0
    }
}

/// Sight an entity showed one side by striking at it: a small circle around
/// where it stood, lasting a while.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExposureCell {
    /// Where the striker stood when it struck.
    pub at: Vec2,
    /// The last tick the circle shows.
    pub until: u32,
    /// Whether it sees past height and trees: a strike made while the last
    /// one still showed.
    pub flying: bool,
}

/// What an entity has shown each side by striking at it, indexed by the
/// side shown. At most one circle a side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exposure(pub [Option<ExposureCell>; 3]);

state_hash!(Visibility(0));
state_hash!(ExposureCell { at, until, flying });
state_hash!(Exposure(0));
