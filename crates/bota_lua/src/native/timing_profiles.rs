//! Bounded setup-only model timing metadata from the local install.

use crate::{BindingError, ModuleBank};
use bota_client_core::ActivityProfile;
use heatel::{
    DirSource, Resource, Vpk,
    keyvalues::{Kv, parse_kv, text},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_MODEL_BYTES: u64 = 64 << 20;
const MAX_TOTAL_BYTES: u64 = 512 << 20;
const MAX_MODELS: usize = 256;
const MAX_UNITS: usize = 1024;

pub(super) fn populate(bank: &mut ModuleBank, runtime: Option<&Path>) -> Result<(), BindingError> {
    let Some(game) = runtime.and_then(|path| {
        path.ancestors()
            .take(8)
            .find(|path| path.join("pak01_dir.vpk").is_file())
    }) else {
        return Ok(());
    };
    let source = DirSource::new(game);
    let vpk = Vpk::open(&source, "pak01_dir.vpk").map_err(|_| error())?;
    let mut remaining = MAX_TOTAL_BYTES;
    let mut trees = Vec::new();
    for path in ["scripts/npc/npc_units.txt", "scripts/npc/npc_heroes.txt"] {
        let bytes = read(&vpk, &source, path, &mut remaining, 16 << 20)?;
        trees.push(parse_kv(text(&bytes).map_err(|_| error())?).map_err(|_| error())?);
    }
    for hero in &bota_core::HEROES {
        let path = format!("scripts/npc/heroes/{}.txt", hero.key);
        if vpk.get(&path).is_some() {
            let bytes = read(&vpk, &source, &path, &mut remaining, MAX_MODEL_BYTES)?;
            trees.push(parse_kv(text(&bytes).map_err(|_| error())?).map_err(|_| error())?);
        }
    }
    let mut units = BTreeMap::new();
    for tree in &trees {
        if let Some(root) = tree.get("DOTAUnits").or_else(|| tree.get("DOTAHeroes")) {
            for (name, definition) in root.block().unwrap_or_default() {
                if definition.block().is_some() {
                    if name.len() > 256 || units.len() >= 20_000 {
                        return Err(error());
                    }
                    units.insert(name.as_str(), definition);
                }
            }
        }
    }
    let names: BTreeSet<_> = bota_core::HEROES
        .iter()
        .map(|hero| hero.key)
        .chain(bota_core::NEUTRALS.iter().map(|neutral| neutral.unit))
        .chain(bota_core::UNIT_KINDS.iter().filter_map(|kind| kind.npc))
        .chain([
            "npc_dota_courier",
            "npc_dota_observer_wards",
            "npc_dota_sentry_wards",
        ])
        .collect();
    let mut models = BTreeMap::new();
    let mut selected = 0;
    for (&name, definition) in &units {
        if !(names.contains(name)
            || name.starts_with("npc_dota_creep_")
            || name.starts_with("npc_dota_goodguys_")
            || name.starts_with("npc_dota_badguys_"))
        {
            continue;
        }
        selected += 1;
        if selected > MAX_UNITS {
            return Err(error());
        }
        let Some(model) = model_name(definition, &units) else {
            continue;
        };
        if model.len() > 512 {
            return Err(error());
        }
        if !models.contains_key(model) {
            if models.len() >= MAX_MODELS {
                return Err(error());
            }
            let path = if model.ends_with("_c") {
                model.to_owned()
            } else {
                format!("{model}_c")
            };
            if vpk.get(&path).is_none() {
                continue;
            }
            let bytes = read(&vpk, &source, &path, &mut remaining, MAX_MODEL_BYTES)?;
            let resource = Resource::parse(&bytes).map_err(|_| error())?;
            let sequences = heatel::anim::read_sequences(&resource).map_err(|_| error())?;
            models.insert(model, from_sequences(&sequences)?);
        }
        if let Some(&profile) = models.get(model) {
            bank.set_activity_profile(name, profile)?;
        }
    }
    Ok(())
}

fn model_name<'a>(mut node: &'a Kv, units: &BTreeMap<&str, &'a Kv>) -> Option<&'a str> {
    for _ in 0..16 {
        if let Some(Kv::Text(model)) = node.get("Model") {
            return (!model.is_empty()).then_some(model.as_str());
        }
        let Kv::Text(base) = node.get("BaseClass")? else {
            return None;
        };
        node = units.get(base.as_str())?;
    }
    None
}

fn read(
    vpk: &Vpk,
    source: &DirSource,
    path: &str,
    remaining: &mut u64,
    limit: u64,
) -> Result<Vec<u8>, BindingError> {
    let entry = vpk.get(path).ok_or_else(error)?;
    if entry.size() > limit {
        return Err(error());
    }
    *remaining = remaining.checked_sub(entry.size()).ok_or_else(error)?;
    vpk.read(source, entry).map_err(|_| error())
}

fn from_sequences(sequences: &[heatel::anim::Sequence]) -> Result<ActivityProfile, BindingError> {
    if sequences.is_empty() {
        return Ok(ActivityProfile {
            idle_ms: Some(0),
            run_ms: Some(0),
            ..ActivityProfile::default()
        });
    }
    let duration = |activity| -> Result<Option<u32>, BindingError> {
        sequences
            .iter()
            .find(|sequence| {
                sequence
                    .activities
                    .iter()
                    .any(|a| a.name == activity && a.modifiers.is_empty() && a.weight > 0)
            })
            .map(|sequence| duration_ms(sequence.duration()))
            .transpose()
    };
    Ok(ActivityProfile {
        idle_ms: duration("ACT_DOTA_IDLE")?,
        rare_idle_ms: duration("ACT_DOTA_IDLE_RARE")?,
        run_ms: duration("ACT_DOTA_RUN")?,
        flail_ms: duration("ACT_DOTA_FLAIL")?,
        channel_ms: [
            duration("ACT_DOTA_CHANNEL_ABILITY_1")?,
            duration("ACT_DOTA_CHANNEL_ABILITY_2")?,
            duration("ACT_DOTA_CHANNEL_ABILITY_3")?,
            duration("ACT_DOTA_CHANNEL_ABILITY_4")?,
            duration("ACT_DOTA_CHANNEL_ABILITY_5")?,
            duration("ACT_DOTA_CHANNEL_ABILITY_6")?,
        ],
        stunned_ms: duration("ACT_DOTA_DISABLED")?,
        die_ms: duration("ACT_DOTA_DIE")?,
        spawn_ms: duration("ACT_DOTA_SPAWN")?,
        alternate_attacks: sequences.iter().any(|sequence| {
            sequence
                .activities
                .iter()
                .any(|a| a.name == "ACT_DOTA_ATTACK2" && a.modifiers.is_empty() && a.weight > 0)
        }),
    })
}

#[expect(
    clippy::float_arithmetic,
    reason = "local animation seconds are checked and converted once into bounded integer timing metadata"
)]
fn duration_ms(seconds: f32) -> Result<u32, BindingError> {
    if !seconds.is_finite() || !(0.0..=600.0).contains(&seconds) {
        return Err(error());
    }
    Ok((seconds * 1000.0).round_ties_even() as u32)
}
fn error() -> BindingError {
    BindingError::Configuration("local activity timing metadata is malformed or exceeds its bounds")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_timing_rejects_nonfinite_and_out_of_range_values() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 601.0] {
            assert!(duration_ms(value).is_err());
        }
        assert_eq!(duration_ms(0.0).expect("static"), 0);
        assert_eq!(duration_ms(600.0).expect("boundary"), 600_000);
        let sequence = heatel::anim::Sequence {
            name: "original_fixture".to_owned(),
            activities: vec![heatel::anim::Activity {
                name: "ACT_DOTA_IDLE".to_owned(),
                weight: 1,
                modifiers: Vec::new(),
            }],
            looping: true,
            frames: 31,
            fps: 30.0,
            events: Vec::new(),
        };
        assert_eq!(
            from_sequences(&[sequence]).expect("fixture").idle_ms,
            Some(1000)
        );
    }
}
