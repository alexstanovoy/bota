use std::{cell::RefCell, rc::Rc};

use bota_proto::{Order, Target, Team};
use luagee::{Args, Lua, LuaError, TableId, Value, Vm};

use crate::{
    BindingError,
    state::State,
    vector::{self, Vectors},
};

#[cfg(test)]
mod fuel_tests;

pub(crate) mod abilities;
pub(crate) mod context;
mod estimates;
mod geometry;
mod globals;
mod lane_desires;
mod match_metadata;
mod math_queries;
mod modifiers;
mod patterns;
pub(crate) mod players;
mod queries;
pub(crate) mod shop;
mod structures;
mod unit_data;
mod unit_lists;
mod unit_names;
mod unit_statistics;

pub(crate) type Shared = Rc<RefCell<State>>;

// Fixed-work queries charge explicitly; families with scan/read charges keep their own cost model.
const CALL_FUEL: u64 = 8;

fn global(
    lua: &mut Lua,
    name: &'static str,
    function: luagee::HostFunction,
) -> Result<(), BindingError> {
    let function = lua.function(name, function)?;
    lua.set_global(name, function)?;
    Ok(())
}

fn method(
    lua: &mut Lua,
    methods: TableId,
    name: &'static str,
    function: luagee::HostFunction,
) -> Result<(), BindingError> {
    let function = lua.function(name, function)?;
    let key = lua.string(name.as_bytes())?;
    lua.vm().raw_set(methods, key, function)?;
    Ok(())
}

fn identity(vm: &mut Vm, state: &State, args: Args) -> Result<bota_proto::EntityId, LuaError> {
    vm.charge_fuel(
        1 + state.units.len() as u64 / 8
            + state
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.units.len()) as u64
                / 8,
    )?;
    state
        .id(vm.arg(args, 0))
        .ok_or_else(|| vm.arg_error(args, 0, "unit handle expected"))
}

pub(crate) fn install(lua: &mut Lua, state: Shared, vectors: Vectors) -> Result<(), BindingError> {
    for (name, value) in [
        ("TEAM_RADIANT", 2),
        ("TEAM_DIRE", 3),
        ("TEAM_NEUTRAL", 4),
        ("BOT_MODE_NONE", 0),
        ("BOT_MODE_ATTACK", 2),
        ("LANE_TOP", 1),
        ("LANE_MID", 2),
        ("LANE_BOT", 3),
    ] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    for (name, value) in [
        ("DAMAGE_TYPE_PHYSICAL", 1),
        ("DAMAGE_TYPE_MAGICAL", 2),
        ("DAMAGE_TYPE_PURE", 4),
        ("DAMAGE_TYPE_ALL", 7),
        ("TOWER_TOP_1", 0),
        ("TOWER_TOP_2", 1),
        ("TOWER_TOP_3", 2),
        ("TOWER_MID_1", 3),
        ("TOWER_MID_2", 4),
        ("TOWER_MID_3", 5),
        ("TOWER_BOT_1", 6),
        ("TOWER_BOT_2", 7),
        ("TOWER_BOT_3", 8),
        ("TOWER_BASE_1", 9),
        ("TOWER_BASE_2", 10),
    ] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    for (name, value) in [
        ("BOT_MODE_DESIRE_NONE", 0.0),
        ("BOT_MODE_DESIRE_VERYLOW", 0.1),
        ("BOT_MODE_DESIRE_LOW", 0.25),
        ("BOT_MODE_DESIRE_MODERATE", 0.5),
        ("BOT_MODE_DESIRE_HIGH", 0.75),
        ("BOT_MODE_DESIRE_VERYHIGH", 0.9),
        ("BOT_MODE_DESIRE_ABSOLUTE", 1.0),
    ] {
        lua.set_global(name, Value::Number(value))?;
    }
    let capture = state.clone();
    global(
        lua,
        "GetTeam",
        Rc::new(move |vm, _| {
            vm.charge_fuel(CALL_FUEL)?;
            vm.results(&[Value::Number(team_number(capture.borrow().team))])
        }),
    )?;
    global(
        lua,
        "GetScriptDirectory",
        Rc::new(|vm, _| {
            let value = vm.new_string(b"bots")?;
            vm.results(&[value])
        }),
    )?;
    let methods = lua.vm().new_table()?;
    lua.pin(Value::Table(methods))?;
    state.borrow_mut().unit_methods = Some(methods);
    let context = context::ApiContext {
        state: state.clone(),
        vectors,
    };
    patterns::install(lua, &context)?;
    estimates::install(lua, &context)?;
    globals::install(lua, state.clone(), vectors)?;
    lane_desires::install(lua, &context)?;
    match_metadata::install(lua, &context)?;
    math_queries::install(lua, &context)?;
    structures::install(lua, &context)?;
    players::install(lua, &context)?;
    unit_lists::install(lua, &context)?;
    unit_statistics::install(lua, &context)?;
    modifiers::install(lua, &context)?;
    queries::install(lua, methods, state.clone(), vectors)?;
    abilities::install(lua, methods, state.clone())?;
    shop::install(lua, &context)?;
    install_unit_reads(lua, methods, state.clone())?;
    install_bot_state(lua, methods, state.clone())?;
    install_actions(lua, methods, state, vectors)?;
    Ok(())
}

