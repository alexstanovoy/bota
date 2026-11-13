//! Throwing a hook, and what it drags back.

use bota_core::{
    ModifierKind, facing_towards, kind_def, move_towards, per_tick, point_ahead, point_along, rules,
};
use bota_proto::{AbilityId, DamageKind, Fixed, Inflictor, Target, Vec2};

use crate::game::{Hook, Modifier, Transform, World};

use crate::engine::Entity;

impl World {
    /// Throws a hook in the aimed direction, always to its full reach, and holds its thrower
    /// in its followthrough.
    pub fn cast_hook(&mut self, caster: Entity, level: usize, target: Target) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let (Some(&Transform { pos: from, facing }), Some(side)) =
            (self.transform.get(caster), self.team.get(caster).copied())
        else {
            return false;
        };
        // The hook flies as far as its cast reaches, a cast range bonus included (Liquipedia,
        // Meat Hook: cast range bonuses lengthen it).
        let reach = rules::units(self.reach_of(caster, rules::HOOK_RANGE));
        let aim = if pos == from {
            point_ahead(from, facing, reach)
        } else {
            point_along(from, pos, reach)
        };
        let damage_amp_bp = self.outgoing_spell_amp_bp(Some(caster), DamageKind::Pure);
        let with = Some(Inflictor::Ability(AbilityId::MEAT_HOOK));
        for &(kind, ticks) in bota_core::ability_def(AbilityId::MEAT_HOOK)
            .into_iter()
            .flat_map(|def| def.caster_modifiers)
        {
            self.put_modifier(caster, Modifier::new(kind, Some(caster), with, Some(ticks)));
        }
        let hook = self.spawn_placed(from, facing_towards(from, aim), side);
        let links = std::array::from_fn(|_| self.spawn_mark(AbilityId::MEAT_HOOK, caster, from, 0));
        self.hook.insert(
            hook,
            Hook {
                owner: caster,
                from,
                aim,
                speed: Fixed::from_int(rules::HOOK_SPEED),
                reach_left: reach,
                radius: rules::units(rules::HOOK_RADIUS),
                damage: self.talented(caster, rules::HOOK_DAMAGE_VALUE, level).int(),
                damage_amp_bp,
                mana: self.ability_mana_cost(caster, AbilityId::MEAT_HOOK, level as u8 + 1),
                caught: None,
                returning: false,
                links,
            },
        );
        true
    }

    /// Lays a hook's chain out evenly between the thrower and the hook.
    fn lay_chain(&mut self, hook: &Hook, home: Vec2, at: Vec2) {
        let count = rules::HOOK_LINKS as i32 + 1;
        for (index, link) in hook.links.into_iter().enumerate() {
            let along = Fixed::from_ratio(index as i32 + 1, count);
            let spot = home + (at - home) * along;
            if let Some(transform) = self.transform.get_mut(link) {
                transform.pos = spot;
            }
        }
    }

    /// Runs every hook one tick on.
    ///
    /// A hook whose thrower has fallen is given up where it is, and lets go of
    /// whatever it was dragging.
    pub fn tick_hooks(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut hook) = self.hook.get(entity).copied() else {
                continue;
            };
            let (Some(at), Some(home)) = (self.pos(entity), self.pos(hook.owner)) else {
                self.let_go(entity);
                continue;
            };
            if !self.alive(hook.owner) {
                self.let_go(entity);
                continue;
            }
            let step = per_tick(hook.speed);
            if hook.returning {
                // It comes back to where it was thrown from, wherever its
                // thrower has gone (Dota, 2013-09-23).
                let next = move_towards(at, hook.from, step);
                self.put_at(entity, next, hook.from);
                self.lay_chain(&hook, home, next);
                if let Some(caught) = hook.caught {
                    self.drag(entity, hook.owner, caught, next);
                }
                if next == hook.from {
                    self.let_go(entity);
                    continue;
                }
                self.hook.insert(entity, hook);
                continue;
            }
            // The final tick cannot spend more than the remaining range,
            // including when a tiny aim vector rounds its endpoint outward.
            let flight = step.min(hook.reach_left);
            let next = move_towards(at, hook.aim, flight);
            hook.reach_left -= flight;
            self.put_at(entity, next, hook.aim);
            self.lay_chain(&hook, home, next);
            if let Some(caught) = self.caught_at(entity, hook.owner, next, hook.radius) {
                self.hook_hit(&mut hook, caught);
            } else if next == hook.aim || hook.reach_left <= Fixed::ZERO {
                hook.returning = true;
            }
            self.hook.insert(entity, hook);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// The hook latches: its thrower's followthrough lifts at once (Dota 7.34), its side sees
    /// the spot, and what it caught is executed, or held and hurt.
    fn hook_hit(&mut self, hook: &mut Hook, caught: Entity) {
        hook.returning = true;
        for &(kind, _) in bota_core::ability_def(AbilityId::MEAT_HOOK)
            .into_iter()
            .flat_map(|def| def.caster_modifiers)
        {
            self.take_modifier(hook.owner, kind, Some(hook.owner));
        }
        // Its side sees where it latched for a while.
        if let (Some(at), Some(side)) = (self.pos(caught), self.team.get(hook.owner).copied()) {
            self.add_fow_viewer(
                side,
                at,
                rules::HOOK_VISION_RADIUS,
                rules::HOOK_VISION_TICKS,
            );
        }
        // A rune comes home with it, and its thrower has the hook's mana
        // back, what it was paid (Dota's hero data; Valve 2018-08-13).
        if let Some(rune) = self.rune.get_mut(caught) {
            rune.away = true;
            if let (Some(pool), Some(stats)) =
                (self.mana.get_mut(hook.owner), self.stats.get(hook.owner))
            {
                pool.mana = (pool.mana + Fixed::from_int(hook.mana)).min(stats.max_mana);
            }
            hook.caught = Some(caught);
            return;
        }
        let enemy = self.team.get(caught) != self.team.get(hook.owner);
        // An illusion is a creep to the hook (Dota 7.34: "Now instantly kills illusions").
        let executed = self.wears_illusion(caught)
            || self
                .kind
                .get(caught)
                .is_some_and(|kind| kind_def(*kind).creep);
        if enemy && executed {
            // A point-target HP removal is neither damage nor a unit-target
            // cast, so nearby camp mates stay asleep. Sources:
            // https://liquipedia.net/dota2/Pudge
            // https://liquipedia.net/dota2/Neutral_Creeps#Drawing_Aggro
            self.kill_by_hp_removal(hook.owner, caught);
            return;
        }
        hook.caught =
            (!self.stats.get(caught).is_some_and(|s| s.forced_move_immune)).then_some(caught);
        if hook.caught.is_some() && self.push.get(caught).is_some_and(|push| push.carries()) {
            self.push.remove(caught);
            self.forget_walk(caught);
        }
        if !enemy {
            return;
        }
        self.push_hit_with_amp(
            Some(hook.owner),
            caught,
            hook.damage,
            DamageKind::Pure,
            hook.damage_amp_bp,
            Some(Inflictor::Ability(AbilityId::MEAT_HOOK)),
        );
    }

    pub(crate) fn held_by_hook(&self, on: Entity) -> bool {
        self.entities
            .iter()
            .any(|e| self.hook.get(e).is_some_and(|h| h.caught == Some(on)))
    }

    /// What a hook catches where it now flies.
    ///
    /// The nearest living unit within its radius of the hook, edge counted,
    /// other than the one who threw it. A structure or a ward is never
    /// caught, nor an invulnerable unit, nor a spell-immune courier (Meat
    /// Hook pierces spell immunity, but not a courier's). A rune lying on the
    /// ground is.
    fn caught_at(&self, hook: Entity, owner: Entity, at: Vec2, radius: Fixed) -> Option<Entity> {
        let mut best: Option<(i64, Entity)> = None;
        for other in self.entities.iter() {
            // A rune lying on the ground is caught as well as a unit.
            let catchable =
                self.rune.get(other).is_some() || (self.alive(other) && self.latches_onto(other));
            if other == hook || other == owner || !catchable {
                continue;
            }
            let Some(spot) = self.pos(other) else {
                continue;
            };
            let hulls = self.hull.get(other).map_or(Fixed::ZERO, |hull| hull.bound);
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

    fn latches_onto(&self, unit: Entity) -> bool {
        let Some(&kind) = self.kind.get(unit) else {
            return false;
        };
        // Valve's Pudge notes say Hook passes through ancient units:
        // https://www.dota2.com/datafeed/herodata?language=english&hero_id=14
        // https://liquipedia.net/dota2/Pudge#Meat_Hook
        if kind_def(kind).fixed || self.def.get(unit).is_some_and(|def| def.0.ancient) {
            return false;
        }
        // Nothing invulnerable is latched onto (Dota 7.03), and a Glyph makes
        // a creep so.
        if self
            .stats
            .get(unit)
            .is_some_and(|stats| stats.invulnerable || (kind_def(kind).creep && stats.glyph))
        {
            return false;
        }
        self.spell_reaches(AbilityId::MEAT_HOOK, unit)
    }

    /// Moves a hook and turns it the way it is going.
    fn put_at(&mut self, hook: Entity, to: Vec2, facing_at: Vec2) {
        if let Some(at) = self.transform.get_mut(hook) {
            at.facing = facing_towards(at.pos, facing_at);
            at.pos = to;
        }
    }

    /// Drags what a hook caught to where the hook now is. An enemy is held
    /// there, stunned; an ally or a rune only moved, so what an ally channels
    /// goes on (Dota stopped the hook interrupting allied channels in 2011).
    ///
    /// The hold is handed out afresh every tick, so it lifts on its own once
    /// the hook lets go.
    fn drag(&mut self, hook: Entity, owner: Entity, caught: Entity, to: Vec2) {
        if let Some(at) = self.transform.get_mut(caught) {
            at.pos = to;
        }
        let side = self.team.get(caught);
        if side.is_some() && side != self.team.get(owner) {
            let with = Some(Inflictor::Ability(AbilityId::MEAT_HOOK));
            self.put_modifier(
                caught,
                Modifier::new(ModifierKind::Stunned, Some(hook), with, Some(2)),
            );
        }
        self.forget_walk(caught);
    }

    /// Takes a hook and its chain out of the world.
    fn let_go(&mut self, hook: Entity) {
        if let Some(flown) = self.hook.remove(hook) {
            if let Some(caught) = flown.caught.filter(|unit| self.hull.contains(*unit))
                && let Some(at) = self.pos(caught)
            {
                let landing = self.resolve_landing_for(caught, at);
                if let Some(transform) = self.transform.get_mut(caught) {
                    transform.pos = landing.at;
                }
                self.forget_walk(caught);
            }
            for link in flown.links {
                self.take_mark(link);
            }
        }
        self.hook.remove(hook);
        self.discard(hook);
    }
}
