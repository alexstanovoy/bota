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
        let required = 1 + 4 + units * if thinking { 3 } else { 2 };
        for budget in [2, required - 1, required] {
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
            0 => 1 + 8 + units * (book + 3) + book,
            1 => 1 + 8 + units + book + snapshot.talents.len() as u64 * 8,
            2 => 1 + 8 + units * (slots + 1) + slots * (effects + 1),
            3 => 1 + 64 + units * 18,
            4 => 1 + 16 + units * 4,
            _ => unreachable!(),
        };
        drop(state);
        for budget in [2, work - 1, work] {
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
