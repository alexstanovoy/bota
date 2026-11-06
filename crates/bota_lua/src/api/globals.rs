#![allow(clippy::float_arithmetic, reason = "finite script utilities and conversion from protocol fixed point")]

use super::*;

pub(super) fn integer(vm: &mut Vm, args: Args, index: usize) -> Result<i64, LuaError> {
    Ok(vector::number(vm, args, index)?.trunc() as i64)
}

pub(super) fn string<'a>(vm: &'a mut Vm, args: Args, index: usize) -> Result<&'a [u8], LuaError> {
    let string = vm.check_string(args, index)?;
    let length = vm.bytes(Value::String(string)).expect("checked string is live").len();
    vm.charge_fuel(1 + length as u64 / 16)?;
    Ok(vm.bytes(Value::String(string)).expect("checked string is live"))
}

pub(super) fn distance(a: bota_proto::Vec2, b: bota_proto::Vec2) -> f64 {
    bota_core::isqrt64(a.distance_squared(b)) as f64 / 65536.0
}

pub(super) fn install(lua: &mut Lua, state: Shared, _vectors: Vectors) -> Result<(), BindingError> {
    for name in ["DotaTime", "GameTime", "RealTime", "GetSelectedHeroName", "SelectHero", "GetTeamMember", "GetUnitToUnitDistance", "GetItemCost"] {
        let capture = state.clone();
        global(lua, name, Rc::new(move |vm, args| {
            let mut state = capture.borrow_mut();
            let value = match name {
                "DotaTime" | "GameTime" | "RealTime" => {
                    let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
                    Value::Number((f64::from(snapshot.tick) - if name == "DotaTime" { f64::from(snapshot.horn) } else { 0.0 }) / 30.0)
                }
                "GetSelectedHeroName" => {
                    let slot = integer(vm, args, 0)?;
                    let hero = state.snapshot.as_ref().and_then(|snapshot| snapshot.players.iter().find(|player| i64::from(player.slot.0) == slot).map(|player| player.hero))
                        .or_else(|| state.seats.iter().position(|seat| i64::from(seat.slot.0) == slot).map(|index| state.picks[index].unwrap_or(state.seats[index].hero)));
                    vm.new_string(hero.and_then(bota_core::hero_def).map_or("", |hero| hero.key).as_bytes())?
                }
                "SelectHero" => {
                    let slot = integer(vm, args, 0)?;
                    let name = string(vm, args, 1)?;
                    let hero = std::str::from_utf8(name).ok().and_then(bota_core::hero_by_key);
                    if state.selecting {
                        if let Some(index) = state.seats.iter().position(|seat| i64::from(seat.slot.0) == slot) {
                            if state.picks[index].is_none() { state.picks[index] = Some(hero.unwrap_or(state.seats[index].hero)); }
                        }
                    }
                    return Ok(0);
                }
                "GetTeamMember" => {
                    let index = integer(vm, args, 0)?;
                    let id = index.checked_sub(1).and_then(|index| usize::try_from(index).ok()).and_then(|index| state.snapshot.as_ref()?.players.iter().filter(|player| player.team == state.team).nth(index)?.unit);
                    state.handle(vm, id)?
                }
                "GetUnitToUnitDistance" => {
                    vm.charge_fuel(1 + (state.units.len() + state.snapshot.as_ref().map_or(0, |snapshot| snapshot.units.len())) as u64 / 4)?;
                    let first = state.id(vm.arg(args, 0)).and_then(|id| state.unit(id)).map(|unit| unit.pos);
                    let second = state.id(vm.arg(args, 1)).and_then(|id| state.unit(id)).map(|unit| unit.pos);
                    Value::Number(match (first, second) { (Some(a), Some(b)) => distance(a, b), _ => return Err(vm.error("two unit handles expected")) })
                }
                "GetItemCost" => {
                    let key = string(vm, args, 0)?;
                    let cost = std::str::from_utf8(key).ok().and_then(bota_core::item_by_key).and_then(bota_core::item_def).map_or(0, |item| item.cost);
                    Value::Number(f64::from(cost))
                }
                _ => unreachable!("registered global"),
            };
            vm.results(&[value])
        }))?;
    }
    install_utilities(lua)
}

fn install_utilities(lua: &mut Lua) -> Result<(), BindingError> {
    for name in ["Clamp", "RemapValClamped"] {
        global(lua, name, Rc::new(move |vm, args| {
            let x = vector::number(vm, args, 0)?;
            let a = vector::number(vm, args, 1)?;
            let b = vector::number(vm, args, 2)?;
            let value = if name == "Clamp" {
                if a > b { return Err(vm.error("Clamp minimum exceeds maximum")); }
                x.clamp(a, b)
            } else {
                let c = vector::number(vm, args, 3)?;
                let d = vector::number(vm, args, 4)?;
                if a == b { if x >= b { d } else { c } } else { c + ((x - a) / (b - a)).clamp(0.0, 1.0) * (d - c) }
            };
            if !value.is_finite() { return Err(vm.error("numeric result is not finite")); }
            vm.results(&[Value::Number(value)])
        }))?;
    }
    Ok(())
}

pub(super) fn seconds(ticks: u32) -> f64 { f64::from(ticks) / 30.0 }
