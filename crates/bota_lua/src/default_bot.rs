//! SF Solo Mid defaults. native_laning.md describes pregame movement and its coverage;
//! the other policy thresholds remain explicit reconstruction assumptions.

mod auxiliary;
mod courier;
mod economy;
mod items;
mod laning;
const COMMAND_RETRY_TICKS: u32 = bota_core::rules::TICKS_PER_SECOND;
pub(crate) use auxiliary::{AuxiliaryHook, install as install_auxiliary};
pub(crate) use laning::Movement as LaningMovement;
#[cfg(test)]
pub(crate) use laning::goal_on_line;

use crate::{
    BindingError,
    api::Shared,
    state::{PendingOnce, Snapshot, State},
    team::BOT_MODES,
};
use bota_client_core::IssuedOrder;
use bota_proto::{Fixed, HeroId, Order, StatusFlags, Target, Team, UnitKind, UnitView, Vec2};
use luagee::{Lua, LuaError, Value, Vm};
use std::rc::Rc;

pub(crate) fn install(lua: &mut Lua, state: Shared) -> Result<Value, BindingError> {
    laning::prepare();
    Ok(lua.function(
        "default_mode",
        Rc::new(move |vm, args| {
            let mode = vm.check_number(args, 0)?;
            if mode.fract() != 0.0 || !(1.0..=BOT_MODES.len() as f64).contains(&mode) {
                return Err(vm.arg_error(args, 0, "known mode expected"));
            }
            let name = BOT_MODES[mode as usize - 1];
            let thinking = matches!(vm.arg(args, 1), Value::Boolean(true));
            callback(vm, &state, name, thinking)
        }),
    )?)
}

fn context(state: &State) -> Option<(&Snapshot, &UnitView)> {
    let snapshot = state.snapshot.as_ref()?;
    if !supported(snapshot) {
        return None;
    }
    let own = state.unit(state.own_id()?)?;
    (own.hp > 0
        && own.max_hp > 0
        && own.kind == UnitKind::Hero
        && own.hero == Some(HeroId::SHADOW_FIEND)
        && own.team == state.team
        && own.owner == Some(state.current_slot)
        && state.player(own.id)?.respawn_left == 0)
        .then_some((snapshot, own))
}

fn supported(snapshot: &Snapshot) -> bool {
    snapshot.map.id == bota_core::MAP2_ID
        && snapshot.players.len() == 2
        && snapshot
            .players
            .iter()
            .all(|player| player.hero == HeroId::SHADOW_FIEND)
        && snapshot
            .players
            .iter()
            .filter(|player| player.team == Team::Radiant)
            .count()
            == 1
        && snapshot
            .players
            .iter()
            .filter(|player| player.team == Team::Dire)
            .count()
            == 1
}

fn callback(vm: &mut Vm, shared: &Shared, mode: &str, thinking: bool) -> Result<usize, LuaError> {
    if !matches!(mode, "laning" | "attack" | "retreat" | "farm") {
        return vm.results(&[Value::Number(0.0)]);
    }
    let mut state = shared.borrow_mut();
    let Some((snapshot, own)) = context(&state) else {
        return vm.results(&[Value::Number(0.0)]);
    };
    let pregame = thinking && mode == "laning" && snapshot.game_tick < snapshot.horn;
    vm.charge_fuel(4 + snapshot.units.len() as u64 * if thinking { 3 } else { 2 })?;
    if !thinking {
        return vm.results(&[Value::Number(desire(&state, snapshot, own, mode))]);
    }
    if !state.orders.is_empty()
        || !own.queue.is_empty()
        || own.statuses.bits
            & (StatusFlags::STUNNED
                | StatusFlags::FEARED
                | StatusFlags::CASTING
                | StatusFlags::CHANNELLING)
            != 0
    {
        return Ok(0);
    }
    if pregame && let Some(target) = laning::pregame_target(vm, snapshot, own)? {
        let source = own.id;
        let previous = state.bots[state.bot_index()]
            .native_laning
            .filter(|action| action.source == source);
        let action = laning::movement(previous, source, own.pos, target, snapshot.tick);
        return laning::publish(vm, &mut state, action);
    }
    let pending = state.bots[state.bot_index()].pending_once;
    if mode != "retreat"
        && pending.is_some_and(|shot| {
            !shot.launched
                && snapshot
                    .units
                    .iter()
                    .any(|known| known.unit.id == shot.target && known.visible && known.unit.hp > 0)
        })
    {
        return Ok(0);
    }
    let (order, once) = decision(snapshot, own, mode)
        .map_err(|()| vm.error("received attack statistics exceed projection range"))?;
    let source = own.id;
    let tick = snapshot.tick;
    let expires_at = native_once_deadline(tick, own.attack_point);
    state.order(vm, IssuedOrder::hero(order))?;
    if once
        && let Order::Attack {
            target: Target::Unit(target),
        } = order
    {
        let index = state.bot_index();
        state.bots[index].pending_once = Some(PendingOnce {
            source,
            target,
            tick,
            launched: false,
            expires_at: Some(expires_at),
        });
    }
    Ok(0)
}

