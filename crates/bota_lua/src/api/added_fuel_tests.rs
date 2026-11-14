use super::*;
use crate::{BotSeat, Keys, Limits, state::Snapshot};
use bota_proto::{HeroId, SlotId};
mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}
fn call(name: &str, fuel: u64) -> Result<Vec<Value>, luagee::Error> {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let own = sessions[0].own_hero().unwrap().id;
    let state = Rc::new(RefCell::new(State::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
    )));
    state.borrow_mut().snapshot = Some(Snapshot::capture(&sessions[0]).unwrap());
    let mut lua = Lua::new(
        Limits::DEFAULT.with_fuel_per_tick(fuel),
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )?;
    let vectors = vector::install(&mut lua).unwrap();
    install(&mut lua, state.clone(), vectors).unwrap();
    let receiver = state.borrow_mut().handle(lua.vm(), Some(own)).unwrap();
    let key = lua.string(name.as_bytes())?;
    let function = lua
        .vm()
        .raw_get(state.borrow().unit_methods.unwrap(), key)
        .unwrap();
    let mut args = vec![receiver];
    match name {
        "WasRecentlyDamagedByCreep" => args.push(Value::Number(1.0)),
        "GetActualIncomingDamage" => args.extend([Value::Number(100.0), Value::Number(2.0)]),
        "ActionImmediate_SwapItems" => args.extend([Value::Number(0.0), Value::Number(6.0)]),
        _ => {}
    }
    lua.begin_tick();
    lua.call(function, &args)
}
#[test]
fn scalar_observation_queries_charge_their_nonempty_view_work_before_results() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let s = &sessions[0];
    let n = s.entities().len() as u64;
    let p = s.current().unwrap().players.len() as u64;
    let t = s.current().unwrap().talents.len() as u64;
    let receiver = 1 + n / 8;
    let shared = receiver + 16 + n + p + t + 64;
    for (name, work) in [
        ("GetAbilityPoints", shared),
        ("GetAttackProjectileSpeed", shared),
        ("GetBaseDamageVariance", shared),
        ("GetStashValue", shared),
        ("GetAttackTarget", receiver + 8 + n),
        ("GetActualIncomingDamage", receiver + 16 + n),
        ("WasRecentlyDamagedByCreep", receiver + 16 + n + p),
        ("IsBot", receiver + 16 + n + p + 1),
        ("ActionImmediate_SwapItems", receiver + 10),
    ] {
        let minimum = support::bare_native_call_fuel() + work;
        assert!(
            call(name, minimum).is_ok(),
            "{name} minimum {minimum}: {:?}",
            call(name, minimum)
        );
        assert!(
            matches!(
                call(name, minimum - 1),
                Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
            ),
            "{name} minimum-1"
        );
    }
}
