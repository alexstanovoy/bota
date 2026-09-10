//! One tick, read into a settled shape.
//!
//! Everything the policy is allowed to weigh comes from here, in one order
//! that does not shift from tick to tick. Bodies are ranked by nearness with
//! the handle breaking ties, so the third creep of one tick is the third
//! creep of the next.

use bota_proto::{
    AbilityId, AbilitySlot, AbilityView, Fixed, HeroId, ItemId, ItemSlot, ItemView, PlayerView,
    SlotId, Team, UnitKind, UnitView, Vec2, WorldView,
};

use crate::{Lane, Role, SHOP_RANGE, SOULS, fountain, gap_between, order_by, other_side, span};

/// One tick as the policy sees it.
#[derive(Clone, Debug)]
pub struct Field<'a> {
    /// The snapshot it was read from.
    pub view: &'a WorldView,
    /// The seat being played.
    pub seat: &'a PlayerView,
    /// The body it drives, while one is standing.
    pub me: Option<&'a UnitView>,
    /// The side it plays for.
    pub team: Team,
    /// The hero it picked.
    pub hero: HeroId,
    /// What the seat is there to do.
    pub role: Role,
    /// The lane that role holds. Absent before the buildings are up.
    pub lane: Option<Lane>,
    /// Wave creeps of the other side, nearest first.
    pub creeps: Vec<&'a UnitView>,
    /// Wave creeps of its own, nearest first.
    pub own_creeps: Vec<&'a UnitView>,
    /// Heroes of the other side, nearest first.
    pub enemies: Vec<&'a UnitView>,
    /// Heroes of its own besides itself, nearest first.
    pub allies: Vec<&'a UnitView>,
    /// Standing towers of its own, nearest first.
    pub own_towers: Vec<&'a UnitView>,
    /// Standing towers of the other side, nearest first.
    pub enemy_towers: Vec<&'a UnitView>,
    /// Everything of the other side's that can be knocked down, nearest
    /// first: towers, barracks and the ancient.
    pub enemy_works: Vec<&'a UnitView>,
    /// Its own courier, while one is standing.
    pub courier: Option<&'a UnitView>,
    /// Where its own fountain stands.
    pub home: Option<Vec2>,
    /// Where the other side's stands.
    pub away: Option<Vec2>,
}

impl<'a> Field<'a> {
    /// Reads one tick for one seat playing one role.
    ///
    /// `None` when the snapshot has no row for that seat.
    pub fn of(view: &'a WorldView, slot: SlotId, role: Role) -> Option<Field<'a>> {
        let seat = view.players.iter().find(|player| player.slot == slot)?;
        let team = seat.team;
        let foe = other_side(team);
        let me = seat
            .unit
            .and_then(|id| view.units.iter().find(|unit| unit.id == id));
        let home = fountain(view, team);
        let away = fountain(view, foe);
        let at = me.map_or(home.unwrap_or(Vec2::ZERO), |unit| unit.pos);
        let standing = |unit: &&UnitView| unit.hp > 0;

        let nearest = |take: &dyn Fn(&UnitView) -> bool| -> Vec<&'a UnitView> {
            let mut found: Vec<&UnitView> = view
                .units
                .iter()
                .filter(standing)
                .filter(|unit| take(unit))
                .collect();
            rank_by_nearness(&mut found, at);
            found
        };

        let about = |unit: &UnitView| span(unit.pos, at) <= NEARBY as f32;
        let creeps = nearest(&|unit| unit.team == foe && is_wave_creep(unit.kind) && about(unit));
        let own_creeps =
            nearest(&|unit| unit.team == team && is_wave_creep(unit.kind) && about(unit));
        let enemies = nearest(&|unit| unit.team == foe && unit.kind == UnitKind::Hero);
        let allies = nearest(&|unit| {
            unit.team == team && unit.kind == UnitKind::Hero && Some(unit.id) != seat.unit
        });
        let own_towers = nearest(&|unit| unit.team == team && unit.kind == UnitKind::Tower);
        let enemy_towers = nearest(&|unit| unit.team == foe && unit.kind == UnitKind::Tower);
        let enemy_works = nearest(&|unit| {
            unit.team == foe
                && matches!(
                    unit.kind,
                    UnitKind::Tower | UnitKind::Barracks | UnitKind::Ancient
                )
        });
        let courier = nearest(&|unit| unit.kind == UnitKind::Courier && unit.owner == Some(slot))
            .first()
            .copied();

        Some(Field {
            view,
            seat,
            me,
            team,
            hero: seat.hero,
            role,
            lane: Lane::read(view, role.lane(team), team),
            creeps,
            own_creeps,
            enemies,
            allies,
            own_towers,
            enemy_towers,
            enemy_works,
            courier,
            home,
            away,
        })
    }

    /// Where the hero stands, or its own fountain while it stands nowhere.
    pub fn at(&self) -> Vec2 {
        self.me
            .map(|unit| unit.pos)
            .or(self.home)
            .unwrap_or(Vec2::ZERO)
    }

