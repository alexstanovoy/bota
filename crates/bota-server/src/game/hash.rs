//! A fingerprint of the world, for telling two runs apart.
//!
//! Everything the simulation acts on goes in, walked in slot order and never
//! through a hash map, so the same run always gives the same number.

use crate::engine::Fnv;
use crate::game::{
    ActionPhase, ActionState, Hit, HitEffect, Inventory, ItemStack, ModifierKind, Target, World,
};

impl World {
    /// A fingerprint of everything a tick acts on.
    ///
    /// Two worlds that agree here have agreed on every position, pool, order
    /// and timer; two that differ have diverged somewhere.
    pub fn hash(&self) -> u64 {
        let mut fnv = Fnv::new();
        fnv.u32(self.tick);
        fnv.some(self.winner.is_some());
        if let Some(winner) = self.winner {
            fnv.team(winner);
        }
        for byte in self.rng.seed() {
            fnv.u8(*byte);
        }
        for draws in self.rng.global_draws() {
            fnv.u64(draws);
        }
        fnv.u32(self.uphill_miss.len() as u32);
        for chance in &self.uphill_miss {
            fnv.some(chance.is_some());
            if let Some(chance) = chance {
                let (draws, failures) = chance.state();
                fnv.u64(draws);
                fnv.u8(failures);
            }
        }
        for stream in [&self.crit, &self.evasion, &self.pierce] {
            fnv.u32(stream.len() as u32);
            for chance in stream {
                fnv.some(chance.is_some());
                if let Some(chance) = chance {
                    let (draws, at) = chance.state();
                    fnv.u64(draws);
                    fnv.u8(at);
                    fnv.u8(chance.current().num());
                    fnv.u8(chance.current().den());
                }
            }
        }
        fnv.u32(self.hits.len() as u32);
        for hit in &self.hits {
            hash_hit(&mut fnv, hit);
        }
        for entity in self.entities.iter() {
            fnv.entity(entity);
            if let Some(kind) = self.kind.get(entity) {
                fnv.kind(*kind);
            }
            if let Some(side) = self.team.get(entity) {
                fnv.team(*side);
            }
            if let Some(at) = self.transform.get(entity) {
                fnv.vec2(at.pos);
                fnv.angle(at.facing);
            }
            if let Some(health) = self.health.get(entity) {
                fnv.fixed(health.hp);
            }
            if let Some(mana) = self.mana.get(entity) {
                fnv.fixed(mana.mana);
            }
            if let Some(Target(on)) = self.target.get(entity) {
                fnv.entity(*on);
            }
            if let Some(action) = self.action.get(entity) {
                fnv.u32(action.attack_cooldown);
                match action.state {
                    ActionState::Ready => fnv.u8(0),
                    ActionState::Attack { target, phase } => {
                        fnv.u8(1);
                        fnv.entity(target);
                        hash_phase(&mut fnv, phase);
                    }
                    ActionState::CastAbility {
                        target,
                        slot,
                        phase,
                    } => {
                        fnv.u8(2);
                        hash_aim(&mut fnv, target);
                        fnv.u8(slot.0);
                        hash_phase(&mut fnv, phase);
                    }
                    ActionState::UseItem {
                        target,
                        slot,
                        phase,
                    } => {
                        fnv.u8(3);
                        hash_aim(&mut fnv, target);
                        fnv.u8(slot.0);
                        hash_phase(&mut fnv, phase);
                    }
                }
            }
            if let Some(march) = self.march.get(entity) {
                fnv.u32(u32::from(march.next));
            }
            if let Some(motion) = self.motion.get(entity) {
                fnv.vec2(motion.delta);
                fnv.u32(motion.stalled);
                fnv.u32(motion.still);
                fnv.u32(motion.wait_until);
                fnv.u32(motion.relaid);
                fnv.u32(motion.bumped);
                fnv.u32(motion.bumps);
            }
            if let Some(route) = self.route.get(entity) {
                fnv.u32(route.corners.len() as u32);
                fnv.some(route.goal.is_some());
                if let Some(goal) = route.goal {
                    fnv.vec2(goal);
                }
                fnv.vec2(route.end);
                fnv.some(route.done);
            }
            if let Some(plan) = self.plan.get(entity) {
                fnv.u32(plan.steps.len() as u32);
                fnv.u32(plan.at as u32);
                fnv.u32(plan.from);
                fnv.u32(plan.laid);
            }
            if let Some(shot) = self.projectile.get(entity) {
                fnv.fixed(shot.speed);
                fnv.some(shot.source.is_some());
                if let Some(source) = shot.source {
                    fnv.entity(source);
                }
                fnv.entity(shot.target);
                fnv.i32(shot.damage);
                fnv.u8(shot.kind as u8);
                fnv.some(shot.ability.is_some());
                if let Some(ability) = shot.ability {
                    fnv.u32(u32::from(ability.0));
                }
                fnv.u8(shot.launch_tier);
                fnv.some(shot.can_miss_uphill);
                fnv.some(shot.crit);
                fnv.some(shot.pierces);
                fnv.i32(shot.pierce_damage);
                fnv.u8(shot.bounces_left);
                fnv.i32(shot.bounce_range);
                fnv.u32(shot.bounced.len() as u32);
                for target in &shot.bounced {
                    fnv.entity(*target);
                }
            }
            if let Some(seen) = self.visibility.get(entity) {
                fnv.u8(seen.bits());
            }
            if let Some(on_it) = self.modifiers.get(entity) {
                fnv.u32(on_it.0.len() as u32);
                for held in on_it.0.iter() {
                    hash_modifier_kind(&mut fnv, held.kind);
                    fnv.some(held.source.is_some());
                    if let Some(source) = held.source {
                        fnv.entity(source);
                    }
                    fnv.some(held.ticks_left.is_some());
                    fnv.u32(held.ticks_left.unwrap_or(0));
                }
            }
            if let Some(bag) = self.inventory.get(entity) {
                hash_bag(&mut fnv, bag);
            }
            fnv.some(self.loot.get(entity).is_some());
            if let Some(loot) = self.loot.get(entity) {
                hash_stack(&mut fnv, &loot.0);
            }
            if let Some(hook) = self.hook.get(entity) {
                fnv.entity(hook.owner);
                fnv.vec2(hook.aim);
                fnv.fixed(hook.reach_left);
                fnv.some(hook.caught.is_some());
                if let Some(caught) = hook.caught {
                    fnv.entity(caught);
                }
                fnv.some(hook.returning);
                for link in hook.links {
                    fnv.entity(link);
                }
            }
            if let Some(mark) = self.mark.get(entity) {
                fnv.u32(u32::from(mark.ability.0));
                fnv.entity(mark.owner);
            }
            if let Some(line) = self.requiem_line.get(entity) {
                fnv.entity(line.owner);
                fnv.vec2(line.aim);
                fnv.fixed(line.speed);
                fnv.fixed(line.travelled);
                fnv.fixed(line.distance);
                fnv.i32(line.damage);
                fnv.i32(line.slow_pct);
                fnv.u32(line.struck.len() as u32);
                for crossed in &line.struck {
                    fnv.entity(*crossed);
                }
            }
            if let Some(ai) = self.neutral_ai.get(entity) {
                fnv.some(ai.awake);
                fnv.some(ai.roused_by.is_some());
                if let Some(by) = ai.roused_by {
                    fnv.entity(by);
                }
            }
            if let Some(gathered) = self.stacks.get(entity) {
                for (kind, many) in gathered.held() {
                    fnv.u32(kind.at() as u32);
                    fnv.u32(many);
                }
            }
        }
        for index in self.trees.felled() {
            fnv.u32(index);
        }
        for tree in self.trees.planted() {
            fnv.vec2(tree.at);
            fnv.u32(tree.until);
        }
        for seat in self.seats.iter() {
            fnv.u32(u32::from(seat.slot.0));
            fnv.i32(seat.gold);
            fnv.i32(seat.xp);
            fnv.u8(seat.level);
            fnv.u32(seat.respawn_left);
            fnv.u32(u32::from(seat.kills));
            fnv.u32(u32::from(seat.deaths));
            fnv.u32(u32::from(seat.last_hits));
            fnv.u32(u32::from(seat.denies));
            hash_bag(&mut fnv, &seat.stash);
            for (item, left) in seat.item_clocks.iter() {
                fnv.u32(u32::from(item.0));
                fnv.u32(*left);
            }
            fnv.some(seat.courier.is_some());
            if let Some(courier) = seat.courier {
                fnv.entity(courier);
            }
            fnv.u32(seat.courier_left);
            fnv.some(seat.courier_kept.is_some());
            if let Some(bag) = &seat.courier_kept {
                hash_bag(&mut fnv, bag);
            }
            fnv.some(seat.kept.is_some());
            if let Some(kept) = &seat.kept {
                for slot in kept.book.slots.iter() {
                    fnv.u32(u32::from(slot.id.0));
                    fnv.u8(slot.level);
                    fnv.u32(slot.cooldown);
                }
                hash_bag(&mut fnv, &kept.bag);
                for (kind, many) in kept.stacks.held() {
                    fnv.u32(kind.at() as u32);
                    fnv.u32(many);
                }
            }
        }
        fnv.done()
    }
}

