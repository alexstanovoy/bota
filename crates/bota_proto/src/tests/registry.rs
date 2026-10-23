//! The wire is append-only: what the registries record never changes.

use std::collections::HashSet;

use serde::Deserialize;
use serde_reflection::{
    ContainerFormat, Format, Named, Registry, Tracer, TracerConfig, VariantFormat,
};

use crate::*;

/// The wire's layout as committed: one line a struct field, tuple element or
/// enum variant, with its position.
const WIRE: &str = include_str!("../../registry/wire.txt");

#[test]
fn every_id_space_is_numbered_densely_once() {
    for space in [
        IdSpace::Hero,
        IdSpace::Ability,
        IdSpace::Item,
        IdSpace::Effect,
        IdSpace::Neutral,
    ] {
        let entries: Vec<RegisteredId> = registered(space).collect();
        let mut ids: Vec<u16> = entries.iter().map(|entry| entry.id).collect();
        ids.sort_unstable();
        let dense: Vec<u16> = (0..).take(ids.len()).collect();
        assert_eq!(
            ids, dense,
            "{space:?} ids must run 0.. with none missing or doubled"
        );
        let mut names = HashSet::new();
        for entry in &entries {
            assert_eq!(entry.name, entry.name.trim(), "{entry:?}");
            assert!(
                names.insert(entry.name),
                "{space:?} names {} twice",
                entry.name
            );
        }
    }
}

#[test]
fn the_wire_layout_only_grows() {
    let live = layout_lines(&trace_wire());
    let live_set: HashSet<&str> = live.iter().map(String::as_str).collect();
    let committed: Vec<&str> = WIRE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let committed_set: HashSet<&str> = committed.iter().copied().collect();
    assert_eq!(
        committed_set.len(),
        committed.len(),
        "registry/wire.txt repeats a line"
    );
    let changed: Vec<&str> = committed
        .iter()
        .copied()
        .filter(|line| !live_set.contains(line))
        .collect();
    assert!(
        changed.is_empty(),
        "these wire positions changed or disappeared; keep them and retire them instead:\n{}",
        changed.join("\n")
    );
    let unregistered: Vec<&String> = live
        .iter()
        .filter(|line| !committed_set.contains(line.as_str()))
        .collect();
    assert!(
        unregistered.is_empty(),
        "append these to crates/bota_proto/registry/wire.txt:\n{}",
        unregistered
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Every type reachable from the things that cross the wire, the report a
/// launcher reads among them, each enum traced on its own so that no variant
/// goes unseen.
fn trace_wire() -> Registry {
    let mut tracer = Tracer::new(TracerConfig::default());
    let mut enums = HashSet::new();
    trace::<ClientMsg>(&mut tracer, &mut enums);
    trace::<ServerMsg>(&mut tracer, &mut enums);
    trace::<ReplayRecord>(&mut tracer, &mut enums);
    trace::<MatchReport>(&mut tracer, &mut enums);
    trace::<Order>(&mut tracer, &mut enums);
    trace::<Target>(&mut tracer, &mut enums);
    trace::<Cheat>(&mut tracer, &mut enums);
    trace::<EventKind>(&mut tracer, &mut enums);
    trace::<RejectReason>(&mut tracer, &mut enums);
    trace::<DamageKind>(&mut tracer, &mut enums);
    trace::<Role>(&mut tracer, &mut enums);
    trace::<TickMode>(&mut tracer, &mut enums);
    trace::<PickPhase>(&mut tracer, &mut enums);
    trace::<Team>(&mut tracer, &mut enums);
    trace::<Aim>(&mut tracer, &mut enums);
    trace::<UnitKind>(&mut tracer, &mut enums);
    trace::<Attribute>(&mut tracer, &mut enums);
    let registry = tracer.registry().expect("every wire type traces whole");
    for (name, container) in &registry {
        if matches!(container, ContainerFormat::Enum(_)) {
            assert!(
                enums.contains(name),
                "trace enum {name} on its own in trace_wire"
            );
        }
    }
    registry
}

fn trace<'de, T: Deserialize<'de>>(tracer: &mut Tracer, enums: &mut HashSet<String>) {
    let (format, _) = tracer.trace_simple_type::<T>().expect("trace");
    if let Format::TypeName(name) = format {
        enums.insert(name);
    }
}

/// The registry as lines, each naming one position on the wire.
fn layout_lines(registry: &Registry) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, container) in registry {
        match container {
            ContainerFormat::UnitStruct => {
                lines.push(format!("unit {name}"));
            }
            ContainerFormat::NewTypeStruct(inner) => {
                lines.push(format!("newtype {name} {}", render(inner)));
            }
            ContainerFormat::TupleStruct(items) => tuple_lines(&mut lines, name, items),
            ContainerFormat::Struct(fields) => field_lines(&mut lines, name, fields),
            ContainerFormat::Enum(variants) => {
                for (index, variant) in variants {
                    variant_lines(&mut lines, name, *index, variant);
                }
            }
        }
    }
    lines
}

