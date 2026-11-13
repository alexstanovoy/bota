use super::*;
use crate::{Keys, Limits, state::Snapshot};
use bota_proto::{EffectId, EffectView, HeroId, SlotId};

mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

fn run(name: &str, fuel: u64) -> Result<Vec<Value>, luagee::Error> {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut snapshot = Snapshot::capture(&sessions[0]).expect("snapshot");
    let own = sessions[0].own_hero().expect("hero").id;
    snapshot.units.retain(|known| {
        known.unit.id == own
            || known.unit.kind == bota_proto::UnitKind::Ancient && known.unit.team == Team::Radiant
    });
    let body = &mut snapshot
        .units
        .iter_mut()
        .find(|known| known.unit.id == own)
        .expect("hero")
        .unit;
    body.effects = vec![
        EffectView {
            id: EffectId::STUNNED,
            ticks_left: Some(75),
            ticks_total: Some(75),
            stacks: Some(7),
            caster: None,
            inflictor: None,
        },
        EffectView {
            id: EffectId::ILLUSION,
            ticks_left: None,
            ticks_total: None,
            stacks: None,
            caster: None,
            inflictor: None,
        },
    ];
    let limits = Limits {
        fuel_per_tick: fuel,
        ..Limits::DEFAULT
    };
    let mut lua = Lua::new(
        limits,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )?;
    let state = Rc::new(RefCell::new(State::new(
        Team::Radiant,
        &[crate::BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
    )));
    state.borrow_mut().snapshot = Some(snapshot);
    let vectors = vector::install(&mut lua).expect("vectors");
    install(&mut lua, state.clone(), vectors).expect("API");
    let mut args = Vec::new();
    let function = if name == "GetUnitList" {
        args.push(Value::Number(0.0));
        lua.global(name)?
    } else if matches!(
        name,
        "GetGameMode"
            | "GetOpposingTeam"
            | "Min"
            | "Max"
            | "DotaTime"
            | "GameTime"
            | "RealTime"
            | "GetScriptDirectory"
    ) {
        if name == "Min" || name == "Max" {
            args.extend([Value::Number(1.0), Value::Number(2.0)]);
        }
        lua.global(name)?
    } else {
        let receiver = state
            .borrow_mut()
            .handle(lua.vm(), Some(own))
            .expect("receiver");
        args.push(receiver);
        if name == "HasModifier" || name == "GetModifierByName" {
            args.push(lua.string(b"modifier_stunned")?);
        } else if name.starts_with("GetModifier") {
            args.push(Value::Number(0.0));
        }
        let methods = state.borrow().unit_methods.expect("methods");
        let key = lua.string(name.as_bytes())?;
        lua.vm().raw_get(methods, key).expect("method")
    };
    lua.begin_tick();
    lua.call(function, &args)
}

#[test]
fn unit_list_fuel_has_a_fixed_minimum_for_two_received_units() {
    assert_budget("GetUnitList", 104);
}

/// `host_work` is the fuel the callback charges above the bare call of a native.
fn assert_budget(name: &str, host_work: u64) {
    let minimum = support::bare_native_call_fuel() + host_work;
    let success = run(name, minimum);
    assert!(success.is_ok(), "{name} at {minimum}: {success:?}");
    assert!(
        matches!(
            run(name, minimum - 1),
            Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
        ),
        "{name} must refuse a budget of {}",
        minimum - 1
    );
}

#[test]
fn modifier_readers_charge_their_fixed_minimum_with_two_received_effects() {
    for (name, host_work) in [
        ("NumModifiers", 33),
        ("HasModifier", 35),
        ("GetModifierByName", 35),
        ("GetModifierName", 38),
        ("GetModifierRemainingDuration", 33),
        ("GetModifierStackCount", 33),
    ] {
        assert_budget(name, host_work);
    }
}

#[test]
fn constant_work_queries_charge_a_fixed_minimum_for_their_host_work() {
    for (name, host_work) in [
        ("GetGameMode", 8),
        ("GetOpposingTeam", 8),
        ("Min", 8),
        ("Max", 8),
        ("GetLocation", 13),
        ("GetOrigin", 13),
    ] {
        assert_budget(name, host_work);
    }
}

#[test]
fn clock_and_script_directory_queries_charge_their_explicit_minimum() {
    for (name, host_work) in [
        ("DotaTime", 8),
        ("GameTime", 8),
        ("RealTime", 8),
        ("GetScriptDirectory", 12),
    ] {
        assert_budget(name, host_work);
    }
}
