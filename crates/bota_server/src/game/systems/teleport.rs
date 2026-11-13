//! Teleporting by Town Portal Scroll: where a scroll may carry, the channel
//! both sides see, how it ends, and carrying.

use bota_core::{Scroll, clamp_to_map, fountain_pos, hero_spawn_pos, kind_def, point_along, rules};
use bota_proto::{EventKind, Fixed, Target, Team, UnitKind, Vec2};

use crate::game::{
    Entity, Event, EventVisibility, LandingBody, LandingResolution, Obstacles, Teleport, UnitOrder,
    World, wire_id,
};

/// Raw [`Fixed`] of one over the square root of two, for the diagonal spots
/// of a landing ring.
const DIAGONAL_RAW: i64 = 46_341;
/// The small clearance beyond a grounded unit's hull when placed.
const LANDING_MARGIN: Fixed = Fixed::from_ratio(1, 5);

impl World {
    /// Where `scroll` read by `user` at `target` carries: the building and
    /// the spot it lands on. Nothing when it carries nowhere.
    ///
    /// A spot on the ground carries to the allied building standing nearest
    /// it, landing on the spot itself when it lies between the scroll's
    /// least and greatest distance of the building's centre and on the nearest point of that band when it
    /// does not. A building aimed at is carried to at the band's inner edge,
    /// on its side's fountain side. Nothing aimed at is Dota's double tap: the
    /// fountain, from [`rules::SCROLL_FOUNTAIN_MIN_DISTANCE`] off it, landing
    /// where heroes come in. Where the landing spot is closed ground,
    /// an open candidate from the bounded search around it is taken.
    pub fn scroll_destination(
        &self,
        user: Entity,
        target: Target,
        scroll: Scroll,
    ) -> Option<(Entity, Vec2)> {
        let side = self.team.get(user).copied()?;
        let fountain = fountain_pos(self.map, side);
        let (building, aim) = match target {
            Target::Pos(pos) => {
                let building = self.nearest_scroll_building(side, pos)?;
                (building, self.scroll_aim(building, pos, fountain, scroll)?)
            }
            Target::Unit(id) => {
                let building = self.of_wire(id)?;
                if !self.takes_scrolls_of(building, side) {
                    return None;
                }
                let at = self.pos(building)?;
                (building, self.scroll_aim(building, at, fountain, scroll)?)
            }
            Target::None => {
                let from = self.pos(user)?;
                if !bota_core::may_scroll_to_fountain(from, fountain) {
                    return None;
                }
                let building = self.entities.iter().find(|entity| {
                    self.kind.get(*entity) == Some(&UnitKind::Fountain)
                        && self.takes_scrolls_of(*entity, side)
                })?;
                (building, hero_spawn_pos(self.map, side))
            }
        };
        Some((building, self.open_landing_for(user, aim)?))
    }

    /// Whether an entity is a standing building of `side` a scroll carries to.
    fn takes_scrolls_of(&self, entity: Entity, side: Team) -> bool {
        self.kind
            .get(entity)
            .copied()
            .is_some_and(|kind| kind_def(kind).takes_scrolls)
            && self.team.get(entity) == Some(&side)
            && self.alive(entity)
    }

    /// The standing allied building nearest a spot, the first in slot order
    /// of those equally near.
    fn nearest_scroll_building(&self, side: Team, to: Vec2) -> Option<Entity> {
        let mut best: Option<(i64, Entity)> = None;
        for entity in self.entities.iter() {
            if !self.takes_scrolls_of(entity, side) {
                continue;
            }
            let Some(at) = self.pos(entity) else {
                continue;
            };
            let far = at.distance_squared(to);
            if best.is_none_or(|(nearest, _)| far < nearest) {
                best = Some((far, entity));
            }
        }
        best.map(|(_, entity)| entity)
    }

    /// The spot aimed at, brought into the band a scroll lands in around a
    /// building. A spot on the building's very centre is pushed out towards
    /// the fountain.
    fn scroll_aim(
        &self,
        building: Entity,
        pos: Vec2,
        fountain: Vec2,
        scroll: Scroll,
    ) -> Option<Vec2> {
        let centre = self.pos(building)?;
        let near = rules::units(scroll.min_distance);
        let far = rules::units(scroll.max_distance);
        let aim = if !pos.within(centre, far) {
            point_along(centre, pos, far)
        } else if pos.within(centre, near) {
            let towards = if pos == centre { fountain } else { pos };
            point_along(centre, towards, near)
        } else {
            pos
        };
        Some(clamp_to_map(aim))
    }