fn team_number(team: Team) -> f64 {
    match team {
        Team::Radiant => 2.0,
        Team::Dire => 3.0,
        Team::Neutral => 4.0,
    }
}

fn install_unit_reads(lua: &mut Lua, methods: TableId, state: Shared) -> Result<(), BindingError> {
    for name in [
        "IsNull",
        "IsAlive",
        "GetMaxHealth",
        "GetUnitName",
        "GetTeam",
        "CanBeSeen",
        "GetLevel",
        "IsInvulnerable",
        "IsMagicImmune",
        "IsChanneling",
        "IsUsingAbility",
        "HasScepter",
    ] {
        let capture = state.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| {
                let state = capture.borrow();
                let id = identity(vm, &state, args)?;
                let unit = state.unit(id);
                let snapshot = state.snapshot.as_ref();
                let dead = snapshot.is_some_and(|snapshot| {
                    snapshot
                        .players
                        .iter()
                        .any(|player| player.unit == Some(id) && player.respawn_left > 0)
                });
                let value = match name {
                    "HasScepter" => Value::Boolean(false),
                    "IsInvulnerable" | "IsMagicImmune" | "IsChanneling" | "IsUsingAbility" => {
                        let flag = match name {
                            "IsInvulnerable" => bota_proto::StatusFlags::INVULNERABLE,
                            "IsMagicImmune" => bota_proto::StatusFlags::MAGIC_IMMUNE,
                            "IsChanneling" => bota_proto::StatusFlags::CHANNELLING,
                            _ => bota_proto::StatusFlags::CASTING,
                        };
                        Value::Boolean(unit.is_some_and(|unit| unit.statuses.bits & flag != 0))
                    }
                    "IsNull" => Value::Boolean(
                        unit.is_none()
                            && !snapshot.is_some_and(|snapshot| {
                                snapshot
                                    .players
                                    .iter()
                                    .any(|player| player.unit == Some(id))
                            }),
                    ),
                    "IsAlive" => Value::Boolean(!dead && unit.is_some_and(|unit| unit.hp > 0)),
                    "GetMaxHealth" => {
                        Value::Number(unit.map_or(0.0, |unit| f64::from(unit.max_hp)))
                    }
                    "GetLevel" => Value::Number(state.player(id).map_or_else(
                        || unit.map_or(0.0, |unit| f64::from(unit.level)),
                        |player| f64::from(player.level),
                    )),
                    "GetTeam" => Value::Number(state.player(id).map_or_else(
                        || unit.map_or(4.0, |unit| team_number(unit.team)),
                        |player| team_number(player.team),
                    )),
                    "CanBeSeen" => Value::Boolean(snapshot.is_some_and(|snapshot| {
                        snapshot
                            .units
                            .iter()
                            .any(|known| known.unit.id == id && known.visible)
                    })),
                    "GetUnitName" => {
                        vm.charge_fuel(32)?;
                        let key = unit_names::key(&state, id).unwrap_or_default();
                        vm.new_string(key.as_bytes())?
                    }
                    _ => unreachable!("registered unit read"),
                };
                vm.results(&[value])
            }),
        )?;
    }
    Ok(())
}

fn install_actions(
    lua: &mut Lua,
    methods: TableId,
    state: Shared,
    _vectors: Vectors,
) -> Result<(), BindingError> {
    for name in [
        "Action_AttackMove",
        "Action_AttackUnit",
        "Action_ClearActions",
        "Action_UseAbility",
        "Action_UseAbilityOnEntity",
        "Action_UseAbilityOnLocation",
    ] {
        let capture = state.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| {
                let mut state = capture.borrow_mut();
                let id = identity(vm, &state, args)?;
                if Some(id) != state.own_id() {
                    return Err(vm.error("action requires the active bot"));
                }
                let once = if name == "Action_AttackUnit" {
                    match vm.arg(args, 2) {
                        Value::Nil => false,
                        Value::Boolean(once) => once,
                        _ => return Err(vm.arg_error(args, 2, "boolean expected")),
                    }
                } else {
                    false
                };
                let order = match name {
                    "Action_UseAbility"
                    | "Action_UseAbilityOnEntity"
                    | "Action_UseAbilityOnLocation" => {
                        vm.charge_fuel(1 + state.abilities.len() as u64 / 8)?;
                        let ability = abilities::identity(&state, vm.arg(args, 1))
                            .ok_or_else(|| vm.error("ability handle expected"))?;
                        if !abilities::valid(&state, ability) {
                            return Err(vm.error("stale ability handle"));
                        }
                        if ability.owner() != id {
                            return Err(vm.error("ability belongs to another unit"));
                        }
                        let target = match name {
                            "Action_UseAbilityOnEntity" => Target::Unit(
                                state
                                    .id(vm.arg(args, 2))
                                    .ok_or_else(|| vm.error("unit handle expected"))?,
                            ),
                            "Action_UseAbilityOnLocation" => {
                                Target::Pos(vector::position(vm, vm.arg(args, 2))?)
                            }
                            _ => Target::None,
                        };
                        ability
                            .order(target)
                            .ok_or_else(|| vm.error("ability has no cast slot"))?
                    }
                    "Action_AttackMove" => Order::Attack {
                        target: Target::Pos(vector::position(vm, vm.arg(args, 1))?),
                    },
                    "Action_AttackUnit" => {
                        let target = state
                            .id(vm.arg(args, 1))
                            .ok_or_else(|| vm.arg_error(args, 1, "unit handle expected"))?;
                        Order::Attack {
                            target: Target::Unit(target),
                        }
                    }
                    "Action_ClearActions" => Order::Move {
                        target: Target::None,
                    },
                    _ => unreachable!("registered action"),
                };
                let index = state.bot_index();
                let pending = match (once, order) {
                    (
                        true,
                        Order::Attack {
                            target: Target::Unit(target),
                        },
                    ) => Some(pending_once(vm, &state, id, target)?),
                    _ => None,
                };
                state.order(vm, bota_client_core::IssuedOrder::hero(order))?;
                if let Some(pending) = pending {
                    state.bots[index].pending_once = Some(pending);
                }
                Ok(0)
            }),
        )?;
    }
    Ok(())
}

