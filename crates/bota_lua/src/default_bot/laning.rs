//! The ordinary, no-creep pregame movement slice. See native_laning.md for behaviour and coverage.

use super::{COMMAND_RETRY_TICKS, distance};
use crate::state::{Snapshot, State};
use bota_client_core::IssuedOrder;
use bota_proto::{EntityId, Fixed, Order, Target, Team, UnitKind, UnitView, Vec2};
use luagee::{LuaError, Vm};
use std::sync::OnceLock;

const FRONT_OFFSET: Fixed = Fixed::from_int(300);
const MOVE_TOLERANCE: Fixed = Fixed::from_int(100);
const MAX_LANE_POINTS: usize = 32;
static GEOMETRY: OnceLock<[Option<Geometry>; 2]> = OnceLock::new();

#[derive(Clone, Copy)]
struct Geometry {
    front: Vec2,
    target: Vec2,
}

/// A controller-owned action, not an observation of the unit's executing order.
#[derive(Clone, Copy)]
pub(crate) struct Movement {
    pub source: EntityId,
    destination: Option<Vec2>,
    issued_at: u32,
}

impl Movement {
    pub fn order(self) -> Order {
        Order::Move {
            target: self.destination.map_or(Target::None, Target::Pos),
        }
    }
}

pub(super) fn movement(
    previous: Option<Movement>,
    source: EntityId,
    position: Vec2,
    target: Vec2,
    tick: u32,
) -> Option<Movement> {
    let tolerance = i128::from(MOVE_TOLERANCE.raw).pow(2);
    if distance(position, target) <= tolerance {
        // Clear this controller's movement without cancelling unrelated actions.
        return previous
            .filter(|action| action.destination.is_some())
            .map(|_| Movement {
                source,
                destination: None,
                issued_at: tick,
            });
    }
    if previous.is_some_and(|action| {
        action.destination == Some(target)
            && tick.saturating_sub(action.issued_at) < COMMAND_RETRY_TICKS
    }) {
        return None;
    }
    Some(Movement {
        source,
        destination: Some(target),
        issued_at: tick,
    })
}

pub(super) fn publish(
    vm: &mut Vm,
    state: &mut State,
    action: Option<Movement>,
) -> Result<usize, LuaError> {
    if let Some(action) = action {
        state.order(vm, IssuedOrder::hero(action.order()))?;
        let index = state.bot_index();
        state.bots[index].native_laning = Some(action);
    }
    Ok(0)
}

pub(super) fn prepare() {
    GEOMETRY.get_or_init(|| {
        let map = bota_core::map_of(bota_core::MAP2_ID);
        let mut line = bota_core::lane_polyline(map, bota_core::rules::LANE_MID);
        let build = |towers: &[(u8, u8, Vec2)], line: &[Vec2]| {
            let front = towers
                .iter()
                .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 1)?
                .2;
            Some(Geometry {
                front,
                target: goal_on_line(line, front)?,
            })
        };
        let radiant = build(map.radiant_towers, &line);
        line.reverse();
        [radiant, build(map.dire_towers, &line)]
    });
}

pub(super) fn pregame_target(
    vm: &mut Vm,
    snapshot: &Snapshot,
    own: &UnitView,
) -> Result<Option<Vec2>, LuaError> {
    let side = usize::from(own.team == Team::Dire);
    let Some(geometry) = GEOMETRY.get().and_then(|goals| goals[side]) else {
        return Ok(None);
    };
    for known in &snapshot.units {
        vm.charge_fuel(1)?;
        if known.visible
            && known.unit.team == own.team
            && known.unit.hp > 0
            && bota_core::kind_def(known.unit.kind).creep
        {
            return Ok(None);
        }
    }
    for known in &snapshot.units {
        vm.charge_fuel(1)?;
        if known.visible
            && known.unit.kind == UnitKind::Tower
            && known.unit.team == own.team
            && known.unit.hp > 0
            && known.unit.pos == geometry.front
        {
            return Ok(Some(geometry.target));
        }
    }
    Ok(None)
}

pub(crate) fn goal_on_line(line: &[Vec2], front: Vec2) -> Option<Vec2> {
    if line.len() > MAX_LANE_POINTS {
        return None;
    }
    let index = line.iter().position(|&point| point == front)?;
    let mut remaining = FRONT_OFFSET;
    let mut position = front;
    for &point in line[..index].iter().rev() {
        let length = Fixed {
            raw: bota_core::isqrt64(position.distance_squared(point)) as i32,
        };
        if remaining <= length {
            return Some(bota_core::point_along(position, point, remaining));
        }
        remaining -= length;
        position = point;
    }
    Some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_movement_target_replaces_an_unexpired_lease() {
        let source = EntityId {
            idx: 1,
            generation: 0,
        };
        let first = Vec2::from_ints(1000, 1000);
        let changed = Vec2::from_ints(1200, 1000);
        let previous = movement(None, source, Vec2::ZERO, first, 10).expect("first command");
        let replacement = movement(Some(previous), source, Vec2::ZERO, changed, 11)
            .expect("retarget without waiting");
        assert_eq!(
            replacement.order(),
            Order::Move {
                target: Target::Pos(changed)
            }
        );
    }
}