    /// Open ground sought around a spot: the spot itself, else the first open
    /// candidate on rings of eight around it, [`rules::SCROLL_LANDING_STEP`]
    /// apart, out to [`rules::SCROLL_LANDING_RINGS`] of them.
    pub fn open_landing(&self, aim: Vec2) -> Option<Vec2> {
        self.search_landing(aim, |at| self.clearance.stands_clear(at))
    }

    pub(crate) fn landing_body(&self, unit: Entity) -> LandingBody {
        let radius = self
            .hull
            .get(unit)
            .map(|hull| hull.collision)
            .or_else(|| {
                self.def
                    .get(unit)
                    .map(|def| Fixed::from_int(def.0.collision))
            })
            .unwrap_or(Fixed::ZERO);
        LandingBody {
            team: self.team.get(unit).copied().unwrap_or(Team::Neutral),
            radius,
            pathing_flight: self.pathing_flight(unit),
        }
    }

    /// Whether the body's team and movement capability permit this field.
    pub(crate) fn field_placement_clear(&self, body: LandingBody, at: Vec2) -> bool {
        body.pathing_flight
            || Obstacles {
                field: &self.clearance,
                extra: &[],
                base_blockers: self.map.base_blockers,
                team: body.team,
            }
            .map_fits(at, body.radius + LANDING_MARGIN)
    }

    pub(crate) fn landing_clear(&self, unit: Entity, at: Vec2) -> bool {
        self.landing_clear_as(self.landing_body(unit), at)
    }

    fn landing_clear_as(&self, body: LandingBody, at: Vec2) -> bool {
        self.clearance.stands_clear(at) && self.field_placement_clear(body, at)
    }

    /// Resolve near the desired point in a bounded deterministic order.
    /// Both sides of a field are eligible; incoming travel does not restrict it.
    pub(crate) fn open_landing_for(&self, unit: Entity, aim: Vec2) -> Option<Vec2> {
        self.open_landing_as(self.landing_body(unit), aim)
    }

    pub(crate) fn open_landing_as(&self, body: LandingBody, aim: Vec2) -> Option<Vec2> {
        let resolved = self.resolve_landing_as(body, aim);
        resolved.cleared.then_some(resolved.at)
    }

    pub(crate) fn resolve_landing_for(&self, unit: Entity, aim: Vec2) -> LandingResolution {
        self.resolve_landing_as(self.landing_body(unit), aim)
    }

    /// Exhausted clear-space search returns a hull-inset map clamp without
    /// certifying obstacle clearance (established from installed build 6975).
    pub(crate) fn resolve_landing_as(&self, body: LandingBody, aim: Vec2) -> LandingResolution {
        if let Some(at) = self.search_landing(aim, |at| self.landing_clear_as(body, at)) {
            return LandingResolution { at, cleared: true };
        }
        let bound = rules::units(rules::MAP_SIZE);
        let inset = body.radius.raw.clamp(0, bound.raw / 2);
        let coordinate = |value: Fixed| Fixed {
            raw: value.raw.clamp(inset, bound.raw - inset),
        };
        LandingResolution {
            at: Vec2 {
                x: coordinate(aim.x),
                y: coordinate(aim.y),
            },
            cleared: false,
        }
    }

    /// Correct a new field overlap without changing unrelated placement rules.
    pub(crate) fn place_body(&self, body: LandingBody, aim: Vec2) -> Option<Vec2> {
        if self.field_placement_clear(body, aim) {
            Some(aim)
        } else {
            self.open_landing_as(body, aim)
        }
    }

    fn search_landing(&self, aim: Vec2, clear: impl Fn(Vec2) -> bool) -> Option<Vec2> {
        if clear(aim) {
            return Some(aim);
        }
        for ring in 1..=rules::SCROLL_LANDING_RINGS {
            let out = i64::from(rules::units(ring * rules::SCROLL_LANDING_STEP).raw);
            let slant = (out * DIAGONAL_RAW) >> 16;
            let offsets = [
                (out, 0),
                (slant, slant),
                (0, out),
                (-slant, slant),
                (-out, 0),
                (-slant, -slant),
                (0, -out),
                (slant, -slant),
            ];
            for (dx, dy) in offsets {
                let spot = clamp_to_map(Vec2 {
                    x: Fixed {
                        raw: aim.x.raw.saturating_add(dx as i32),
                    },
                    y: Fixed {
                        raw: aim.y.raw.saturating_add(dy as i32),
                    },
                });
                if clear(spot) {
                    return Some(spot);
                }
            }
        }
        None
    }

