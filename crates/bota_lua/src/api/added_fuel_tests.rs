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

mod pins {
    use super::*;
    use crate::api::guard_tests::Rig;

    /// The fuel a call takes above the interpreter's own price for calling a native.
    fn spent(rig: &mut Rig, function: Value, args: &[Value]) -> u64 {
        rig.lua.begin_tick();
        let before = rig.lua.tick_fuel_left();
        rig.lua.call(function, args).expect("call");
        before - rig.lua.tick_fuel_left() - support::bare_native_call_fuel()
    }

    /// Fuel a call needs: the exact minimum passes and one less fails with the per-tick limit.
    fn assert_exact(rig: &mut Rig, label: &str, function: Value, args: &[Value], work: u64) {
        let minimum = support::bare_native_call_fuel() + work;
        for (fuel, passes) in [(minimum - 1, false), (minimum, true)] {
            rig.lua.begin_tick();
            let spend = rig
                .lua
                .tick_fuel_left()
                .checked_sub(fuel + support::bare_native_call_fuel())
                .expect("tick budget");
            let drain = rig
                .lua
                .function(
                    "budget",
                    Rc::new(move |vm, _| {
                        vm.charge_fuel(spend)?;
                        Ok(0)
                    }),
                )
                .expect("drain");
            rig.lua.call(drain, &[]).expect("drain call");
            assert_eq!(rig.lua.tick_fuel_left(), fuel);
            let result = rig.lua.call(function, args);
            if passes {
                assert!(result.is_ok(), "{label} needs {work}: {result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
                    ),
                    "{label} must fail with {fuel}: {result:?}"
                );
            }
        }
    }

    fn global(rig: &mut Rig, name: &str) -> Value {
        rig.lua.global(name).expect("global")
    }

    fn method(rig: &mut Rig, abilities: bool, name: &str) -> Value {
        let table = if abilities {
            rig.state.borrow().ability_methods
        } else {
            rig.state.borrow().unit_methods
        }
        .expect("methods");
        let key = rig.lua.string(name.as_bytes()).expect("name");
        rig.lua.vm().raw_get(table, key).expect("method")
    }

    #[test]
    fn every_other_new_call_charges_its_documented_work_exactly() {
        let mut rig = Rig::new(|_, _| {});
        rig.run("me = GetBot(); ab = me:GetAbilityByName('nevermore_shadowraze1'); pt = Vector(0, 0, 0); path = {pt, pt, pt}")
            .expect("setup");
        let (n, p, ability_count, stock, corners, felled, planted, blockers, trees, opaque) = {
            let state = rig.state.borrow();
            let snapshot = state.snapshot.as_ref().unwrap();
            (
                snapshot.units.len() as u64,
                snapshot.players.len() as u64,
                state.abilities.len() as u64,
                snapshot.stock.len() as u64,
                snapshot
                    .map
                    .secret_shops
                    .iter()
                    .map(|shop| shop.corners.len() as u64)
                    .sum::<u64>(),
                snapshot.felled_trees.len() as u64,
                snapshot.planted_trees.len() as u64,
                snapshot.map.base_blockers.len() as u64,
                snapshot.statics.trees().len() as u64,
                snapshot.statics.opaque_cells().len() as u64,
            )
        };
        let units = rig.state.borrow().units.len() as u64;
        let receiver = 1 + units / 8 + n / 8;
        let me = global(&mut rig, "me");
        let ab = global(&mut rig, "ab");
        let pt = global(&mut rig, "pt");
        let path = global(&mut rig, "path");
        let grid = bota_core::rules::GRID_CELLS as u64;

        // The price of making a Vector, and of an empty result table, are the interpreter's.
        let vector = {
            let make = rig.vectors;
            let function = rig
                .lua
                .function(
                    "vector",
                    Rc::new(move |vm, _| {
                        make.location(vm, bota_proto::Vec2::ZERO)?;
                        Ok(0)
                    }),
                )
                .unwrap();
            spent(&mut rig, function, &[])
        };
        let table = {
            let function = rig
                .lua
                .function(
                    "table",
                    Rc::new(|vm, _| {
                        vm.new_table()?;
                        Ok(0)
                    }),
                )
                .unwrap();
            spent(&mut rig, function, &[])
        };
        let key = rig.lua.string(b"item_branches").unwrap();
        let ability_work = 16 + bota_core::ABILITIES.len() as u64 + ability_count + n + p;
        let scan = 16 + n + p;
        let creeps = receiver + 16 + n * (8 + u64::from(n.max(1).ilog2())) + table;
        let seconds = Value::Number(5.0);
        let cases: Vec<(&str, Value, Vec<Value>, u64)> = vec![
            (
                "GetItemStockCount",
                global(&mut rig, "GetItemStockCount"),
                vec![key],
                8 + 13 + bota_core::ITEMS.len() as u64 + 1 + stock,
            ),
            (
                "GetChannelTime",
                method(&mut rig, true, "GetChannelTime"),
                vec![ab],
                ability_work,
            ),
            (
                "IsInAbilityPhase",
                method(&mut rig, true, "IsInAbilityPhase"),
                vec![ab],
                ability_work,
            ),
            (
                "GetPowerTreadsStat",
                method(&mut rig, true, "GetPowerTreadsStat"),
                vec![ab],
                ability_work,
            ),
            (
                "GetExtrapolatedLocation",
                method(&mut rig, false, "GetExtrapolatedLocation"),
                vec![me, Value::Number(1.0)],
                receiver + 16 + n + vector,
            ),
            (
                "WasRecentlyDamagedByHero",
                method(&mut rig, false, "WasRecentlyDamagedByHero"),
                vec![me, me, seconds],
                2 * receiver + scan,
            ),
            (
                "GetNearbyLaneCreeps",
                method(&mut rig, false, "GetNearbyLaneCreeps"),
                vec![me, Value::Number(500.0), Value::Boolean(true)],
                creeps,
            ),
            (
                "GetNearbyNeutralCreeps",
                method(&mut rig, false, "GetNearbyNeutralCreeps"),
                vec![me, Value::Number(500.0)],
                creeps,
            ),
            (
                "GetHeightLevel",
                global(&mut rig, "GetHeightLevel"),
                vec![pt],
                8,
            ),
            ("GetTimeOfDay", global(&mut rig, "GetTimeOfDay"), vec![], 8),
            (
                "GetItemSlotType",
                method(&mut rig, false, "GetItemSlotType"),
                vec![me, Value::Number(0.0)],
                8 + receiver + n,
            ),
            (
                "DistanceFromSecretShop",
                method(&mut rig, false, "DistanceFromSecretShop"),
                vec![me],
                receiver + 16 + n + corners,
            ),
            (
                "GetTreeLocation",
                global(&mut rig, "GetTreeLocation"),
                vec![Value::Number(0.0)],
                8 + vector,
            ),
            (
                "ActionQueue_MoveToLocation",
                method(&mut rig, false, "ActionQueue_MoveToLocation"),
                vec![me, pt],
                receiver + 8,
            ),
            (
                "Action_MovePath",
                method(&mut rig, false, "Action_MovePath"),
                vec![me, path],
                // Prepaid scan, plus the interpreter's one fuel for each of the slots read to find a hole.
                receiver + 16 + 9 * bota_core::rules::MAX_QUEUED_ORDERS as u64 + 1,
            ),
            (
                "Action_UseAbilityOnTree",
                method(&mut rig, false, "Action_UseAbilityOnTree"),
                vec![me, ab, Value::Number(0.0)],
                receiver + 1 + ability_count / 8,
            ),
        ];
        for (label, function, args, work) in cases {
            assert_exact(&mut rig, label, function, &args, work);
            rig.state.borrow_mut().orders.clear();
        }

        // Visibility pays its scan every call and the forest rebuild only when the forest changed.
        let visible = global(&mut rig, "IsLocationVisible");
        let scan = 16 + felled + planted + (n + blockers) * (grid + 1);
        let rebuild = 3 * grid * grid + opaque + trees * 16 + planted * 16;
        assert_exact(
            &mut rig,
            "IsLocationVisible (first)",
            visible,
            &[pt],
            scan + rebuild,
        );
        assert_exact(&mut rig, "IsLocationVisible (cached)", visible, &[pt], scan);
    }
}
