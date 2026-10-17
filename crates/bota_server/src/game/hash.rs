//! A fingerprint of the world, for telling two runs apart.
//!
//! Everything the simulation acts on goes in, walked in slot order.

use bota_core::ModifierKind;

use crate::engine::Fnv;
use crate::game::{
    Action, ActionPhase, ActionState, AppliedModifiers, AppliedOrigin, Entity, Exposure, Hit,
    HitEffect, Hook, Inventory, ItemStack, ModifierDuration, Modifiers, Motion, Projectile,
    RequiemLine, Seat, SpawnCategory, SpawnModifier, SpawnTarget, Stacks, Target, World,
};

impl World {
    /// A fingerprint of everything a tick acts on.
    pub fn hash(&self) -> u64 {
        let mut fnv = Fnv::new();
        fnv.u32(self.tick);
        fnv.opt(self.winner, Fnv::team);
        self.hash_randomness(&mut fnv);
        fnv.u32(self.hits.len() as u32);
        for hit in &self.hits {
            hash_hit(&mut fnv, hit);
        }
        for entity in self.entities.iter() {
            fnv.entity(entity);
            self.hash_body(&mut fnv, entity);
            self.hash_effects(&mut fnv, entity);
        }
        for index in self.trees.felled() {
            fnv.u32(index);
        }
        for tree in self.trees.planted() {
            fnv.vec2(tree.at);
            fnv.u32(tree.until);
        }
        for seat in self.seats.iter() {
            hash_seat(&mut fnv, seat);
        }
        for held in self.shop_stock.iter() {
            fnv.team(held.team);
            fnv.u32(u32::from(held.item.0));
            fnv.u8(held.left);
            fnv.u32(held.restock_left);
        }
        if !self.spawn_modifiers.is_empty() {
            assert!(self.spawn_modifiers.len() <= crate::game::MAX_SPAWN_MODIFIERS);
            fnv.u8(1);
            fnv.u32(self.spawn_modifiers.len() as u32);
            for rule in &self.spawn_modifiers {
                hash_spawn_modifier(&mut fnv, rule);
            }
        }
        fnv.done()
    }

    /// The seed, the global draw counts and every per-entity chance stream.
    fn hash_randomness(&self, fnv: &mut Fnv) {
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
        for stream in [&self.evasion, &self.pierce] {
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
    }

    /// What an entity is, where it stands, its pools, and how it acts and moves.
    fn hash_body(&self, fnv: &mut Fnv, entity: Entity) {
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
            hash_action(fnv, action);
        }
        if let Some(march) = self.march.get(entity) {
            fnv.u32(u32::from(march.next));
        }
        if let Some(motion) = self.motion.get(entity) {
            hash_motion(fnv, motion);
        }
        if let Some(route) = self.route.get(entity) {
            fnv.u32(route.corners.len() as u32);
            fnv.opt(route.goal, Fnv::vec2);
            fnv.vec2(route.end);
            fnv.some(route.done);
        }
        if let Some(plan) = self.plan.get(entity) {
            fnv.u32(plan.steps.len() as u32);
            fnv.u32(plan.at as u32);
            fnv.u32(plan.from);
            fnv.u32(plan.laid);
        }
        if let Some(Exposure(cells)) = self.exposure.get(entity) {
            for cell in cells {
                fnv.some(cell.is_some());
                if let Some(cell) = cell {
                    fnv.vec2(cell.at);
                    fnv.u32(cell.until);
                    fnv.some(cell.flying);
                }
            }
        }
    }