fn variant_lines(lines: &mut Vec<String>, owner: &str, index: u32, variant: &Named<VariantFormat>) {
    let head = format!("variant {owner} {index} {}", variant.name);
    match &variant.value {
        VariantFormat::Unit => lines.push(head),
        VariantFormat::NewType(inner) => lines.push(format!("{head}({})", render(inner))),
        VariantFormat::Tuple(items) => {
            lines.push(format!("{head}()"));
            tuple_lines(lines, &format!("{owner}::{}", variant.name), items);
        }
        VariantFormat::Struct(fields) => {
            lines.push(format!("{head} {{}}"));
            field_lines(lines, &format!("{owner}::{}", variant.name), fields);
        }
        VariantFormat::Variable(_) => panic!("{owner}::{} traced incompletely", variant.name),
    }
}

fn tuple_lines(lines: &mut Vec<String>, owner: &str, items: &[Format]) {
    for (index, item) in items.iter().enumerate() {
        lines.push(format!("element {owner} {index} {}", render(item)));
    }
}

fn field_lines(lines: &mut Vec<String>, owner: &str, fields: &[Named<Format>]) {
    for (index, field) in fields.iter().enumerate() {
        lines.push(format!(
            "field {owner} {index} {}: {}",
            field.name,
            render(&field.value)
        ));
    }
}

/// A field's type as Rust spells it, primitives in lower case.
fn render(format: &Format) -> String {
    match format {
        Format::TypeName(name) => name.clone(),
        Format::Unit => "()".to_string(),
        Format::Bool => "bool".to_string(),
        Format::I8 => "i8".to_string(),
        Format::I16 => "i16".to_string(),
        Format::I32 => "i32".to_string(),
        Format::I64 => "i64".to_string(),
        Format::I128 => "i128".to_string(),
        Format::U8 => "u8".to_string(),
        Format::U16 => "u16".to_string(),
        Format::U32 => "u32".to_string(),
        Format::U64 => "u64".to_string(),
        Format::U128 => "u128".to_string(),
        Format::F32 => "f32".to_string(),
        Format::F64 => "f64".to_string(),
        Format::Char => "char".to_string(),
        Format::Str => "String".to_string(),
        Format::Bytes => "bytes".to_string(),
        Format::Option(inner) => format!("Option<{}>", render(inner)),
        Format::Seq(inner) => format!("Vec<{}>", render(inner)),
        Format::Map { key, value } => format!("Map<{}, {}>", render(key), render(value)),
        Format::Tuple(items) => {
            let items: Vec<String> = items.iter().map(render).collect();
            format!("({})", items.join(", "))
        }
        Format::TupleArray { content, size } => format!("[{}; {size}]", render(content)),
        Format::Variable(_) => panic!("a type traced incompletely"),
    }
}

