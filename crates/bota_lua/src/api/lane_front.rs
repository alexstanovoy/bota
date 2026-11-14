//! Lane fronts are derived from the client's own observations, never hidden units.
#![expect(
    clippy::float_arithmetic,
    reason = "the API returns normalized geometry of the configured lane"
)]
use super::{
    context::{ApiContext, register_global},
    globals::bounded_integer,
};
use crate::BindingError;
use bota_proto::{Team, UnitKind, Vec2};
use luagee::{Args, Lua, LuaError, Value, Vm};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetLaneFrontAmount", front)
}
fn front(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let team = if bounded_integer(vm, args, 0, 2, 3)? == 2 {
        Team::Radiant
    } else {
        Team::Dire
    };
    let lane = super::queries::lane_id(i64::from(bounded_integer(vm, args, 1, 1, 3)?))
        .expect("checked lane");
    let ignore_towers = match vm.arg(args, 2) {
        Value::Boolean(value) => value,
        _ => return Err(vm.arg_error(args, 2, "boolean expected")),
    };
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    if lane >= snapshot.map.lanes {
        return Err(vm.error("lane is not on this map"));
    }
    if snapshot.map.wave_lanes.len() != 1 {
        return Err(vm.error("lane fronts require a single wave lane on this host"));
    }
    let line = bota_core::lane_polyline(snapshot.map, lane);
    let towers = if team == Team::Radiant {
        snapshot.map.radiant_towers
    } else {
        snapshot.map.dire_towers
    };
    let work = work(
        line.len().saturating_sub(1),
        snapshot.units.len(),
        towers.len(),
    );
    vm.charge_fuel(work)?;
    let value = extreme(
        snapshot,
        state.team,
        team,
        lane,
        ignore_towers,
        &line,
        towers,
    );

    vm.results(&[Value::Number(f64::from(value.clamp(0.0, 1.0) as f32))])
}
fn extreme(
    snapshot: &crate::state::Snapshot,
    viewer: Team,
    team: Team,
    lane: u8,
    ignore_towers: bool,
    line: &[Vec2],
    towers: &[(u8, u8, Vec2)],
) -> f64 {
    let allied = team == viewer;
    let mut value = if allied { 0.0f64 } else { 1.0 };
    for known in &snapshot.units {
        let unit = &known.unit;
        if !known.visible || unit.hp <= 0 || unit.team != team {
            continue;
        }
        let creep = bota_core::kind_def(unit.kind).lane_creep && snapshot.map.wave_lanes[0] == lane;
        let tower = !ignore_towers
            && unit.kind == UnitKind::Tower
            && towers
                .iter()
                .any(|&(at, tier, pos)| at == lane && tier <= 3 && pos == unit.pos);
        if !creep && !tower {
            continue;
        }
        let (amount, distance) = along(line, unit.pos);
        if creep && distance >= 1000.0 * 1000.0 {
            continue;
        }
        let amount = if viewer == Team::Dire {
            1.0 - amount
        } else {
            amount
        };
        value = if allied {
            value.max(amount)
        } else {
            value.min(amount)
        };
    }
    value
}
fn work(segments: usize, units: usize, towers: usize) -> u64 {
    32 + 8 * segments as u64 + units as u64 * (4 + 8 * segments as u64 + towers as u64)
}
fn point(position: Vec2) -> [f64; 2] {
    [
        f64::from(position.x.raw) / 65536.0,
        f64::from(position.y.raw) / 65536.0,
    ]
}
fn along(line: &[Vec2], position: Vec2) -> (f64, f64) {
    let position = point(position);
    let mut total = 0.0;
    let mut nearest = (f64::MAX, 0.0);
    for segment in line.windows(2) {
        let a = point(segment[0]);
        let b = point(segment[1]);
        let delta = [b[0] - a[0], b[1] - a[1]];
        let squared = delta[0] * delta[0] + delta[1] * delta[1];
        let length = squared.sqrt();
        let fraction = if squared == 0.0 {
            0.0
        } else {
            ((position[0] - a[0]) * delta[0] + (position[1] - a[1]) * delta[1]) / squared
        }
        .clamp(0.0, 1.0);
        let dx = position[0] - a[0] - delta[0] * fraction;
        let dy = position[1] - a[1] - delta[1] * fraction;
        let distance = dx * dx + dy * dy;
        if distance < nearest.0 {
            nearest = (distance, total + length * fraction);
        }
        total += length;
    }
    (if total > 0.0 { nearest.1 / total } else { 0.0 }, nearest.0)
}