fn pending_once(
    vm: &mut Vm,
    state: &State,
    source: bota_proto::EntityId,
    target: bota_proto::EntityId,
) -> Result<crate::state::PendingOnce, LuaError> {
    if let Some(pending) = state.bots[state.bot_index()]
        .pending_once
        .filter(|pending| pending.source == source && pending.target == target && !pending.launched)
    {
        return Ok(crate::state::PendingOnce {
            expires_at: None,
            ..pending
        });
    }
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    Ok(crate::state::PendingOnce {
        source,
        target,
        tick: snapshot.tick,
        launched: false,
        expires_at: None,
    })
}

fn install_bot_state(lua: &mut Lua, methods: TableId, state: Shared) -> Result<(), BindingError> {
    for name in [
        "GetGold",
        "GetRespawnTime",
        "GetActiveMode",
        "GetActiveModeDesire",
        "SetTarget",
        "GetTarget",
        "SetNextItemPurchaseValue",
        "GetNextItemPurchaseValue",
        "IsHero",
    ] {
        let capture = state.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| {
                let mut state = capture.borrow_mut();
                let id = identity(vm, &state, args)?;
                let active = state.bot_index();
                let owner = state
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .players
                            .iter()
                            .find(|player| player.unit == Some(id))
                            .map(|player| player.slot)
                    })
                    .or_else(|| state.unit(id).and_then(|unit| unit.owner));
                let index =
                    owner.and_then(|slot| state.seats.iter().position(|seat| seat.slot == slot));
                let value = match name {
                    "GetGold" | "GetRespawnTime" => {
                        let player = state.snapshot.as_ref().and_then(|snapshot| {
                            snapshot
                                .players
                                .iter()
                                .find(|player| player.unit == Some(id))
                        });
                        Value::Number(player.map_or(0.0, |player| {
                            if name == "GetGold" {
                                f64::from(
                                    state
                                        .purchases
                                        .as_ref()
                                        .and_then(|projection| projection.gold(player.slot))
                                        .or(player.gold)
                                        .unwrap_or(0),
                                )
                            } else {
                                globals::seconds(player.respawn_left)
                            }
                        }))
                    }
                    "GetActiveMode" => {
                        Value::Number(index.map_or(0.0, |index| f64::from(state.bots[index].mode)))
                    }
                    "GetActiveModeDesire" => {
                        Value::Number(index.map_or(0.0, |index| state.bots[index].desire))
                    }
                    "GetNextItemPurchaseValue" => Value::Number(
                        index.map_or(0.0, |index| f64::from(state.bots[index].next_purchase)),
                    ),
                    "GetTarget" => {
                        let target = index.and_then(|index| state.bots[index].target);
                        state.handle(vm, target)?
                    }
                    "SetTarget" => {
                        if Some(id) != state.own_id() {
                            return Err(vm.error("mutation requires the active bot"));
                        }
                        let target = vm.arg(args, 1);
                        state.bots[active].target = if target == Value::Nil {
                            None
                        } else {
                            Some(
                                state
                                    .id(target)
                                    .ok_or_else(|| vm.error("unit handle expected"))?,
                            )
                        };
                        return Ok(0);
                    }
                    "SetNextItemPurchaseValue" => {
                        if Some(id) != state.own_id() {
                            return Err(vm.error("mutation requires the active bot"));
                        }
                        let value = globals::integer(vm, args, 1)?;
                        state.bots[active].next_purchase = i32::try_from(value)
                            .map_err(|_| vm.error("purchase value is outside integer range"))?;
                        return Ok(0);
                    }
                    "IsHero" => Value::Boolean(
                        state.player(id).is_some()
                            || state
                                .unit(id)
                                .is_some_and(|unit| unit.kind == bota_proto::UnitKind::Hero),
                    ),
                    _ => unreachable!("registered bot state"),
                };
                vm.results(&[value])
            }),
        )?;
    }
    Ok(())
}
