//! Teleporting by Town Portal Scroll: where a scroll may carry, the channel
//! both sides see, and carrying.

use bota_core::{clamp_to_map, fountain_pos, hero_spawn_pos, kind_def, point_along, rules};
use bota_proto::{Fixed, Target, Team, UnitKind, Vec2};

use crate::game::{Entity, Teleport, UnitOrder, World, wire_id};

/// Raw [`Fixed`] of one over the square root of two, for the diagonal spots
/// of a landing ring.
const DIAGONAL_RAW: i64 = 46_341;

impl World {
    /// Where a scroll read by `user` at `target` carries: the building and
    /// the spot it lands on. Nothing when it carries nowhere.
    ///
    /// A spot on the ground carries to the allied building standing nearest
    /// it, landing on the spot itself when it lies between
    /// [`rules::SCROLL_MIN_DISTANCE`] and [`rules::SCROLL_MAX_DISTANCE`] of
    /// the building's centre and on the nearest point of that band when it
    /// does not. A building aimed at is carried to at the band's inner edge,
    /// on its side's fountain side. Nothing aimed at is Dota's double tap: the
    /// fountain, from [`rules::SCROLL_FOUNTAIN_MIN_DISTANCE`] off it, landing
    /// where heroes come in. Where the landing spot is closed ground the
    /// nearest open ground around it is taken.
    pub fn scroll_destination(&self, user: Entity, target: Target) -> Option<(Entity, Vec2)> {
        let side = self.team.get(user).copied()?;
        let fountain = fountain_pos(self.map, side);
        let (building, aim) = match target {
            Target::Pos(pos) => {
                let building = self.nearest_scroll_building(side, pos)?;
                (building, self.scroll_aim(building, pos, fountain)?)
            }
            Target::Unit(id) => {
                let building = self.of_wire(id)?;
                if !self.takes_scrolls_of(building, side) {
                    return None;
                }
                let at = self.pos(building)?;
                (building, self.scroll_aim(building, at, fountain)?)
            }
            Target::None => {
                let from = self.pos(user)?;
                if from.within(fountain, rules::units(rules::SCROLL_FOUNTAIN_MIN_DISTANCE)) {
                    return None;
                }
                let building = self.entities.iter().find(|entity| {
                    self.kind.get(*entity) == Some(&UnitKind::Fountain)
                        && self.takes_scrolls_of(*entity, side)
                })?;
                (building, hero_spawn_pos(self.map, side))
            }
        };
        Some((building, self.open_landing(aim)?))
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
    fn scroll_aim(&self, building: Entity, pos: Vec2, fountain: Vec2) -> Option<Vec2> {
        let centre = self.pos(building)?;
        let near = rules::units(rules::SCROLL_MIN_DISTANCE);
        let far = rules::units(rules::SCROLL_MAX_DISTANCE);
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

    /// The open ground nearest a spot: the spot itself, else the first open
    /// spot on rings of eight around it, [`rules::SCROLL_LANDING_STEP`]
    /// apart, out to [`rules::SCROLL_LANDING_RINGS`] of them.
    pub fn open_landing(&self, aim: Vec2) -> Option<Vec2> {
        if self.clearance.stands_clear(aim) {
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
                if self.clearance.stands_clear(spot) {
                    return Some(spot);
                }
            }
        }
        None
    }

    /// Begins reading a scroll: works out where it carries and shows both
    /// sides the channel. `false`, with nothing begun, when it carries
    /// nowhere.
    pub fn begin_teleport(&mut self, user: Entity, target: Target) -> bool {
        let Some((building, landing)) = self.scroll_destination(user, target) else {
            return false;
        };
        let Some(seat) = self.seat_of(user) else {
            return false;
        };
        let lands = self.tick + rules::SCROLL_CHANNEL_TICKS;
        self.seats[seat].teleport = Some(Teleport {
            building,
            landing,
            started: self.tick,
            lands,
            live: true,
            sight_until: lands + rules::SCROLL_SIGHT_LINGER_TICKS,
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
    /// lingers its second.
    pub fn break_teleport(&mut self, user: Entity) {
        let tick = self.tick;
        if let Some(seat) = self.seat_of(user)
            && let Some(teleport) = self.seats[seat].teleport.as_mut()
        {
            teleport.live = false;
            teleport.sight_until = tick + rules::SCROLL_SIGHT_LINGER_TICKS;
        }
    }

    /// Carries a hero to where its scroll lands, leaves it standing idle
    /// there, and spends the scroll. Open ground is looked for again around
    /// the landing spot, since something may have come to stand on it.
    pub fn teleport_to(&mut self, user: Entity, slot: usize) {
        let Some(teleport) = self.teleport_of(user).filter(|teleport| teleport.live) else {
            return;
        };
        self.break_teleport(user);
        let Some(landing) = self.open_landing(teleport.landing) else {
            return;
        };
        if let Some(at) = self.transform.get_mut(user) {
            at.pos = landing;
        }
        self.forget_walk(user);
        self.set_order(user, UnitOrder::Idle);
        self.spend_charge(user, slot);
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
    pub fn teleport_sights(&self) -> impl Iterator<Item = (Team, Vec2)> + '_ {
        self.seats.iter().filter_map(|seat| {
            let teleport = seat.teleport?;
            (teleport.live || self.tick <= teleport.sight_until)
                .then_some((seat.team, teleport.landing))
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

    /// The teleport of the seat whose hero `user` is.
    fn teleport_of(&self, user: Entity) -> Option<Teleport> {
        self.seats[self.seat_of(user)?].teleport
    }
}
