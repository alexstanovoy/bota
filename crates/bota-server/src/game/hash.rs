//! A fingerprint of the world, for telling two runs apart.
//!
//! Everything the simulation acts on goes in, walked in slot order and never
//! through a hash map, so the same run always gives the same number.

use crate::engine::Fnv;
use crate::game::{Inventory, StatusKind, Target, World};

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
            if let Some(attacking) = self.attacking.get(entity) {
                fnv.u32(attacking.cooldown);
                fnv.u32(attacking.recovering);
                fnv.some(attacking.windup.is_some());
                if let Some(windup) = attacking.windup {
                    fnv.entity(windup.target);
                    fnv.u32(windup.ticks_left);
                }
            }
            if let Some(march) = self.march.get(entity) {
                fnv.u32(u32::from(march.route_step));
                fnv.u32(march.shove);
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
            if let Some(statuses) = self.statuses.get(entity) {
                for status in statuses.0.iter() {
                    hash_status_kind(&mut fnv, status.kind);
                    fnv.u32(status.ticks_left);
                }
            }
            if let Some(bag) = self.inventory.get(entity) {
                hash_bag(&mut fnv, bag);
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
            }
            if let Some(ai) = self.neutral_ai.get(entity) {
                fnv.some(ai.awake);
                fnv.some(ai.roused_by.is_some());
                if let Some(by) = ai.roused_by {
                    fnv.entity(by);
                }
            }
            if let Some(rot) = self.rotting.get(entity) {
                fnv.u32(rot.level as u32);
            }
            if let Some(eating) = self.dismember.get(entity) {
                fnv.entity(eating.target);
                fnv.u32(eating.ticks_left);
                fnv.u32(eating.level as u32);
            }
            if let Some(gathered) = self.stacks.get(entity) {
                for (kind, many) in gathered.held() {
                    fnv.u32(kind.at() as u32);
                    fnv.u32(many);
                }
            }
            if let Some(going) = self.teleport.get(entity) {
                fnv.u32(going.ticks_left);
                fnv.vec2(going.to);
                fnv.u32(going.slot as u32);
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

/// One effect kind and everything it carries.
fn hash_status_kind(fnv: &mut Fnv, kind: StatusKind) {
    match kind {
        StatusKind::Haste { speed } => {
            fnv.u8(0);
            fnv.i32(speed);
        }
        StatusKind::Phased => fnv.u8(9),
        StatusKind::Mending { per_tick, breaks } => {
            fnv.u8(1);
            fnv.i32(per_tick);
            fnv.some(breaks);
        }
        StatusKind::Clarity { per_tick, breaks } => {
            fnv.u8(2);
            fnv.i32(per_tick);
            fnv.some(breaks);
        }
        StatusKind::Fountain {
            hp_per_tick,
            mana_per_tick,
        } => {
            fnv.u8(3);
            fnv.i32(hp_per_tick);
            fnv.i32(mana_per_tick);
        }
        StatusKind::Stunned => fnv.u8(4),
        StatusKind::Shielded => fnv.u8(8),
        StatusKind::Slowed { pct } => {
            fnv.u8(5);
            fnv.i32(pct);
        }
        StatusKind::Hastened { pct } => {
            fnv.u8(7);
            fnv.i32(pct);
        }
        StatusKind::ArmorBroken { armor } => {
            fnv.u8(10);
            fnv.i32(armor);
        }
        StatusKind::Burning {
            amount,
            kind,
            from,
            lethal,
        } => {
            fnv.u8(6);
            fnv.i32(amount);
            fnv.u8(kind as u8);
            fnv.some(from.is_some());
            if let Some(from) = from {
                fnv.entity(from);
            }
            fnv.some(lethal);
        }
    }
}

/// Every slot of a bag, empty ones counted so slots keep their numbers.
fn hash_bag(fnv: &mut Fnv, bag: &Inventory) {
    for slot in bag.slots.iter() {
        match slot {
            None => fnv.u8(0),
            Some(stack) => {
                fnv.u8(1);
                fnv.u32(u32::from(stack.id.0));
                fnv.u8(stack.charges);
                fnv.u32(stack.cooldown);
                fnv.u32(stack.mute);
                fnv.u32(stack.bought_tick);
                fnv.u8(u8::from(stack.touched));
            }
        }
    }
}