/// What a cast was aimed at.
fn hash_aim(fnv: &mut Fnv, target: bota_proto::Target) {
    match target {
        bota_proto::Target::None => fnv.u8(0),
        bota_proto::Target::Pos(pos) => {
            fnv.u8(1);
            fnv.vec2(pos);
        }
        bota_proto::Target::Unit(id) => {
            fnv.u8(2);
            fnv.u32(id.idx);
            fnv.u32(id.generation);
        }
    }
}

/// Which phase an action stands in and how far into it.
fn hash_phase(fnv: &mut Fnv, phase: ActionPhase) {
    match phase {
        ActionPhase::Before { progress } => {
            fnv.u8(0);
            fnv.u32(progress);
        }
        ActionPhase::During { progress } => {
            fnv.u8(1);
            fnv.u32(progress);
        }
        ActionPhase::After { progress } => {
            fnv.u8(2);
            fnv.u32(progress);
        }
    }
}

/// A queued blow and its pending modifier before resolution.
fn hash_hit(fnv: &mut Fnv, hit: &Hit) {
    fnv.some(hit.source.is_some());
    if let Some(source) = hit.source {
        fnv.entity(source);
    }
    fnv.entity(hit.target);
    fnv.i32(hit.amount);
    fnv.u8(hit.kind as u8);
    fnv.some(hit.crit);
    fnv.some(hit.attack);
    fnv.some(hit.pierces);
    match hit.effect {
        HitEffect::None => fnv.u8(0),
        HitEffect::Shadowraze { level } => {
            fnv.u8(1);
            fnv.u8(level);
        }
    }
}

