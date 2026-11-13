use super::*;
mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

fn fixture(budget: u64) -> LuaTeam {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits {
            fuel_per_tick: budget,
            ..Limits::DEFAULT
        },
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.state.borrow_mut().snapshot =
        Some(Snapshot::capture(&sessions[0]).expect("filtered snapshot"));
    lua.lua.begin_tick();
    lua
}

#[test]
fn native_mode_callbacks_charge_the_documented_work_above_the_bare_call() {
    let bare = support::bare_native_call_fuel();
    for thinking in [false, true] {
        let probe = fixture(10000);
        let units = probe
            .state
            .borrow()
            .snapshot
            .as_ref()
            .expect("snapshot")
            .units
            .len() as u64;
        let required = bare + 4 + units * if thinking { 3 } else { 2 };
        for budget in [bare, required - 1, required] {
            let mut lua = fixture(budget);
            let result = lua.lua.call(
                lua.default_mode,
                &[Value::Number(1.0), Value::Boolean(thinking)],
            );
            if budget == required {
                assert!(result.is_ok(), "{result:?}");
                assert_eq!(lua.lua.tick_fuel_left(), 0);
            } else {
                assert!(
                    matches!(result, Err(Error::Limit(Limit::FuelPerTick))),
                    "mode thinking={thinking} budget={budget} must fail beyond bare-call cost"
                );
            }
        }
    }
}

