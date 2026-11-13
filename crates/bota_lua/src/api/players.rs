//! Player identities are team-relative engine ids, not protocol seat ids.

use super::{
    context::{ApiContext, register_global, register_unit},
    globals::{bounded_integer, seconds},
    *,
};
use crate::state::{Player, Snapshot};
use bota_proto::{HeroId, SlotId};

pub(crate) fn bot_declarations(slots: &[SlotId]) -> Result<[bool; 10], BindingError> {
    if slots.len() > 10 {
        return Err(BindingError::Configuration(
            "at most ten bot declarations are permitted",
        ));
    }
    let mut declared = [false; 10];
    for slot in slots {
        let value = declared
            .get_mut(usize::from(slot.0))
            .ok_or(BindingError::Configuration(
                "declared bot slot is outside 0..10",
            ))?;
        if *value {
            return Err(BindingError::Configuration(
                "declared bot slots must be distinct",
            ));
        }
        *value = true;
    }
    Ok(declared)
}

pub(super) fn engine_id(snapshot: &Snapshot, slot: SlotId) -> Option<i32> {
    let player = snapshot.players.iter().find(|player| player.slot == slot)?;
    let base = match player.team {
        Team::Radiant => 0,
        Team::Dire => 5,
        _ => return None,
    };
    let rank = snapshot
        .players
        .iter()
        .filter(|other| other.team == player.team && other.slot.0 < slot.0)
        .count();
    (rank < 5).then_some(base + rank as i32)
}

fn player(snapshot: &Snapshot, id: i32) -> Option<&Player> {
    snapshot
        .players
        .iter()
        .find(|player| engine_id(snapshot, player.slot) == Some(id))
}

pub(super) fn selected_hero(state: &State, id: i32) -> Option<HeroId> {
    if let Some(snapshot) = &state.snapshot {
        return player(snapshot, id).map(|player| player.hero);
    }
    let base = match state.team {
        Team::Radiant => 0,
        Team::Dire => 5,
        _ => return None,
    };
    let rank = usize::try_from(id.checked_sub(base)?).ok()?;
    if rank >= 5 {
        return None;
    }
    // Without a roster, only a contiguous prefix from wire slot zero proves a rank.
    // A missing lower slot could belong to another player on this same side.
    for lower in 0..=rank {
        if !state
            .seats
            .iter()
            .any(|seat| usize::from(seat.slot.0) == lower)
        {
            return None;
        }
    }
    let index = state
        .seats
        .iter()
        .position(|seat| usize::from(seat.slot.0) == rank)?;
    Some(state.picks[index].unwrap_or(state.seats[index].hero))
}

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetTeamPlayers", team_players)?;
    register_global(lua, context, "GetOpposingTeam", opposing_team)?;
    register_global(lua, context, "GetHeroLastSeenInfo", last_seen)?;
    register_unit(lua, context, "GetPlayerID", player_id)?;
    for name in [
        "GetTeamForPlayer",
        "IsPlayerBot",
        "GetHeroLevel",
        "GetHeroDeaths",
        "IsHeroAlive",
    ] {
        let context = context.clone();
        global(
            lua,
            name,
            Rc::new(move |vm, args| player_read(&context, vm, args, name)),
        )?;
    }
    Ok(())
}

fn roster<'a>(state: &'a State, vm: &mut Vm) -> Result<&'a Snapshot, LuaError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + 3 * (snapshot.players.len() * snapshot.players.len()) as u64)?;
    Ok(snapshot)
}

fn team_players(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let side = bounded_integer(vm, args, 0, 2, 3)?;
    let state = context.state.borrow();
    let snapshot = roster(&state, vm)?;
    let mut found = Vec::new();
    for player in &snapshot.players {
        if team_number(player.team) == f64::from(side)
            && let Some(id) = engine_id(snapshot, player.slot)
        {
            found.push(id);
        }
    }
    found.sort_unstable();
    let table = vm.new_table()?;
    for (index, id) in found.into_iter().enumerate() {
        vm.raw_set(
            table,
            Value::Number((index + 1) as f64),
            Value::Number(f64::from(id)),
        )?;
    }
    vm.results(&[Value::Table(table)])
}

fn opposing_team(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    let team = context.state.borrow().team;
    let other = match team {
        Team::Radiant => Team::Dire,
        Team::Dire => Team::Radiant,
        _ => return Err(vm.error("a playing team is required")),
    };
    vm.results(&[Value::Number(team_number(other))])
}

fn player_read(
    context: &ApiContext,
    vm: &mut Vm,
    args: Args,
    name: &str,
) -> Result<usize, LuaError> {
    let id = bounded_integer(vm, args, 0, 0, 9)?;
    let state = context.state.borrow();
    let snapshot = roster(&state, vm)?;
    let player =
        player(snapshot, id).ok_or_else(|| vm.error("player is not in the received roster"))?;
    let value = match name {
        "GetTeamForPlayer" => Value::Number(team_number(player.team)),
        "GetHeroLevel" => Value::Number(f64::from(player.level)),
        "GetHeroDeaths" => Value::Number(f64::from(player.deaths)),
        "IsHeroAlive" => Value::Boolean(player.respawn_left == 0),
        "IsPlayerBot" => Value::Boolean(
            state.seats.iter().any(|seat| seat.slot == player.slot)
                || state
                    .declared_bots
                    .get(usize::from(player.slot.0))
                    .copied()
                    .unwrap_or(false),
        ),
        _ => return Err(vm.error("unknown player query")),
    };
    vm.results(&[value])
}

fn player_id(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = roster(&state, vm)?;
    let slot = state
        .player(id)
        .map(|player| player.slot)
        .or_else(|| state.unit(id).and_then(|unit| unit.owner));
    if slot.is_none() && state.unit(id).is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    let result = slot
        .and_then(|slot| engine_id(snapshot, slot))
        .unwrap_or(-1);
    vm.results(&[Value::Number(f64::from(result))])
}

fn last_seen(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = bounded_integer(vm, args, 0, 0, 9)?;
    let state = context.state.borrow();
    let snapshot = roster(&state, vm)?;
    let player =
        player(snapshot, id).ok_or_else(|| vm.error("player is not in the received roster"))?;
    vm.charge_fuel(16 + 2 * snapshot.units.len() as u64)?;
    let known = if player.unit.is_some() {
        player.last_seen.into_iter().collect::<Vec<_>>()
    } else {
        snapshot
            .units
            .iter()
            .filter(|known| {
                known.unit.kind == bota_proto::UnitKind::Hero
                    && known.unit.owner == Some(player.slot)
            })
            .map(|known| known.last_visible)
            .collect()
    };
    let table = vm.new_table()?;
    for (index, (tick, position)) in known.into_iter().enumerate() {
        let entry = vm.new_table()?;
        let location = context.vectors.location(vm, position)?;
        let key = vm.new_string(b"location")?;
        vm.raw_set(entry, key, location)?;
        let key = vm.new_string(b"time_since_seen")?;
        vm.raw_set(
            entry,
            key,
            Value::Number(seconds(snapshot.tick.saturating_sub(tick))),
        )?;
        vm.raw_set(
            table,
            Value::Number((index + 1) as f64),
            Value::Table(entry),
        )?;
    }
    vm.results(&[Value::Table(table)])
}