fn native_once_deadline(tick: u32, windup: u32) -> u32 {
    let second = u64::from(bota_core::rules::TICKS_PER_SECOND);
    let windup_ticks = (u64::from(windup) * second).div_ceil(1000);
    let lease = (windup_ticks + second).clamp(second, 10 * second) as u32;
    tick.saturating_add(lease)
}

fn distance(a: Vec2, b: Vec2) -> i128 {
    let x = i128::from(a.x.raw) - i128::from(b.x.raw);
    let y = i128::from(a.y.raw) - i128::from(b.y.raw);
    x * x + y * y
}

fn enemy<'a>(snapshot: &'a Snapshot, own: &UnitView, kind: UnitKind) -> Option<&'a UnitView> {
    snapshot
        .units
        .iter()
        .filter(|known| {
            known.visible
                && known.unit.team != own.team
                && known.unit.hp > 0
                && known.unit.kind == kind
                && known.unit.statuses.bits & StatusFlags::INVULNERABLE == 0
        })
        .map(|known| &known.unit)
        .min_by_key(|unit| distance(own.pos, unit.pos))
}

fn desire(state: &State, snapshot: &Snapshot, own: &UnitView, mode: &str) -> f64 {
    match mode {
        "laning" => 0.3,
        "retreat"
            if i64::from(own.hp) * 100 < i64::from(own.max_hp) * 35
                || (mode_is(state, "retreat")
                    && i64::from(own.hp) * 100 < i64::from(own.max_hp) * 90) =>
        {
            0.95
        }
        "attack"
            if enemy(snapshot, own, UnitKind::Hero).is_some_and(|target| {
                distance(own.pos, target.pos) <= i128::from(900 * 65536i64).pow(2)
                    && i64::from(own.hp) * i64::from(target.max_hp)
                        >= i64::from(target.hp) * i64::from(own.max_hp)
            }) =>
        {
            0.6
        }
        "farm"
            if enemy(snapshot, own, UnitKind::CreepNeutral).is_some_and(|target| {
                distance(own.pos, target.pos) <= i128::from(900 * 65536i64).pow(2)
            }) =>
        {
            0.4
        }
        _ => 0.0,
    }
}

fn projected_hit(own: &UnitView, target: &UnitView) -> Result<i32, ()> {
    let class = bota_core::attack_class_pct(own.kind, target.kind);
    let amount = (i64::from(own.attack_damage_min.max(0)) * i64::from(class) / 100)
        .clamp(0, i64::from(i32::MAX)) as i32;
    crate::numeric::physical_damage(amount, target.armor).ok_or(())
}

fn deniable_observation(target: &UnitView) -> bool {
    // Whole-point observations are i32, not necessarily representable in Q16.16.
    bota_core::kind_def(target.kind)
        .deny_below_pct
        .is_some_and(|share| {
            i64::from(target.hp) * 100 < i64::from(target.max_hp) * i64::from(share)
        })
}

