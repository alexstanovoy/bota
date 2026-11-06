#![allow(clippy::float_arithmetic, reason = "finite script geometry is converted to protocol fixed point at the boundary")]

use bota_proto::{UnitKind, Vec2};

use super::*;
use super::globals::{distance, integer};

fn boolean(vm: &mut Vm, args: Args, index: usize) -> Result<bool, LuaError> {
    match vm.arg(args, index) { Value::Boolean(value) => Ok(value), _ => Err(vm.arg_error(args, index, "boolean expected")) }
}

pub(super) fn install(lua: &mut Lua, methods: TableId, state: Shared, vectors: Vectors) -> Result<(), BindingError> {
    install_nearby(lua, methods, state.clone())?;
    install_positions(lua, methods, state.clone(), vectors)?;
    let capture = state.clone();
    method(lua, methods, "WasRecentlyDamagedByHero", Rc::new(move |vm, args| {
        let state = capture.borrow();
        let id = identity(vm, &state, args)?;
        let attacker = state.id(vm.arg(args, 1)).ok_or_else(|| vm.error("unit handle expected"))?;
        let seconds = vector::number(vm, args, 2)?;
        let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
        vm.charge_fuel(1 + snapshot.damage.len() as u64 / 8)?;
        let result = state.unit(attacker).is_some_and(|unit| unit.kind == UnitKind::Hero)
            && snapshot.damage.iter().any(|&(tick, source, target)| source == attacker && target == id && f64::from(snapshot.tick.saturating_sub(tick)) <= seconds * 30.0);
        vm.results(&[Value::Boolean(result)])
    }))?;
    method(lua, methods, "FindAoELocation", Rc::new(move |vm, args| aoe(vm, args, &capture_state(&state), vectors)))?;
    Ok(())
}

// These callbacks never invoke Lua or table metamethods while borrowing host state.
fn capture_state(state: &Shared) -> std::cell::Ref<'_, State> { state.borrow() }

fn install_nearby(lua: &mut Lua, methods: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["GetNearbyHeroes", "GetNearbyCreeps", "GetNearbyTowers"] {
        let capture = state.clone();
        method(lua, methods, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let own = identity(vm, &state, args)?;
            let radius = vector::number(vm, args, 1)?.clamp(0.0, 1600.0);
            let enemies = boolean(vm, args, 2)?;
            let mode = if name == "GetNearbyHeroes" { integer(vm, args, 3)? } else { 0 };
            let unit = state.unit(own).ok_or_else(|| vm.error("unit is no longer known"))?;
            let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
            let length = snapshot.units.len() as u64;
            vm.charge_fuel(1 + length * (1 + u64::from(length.max(1).ilog2())) / 4)?;
            let mut found: Vec<_> = snapshot.units.iter().filter(|known| {
                let other = &known.unit;
                let kind = match name { "GetNearbyHeroes" => other.kind == UnitKind::Hero, "GetNearbyCreeps" => bota_core::UNIT_KINDS[other.kind as usize].creep, _ => other.kind == UnitKind::Tower };
                let matches_mode = mode == 0 || other.owner.and_then(|slot| state.seats.iter().position(|seat| seat.slot == slot)).is_some_and(|index| i64::from(state.bots[index].mode) == mode);
                known.visible && other.id != own && other.hp > 0 && kind && matches_mode && (other.team != unit.team) == enemies && distance(other.pos, unit.pos) <= radius
            }).map(|known| (known.unit.pos.distance_squared(unit.pos), known.unit.id)).collect();
            found.sort_unstable();
            let table = vm.new_table()?;
            for (index, (_, id)) in found.into_iter().enumerate() {
                let value = state.handle(vm, Some(id))?;
                vm.raw_set(table, Value::Number((index + 1) as f64), value)?;
            }
            vm.results(&[Value::Table(table)])
        }))?;
    }
    Ok(())
}