#[test]
fn native_auxiliary_and_courier_callbacks_charge_their_exact_work() {
    let bare = support::bare_native_call_fuel();
    for phase in 0..=4 {
        let probe = fixture(10000);
        let state = probe.state.borrow();
        let snapshot = state.snapshot.as_ref().expect("snapshot");
        let own = state.own_id().expect("hero");
        let units = snapshot.units.len() as u64;
        let book = state.ability_book(own).expect("book").len() as u64;
        let effects = state.unit(own).expect("hero").effects.len() as u64;
        let slots = bota_proto::limits::INVENTORY_SLOTS as u64;
        let work = match phase {
            0 => bare + 8 + units * (book + 3) + book,
            1 => bare + 8 + units + book + snapshot.talents.len() as u64 * 8,
            2 => bare + 8 + units * (slots + 1) + slots * (effects + 1),
            3 => bare + 64 + units * 18,
            4 => bare + 16 + units * 4,
            _ => unreachable!(),
        };
        drop(state);
        for budget in [bare, work - 1, work] {
            let mut lua = fixture(budget);
            if phase == 3 {
                lua.state
                    .borrow_mut()
                    .snapshot
                    .as_mut()
                    .expect("snapshot")
                    .catalogue = bota_core::Catalogue::first(0).expect("empty catalogue");
            }
            let result = lua
                .lua
                .call(lua.default_auxiliary, &[Value::Number(f64::from(phase))]);
            if budget == work {
                assert!(result.is_ok(), "phase={phase}: {result:?}");
                assert_eq!(lua.lua.tick_fuel_left(), 0);
            } else {
                assert!(
                    matches!(result, Err(Error::Limit(Limit::FuelPerTick))),
                    "phase={phase} budget={budget}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn a_native_launch_observed_at_lease_expiry_still_emits_its_completion_stop() {
    let (_, sessions, target) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut lua = fixture(10000);
    let mut snapshot = Snapshot::capture(&sessions[0]).expect("snapshot");
    let source = sessions[0].own_hero().expect("hero").id;
    let tick = snapshot.tick;
    snapshot.attacks.push(crate::state::AttackFact {
        tick,
        source,
        target: Some(target),
    });
    {
        let mut state = lua.state.borrow_mut();
        state.snapshot = Some(snapshot);
        state.bots[0].pending_once = Some(crate::state::PendingOnce {
            source,
            target,
            tick: tick - 1,
            launched: false,
            expires_at: Some(tick),
        });
    }
    let pending = lua.complete_once(0).expect("observed launch");
    assert!(pending.launched);
    assert_eq!(
        lua.state.borrow().orders,
        vec![IssuedOrder::hero(bota_proto::Order::Move {
            target: bota_proto::Target::None
        })]
    );
}

fn receive_promotion_tick(session: &mut ClientSession, view: &mut bota_proto::WorldView) {
    view.tick = session.current().expect("previous view").tick + 1;
    let tick = view.tick;
    session
        .ingest(
            bota_proto::ServerMsg::Snapshot { view: view.clone() },
            &mut (),
        )
        .expect("snapshot");
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

fn native_promotion_observation() -> (ClientSession, bota_proto::WorldView, bota_proto::EntityId) {
    let (_, mut sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut session = sessions.remove(0);
    let mut view = session.current().expect("view").clone();
    let own = session.own_hero().expect("hero").id;
    let target = view
        .units
        .iter()
        .find(|unit| unit.team == Team::Radiant && unit.kind == bota_proto::UnitKind::CreepMelee)
        .expect("friendly creep");
    let (target, position) = (target.id, target.pos);
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
    hero.pos = position;
    hero.attack_damage = 1000;
    hero.attack_damage_min = 1000;
    hero.attack_damage_max = 1000;
    hero.attack_point = 400;
    let creep = view
        .units
        .iter_mut()
        .find(|unit| unit.id == target)
        .expect("target");
    creep.hp = 50;
    creep.max_hp = 200;
    receive_promotion_tick(&mut session, &mut view);
    (session, view, target)
}

#[test]
fn an_explicit_script_once_promotes_a_native_request_beyond_its_original_lease() {
    let (mut session, mut view, target) = native_promotion_observation();
    let mut lua = fixture(Limits::DEFAULT.fuel_per_tick);
    lua.add_script(
        SlotId(0),
        ScriptKind::Mode(1),
        "once_promotion",
        br#"
        local evaluations, promoted = 0, false
        local function promote()
            if promoted then return end
            local bot = GetBot()
            for _, creep in ipairs(bot:GetNearbyCreeps(1600, false)) do
                if creep:GetHealth() == 50 then
                    bot:Action_AttackUnit(creep, true)
                    promoted = true
                    return
                end
            end
            error('the native once target must remain available')
        end
        function GetDesire()
            evaluations = evaluations + 1
            if evaluations >= 2 then Think = promote end
            return 1
        end
    "#,
    )
    .expect("partial mode");
    let attack = IssuedOrder::hero(bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(target),
    });
    let first = lua.think(&session).expect("native turn");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert!(first.bots[0].orders.contains(&attack));
    let native = lua.state.borrow().bots[0]
        .pending_once
        .expect("native last-hit request");
    assert_eq!(native.target, target);
    assert!(!native.launched);
    let deadline = native
        .expires_at
        .expect("native request must actually have a lease");
    assert_eq!(
        deadline,
        native.tick + 42,
        "400ms windup plus the native grace period"
    );
    assert!(
        deadline > native.tick + 9,
        "promotion must happen before expiry"
    );
    let mut script_attacks = 0;
    while session.current().expect("view").tick <= deadline {
        receive_promotion_tick(&mut session, &mut view);
        let tick = view.tick;
        let turn = lua.think(&session).expect("next turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        if turn.bots[0].orders.contains(&attack) {
            script_attacks += 1;
            assert_eq!(
                tick,
                native.tick + 9,
                "the second arbitration installs the scripted Think"
            );
        }
        let state = lua.state.borrow();
        let pending = state.bots[0]
            .pending_once
            .expect("the promoted script request must survive the native deadline");
        assert_eq!(
            (pending.source, pending.target, pending.tick),
            (native.source, target, native.tick)
        );
        assert!(!pending.launched, "no launch fact was supplied");
    }
    assert_eq!(
        script_attacks, 1,
        "one explicit script promotion, without resending"
    );
    assert!(view.tick > deadline);
    assert_eq!(
        lua.state.borrow().bots[0]
            .pending_once
            .expect("script once remains pending")
            .expires_at,
        None
    );
    view.tick += 1;
    let tick = view.tick;
    session
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("launch snapshot");
    session
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: vec![bota_proto::EventKind::AttackLaunched {
                    attacker: native.source,
                    target: Some(target),
                }],
            },
            &mut (),
        )
        .expect("delayed launch fact");
    let completed = lua
        .think(&session)
        .expect("script once completion after native expiry");
    assert!(completed.failures.is_empty(), "{:?}", completed.failures);
    assert_eq!(
        completed.bots[0]
            .orders
            .iter()
            .filter(|order| order.order
                == bota_proto::Order::Move {
                    target: bota_proto::Target::None
                })
            .count(),
        1,
        "the promoted request still completes through an ordinary Stop"
    );
    assert!(lua.state.borrow().bots[0].pending_once.is_none());
}
