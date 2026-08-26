//! The rot, the dismember and what the flesh heap keeps.

use bota_proto::{DamageKind, Fixed, Target};

use crate::engine::Entity;
use crate::game::{Dismembering, Rotting, StackKind, Status, StatusKind, World, ability, rules};

impl World {
    /// Switches the rot on, or off if it already burns.
    pub fn toggle_rot(&mut self, caster: Entity, level: usize) -> bool {
        match self.rotting.get(caster) {
            Some(_) => {
                self.rotting.remove(caster);
            }
            None => {
                self.rotting.insert(caster, Rotting { level });
            }
        }
        true
    }

    /// Burns and slows everything standing in the rot, its owner included.
    ///
    /// What it does is handed out afresh every tick, so walking out of it
    /// lifts it, and switching it off lifts it everywhere at once. Its owner
    /// burns by the same amount but never to death.
    pub fn tick_rot(&mut self) {
        for owner in self.entities.iter().collect::<Vec<_>>() {
            let Some(rot) = self.rotting.get(owner).copied() else {
                continue;
            };
            if !self.alive(owner) {
                self.rotting.remove(owner);
                continue;
            }
            let (Some(at), Some(side)) = (
                self.transform.get(owner).map(|t| t.pos),
                self.team.get(owner).copied(),
            ) else {
                continue;
            };
            let amount = rules::ROT_DAMAGE_PER_SECOND[rot.level] * rules::BURN_PERIOD_TICKS as i32
                / rules::TICKS_PER_SECOND as i32;
            let reach = rules::units(rules::ROT_RADIUS);
            for other in self.entities.iter().collect::<Vec<_>>() {
                if !self.alive(other) || self.hull.get(other).is_none() {
                    continue;
                }
                if !self
                    .transform
                    .get(other)
                    .is_some_and(|t| t.pos.within(at, reach))
                {
                    continue;
                }
                let theirs = self.team.get(other).copied();
                if other == owner {
                    self.put_for_a_tick(
                        owner,
                        StatusKind::Burning {
                            amount,
                            kind: DamageKind::Magical,
                            from: None,
                            lethal: false,
                        },
                    );
                    continue;
                }
                if theirs == Some(side) {
                    continue;
                }
                self.put_for_a_tick(
                    other,
                    StatusKind::Burning {
                        amount,
                        kind: DamageKind::Magical,
                        from: Some(owner),
                        lethal: true,
                    },
                );
                self.put_for_a_tick(
                    other,
                    StatusKind::Slowed {
                        pct: rules::ROT_SLOW_PCT[rot.level],
                    },
                );
            }
        }
    }

    /// Takes hold of one unit within reach and starts eating it.
    pub fn cast_dismember(&mut self, caster: Entity, level: usize, target: Target) -> bool {
        let Target::Unit(target) = target else {
            return false;
        };
        let Some(on) = self.of_wire(target) else {
            return false;
        };
        if !self.hostile(caster, on) || !self.in_range_of(caster, on, rules::DISMEMBER_RANGE) {
            return false;
        }
        self.dismember.insert(
            caster,
            Dismembering {
                target: on,
                ticks_left: rules::DISMEMBER_TICKS,
                level,
            },
        );
        true
    }

    /// Runs every dismember one tick on.
    ///
    /// It ends when its time runs out, when what it holds is gone or walks
    /// out of reach, or when whoever is channelling it can no longer act.
    pub fn tick_dismembers(&mut self) {
        for caster in self.entities.iter().collect::<Vec<_>>() {
            let Some(mut eating) = self.dismember.get(caster).copied() else {
                continue;
            };
            let out_of_reach = !self.in_range_of(caster, eating.target, rules::DISMEMBER_RANGE);
            if !self.alive(caster)
                || self.held(caster)
                || !self.alive(eating.target)
                || out_of_reach
            {
                self.dismember.remove(caster);
                continue;
            }
            eating.ticks_left = eating.ticks_left.saturating_sub(1);
            if eating.ticks_left == 0 {
                self.dismember.remove(caster);
            } else {
                self.dismember.insert(caster, eating);
            }
            let amount = rules::DISMEMBER_DAMAGE_PER_SECOND[eating.level]
                * rules::BURN_PERIOD_TICKS as i32
                / rules::TICKS_PER_SECOND as i32;
            self.put_for_a_tick(eating.target, StatusKind::Stunned);
            self.put_for_a_tick(
                eating.target,
                StatusKind::Burning {
                    amount,
                    kind: DamageKind::Pure,
                    from: Some(caster),
                    lethal: true,
                },
            );
            // What it eats it keeps: the one channelling mends by as much.
            self.put_for_a_tick(
                caster,
                StatusKind::Mending {
                    per_tick: amount * 100 / rules::BURN_PERIOD_TICKS as i32,
                    breaks: false,
                },
            );
        }
    }

    /// Feeds the flesh heap of every hero near a death.
    ///
    /// A structure or a ward going down feeds nothing.
    pub fn feed_flesh_heaps(&mut self, fallen: Entity) {
        if self
            .kind
            .get(fallen)
            .copied()
            .is_none_or(|kind| !crate::game::leaves_a_death(kind))
        {
            return;
        }
        let Some(at) = self.transform.get(fallen).map(|t| t.pos) else {
            return;
        };
        let reach = rules::units(rules::FLESH_HEAP_RANGE);
        for hero in self.entities.iter().collect::<Vec<_>>() {
            if hero == fallen || self.heap_level(hero) == 0 {
                continue;
            }
            if !self
                .transform
                .get(hero)
                .is_some_and(|t| t.pos.within(at, reach))
            {
                continue;
            }
            let mut kept = self.stacks.get(hero).copied().unwrap_or_default();
            kept.gather(StackKind::FleshHeap, 1);
            self.stacks.insert(hero, kept);
        }
    }

    /// Which level of the flesh heap an entity has learned. Zero for one that
    /// does not carry it at all.
    pub fn heap_level(&self, entity: Entity) -> u8 {
        self.abilities.get(entity).map_or(0, |book| {
            book.slots
                .iter()
                .find(|slot| slot.id == ability::FLESH_HEAP)
                .map_or(0, |slot| slot.level)
        })
    }

    /// Whether one entity stands within a reach of another, edge to edge.
    fn in_range_of(&self, from: Entity, to: Entity, range: i32) -> bool {
        let (Some(here), Some(there)) = (
            self.transform.get(from).map(|t| t.pos),
            self.transform.get(to).map(|t| t.pos),
        ) else {
            return false;
        };
        let hulls = self.hull.get(from).map_or(Fixed::ZERO, |hull| hull.radius)
            + self.hull.get(to).map_or(Fixed::ZERO, |hull| hull.radius);
        here.within(there, rules::units(range) + hulls)
    }

    /// Puts an effect on for this tick alone.
    fn put_for_a_tick(&mut self, on: Entity, kind: StatusKind) {
        let mut on_it = self.statuses.remove(on).unwrap_or_default();
        on_it.put(Status {
            kind,
            ticks_left: 2,
        });
        self.statuses.insert(on, on_it);
    }
}
