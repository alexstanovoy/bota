//! Who a unit picks to attack when nobody told it.

use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::game::{CAMPS, Spots, isqrt64, rules};
use crate::game::{Entity, World, is_lane_creep, is_structure};

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
enum TargetClass {
    /// Anything not in another class, a barracks included.
    Unit,
    /// A siege creep.
    Siege,
    /// A tower, an ancient or a fountain.
    Building,
    /// A ward.
    Ward,
}

/// Which class a kind of unit falls in.
fn class_of(kind: UnitKind) -> TargetClass {
    match kind {
        UnitKind::CreepSiege => TargetClass::Siege,
        UnitKind::Tower | UnitKind::Ancient | UnitKind::Fountain => TargetClass::Building,
        UnitKind::Ward => TargetClass::Ward,
        _ => TargetClass::Unit,
    }
}

/// Where a kind of unit sits in an order. Lower is taken first.
pub fn class_rank_of(kind: UnitKind, order: PriorityOrder) -> u8 {
    class_rank(class_of(kind), order)
}

/// Where a class sits in an order. Lower is taken first.
fn class_rank(class: TargetClass, order: PriorityOrder) -> u8 {
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

/// Every entity that stands somewhere, by where it stood when laid, for
/// choosing targets among.
#[derive(Default)]
pub struct Candidates {
    /// Every entity that stands somewhere.
    spots: Spots<Entity>,
    /// The largest bound radius of any entity laid, in raw units.
    widest: i64,
}

/// Whether the camp at this spot is one lane creeps will fight.
pub fn pullable_camp(pos: Vec2) -> bool {
    CAMPS.iter().any(|camp| camp.pullable && camp.pos == pos)
}

impl World {
    /// Where every entity stands now, for choosing targets among.
    pub fn candidates(&self) -> Candidates {
        let mut candidates = Candidates::default();
        self.lay_candidates(&mut candidates);
        candidates
    }

    /// Lays candidates afresh from where every entity stands now.
    pub fn lay_candidates(&self, candidates: &mut Candidates) {
        candidates.spots.clear();
        candidates.widest = 0;
        for entity in self.entities.iter() {
            let Some(at) = self.transform.get(entity) else {
                continue;
            };
            candidates.spots.push(at.pos, entity);
            let bound = self.hull.get(entity).map_or(0, |hull| hull.bound.raw);
            candidates.widest = candidates.widest.max(i64::from(bound).abs());
        }
        candidates.spots.sort();
    }

    /// Every candidate that may stand within a range of a seeker at a spot,
    /// edge to edge; the range itself is checked by the caller.
    pub(crate) fn near<'a>(
        &self,
        seeker: Entity,
        at: Vec2,
        range: Fixed,
        candidates: &'a Candidates,
    ) -> impl Iterator<Item = Entity> + 'a {
        let own = self.hull.get(seeker).map_or(0, |hull| hull.bound.raw);
        let reach = i64::from(range.raw).abs() + i64::from(own).abs() + candidates.widest;
        candidates.spots.around(at, reach)
    }

    /// The class order a unit ranks targets by.
    pub fn priority_of(&self, entity: Entity) -> PriorityOrder {
        if self.kind.get(entity) == Some(&UnitKind::CreepSiege) {
            PriorityOrder::SiegeFirst
        } else {
            PriorityOrder::Normal
        }
    }

    /// Whether one entity attacks another of its own accord.
    ///
    /// Never one of its own side, anything down, anything its side cannot
    /// see, or anything invulnerable. The jungle pays a courier no mind; a
    /// structure never takes on the jungle, and a lane creep takes on only the
    /// neutrals of a pull camp.
    pub fn hostile(&self, seeker: Entity, target: Entity) -> bool {
        let (Some(mine), Some(theirs)) = (
            self.team.get(seeker).copied(),
            self.team.get(target).copied(),
        ) else {
            return false;
        };
        if mine == theirs || !self.alive(target) {
            return false;
        }
        if !self.can_see(mine, target) {
            return false;
        }
        if self.stats.get(target).is_some_and(|s| s.invulnerable) {
            return false;
        }
        if mine == Team::Neutral && self.kind.get(target) == Some(&UnitKind::Courier) {
            return false;
        }
        if theirs == Team::Neutral {
            let seeker_kind = self.kind.get(seeker).copied();
            if seeker_kind.is_some_and(is_structure) {
                return false;
            }
            if seeker_kind.is_some_and(is_lane_creep) {
                return self
                    .camp_home
                    .get(target)
                    .is_some_and(|home| pullable_camp(home.home));
            }
        }
        true
    }

    /// Where a hero sits among equally close candidates.
    ///
    /// Zero for one attacking a hero of the seeker's own side, two for one
    /// attacking its own allies, one for everything else.
    fn behaviour_rank(&self, seeker_team: Team, candidate: Entity) -> u8 {
        if self.kind.get(candidate) != Some(&UnitKind::Hero) {
            return 1;
        }
        let Some(crate::game::UnitOrder::Attack { target, .. }) =
            self.orders.get(candidate).map(|o| o.current)
        else {
            return 1;
        };
        let their_team = self.team.get(target).copied();
        let their_kind = self.kind.get(target).copied();
        match (their_team, their_kind) {
            (Some(team), Some(UnitKind::Hero)) if team == seeker_team => 0,
            (Some(team), _) if Some(&team) == self.team.get(candidate) => 2,
            _ => 1,
        }
    }

    /// The target an entity acquires within `range`, or nothing.
    ///
    /// The best class decides first. Inside it the closest candidate sets the
    /// mark, and everything within [`rules::AGGRO_TIE_RANGE`] of that mark
    /// counts as equally close; among those, what a hero is doing decides,
    /// then the distance itself, then the entity.
    pub fn acquire(
        &self,
        seeker: Entity,
        range: Fixed,
        order: PriorityOrder,
        candidates: &Candidates,
    ) -> Option<Entity> {
        self.acquire_demoting(seeker, range, order, None, candidates)
    }

    /// The same, with one candidate put last however close it stands.
    pub fn acquire_demoting(
        &self,
        seeker: Entity,
        range: Fixed,
        order: PriorityOrder,
        demoted: Option<Entity>,
        candidates: &Candidates,
    ) -> Option<Entity> {
        self.ranked(seeker, range, order, demoted, candidates)
            .or_else(|| demoted.filter(|&last| self.reachable(seeker, range, last)))
    }

    /// Whether an entity is a candidate for another at all.
    pub fn reachable(&self, seeker: Entity, range: Fixed, other: Entity) -> bool {
        let (Some(at), Some(their_at)) = (self.transform.get(seeker), self.transform.get(other))
        else {
            return false;
        };
        at.pos
            .within(their_at.pos, range + self.hulls(seeker, other))
            && self.hostile(seeker, other)
    }

    /// The two hulls that stand between a pair, edge to edge.
    fn hulls(&self, one: Entity, other: Entity) -> Fixed {
        self.hull.get(one).map_or(Fixed::ZERO, |h| h.bound)
            + self.hull.get(other).map_or(Fixed::ZERO, |h| h.bound)
    }

    /// The best candidate by class, then closeness, then what it is doing.
    fn ranked(
        &self,
        seeker: Entity,
        range: Fixed,
        order: PriorityOrder,
        skip: Option<Entity>,
        candidates: &Candidates,
    ) -> Option<Entity> {
        let side = self.team.get(seeker).copied()?;
        let at = self.transform.get(seeker)?.pos;
        let mut best_class = u8::MAX;
        let mut nearest = i64::MAX;
        let mut found: Vec<(u8, i64, Entity)> = Vec::new();
        for other in self.near(seeker, at, range, candidates) {
            if other == seeker || Some(other) == skip {
                continue;
            }
            let Some(their_at) = self.transform.get(other).map(|t| t.pos) else {
                continue;
            };
            if !at.within(their_at, range + self.hulls(seeker, other)) {
                continue;
            }
            if !self.hostile(seeker, other) {
                continue;
            }
            let Some(kind) = self.kind.get(other).copied() else {
                continue;
            };
            let class = class_rank(class_of(kind), order);
            let distance = isqrt64(at.distance_squared(their_at));
            if class < best_class {
                best_class = class;
                nearest = distance;
            } else if class == best_class && distance < nearest {
                nearest = distance;
            }
            found.push((class, distance, other));
        }
        let tie = i64::from(rules::units(rules::AGGRO_TIE_RANGE).raw);
        found
            .into_iter()
            .filter(|&(class, distance, _)| class == best_class && distance <= nearest + tie)
            .min_by_key(|&(_, distance, other)| (self.behaviour_rank(side, other), distance, other))
            .map(|(_, _, other)| other)
    }
}

