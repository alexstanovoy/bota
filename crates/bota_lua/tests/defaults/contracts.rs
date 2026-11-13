use super::*;
use bota_proto::{StatusFlags, UnitKind};

fn assert_order_limit(turn: &bota_lua::TeamTurn) {
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.failures.len(), 1);
    assert!(
        matches!(&turn.failures[0].error,luagee::Error::Runtime{message,..} if message.contains("order limit exceeded")),
        "{:?}",
        turn.failures
    );
}

#[test]
fn takeover_and_mode_plus_native_defaults_enforce_the_same_64_order_cap() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for n in [64, 70] {
        let source = format!(
            "function Think() local b=GetBot() for i=1,{n} do b:Action_MoveToLocation(Vector(i,i,0)) end end"
        );
        let turn = team(&[("bot_nevermore", &source)])
            .think(&sessions[0])
            .expect("takeover");
        if n == 64 {
            assert!(turn.failures.is_empty());
            assert_eq!(turn.bots[0].orders.len(), 64);
        } else {
            assert_order_limit(&turn);
        }
    }
    for n in [62, 63] {
        let source = format!(
            "function GetDesire() return .99 end function Think() local b=GetBot() for i=1,{n} do b:Action_MoveToLocation(Vector(i,i,0)) end end"
        );
        let turn = team(&[("mode_laning_generic", &source)])
            .think(&sessions[0])
            .expect("mode");
        if n == 62 {
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(turn.bots[0].orders.len(), 64);
            assert!(matches!(turn.bots[0].orders[62].order, Order::Learn { .. }));
            assert!(matches!(turn.bots[0].orders[63].order, Order::Buy { .. }));
        } else {
            assert_order_limit(&turn);
        }
    }
}

#[test]
fn mode_desires_reject_boundary_values_and_accept_signed_zero() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for expr in [
        "0/0",
        "1/0",
        "-1/0",
        "1e308",
        "{}",
        "'0.5'",
        "true",
        "1.0000000000000002",
        "-1e-320",
    ] {
        let source = format!("function GetDesire() return {expr} end");
        let turn = team(&[("mode_laning_generic", &source)])
            .think(&sessions[0])
            .expect("mode");
        assert_eq!(turn.failures.len(), 1, "{expr}");
        assert!(turn.bots[0].orders.is_empty());
        assert!(
            matches!(&turn.failures[0].error,luagee::Error::Runtime{message,..} if message.contains("finite number between zero and one")),
            "{:?}",
            turn.failures
        );
    }
    let turn = team(&[(
        "mode_laning_generic",
        "function GetDesire() return -0.0 end",
    )])
    .think(&sessions[0])
    .expect("zero");
    assert!(turn.failures.is_empty());
}

#[test]
fn native_purchase_survives_method_metatable_and_coroutine_tampering() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for source in [
        "function GetDesire() return .99 end function Think() local b=GetBot() pcall(function() b.ActionImmediate_PurchaseItem=function() error('replaced method') end end) pcall(function() getmetatable(b).__index.ActionImmediate_PurchaseItem=function() error('replaced fallback') end end) pcall(function() debug.getmetatable(b).__index.ActionImmediate_PurchaseItem=function() error('debug replacement') end end) pcall(function() setmetatable(b,nil) end) end",
        "function GetDesire() local co=coroutine.create(function() GetBot():GetNearbyHeroes(1600,true,0) coroutine.yield() end) assert(coroutine.resume(co)) setmetatable(_G,{__index=function() error('global lookup') end}) return .99 end function Think() end",
    ] {
        let turn = team(&[("mode_laning_generic", source)])
            .think(&sessions[0])
            .expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let purchases: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .filter(|order| matches!(order.order, Order::Buy { .. }))
            .collect();
        assert_eq!(
            purchases,
            vec![&IssuedOrder::hero(Order::Buy {
                item: bota_proto::ItemId::IRON_BRANCH
            })]
        );
    }
}

#[test]
fn separate_teams_do_not_share_globals_or_standard_library_tables() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut left = team(&[(
        "bot_nevermore",
        "function Think() shared_marker=42 string.shared_marker=7 end",
    )]);
    let mut right = team_for(
        Team::Dire,
        SlotId(1),
        &[(
            "bot_nevermore",
            "function Think() assert(shared_marker==nil) assert(string.shared_marker==nil) GetBot():Action_MoveToLocation(Vector(1,0,0)) end",
        )],
    );
    assert!(left.think(&sessions[0]).expect("left").failures.is_empty());
    let result = right.think(&sessions[1]).expect("right");
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(result.bots[0].orders.len(), 1);
}