    /// What an entity flies as, carries, and has on it.
    fn hash_effects(&self, fnv: &mut Fnv, entity: Entity) {
        if let Some(shot) = self.projectile.get(entity) {
            hash_projectile(fnv, shot);
        }
        if let Some(seen) = self.visibility.get(entity) {
            fnv.u8(seen.bits());
        }
        if let Some(on_it) = self.modifiers.get(entity) {
            hash_modifiers(fnv, on_it);
        }
        if let Some(applied) = self.applied.get(entity) {
            hash_applied(fnv, applied);
        }
        if let Some(bag) = self.inventory.get(entity) {
            hash_bag(fnv, bag);
        }
        if let Some(book) = self.abilities.get(entity) {
            fnv.u8(book.talents.bits);
            fnv.u8(book.attribute_bonus);
        }
        fnv.opt(self.loot.get(entity), |fnv, loot| hash_stack(fnv, &loot.0));
        if let Some(hook) = self.hook.get(entity) {
            hash_hook(fnv, hook);
        }
        if let Some(mark) = self.mark.get(entity) {
            fnv.u32(u32::from(mark.ability.0));
            fnv.entity(mark.owner);
        }
        if let Some(line) = self.requiem_line.get(entity) {
            hash_requiem_line(fnv, line);
        }
        if let Some(shot) = self.grenade.get(entity) {
            fnv.entity(shot.owner);
            fnv.vec2(shot.aim);
            hash_damage_amp(fnv, shot.damage_amp_bp);
        }
        if let Some(ai) = self.neutral_ai.get(entity) {
            fnv.some(ai.awake);
            fnv.opt(ai.roused_by, Fnv::entity);
        }
        if let Some(gathered) = self.stacks.get(entity) {
            hash_stacks(fnv, gathered);
        }
        if let Some(feast) = self.feast.get(entity) {
            hash_feast(fnv, feast);
        }
    }
}

/// The attack clock and whatever the entity is doing.
fn hash_action(fnv: &mut Fnv, action: &Action) {
    fnv.u32(action.attack_cooldown);
    match action.state {
        ActionState::Ready => fnv.u8(0),
        ActionState::Attack { target, phase } => {
            fnv.u8(1);
            fnv.entity(target);
            hash_phase(fnv, phase);
        }
        ActionState::CastAbility {
            target,
            slot,
            phase,
        } => hash_cast(fnv, 2, target, slot.0, phase),
        ActionState::UseItem {
            target,
            slot,
            phase,
        } => hash_cast(fnv, 3, target, slot.0, phase),
    }
}

/// A cast of an ability or a use of an item, told apart by `tag`.
fn hash_cast(fnv: &mut Fnv, tag: u8, target: bota_proto::Target, slot: u8, phase: ActionPhase) {
    fnv.u8(tag);
    hash_aim(fnv, target);
    fnv.u8(slot);
    hash_phase(fnv, phase);
}

fn hash_motion(fnv: &mut Fnv, motion: &Motion) {
    fnv.vec2(motion.delta);
    fnv.u32(motion.stalled);
    fnv.u32(motion.still);
    fnv.u32(motion.wait_until);
    fnv.u32(motion.relaid);
    fnv.u32(motion.bumped);
    fnv.u32(motion.bumps);
    fnv.opt(motion.queued_behind, |fnv, (behind, tick)| {
        fnv.entity(behind);
        fnv.u32(tick);
    });
}

fn hash_projectile(fnv: &mut Fnv, shot: &Projectile) {
    fnv.fixed(shot.speed);
    fnv.opt(shot.source, Fnv::entity);
    fnv.entity(shot.target);
    fnv.i32(shot.damage);
    fnv.u8(shot.kind as u8);
    hash_damage_amp(fnv, shot.damage_amp_bp);
    fnv.u8(shot.launch_tier);
    fnv.some(shot.can_miss_uphill);
    fnv.some(shot.pierces);
    fnv.i32(shot.pierce_damage);
    hash_damage_amp(fnv, shot.pierce_amp_bp);
}

fn hash_modifiers(fnv: &mut Fnv, on_it: &Modifiers) {
    fnv.u32(on_it.0.len() as u32);
    for held in on_it.0.iter() {
        hash_modifier_kind(fnv, held.kind);
        fnv.opt(held.source, Fnv::entity);
        fnv.some(held.ticks_left.is_some());
        fnv.u32(held.ticks_left.unwrap_or(0));
    }
}

fn hash_applied(fnv: &mut Fnv, applied: &AppliedModifiers) {
    fnv.u8(1);
    fnv.u32(applied.iter().count() as u32);
    for held in applied.iter() {
        hash_modifier_spec(fnv, held.spec);
        fnv.some(held.ticks_left.is_some());
        fnv.u32(held.ticks_left.unwrap_or(0));
        fnv.u8(match held.origin {
            AppliedOrigin::Setup => 0,
            AppliedOrigin::Cheat => 1,
        });
    }
}

