use std::rc::Rc;
use bota_client_core::IssuedOrder;
use bota_proto::{AbilityId, AbilitySlot, EffectId, Order, StatusFlags, TalentSlot, Target, UnitKind, UnitView, Vec2};
use luagee::{Lua, Value};
use crate::{BindingError, api::Shared, state::{Snapshot, State}};
use super::{context, distance, enemy, mode_is};

#[derive(Clone, Copy)]
pub(crate) enum AuxiliaryHook { Ability, Level, Item, Purchase, Courier }

pub(crate) fn install(lua: &mut Lua, shared: Shared) -> Result<Value, BindingError> {
    Ok(lua.function("default_auxiliary", Rc::new(move |vm, args| {
        let phase = vm.check_number(args, 0)?;
        if phase.fract() != 0.0 || !(0.0..=4.0).contains(&phase) { return Err(vm.arg_error(args, 0, "known auxiliary hook expected")); }
        if phase == 3.0 { return super::economy::purchase(vm, &shared); }
        if phase == 4.0 { return super::courier::run(vm, &shared); }
        let mut state = shared.borrow_mut();
        let Some((snapshot, own)) = context(&state) else { return Ok(0); };
        let book = state.ability_book(own.id).unwrap_or(&[]);
        let work = if phase == 1.0 {
            snapshot.units.len() + book.len() + snapshot.talents.len() * 8
        } else if phase == 2.0 { snapshot.units.len() * (bota_proto::limits::INVENTORY_SLOTS + 1) + bota_proto::limits::INVENTORY_SLOTS * (own.effects.len() + 1) }
        else { snapshot.units.len() * (book.len() + 3) + book.len() };
        vm.charge_fuel(8 + work as u64 + state.orders.len() as u64)?;
        let order = match phase as u8 {
            0 => casting(&state, snapshot, own),
            1 => learning(&state, snapshot, own),
            2 => super::items::use_item(&state, snapshot, own),
            _ => None,
        };
        if let Some(order) = order { state.order(vm, IssuedOrder::hero(order))?; }
        Ok(0)
    }))?)
}

fn learning(state: &State, snapshot: &Snapshot, own: &UnitView) -> Option<Order> {
    if state.orders.iter().any(|order| order.unit.is_none() && matches!(order.order, Order::Learn { .. } | Order::LearnTalent { .. } | Order::LearnAttributeBonus)) { return None; }
    if let Some(talents) = snapshot.talents.iter().find(|talents| talents.slot == state.current_slot)
        && let Some(allowed) = talents.may_learn {
        for slot in 0..8 { if allowed.has(TalentSlot(slot)) { return Some(Order::LearnTalent { talent: TalentSlot(slot) }); } }
    }
    state.ability_book(own.id)?.iter().enumerate()
        .filter(|(_, ability)| ability.learnable == Some(true))
        .filter_map(|(slot, ability)| Some((u8::try_from(slot).ok()?, bota_core::ability_def(ability.id)?)))
        .min_by_key(|(slot, definition)| (!definition.ultimate, definition.passive, *slot))
        .map(|(slot, _)| Order::Learn { slot: AbilitySlot(slot) })
}

fn casting(state: &State, snapshot: &Snapshot, own: &UnitView) -> Option<Order> {
    if mode_is(state, "retreat") || !own.queue.is_empty()
        || own.statuses.bits & (StatusFlags::STUNNED | StatusFlags::FEARED | StatusFlags::CASTING | StatusFlags::CHANNELLING) != 0
        || state.orders.iter().any(|order| order.unit.is_none() && matches!(order.order, Order::Cast { .. } | Order::Use { .. })) { return None; }
    let hero = enemy(snapshot, own, UnitKind::Hero).filter(|target| target.statuses.bits & StatusFlags::MAGIC_IMMUNE == 0);
    let creep = snapshot.units.iter().filter(|known| known.visible && known.unit.team != own.team && known.unit.hp > 0
        && bota_core::kind_def(known.unit.kind).creep && known.unit.statuses.bits & (StatusFlags::INVULNERABLE | StatusFlags::MAGIC_IMMUNE) == 0)
        .map(|known| &known.unit).min_by_key(|target| (distance(own.pos, target.pos), target.id));
    for (slot, ability) in state.ability_book(own.id)?.iter().enumerate() {
        if ability.level == 0 || ability.passive || ability.cooldown_left != 0 || ability.mana_cost.max(0) > own.mana { continue; }
        let slot = AbilitySlot(u8::try_from(slot).ok()?);
        if let Some(reach) = bota_core::heroes::shadow_fiend::SHADOW_FIEND_RAZES.iter().position(|known| *known == ability.id) {
            let target = hero.filter(|target| radial_fit(own, target, reach)).or_else(|| {
                creep.filter(|target| i64::from(own.mana) >= i64::from(ability.mana_cost.max(0)) * 2
                    && radial_fit(own, target, reach) && creep_cluster(snapshot, own, target) >= 2)
            });
            if let Some(target) = target {
                return Some(if facing_hits(own, target, reach) { Order::Cast { slot, target: Target::None } }
                    else { Order::Move { target: Target::Pos(target.pos) } });
            }
        } else if ability.id == AbilityId::REQUIEM
            && own.effects.iter().any(|effect| effect.id == EffectId::SOULS && effect.stacks.is_some_and(|souls| souls > 0))
            && hero.is_some_and(|target| distance(own.pos, target.pos) <= i128::from(own.attack_range.raw.max(0)).pow(2)) {
            return Some(Order::Cast { slot, target: Target::None });
        }
    }
    None
}

fn radial_fit(own: &UnitView, target: &UnitView, reach: usize) -> bool {
    let range = i128::from(bota_core::rules::RAZE_DISTANCE[reach]) * 65536;
    let radius = i128::from(bota_core::rules::RAZE_RADIUS) * 65536 + i128::from(target.bound.raw.max(0));
    let lower = (range - radius).max(0);
    let upper = range + radius;
    (lower * lower..=upper * upper).contains(&distance(own.pos, target.pos))
}

fn facing_hits(own: &UnitView, target: &UnitView, reach: usize) -> bool {
    // Project from zero first so received coordinates cannot overflow a Fixed addition.
    let offset = bota_core::raze_landing(Vec2::from_ints(0,0), own.facing, reach);
    let x = i128::from(own.pos.x.raw) + i128::from(offset.x.raw) - i128::from(target.pos.x.raw);
    let y = i128::from(own.pos.y.raw) + i128::from(offset.y.raw) - i128::from(target.pos.y.raw);
    let radius = i128::from(bota_core::rules::RAZE_RADIUS) * 65536 + i128::from(target.bound.raw.max(0));
    x * x + y * y <= radius * radius
}

fn creep_cluster(snapshot: &Snapshot, own: &UnitView, target: &UnitView) -> usize {
    let radius = i128::from(bota_core::rules::RAZE_RADIUS) * 65536;
    snapshot.units.iter().filter(|known| known.visible && known.unit.team != own.team && known.unit.hp > 0
        && bota_core::kind_def(known.unit.kind).creep && known.unit.statuses.bits & (StatusFlags::INVULNERABLE | StatusFlags::MAGIC_IMMUNE) == 0
        && distance(target.pos, known.unit.pos) <= radius * radius).count()
}
