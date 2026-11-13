//! Building identities resolved from public map positions and received sight.

use super::{
    context::{ApiContext, register_global},
    globals::bounded_integer,
};
use crate::BindingError;
use bota_proto::{Team, UnitKind};
use luagee::{Args, Lua, LuaError, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetBarracks", barracks)?;
    register_global(lua, context, "GetAncient", ancient)
}

fn barracks(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let side = bounded_integer(vm, args, 0, 2, 3)?;
    let index = bounded_integer(vm, args, 1, 0, i32::MAX)?;
    building(context, vm, side, Some(index))
}

fn ancient(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let side = bounded_integer(vm, args, 0, 2, 3)?;
    building(context, vm, side, None)
}

fn building(
    context: &ApiContext,
    vm: &mut Vm,
    side: i32,
    barracks: Option<i32>,
) -> Result<usize, LuaError> {
    use bota_core::rules::{LANE_BOT, LANE_MID, LANE_TOP};
    let mut state = context.state.borrow_mut();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        8 + snapshot.units.len() as u64
            + snapshot.map.barracks[0].len() as u64
            + snapshot.map.barracks[1].len() as u64,
    )?;
    let (team, at) = if side == 2 {
        (Team::Radiant, 0)
    } else {
        (Team::Dire, 1)
    };
    let (kind, position) = match barracks {
        None => (UnitKind::Ancient, snapshot.map.ancients[at]),
        Some(index) => {
            let lane = match index {
                0 | 1 => Some(LANE_TOP),
                2 | 3 => Some(LANE_MID),
                4 | 5 => Some(LANE_BOT),
                _ => None,
            };
            let position = lane.and_then(|lane| {
                snapshot.map.barracks[at]
                    .iter()
                    .find(|&&(l, ranged, _)| l == lane && ranged == (index % 2 == 1))
                    .map(|&(_, _, pos)| pos)
            });
            (UnitKind::Barracks, position)
        }
    };
    let id = position.and_then(|position| {
        snapshot
            .units
            .iter()
            .find(|known| {
                known.visible
                    && known.unit.kind == kind
                    && known.unit.team == team
                    && known.unit.pos == position
                    && known.unit.hp > 0
            })
            .map(|known| known.unit.id)
    });
    let value = state.handle(vm, id)?;
    vm.results(&[value])
}