fn hash_hook(fnv: &mut Fnv, hook: &Hook) {
    fnv.entity(hook.owner);
    fnv.vec2(hook.aim);
    fnv.fixed(hook.reach_left);
    hash_damage_amp(fnv, hook.damage_amp_bp);
    fnv.opt(hook.caught, Fnv::entity);
    fnv.some(hook.returning);
    for link in hook.links {
        fnv.entity(link);
    }
}

fn hash_requiem_line(fnv: &mut Fnv, line: &RequiemLine) {
    fnv.entity(line.owner);
    fnv.vec2(line.aim);
    fnv.fixed(line.speed);
    fnv.fixed(line.travelled);
    fnv.fixed(line.distance);
    fnv.i32(line.damage);
    hash_damage_amp(fnv, line.damage_amp_bp);
    fnv.u8(line.level);
    fnv.some(line.frightens);
    fnv.u32(line.fear.line);
    fnv.u32(line.fear.most);
    fnv.u32(line.struck.len() as u32);
    for crossed in &line.struck {
        fnv.entity(*crossed);
    }
}

fn hash_feast(fnv: &mut Fnv, feast: &crate::game::Feast) {
    fnv.u8(feast.level);
    fnv.u8(feast.phase as u8);
    fnv.u32(feast.ticks_left);
    fnv.u32(feast.fed.len() as u32);
    for fed in &feast.fed {
        fnv.entity(fed.from);
        fnv.u32(fed.souls);
    }
}

fn hash_stacks(fnv: &mut Fnv, gathered: &Stacks) {
    for (kind, many) in gathered.held() {
        fnv.u32(kind.at() as u32);
        fnv.u32(many);
    }
}

