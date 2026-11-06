use std::{cell::RefCell, rc::Rc};

use bota_proto::{Order, Target, Team};
use luagee::{Args, Lua, LuaError, TableId, Value, Vm};

use crate::{BindingError, state::State, vector::{self, Vectors}};

mod globals;
mod queries;
mod geometry;
pub(crate) mod abilities;

type Shared = Rc<RefCell<State>>;

fn global(lua: &mut Lua, name: &'static str, function: luagee::HostFunction) -> Result<(), BindingError> {
    let function = lua.function(name, function)?;
    lua.set_global(name, function)?;
    Ok(())
}

fn method(lua: &mut Lua, methods: TableId, name: &'static str, function: luagee::HostFunction) -> Result<(), BindingError> {
    let function = lua.function(name, function)?;
    let key = lua.string(name.as_bytes())?;
    lua.vm().raw_set(methods, key, function)?;
    Ok(())
}

fn identity(vm: &mut Vm, state: &State, args: Args) -> Result<bota_proto::EntityId, LuaError> {
    vm.charge_fuel(1 + state.units.len() as u64 / 8 + state.snapshot.as_ref().map_or(0, |snapshot| snapshot.units.len()) as u64 / 8)?;
    state.id(vm.arg(args, 0)).ok_or_else(|| vm.arg_error(args, 0, "unit handle expected"))
}

pub(crate) fn install(lua: &mut Lua, state: Shared, vectors: Vectors) -> Result<(), BindingError> {
    for (name, value) in [("TEAM_RADIANT", 2), ("TEAM_DIRE", 3), ("TEAM_NEUTRAL", 4), ("BOT_MODE_NONE", 0), ("BOT_MODE_ATTACK", 2), ("LANE_TOP", 1), ("LANE_MID", 2), ("LANE_BOT", 3)] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    let capture = state.clone();
    global(lua, "GetBot", Rc::new(move |vm, _| {
        let mut state = capture.borrow_mut();
        let id = state.own_id();
        let value = state.handle(vm, id)?;
        vm.results(&[value])
    }))?;
    let capture = state.clone();
    global(lua, "GetTeam", Rc::new(move |vm, _| vm.results(&[Value::Number(team_number(capture.borrow().team))])))?;
    global(lua, "GetScriptDirectory", Rc::new(|vm, _| { let value = vm.new_string(b"bots")?; vm.results(&[value]) }))?;
    let methods = lua.vm().new_table()?;
    lua.pin(Value::Table(methods))?;
    state.borrow_mut().unit_methods = Some(methods);
    globals::install(lua, state.clone(), vectors)?;
    queries::install(lua, methods, state.clone(), vectors)?;
    abilities::install(lua, methods, state.clone())?;
    install_unit_reads(lua, methods, state.clone(), vectors)?;
    install_bot_state(lua, methods, state.clone())?;
    install_actions(lua, methods, state, vectors)?;
    Ok(())
}

fn team_number(team: Team) -> f64 { match team { Team::Radiant => 2.0, Team::Dire => 3.0, Team::Neutral => 4.0 } }

fn install_unit_reads(lua: &mut Lua, methods: TableId, state: Shared, vectors: Vectors) -> Result<(), BindingError> {
    for name in ["IsNull", "IsAlive", "GetHealth", "GetMaxHealth", "GetLocation", "GetUnitName", "GetTeam", "CanBeSeen", "GetLevel", "IsInvulnerable", "IsMagicImmune", "IsChanneling", "IsUsingAbility", "HasScepter"] {
        let capture = state.clone();
        method(lua, methods, name, Rc::new(move |vm, args| {
            let state = capture.borrow();
            let id = identity(vm, &state, args)?;
            let unit = state.unit(id);
            let snapshot = state.snapshot.as_ref();
            let dead = snapshot.is_some_and(|snapshot| snapshot.players.iter().any(|player| player.unit == Some(id) && player.respawn_left > 0));
            let value = match name {
                "HasScepter" => Value::Boolean(false),
                "IsInvulnerable" | "IsMagicImmune" | "IsChanneling" | "IsUsingAbility" => {
                    let flag = match name { "IsInvulnerable" => bota_proto::StatusFlags::INVULNERABLE, "IsMagicImmune" => bota_proto::StatusFlags::MAGIC_IMMUNE, "IsChanneling" => bota_proto::StatusFlags::CHANNELLING, _ => bota_proto::StatusFlags::CASTING };
                    Value::Boolean(unit.is_some_and(|unit| unit.statuses.bits & flag != 0))
                }
                "IsNull" => Value::Boolean(unit.is_none()),
                "IsAlive" => Value::Boolean(!dead && unit.is_some_and(|unit| unit.hp > 0)),
                "GetHealth" => Value::Number(if dead { 0.0 } else { unit.map_or(0.0, |unit| f64::from(unit.hp)) }),
                "GetMaxHealth" => Value::Number(unit.map_or(0.0, |unit| f64::from(unit.max_hp))),
                "GetLevel" => Value::Number(unit.map_or(0.0, |unit| f64::from(unit.level))),
                "GetTeam" => Value::Number(unit.map_or(4.0, |unit| team_number(unit.team))),
                "CanBeSeen" => Value::Boolean(snapshot.is_some_and(|snapshot| snapshot.units.iter().any(|known| known.unit.id == id && known.visible))),
                "GetLocation" => match unit { Some(unit) => vectors.location(vm, unit.pos)?, None => vectors.value(vm, [0.0; 3])? },
                "GetUnitName" => {
                    let key = unit.and_then(|unit| unit.hero).and_then(bota_core::hero_def).map_or("", |hero| hero.key);
                    vm.new_string(key.as_bytes())?
                }
                _ => unreachable!("registered unit read"),
            };
            vm.results(&[value])
        }))?;
    }
    Ok(())
}

