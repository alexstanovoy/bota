use bota_proto::{AbilityId, AbilitySlot, EntityId, ItemId, ItemSlot, StatusFlags, TalentSlot};

use super::globals::{integer, string};
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Identity {
    Ability {
        unit: EntityId,
        slot: AbilitySlot,
        id: AbilityId,
    },
    Innate {
        unit: EntityId,
        id: AbilityId,
    },
    Talent {
        unit: EntityId,
        slot: TalentSlot,
    },
    Item {
        unit: EntityId,
        slot: ItemSlot,
        id: ItemId,
    },
}

impl Identity {
    pub(super) fn order(self, target: Target) -> Option<Order> {
        match self {
            Self::Ability { slot, .. } => Some(Order::Cast { slot, target }),
            Self::Item { slot, .. } => Some(Order::Use { slot, target }),
            Self::Innate { .. } | Self::Talent { .. } => None,
        }
    }

    pub(crate) fn owner(self) -> EntityId {
        match self {
            Self::Ability { unit, .. }
            | Self::Innate { unit, .. }
            | Self::Talent { unit, .. }
            | Self::Item { unit, .. } => unit,
        }
    }
}

struct View {
    key: &'static str,
    level: u8,
    range: i32,
    castable: bool,
    learnable: bool,
}

fn view(state: &State, identity: Identity) -> Option<View> {
    let unit = state.unit(identity.owner());
    let alive = state
        .player(identity.owner())
        .is_none_or(|player| player.respawn_left == 0)
        && unit.is_some_and(|unit| unit.hp > 0);
    let able = alive
        && unit.is_some_and(|unit| {
            unit.statuses.bits & (StatusFlags::STUNNED | StatusFlags::FEARED) == 0
        });
    match identity {
        Identity::Talent { unit, slot } => {
            let player = state.player(unit)?;
            let talent = state
                .snapshot
                .as_ref()?
                .talents
                .iter()
                .find(|talent| talent.slot == player.slot)?;
            let tree = bota_core::hero_def(player.hero)?.talents?;
            Some(View {
                key: tree.get(usize::from(slot.0))?.key(),
                level: u8::from(talent.learned.has(slot)),
                range: 0,
                castable: false,
                learnable: alive && talent.may_learn.is_some_and(|slots| slots.has(slot)),
            })
        }
        Identity::Innate { id, .. } => {
            let innate = state
                .hero(identity.owner())
                .and_then(bota_core::hero_def)?
                .innate?;
            if innate != id {
                return None;
            }
            let definition = bota_core::ability_def(id)?;
            Some(View {
                key: definition.key,
                level: 1,
                range: definition.range,
                castable: false,
                learnable: false,
            })
        }
        Identity::Ability { slot, id, .. } => {
            let ability = state
                .ability_book(identity.owner())?
                .get(usize::from(slot.0))
                .filter(|ability| ability.id == id)?;
            Some(View {
                key: bota_core::ability_def(id)?.key,
                level: ability.level,
                range: ability.range,
                castable: able
                    && ability.level > 0
                    && !ability.passive
                    && ability.cooldown_left == 0
                    && unit.is_some_and(|unit| unit.mana >= ability.mana_cost),
                learnable: alive && ability.learnable == Some(true),
            })
        }
        Identity::Item { slot, id, .. } => {
            let item = state
                .item(identity.owner(), slot)
                .filter(|item| item.id == id)?;
            Some(View {
                key: bota_core::item_def(id)?.key,
                level: item.level,
                range: item.range,
                castable: able
                    && (slot.0 < 6 || slot == ItemSlot::TOWN_PORTAL)
                    && item.aim.is_some()
                    && item.cooldown_left == 0
                    && item.mute_left == 0
                    && unit.is_some_and(|unit| unit.mana >= item.mana_cost),
                learnable: false,
            })
        }
    }
}

pub(crate) fn valid(state: &State, identity: Identity) -> bool {
    view(state, identity).is_some()
}