fn last_hit<'a>(snapshot: &'a Snapshot, own: &UnitView) -> Result<Option<&'a UnitView>, ()> {
    let mut best: Option<&UnitView> = None;
    for known in &snapshot.units {
        let target = &known.unit;
        if !known.visible
            || target.hp <= 0
            || target.statuses.bits & StatusFlags::INVULNERABLE != 0
            || !matches!(
                target.kind,
                UnitKind::CreepMelee
                    | UnitKind::CreepFlagbearer
                    | UnitKind::CreepRanged
                    | UnitKind::CreepSiege
            )
        {
            continue;
        }
        let range = i128::from(own.attack_range.raw.max(0))
            + i128::from(own.bound.raw.max(0))
            + i128::from(target.bound.raw.max(0));
        if distance(own.pos, target.pos) > range * range {
            continue;
        }
        let hit = projected_hit(own, target)?;
        if target.hp > hit || (target.team == own.team && !deniable_observation(target)) {
            continue;
        }
        let rank = |unit: &UnitView| (unit.team == own.team, unit.hp, unit.id);
        if best.is_none_or(|previous| rank(target) < rank(previous)) {
            best = Some(target);
        }
    }
    Ok(best)
}

fn decision(snapshot: &Snapshot, own: &UnitView, mode: &str) -> Result<(Order, bool), ()> {
    if mode == "retreat" {
        return Ok((
            Order::Move {
                target: Target::Pos(bota_core::fountain_pos(snapshot.map, own.team)),
            },
            false,
        ));
    }
    let target = match mode {
        "attack" => enemy(snapshot, own, UnitKind::Hero).map(|target| (target, false)),
        "farm" => enemy(snapshot, own, UnitKind::CreepNeutral).map(|target| (target, false)),
        _ => last_hit(snapshot, own)?.map(|target| (target, true)),
    };
    if let Some((target, once)) = target {
        return Ok((
            Order::Attack {
                target: Target::Unit(target.id),
            },
            once,
        ));
    }
    Ok((
        Order::Attack {
            target: Target::Pos(lane_goal(snapshot, own)),
        },
        false,
    ))
}

fn lane_goal(snapshot: &Snapshot, own: &UnitView) -> Vec2 {
    let mid = bota_core::rules::MAP_SIZE / 2;
    let middle = Vec2::from_ints(mid, mid);
    let other = if own.team == Team::Radiant {
        Team::Dire
    } else {
        Team::Radiant
    };
    let enemy_base = bota_core::fountain_pos(snapshot.map, other);
    let front = snapshot
        .units
        .iter()
        .filter(|known| {
            known.visible
                && known.unit.team == own.team
                && known.unit.hp > 0
                && matches!(
                    known.unit.kind,
                    UnitKind::CreepMelee
                        | UnitKind::CreepFlagbearer
                        | UnitKind::CreepRanged
                        | UnitKind::CreepSiege
                )
        })
        .map(|known| bota_core::clamp_to_map(known.unit.pos))
        .min_by_key(|point| distance(*point, enemy_base));
    let Some(front) =
        front.filter(|front| distance(*front, enemy_base) < distance(middle, enemy_base))
    else {
        return middle;
    };
    let home = bota_core::fountain_pos(snapshot.map, own.team);
    let behind = bota_core::point_along(front, home, Fixed::from_int(200));
    // Stable waypoints avoid replacing a standing attack for every small movement of the wave.
    let cell = 256 * i64::from(Fixed::ONE.raw);
    let snap = |value: Fixed, from: Fixed, towards: Fixed| {
        let raw = i64::from(value.raw);
        let rounded = if towards >= from {
            (raw + cell - 1).div_euclid(cell) * cell
        } else {
            raw.div_euclid(cell) * cell
        };
        Fixed {
            raw: rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        }
    };
    bota_core::clamp_to_map(Vec2 {
        x: snap(behind.x, front.x, home.x),
        y: snap(behind.y, front.y, home.y),
    })
}

fn mode_is(state: &State, name: &str) -> bool {
    state.bots[state.bot_index()]
        .mode
        .checked_sub(1)
        .and_then(|index| BOT_MODES.get(usize::from(index)))
        .is_some_and(|mode| *mode == name)
}
