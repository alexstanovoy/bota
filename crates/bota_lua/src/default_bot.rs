//! Deterministic SF Solo Mid fallback policy. Hook semantics follow the engine API;
//! policy thresholds are explicit reconstruction assumptions, not recovered engine internals.

use std::rc::Rc;
use bota_client_core::IssuedOrder;
use bota_proto::{DamageKind, HeroId, Order, StatusFlags, Target, Team, UnitKind, UnitView, Vec2};
use luagee::{Lua, LuaError, Value, Vm};
use crate::{BindingError, api::Shared, state::{PendingOnce, Snapshot, State}, team::BOT_MODES};

pub(crate) fn install(lua: &mut Lua, state: Shared) -> Result<Value, BindingError> {
    Ok(lua.function("default_mode", Rc::new(move |vm, args| {
        let mode = vm.check_number(args, 0)?;
        if mode.fract() != 0.0 || !(1.0..=BOT_MODES.len() as f64).contains(&mode) {
            return Err(vm.arg_error(args, 0, "known mode expected"));
        }
        let name = BOT_MODES[mode as usize - 1];
        let thinking = matches!(vm.arg(args, 1), Value::Boolean(true));
        callback(vm, &state, name, thinking)
    }))?)
}

fn context(state: &State) -> Option<(&Snapshot, &UnitView)> {
    let snapshot = state.snapshot.as_ref()?;
    if snapshot.map.id != bota_core::MAP2_ID || snapshot.players.len() != 2
        || snapshot.players.iter().any(|player| player.hero != HeroId::SHADOW_FIEND) { return None; }
    let own = state.unit(state.own_id()?)?;
    (own.hp > 0 && own.max_hp > 0 && own.kind == UnitKind::Hero && own.hero == Some(HeroId::SHADOW_FIEND)
        && own.team == state.team && own.owner == Some(state.current_slot)
        && state.player(own.id)?.respawn_left == 0).then_some((snapshot, own))
}

fn callback(vm: &mut Vm, shared: &Shared, mode: &str, thinking: bool) -> Result<usize, LuaError> {
    if !matches!(mode, "laning" | "attack" | "retreat" | "farm") {
        return vm.results(&[Value::Number(0.0)]);
    }
    let mut state = shared.borrow_mut();
    let Some((snapshot, own)) = context(&state) else { return vm.results(&[Value::Number(0.0)]); };
    vm.charge_fuel(4 + snapshot.units.len() as u64 * if thinking { 3 } else { 2 })?;
    if !thinking { return vm.results(&[Value::Number(desire(&state, snapshot, own, mode))]); }
    if !state.orders.is_empty() || !own.queue.is_empty()
        || own.statuses.bits & (StatusFlags::STUNNED | StatusFlags::FEARED | StatusFlags::CASTING | StatusFlags::CHANNELLING) != 0 {
        return Ok(0);
    }
    let pending = state.bots[state.bot_index()].pending_once;
    if pending.is_some_and(|shot| !shot.launched && snapshot.units.iter().any(|known| known.unit.id == shot.target && known.visible && known.unit.hp > 0)) { return Ok(0); }
    let (order, once) = decision(snapshot, own, mode);
    let source = own.id;
    let tick = snapshot.tick;
    state.order(vm, IssuedOrder::hero(order))?;
    if once && let Order::Attack { target: Target::Unit(target) } = order {
        let index = state.bot_index();
        state.bots[index].pending_once = Some(PendingOnce { source, target, tick, launched: false });
    }
    Ok(0)
}

fn distance(a: Vec2, b: Vec2) -> i128 {
    let x = i128::from(a.x.raw) - i128::from(b.x.raw);
    let y = i128::from(a.y.raw) - i128::from(b.y.raw);
    x * x + y * y
}

fn enemy<'a>(snapshot: &'a Snapshot, own: &UnitView, kind: UnitKind) -> Option<&'a UnitView> {
    snapshot.units.iter().filter(|known| known.visible && known.unit.team != own.team
        && known.unit.hp > 0 && known.unit.kind == kind
        && known.unit.statuses.bits & StatusFlags::INVULNERABLE == 0)
        .map(|known| &known.unit).min_by_key(|unit| distance(own.pos, unit.pos))
}

fn desire(state: &State, snapshot: &Snapshot, own: &UnitView, mode: &str) -> f64 {
    match mode {
        "laning" => 0.3,
        "retreat" if i64::from(own.hp) * 100 < i64::from(own.max_hp) * 35
            || (state.bots[state.bot_index()].mode == 4 && i64::from(own.hp) * 100 < i64::from(own.max_hp) * 90) => 0.95,
        "attack" if enemy(snapshot, own, UnitKind::Hero).is_some_and(|target| {
            distance(own.pos, target.pos) <= i128::from(900 * 65536i64).pow(2)
                && i64::from(own.hp) * i64::from(target.max_hp) >= i64::from(target.hp) * i64::from(own.max_hp)
        }) => 0.6,
        "farm" if enemy(snapshot, own, UnitKind::CreepNeutral).is_some_and(|target| distance(own.pos, target.pos) <= i128::from(900 * 65536i64).pow(2)) => 0.4,
        _ => 0.0,
    }
}

fn last_hit<'a>(snapshot: &'a Snapshot, own: &UnitView) -> Option<&'a UnitView> {
    snapshot.units.iter().filter(|known| known.visible && known.unit.hp > 0 && known.unit.statuses.bits & StatusFlags::INVULNERABLE == 0)
        .map(|known| &known.unit).filter(|target| {
            if !matches!(target.kind, UnitKind::CreepMelee | UnitKind::CreepFlagbearer | UnitKind::CreepRanged | UnitKind::CreepSiege) { return false; }
            let range = i128::from(own.attack_range.raw.max(0)) + i128::from(own.bound.raw.max(0)) + i128::from(target.bound.raw.max(0));
            if distance(own.pos, target.pos) > range * range { return false; }
            let class = bota_core::attack_class_pct(own.kind, target.kind);
            let damage = (i64::from(own.attack_damage_min.max(0)) * i64::from(class) / 100).clamp(0, i64::from(i32::MAX)) as i32;
            let hit = bota_core::mitigate(damage, DamageKind::Physical, target.armor, 0);
            target.hp <= hit && (target.team != own.team || i64::from(target.hp) * 100 <= i64::from(target.max_hp) * i64::from(bota_core::rules::DENY_HP_PCT))
        }).min_by_key(|target| (target.team == own.team, target.hp, target.id))
}

fn decision(snapshot: &Snapshot, own: &UnitView, mode: &str) -> (Order, bool) {
    if mode == "retreat" { return (Order::Move { target: Target::Pos(bota_core::fountain_pos(snapshot.map, own.team)) }, false); }
    let target = match mode {
        "attack" => enemy(snapshot, own, UnitKind::Hero).map(|target| (target, false)),
        "farm" => enemy(snapshot, own, UnitKind::CreepNeutral).map(|target| (target, false)),
        _ => last_hit(snapshot, own).map(|target| (target, true)),
    };
    if let Some((target, once)) = target { return (Order::Attack { target: Target::Unit(target.id) }, once); }
    let other = if own.team == Team::Radiant { Team::Dire } else { Team::Radiant };
    (Order::Attack { target: Target::Pos(bota_core::fountain_pos(snapshot.map, other)) }, false)
}
