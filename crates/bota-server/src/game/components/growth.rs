//! What raises an entity above the plain form of its type.

/// A hero's level, from one up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level(pub u8);

/// How many upgrade intervals a creep spawned after. Zero for the first wave
/// and for a jungle camp before the first upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Upgrades(pub u32);

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

/// What killing an entity pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounty {
    /// Gold to the killer.
    pub gold: i32,
    /// Experience shared among those near the kill.
    pub xp: i32,
}