/// A seat's purse, tallies, stash, item clocks, courier and kept hero state.
fn hash_seat(fnv: &mut Fnv, seat: &Seat) {
    fnv.u32(u32::from(seat.slot.0));
    fnv.i32(seat.gold);
    fnv.i32(seat.xp);
    fnv.u8(seat.level);
    fnv.u32(seat.respawn_left);
    fnv.u32(u32::from(seat.kills));
    fnv.u32(u32::from(seat.deaths));
    fnv.u32(u32::from(seat.last_hits));
    fnv.u32(u32::from(seat.denies));
    hash_bag(fnv, &seat.stash);
    for (item, left) in seat.item_clocks.iter() {
        fnv.u32(u32::from(item.0));
        fnv.u32(*left);
    }
    fnv.opt(seat.courier, Fnv::entity);
    fnv.u32(seat.courier_left);
    fnv.opt(seat.courier_kept.as_ref(), hash_bag);
    fnv.some(seat.kept.is_some());
    if let Some(kept) = &seat.kept {
        for slot in kept.book.slots.iter() {
            fnv.u32(u32::from(slot.id.0));
            fnv.u8(slot.level);
            fnv.u32(slot.cooldown);
        }
        fnv.u8(kept.book.talents.bits);
        fnv.u8(kept.book.attribute_bonus);
        hash_bag(fnv, &kept.bag);
        hash_stacks(fnv, &kept.stacks);
    }
    fnv.some(seat.teleport.is_some());
    if let Some(teleport) = seat.teleport {
        fnv.entity(teleport.building);
        fnv.vec2(teleport.landing);
        fnv.u32(teleport.started);
        fnv.u32(teleport.lands);
        fnv.some(teleport.live);
        fnv.u32(teleport.sight_until);
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
    let (tag, progress) = match phase {
        ActionPhase::Before { progress } => (0, progress),
        ActionPhase::During { progress } => (1, progress),
        ActionPhase::After { progress } => (2, progress),
    };
    fnv.u8(tag);
    fnv.u32(progress);
}

/// A queued blow and its pending modifier before resolution.
fn hash_hit(fnv: &mut Fnv, hit: &Hit) {
    fnv.opt(hit.source, Fnv::entity);
    fnv.entity(hit.target);
    fnv.i32(hit.amount);
    fnv.u8(hit.kind as u8);
    hash_damage_amp(fnv, hit.damage_amp_bp);
    fnv.some(hit.attack);
    fnv.some(hit.pierces);
    match hit.effect {
        HitEffect::None => fnv.u8(0),
        HitEffect::Requiem { level, fear } => {
            fnv.u8(2);
            fnv.u8(level);
            fnv.u32(fear.line);
            fnv.u32(fear.most);
        }
        HitEffect::Shadowraze { stack_damage } => {
            fnv.u8(1);
            fnv.i32(stack_damage);
        }
    }
}

/// One outgoing amplification; nothing for [`rules::NOMINAL_BP`].
///
/// [`rules::NOMINAL_BP`]: bota_core::rules::NOMINAL_BP
fn hash_damage_amp(fnv: &mut Fnv, bp: i32) {
    if bp != bota_core::rules::NOMINAL_BP {
        fnv.u8(1);
        fnv.i32(bp);
    }
}

/// One modifier kind and everything it carries.
fn hash_modifier_kind(fnv: &mut Fnv, kind: ModifierKind) {
    match kind {
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
        ModifierKind::Fountain { hp_pct, mana_pct } => {
            fnv.u8(3);
            fnv.i32(hp_pct);
            fnv.i32(mana_pct);
        }
        ModifierKind::Stunned => fnv.u8(4),
        ModifierKind::Shielded => fnv.u8(8),
        ModifierKind::Slowed { pct } => hash_tagged(fnv, 5, pct),
        ModifierKind::Hastened { pct } => hash_tagged(fnv, 7, pct),
        ModifierKind::Guarded {
            armor,
            hp_per_second,
        } => {
            fnv.u8(12);
            fnv.i32(armor);
            fnv.i32(hp_per_second);
        }
        ModifierKind::Inspired {
            hp_per_second,
            magic_resist_pct,
        } => {
            fnv.u8(13);
            fnv.i32(hp_per_second);
            fnv.i32(magic_resist_pct);
        }
        ModifierKind::ArmorBroken { armor } => hash_tagged(fnv, 10, armor.raw),
        ModifierKind::MagicBroken { pct } => hash_tagged(fnv, 16, pct),
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
        ModifierKind::BloodGrenade { since } => {
            fnv.u8(17);
            fnv.u32(since);
        }
    }
}

/// A tag and one number.
fn hash_tagged(fnv: &mut Fnv, tag: u8, value: i32) {
    fnv.u8(tag);
    fnv.i32(value);
}

/// One trusted spawn rule: what it takes, what it puts on, how long.
fn hash_spawn_modifier(fnv: &mut Fnv, rule: &SpawnModifier) {
    assert!(rule.select.targets.len() <= crate::game::MAX_SPAWN_TARGETS);
    fnv.opt(rule.select.team, Fnv::team);
    fnv.u32(rule.select.targets.len() as u32);
    for target in &rule.select.targets {
        match target {
            SpawnTarget::Kind(kind) => {
                fnv.u8(0);
                fnv.kind(*kind);
            }
            SpawnTarget::Category(category) => {
                fnv.u8(1);
                fnv.u8(match category {
                    SpawnCategory::Hero => 0,
                    SpawnCategory::LaneCreep => 1,
                    SpawnCategory::NeutralCreep => 2,
                    SpawnCategory::Structure => 3,
                });
            }
        }
    }
    match rule.duration {
        ModifierDuration::MatchLong => fnv.u8(0),
        ModifierDuration::Ticks(ticks) => {
            fnv.u8(1);
            fnv.u32(ticks);
        }
    }
    hash_modifier_spec(fnv, rule.spec);
}

/// An applied stat change, field by field in declaration order.
fn hash_modifier_spec(fnv: &mut Fnv, spec: bota_proto::ModifierSpec) {
    fnv.i32(spec.magic_resist);
    fnv.i32(spec.status_resist);
    fnv.i32(spec.physical_damage);
    fnv.i32(spec.magic_damage);
    fnv.i32(spec.pure_damage);
    fnv.i32(spec.cooldown_rate);
    fnv.i32(spec.mana_cost_rate);
    fnv.i32(spec.move_speed);
    fnv.i32(spec.max_hp);
    fnv.i32(spec.max_mana);
    fnv.i32(spec.gold_income);
}

/// Every slot of a bag, empty ones counted so slots keep their numbers.
fn hash_bag(fnv: &mut Fnv, bag: &Inventory) {
    fnv.some(bag.town_portal.is_some());
    for slot in bag.slots.iter().chain(bag.town_portal.iter()) {
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
    fnv.u8(stack.free_charges);
    fnv.opt(stack.mode, |fnv, mode| fnv.u8(mode as u8));
}
