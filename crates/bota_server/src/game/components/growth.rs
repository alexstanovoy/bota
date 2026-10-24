//! What raises an entity above the plain form of its type.

use bota_core::GoldRange;

/// A hero's level, from one up, and a courier's, its owner's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level(pub u8);

/// How many upgrade intervals a creep spawned after. Zero for the first wave
/// and for a jungle camp before the first upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Upgrades(pub u32);

/// Health and attack damage a creep was raised with at its spawn, on top of
/// its kind's own: a summon whose strength is its spell's level, not its
/// kind's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatBonus {
    pub max_hp: i32,
    pub damage: i32,
}

/// Which tier a building is, from one up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tier(pub u8);

/// Which lane an entity belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lane(pub u8);

/// Which of a lane's two barracks a building is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rax {
    /// Whether it is the one feeding the ranged creeps.
    pub ranged: bool,
}

/// A structure's backdoor protection: the quiet that lets it hold, and the
/// health enemy blows took from it under protection that it still mends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backdoor {
    /// Ticks since an enemy lane creep last came within reach. Kept only by
    /// the structures that watch for creeps, a tier two tower and the Ancient;
    /// the rest read the Ancient's.
    pub quiet_ticks: u32,
    /// Health still to mend.
    pub owed: bota_proto::Fixed,
}

/// What killing an entity pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounty {
    /// Gold to the killer, drawn from the range each time it is paid.
    pub gold: GoldRange,
    /// Gold its fall pays every enemy hero near it, and the killer
    /// wherever it stands, on top of `gold`. Zero for all but a flagbearer.
    pub bonus: i32,
    /// Gold its fall pays every seat of the side that brought it down, and
    /// half of it to every seat of both sides on a deny. Zero for all but a
    /// tower and a courier, which is never denied.
    pub team: i32,
    /// Experience shared among those near the kill.
    pub xp: i32,
}
