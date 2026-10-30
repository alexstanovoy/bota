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
            assert!(is_dota_key(space, entry.name), "{entry:?} is no Dota key");
            assert!(
                names.insert(entry.name),
                "{space:?} names {} twice",
                entry.name
            );
        }
    }
}

/// Whether a name has the form of Dota's internal names in its space: lower
/// case words joined by underscores, under the space's prefix. An effect Dota
/// has no modifier for is bota's own.
fn is_dota_key(space: IdSpace, name: &str) -> bool {
    let prefixes: &[&str] = match space {
        IdSpace::Hero => &["npc_dota_hero_"],
        IdSpace::Ability => &[""],
        IdSpace::Item => &["item_"],
        IdSpace::Effect => &["modifier_", "bota_"],
        IdSpace::Neutral => &["npc_dota_"],
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && prefixes.iter().any(|prefix| name.starts_with(prefix))
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

/// What two builds of different wires still read of each other: a client's
/// hello with its role, the server's refusal of another wire and a replay's
/// header. Postcard reads by position, so a field or a role appended to them
/// would make the build without it fail to read the build with it, and two
/// builds would part on a decoding error instead of naming both fingerprints.
#[test]
fn what_builds_of_other_wires_read_of_each_other_never_grows() {
    let frozen = [
        "variant ClientMsg 0 Hello {}",
        "field ClientMsg::Hello 0 role: Role",
        "field ClientMsg::Hello 1 name: String",
        "field ClientMsg::Hello 2 key: Option<SeatKey>",
        "field ClientMsg::Hello 3 wire_fingerprint: u64",
        "variant Role 0 Player",
        "variant Role 1 Bot",
        "variant Role 2 Spectator",
        "variant Role 3 Referee",
        "newtype SeatKey [u8; 32]",
        "variant ServerMsg 9 WireMismatch {}",
        "field ServerMsg::WireMismatch 0 server_fingerprint: u64",
        "variant ReplayRecord 2 Header(ReplayHeader)",
        "field ReplayHeader 0 magic: [u8; 8]",
        "field ReplayHeader 1 wire_fingerprint: u64",
    ];
    let owners = [
        "ClientMsg::Hello",
        "Role",
        "SeatKey",
        "ServerMsg::WireMismatch",
        "ReplayHeader",
    ];
    let live = layout_lines(&trace_wire());
    let owned: Vec<&str> = live
        .iter()
        .map(String::as_str)
        .filter(|line| line.split(' ').nth(1).is_some_and(|o| owners.contains(&o)))
        .collect();
    let grown: Vec<&str> = owned
        .iter()
        .copied()
        .filter(|line| !frozen.contains(line))
        .collect();
    assert!(
        grown.is_empty(),
        "the version handshake is frozen; say this in a message of its own instead:\n{}",
        grown.join("\n")
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
    trace::<Inflictor>(&mut tracer, &mut enums);
    trace::<ProjectileModel>(&mut tracer, &mut enums);
    trace::<Role>(&mut tracer, &mut enums);
    trace::<TickMode>(&mut tracer, &mut enums);
    trace::<PickPhase>(&mut tracer, &mut enums);
    trace::<Team>(&mut tracer, &mut enums);
    trace::<Aim>(&mut tracer, &mut enums);
    trace::<UnitKind>(&mut tracer, &mut enums);
    trace::<Attribute>(&mut tracer, &mut enums);
    trace::<Audience>(&mut tracer, &mut enums);
    trace::<Body>(&mut tracer, &mut enums);
    trace::<Speaker>(&mut tracer, &mut enums);
    trace::<SayRefusal>(&mut tracer, &mut enums);
    trace::<Codec>(&mut tracer, &mut enums);
    trace::<PingKind>(&mut tracer, &mut enums);
    trace::<PingAt>(&mut tracer, &mut enums);
    trace::<CreepSide>(&mut tracer, &mut enums);
    trace::<RuneKind>(&mut tracer, &mut enums);
    trace::<ObjectiveKind>(&mut tracer, &mut enums);
    trace::<Disassembly>(&mut tracer, &mut enums);
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

/// Every named id constant, with the key of its line in `registry/ids.txt`.
/// Only what code names has a constant: the tables name the rest.
const NAMED: &[(IdSpace, u16, &str)] = &[
    (
        IdSpace::Effect,
        EffectId::RUNE_HASTE.0,
        "modifier_rune_haste",
    ),
    (
        IdSpace::Effect,
        EffectId::RUNE_AMPLIFY_DAMAGE.0,
        "modifier_rune_doubledamage",
    ),
    (
        IdSpace::Effect,
        EffectId::RUNE_ARCANE.0,
        "modifier_rune_arcane",
    ),
    (
        IdSpace::Effect,
        EffectId::RUNE_REGENERATION.0,
        "modifier_rune_regen",
    ),
    (
        IdSpace::Effect,
        EffectId::RUNE_INVISIBILITY.0,
        "modifier_rune_invis",
    ),
    (
        IdSpace::Effect,
        EffectId::RUNE_SHIELD.0,
        "modifier_rune_shield",
    ),
    (
        IdSpace::Effect,
        EffectId::BOTTLE_REGENERATION.0,
        "modifier_bottle_regeneration",
    ),
    (IdSpace::Item, ItemId::BOTTLE.0, "item_bottle"),
    (IdSpace::Hero, HeroId::PUDGE.0, "npc_dota_hero_pudge"),
    (
        IdSpace::Hero,
        HeroId::SHADOW_FIEND.0,
        "npc_dota_hero_nevermore",
    ),
    (IdSpace::Ability, AbilityId::MEAT_HOOK.0, "pudge_meat_hook"),
    (IdSpace::Ability, AbilityId::ROT.0, "pudge_rot"),
    (
        IdSpace::Ability,
        AbilityId::FLESH_HEAP.0,
        "pudge_innate_graft_flesh",
    ),
    (IdSpace::Ability, AbilityId::DISMEMBER.0, "pudge_dismember"),
    (IdSpace::Ability, AbilityId::BURST.0, "courier_burst"),
    (
        IdSpace::Ability,
        AbilityId::RETURN_ITEMS.0,
        "courier_return_to_base",
    ),
    (
        IdSpace::Ability,
        AbilityId::TAKE_STASH.0,
        "courier_take_stash_items",
    ),
    (
        IdSpace::Ability,
        AbilityId::DELIVER.0,
        "courier_transfer_items",
    ),
    (IdSpace::Ability, AbilityId::SHIELD.0, "courier_shield"),
    (
        IdSpace::Ability,
        AbilityId::RAZE_NEAR.0,
        "nevermore_shadowraze1",
    ),
    (
        IdSpace::Ability,
        AbilityId::RAZE_MID.0,
        "nevermore_shadowraze2",
    ),
    (
        IdSpace::Ability,
        AbilityId::RAZE_FAR.0,
        "nevermore_shadowraze3",
    ),
    (IdSpace::Ability, AbilityId::REQUIEM.0, "nevermore_requiem"),
    (
        IdSpace::Ability,
        AbilityId::NECROMASTERY.0,
        "nevermore_necromastery",
    ),
    (
        IdSpace::Ability,
        AbilityId::PRESENCE.0,
        "nevermore_dark_lord",
    ),
    (
        IdSpace::Ability,
        AbilityId::FEAST_OF_SOULS.0,
        "nevermore_frenzy",
    ),
    (
        IdSpace::Ability,
        AbilityId::MEAT_SHIELD.0,
        "pudge_flesh_heap",
    ),
    (IdSpace::Ability, AbilityId::CAPTURE.0, "ability_capture"),
    (IdSpace::Ability, AbilityId::LAMP_USE.0, "ability_lamp_use"),
    (IdSpace::Item, ItemId::BOOTS.0, "item_boots"),
    (IdSpace::Item, ItemId::CLARITY.0, "item_clarity"),
    (IdSpace::Item, ItemId::HEALING_SALVE.0, "item_flask"),
    (IdSpace::Item, ItemId::IRON_BRANCH.0, "item_branches"),
    (IdSpace::Item, ItemId::OBSERVER_WARD.0, "item_ward_observer"),
    (
        IdSpace::Item,
        ItemId::QUELLING_BLADE.0,
        "item_quelling_blade",
    ),
    (IdSpace::Item, ItemId::SENTRY_WARD.0, "item_ward_sentry"),
    (IdSpace::Item, ItemId::TANGO.0, "item_tango"),
    (IdSpace::Item, ItemId::TOWN_PORTAL_SCROLL.0, "item_tpscroll"),
    (IdSpace::Item, ItemId::CIRCLET.0, "item_circlet"),
    (IdSpace::Item, ItemId::GAUNTLETS.0, "item_gauntlets"),
    (IdSpace::Item, ItemId::SLIPPERS.0, "item_slippers"),
    (IdSpace::Item, ItemId::MANTLE.0, "item_mantle"),
    (IdSpace::Item, ItemId::BELT.0, "item_belt_of_strength"),
    (IdSpace::Item, ItemId::BAND.0, "item_boots_of_elves"),
    (IdSpace::Item, ItemId::ROBE.0, "item_robe"),
    (IdSpace::Item, ItemId::OGRE_AXE.0, "item_ogre_axe"),
    (
        IdSpace::Item,
        ItemId::BLADE_OF_ALACRITY.0,
        "item_blade_of_alacrity",
    ),
    (
        IdSpace::Item,
        ItemId::STAFF_OF_WIZARDRY.0,
        "item_staff_of_wizardry",
    ),
    (IdSpace::Item, ItemId::GLOVES.0, "item_gloves"),
    (
        IdSpace::Item,
        ItemId::BLADES_OF_ATTACK.0,
        "item_blades_of_attack",
    ),
    (IdSpace::Item, ItemId::BROADSWORD.0, "item_broadsword"),
    (IdSpace::Item, ItemId::QUARTERSTAFF.0, "item_quarterstaff"),
    (
        IdSpace::Item,
        ItemId::RING_OF_PROTECTION.0,
        "item_ring_of_protection",
    ),
    (IdSpace::Item, ItemId::CHAINMAIL.0, "item_chainmail"),
    (IdSpace::Item, ItemId::RING_OF_REGEN.0, "item_ring_of_regen"),
    (IdSpace::Item, ItemId::SAGES_MASK.0, "item_sobi_mask"),
    (
        IdSpace::Item,
        ItemId::VITALITY_BOOSTER.0,
        "item_vitality_booster",
    ),
    (
        IdSpace::Item,
        ItemId::ENERGY_BOOSTER.0,
        "item_energy_booster",
    ),
    (IdSpace::Item, ItemId::POWER_TREADS.0, "item_power_treads"),
    (IdSpace::Item, ItemId::PHASE_BOOTS.0, "item_phase_boots"),
    (IdSpace::Item, ItemId::BLINK_DAGGER.0, "item_blink"),
    (IdSpace::Item, ItemId::BRACER.0, "item_bracer"),
    (IdSpace::Item, ItemId::WRAITH_BAND.0, "item_wraith_band"),
    (IdSpace::Item, ItemId::NULL_TALISMAN.0, "item_null_talisman"),
    (IdSpace::Item, ItemId::MAGIC_STICK.0, "item_magic_stick"),
    (IdSpace::Item, ItemId::MAGIC_WAND.0, "item_magic_wand"),
    (IdSpace::Item, ItemId::RECIPE_BRACER.0, "item_recipe_bracer"),
    (
        IdSpace::Item,
        ItemId::RECIPE_WRAITH_BAND.0,
        "item_recipe_wraith_band",
    ),
    (
        IdSpace::Item,
        ItemId::RECIPE_NULL_TALISMAN.0,
        "item_recipe_null_talisman",
    ),
    (
        IdSpace::Item,
        ItemId::RECIPE_MAGIC_WAND.0,
        "item_recipe_magic_wand",
    ),
    (IdSpace::Item, ItemId::MANGO.0, "item_enchanted_mango"),
    (IdSpace::Item, ItemId::EAGLESONG.0, "item_eagle"),
    (IdSpace::Item, ItemId::CLAYMORE.0, "item_claymore"),
    (
        IdSpace::Item,
        ItemId::TALISMAN_OF_EVASION.0,
        "item_talisman_of_evasion",
    ),
    (IdSpace::Item, ItemId::BUTTERFLY.0, "item_butterfly"),
    (IdSpace::Item, ItemId::JAVELIN.0, "item_javelin"),
    (IdSpace::Item, ItemId::DEMON_EDGE.0, "item_demon_edge"),
    (
        IdSpace::Item,
        ItemId::BLITZ_KNUCKLES.0,
        "item_blitz_knuckles",
    ),
    (
        IdSpace::Item,
        ItemId::RECIPE_MONKEY_KING_BAR.0,
        "item_recipe_monkey_king_bar",
    ),
    (
        IdSpace::Item,
        ItemId::MONKEY_KING_BAR.0,
        "item_monkey_king_bar",
    ),
    (IdSpace::Item, ItemId::WIND_LACE.0, "item_wind_lace"),
    (IdSpace::Item, ItemId::BLOOD_GRENADE.0, "item_blood_grenade"),
    (
        IdSpace::Item,
        ItemId::WARD_DISPENSER.0,
        "item_ward_dispenser",
    ),
    (IdSpace::Item, ItemId::TANGO_SHARED.0, "item_tango_single"),
    (
        IdSpace::Effect,
        EffectId::MENDING.0,
        "modifier_flask_healing",
    ),
    (
        IdSpace::Effect,
        EffectId::CLARITY.0,
        "modifier_clarity_potion",
    ),
    (
        IdSpace::Effect,
        EffectId::FOUNTAIN.0,
        "modifier_fountain_aura_buff",
    ),
    (IdSpace::Effect, EffectId::STUNNED.0, "modifier_stunned"),
    (IdSpace::Effect, EffectId::SLOWED.0, "bota_slowed"),
    (IdSpace::Effect, EffectId::BURNING.0, "bota_burning"),
    (
        IdSpace::Effect,
        EffectId::HASTENED.0,
        "modifier_courier_burst",
    ),
    (
        IdSpace::Effect,
        EffectId::SHIELDED.0,
        "modifier_courier_shield",
    ),
    (
        IdSpace::Effect,
        EffectId::PHASED.0,
        "modifier_item_phase_boots_active",
    ),
    (
        IdSpace::Effect,
        EffectId::FLESH_HEAP.0,
        "modifier_pudge_innate_graft_flesh",
    ),
    (
        IdSpace::Effect,
        EffectId::SOULS.0,
        "modifier_nevermore_necromastery",
    ),
    (
        IdSpace::Effect,
        EffectId::ARMOR_BROKEN.0,
        "modifier_nevermore_presence",
    ),
    (
        IdSpace::Effect,
        EffectId::GUARDED.0,
        "modifier_tower_armor_bonus",
    ),
    (
        IdSpace::Effect,
        EffectId::INSPIRED.0,
        "modifier_flagbearer_creep_aura_effect",
    ),
    (
        IdSpace::Effect,
        EffectId::SHADOWRAZE.0,
        "modifier_nevermore_shadowraze_debuff",
    ),
    (IdSpace::Effect, EffectId::ROT.0, "modifier_pudge_rot"),
    (
        IdSpace::Effect,
        EffectId::FEARED.0,
        "modifier_nevermore_requiem_fear",
    ),
    (
        IdSpace::Effect,
        EffectId::MAGIC_BROKEN.0,
        "modifier_nevermore_requiem_slow",
    ),
    (
        IdSpace::Effect,
        EffectId::FEASTING.0,
        "modifier_nevermore_frenzy",
    ),
    (
        IdSpace::Effect,
        EffectId::FEAST_LINGER.0,
        "modifier_nevermore_feast_of_souls_linger",
    ),
    (
        IdSpace::Effect,
        EffectId::BLOOD_GRENADE.0,
        "modifier_item_blood_grenade_debuff",
    ),
    (IdSpace::Effect, EffectId::ROOTED.0, "modifier_rooted"),
    (IdSpace::Effect, EffectId::DISARMED.0, "modifier_disarmed"),
    (IdSpace::Effect, EffectId::BROKEN.0, "modifier_break"),
    (
        IdSpace::Effect,
        EffectId::FORTIFIED.0,
        "modifier_fountain_glyph",
    ),
    (
        IdSpace::Effect,
        EffectId::MEAT_SHIELD.0,
        "modifier_pudge_flesh_heap_block",
    ),
    (
        IdSpace::Effect,
        EffectId::DISMEMBERED.0,
        "modifier_pudge_dismember",
    ),
    (
        IdSpace::Effect,
        EffectId::BACKDOOR_PROTECTION.0,
        "modifier_backdoor_protection_active",
    ),
    (IdSpace::Effect, EffectId::TIMED_LIFE.0, "modifier_kill"),
    (
        IdSpace::Effect,
        EffectId::NEUTRAL_UPGRADE.0,
        "modifier_neutral_upgrade",
    ),
    (
        IdSpace::Effect,
        EffectId::VISION_GRANT.0,
        "modifier_lamp_on",
    ),
    (IdSpace::Effect, EffectId::LAMP_OFF.0, "modifier_lamp_off"),
    (
        IdSpace::Effect,
        EffectId::CAPTURE_GUARD.0,
        "modifier_watch_tower_invulnerable",
    ),
    (
        IdSpace::Effect,
        EffectId::SHRINE_READY.0,
        "bota_shrine_ready",
    ),
];

/// Every named id constant is the registered id of the key the registry
/// gives it, so a constant cannot drift off its line in `registry/ids.txt`.
#[test]
fn every_named_id_constant_sits_on_its_registry_line() {
    for &(space, id, key) in NAMED {
        let entry = registered(space)
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("{space:?} {id} is not in registry/ids.txt"));
        assert_eq!(entry.name, key, "{space:?} {id} names another thing");
        assert!(!entry.retired, "{space:?} {id} ({key}) is retired");
    }
}
