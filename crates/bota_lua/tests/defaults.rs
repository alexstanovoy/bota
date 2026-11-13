mod support;

use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ModuleBank};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

fn team(files: &[(&str, &str)]) -> LuaTeam {
    team_for(Team::Radiant, SlotId(0), files)
}

fn team_for(side: Team, slot: SlotId, files: &[(&str, &str)]) -> LuaTeam {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    for &(name, source) in files {
        modules.insert(name, source.as_bytes()).expect("source");
    }
    let mut team = LuaTeam::new(
        side,
        &[BotSeat {
            slot,
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    team.set_modules(modules).expect("modules");
    team.load_pack().expect("pack");
    team
}

fn body(order: &IssuedOrder) -> bool {
    order.unit.is_none()
        && matches!(
            order.order,
            Order::Move { .. } | Order::Attack { .. } | Order::Cast { .. } | Order::Use { .. }
        )
}

fn observation(session: &mut bota_client_core::ClientSession, mut view: bota_proto::WorldView) {
    view.tick = session.current().expect("previous view").tick + 1;
    let tick = view.tick;
    session
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("observation");
    session
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
}

#[test]
fn default_denies_are_strictly_below_the_core_health_boundary() {
    let (_, sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let original = sessions[0].current().expect("view");
    let own = sessions[0].own_hero().expect("hero").id;
    let creep = original
        .units
        .iter()
        .find(|unit| unit.team == Team::Radiant && unit.kind == bota_proto::UnitKind::CreepMelee)
        .expect("friendly creep");
    let (target, position) = (creep.id, creep.pos);
    for (hp, max_hp, damage, expected) in [
        (100, 200, 1000, false),
        (98, 200, 1000, true),
        (19000, 40000, 30000, true),
        (20000, 40000, 30000, false),
        (10_000_000, 100_000_000, 100_000_000, true),
        (30_000_000, 100_000_000, 100_000_000, true),
        (50_000_000, 100_000_000, 100_000_000, false),
    ] {
        let mut session = sessions[0].clone();
        let mut view = original.clone();
        let hero = view
            .units
            .iter_mut()
            .find(|unit| unit.id == own)
            .expect("hero");
        hero.pos = position;
        hero.attack_damage_min = damage;
        hero.attack_damage_max = damage;
        hero.attack_damage = damage;
        for unit in &mut view.units {
            if unit.id != target && bota_core::kind_def(unit.kind).creep {
                unit.hp = 100000;
                unit.max_hp = 100000;
            }
        }
        let creep = view
            .units
            .iter_mut()
            .find(|unit| unit.id == target)
            .expect("creep");
        creep.hp = hp;
        creep.max_hp = max_hp;
        observation(&mut session, view);
        let turn = team(&[]).think(&session).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders.iter().any(|order| matches!(order.order, Order::Attack { target: Target::Unit(id) } if id == target)), expected, "observed health {hp}/{max_hp}");
    }
}

#[test]
fn out_of_range_observations_are_rejected_without_retiring_the_interpreter() {
    let (_, sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut session = sessions[0].clone();
    let original = session.current().expect("view").clone();
    let own = session.own_hero().expect("hero").id;
    let target = original
        .units
        .iter()
        .find(|unit| unit.team == Team::Radiant && unit.kind == bota_proto::UnitKind::CreepMelee)
        .expect("creep");
    let (id, position) = (target.id, target.pos);
    let mut view = original.clone();
    let own = view
        .units
        .iter_mut()
        .find(|unit| unit.id == own)
        .expect("hero");
    own.pos = position;
    own.attack_damage_min = i32::MAX;
    own.attack_damage_max = i32::MAX;
    own.attack_damage = i32::MAX;
    let target = view
        .units
        .iter_mut()
        .find(|unit| unit.id == id)
        .expect("creep");
    target.hp = 1;
    target.max_hp = 200;
    target.armor = bota_proto::Fixed::MIN;
    observation(&mut session, view);
    let mut lua = team(&[]);
    let rejected = lua.think(&session).expect("bounded input error");
    assert!(rejected.bots[0].orders.is_empty());
    assert_eq!(rejected.failures.len(), 1);
    assert!(
        matches!(&rejected.failures[0].error, luagee::Error::Runtime { message, .. } if message.contains("statistics"))
    );
    observation(&mut session, original);
    let recovered = lua.think(&session).expect("next valid observation");
    assert!(recovered.failures.is_empty(), "{:?}", recovered.failures);
    assert!(recovered.bots[0].orders.iter().any(body));
}

#[test]
fn default_retreat_supersedes_an_unlaunched_once_request() {
    let (mut local, mut sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut lua = team(&[(
        "mode_laning_generic",
        "function GetDesire() if GetBot():GetHealth()>10 then return .99 else return 0 end end function Think() if not requested then local b=GetBot() b:Action_AttackUnit(b:GetNearbyCreeps(1600,true)[1],true) requested=true end end",
    )]);
    let first = lua.think(&sessions[0]).expect("once turn");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    let target = first.bots[0]
        .orders
        .iter()
        .find_map(|order| match order.order {
            Order::Attack {
                target: Target::Unit(id),
            } => Some(id),
            _ => None,
        })
        .expect("once target");
    let orders: Vec<_> = first.bots[0]
        .orders
        .iter()
        .map(|&order| sessions[0].issue(order).expect("order"))
        .collect();
    for tick in 0..9 {
        let sent = if tick == 0 { orders.as_slice() } else { &[] };
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[sent, &[]]).expect("step"))
        {
            support::fold(session, stream);
        }
    }
    let own = sessions[0].own_hero().expect("hero");
    let enemy = sessions[0]
        .current()
        .expect("view")
        .units
        .iter()
        .find(|unit| unit.id == target)
        .expect("living target");
    assert!(enemy.hp > 0);
    assert!(
        own.pos.distance_squared(enemy.pos)
            > (own.attack_range + own.bound + enemy.bound).squared_raw()
    );
    let source = own.id;
    let mut view = sessions[0].current().expect("view").clone();
    view.units
        .iter_mut()
        .find(|unit| unit.id == source)
        .expect("hero")
        .hp = 1;
    observation(&mut sessions[0], view);
    let turn = lua.think(&sessions[0]).expect("retreat");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.iter().any(|order| order.order
        == Order::Move {
            target: Target::Pos(bota_core::fountain_pos(sessions[0].map(), Team::Radiant))
        }));
}

#[test]
fn lane_waypoints_stay_behind_the_friendly_front_for_both_sides() {
    let (_, sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    for (index, side, front) in [
        (0, Team::Radiant, Vec2::from_ints(9500, 9500)),
        (1, Team::Dire, Vec2::from_ints(9000, 9000)),
    ] {
        let mut session = sessions[index].clone();
        let mut view = session.current().expect("view").clone();
        let home = bota_core::fountain_pos(session.map(), side);
        let mut selected = false;
        for unit in &mut view.units {
            if unit.team == side
                && matches!(
                    unit.kind,
                    bota_proto::UnitKind::CreepMelee
                        | bota_proto::UnitKind::CreepFlagbearer
                        | bota_proto::UnitKind::CreepRanged
                        | bota_proto::UnitKind::CreepSiege
                )
            {
                unit.pos = if selected {
                    home
                } else {
                    selected = true;
                    front
                };
            }
        }
        assert!(selected, "friendly wave");
        observation(&mut session, view);
        let turn = team_for(side, SlotId(index as u8), &[])
            .think(&session)
            .expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let goal = turn.bots[0]
            .orders
            .iter()
            .find_map(|order| match order.order {
                Order::Attack {
                    target: Target::Pos(goal),
                } if order.unit.is_none() => Some(goal),
                _ => None,
            })
            .expect("lane waypoint");
        let dot = (i128::from(goal.x.raw) - i128::from(front.x.raw))
            * (i128::from(home.x.raw) - i128::from(front.x.raw))
            + (i128::from(goal.y.raw) - i128::from(front.y.raw))
                * (i128::from(home.y.raw) - i128::from(front.y.raw));
        assert!(
            dot >= 0,
            "{side:?} waypoint {goal:?} is ahead of the wave {front:?}"
        );
    }
}

#[test]
fn default_laning_moves_behind_its_mid_tower_before_creeps_spawn() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = team(&[]).think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders,
        vec![
            IssuedOrder::hero(Order::Move {
                target: Target::Pos(support::pregame_laning_goal(
                    sessions[0].map(),
                    Team::Radiant
                )),
            }),
            IssuedOrder::hero(Order::Learn {
                slot: bota_proto::AbilitySlot(0)
            }),
            IssuedOrder::hero(Order::Buy {
                item: bota_proto::ItemId::IRON_BRANCH
            }),
        ]
    );
}

#[test]
fn missing_or_nil_mode_hooks_use_defaults_and_partial_think_still_overrides() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for source in [
        "",
        "function GetDesire() return nil end",
        "function GetDesire() return .9 end",
    ] {
        let mut lua = team(&[("mode_laning_generic", source)]);
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(
            turn.bots[0].orders.iter().any(body),
            "missing native Think for {source:?}"
        );
    }
    let mut lua = team(&[(
        "mode_laning_generic",
        "function Think() GetBot():Action_MoveToLocation(Vector(100,200,0)) end",
    )]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.iter().any(|order| order.order
        == Order::Move {
            target: Target::Pos(Vec2::from_ints(9316, 9416))
        }));
}

