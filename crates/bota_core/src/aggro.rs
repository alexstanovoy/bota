//! Whom a unit picks to attack when nobody told it: the class order, and
//! the order within a class.

use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::{kind_def, rules};

/// The class order a unit ranks its targets by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityOrder {
    /// Heroes and ordinary units, then siege creeps, then buildings, then
    /// wards. What everything but a siege creep uses.
    Normal,
    /// Buildings, then siege creeps, then everything else, then wards.
    SiegeFirst,
}

/// The priority class of a target, before the class order is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetClass {
    /// Anything not in another class, a barracks included.
    Unit,
    /// A siege creep.
    Siege,
    /// A tower, an ancient or a fountain.
    Building,
    Ward,
}

/// Where a class sits in an order. Lower is taken first.
#[inline]
pub const fn class_rank(class: TargetClass, order: PriorityOrder) -> u8 {
    match order {
        PriorityOrder::Normal => match class {
            TargetClass::Unit => 0,
            TargetClass::Siege => 1,
            TargetClass::Building => 2,
            TargetClass::Ward => 3,
        },
        PriorityOrder::SiegeFirst => match class {
            TargetClass::Building => 0,
            TargetClass::Siege => 1,
            TargetClass::Unit => 2,
            TargetClass::Ward => 3,
        },
    }
}

/// Where a hero sits among equally close candidates, by whom it attacks.
///
/// Zero for one attacking a hero of the seeker's own side, two for one
/// attacking its own allies, one for everything else.
#[inline]
pub fn behaviour_rank(
    seeker_team: Team,
    candidate_team: Option<Team>,
    victim_team: Option<Team>,
    victim_kind: Option<UnitKind>,
) -> u8 {
    match (victim_team, victim_kind) {
        (Some(team), Some(UnitKind::Hero)) if team == seeker_team => 0,
        (Some(team), _) if Some(team) == candidate_team => 2,
        _ => 1,
    }
}

/// The candidate a seeker acquires among those in reach, given as
/// `(class rank, distance, key)`.
///
/// The best class decides first. Inside it the closest candidate sets the
/// mark, and everything within [`rules::AGGRO_TIE_RANGE`] of that mark counts
/// as equally close; among those, `behaviour` decides (see
/// [`behaviour_rank`]), then the distance itself, then the key.
pub fn pick_target<K: Ord + Copy>(
    found: &[(u8, i64, K)],
    behaviour: impl Fn(K) -> u8,
) -> Option<K> {
    let best_class = found.iter().map(|&(class, ..)| class).min()?;
    let nearest = found
        .iter()
        .filter(|&&(class, ..)| class == best_class)
        .map(|&(_, distance, _)| distance)
        .min()?;
    let tie = i64::from(rules::units(rules::AGGRO_TIE_RANGE).raw);
    found
        .iter()
        .filter(|&&(class, distance, _)| class == best_class && distance <= nearest + tie)
        .min_by_key(|&&(_, distance, key)| (behaviour(key), distance, key))
        .map(|&(_, _, key)| key)
}

/// What a lane creep knows of another unit when asking whether it may be
/// called on early. Every part is absent when the unit lacks it.
#[derive(Clone, Copy, Debug)]
pub struct Neighbour {
    pub team: Option<Team>,
    /// What it is.
    pub kind: Option<UnitKind>,
    /// Its building tier, from one.
    pub tier: Option<u8>,
    /// Where it stands.
    pub pos: Option<Vec2>,
    /// Whether it still lives.
    pub alive: bool,
}

/// Whether a lane creep of `side` standing at `at`, looking `reach` far, may
/// be called on by an attack order at `tick`.
///
/// Free from [`rules::FREE_AGGRO_TICK`]. Before it, only a creep that already
/// has an enemy lane creep or a neutral within its acquisition, or that
/// stands near one of its own tier-one towers.
pub fn aggroable_yet(
    tick: u32,
    side: Team,
    at: Vec2,
    reach: Fixed,
    neighbours: impl Iterator<Item = Neighbour>,
) -> bool {
    if tick >= rules::FREE_AGGRO_TICK {
        return true;
    }
    let near_home = rules::units(rules::EARLY_AGGRO_TOWER_RANGE);
    let mut at_home = false;
    for other in neighbours {
        let creepish = other.kind.is_some_and(|kind| kind_def(kind).lane_creep)
            && other.team.is_some_and(|their_side| their_side != side);
        let busy = (creepish || other.team == Some(Team::Neutral))
            && other.alive
            && other.pos.is_some_and(|pos| at.within(pos, reach));
        if busy {
            return true;
        }
        at_home |= other.kind == Some(UnitKind::Tower)
            && other.team == Some(side)
            && other.tier == Some(1)
            && other.pos.is_some_and(|pos| at.within(pos, near_home));
    }
    at_home
}
