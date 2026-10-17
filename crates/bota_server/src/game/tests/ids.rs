//! The ids the server hands out are the ones the protocol's registry holds.

use bota_core::{ABILITIES, EFFECTS, HEROES, ITEMS, ModifierKind};
use bota_proto::{DamageKind, EffectId, Fixed, IdSpace, RegisteredId, registered};

use crate::game::{StackKind, stack_effect_id};

/// Each id of `space` the server defines, by `names[id]`, is registered under
/// that name and live, and every live one is defined. A slot left in `names`
/// for a retired id is its placeholder.
fn defined_as_registered(space: IdSpace, names: &[Option<&str>]) {
    let registry: Vec<RegisteredId> = registered(space).collect();
    for (id, name) in names.iter().enumerate() {
        let entry = registry
            .iter()
            .find(|entry| usize::from(entry.id) == id)
            .unwrap_or_else(|| panic!("{space:?} {id} is not in registry/ids.txt"));
        if let Some(name) = name {
            assert_eq!(*name, entry.name, "{space:?} {id} changed meaning");
        }
    }
    for entry in registry.iter().filter(|entry| !entry.retired) {
        assert!(
            usize::from(entry.id) < names.len(),
            "{space:?} {} {} is live but the server lacks it",
            entry.id,
            entry.name
        );
    }
}

#[test]
fn the_tables_keep_their_registered_ids() {
    let heroes: Vec<_> = HEROES.iter().map(|hero| Some(hero.name)).collect();
    defined_as_registered(IdSpace::Hero, &heroes);
    let abilities: Vec<_> = ABILITIES.iter().map(|ability| Some(ability.name)).collect();
    defined_as_registered(IdSpace::Ability, &abilities);
    let items: Vec<_> = ITEMS.iter().map(|item| Some(item.name)).collect();
    defined_as_registered(IdSpace::Item, &items);
    let effects: Vec<_> = EFFECTS.iter().map(|effect| Some(effect.name)).collect();
    defined_as_registered(IdSpace::Effect, &effects);
}

/// One modifier of every kind. A kind added to the enum fails to compile in
/// the match below until it is listed here as well.
fn every_modifier() -> Vec<ModifierKind> {
    let every = vec![
        ModifierKind::Mending {
            per_tick: 0,
            breaks: false,
        },
        ModifierKind::Clarity {
            per_tick: 0,
            breaks: false,
        },
        ModifierKind::Fountain {
            hp_per_tick: 0,
            mana_per_tick: 0,
        },
        ModifierKind::Stunned,
        ModifierKind::Shielded,
        ModifierKind::Phased,
        ModifierKind::Slowed { pct: 0 },
        ModifierKind::Hastened { pct: 0 },
        ModifierKind::ArmorBroken { armor: Fixed::ZERO },
        ModifierKind::MagicBroken { pct: 0 },
        ModifierKind::Guarded {
            armor: 0,
            hp_per_second: 0,
        },
        ModifierKind::Inspired {
            hp_per_second: 0,
            magic_resist_pct: 0,
        },
        ModifierKind::Burning {
            amount: 0,
            kind: DamageKind::Pure,
            lethal: false,
        },
        ModifierKind::Shadowraze { stacks: 1 },
        ModifierKind::Rot { level: 0 },
        ModifierKind::Feared,
        ModifierKind::BloodGrenade { since: 0 },
    ];
    for kind in &every {
        match kind {
            ModifierKind::Mending { .. }
            | ModifierKind::Clarity { .. }
            | ModifierKind::Fountain { .. }
            | ModifierKind::Stunned
            | ModifierKind::Shielded
            | ModifierKind::Phased
            | ModifierKind::Slowed { .. }
            | ModifierKind::Hastened { .. }
            | ModifierKind::ArmorBroken { .. }
            | ModifierKind::MagicBroken { .. }
            | ModifierKind::Guarded { .. }
            | ModifierKind::Inspired { .. }
            | ModifierKind::Burning { .. }
            | ModifierKind::Shadowraze { .. }
            | ModifierKind::Rot { .. }
            | ModifierKind::Feared
            | ModifierKind::BloodGrenade { .. } => {}
        }
    }
    every
}

/// A value's variant name, as its `Debug` spells it.
fn variant_name(value: &impl std::fmt::Debug) -> String {
    format!("{value:?}")
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

#[test]
fn every_effect_is_shown_under_its_registered_id() {
    let mut shown: Vec<(u16, String)> = every_modifier()
        .iter()
        .map(|kind| (kind.effect_id().0, variant_name(kind)))
        .collect();
    for kind in StackKind::ALL {
        shown.push((stack_effect_id(kind).0, variant_name(&kind)));
    }
    shown.push((EffectId::FEASTING.0, "Feasting".to_string()));
    shown.push((EffectId::FEAST_LINGER.0, "FeastLinger".to_string()));
    let registry: Vec<RegisteredId> = registered(IdSpace::Effect).collect();
    for (id, name) in &shown {
        let entry = registry
            .iter()
            .find(|entry| entry.id == *id)
            .unwrap_or_else(|| panic!("effect {id} ({name}) is not in registry/ids.txt"));
        assert_eq!(entry.name, name, "effect {id} changed meaning");
        assert!(!entry.retired, "effect {id} ({name}) is retired");
    }
    for entry in registry.iter().filter(|entry| !entry.retired) {
        assert!(
            shown.iter().any(|(id, _)| *id == entry.id),
            "effect {} {} is live but nothing shows it",
            entry.id,
            entry.name
        );
    }
}
