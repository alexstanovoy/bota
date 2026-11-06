use bota_proto::{AbilityId, AbilitySlot, EntityId, ItemId, ItemSlot, StatusFlags};

use super::*;
use super::globals::{integer, string};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Identity {
    Ability { unit: EntityId, slot: AbilitySlot, id: AbilityId },
    Innate { unit: EntityId, id: AbilityId },
    Item { unit: EntityId, slot: ItemSlot, id: ItemId },
}

impl Identity {
    pub(super) fn order(self, target: Target) -> Option<Order> {
        match self { Self::Ability { slot, .. } => Some(Order::Cast { slot, target }), Self::Item { slot, .. } => Some(Order::Use { slot, target }), Self::Innate { .. } => None }
    }

    pub(super) fn owner(self) -> EntityId { match self { Self::Ability { unit, .. } | Self::Innate { unit, .. } | Self::Item { unit, .. } => unit } }
}

struct View { key: &'static str, level: u8, range: i32, castable: bool, learnable: bool }

fn view(state: &State, identity: Identity) -> Option<View> {
    let unit = state.unit(identity.owner())?;
    let able = unit.hp > 0 && unit.statuses.bits & (StatusFlags::STUNNED | StatusFlags::FEARED) == 0;
    match identity {
        Identity::Innate { id, .. } => {
            let innate = unit.hero.and_then(bota_core::hero_def)?.innate?;
            if innate != id { return None; }
            let definition = bota_core::ability_def(id)?;
            Some(View { key: definition.key, level: 1, range: definition.range, castable: false, learnable: false })
        }
        Identity::Ability { slot, id, .. } => {
            let ability = unit.abilities.get(usize::from(slot.0)).filter(|ability| ability.id == id)?;
            Some(View { key: bota_core::ability_def(id)?.key, level: ability.level, range: ability.range,
                castable: able && ability.level > 0 && !ability.passive && ability.cooldown_left == 0 && unit.mana >= ability.mana_cost,
                learnable: ability.learnable == Some(true) })
        }
        Identity::Item { slot, id, .. } => {
            let item = unit.items.get(usize::from(slot.0))?.as_ref().filter(|item| item.id == id)?;
            Some(View { key: bota_core::item_def(id)?.key, level: 1, range: item.range,
                castable: able && item.aim.is_some() && item.cooldown_left == 0 && item.mute_left == 0 && unit.mana >= item.mana_cost,
                learnable: false })
        }
    }
}

pub(super) fn valid(state: &State, identity: Identity) -> bool { view(state, identity).is_some() }

pub(super) fn identity(state: &State, value: Value) -> Option<Identity> {
    state.abilities.iter().find(|(_, table)| value == Value::Table(*table)).map(|(identity, _)| *identity)
}

fn handle(vm: &mut Vm, state: &mut State, identity: Option<Identity>) -> Result<Value, LuaError> {
    let Some(identity) = identity else { return Ok(Value::Nil) };
    vm.charge_fuel(1 + state.abilities.len() as u64 / 8)?;
    if let Some((_, table)) = state.abilities.iter().find(|(known, _)| *known == identity) { return Ok(Value::Table(*table)); }
    if state.abilities.len() >= 8192 { return Err(vm.error("ability handle limit exceeded")); }
    let table = vm.new_table()?;
    let meta = vm.new_table()?;
    let key = vm.new_string(b"__index")?;
    vm.raw_set(meta, key, state.ability_methods.map_or(Value::Nil, Value::Table))?;
    let key = vm.new_string(b"__metatable")?;
    vm.raw_set(meta, key, Value::Boolean(false))?;
    vm.set_metatable(table, Some(meta))?;
    vm.pin(Value::Table(table))?;
    state.abilities.push((identity, table));
    Ok(Value::Table(table))
}

pub(super) fn install(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    let methods = lua.vm().new_table()?;
    lua.pin(Value::Table(methods))?;
    state.borrow_mut().ability_methods = Some(methods);
    for name in ["IsNull", "GetName", "GetLevel", "GetCastRange", "IsFullyCastable", "CanAbilityBeUpgraded", "GetSpecialValueInt", "GetAbilityDamage"] {
        let capture = state.clone();
        method(lua, methods, name, Rc::new(move |vm, args| {
            let state = capture.borrow();
            vm.charge_fuel(1 + state.abilities.len() as u64 / 8 + state.snapshot.as_ref().map_or(0, |snapshot| snapshot.units.len()) as u64 / 8)?;
            let identity = identity(&state, vm.arg(args, 0)).ok_or_else(|| vm.error("ability handle expected"))?;
            let view = view(&state, identity);
            let value = match name {
                "GetSpecialValueInt" | "GetAbilityDamage" => {
                    let key = if name == "GetSpecialValueInt" { string(vm, args, 1)? } else { b"damage" };
                    Value::Number(f64::from(special(identity, view.as_ref().map_or(0, |view| view.level), key)))
                }
                "IsNull" => Value::Boolean(view.is_none()),
                "GetName" => vm.new_string(view.as_ref().map_or("", |view| view.key).as_bytes())?,
                "GetLevel" => Value::Number(view.as_ref().map_or(0.0, |view| f64::from(view.level))),
                "GetCastRange" => Value::Number(view.as_ref().map_or(0.0, |view| f64::from(view.range))),
                "IsFullyCastable" => Value::Boolean(view.is_some_and(|view| view.castable)),
                "CanAbilityBeUpgraded" => Value::Boolean(view.is_some_and(|view| view.learnable)),
                _ => unreachable!("registered ability read"),
            };
            vm.results(&[value])
        }))?;
    }
    install_lookup(lua, units, state.clone())?;
    install_immediate(lua, units, state)
}

fn install_lookup(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["GetAbilityByName", "GetItemInSlot"] {
        let capture = state.clone();
        method(lua, units, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let id = super::identity(vm, &state, args)?;
            let identity = if name == "GetAbilityByName" {
                let key = string(vm, args, 1)?;
                lookup_ability(&state, id, key)
            } else {
                let index = integer(vm, args, 1)?;
                usize::try_from(index).ok().and_then(|index| state.unit(id)?.items.get(index)?.as_ref().map(|item| Identity::Item { unit: id, slot: ItemSlot(index as u8), id: item.id }))
            };
            let value = handle(vm, &mut state, identity)?;
            vm.results(&[value])
        }))?;
    }
    Ok(())
}