    /// Begins reading `scroll`, channelled for `channel` ticks: works out
    /// where it carries and shows both sides the channel. `false`, with
    /// nothing begun, when it carries nowhere.
    pub fn begin_teleport(
        &mut self,
        user: Entity,
        target: Target,
        scroll: Scroll,
        channel: u32,
    ) -> bool {
        let Some((building, landing)) = self.scroll_destination(user, target, scroll) else {
            return false;
        };
        let Some(seat) = self.seat_of(user) else {
            return false;
        };
        let lands = self.tick + channel;
        self.seats[seat].teleport = Some(Teleport {
            building,
            landing,
            started: self.tick,
            lands,
            live: true,
            sight_until: lands + rules::SCROLL_SIGHT_LINGER_TICKS,
            sight_radius: scroll.sight_radius,
        });
        true
    }

    /// Whether the channel still has somewhere to carry to: the building it
    /// was aimed at stands.
    pub fn teleport_holds(&self, user: Entity) -> bool {
        self.teleport_of(user)
            .is_some_and(|teleport| teleport.live && self.alive(teleport.building))
    }

    /// Ends a channel without carrying: the sight where it would have landed
    /// lingers its second, and both sides are told it was broken off.
    pub fn break_teleport(&mut self, user: Entity) {
        self.end_teleport(user, false);
    }

    /// Carries a hero to where its scroll lands, leaves it standing idle
    /// there, and spends the scroll. Open ground is looked for again around
    /// the landing spot, since something may have come to stand on it; where
    /// there is none the hero stays, with its scroll, and the channel is told
    /// as one that did not land.
    pub fn teleport_to(&mut self, user: Entity, slot: usize) {
        let Some(teleport) = self.teleport_of(user).filter(|teleport| teleport.live) else {
            return;
        };
        let landing = self.open_landing_for(user, teleport.landing);
        self.end_teleport(user, landing.is_some());
        let Some(landing) = landing else {
            return;
        };
        if let Some(at) = self.transform.get_mut(user) {
            at.pos = landing;
        }
        self.forget_walk(user);
        self.set_order(user, UnitOrder::Idle);
        self.spend_charge(user, slot);
    }

    /// Ends a channel under way, telling both sides on this tick whether the
    /// hero `landed`, as the [`bota_proto::TeleportView`] they were shown
    /// disappears. The sight where it lands lingers its second.
    fn end_teleport(&mut self, user: Entity, landed: bool) {
        let tick = self.tick;
        let Some(seat) = self.seat_of(user) else {
            return;
        };
        let Some(teleport) = self.seats[seat].teleport.as_mut().filter(|t| t.live) else {
            return;
        };
        teleport.live = false;
        teleport.sight_until = tick + rules::SCROLL_SIGHT_LINGER_TICKS;
        self.events.push(Event::Fact {
            kind: EventKind::Teleported {
                slot: self.seats[seat].slot,
                landed,
            },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }

    /// Forgets every teleport whose sight has run out.
    pub fn tick_teleports(&mut self) {
        let tick = self.tick;
        for seat in self.seats.iter_mut() {
            if seat
                .teleport
                .is_some_and(|teleport| !teleport.live && tick > teleport.sight_until)
            {
                seat.teleport = None;
            }
        }
    }

    /// Every spot a scroll shows its side right now, and which side.
    pub fn teleport_sights(&self) -> impl Iterator<Item = (Team, Vec2, i32)> + '_ {
        self.seats.iter().filter_map(|seat| {
            let teleport = seat.teleport?;
            (teleport.live || self.tick <= teleport.sight_until).then_some((
                seat.team,
                teleport.landing,
                teleport.sight_radius,
            ))
        })
    }

    /// Every channel under way as both sides see it, by seat.
    pub fn teleport_views(&self) -> Vec<bota_proto::TeleportView> {
        self.seats
            .iter()
            .filter_map(|seat| {
                let teleport = seat.teleport.filter(|teleport| teleport.live)?;
                assert!(teleport.lands > teleport.started);
                Some(bota_proto::TeleportView {
                    slot: seat.slot,
                    team: seat.team,
                    building: wire_id(teleport.building),
                    destination: teleport.landing,
                    ticks_left: teleport.lands.saturating_sub(self.tick).max(1),
                    ticks_total: teleport.lands - teleport.started,
                })
            })
            .collect()
    }

    fn teleport_of(&self, user: Entity) -> Option<Teleport> {
        self.seats[self.seat_of(user)?].teleport
    }
}