#[test]
fn remembered_enemy_heroes_and_neutrals_never_receive_native_attack_orders() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    local.world_mut().cheats = true;
    local.world_mut().cheat(
        SlotId(0),
        None,
        bota_proto::Cheat::AllVision { on: true },
        &mut Vec::new(),
    );
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("public sight"))
    {
        support::fold(session, stream);
    }
    for kind in [UnitKind::Hero, UnitKind::CreepNeutral] {
        let mut session = sessions[0].clone();
        let mut view = session.current().expect("view").clone();
        let hero = session.own_hero().expect("hero").clone();
        let mut enemy = view
            .units
            .iter()
            .find(|unit| unit.team == Team::Dire && unit.kind == UnitKind::Hero)
            .expect("public enemy")
            .clone();
        enemy.id = bota_proto::EntityId {
            idx: 90000,
            generation: 0,
        };
        enemy.team = Team::Dire;
        enemy.owner = if kind == UnitKind::Hero {
            Some(SlotId(1))
        } else {
            None
        };
        enemy.kind = kind;
        enemy.abilities.clear();
        enemy.items.clear();
        enemy.town_portal = None;
        enemy.queue.clear();
        if kind != UnitKind::Hero {
            enemy.hero = None;
        }
        enemy.hp = 1;
        enemy.pos = hero.pos;
        enemy.statuses.bits = 0;
        let target = enemy.id;
        view.units.push(enemy);
        view.units.sort_by_key(|unit| unit.id);
        observation(&mut session, view.clone());
        let visible = team(&[]).think(&session).expect("visible");
        assert!(visible.failures.is_empty(), "{:?}", visible.failures);
        assert!(
            visible.bots[0].orders.iter().any(
                |order| matches!(order.order,Order::Attack{target:Target::Unit(id)} if id==target)
            ),
            "positive control {kind:?}"
        );
        view.units.retain(|unit| unit.id != target);
        observation(&mut session, view);
        assert!(
            session
                .entities()
                .iter()
                .any(|track| track.unit.id == target),
            "enemy is still remembered"
        );
        let hidden = team(&[]).think(&session).expect("hidden");
        assert!(hidden.failures.is_empty(), "{:?}", hidden.failures);
        assert!(
            !hidden.bots[0].orders.iter().any(
                |order| matches!(order.order,Order::Attack{target:Target::Unit(id)} if id==target)
            ),
            "remembered {kind:?} must be filtered"
        );
    }
}

#[test]
fn native_retreat_health_products_do_not_wrap_at_large_observed_values() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (hp, expected_retreat) in [(30_000_000, true), (70_000_000, false)] {
        let mut session = sessions[0].clone();
        let mut view = session.current().expect("view").clone();
        let own = session.own_hero().expect("hero").id;
        let hero = view
            .units
            .iter_mut()
            .find(|unit| unit.id == own)
            .expect("hero");
        hero.hp = hp;
        hero.max_hp = 100_000_000;
        observation(&mut session, view);
        let turn = team(&[]).think(&session).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(
            turn.bots[0].orders.iter().any(|order| order.order
                == Order::Move {
                    target: Target::Pos(bota_core::fountain_pos(session.map(), Team::Radiant))
                }),
            expected_retreat
        );
    }
}

#[test]
fn native_once_waits_without_respamming_then_recovers_a_rejected_or_dropped_attack() {
    let (_, sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    for (point, lease) in [(0, 30), (400, 42), (u32::MAX, 300)] {
        for rejected in [false, true] {
            let mut session = sessions[0].clone();
            let mut view = session.current().expect("view").clone();
            let own = session.own_hero().expect("hero").id;
            let creep = view
                .units
                .iter()
                .find(|unit| unit.team == Team::Radiant && unit.kind == UnitKind::CreepMelee)
                .expect("creep");
            let (target, pos) = (creep.id, creep.pos);
            for unit in &mut view.units {
                if unit.id != target && bota_core::kind_def(unit.kind).creep {
                    unit.hp = 100000;
                    unit.max_hp = 100000;
                }
            }
            let hero = view
                .units
                .iter_mut()
                .find(|unit| unit.id == own)
                .expect("hero");
            hero.pos = pos;
            hero.attack_damage = 1000;
            hero.attack_damage_min = 1000;
            hero.attack_damage_max = 1000;
            hero.attack_point = point;
            let creep = view
                .units
                .iter_mut()
                .find(|unit| unit.id == target)
                .expect("creep");
            creep.hp = 50;
            creep.max_hp = 200;
            observation(&mut session, view.clone());
            let mut lua = team(&[]);
            let first = lua.think(&session).expect("initial attack");
            assert!(first.failures.is_empty());
            let attack=first.bots[0].orders.iter().copied().find(|order|matches!(order.order,Order::Attack{target:Target::Unit(id)} if id==target)).expect("native once");
            if rejected {
                let issued = session.issue(attack).expect("issued");
                session
                    .ingest(
                        bota_proto::ServerMsg::OrderRejected {
                            seq: issued.seq,
                            reason: bota_proto::RejectReason::Disabled,
                        },
                        &mut (),
                    )
                    .expect("rejection");
            }
            view.units
                .iter_mut()
                .find(|unit| unit.id == target)
                .expect("target")
                .statuses
                .bits |= StatusFlags::INVULNERABLE;
            for elapsed in 1..=lease {
                observation(&mut session, view.clone());
                let turn = lua.think(&session).expect("pending");
                assert!(turn.failures.is_empty(), "{:?}", turn.failures);
                let body_orders = turn.bots[0]
                    .orders
                    .iter()
                    .filter(|order| body(order))
                    .count();
                assert_eq!(
                    body_orders,
                    usize::from(elapsed == lease),
                    "rejected={rejected}, elapsed={elapsed}, lease={lease}"
                );
            }
        }
    }
}