fn lookup_ability(state: &State, id: EntityId, key: &[u8]) -> Option<Identity> {
    let unit = state.unit(id)?;
    let matches = |id| bota_core::ability_def(id).is_some_and(|definition| definition.key.as_bytes() == key);
    if let Some((slot, ability)) = unit.abilities.iter().enumerate().find(|(_, ability)| matches(ability.id)) {
        return Some(Identity::Ability { unit: id, slot: AbilitySlot(u8::try_from(slot).ok()?), id: ability.id });
    }
    let innate = unit.hero.and_then(bota_core::hero_def)?.innate?;
    matches(innate).then_some(Identity::Innate { unit: id, id: innate })
}

fn install_immediate(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["ActionImmediate_LevelAbility", "ActionImmediate_PurchaseItem", "Action_LevelAbility", "Action_PurchaseItem"] {
        let capture = state.clone();
        method(lua, units, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let id = super::identity(vm, &state, args)?;
            if Some(id) != state.own_id() { return Err(vm.error("action requires the active bot")); }
            let key = string(vm, args, 1)?;
            if matches!(name, "ActionImmediate_LevelAbility" | "Action_LevelAbility") {
                let slot = state.unit(id).and_then(|unit| unit.abilities.iter().position(|ability| bota_core::ability_def(ability.id).is_some_and(|def| def.key.as_bytes() == key)));
                if let Some(slot) = slot { state.order(vm, bota_client_core::IssuedOrder::hero(Order::Learn { slot: AbilitySlot(slot as u8) }))?; }
                Ok(0)
            } else {
                let item = std::str::from_utf8(key).ok().and_then(bota_core::item_by_key);
                let result = if let Some(item) = item {
                    state.order(vm, bota_client_core::IssuedOrder::hero(Order::Buy { item }))?;
                    0.0
                } else { 6.0 };
                vm.results(&[Value::Number(result)])
            }
        }))?;
    }
    Ok(())
}

fn special(identity: Identity, level: u8, key: &[u8]) -> i32 {
    use bota_core::{heroes::shadow_fiend::SHADOW_FIEND_RAZES, rules};
    match identity {
        Identity::Item { id, .. } => std::str::from_utf8(key).ok().and_then(|key| bota_core::item_def(id)?.value(key)).and_then(|values| values.first()).map_or(0, |value| value.0 / 1000),
        Identity::Ability { id, .. } | Identity::Innate { id, .. } => {
            if let Some(index) = SHADOW_FIEND_RAZES.iter().position(|known| *known == id) {
                return match key {
                    b"shadowraze_radius" => rules::RAZE_RADIUS,
                    b"shadowraze_range" => rules::RAZE_DISTANCE[index],
                    b"shadowraze_damage" | b"damage" => rules::RAZE_DAMAGE[usize::from(level.saturating_sub(1)).min(rules::RAZE_DAMAGE.len() - 1)],
                    _ => 0,
                };
            }
            match bota_core::ability_def(id).map(|def| def.key) {
                Some("nevermore_requiem") if matches!(key, b"damage" | b"requiem_damage") => rules::REQUIEM_LINE_DAMAGE[usize::from(level.saturating_sub(1)).min(rules::REQUIEM_LINE_DAMAGE.len() - 1)],
                Some("pudge_meat_hook") if matches!(key, b"damage" | b"hook_damage") => rules::HOOK_DAMAGE[usize::from(level.saturating_sub(1)).min(rules::HOOK_DAMAGE.len() - 1)],
                _ => 0,
            }
        }
    }
}
