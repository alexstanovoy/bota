//! The rot, the dismember, the meat shield and what the flesh heap keeps.

use bota_core::{ModifierKind, dismember_bite_damage, hero_def, move_towards, per_tick, rules};
use bota_proto::{AbilityId, DamageKind, Fixed, Inflictor, Target};

use crate::game::{HitEffect, Modifier, World};

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
            Modifier::new(
                ModifierKind::Rot { level: level as u8 },
                Some(caster),
                Some(Inflictor::Ability(AbilityId::ROT)),
                None,
            ),
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

    /// Takes hold of one unit within reach: it stands stunned, bitten as it
    /// is taken and at every beat of the channel after, and the one holding
    /// it mends by as much as the bites take.
    pub fn dismember_takes_hold(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        if !self.hostile(caster, on)
            || !self.spell_reaches(AbilityId::DISMEMBER, on)
            || !self.within_reach(caster, on, rules::units(rules::DISMEMBER_RANGE))
        {
            return false;
        }
        let level = usize::from(self.carried_level(caster, AbilityId::DISMEMBER).max(1) - 1);
        let with = Some(Inflictor::Ability(AbilityId::DISMEMBER));
        self.put_modifier(
            on,
            Modifier::new(ModifierKind::Stunned, Some(caster), with, None),
        );
        let held = ModifierKind::Dismembered { level: level as u8 };
        self.put_modifier(
            on,
            Modifier::new(held, Some(caster), with, Some(rules::DISMEMBER_TICKS)),
        );
        self.dismember_bites(caster, on, level);
        if let Some(at) = self.pos(on) {
            self.spawn_mark(AbilityId::DISMEMBER, caster, at, 0);
        }
        true
    }

    /// One bite of a dismember: magic damage by the holder's strength, which
    /// heals the holder by what it takes.
    pub fn dismember_bites(&mut self, caster: Entity, on: Entity, level: usize) {
        let strength = self
            .stats
            .get(caster)
            .map_or(Fixed::ZERO, |stats| stats.attributes.strength);
        let amount = dismember_bite_damage(level, strength);
        self.push_hit_with_effect(
            Some(caster),
            on,
            amount,
            DamageKind::Magical,
            HitEffect::Dismember,
            Some(Inflictor::Ability(AbilityId::DISMEMBER)),
        );
    }

    /// Pulls what a dismember holds towards its holder, no nearer than
    /// [`rules::DISMEMBER_PULL_LIMIT`] between their centers.
    fn dismember_pulls(&mut self, caster: Entity, on: Entity) {
        let (Some(me), Some(them)) = (self.pos(caster), self.pos(on)) else {
            return;
        };
        let limit = rules::units(rules::DISMEMBER_PULL_LIMIT);
        if them.within(me, limit) {
            return;
        }
        let mut next = move_towards(
            them,
            me,
            per_tick(rules::units(rules::DISMEMBER_PULL_SPEED)),
        );
        if next.within(me, limit) {
            next = move_towards(me, them, limit);
        }
        if let Some(transform) = self.transform.get_mut(on) {
            transform.pos = next;
        }
        self.forget_walk(on);
    }

    /// Puts a meat shield on its caster for the level's time.
    pub fn cast_meat_shield(&mut self, caster: Entity, level: usize) -> bool {
        let block = ModifierKind::MeatShield {
            amount: rules::MEAT_SHIELD_BLOCK[level],
        };
        let ticks = rules::MEAT_SHIELD_TICKS[level];
        let with = Some(Inflictor::Ability(AbilityId::MEAT_SHIELD));
        self.put_modifier(
            caster,
            Modifier::new(block, Some(caster), with, Some(ticks)),
        );
        true
    }

    /// Whether a dismember still has something to hold: its mark stands and
    /// is within reach. The hold shown on it follows it.
    pub fn dismember_holds(&mut self, caster: Entity, target: Target) -> bool {
        let Some(on) = self.dismember_mark(target) else {
            return false;
        };
        self.dismember_pulls(caster, on);
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
            self.take_modifier(on, ModifierKind::Dismembered { level: 0 }, Some(caster));
        }
    }

    /// The unit a dismember is aimed at, while it is one.
    fn dismember_mark(&self, target: Target) -> Option<Entity> {
        let Target::Unit(target) = target else {
            return None;
        };
        self.of_wire(target)
    }

    /// Whether an entity is a hero whose innate is the flesh heap.
    fn keeps_flesh_heap(&self, entity: Entity) -> bool {
        self.hero
            .get(entity)
            .and_then(|hero| hero_def(*hero))
            .is_some_and(|def| def.innate == Some(AbilityId::FLESH_HEAP))
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
            if hero == fallen || !self.keeps_flesh_heap(hero) || !enemy {
                continue;
            }
            if !self.pos(hero).is_some_and(|pos| pos.within(at, reach)) {
                continue;
            }
            if let Some(on_it) = self.modifiers.get_mut(hero) {
                let kept = on_it.count_of(ModifierKind::flesh_heap);
                on_it.set_count(
                    ModifierKind::flesh_heap,
                    kept.saturating_add(1),
                    (hero, AbilityId::FLESH_HEAP),
                );
            }
        }
    }
}
