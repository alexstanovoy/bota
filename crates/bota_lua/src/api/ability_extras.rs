//! Name conventions over shared visible cast and catalogue channel queries.
use super::{
    abilities::{self, Identity},
    context::ApiContext,
    *,
};
use bota_proto::{Attribute, ItemId};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    let methods = context
        .state
        .borrow()
        .ability_methods
        .expect("installed ability methods");
    for name in ["GetChannelTime", "IsInAbilityPhase", "GetPowerTreadsStat"] {
        let context = context.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| read(&context, vm, args, name)),
        )?;
    }
    Ok(())
}
fn read(context: &ApiContext, vm: &mut Vm, args: Args, name: &str) -> Result<usize, LuaError> {
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        16 + bota_core::ABILITIES.len() as u64
            + state.abilities.len() as u64
            + snapshot.units.len() as u64
            + snapshot.players.len() as u64,
    )?;
    let identity = abilities::identity(&state, vm.arg(args, 0))
        .ok_or_else(|| vm.arg_error(args, 0, "ability or item handle expected"))?;
    let value = match name {
        "GetChannelTime" => channel(&state, identity),
        "IsInAbilityPhase" => phase(&state, snapshot, identity),
        "GetPowerTreadsStat" => treads(&state, snapshot, identity),
        _ => unreachable!(),
    };
    vm.results(&[value])
}
#[expect(
    clippy::float_arithmetic,
    reason = "imported channel milliseconds are converted directly to API seconds"
)]
fn channel(state: &State, identity: Identity) -> Value {
    let time = match identity {
        Identity::Ability { unit, slot, id } => bota_core::ability_channel_time(
            id,
            state
                .ability_book(unit)
                .and_then(|book| book.get(usize::from(slot.0)))
                .map_or(0, |ability| ability.level),
        ),
        Identity::Innate { id, .. } => bota_core::ability_channel_time(id, 1),
        Identity::Item { id, .. } => bota_core::item_channel_time(id),
        Identity::Talent { .. } => Some(bota_core::Ms(0)),
    };
    time.map_or(Value::Nil, |time| Value::Number(f64::from(time.0) / 1000.0))
}
fn phase(state: &State, snapshot: &crate::state::Snapshot, identity: Identity) -> Value {
    if state
        .player(identity.owner())
        .is_some_and(|player| player.respawn_left > 0)
    {
        return Value::Boolean(false);
    }
    let known = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == identity.owner() && known.visible);
    let phase = known.and_then(|known| match identity {
        Identity::Ability { id, .. } | Identity::Innate { id, .. } => {
            Some(bota_client_core::spell_in_cast_point(&known.unit, id))
        }
        Identity::Item { .. } => bota_client_core::item_in_cast_point(&known.unit),
        Identity::Talent { .. } => Some(false),
    });
    phase.map_or(Value::Nil, Value::Boolean)
}
fn treads(state: &State, snapshot: &crate::state::Snapshot, identity: Identity) -> Value {
    let Identity::Item { unit, slot, id } = identity else {
        return Value::Number(-1.0);
    };
    if id != ItemId::POWER_TREADS {
        return Value::Number(-1.0);
    }
    let observed = state
        .player(unit)
        .is_some_and(|player| player.team == state.team)
        || snapshot
            .units
            .iter()
            .any(|known| known.unit.id == unit && known.visible);
    if !observed {
        return Value::Nil;
    }
    state
        .item(unit, slot)
        .filter(|item| item.id == id)
        .and_then(|item| item.mode)
        .map_or(Value::Nil, |mode| {
            Value::Number(f64::from(match mode {
                Attribute::Strength => 0,
                Attribute::Agility => 1,
                Attribute::Intelligence => 2,
            }))
        })
}
