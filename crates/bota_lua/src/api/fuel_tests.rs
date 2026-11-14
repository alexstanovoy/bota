use super::*;
use crate::{Keys, Limits, state::Snapshot};
use bota_proto::{EffectId, EffectView, HeroId, SlotId};

mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

fn run(name: &str, fuel: u64) -> Result<Vec<Value>, luagee::Error> {
    let unrelated = name == "IncomingUnrelated";
    let populated = name == "IncomingPopulated";
    let name = if unrelated || populated {
        "GetIncomingTrackingProjectiles"
    } else {
        name
    };
    let active = name == "GetCurrentActiveAbilityCasting";
    let name = if active {
        "GetCurrentActiveAbility"
    } else {
        name
    };
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
    if active {
        assert_eq!(body.abilities.len(), 6, "six received Shadow Fiend slots");
        body.cast = Some(bota_proto::CastView {
            ability: Some(body.abilities[0].id),
            goes_off_in: 1,
        });
        body.statuses.bits |= bota_proto::StatusFlags::CASTING;
    }
    let position = body.pos;
    if unrelated || populated {
        snapshot.projectiles = vec![bota_proto::ProjectileView {
            id: bota_proto::EntityId {
                idx: 99999,
                generation: 1,
            },
            pos: position,
            facing: bota_proto::Angle::default(),
            team: Team::Dire,
            ability: None,
            source: None,
            target: populated.then_some(own),
            inflictor: None,
            seen_by: None,
            model: Some(bota_proto::ProjectileModel::Hero(HeroId::SHADOW_FIEND)),
        }];
    }

    let metadata = matches!(name, "IsTrained" | "IsTalent" | "IsUltimate");
    let skill = bota_core::ability_def(body.abilities[0].id)
        .expect("skill")
        .key;
    let limits = Limits::DEFAULT.with_fuel_per_tick(if metadata || active {
        Limits::DEFAULT.fuel_per_call
    } else {
        fuel
    });
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
    state.borrow_mut().activity_profiles =
        std::sync::Arc::new(std::collections::BTreeMap::from([(
            "npc_dota_hero_nevermore".to_owned(),
            bota_client_core::ActivityProfile {
                idle_ms: Some(1000),
                ..Default::default()
            },
        )]));
    state.borrow_mut().snapshot = Some(snapshot);
    let vectors = vector::install(&mut lua).expect("vectors");
    install(&mut lua, state.clone(), vectors).expect("API");
    let mut args = Vec::new();
    let function = if matches!(
        name,
        "GetUnitToLocationDistance" | "GetUnitToLocationDistanceSqr" | "GetUnitToUnitDistanceSqr"
    ) {
        let receiver = state
            .borrow_mut()
            .handle(lua.vm(), Some(own))
            .expect("receiver");
        args.push(receiver);
        if name == "GetUnitToUnitDistanceSqr" {
            args.push(receiver);
        } else {
            args.push(vectors.location(lua.vm(), position).expect("point"));
        }
        lua.global(name)?
    } else if name == "GetUnitList" {
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
            | "GetGlyphCooldown"
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
        if metadata || active {
            let methods = state.borrow().unit_methods.expect("unit methods");
            let key = lua.string(b"GetAbilityByName")?;
            let getter = lua.vm().raw_get(methods, key).expect("getter");
            let key = lua.string(skill.as_bytes())?;
            let values = lua.call(getter, &[receiver, key])?;
            args.push(if metadata { values[0] } else { receiver });
        } else {
            args.push(receiver);
        }
        if name == "HasModifier" || name == "GetModifierByName" {
            args.push(lua.string(b"modifier_stunned")?);
        } else if name == "FindItemSlot" {
            args.push(lua.string(b"item_branches")?);
        } else if name.starts_with("GetModifier") {
            args.push(Value::Number(0.0));
        }
        let methods = if metadata {
            state.borrow().ability_methods
        } else {
            state.borrow().unit_methods
        }
        .expect("methods");
        let key = lua.string(name.as_bytes())?;
        lua.vm().raw_get(methods, key).expect("method")
    };
    lua.begin_tick();
    if metadata || active {
        // Handle creation is trusted setup. Drain the tick to the requested test budget
        // without reading a callback's charges or changing interpreter limits in flight.
        let spend = lua
            .tick_fuel_left()
            .checked_sub(fuel + support::bare_native_call_fuel())
            .expect("setup budget");
        let drain = lua.function(
            "test_budget",
            Rc::new(move |vm, _| {
                vm.charge_fuel(spend)?;
                Ok(0)
            }),
        )?;
        lua.call(drain, &[])?;
        assert_eq!(lua.tick_fuel_left(), fuel);
    }
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

#[test]
fn ability_metadata_queries_charge_their_explicit_minimum() {
    for name in ["IsTrained", "IsTalent", "IsUltimate"] {
        assert_budget(name, 9);
    }
}

#[test]
fn named_item_slot_search_pays_for_its_bounded_storage_scan() {
    assert_budget("FindItemSlot", 210);
}

#[test]
fn distance_queries_pay_for_each_observed_position_lookup() {
    for (name, host_work) in [
        ("GetUnitToLocationDistance", 25),
        ("GetUnitToLocationDistanceSqr", 25),
        ("GetUnitToUnitDistanceSqr", 26),
    ] {
        assert_budget(name, host_work);
    }
}

#[test]
fn glyph_cooldown_and_request_pay_their_explicit_minimum() {
    assert_budget("GetGlyphCooldown", 8);
    assert_budget("ActionImmediate_Glyph", 11);
}

#[test]
fn idle_active_ability_query_charges_its_explicit_minimum() {
    assert_budget("GetCurrentActiveAbility", 9);
}

#[test]
fn active_ability_query_charges_its_six_slot_scan_and_cached_handle() {
    assert_budget("GetCurrentActiveAbilityCasting", 17);
}

#[test]
fn buyback_calls_pay_a_fixed_minimum_for_two_received_seats() {
    for name in [
        "GetBuybackCooldown",
        "GetBuybackCost",
        "HasBuyback",
        "ActionImmediate_Buyback",
    ] {
        assert_budget(name, 11);
    }
}

#[test]
fn activity_queries_charge_their_explicit_work_and_received_unit_searches() {
    // Two retained bodies and two public roster rows: 8 work + 1 handle + 8 searches.
    assert_budget("GetAnimActivity", 17);
    assert_budget("GetAnimCycle", 17);
}

#[test]
fn empty_incoming_projectile_query_charges_scan_and_result_table() {
    // 16 work + 1 handle + 8 body/roster scan + 4 result allocation.
    assert_budget("GetIncomingTrackingProjectiles", 29);
}

#[test]
fn nonempty_projectile_scans_pay_for_every_received_record() {
    assert_budget("IncomingUnrelated", 31);
}
#[test]
fn populated_projectile_results_pay_their_exact_scan_row_and_allocation_budget() {
    // Frozen luagee 0.1 allocation prices plus the documented host row charge.
    assert_budget("IncomingPopulated", 111);
}

#[test]
fn environment_queries_pay_exact_native_and_host_prices() {
    fn call(name: &str, fuel: u64) -> Result<Vec<Value>, luagee::Error> {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let snapshot = Snapshot::capture(&sessions[0]).unwrap();
        let own = sessions[0].own_hero().unwrap().id;
        let state = Rc::new(RefCell::new(State::new(
            Team::Radiant,
            &[crate::BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
        )));
        state.borrow_mut().snapshot = Some(snapshot);
        let mut lua = Lua::new(
            Limits::DEFAULT.with_fuel_per_tick(fuel),
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )?;
        let vectors = vector::install(&mut lua).unwrap();
        install(&mut lua, state.clone(), vectors).unwrap();
        if name == "GetCourier" {
            let id = state
                .borrow()
                .snapshot
                .as_ref()
                .unwrap()
                .units
                .iter()
                .find(|known| {
                    known.unit.kind == bota_proto::UnitKind::Courier
                        && known.unit.team == Team::Radiant
                })
                .unwrap()
                .unit
                .id;
            state.borrow_mut().handle(lua.vm(), Some(id)).unwrap();
        }
        let (function, args) = if matches!(
            name,
            "DistanceFromSideShop"
                | "ActionImmediate_Chat"
                | "Action_Chat"
                | "GetNearbyTrees"
                | "NumQueuedActions"
        ) {
            let receiver = state.borrow_mut().handle(lua.vm(), Some(own)).unwrap();
            let key = lua.string(name.as_bytes())?;
            {
                let mut args = vec![receiver];
                if name == "GetNearbyTrees" {
                    args.push(Value::Number(0.0));
                } else if !matches!(name, "DistanceFromSideShop" | "NumQueuedActions") {
                    args.push(lua.string(b"hi")?);
                    args.push(Value::Boolean(false));
                }
                (
                    lua.vm()
                        .raw_get(state.borrow().unit_methods.unwrap(), key)
                        .unwrap(),
                    args,
                )
            }
        } else {
            let args = match name {
                "GetLaneFrontAmount" => {
                    vec![Value::Number(2.0), Value::Number(2.0), Value::Boolean(true)]
                }
                "GetShopLocation" => vec![Value::Number(2.0), Value::Number(1.0)],
                "GetCourier" => vec![Value::Number(0.0)],
                "IsItemPurchasedFromSideShop" => vec![lua.string(b"item_boots")?],
                _ => vec![],
            };
            (lua.global(name)?, args)
        };
        lua.begin_tick();
        lua.call(function, &args)
    }
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let units = sessions[0].entities().len() as u64;
    let trees = sessions[0].statics().trees().len() as u64;
    let felled = sessions[0].current().unwrap().felled_trees.len();
    let search = 1 + felled.max(1).ilog2() as u64;
    let segments =
        bota_core::lane_polyline(sessions[0].map(), bota_core::rules::LANE_MID).len() - 1;
    let towers = sessions[0].map().radiant_towers.len();
    let front_price = 32 + 8 * segments as u64 + units * (4 + 8 * segments as u64 + towers as u64);
    for (name, work) in [
        ("GetShopLocation", 12),
        ("GetGameState", 8),
        ("IsItemPurchasedFromSideShop", 9),
        ("GetNumCouriers", 8 + units),
        ("GetCourier", 9 + units),
        ("DistanceFromSideShop", 9 + units / 8),
        ("ActionImmediate_Chat", 14 + units / 8),
        ("Action_Chat", 14 + units / 8),
        (
            "GetNearbyTrees",
            13 + units + units / 8 + trees * (1 + search),
        ),
        ("NumQueuedActions", 9 + units / 8),
        ("GetLaneFrontAmount", front_price),
    ] {
        let minimum = support::bare_native_call_fuel() + work;
        assert!(call(name, minimum).is_ok(), "{name} minimum {minimum}");
        assert!(
            matches!(
                call(name, minimum - 1),
                Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
            ),
            "{name} minimum-1"
        );
    }
}

#[test]
fn nonempty_tree_rows_pay_fuel_above_the_interpreters_table_storage_work() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Fixed, Order};
    fn fixture() -> Snapshot {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let tree = sessions[0].statics().trees()[0];
        let command = sessions[0]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport { at: tree },
            }))
            .unwrap();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[command], &[]]).unwrap())
        {
            support::fold(session, stream);
        }
        Snapshot::capture(&sessions[0]).unwrap()
    }
    fn storage(rows: usize) -> u64 {
        let limits = Limits::DEFAULT;
        let mut lua = Lua::new(
            limits,
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        let function = lua
            .function(
                "result_table_baseline",
                Rc::new(move |vm, _| {
                    let table = vm.new_table()?;
                    for index in 0..rows {
                        vm.raw_set(
                            table,
                            Value::Number((index + 1) as f64),
                            Value::Number(index as f64),
                        )?;
                    }
                    vm.results(&[Value::Table(table)])
                }),
            )
            .unwrap();
        lua.begin_tick();
        lua.call(function, &[]).unwrap();
        limits.fuel_per_tick - lua.tick_fuel_left() - support::bare_native_call_fuel()
    }
    fn call(budget: u64) -> Result<Vec<Value>, luagee::Error> {
        let snapshot = fixture();
        let own = snapshot.players[0].unit.unwrap();
        let state = Rc::new(RefCell::new(State::new(
            Team::Radiant,
            &[crate::BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
        )));
        state.borrow_mut().snapshot = Some(snapshot);
        let mut lua = Lua::new(
            Limits::DEFAULT.with_fuel_per_tick(budget),
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )?;
        let vectors = vector::install(&mut lua).unwrap();
        install(&mut lua, state.clone(), vectors).unwrap();
        let receiver = state.borrow_mut().handle(lua.vm(), Some(own)).unwrap();
        let key = lua.string(b"GetNearbyTrees")?;
        let function = lua
            .vm()
            .raw_get(state.borrow().unit_methods.unwrap(), key)
            .unwrap();
        lua.begin_tick();
        lua.call(function, &[receiver, Value::Number(256.0)])
    }
    let snapshot = fixture();
    let own = snapshot.players[0].unit.unwrap();
    let position = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == own)
        .unwrap()
        .unit
        .pos;
    let rows = snapshot
        .statics
        .trees()
        .iter()
        .enumerate()
        .filter(|(index, tree)| {
            snapshot
                .felled_trees
                .binary_search(&(*index as u32))
                .is_err()
                && position.within(**tree, Fixed::from_int(256))
        })
        .count();
    assert!(rows > 0, "real forest fixture must return tree rows");
    let units = snapshot.units.len() as u64;
    let trees = snapshot.statics.trees().len() as u64;
    let search = 1 + snapshot.felled_trees.len().max(1).ilog2() as u64;
    let host = 1 + units / 8 + 8 + units + trees * (1 + search) + rows as u64;
    let minimum = support::bare_native_call_fuel() + storage(rows) + host;
    assert!(call(minimum).is_ok(), "exact populated minimum {minimum}");
    assert!(matches!(
        call(minimum - 1),
        Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
    ));
}

