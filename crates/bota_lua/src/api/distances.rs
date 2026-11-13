//! Planar distance queries over the same received positions as GetLocation.

use super::context::{ApiContext, register_global};
use crate::{BindingError, vector};
use bota_proto::{EntityId, Vec2};
use luagee::{Args, Lua, LuaError, Value, Vm};

#[derive(Clone, Copy)]
enum Query {
    Location,
    LocationSquared,
    UnitsSquared,
}
enum Destination {
    Point(Vec2),
    Unit(EntityId),
}

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetUnitToLocationDistance", location)?;
    register_global(
        lua,
        context,
        "GetUnitToLocationDistanceSqr",
        location_squared,
    )?;
    register_global(lua, context, "GetUnitToUnitDistanceSqr", units_squared)
}
fn location(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    query(context, vm, args, Query::Location)
}
fn location_squared(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    query(context, vm, args, Query::LocationSquared)
}
fn units_squared(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    query(context, vm, args, Query::UnitsSquared)
}

#[expect(
    clippy::float_arithmetic,
    reason = "API distances use floating world units; integer coordinates are widened before subtraction and squaring"
)]
fn squared(a: Vec2, b: Vec2) -> f64 {
    let x = f64::from(a.x.raw) - f64::from(b.x.raw);
    let y = f64::from(a.y.raw) - f64::from(b.y.raw);
    (x * x + y * y) / 4294967296.0
}

fn query(context: &ApiContext, vm: &mut Vm, args: Args, kind: Query) -> Result<usize, LuaError> {
    let id = context.unit_id(vm, args, 0)?;
    let destination = match kind {
        Query::UnitsSquared => Destination::Unit(context.unit_id(vm, args, 1)?),
        Query::Location | Query::LocationSquared => {
            Destination::Point(vector::position(vm, vm.arg(args, 1))?)
        }
    };
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + 2 * snapshot.units.len() as u64 + 2 * snapshot.players.len() as u64)?;
    let a = state
        .position(id)
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    let b = match destination {
        Destination::Point(point) => point,
        Destination::Unit(id) => state
            .position(id)
            .ok_or_else(|| vm.error("unit is no longer known"))?,
    };
    let distance = squared(a, b);
    vm.results(&[Value::Number(if matches!(kind, Query::Location) {
        distance.sqrt()
    } else {
        distance
    })])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn squared_distance_widens_both_raw_axes_before_subtraction_and_summing() {
        let high = Vec2 {
            x: bota_proto::Fixed { raw: i32::MAX },
            y: bota_proto::Fixed { raw: i32::MAX },
        };
        let low = Vec2 {
            x: bota_proto::Fixed { raw: i32::MIN },
            y: bota_proto::Fixed { raw: i32::MIN },
        };
        assert_eq!(squared(high, low), 8589934588.0);
        assert_eq!(squared(low, high), 8589934588.0);
    }
}
