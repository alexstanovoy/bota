use super::{COMMAND_RETRY_TICKS, distance, supported};
use crate::{api::Shared, state::NativeCourierCommand};
use bota_client_core::IssuedOrder;
use bota_proto::{AbilityId, AbilitySlot, Order, StatusFlags, Target, UnitKind};
use luagee::{LuaError, Vm};

pub(super) fn run(vm: &mut Vm, shared: &Shared) -> Result<usize, LuaError> {
    let mut state = shared.borrow_mut();
    let Some(snapshot) = state
        .snapshot
        .as_ref()
        .filter(|snapshot| supported(snapshot))
    else {
        return Ok(0);
    };
    vm.charge_fuel(16 + snapshot.units.len() as u64 * 4)?;
    let Some(player) = snapshot
        .players
        .iter()
        .find(|player| player.slot == state.current_slot)
    else {
        return Ok(0);
    };
    let Some(courier) = snapshot
        .units
        .iter()
        .filter(|known| {
            known.visible
                && known.unit.hp > 0
                && known.unit.kind == UnitKind::Courier
                && known.unit.team == state.team
                && known.unit.owner == Some(state.current_slot)
        })
        .map(|known| &known.unit)
        .min_by_key(|unit| unit.id)
    else {
        let index = state.bot_index();
        state.bots[index].native_courier = None;
        return Ok(0);
    };
    if courier.statuses.bits
        & (StatusFlags::STUNNED
            | StatusFlags::FEARED
            | StatusFlags::CASTING
            | StatusFlags::CHANNELLING)
        != 0
    {
        return Ok(0);
    }
    if state
        .orders
        .iter()
        .any(|order| order.unit == Some(courier.id))
    {
        return Ok(0);
    }
    let hero = player
        .unit
        .and_then(|id| state.unit(id))
        .filter(|unit| unit.hp > 0 && player.respawn_left == 0);
    let space = hero.is_some_and(|unit| unit.items.iter().any(Option::is_none));
    let carrying = courier.items.iter().any(Option::is_some);
    let stash = player
        .stash
        .as_ref()
        .is_some_and(|items| items.iter().any(Option::is_some));
    let home = bota_core::fountain_pos(snapshot.map, state.team);
    let home_range = i128::from(i64::from(bota_core::rules::COURIER_STASH_RANGE) * 65536);
    if !carrying
        && (!stash || !space || hero.is_none())
        && distance(courier.pos, home) <= home_range * home_range
    {
        let index = state.bot_index();
        state.bots[index].native_courier = None;
        return Ok(0);
    }
    let action = if hero.is_none() || !space {
        AbilityId::RETURN_ITEMS
    } else if carrying {
        AbilityId::DELIVER
    } else if stash {
        AbilityId::TAKE_STASH
    } else {
        AbilityId::RETURN_ITEMS
    };
    let index = state.bot_index();
    let prior = state.bots[index].native_courier;
    // A bounded lease also recovers after a death/respawn wholly between received snapshots.
    let fresh = prior.is_some_and(|command| {
        command.unit == courier.id
            && command.ability == action
            && snapshot.tick.saturating_sub(command.issued_at) < COMMAND_RETRY_TICKS
    });
    let ready = |id| {
        courier
            .abilities
            .iter()
            .enumerate()
            .find(|(_, ability)| {
                ability.id == id
                    && ability.level > 0
                    && ability.cooldown_left == 0
                    && ability.mana_cost.max(0) <= courier.mana
            })
            .and_then(|(slot, _)| u8::try_from(slot).ok())
            .map(|slot| Order::Cast {
                slot: AbilitySlot(slot),
                target: Target::None,
            })
    };
    let primary = if fresh { None } else { ready(action) };
    if !fresh && primary.is_none() {
        return Ok(0);
    }
    let threatened = snapshot.units.iter().any(|known| {
        known.visible
            && known.unit.team != state.team
            && known.unit.kind == UnitKind::Hero
            && known.unit.hp > 0
            && distance(courier.pos, known.unit.pos) <= i128::from(1000 * 65536i64).pow(2)
    });
    let travelling = if action == AbilityId::DELIVER {
        hero.is_some_and(|hero| {
            distance(courier.pos, hero.pos)
                > i128::from(i64::from(bota_core::rules::COURIER_HANDOFF_RANGE) * 65536).pow(2)
        })
    } else {
        distance(courier.pos, home) > home_range * home_range
    };
    let boost = if travelling {
        ready(AbilityId::BURST)
    } else {
        None
    };
    let shield = if threatened {
        ready(AbilityId::SHIELD)
    } else {
        None
    };
    let unit = courier.id;
    let issued_at = if primary.is_some() {
        snapshot.tick
    } else {
        prior.map_or(snapshot.tick, |command| command.issued_at)
    };
    let command = NativeCourierCommand {
        unit,
        ability: action,
        issued_at,
    };
    for order in [primary, boost, shield].into_iter().flatten() {
        state.order(vm, IssuedOrder::courier(unit, order))?;
    }
    state.bots[index].native_courier = Some(command);
    Ok(0)
}