    /// Whether the hero is standing at all.
    pub fn alive(&self) -> bool {
        self.me.is_some()
    }

    /// The side that is not its own.
    pub fn foe(&self) -> Team {
        other_side(self.team)
    }

    /// The ground between the hero and a body, edge to edge.
    pub fn gap_to(&self, other: &UnitView) -> f32 {
        self.me.map_or(f32::MAX, |me| gap_between(me, other))
    }

    /// Whether a body stands within a swing.
    pub fn in_reach(&self, other: &UnitView) -> bool {
        self.me
            .is_some_and(|me| self.gap_to(other) <= me.attack_range.to_f32())
    }

    /// Health the hero has left, as a part of the whole.
    pub fn health(&self) -> f32 {
        self.me.map_or(0.0, |me| part(me.hp, me.max_hp))
    }

    /// Mana the hero has left, as a part of the whole.
    pub fn mana(&self) -> f32 {
        self.me.map_or(0.0, |me| part(me.mana, me.max_mana))
    }

    /// Gold the seat has in hand.
    pub fn gold(&self) -> i32 {
        self.seat.gold.unwrap_or(0)
    }

    /// Whether the hero stands where buying and selling are taken.
    pub fn at_shop(&self) -> bool {
        match (self.me, self.home) {
            (Some(me), Some(home)) => me.pos.within(home, Fixed::from_int(SHOP_RANGE)),
            _ => false,
        }
    }

    /// One of the hero's ability slots, by the ability that sits in it.
    pub fn ability(&self, id: AbilityId) -> Option<(AbilitySlot, &'a AbilityView)> {
        let book =
            self.me
                .map(|me| &me.abilities)
                .or(self.seat.kit.as_ref().map(|kit| &kit.abilities))?;
        book.iter()
            .position(|slot| slot.id == id)
            .map(|at| (AbilitySlot(at as u8), &book[at]))
    }

    /// Every ability slot the hero carries, in the order they are shown.
    pub fn abilities(&self) -> &'a [AbilityView] {
        self.me.map_or(&[][..], |me| &me.abilities)
    }

    /// One of the hero's working inventory slots, by the item in it.
    ///
    /// The backpack takes no part: an item there does nothing until it is
    /// moved forward.
    pub fn item(&self, id: ItemId) -> Option<(ItemSlot, &'a ItemView)> {
        let bag = self.me.map(|me| &me.items)?;
        bag.iter()
            .take(WORN_SLOTS)
            .enumerate()
            .find_map(|(at, slot)| match slot {
                Some(held) if held.id == id => Some((ItemSlot(at as u8), held)),
                _ => None,
            })
    }

    /// How many of the bag's working slots hold nothing.
    pub fn free_slots(&self) -> usize {
        self.me.map_or(0, |me| {
            me.items
                .iter()
                .take(WORN_SLOTS)
                .filter(|s| s.is_none())
                .count()
        })
    }

    /// How many souls the hero has gathered.
    pub fn souls(&self) -> u32 {
        self.me.map_or(0, |me| {
            me.effects
                .iter()
                .find(|effect| effect.id == SOULS)
                .and_then(|effect| effect.stacks)
                .unwrap_or(0)
        })
    }

    /// Enemy heroes standing within a reach of the hero.
    pub fn foes_within(&self, reach: i32) -> impl Iterator<Item = &'a UnitView> + use<'_, 'a> {
        let at = self.at();
        let reach = reach as f32;
        self.enemies
            .iter()
            .copied()
            .filter(move |unit| span(unit.pos, at) <= reach)
    }

    /// The frontmost spot along the lane its own side holds: the standing
    /// tower nearest the other side's end.
    pub fn own_front(&self) -> Option<Vec2> {
        let lane = self.lane.as_ref()?;
        self.own_towers
            .iter()
            .map(|tower| tower.pos)
            .max_by(|one, other| order_by(lane.how_far_along(*one), lane.how_far_along(*other)))
            .or(self.home)
    }
}

/// Slots of the bag where an item works, before the backpack begins.
pub const WORN_SLOTS: usize = 6;

/// How far from the hero a creep is still one of the creeps it is dealing
/// with.
///
/// Waves on the other lanes are visible and have nothing to do with where
/// this hero stands or what it swings at.
pub const NEARBY: i32 = 2400;

/// Whether a kind is one of the creeps a lane wave is made of.
pub fn is_wave_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepRanged
            | UnitKind::CreepSiege
    )
}

/// One number as a part of another, and nought where there is no whole.
pub fn part(some: i32, whole: i32) -> f32 {
    if whole <= 0 {
        return 0.0;
    }
    some.max(0) as f32 / whole as f32
}

/// Puts bodies in order of nearness, with the handle breaking ties.
fn rank_by_nearness(bodies: &mut [&UnitView], at: Vec2) {
    bodies.sort_by(|one, other| {
        order_by(span(at, one.pos), span(at, other.pos)).then(one.id.idx.cmp(&other.id.idx))
    });
}