pub(super) fn identity(state: &State, value: Value) -> Option<Identity> {
    state
        .abilities
        .iter()
        .find(|(_, table)| value == Value::Table(*table))
        .map(|(identity, _)| *identity)
        .or_else(|| state.ability_identities.value(value))
}

fn handle(vm: &mut Vm, state: &mut State, identity: Option<Identity>) -> Result<Value, LuaError> {
    let Some(identity) = identity else {
        return Ok(Value::Nil);
    };
    vm.charge_fuel(1 + state.abilities.len() as u64 / 8)?;
    if let Some((_, table)) = state.abilities.iter().find(|(known, _)| *known == identity) {
        return Ok(Value::Table(*table));
    }
    if state.abilities.len() >= 8192 {
        return Err(vm.error("ability handle limit exceeded"));
    }
    if let Some(table) = state.ability_identities.table(vm, identity)? {
        vm.pin(Value::Table(table))?;
        state.abilities.push((identity, table));
        return Ok(Value::Table(table));
    }
    let table = vm.new_table()?;
    let meta = vm.new_table()?;
    let key = vm.new_string(b"__index")?;
    vm.raw_set(
        meta,
        key,
        state.ability_methods.map_or(Value::Nil, Value::Table),
    )?;
    let key = vm.new_string(b"__metatable")?;
    vm.raw_set(meta, key, Value::Boolean(false))?;
    state.ability_identities.remember(vm, table, meta, identity)?;
    vm.set_metatable(table, Some(meta))?;
    vm.pin(Value::Table(table))?;
    state.abilities.push((identity, table));
    Ok(Value::Table(table))
}

pub(super) fn install(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    let methods = lua.vm().new_table()?;
    lua.pin(Value::Table(methods))?;
    state.borrow_mut().ability_methods = Some(methods);
    for name in [
        "IsNull",
        "GetName",
        "GetLevel",
        "GetCastRange",
        "IsFullyCastable",
        "CanAbilityBeUpgraded",
        "GetSpecialValueInt",
        "GetAbilityDamage",
    ] {
        let capture = state.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| {
                let state = capture.borrow();
                vm.charge_fuel(
                    1 + state.abilities.len() as u64 / 8
                        + state
                            .snapshot
                            .as_ref()
                            .map_or(0, |snapshot| snapshot.units.len())
                            as u64
                            / 8,
                )?;
                let identity = identity(&state, vm.arg(args, 0))
                    .ok_or_else(|| vm.error("ability handle expected"))?;
                let view = view(&state, identity);
                let value = match name {
                    "GetSpecialValueInt" | "GetAbilityDamage" => {
                        let key = if name == "GetSpecialValueInt" {
                            string(vm, args, 1)?
                        } else {
                            b"damage"
                        };
                        let level = view.as_ref().map_or(0, |view| view.level);
                        let value = if name == "GetAbilityDamage" {
                            ability_damage(identity, level)
                        } else { special(identity, level, key) };
                        Value::Number(f64::from(value))
                    }
                    "IsNull" => Value::Boolean(view.is_none()),
                    "GetName" => {
                        vm.new_string(view.as_ref().map_or("", |view| view.key).as_bytes())?
                    }
                    "GetLevel" => {
                        Value::Number(view.as_ref().map_or(0.0, |view| f64::from(view.level)))
                    }
                    "GetCastRange" => {
                        Value::Number(view.as_ref().map_or(0.0, |view| f64::from(view.range)))
                    }
                    "IsFullyCastable" => Value::Boolean(view.is_some_and(|view| view.castable)),
                    "CanAbilityBeUpgraded" => {
                        Value::Boolean(view.is_some_and(|view| view.learnable))
                    }
                    _ => unreachable!("registered ability read"),
                };
                vm.results(&[value])
            }),
        )?;
    }
    install_lookup(lua, units, state.clone())?;
    install_immediate(lua, units, state)
}