impl World {
    /// Whether one entity may be attacked by another on an order.
    ///
    /// Only what is alive and seen by the attacker's side. An enemy may unless
    /// it is invulnerable; one of your own only once it is
    /// [`deniable`](World::deniable).
    pub fn may_attack_on_order(&self, attacker: Entity, on: Entity) -> bool {
        if !self.alive(on) || !self.can_see_of(attacker, on) {
            return false;
        }
        let (Some(mine), Some(theirs)) =
            (self.team.get(attacker).copied(), self.team.get(on).copied())
        else {
            return false;
        };
        if mine != theirs {
            return !self.stats.get(on).is_some_and(|s| s.invulnerable);
        }
        self.deniable(on)
    }

    /// Whether one of your own is worn down far enough to be put out.
    ///
    /// A lane creep goes at [`rules::DENY_HP_PCT`] of what it can hold, a
    /// building only at [`rules::DENY_BUILDING_HP_PCT`]. Nothing else of your
    /// own goes at all.
    pub fn deniable(&self, entity: Entity) -> bool {
        let Some(kind) = self.kind.get(entity).copied() else {
            return false;
        };
        let share = if is_lane_creep(kind) {
            rules::DENY_HP_PCT
        } else if is_structure(kind) {
            rules::DENY_BUILDING_HP_PCT
        } else {
            return false;
        };
        let (Some(health), Some(stats)) = (self.health.get(entity), self.stats.get(entity)) else {
            return false;
        };
        i64::from(health.hp.raw) * 100 < i64::from(stats.max_hp.raw) * i64::from(share)
    }

    /// Whether the attacker's side sees the other one.
    pub fn can_see_of(&self, attacker: Entity, on: Entity) -> bool {
        self.team
            .get(attacker)
            .copied()
            .is_some_and(|side| self.can_see(side, on))
    }
}
