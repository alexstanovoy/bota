//! Script chat uses the same checked transport lines as every other client.
use super::{
    context::{ApiContext, register_unit},
    globals::string,
};
use crate::BindingError;
use bota_proto::{Audience, ClientMsg};
use luagee::{Args, Lua, LuaError, Value, Vm};
const CHAT_INTERVAL_TICKS: u32 = 30;
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "ActionImmediate_Chat", chat)?;
    register_unit(lua, context, "Action_Chat", chat)
}
fn chat(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    vm.charge_fuel(super::CALL_FUEL)?;
    let raw = string(vm, args, 1)?;
    if raw.len() > bota_proto::MAX_CHAT_BYTES {
        return Err(vm.error("chat text exceeds the byte limit"));
    }
    let length = raw.len() as u64;
    let text = match std::str::from_utf8(raw) {
        Ok(text) => text.to_owned(),
        Err(_) => return Err(vm.error("chat text must be UTF-8")),
    };
    vm.charge_fuel(length * 2)?;
    if text.chars().any(char::is_control) {
        return Err(vm.error("chat text contains control characters"));
    }
    let all = match vm.arg(args, 2) {
        Value::Boolean(all) => all,
        _ => return Err(vm.arg_error(args, 2, "boolean expected")),
    };
    let message = bota_client_core::say(if all { Audience::All } else { Audience::Team }, &text)
        .map_err(|_| vm.error("chat text does not meet transport limits"))?;
    let ClientMsg::Say(line) = message else {
        unreachable!("say produces a line")
    };
    let mut state = context.state.borrow_mut();
    if state.own_id() != Some(id) {
        return Err(vm.error("chat requires the active bot"));
    }
    let tick = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?
        .tick;
    let index = state.bot_index();
    if state.bots[index]
        .last_chat
        .is_some_and(|last| tick.saturating_sub(last) < CHAT_INTERVAL_TICKS)
    {
        return Ok(0);
    }
    if !state.chat.is_empty() {
        return Ok(0);
    }
    state.chat.push(line);
    state.bots[index].last_chat = Some(tick);
    Ok(0)
}