fn install_lookup(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["GetAbilityByName", "GetItemInSlot"] {
        let capture = state.clone();
        method(
            lua,
            units,
            name,
            Rc::new(move |vm, args| {
                let mut state = capture.borrow_mut();
                let id = super::identity(vm, &state, args)?;
                let identity = if name == "GetAbilityByName" {
                    let key = string(vm, args, 1)?;
                    lookup_ability(&state, id, key)
                } else {
                    let index = integer(vm, args, 1)?;
                    u8::try_from(index).ok().and_then(|index| {
                        state.item(id, ItemSlot(index)).map(|item| Identity::Item {
                            unit: id,
                            slot: ItemSlot(index),
                            id: item.id,
                        })
                    })
                };
                let value = handle(vm, &mut state, identity)?;
                vm.results(&[value])
            }),
        )?;
    }
    Ok(())
}

fn lookup_ability(state: &State, id: EntityId, key: &[u8]) -> Option<Identity> {
    let book = state.ability_book(id).unwrap_or_default();
    let matches =
        |id| bota_core::ability_def(id).is_some_and(|definition| definition.key.as_bytes() == key);
    if let Some((slot, ability)) = book
        .iter()
        .enumerate()
        .find(|(_, ability)| matches(ability.id))
    {
        return Some(Identity::Ability {
            unit: id,
            slot: AbilitySlot(u8::try_from(slot).ok()?),
            id: ability.id,
        });
    }
    let hero = state.hero(id).and_then(bota_core::hero_def)?;
    if let Some(innate) = hero.innate.filter(|&innate| matches(innate)) {
        return Some(Identity::Innate {
            unit: id,
            id: innate,
        });
    }
    let player = state.player(id)?;
    if !state
        .snapshot
        .as_ref()?
        .talents
        .iter()
        .any(|talent| talent.slot == player.slot)
    {
        return None;
    }
    let slot = hero
        .talents?
        .iter()
        .position(|talent| talent.key().as_bytes() == key)?;
    Some(Identity::Talent {
        unit: id,
        slot: TalentSlot(u8::try_from(slot).ok()?),
    })
}

fn install_immediate(lua: &mut Lua, units: TableId, state: Shared) -> Result<(), BindingError> {
    for name in ["ActionImmediate_LevelAbility", "Action_LevelAbility"] {
        let capture = state.clone();
        method(
            lua,
            units,
            name,
            Rc::new(move |vm, args| {
                let mut state = capture.borrow_mut();
                let id = super::identity(vm, &state, args)?;
                if Some(id) != state.own_id() {
                    return Err(vm.error("action requires the active bot"));
                }
                let key = string(vm, args, 1)?;
                let order = match lookup_ability(&state, id, key) {
                    Some(Identity::Ability { slot, .. }) => Some(Order::Learn { slot }),
                    Some(Identity::Talent { slot, .. }) => {
                        Some(Order::LearnTalent { talent: slot })
                    }
                    _ => None,
                };
                if let Some(order) = order {
                    state.order(vm, bota_client_core::IssuedOrder::hero(order))?;
                }
                Ok(0)
            }),
        )?;
    }
    Ok(())
}

fn ability_damage(identity: Identity, level: u8) -> i32 {
    match identity {
        Identity::Ability { id, .. } | Identity::Innate { id, .. } => bota_core::ability_base_damage(id, level),
        Identity::Item { .. } => special(identity, level, b"damage"),
        Identity::Talent { .. } => 0,
    }
}

fn special(identity: Identity, level: u8, key: &[u8]) -> i32 {
    let Ok(key) = std::str::from_utf8(key) else { return 0 };
    let value = match identity {
        Identity::Talent { .. } => None,
        Identity::Item { id, .. } => bota_core::item_def(id).and_then(|item| item.value(key))
            .and_then(|values| values.get(usize::from(level.saturating_sub(1))).or_else(|| values.last())).copied(),
        Identity::Ability { id, .. } | Identity::Innate { id, .. } => bota_core::ability_value(id, level, key),
    };
    value.map_or(0, |value| value.0 / 1000)
}
