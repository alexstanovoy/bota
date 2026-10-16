//! The item keys of a template: price, uses and what an item carries.

use bota_core::{ITEMS, ItemDef, Pool, Ratio, UseSpec, item_def, rules};
use bota_proto::{Attributes, Fixed, ItemId};

use super::{fixed, per_second, percent, secs, secs_ms};

/// The value of a key read from an item's own entry.
pub(super) fn key(id: ItemId, key: &str) -> Option<String> {
    let def = item_def(id)?;
    let carried = &def.carried;
    Some(match key {
        "carried" => carried_long(def),
        "short" => carried_short(def),
        "builds" => builds(id),
        "range" => def.range.to_string(),
        "wait" => secs(def.cooldown),
        "point" => secs_ms(def.point),
        "health_cost" => def.health_cost.to_string(),
        "charges" => def.charges.to_string(),
        "stack_limit" => def.stack_limit.to_string(),
        "cast_charges" => def.cast_charges.to_string(),
        "break_secs" => secs(def.breaks_on_damage),
        "mode_bonus" => def.mode_bonus.to_string(),
        "evasion" => percent(carried.evasion),
        "pierce" => percent(carried.pierce),
        "pierce_damage" => carried.pierce_damage.to_string(),
        "hp_regen" => per_second(carried.hp_regen),
        "stock_initial" => def.stock?.initial.to_string(),
        "stock_max" => def.stock?.max.to_string(),
        "stock_minutes" => {
            let ticks = def.stock?.restock_ticks;
            (ticks / (60 * rules::TICKS_PER_SECOND)).to_string()
        }
        _ => return use_key(&def.effect, key),
    })
}

/// The keys of what a use of an item does.
fn use_key(effect: &UseSpec, key: &str) -> Option<String> {
    Some(match (effect, key) {
        (UseSpec::Mend(mend), "mend_total") => mend.total.to_string(),
        (UseSpec::Mend(mend), "mend_secs") => secs(mend.ticks),
        (UseSpec::Mend(mend), "mend_pool") => match mend.pool {
            Pool::Health => "health",
            Pool::Mana => "mana",
        }
        .to_string(),
        (UseSpec::Mend(mend), "mend_pool_short") => match mend.pool {
            Pool::Health => "HP",
            Pool::Mana => "MP",
        }
        .to_string(),
        (UseSpec::Ward { unit, .. }, "ward_vision") => unit.vision.to_string(),
        (UseSpec::Phase { pct, .. }, "phase_pct") => pct.to_string(),
        (UseSpec::Phase { ticks, .. }, "phase_secs") => secs(*ticks),
        (UseSpec::Blink { range }, "blink_range") => range.to_string(),
        (UseSpec::Restore { hp_per_charge, .. }, "restore") => hp_per_charge.to_string(),
        (UseSpec::Replenish { mana }, "replenish") => mana.to_string(),
        _ => return None,
    })
}

/// The attributes added, the biggest first, as `(long name, short name,
/// points)`; every attribute alike is one entry named "every attribute".
fn attribute_parts(has: Attributes) -> Vec<(&'static str, &'static str, i32)> {
    let [str_, agi, int] = [has.strength, has.agility, has.intelligence].map(Fixed::to_int);
    if str_ != 0 && str_ == agi && agi == int {
        return vec![("to every attribute", "all", str_)];
    }
    let mut parts = vec![
        ("strength", "STR", str_),
        ("agility", "AGI", agi),
        ("intelligence", "INT", int),
    ];
    parts.retain(|part| part.2 != 0);
    parts.sort_by_key(|part| -part.2);
    parts
}

/// What an item adds to its carrier, as a clause list: "+6 strength, +3
/// agility".
pub(crate) fn carried_long(def: &ItemDef) -> String {
    let c = &def.carried;
    let mut parts: Vec<String> = attribute_parts(c.attributes)
        .into_iter()
        .map(|(long, _, n)| format!("+{n} {long}"))
        .collect();
    let plain = [
        (c.move_speed, "movement speed"),
        (c.lace_speed, "movement speed"),
        (c.hp, "maximum health"),
        (c.mana, "maximum mana"),
        (c.damage, "attack damage"),
        (c.attack_speed, "attack speed"),
        (c.base_attack_speed_pct, "% base attack speed"),
        (c.damage_to_creeps, "attack damage against creeps"),
        (c.melee_range, "attack range in melee hands"),
    ];
    for (n, what) in plain.into_iter().filter(|(n, _)| *n != 0) {
        let gap = if what.starts_with('%') { "" } else { " " };
        parts.push(format!("+{n}{gap}{what}"));
    }
    if c.armor != Fixed::ZERO {
        parts.push(format!("+{} armor", fixed(c.armor)));
    }
    if c.hp_regen != Fixed::ZERO {
        parts.push(format!("+{} health a second", per_second(c.hp_regen)));
    }
    if c.mana_regen != Fixed::ZERO {
        parts.push(format!("+{} mana a second", per_second(c.mana_regen)));
    }
    parts.join(", ")
}

/// The same in a few characters, for a shop row: "+6 STR +3 AGI".
pub(crate) fn carried_short(def: &ItemDef) -> String {
    let c = &def.carried;
    let mut parts: Vec<String> = attribute_parts(c.attributes)
        .into_iter()
        .map(|(_, short, n)| format!("+{n} {short}"))
        .collect();
    let plain = [
        (c.move_speed + c.lace_speed, "+{} MS"),
        (c.hp, "+{} HP"),
        (c.mana, "+{} MP"),
        (c.damage, "+{} DMG"),
        (c.attack_speed, "+{} AS"),
        (c.base_attack_speed_pct, "+{}% base AS"),
        (c.damage_to_creeps, "+{} creep"),
        (c.melee_range, "+{} range"),
    ];
    for (n, shape) in plain.into_iter().filter(|(n, _)| *n != 0) {
        parts.push(shape.replace("{}", &n.to_string()));
    }
    for (rate, unit) in [(c.hp_regen, "HP/s"), (c.mana_regen, "MP/s")] {
        if rate != Fixed::ZERO {
            parts.push(format!("+{} {unit}", per_second(rate)));
        }
    }
    if c.armor != Fixed::ZERO {
        parts.push(format!("+{} ARM", fixed(c.armor)));
    }
    for (share, what) in [(c.evasion, "eva"), (c.pierce, "pierce")] {
        if share != Ratio::NEVER {
            parts.push(format!("{}% {what}", percent(share)));
        }
    }
    parts.join(" ")
}

/// What a recipe builds and from what, by the catalog's short names.
pub(crate) fn builds(recipe: ItemId) -> String {
    let name = |id: ItemId| crate::catalog::item(id.0).map_or("?", |face| face.name);
    let Some(parent) = ITEMS
        .iter()
        .position(|def| def.components.contains(&recipe))
    else {
        return String::new();
    };
    let mut parts: Vec<(ItemId, usize)> = Vec::new();
    for part in ITEMS[parent]
        .components
        .iter()
        .filter(|part| **part != recipe)
    {
        match parts.iter_mut().find(|(id, _)| id == part) {
            Some((_, count)) => *count += 1,
            None => parts.push((*part, 1)),
        }
    }
    let list: Vec<String> = parts
        .iter()
        .map(|(id, count)| match count {
            1 => name(*id).to_string(),
            n => format!("{n}x {}", name(*id)),
        })
        .collect();
    let (last, first) = list.split_last().expect("a recipe builds from parts");
    let from = if first.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", first.join(", "))
    };
    format!("Builds {} out of {from}.", name(ItemId(parent as u16)))
}
