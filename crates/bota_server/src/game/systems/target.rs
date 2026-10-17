//! Who a unit picks to attack when nobody told it.

use bota_core::{CAMPS, PriorityOrder, class_rank, deniable, isqrt64, kind_def, pick_target};
use bota_proto::{Fixed, Team, UnitKind, Vec2};

use crate::game::{Entity, Spots, UnitOrder, World};

/// Every entity that stands somewhere, by where it stood when laid, for
/// choosing targets among.
#[derive(Clone, Default)]
pub struct Candidates {
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

    pub fn priority_of(&self, entity: Entity) -> PriorityOrder {
        self.kind
            .get(entity)
            .map_or(PriorityOrder::Normal, |kind| kind_def(*kind).priority)
    }

    /// Whether one entity attacks another of its own accord.
    ///
    /// Never one of its own side, anything down, anything its side cannot
    /// see, or anything invulnerable. The jungle pays a courier no mind; a
    /// structure never takes on the jungle, and a lane creep takes on only the
    /// neutrals of a pull camp.
    pub fn hostile(&self, seeker: Entity, target: Entity) -> bool {
        self.can_see_of(seeker, target) && self.hostile_unseen(seeker, target)
    }

    /// Whether one entity would attack another of its own accord were it
    /// seen: [`hostile`](World::hostile) less the sight.
    pub fn hostile_unseen(&self, seeker: Entity, target: Entity) -> bool {
        let (Some(mine), Some(theirs)) = (
            self.team.get(seeker).copied(),
            self.team.get(target).copied(),
        ) else {
            return false;
        };
        if mine == theirs || !self.alive(target) {
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
            if seeker_kind.is_some_and(|kind| kind_def(kind).structure) {
                return false;
            }
            if seeker_kind.is_some_and(|kind| kind_def(kind).lane_creep) {
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
        let Some(UnitOrder::Attack { target, .. }) = self.orders.get(candidate).map(|o| o.current)
        else {
            return 1;
        };
        bota_core::behaviour_rank(
            seeker_team,
            self.team.get(candidate).copied(),
            self.team.get(target).copied(),
            self.kind.get(target).copied(),
        )
    }

    /// The target an entity acquires within `range`, or nothing.
    ///
    /// The best class decides first. Inside it the closest candidate sets the
    /// mark, and everything within [`bota_core::rules::AGGRO_TIE_RANGE`] of that mark
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
        self.within_reach(seeker, other, range) && self.hostile(seeker, other)
    }

    /// Whether two entities stand within `reach` of each other, edge to edge.
    pub(crate) fn within_reach(&self, one: Entity, other: Entity, reach: Fixed) -> bool {
        let (Some(here), Some(there)) = (self.pos(one), self.pos(other)) else {
            return false;
        };
        here.within(there, reach + self.hulls(one, other))
    }

    /// The bound radii of a pair added together: how far apart their
    /// centres stand when they touch edge to edge.
    pub(crate) fn hulls(&self, one: Entity, other: Entity) -> Fixed {
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
        let at = self.pos(seeker)?;
        let mut found: Vec<(u8, i64, Entity)> = Vec::new();
        for other in self.near(seeker, at, range, candidates) {
            if other == seeker || Some(other) == skip || !self.reachable(seeker, range, other) {
                continue;
            }
            let (Some(their_at), Some(kind)) = (self.pos(other), self.kind.get(other).copied())
            else {
                continue;
            };
            let class = class_rank(kind_def(kind).target_class, order);
            let distance = isqrt64(at.distance_squared(their_at));
            found.push((class, distance, other));
        }
        pick_target(&found, |other| self.behaviour_rank(side, other))
    }

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
        let (Some(kind), Some(health), Some(stats)) = (
            self.kind.get(entity).copied(),
            self.health.get(entity),
            self.stats.get(entity),
        ) else {
            return false;
        };
        deniable(kind, health.hp, stats.max_hp)
    }

    /// Whether the attacker's side sees the other one.
    pub fn can_see_of(&self, attacker: Entity, on: Entity) -> bool {
        self.team
            .get(attacker)
            .copied()
            .is_some_and(|side| self.can_see(side, on))
    }
}