fn install_positions(lua: &mut Lua, methods: TableId, state: Shared, vectors: Vectors) -> Result<(), BindingError> {
    let capture = state.clone();
    method(lua, methods, "DistanceFromFountain", Rc::new(move |vm, args| {
        let state = capture.borrow();
        let id = identity(vm, &state, args)?;
        let unit = state.unit(id).ok_or_else(|| vm.error("unit is no longer known"))?;
        let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
        vm.results(&[Value::Number(distance(unit.pos, bota_core::fountain_pos(snapshot.map, unit.team)))])
    }))?;
    let capture = state.clone();
    global(lua, "GetTower", Rc::new(move |vm, args| {
        let mut state = capture.borrow_mut();
        let team = match integer(vm, args, 0)? { 2 => Team::Radiant, 3 => Team::Dire, _ => return vm.results(&[Value::Nil]) };
        let tower = integer(vm, args, 1)?;
        let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
        vm.charge_fuel(1 + snapshot.units.len() as u64 / 8)?;
        let lane = u8::try_from(tower / 3).ok();
        let tier = u8::try_from(tower % 3 + 1).ok();
        let towers = if team == Team::Radiant { snapshot.map.radiant_towers } else { snapshot.map.dire_towers };
        let position = towers.iter().find(|&&(l, t, _)| Some(l) == lane && Some(t) == tier).map(|&(_, _, pos)| pos);
        let id = snapshot.units.iter().find(|known| known.visible && known.unit.kind == UnitKind::Tower && known.unit.team == team && Some(known.unit.pos) == position && known.unit.hp > 0).map(|known| known.unit.id);
        let value = state.handle(vm, id)?;
        vm.results(&[value])
    }))?;
    global(lua, "GetLocationAlongLane", Rc::new(move |vm, args| {
        let state = state.borrow();
        let lane = integer(vm, args, 0)?;
        let amount = vector::number(vm, args, 1)?.clamp(0.0, 1.0);
        let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
        let lane = u8::try_from(lane).ok().and_then(|lane| lane.checked_sub(1)).filter(|&lane| lane < snapshot.map.lanes).ok_or_else(|| vm.error("lane is not on this map"))?;
        vm.charge_fuel(128)?;
        let line = bota_core::lane_polyline(snapshot.map, lane);
        let total: f64 = line.windows(2).map(|points| distance(points[0], points[1])).sum();
        let mut remaining = total * amount;
        let mut result = *line.last().expect("a lane has endpoints");
        for points in line.windows(2) {
            let length = distance(points[0], points[1]);
            if remaining <= length && length > 0.0 {
                result = interpolate(points[0], points[1], remaining / length);
                break;
            }
            remaining -= length;
        }
        let value = vectors.location(vm, result)?;
        vm.results(&[value])
    }))?;
    Ok(())
}

fn interpolate(a: Vec2, b: Vec2, fraction: f64) -> Vec2 {
    let component = |a: i32, b: i32| bota_proto::Fixed { raw: (f64::from(a) + (f64::from(b) - f64::from(a)) * fraction).round_ties_even() as i32 };
    Vec2 { x: component(a.x.raw, b.x.raw), y: component(a.y.raw, b.y.raw) }
}

fn aoe(vm: &mut Vm, args: Args, state: &State, vectors: Vectors) -> Result<usize, LuaError> {
    let own = identity(vm, state, args)?;
    let enemies = boolean(vm, args, 1)?;
    let heroes = boolean(vm, args, 2)?;
    let base = vector::position(vm, vm.arg(args, 3))?;
    let range = vector::number(vm, args, 4)?.clamp(0.0, 32767.0);
    let radius = vector::number(vm, args, 5)?.clamp(0.0, 1600.0);
    vector::number(vm, args, 6)?;
    let health = integer(vm, args, 7)?;
    let unit = state.unit(own).ok_or_else(|| vm.error("unit is no longer known"))?;
    let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(1 + snapshot.units.len() as u64 / 4)?;
    let points: Vec<Vec2> = snapshot.units.iter().filter(|known| known.visible && known.unit.id != own && known.unit.hp > 0 && (known.unit.team != unit.team) == enemies && (!heroes || known.unit.kind == UnitKind::Hero) && (health <= 0 || i64::from(known.unit.hp) <= health) && distance(base, known.unit.pos) <= range + radius).map(|known| known.unit.pos).collect();
    let length = points.len() as u64;
    vm.charge_fuel(1 + length.saturating_mul(length).saturating_mul(length + 3).saturating_mul(8))?;
    let (target, count) = super::geometry::best_circle(base, range, radius, &points);
    let table = vm.new_table()?;
    let key = vm.new_string(b"count")?;
    vm.raw_set(table, key, Value::Number(count as f64))?;
    let key = vm.new_string(b"targetloc")?;
    let value = vectors.location(vm, target)?;
    vm.raw_set(table, key, value)?;
    vm.results(&[Value::Table(table)])
}
