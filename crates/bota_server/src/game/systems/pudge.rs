//! The rot, the dismember and what the flesh heap keeps.

use bota_core::{ModifierKind, rules};
use bota_proto::{AbilityId, DamageKind, Target};

use crate::game::{Modifier, StackKind, World};

use crate::engine::Entity;

impl World {
    /// Switches the rot on, or off if it already burns. On, its cloud shows
    /// where its owner stands.
    pub fn toggle_rot(&mut self, caster: Entity, level: usize) -> bool {
        let Some(on_it) = self.modifiers.get(caster) else {
            return false;
        };
        let running = on_it
            .0
            .iter()
            .any(|held| matches!(held.kind, ModifierKind::Rot { .. }));
        if running {
            self.rot_goes_out(caster);
            return true;
        }
        self.put_modifier(
            caster,
            Modifier::new(ModifierKind::Rot { level: level as u8 }, Some(caster), None),
        );
        if let Some(at) = self.pos(caster) {
            self.spawn_mark(AbilityId::ROT, caster, at, 0);
        }
        true
    }

    /// Puts an entity's rot out, and the cloud shown for it goes with it.
    pub fn rot_goes_out(&mut self, owner: Entity) {
        if let Some(on_it) = self.modifiers.get_mut(owner) {
            on_it
                .0
                .retain(|held| !matches!(held.kind, ModifierKind::Rot { .. }));
        }
        if let Some(shown) = self.mark_of(owner, AbilityId::ROT) {
            self.take_mark(shown);
        }
    }

    /// Takes hold of one unit within reach: it stands stunned and burning
    /// for as long as it is held, and the one holding it mends by as much.
    pub fn dismember_takes_hold(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        if !self.hostile(caster, on)
            || !self.within_reach(caster, on, rules::units(rules::DISMEMBER_RANGE))
        {
            return false;
        }
        let level = usize::from(self.carried_level(caster, AbilityId::DISMEMBER).max(1) - 1);
        let amount = rules::DISMEMBER_DAMAGE_PER_SECOND[level] * rules::BURN_PERIOD_TICKS as i32
            / rules::TICKS_PER_SECOND as i32;
        let burn = ModifierKind::Burning {
            amount,
            kind: DamageKind::Pure,
            lethal: true,
        };
        let mend = ModifierKind::Mending {
            per_tick: amount * 100 / rules::BURN_PERIOD_TICKS as i32,
            breaks: false,
        };
        for (who, kind) in [(on, ModifierKind::Stunned), (on, burn), (caster, mend)] {
            self.put_modifier(who, Modifier::new(kind, Some(caster), None));
        }
        if let Some(at) = self.pos(on) {
            self.spawn_mark(AbilityId::DISMEMBER, caster, at, 0);
        }
        true
    }

    /// Whether a dismember still has something to hold: its mark stands and
    /// is within reach. The hold shown on it follows it.
    pub fn dismember_holds(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        if let (Some(at), Some(shown)) = (self.pos(on), self.mark_of(caster, AbilityId::DISMEMBER))
            && let Some(transform) = self.transform.get_mut(shown)
        {
            transform.pos = at;
        }
        self.alive(on) && self.within_reach(caster, on, rules::units(rules::DISMEMBER_RANGE))
    }

    /// Lets go of what a dismember held: what it put on the mark and on
    /// the one holding it is taken off, and the hold shown on it goes.
    pub fn dismember_lets_go(&mut self, caster: Entity, target: Target) {
        if let Some(shown) = self.mark_of(caster, AbilityId::DISMEMBER) {
            self.take_mark(shown);
        }
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

    /// Feeds the flesh heap of every hero standing near an enemy hero's
    /// death, whoever brought it down. Nothing else that falls feeds one.
    pub fn feed_flesh_heaps(&mut self, fallen: Entity) {
        if !self.is_hero(fallen) {
            return;
        }
        let (Some(at), Some(theirs)) = (self.pos(fallen), self.team.get(fallen).copied()) else {
            return;
        };
        let reach = rules::units(rules::FLESH_HEAP_RANGE);
        for hero in self.entities.iter() {
            // The fallen is dead by now, so it is its side that is looked at,
            // not whether it could still be fought.
            let enemy = self.team.get(hero).is_some_and(|mine| *mine != theirs);
            if hero == fallen || self.carried_level(hero, AbilityId::FLESH_HEAP) == 0 || !enemy {
                continue;
            }
            if !self.pos(hero).is_some_and(|pos| pos.within(at, reach)) {
                continue;
            }
            let mut kept = self.stacks.get(hero).copied().unwrap_or_default();
            kept.gather(StackKind::FleshHeap, 1);
            self.stacks.insert(hero, kept);
        }
    }
}