fn install_actions(lua: &mut Lua, methods: TableId, state: Shared, _vectors: Vectors) -> Result<(), BindingError> {
    for name in ["Action_MoveToLocation", "Action_AttackMove", "Action_AttackUnit", "Action_ClearActions", "Action_UseAbility", "Action_UseAbilityOnEntity", "Action_UseAbilityOnLocation"] {
        let capture = state.clone();
        method(lua, methods, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let id = identity(vm, &state, args)?;
            if Some(id) != state.own_id() { return Err(vm.error("action requires the active bot")); }
            let order = match name {
                "Action_UseAbility" | "Action_UseAbilityOnEntity" | "Action_UseAbilityOnLocation" => {
                    vm.charge_fuel(1 + state.abilities.len() as u64 / 8)?;
                    let ability = abilities::identity(&state, vm.arg(args, 1)).ok_or_else(|| vm.error("ability handle expected"))?;
                    if !abilities::valid(&state, ability) { return Err(vm.error("stale ability handle")); }
                    if ability.owner() != id { return Err(vm.error("ability belongs to another unit")); }
                    let target = match name {
                        "Action_UseAbilityOnEntity" => Target::Unit(state.id(vm.arg(args, 2)).ok_or_else(|| vm.error("unit handle expected"))?),
                        "Action_UseAbilityOnLocation" => Target::Pos(vector::position(vm, vm.arg(args, 2))?),
                        _ => Target::None,
                    };
                    ability.order(target)
                }
                "Action_MoveToLocation" => Order::Move { target: Target::Pos(vector::position(vm, vm.arg(args, 1))?) },
                "Action_AttackMove" => Order::Attack { target: Target::Pos(vector::position(vm, vm.arg(args, 1))?) },
                "Action_AttackUnit" => {
                    let target = state.id(vm.arg(args, 1)).ok_or_else(|| vm.arg_error(args, 1, "unit handle expected"))?;
                    Order::Attack { target: Target::Unit(target) }
                }
                "Action_ClearActions" => Order::Move { target: Target::None },
                _ => unreachable!("registered action"),
            };
            state.order(vm, bota_client_core::IssuedOrder::hero(order))?;
            Ok(0)
        }))?;
    }
    Ok(())
}

fn install_bot_state(lua: &mut Lua, methods: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["GetGold", "GetRespawnTime", "GetActiveMode", "GetActiveModeDesire", "SetTarget", "GetTarget", "SetNextItemPurchaseValue", "GetNextItemPurchaseValue", "IsHero"] {
        let capture = state.clone();
        method(lua, methods, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let id = identity(vm, &state, args)?;
            let active = state.bot_index();
            let index = state.unit(id).and_then(|unit| unit.owner).and_then(|slot| state.seats.iter().position(|seat| seat.slot == slot));
            let value = match name {
                "GetGold" | "GetRespawnTime" => {
                    let player = state.snapshot.as_ref().and_then(|snapshot| snapshot.players.iter().find(|player| player.unit == Some(id)));
                    Value::Number(player.map_or(0.0, |player| if name == "GetGold" { f64::from(player.gold.unwrap_or(0)) } else { globals::seconds(player.respawn_left) }))
                }
                "GetActiveMode" => Value::Number(index.map_or(0.0, |index| f64::from(state.bots[index].mode))),
                "GetActiveModeDesire" => Value::Number(index.map_or(0.0, |index| state.bots[index].desire)),
                "GetNextItemPurchaseValue" => Value::Number(index.map_or(0.0, |index| f64::from(state.bots[index].next_purchase))),
                "GetTarget" => { let target = index.and_then(|index| state.bots[index].target); state.handle(vm, target)? }
                "SetTarget" => {
                    if Some(id) != state.own_id() { return Err(vm.error("mutation requires the active bot")); }
                    let target = vm.arg(args, 1);
                    state.bots[active].target = if target == Value::Nil { None } else { Some(state.id(target).ok_or_else(|| vm.error("unit handle expected"))?) };
                    return Ok(0);
                }
                "SetNextItemPurchaseValue" => {
                    if Some(id) != state.own_id() { return Err(vm.error("mutation requires the active bot")); }
                    let value = globals::integer(vm, args, 1)?;
                    state.bots[active].next_purchase = i32::try_from(value).map_err(|_| vm.error("purchase value is outside integer range"))?;
                    return Ok(0);
                }
                "IsHero" => Value::Boolean(state.unit(id).is_some_and(|unit| unit.kind == bota_proto::UnitKind::Hero)),
                _ => unreachable!("registered bot state"),
            };
            vm.results(&[value])
        }))?;
    }
    Ok(())
}
