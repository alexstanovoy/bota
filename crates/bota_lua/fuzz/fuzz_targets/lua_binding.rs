//! Lua source and filtered game-state variations through the public binding API.
#![no_main]
use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ModuleBank, ScriptKind};
use bota_proto::{EventKind, HeroId, ItemId, Order, ServerMsg, SlotId, Team};
use libfuzzer_sys::fuzz_target;
use std::sync::{Mutex, OnceLock};
#[path = "../../tests/support/mod.rs"]
mod support;

fn fresh() -> ClientSession {
    static INITIAL: OnceLock<Mutex<ClientSession>> = OnceLock::new();
    INITIAL
        .get_or_init(|| {
            let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
            let buy = sessions[0]
                .issue(IssuedOrder::hero(Order::Buy {
                    item: ItemId::HEALING_SALVE,
                }))
                .expect("fixture purchase");
            let streams = local.step(&[&[buy], &[]]).expect("fixture step");
            for (session, stream) in sessions.iter_mut().zip(streams) {
                support::fold(session, stream);
            }
            assert!(
                sessions[0]
                    .own_hero()
                    .expect("hero")
                    .items
                    .iter()
                    .flatten()
                    .any(|item| item.id == ItemId::HEALING_SALVE),
                "fixture owns an active item handle"
            );
            Mutex::new(sessions.remove(0))
        })
        .lock()
        .expect("one fuzzing thread")
        .clone()
}
fuzz_target!(|data: &[u8]| {
    if data.len() < 32 || data.len() > 8192 {
        return;
    }
    let limits = Limits {
        fuel_per_call: 20_000,
        fuel_per_tick: 80_000,
        memory: 32 << 20,
        string_length: 4096,
        module_size: 8192,
        scripts_size: 32768,
        module_count: 64,
        syntax_depth: 32,
        pattern_depth: 32,
        call_depth: 256,
    };
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
    .expect("bounded team");
    let mut modules = ModuleBank::new(limits);
    modules
        .insert("bots/module", b"return {number=17}")
        .expect("fixture module");
    lua.set_modules(modules).expect("module budget");
    let source = if data[0] & 1 == 0 {
        data[32..].to_vec()
    } else {
        structured(data)
    };
    let kind = if data[31] & 128 == 0 {
        ScriptKind::Takeover
    } else {
        ScriptKind::Mode(1)
    };
    if lua.add_script(SlotId(0), kind, "fuzz", &source).is_err() {
        return;
    }
    lua.set_gc_stress(match data[1] % 3 {
        0 => GcStress::Off,
        1 => GcStress::Full,
        _ => GcStress::Incremental,
    });
    let mut session = fresh();
    for frame in 0..2u32 {
        let mut view = session.current().expect("fixture view").clone();
        view.tick += if frame > 0 && data[29] & 1 != 0 {
            20_000
        } else {
            1
        };
        view.units
            .retain(|unit| unit.kind != bota_proto::UnitKind::CreepMelee);
        if let Some(actor) = session.own_hero() {
            for index in 0..u32::from(data[30] % 41) {
                let mut creep = actor.clone();
                creep.id = bota_proto::EntityId {
                    idx: 100_000 + frame * 64 + index,
                    generation: 0,
                };
                creep.kind = bota_proto::UnitKind::CreepMelee;
                creep.team = Team::Dire;
                creep.hero = None;
                creep.owner = None;
                creep.abilities.clear();
                creep.items.clear();
                creep.town_portal = None;
                creep.effects.clear();
                creep.queue.clear();
                creep.pos.x.raw += (index as i32 % 7) * 40 * 65536;
                creep.pos.y.raw += (index as i32 / 7) * 40 * 65536;
                view.units.push(creep);
            }
        }
        view.units.sort_by_key(|unit| unit.id);
        for (index, unit) in view.units.iter_mut().enumerate() {
            let byte = data[2 + index % 28];
            unit.hp = 1 + i32::from(byte) * 3;
            unit.max_hp = unit.max_hp.max(unit.hp);
            unit.mana = i32::from(byte);
            unit.max_mana = unit.max_mana.max(unit.mana);
            if data[31] & 64 != 0 {
                unit.armor = bota_proto::Fixed::MIN;
                unit.attack_damage = i32::MAX;
                unit.attack_damage_min = i32::MAX;
                unit.attack_damage_max = i32::MAX;
                unit.max_hp = 40_000;
                unit.statuses.bits &= !bota_proto::StatusFlags::INVULNERABLE;
            }
            for ability in &mut unit.abilities {
                ability.cooldown_left = u32::from(byte % 3);
            }
        }
        let tick = view.tick;
        let events = if data[31] & 2 != 0 {
            view.units
                .iter()
                .find(|unit| unit.hero.is_some())
                .map(|unit| EventKind::AttackLaunched {
                    attacker: unit.id,
                    target: if data[31] & 4 != 0 {
                        view.units.last().map(|unit| unit.id)
                    } else {
                        None
                    },
                })
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        if session
            .ingest(ServerMsg::Snapshot { view }, &mut ())
            .is_err()
        {
            return;
        }
        if session
            .ingest(ServerMsg::Events { tick, events }, &mut ())
            .is_err()
        {
            return;
        }
        if let Ok(turn) = lua.think(&session) {
            assert_eq!(turn.bots.len(), 1);
            assert!(turn.bots[0].orders.len() <= 64);
        }
    }
});
fn structured(data: &[u8]) -> Vec<u8> {
    let values = [
        "nil",
        "false",
        "true",
        "0",
        "-1",
        "1e308",
        "0/0",
        "math.huge",
        "'x'",
        "{}",
        "bot",
        "Vector(0,0,0)",
        "ability",
        "item",
    ];
    let methods = [
        "GetHealth",
        "IsNull",
        "GetItemInSlot",
        "GetNearbyHeroes",
        "GetNearbyCreeps",
        "GetNearbyTowers",
        "FindAoELocation",
        "SetTarget",
        "SetNextItemPurchaseValue",
        "Action_MoveToLocation",
        "Action_AttackUnit",
        "Action_UseAbility",
        "Action_UseAbilityOnEntity",
        "ActionImmediate_PurchaseItem",
        "GetEstimatedDamageToTarget",
        "GetMaxHealth",
        "IsAlive",
        "GetLevel",
        "GetUnitName",
        "GetGold",
        "GetRespawnTime",
        "GetLocation",
        "DistanceFromFountain",
        "GetActiveMode",
        "GetActiveModeDesire",
        "GetTarget",
        "CanBeSeen",
        "IsHero",
        "IsInvulnerable",
        "IsMagicImmune",
        "HasScepter",
        "WasRecentlyDamagedByHero",
        "Action_ClearActions",
        "ActionImmediate_LevelAbility",
        "Action_AttackMove",
    ];
    let value = |index: usize| values[usize::from(data[index]) % values.len()];
    let method = methods[usize::from(data[3]) % methods.len()];
    let ability_methods = [
        "GetName",
        "GetLevel",
        "GetCastRange",
        "GetAbilityDamage",
        "GetSpecialValueInt",
        "IsFullyCastable",
        "CanAbilityBeUpgraded",
    ];
    let ability_method = ability_methods[usize::from(data[7]) % ability_methods.len()];
    format!("function Think() local bot=GetBot() local ability=bot:GetAbilityByName('nevermore_shadowraze1') local item=bot:GetItemInSlot(0) local a,b,c={},{},{} pcall(function() bot:{}(a,b,c,a,b,c,a) end) pcall(function() ability:{}(a,b,c) end) pcall(function() item:{}(a,b,c) end) pcall(function() return Vector(1,2,3)*a end) collectgarbage() assert(require('bots/module').number==17) end", value(4), value(5), value(6), method, ability_method, ability_method).into_bytes()
}
