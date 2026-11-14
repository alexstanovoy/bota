//! Thin name/identity adapters over shared observation and catalogue queries.
use super::{context::ApiContext, method, *};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    let methods = context
        .state
        .borrow()
        .unit_methods
        .expect("installed methods");
    super::context::register_unit(lua, context, "DistanceFromSecretShop", secret_distance)?;
    super::context::register_unit(lua, context, "GetActualIncomingDamage", incoming_damage)?;
    super::context::register_unit(lua, context, "GetBaseDamageVariance", variance)?;
    super::context::register_unit(lua, context, "GetAttackTarget", attack_target)?;
    super::context::register_unit(lua, context, "GetNearbyLaneCreeps", lane_creeps)?;
    super::context::register_unit(lua, context, "GetNearbyNeutralCreeps", neutral_creeps)?;
    super::context::register_unit(lua, context, "GetExtrapolatedLocation", extrapolated)?;
    for name in [
        "GetAbilityPoints",
        "GetAttackProjectileSpeed",
        "GetStashValue",
    ] {
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
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        16 + snapshot.units.len() as u64
            + snapshot.players.len() as u64
            + snapshot.talents.len() as u64
            + 64,
    )?;
    let player = state.player(id);
    let known = snapshot.units.iter().find(|known| known.unit.id == id);
    if known.is_none() && player.is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    let own = player.is_some_and(|player| player.team == state.team);
    let value = match name {
        "GetAbilityPoints" => points(&state, snapshot, id, known, player),
        "GetStashValue" => player
            .filter(|_| own)
            .and_then(|player| bota_client_core::stash_value(player.stash.as_deref()))
            .or(Some(-1)),
        "GetAttackProjectileSpeed" => {
            if own || known.is_some_and(|known| known.visible) {
                unit_data::base_definition(&state, id).map(|def| def.projectile_speed.unwrap_or(0))
            } else {
                Some(-1)
            }
        }
        _ => unreachable!(),
    };
    let value = value.map_or(Value::Nil, |value| Value::Number(f64::from(value)));
    vm.results(&[value])
}

fn attack_target(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(8 + snapshot.units.len() as u64)?;
    let target = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id && known.visible)
        .and_then(|known| known.attack_target);
    drop(state);
    let value = context.unit_value(vm, target)?;
    vm.results(&[value])
}
fn extrapolated(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let delay = vector::number(vm, args, 1)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + snapshot.units.len() as u64)?;
    let position = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id && known.visible)
        .and_then(|known| {
            bota_client_core::project_observed_position(known.unit.pos, known.velocity, delay)
        });
    let value = match position {
        Some(position) => context.vectors.location(vm, position)?,
        None => Value::Nil,
    };
    vm.results(&[value])
}

fn lane_creeps(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let enemies = match vm.arg(args, 2) {
        Value::Boolean(value) => value,
        _ => return Err(vm.arg_error(args, 2, "boolean expected")),
    };
    creeps(
        context,
        vm,
        args,
        bota_client_core::NearbyCreeps::Lane,
        enemies,
    )
}
fn neutral_creeps(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    creeps(
        context,
        vm,
        args,
        bota_client_core::NearbyCreeps::Neutral,
        false,
    )
}
#[expect(
    clippy::float_arithmetic,
    reason = "finite API radius rounded down to fixed planar range"
)]
fn creeps(
    context: &ApiContext,
    vm: &mut Vm,
    args: Args,
    kind: bota_client_core::NearbyCreeps,
    enemies: bool,
) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let radius = vector::number(vm, args, 1)?.clamp(0.0, 1600.0);
    let radius = bota_proto::Fixed {
        raw: (radius * 65536.0).floor() as i32,
    };
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let n = snapshot.units.len() as u64;
    vm.charge_fuel(16 + n * (8 + u64::from(n.max(1).ilog2())))?;
    let Some(actor) = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id && known.visible)
    else {
        return vm.results(&[Value::Nil]);
    };
    let ids = bota_client_core::nearby_creeps(
        &actor.unit,
        snapshot
            .units
            .iter()
            .filter(|known| known.visible)
            .map(|known| &known.unit),
        radius,
        kind,
        enemies,
    );
    drop(state);
    let table = vm.new_table()?;
    for (at, id) in ids.into_iter().enumerate() {
        let value = context.unit_value(vm, Some(id))?;
        vm.raw_set(table, Value::Number((at + 1) as f64), value)?;
    }
    vm.results(&[Value::Table(table)])
}

fn secret_distance(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        16 + snapshot.units.len() as u64
            + snapshot
                .map
                .secret_shops
                .iter()
                .map(|shop| shop.corners.len() as u64)
                .sum::<u64>(),
    )?;
    let distance = snapshot
        .units
        .iter()
        .find(|known| known.visible && known.unit.id == id)
        .and_then(|known| {
            bota_core::distance_from_secret_shop(snapshot.map, known.unit.pos, known.unit.bound)
        });
    vm.results(&[distance.map_or(Value::Nil, |value| Value::Number(f64::from(value)))])
}

fn incoming_damage(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let amount =
        vector::number(vm, args, 1)?.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let kind = match super::globals::integer(vm, args, 2)? {
        1 => bota_proto::DamageKind::Physical,
        2 => bota_proto::DamageKind::Magical,
        4 => bota_proto::DamageKind::Pure,
        _ => return Err(vm.arg_error(args, 2, "physical, magical or pure damage type expected")),
    };
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + snapshot.units.len() as u64)?;
    let value = match snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id && known.visible)
    {
        Some(known) => bota_client_core::observed_incoming_damage(&known.unit, amount, kind)
            .ok_or_else(|| vm.error("received damage statistics exceed projection range"))?,
        None => -1,
    };
    vm.results(&[Value::Number(f64::from(value))])
}

#[expect(
    clippy::float_arithmetic,
    reason = "the catalogue damage variance may be a half-integer"
)]
fn variance(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        16 + snapshot.units.len() as u64
            + snapshot.players.len() as u64
            + snapshot.talents.len() as u64
            + 64,
    )?;
    let owned = state
        .player(id)
        .is_some_and(|player| player.team == state.team);
    let visible = snapshot
        .units
        .iter()
        .any(|known| known.visible && known.unit.id == id);
    let value = if owned || visible {
        unit_data::base_definition(&state, id)
            .map(bota_core::base_damage_variance)
            .map_or(Value::Number(-1.0), |value| {
                Value::Number(f64::from(value.raw) / 65536.0)
            })
    } else {
        Value::Number(-1.0)
    };
    vm.results(&[value])
}

fn points(
    state: &State,
    snapshot: &crate::state::Snapshot,
    id: bota_proto::EntityId,
    known: Option<&crate::state::KnownUnit>,
    player: Option<&crate::state::Player>,
) -> Option<i32> {
    if known.is_some_and(|known| known.unit.team != state.team)
        || player.is_some_and(|player| player.team != state.team)
    {
        return Some(-1);
    }
    let Some(player) = player else {
        return Some(0);
    };
    let talent = snapshot
        .talents
        .iter()
        .find(|talent| talent.slot == player.slot);
    if let Some(known) = known {
        return bota_client_core::ability_points(&known.unit, talent, state.team).map(i32::from);
    }
    state
        .ability_book(id)
        .zip(talent)
        .map(|(abilities, talent)| {
            i32::from(bota_client_core::unspent_points(
                player.level,
                abilities,
                talent.attribute_bonus,
            ))
        })
}