#[test]
fn forgotten_tree_receivers_raise_the_unknown_unit_error() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let own = sessions[0].own_hero().unwrap().id;
    let state = Rc::new(RefCell::new(State::new(
        Team::Radiant,
        &[crate::BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
    )));
    state.borrow_mut().snapshot = Some(Snapshot::capture(&sessions[0]).unwrap());
    let mut lua = Lua::new(
        Limits::DEFAULT,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .unwrap();
    let vectors = vector::install(&mut lua).unwrap();
    install(&mut lua, state.clone(), vectors).unwrap();
    let receiver = state.borrow_mut().handle(lua.vm(), Some(own)).unwrap();
    let key = lua.string(b"GetNearbyTrees").unwrap();
    let function = lua
        .vm()
        .raw_get(state.borrow().unit_methods.unwrap(), key)
        .unwrap();
    {
        let mut state = state.borrow_mut();
        let snapshot = state.snapshot.as_mut().unwrap();
        snapshot.units.clear();
        snapshot.players.clear();
    }
    lua.begin_tick();
    let error = lua
        .call(function, &[receiver, Value::Number(256.0)])
        .unwrap_err();
    assert!(error.to_string().contains("unit is no longer known"));
}

#[test]
fn courier_count_pins_scan_fuel_in_a_populated_server_world() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let position = sessions[0].own_hero().unwrap().pos;
    let command = sessions[0]
        .issue(bota_client_core::IssuedOrder::hero(Order::Cheat {
            cheat: bota_proto::Cheat::CreateCreeps {
                enemy: false,
                at: position,
            },
        }))
        .unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[command], &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    let snapshot = Snapshot::capture(&sessions[0]).unwrap();
    let count = snapshot.units.len() as u64;
    assert!(count > 2);
    for fuel in [
        support::bare_native_call_fuel() + 8 + count,
        support::bare_native_call_fuel() + 7 + count,
    ] {
        let state = Rc::new(RefCell::new(State::new(
            Team::Radiant,
            &[crate::BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
        )));
        state.borrow_mut().snapshot = Some(Snapshot::capture(&sessions[0]).unwrap());
        let mut lua = Lua::new(
            Limits::DEFAULT.with_fuel_per_tick(fuel),
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        let vectors = vector::install(&mut lua).unwrap();
        install(&mut lua, state, vectors).unwrap();
        let function = lua.global("GetNumCouriers").unwrap();
        lua.begin_tick();
        let result = lua.call(function, &[]);
        if fuel == support::bare_native_call_fuel() + 8 + count {
            assert_eq!(result.unwrap(), vec![Value::Number(1.0)]);
        } else {
            assert!(matches!(
                result,
                Err(luagee::Error::Limit(luagee::Limit::FuelPerTick))
            ));
        }
    }
}
#[test]
fn game_state_reports_init_selection_strategy_and_abort_from_host_facts() {
    let state = Rc::new(RefCell::new(State::new(
        Team::Radiant,
        &[crate::BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
    )));
    let mut lua = Lua::new(
        Limits::DEFAULT,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .unwrap();
    let vectors = vector::install(&mut lua).unwrap();
    install(&mut lua, state.clone(), vectors).unwrap();
    let function = lua.global("GetGameState").unwrap();
    let query = |lua: &mut Lua| {
        lua.begin_tick();
        lua.call(function, &[]).unwrap()
    };
    assert_eq!(query(&mut lua), vec![Value::Number(0.0)]);
    state.borrow_mut().selecting = true;
    assert_eq!(query(&mut lua), vec![Value::Number(2.0)]);
    state.borrow_mut().selecting = false;
    state.borrow_mut().selection_finished = true;
    assert_eq!(query(&mut lua), vec![Value::Number(3.0)]);
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    sessions[0]
        .ingest(
            bota_proto::ServerMsg::MatchAborted {
                reason: bota_proto::AbortReason::Stopped,
                stats: None,
            },
            &mut (),
        )
        .unwrap();
    state.borrow_mut().snapshot = Some(Snapshot::capture(&sessions[0]).unwrap());
    assert_eq!(query(&mut lua), vec![Value::Number(7.0)]);
}