#[test]
fn an_empty_think_is_an_override_and_full_takeovers_receive_no_default_actions() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(&[(
        "mode_laning_generic",
        "function GetDesire() return 1 end function Think() end",
    )]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(!turn.bots[0].orders.iter().any(body));
    let mut lua = team(&[("bot_nevermore", "function Think() end")]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.is_empty());
}

#[test]
fn default_work_uses_the_interpreters_tick_fuel_and_discards_its_orders_on_exhaustion() {
    let limits = Limits::DEFAULT
        .with_fuel_per_call(2000)
        .with_fuel_per_tick(8);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        limits,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.failures.len(), 1);
    assert!(matches!(
        turn.failures[0].error,
        luagee::Error::Limit(luagee::Limit::FuelPerTick)
    ));
}

#[test]
fn an_unoverridden_sf_bot_plays_to_match_end_through_ordinary_orders() {
    let mut lua = team(&[]);
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut sent = 0;
    let mut ended = false;
    for tick in 0..bota_core::MAP2_TICK_CAP {
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        if tick == 0 {
            assert!(turn.bots[0].orders.iter().any(body), "default bot must act");
        }
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("order"))
            .collect();
        sent += orders.len();
        let opponent = sessions[1]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(Vec2::from_ints(9216, 9216)),
            }))
            .expect("opponent");
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&orders, &[opponent]]).expect("step"))
        {
            ended |= support::fold(session, stream).is_some();
        }
        if ended {
            break;
        }
    }
    assert!(ended);
    assert!(sent > 0);
}

