//! What the map's objectives keep: an Outpost's home side and a capture
//! under way at it, and the standing-zone countdown a Shrine runs.

use bota_proto::{Fixed, LotusTier, Team};

use crate::engine::state_hash;

/// An Outpost of the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outpost {
    /// The side on whose half it stands: the other side may capture it
    /// only once [`bota_core::OUTPOST_OPENS`] holds for this one.
    pub home: Team,
}

/// A capture under way at an Outpost: present while a side's heroes
/// channel at it, gone when the last of them stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The side channelling.
    pub team: Team,
    /// Ticks of channel gathered: one a tick for each of its heroes
    /// channelling, so that several share the time.
    pub progress: u32,
}

/// What a Lotus Pool holds and how its row of plucks goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LotusPool {
    /// Which lotus it holds: one kind at a time.
    pub tier: LotusTier,
    /// How many lotuses it holds.
    pub lotuses: u8,
    /// Lotuses plucked in a row by heroes that have not left it since: the
    /// next takes less time for each.
    pub streak: u8,
    /// Lotuses ever plucked from it, which keys the draw of who gets the next.
    pub plucked: u32,
}

/// A countdown that runs while one side alone stands within `radius` of its
/// keeper, and back down while it is contested, empty or the other side's
/// alone (Dota 7.41's Shrines of Wisdom and Lotus Pools: "an enemy hero
/// inside the radius makes the countdown run backwards").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    /// How near a hero must stand to count.
    pub radius: Fixed,
    /// Ticks the countdown runs to its end.
    pub ticks: u32,
    /// The side the countdown runs for. Absent while it stands at zero.
    pub side: Option<Team>,
    /// Ticks run so far.
    pub progress: u32,
}

state_hash!(LotusTier as u8: LotusTier::Healing, LotusTier::Great);
state_hash!(LotusPool {
    tier,
    lotuses,
    streak,
    plucked
});
state_hash!(Outpost { home });
state_hash!(Capture { team, progress });
state_hash!(Zone {
    radius,
    ticks,
    side,
    progress
});
