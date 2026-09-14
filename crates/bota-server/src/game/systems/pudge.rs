//! The rot, the dismember and what the flesh heap keeps.

use bota_proto::{DamageKind, Fixed, Target};

use crate::engine::Entity;
use crate::game::{Modifier, ModifierKind, StackKind, World, ability, rules};

impl World {
    /// Switches the rot on, or off if it already burns.
    pub fn toggle_rot(&mut self, caster: Entity, level: usize) -> bool {
        let Some(on_it) = self.modifiers.get_mut(caster) else {
            return false;
        };
        let running = on_it
            .0
            .iter()
            .position(|held| matches!(held.kind, ModifierKind::Rot { .. }));
        match running {
            Some(at) => {
                on_it.0.remove(at);
            }
            None => on_it.put(Modifier {
                kind: ModifierKind::Rot { level: level as u8 },
                source: Some(caster),
                ticks_left: None,
            }),
        }
        true
    }

    /// Takes hold of one unit within reach: it stands stunned and burning
    /// for as long as it is held, and the one holding it mends by as much.
    pub fn dismember_takes_hold(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        if !self.hostile(caster, on) || !self.in_range_of(caster, on, rules::DISMEMBER_RANGE) {
            return false;
        }
        let level = usize::from(self.carried_level(caster, ability::DISMEMBER).max(1) - 1);
        let amount = rules::DISMEMBER_DAMAGE_PER_SECOND[level] * rules::BURN_PERIOD_TICKS as i32
            / rules::TICKS_PER_SECOND as i32;
        self.put_modifier(
            on,
            Modifier {
                kind: ModifierKind::Stunned,
                source: Some(caster),
                ticks_left: None,
            },
        );
        self.put_modifier(
            on,
            Modifier {
                kind: ModifierKind::Burning {
                    amount,
                    kind: DamageKind::Pure,
                    lethal: true,
                },
                source: Some(caster),
                ticks_left: None,
            },
        );
        // What it eats it keeps: the one channelling mends by as much.
        self.put_modifier(
            caster,
            Modifier {
                kind: ModifierKind::Mending {
                    per_tick: amount * 100 / rules::BURN_PERIOD_TICKS as i32,
                    breaks: false,
                },
                source: Some(caster),
                ticks_left: None,
            },
        );
        true
    }

    /// Whether a dismember still has something to hold: its mark stands and
    /// is within reach.
    pub fn dismember_holds(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        self.alive(on) && self.in_range_of(caster, on, rules::DISMEMBER_RANGE)
    }

    /// Lets go of what a dismember held: what it put on the mark and on
    /// the one holding it is taken off.
    pub fn dismember_lets_go(&mut self, caster: Entity, target: Target) {
        if let Some(on) = self.dismember_mark(target) {
            self.take_modifier(on, ModifierKind::Stunned, Some(caster));
            self.take_modifier(
                on,
                ModifierKind::Burning {
                    amount: 0,
                    kind: DamageKind::Pure,
                    lethal: true,
                },
                Some(caster),
            );
        }
        self.take_modifier(
            caster,
            ModifierKind::Mending {
                per_tick: 0,
                breaks: false,
            },
            Some(caster),
        );
    }

    /// The unit a dismember is aimed at, while it is one.
    fn dismember_mark(&self, target: Target) -> Option<Entity> {
        let Target::Unit(target) = target else {
            return None;
        };
        self.of_wire(target)
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
}