#[test]
fn a_minion_only_hero_file_preserves_modes_and_auxiliary_defaults() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(&[
        ("bot_nevermore", "function MinionThink(unit) end"),
        (
            "mode_laning_generic",
            "function GetDesire() return 1 end function Think() GetBot():Action_MoveToLocation(Vector(100,200,0)) end",
        ),
    ]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.iter().any(|order| order.order
        == Order::Move {
            target: Target::Pos(Vec2::from_ints(9316, 9416))
        }));
    assert!(
        turn.bots[0]
            .orders
            .iter()
            .any(|order| matches!(order.order, Order::Learn { .. })),
        "a minion hook does not replace hero skill learning"
    );
}

#[test]
fn a_full_takeover_does_not_load_unused_mode_sources() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(&[
        ("bot_nevermore", "function Think() end"),
        ("mode_laning_generic", "this is not valid Lua"),
    ]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.is_empty());
}

#[test]
fn lazy_mode_setup_preserves_the_active_turn_fuel_budget() {
    let limits = Limits::DEFAULT
        .with_fuel_per_call(4000)
        .with_fuel_per_tick(1500);
    let mut modules = ModuleBank::new(limits);
    modules
        .insert(
            "bot_nevermore",
            b"local n=0 for i=1,250 do n=n+i end function MinionThink(unit) end",
        )
        .expect("hero source");
    modules.insert("mode_laning_generic", b"function GetDesire() local n=0 for i=1,250 do n=n+i end return 1 end function Think() end").expect("mode source");
    modules.insert("ability_item_usage_nevermore", b"function AbilityUsageThink() end function ItemUsageThink() end function CourierUsageThink() end function BuybackUsageThink() end function AbilityLevelUpThink() end").expect("usage source");
    modules
        .insert(
            "item_purchase_nevermore",
            b"function ItemPurchaseThink() end",
        )
        .expect("purchase source");
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        limits,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_modules(modules).expect("modules");
    lua.load_pack().expect("pack");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(
        turn.failures.len(),
        1,
        "lazy setup must not renew the hero's turn fuel"
    );
    assert!(matches!(
        turn.failures[0].error,
        luagee::Error::Limit(luagee::Limit::FuelPerTick)
    ));
}

#[test]
fn conformance_counts_distinguish_script_hooks_from_native_fallback() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let scripted = team(&[("bot_nevermore", "function Think() end")])
        .think(&sessions[0])
        .expect("takeover turn");
    assert_eq!(scripted.hook_activity.initializations, 1);
    assert_eq!(scripted.hook_activity.scripted, 1);
    assert_eq!(scripted.hook_activity.native, 0);
    let mut fallback = team(&[]);
    let native = fallback.think(&sessions[0]).expect("native turn");
    assert_eq!(native.hook_activity.initializations, 0);
    assert_eq!(native.hook_activity.scripted, 0);
    assert!(native.hook_activity.native > 0);
    let initialized = team(&[("bot_nevermore", "return {}")])
        .think(&sessions[0])
        .expect("initialization-only turn");
    assert_eq!(initialized.hook_activity.initializations, 1);
    assert_eq!(initialized.hook_activity.scripted, 0);
    assert!(initialized.hook_activity.native > 0);
    let mut session = sessions[0].clone();
    observation(&mut session, sessions[0].current().expect("view").clone());
    let next = fallback.think(&session).expect("next native turn");
    assert_eq!(next.hook_activity.scripted, 0);
    assert!(
        next.hook_activity.native < native.hook_activity.native,
        "per-turn counts exclude earlier mode arbitration"
    );
}

#[path = "defaults/contracts.rs"]
mod contracts;