/// Every named id constant is the registered id of the name the registry
/// gives it, so a constant cannot drift off its line in `registry/ids.txt`.
#[test]
fn every_named_id_constant_sits_on_its_registry_line() {
    let named: [(IdSpace, u16, &str); 101] = [
        (IdSpace::Hero, HeroId::PUDGE.0, "Pudge"),
        (IdSpace::Hero, HeroId::SHADOW_FIEND.0, "Shadow Fiend"),
        (IdSpace::Ability, AbilityId::MEAT_HOOK.0, "Meat Hook"),
        (IdSpace::Ability, AbilityId::ROT.0, "Rot"),
        (IdSpace::Ability, AbilityId::FLESH_HEAP.0, "Flesh Heap"),
        (IdSpace::Ability, AbilityId::DISMEMBER.0, "Dismember"),
        (IdSpace::Ability, AbilityId::BURST.0, "Burst"),
        (IdSpace::Ability, AbilityId::RETURN_ITEMS.0, "Return"),
        (IdSpace::Ability, AbilityId::TAKE_STASH.0, "Take Stash"),
        (IdSpace::Ability, AbilityId::DELIVER.0, "Deliver"),
        (IdSpace::Ability, AbilityId::SHIELD.0, "Shield"),
        (
            IdSpace::Ability,
            AbilityId::RAZE_NEAR.0,
            "Shadowraze (Near)",
        ),
        (IdSpace::Ability, AbilityId::RAZE_MID.0, "Shadowraze (Mid)"),
        (IdSpace::Ability, AbilityId::RAZE_FAR.0, "Shadowraze (Far)"),
        (IdSpace::Ability, AbilityId::REQUIEM.0, "Requiem of Souls"),
        (IdSpace::Ability, AbilityId::NECROMASTERY.0, "Necromastery"),
        (
            IdSpace::Ability,
            AbilityId::PRESENCE.0,
            "Presence of the Dark Lord",
        ),
        (
            IdSpace::Ability,
            AbilityId::FEAST_OF_SOULS.0,
            "Feast of Souls",
        ),
        (
            IdSpace::Ability,
            AbilityId::THROW_GRENADE.0,
            "Throw Grenade",
        ),
        (IdSpace::Ability, AbilityId::MEAT_SHIELD.0, "Meat Shield"),
        (IdSpace::Item, ItemId::BOOTS.0, "Boots of Speed"),
        (IdSpace::Item, ItemId::CLARITY.0, "Clarity"),
        (IdSpace::Item, ItemId::HEALING_SALVE.0, "Healing Salve"),
        (IdSpace::Item, ItemId::IRON_BRANCH.0, "Iron Branch"),
        (IdSpace::Item, ItemId::OBSERVER_WARD.0, "Observer Ward"),
        (IdSpace::Item, ItemId::QUELLING_BLADE.0, "Quelling Blade"),
        (IdSpace::Item, ItemId::SENTRY_WARD.0, "Sentry Ward"),
        (IdSpace::Item, ItemId::TANGO.0, "Tango"),
        (
            IdSpace::Item,
            ItemId::TOWN_PORTAL_SCROLL.0,
            "Town Portal Scroll",
        ),
        (IdSpace::Item, ItemId::CIRCLET.0, "Circlet"),
        (IdSpace::Item, ItemId::GAUNTLETS.0, "Gauntlets of Strength"),
        (IdSpace::Item, ItemId::SLIPPERS.0, "Slippers of Agility"),
        (IdSpace::Item, ItemId::MANTLE.0, "Mantle of Intelligence"),
        (IdSpace::Item, ItemId::BELT.0, "Belt of Strength"),
        (IdSpace::Item, ItemId::BAND.0, "Band of Elvenskin"),
        (IdSpace::Item, ItemId::ROBE.0, "Robe of the Magi"),
        (IdSpace::Item, ItemId::OGRE_AXE.0, "Ogre Axe"),
        (
            IdSpace::Item,
            ItemId::BLADE_OF_ALACRITY.0,
            "Blade of Alacrity",
        ),
        (
            IdSpace::Item,
            ItemId::STAFF_OF_WIZARDRY.0,
            "Staff of Wizardry",
        ),
        (IdSpace::Item, ItemId::GLOVES.0, "Gloves of Haste"),
        (
            IdSpace::Item,
            ItemId::BLADES_OF_ATTACK.0,
            "Blades of Attack",
        ),
        (IdSpace::Item, ItemId::BROADSWORD.0, "Broadsword"),
        (IdSpace::Item, ItemId::QUARTERSTAFF.0, "Quarterstaff"),
        (
            IdSpace::Item,
            ItemId::RING_OF_PROTECTION.0,
            "Ring of Protection",
        ),
        (IdSpace::Item, ItemId::CHAINMAIL.0, "Chainmail"),
        (IdSpace::Item, ItemId::RING_OF_REGEN.0, "Ring of Regen"),
        (IdSpace::Item, ItemId::SAGES_MASK.0, "Sage's Mask"),
        (
            IdSpace::Item,
            ItemId::VITALITY_BOOSTER.0,
            "Vitality Booster",
        ),
        (IdSpace::Item, ItemId::ENERGY_BOOSTER.0, "Energy Booster"),
        (IdSpace::Item, ItemId::POWER_TREADS.0, "Power Treads"),
        (IdSpace::Item, ItemId::PHASE_BOOTS.0, "Phase Boots"),
        (IdSpace::Item, ItemId::BLINK_DAGGER.0, "Blink Dagger"),
        (IdSpace::Item, ItemId::BRACER.0, "Bracer"),
        (IdSpace::Item, ItemId::WRAITH_BAND.0, "Wraith Band"),
        (IdSpace::Item, ItemId::NULL_TALISMAN.0, "Null Talisman"),
        (IdSpace::Item, ItemId::MAGIC_STICK.0, "Magic Stick"),
        (IdSpace::Item, ItemId::MAGIC_WAND.0, "Magic Wand"),
        (IdSpace::Item, ItemId::RECIPE_BRACER.0, "Recipe: Bracer"),
        (
            IdSpace::Item,
            ItemId::RECIPE_WRAITH_BAND.0,
            "Recipe: Wraith Band",
        ),
        (
            IdSpace::Item,
            ItemId::RECIPE_NULL_TALISMAN.0,
            "Recipe: Null Talisman",
        ),
        (
            IdSpace::Item,
            ItemId::RECIPE_MAGIC_WAND.0,
            "Recipe: Magic Wand",
        ),
        (IdSpace::Item, ItemId::MANGO.0, "Enchanted Mango"),
        (IdSpace::Item, ItemId::EAGLESONG.0, "Eaglesong"),
        (IdSpace::Item, ItemId::CLAYMORE.0, "Claymore"),
        (
            IdSpace::Item,
            ItemId::TALISMAN_OF_EVASION.0,
            "Talisman of Evasion",
        ),
        (IdSpace::Item, ItemId::BUTTERFLY.0, "Butterfly"),
        (IdSpace::Item, ItemId::JAVELIN.0, "Javelin"),
        (IdSpace::Item, ItemId::DEMON_EDGE.0, "Demon Edge"),
        (IdSpace::Item, ItemId::BLITZ_KNUCKLES.0, "Blitz Knuckles"),
        (
            IdSpace::Item,
            ItemId::RECIPE_MONKEY_KING_BAR.0,
            "Recipe: Monkey King Bar",
        ),
        (IdSpace::Item, ItemId::MONKEY_KING_BAR.0, "Monkey King Bar"),
        (IdSpace::Item, ItemId::WIND_LACE.0, "Wind Lace"),
        (IdSpace::Item, ItemId::BLOOD_GRENADE.0, "Blood Grenade"),
        (IdSpace::Effect, EffectId::MENDING.0, "Mending"),
        (IdSpace::Effect, EffectId::CLARITY.0, "Clarity"),
        (IdSpace::Effect, EffectId::FOUNTAIN.0, "Fountain"),
        (IdSpace::Effect, EffectId::STUNNED.0, "Stunned"),
        (IdSpace::Effect, EffectId::SLOWED.0, "Slowed"),
        (IdSpace::Effect, EffectId::BURNING.0, "Burning"),
        (IdSpace::Effect, EffectId::HASTENED.0, "Hastened"),
        (IdSpace::Effect, EffectId::SHIELDED.0, "Shielded"),
        (IdSpace::Effect, EffectId::PHASED.0, "Phased"),
        (IdSpace::Effect, EffectId::FLESH_HEAP.0, "FleshHeap"),
        (IdSpace::Effect, EffectId::SOULS.0, "Souls"),
        (IdSpace::Effect, EffectId::ARMOR_BROKEN.0, "ArmorBroken"),
        (IdSpace::Effect, EffectId::GUARDED.0, "Guarded"),
        (IdSpace::Effect, EffectId::INSPIRED.0, "Inspired"),
        (IdSpace::Effect, EffectId::SHADOWRAZE.0, "Shadowraze"),
        (IdSpace::Effect, EffectId::ROT.0, "Rot"),
        (IdSpace::Effect, EffectId::FEARED.0, "Feared"),
        (IdSpace::Effect, EffectId::MAGIC_BROKEN.0, "MagicBroken"),
        (IdSpace::Effect, EffectId::FEASTING.0, "Feasting"),
        (IdSpace::Effect, EffectId::FEAST_LINGER.0, "FeastLinger"),
        (IdSpace::Effect, EffectId::BLOOD_GRENADE.0, "BloodGrenade"),
        (IdSpace::Effect, EffectId::ROOTED.0, "Rooted"),
        (IdSpace::Effect, EffectId::DISARMED.0, "Disarmed"),
        (IdSpace::Effect, EffectId::BROKEN.0, "Broken"),
        (IdSpace::Effect, EffectId::FORTIFIED.0, "Fortified"),
        (IdSpace::Effect, EffectId::MEAT_SHIELD.0, "MeatShield"),
        (IdSpace::Effect, EffectId::DISMEMBERED.0, "Dismembered"),
        (
            IdSpace::Effect,
            EffectId::BACKDOOR_PROTECTION.0,
            "BackdoorProtection",
        ),
    ];
    for (space, id, name) in named {
        let entry = registered(space)
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("{space:?} {id} is not in registry/ids.txt"));
        assert_eq!(entry.name, name, "{space:?} {id} names another thing");
        assert!(!entry.retired, "{space:?} {id} ({name}) is retired");
    }
    // The neutrals, their abilities and the effects their spells leave are
    // named by the tables that define them, not by constants here: the
    // server's registry test holds those tables to the registry.
    for space in [IdSpace::Hero, IdSpace::Item] {
        for entry in registered(space).filter(|entry| !entry.retired) {
            assert!(
                named
                    .iter()
                    .any(|(s, id, _)| *s == space && *id == entry.id),
                "{space:?} {} {} has no named constant",
                entry.id,
                entry.name
            );
        }
    }
}
