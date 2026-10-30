//! Every effect the server hands out is shown under its registered id.

use bota_core::{ModifierKind, NEUTRAL_ABILITIES, effect_def, spell_effect};
use bota_proto::{DamageKind, EffectId, Fixed, IdSpace, RegisteredId, registered};

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
            hp_pct: 0,
            mana_pct: 0,
        },
        ModifierKind::Stunned,
        ModifierKind::Shielded,
        ModifierKind::Phased,
        ModifierKind::Flying,
        ModifierKind::FountainInvulnerable,
        ModifierKind::Slowed { pct: 0 },
        ModifierKind::Hastened { pct: 0 },
        ModifierKind::Haste { pct: 0 },
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
        ModifierKind::Rotting { level: 0 },
        ModifierKind::Feared,
        ModifierKind::BloodGrenade { since: 0 },
        ModifierKind::Rooted,
        ModifierKind::Disarmed,
        ModifierKind::Broken,
        ModifierKind::Dismembered { level: 0 },
        ModifierKind::MeatShield { amount: 8 },
        ModifierKind::BackdoorProtection,
        ModifierKind::Fortified,
        ModifierKind::TimedLife,
        ModifierKind::Illusion,
        ModifierKind::NeutralUpgrade { count: 1 },
        ModifierKind::FleshHeap { count: 1 },
        ModifierKind::Souls { count: 1 },
        ModifierKind::Feasting { level: 0, souls: 0 },
        ModifierKind::FeastLinger { souls: 1 },
        ModifierKind::FountainPassive,
        ModifierKind::FountainDamage { stacks: 1 },
        ModifierKind::VisionGrant {
            side: bota_proto::Team::Radiant,
            day: 0,
            night: 0,
        },
        ModifierKind::LampOff,
        ModifierKind::CaptureGuard,
        ModifierKind::ShrineReady,
    ];
    for kind in &every {
        match kind {
            ModifierKind::Mending { .. }
            | ModifierKind::Clarity { .. }
            | ModifierKind::Fountain { .. }
            | ModifierKind::Stunned
            | ModifierKind::Shielded
            | ModifierKind::Phased
            | ModifierKind::Flying
            | ModifierKind::FountainInvulnerable
            | ModifierKind::FountainPassive
            | ModifierKind::FountainDamage { .. }
            | ModifierKind::Slowed { .. }
            | ModifierKind::Hastened { .. }
            | ModifierKind::Haste { .. }
            | ModifierKind::ArmorBroken { .. }
            | ModifierKind::MagicBroken { .. }
            | ModifierKind::Guarded { .. }
            | ModifierKind::Inspired { .. }
            | ModifierKind::Burning { .. }
            | ModifierKind::Shadowraze { .. }
            | ModifierKind::Rot { .. }
            | ModifierKind::Rotting { .. }
            | ModifierKind::Feared
            | ModifierKind::BloodGrenade { .. }
            | ModifierKind::Rooted
            | ModifierKind::Disarmed
            | ModifierKind::Broken
            | ModifierKind::BackdoorProtection
            | ModifierKind::Fortified
            | ModifierKind::TimedLife
            | ModifierKind::Illusion
            | ModifierKind::NeutralUpgrade { .. }
            | ModifierKind::FleshHeap { .. }
            | ModifierKind::Souls { .. }
            | ModifierKind::Feasting { .. }
            | ModifierKind::FeastLinger { .. }
            | ModifierKind::VisionGrant { .. }
            | ModifierKind::LampOff
            | ModifierKind::CaptureGuard
            | ModifierKind::ShrineReady
            | ModifierKind::Spell { .. } => {}
            ModifierKind::Dismembered { .. } | ModifierKind::MeatShield { .. } => {}
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
    // The enemies in a rot wear the effect its owner does: Dota's one
    // modifier_pudge_rot. Hasted movement shows as the haste it is.
    let mut shown: Vec<(u16, String)> = every_modifier()
        .iter()
        .map(|kind| match kind {
            ModifierKind::Rotting { .. } => (kind.effect_id().0, "Rot".to_string()),
            ModifierKind::Haste { .. } => (kind.effect_id().0, "Hastened".to_string()),
            _ => (kind.effect_id().0, variant_name(kind)),
        })
        .collect();
    shown.push((EffectId::FORTIFIED.0, "Fortified".to_string()));
    shown.push((EffectId::ILLUSION.0, "Illusion".to_string()));
    for entry in NEUTRAL_ABILITIES {
        let Some(effect) = spell_effect(entry.ability) else {
            continue;
        };
        let def = effect_def(effect).expect("a spell's effect is live");
        shown.push((effect.0, def.name.to_string()));
    }
    let registry: Vec<RegisteredId> = registered(IdSpace::Effect).collect();
    for (id, name) in &shown {
        let def = effect_def(EffectId(*id))
            .unwrap_or_else(|| panic!("effect {id} ({name}) is retired or not in the table"));
        assert_eq!(def.name, name, "effect {id} changed meaning");
        assert!(
            registry
                .iter()
                .any(|entry| entry.id == *id && !entry.retired),
            "effect {id} ({name}) is not live in registry/ids.txt"
        );
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