/// One modifier kind and everything it carries.
fn hash_modifier_kind(fnv: &mut Fnv, kind: ModifierKind) {
    match kind {
        ModifierKind::Haste { speed } => {
            fnv.u8(0);
            fnv.i32(speed);
        }
        ModifierKind::Phased => fnv.u8(9),
        ModifierKind::Mending { per_tick, breaks } => {
            fnv.u8(1);
            fnv.i32(per_tick);
            fnv.some(breaks);
        }
        ModifierKind::Clarity { per_tick, breaks } => {
            fnv.u8(2);
            fnv.i32(per_tick);
            fnv.some(breaks);
        }
        ModifierKind::Fountain {
            hp_per_tick,
            mana_per_tick,
        } => {
            fnv.u8(3);
            fnv.i32(hp_per_tick);
            fnv.i32(mana_per_tick);
        }
        ModifierKind::Stunned => fnv.u8(4),
        ModifierKind::Shielded => fnv.u8(8),
        ModifierKind::Slowed { pct } => {
            fnv.u8(5);
            fnv.i32(pct);
        }
        ModifierKind::Hastened { pct } => {
            fnv.u8(7);
            fnv.i32(pct);
        }
        ModifierKind::Guarded {
            armor,
            hp_per_second,
        } => {
            fnv.u8(12);
            fnv.i32(armor);
            fnv.i32(hp_per_second);
        }
        ModifierKind::Inspired { hp_per_second } => {
            fnv.u8(13);
            fnv.i32(hp_per_second);
        }
        ModifierKind::ArmorBroken { armor } => {
            fnv.u8(10);
            fnv.i32(armor);
        }
        ModifierKind::Burning {
            amount,
            kind,
            lethal,
        } => {
            fnv.u8(6);
            fnv.i32(amount);
            fnv.u8(kind as u8);
            fnv.some(lethal);
        }
        ModifierKind::Shadowraze { stacks } => {
            fnv.u8(11);
            fnv.u8(stacks);
        }
        ModifierKind::Rot { level } => {
            fnv.u8(14);
            fnv.u8(level);
        }
        ModifierKind::Feared => fnv.u8(15),
    }
}

/// Every slot of a bag, empty ones counted so slots keep their numbers.
fn hash_bag(fnv: &mut Fnv, bag: &Inventory) {
    for slot in bag.slots.iter() {
        match slot {
            None => fnv.u8(0),
            Some(stack) => {
                fnv.u8(1);
                hash_stack(fnv, stack);
            }
        }
    }
}

/// Complete charge, ownership, sale, attribute and timer state of one stack.
fn hash_stack(fnv: &mut Fnv, stack: &ItemStack) {
    fnv.u32(u32::from(stack.id.0));
    fnv.u8(stack.charges);
    fnv.u32(stack.cooldown);
    fnv.u32(stack.mute);
    fnv.u32(stack.bought_tick);
    fnv.u8(u8::from(stack.touched));
    fnv.u8(stack.owner.0);
    fnv.some(stack.for_sale);
    fnv.some(stack.mode.is_some());
    if let Some(mode) = stack.mode {
        fnv.u8(mode as u8);
    }
}
