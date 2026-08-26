//! Throwing a hook, and what it drags back.

use bota_proto::{DamageKind, Fixed, Target, UnitKind, Vec2};

use crate::engine::Entity;
use crate::game::{Hook, Status, StatusKind, Transform, World, is_structure, rules};
use crate::game::{facing_towards, move_towards, per_tick};

impl World {
    /// Throws a hook at a spot, no further out than it reaches.
    pub fn cast_hook(&mut self, caster: Entity, level: usize, target: Target) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let (Some(from), Some(side)) = (
            self.transform.get(caster).map(|t| t.pos),
            self.team.get(caster).copied(),
        ) else {
            return false;
        };
        let reach = rules::units(rules::HOOK_RANGE);
        let aim = move_towards(from, pos, reach);
        let hook = self.spawn();
        self.transform.insert(
            hook,
            Transform {
                pos: from,
                facing: facing_towards(from, aim),
            },
        );
        self.set_team(hook, side);
        self.hook.insert(
            hook,
            Hook {
                owner: caster,
                aim,
                speed: Fixed::from_int(rules::HOOK_SPEED),
                reach_left: reach,
                radius: rules::units(rules::HOOK_RADIUS),
                damage: rules::HOOK_DAMAGE[level],
                caught: None,
                returning: false,
            },
        );
        true
    }

    /// Runs every hook one tick on.
    ///
    /// A hook whose thrower has fallen is given up where it is, and lets go of
    /// whatever it was dragging.
    pub fn tick_hooks(&mut self) {
        for entity in self.entities.iter().collect::<Vec<_>>() {
            let Some(mut hook) = self.hook.get(entity).copied() else {
                continue;
            };
            let (Some(at), Some(home)) = (
                self.transform.get(entity).map(|t| t.pos),
                self.transform.get(hook.owner).map(|t| t.pos),
            ) else {
                self.let_go(entity);
                continue;
            };
            if !self.alive(hook.owner) {
                self.let_go(entity);
                continue;
            }
            let step = per_tick(hook.speed);
            if hook.returning {
                let next = move_towards(at, home, step);
                self.put_at(entity, next, home);
                if let Some(caught) = hook.caught {
                    self.drag(caught, next);
                }
                if next == home {
                    self.let_go(entity);
                    continue;
                }
                self.hook.insert(entity, hook);
                continue;
            }
            let next = move_towards(at, hook.aim, step);
            hook.reach_left -= step;
            self.put_at(entity, next, hook.aim);
            if let Some(caught) = self.caught_at(entity, hook.owner, next, hook.radius) {
                hook.caught = Some(caught);
                hook.returning = true;
                if self.team.get(caught).copied() != self.team.get(hook.owner).copied() {
                    self.push_hit(Some(hook.owner), caught, hook.damage, DamageKind::Pure);
                }
            } else if next == hook.aim || hook.reach_left <= Fixed::ZERO {
                hook.returning = true;
            }
            self.hook.insert(entity, hook);
        }
    }

    /// What a hook catches where it now flies.
    ///
    /// The nearest body in its way that is neither the one who threw it nor
    /// anything rooted to the map: a hook takes units, not buildings.
    fn caught_at(&self, hook: Entity, owner: Entity, at: Vec2, radius: Fixed) -> Option<Entity> {
        let mut best: Option<(i64, Entity)> = None;
        for other in self.entities.iter() {
            if other == hook || other == owner || !self.alive(other) {
                continue;
            }
            let Some(kind) = self.kind.get(other).copied() else {
                continue;
            };
            if is_structure(kind) || kind == UnitKind::Ward {
                continue;
            }
            let Some(spot) = self.transform.get(other).map(|t| t.pos) else {
                continue;
            };
            let hulls = self.hull.get(other).map_or(Fixed::ZERO, |hull| hull.radius);
            if !spot.within(at, radius + hulls) {
                continue;
            }
            let far = at.distance_squared(spot);
            if best.is_none_or(|(nearest, _)| far < nearest) {
                best = Some((far, other));
            }
        }
        best.map(|(_, caught)| caught)
    }

    /// Moves a hook and turns it the way it is going.
    fn put_at(&mut self, hook: Entity, to: Vec2, facing_at: Vec2) {
        if let Some(at) = self.transform.get_mut(hook) {
            at.facing = facing_towards(at.pos, facing_at);
            at.pos = to;
        }
    }

    /// Drags what a hook caught to where the hook now is, and holds it there.
    ///
    /// The hold is handed out afresh every tick, so it lifts on its own once
    /// the hook lets go.
    fn drag(&mut self, caught: Entity, to: Vec2) {
        if let Some(at) = self.transform.get_mut(caught) {
            at.pos = to;
        }
        let mut on_it = self.statuses.remove(caught).unwrap_or_default();
        on_it.put(Status {
            kind: StatusKind::Stunned,
            ticks_left: 2,
        });
        self.statuses.insert(caught, on_it);
        self.route.remove(caught);
    }

    /// Takes a hook out of the world.
    fn let_go(&mut self, hook: Entity) {
        self.hook.remove(hook);
        self.transform.remove(hook);
        self.team.remove(hook);
        self.despawn(hook);
    }
}
