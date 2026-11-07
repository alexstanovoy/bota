//! Engine unit identifiers; hero and neutral identifiers come from core registries.

use crate::state::State;
use bota_proto::{EntityId, Team, UnitKind};

// The lane rows describe the npc_units naming families, including siege's different prefix.
const LANE_UNITS: &[(UnitKind, &str, bool)] = &[
    (UnitKind::CreepMelee, "melee", true),
    (UnitKind::CreepFlagbearer, "flagbearer", true),
    (UnitKind::CreepRanged, "ranged", true),
    (UnitKind::CreepSiege, "siege", false),
];

const NAMED_UNITS: &[(UnitKind, &str)] = &[
    (UnitKind::Courier, "npc_dota_courier"),
    (UnitKind::Outpost, "npc_dota_watch_tower"),
    (UnitKind::Watcher, "npc_dota_lantern"),
    (UnitKind::Shrine, "npc_dota_xp_fountain"),
    (UnitKind::LotusPool, "npc_dota_lotus_pool"),
];

pub(super) fn key(state: &State, id: EntityId) -> Option<String> {
    if let Some(hero) = state.hero(id).and_then(bota_core::hero_def) {
        return Some(hero.key.to_owned());
    }
    let unit = state.unit(id)?;
    if let Some(neutral) = unit.neutral {
        return Some(
            bota_core::NEUTRALS
                .get(usize::from(neutral.0))?
                .unit
                .to_owned(),
        );
    }
    if let Some(&(_, key)) = NAMED_UNITS.iter().find(|&&(kind, _)| kind == unit.kind) {
        return Some(key.to_owned());
    }
    if unit.kind == UnitKind::Ward {
        return Some(
            if unit.true_sight_radius > bota_proto::Fixed::ZERO {
                "npc_dota_sentry_wards"
            } else {
                "npc_dota_observer_wards"
            }
            .to_owned(),
        );
    }
    let faction = match unit.team {
        Team::Radiant => "goodguys",
        Team::Dire => "badguys",
        Team::Neutral => return None,
    };
    if let Some(&(_, suffix, creep_prefix)) =
        LANE_UNITS.iter().find(|&&(kind, _, _)| kind == unit.kind)
    {
        return Some(format!(
            "npc_dota_{}{faction}_{suffix}",
            if creep_prefix { "creep_" } else { "" }
        ));
    }
    let map = state.snapshot.as_ref()?.map;
    match unit.kind {
        UnitKind::Tower => {
            let towers = if unit.team == Team::Radiant {
                map.radiant_towers
            } else {
                map.dire_towers
            };
            let &(lane, tier, _) = towers
                .iter()
                .find(|&&(_, _, position)| position == unit.pos)?;
            Some(if tier == 4 {
                format!("npc_dota_{faction}_tower4")
            } else {
                format!("npc_dota_{faction}_tower{tier}_{}", lane_name(lane)?)
            })
        }
        UnitKind::Barracks => {
            let barracks = map.barracks[usize::from(unit.team == Team::Dire)];
            let &(lane, ranged, _) = barracks
                .iter()
                .find(|&&(_, _, position)| position == unit.pos)?;
            Some(format!(
                "npc_dota_{faction}_{}_rax_{}",
                if ranged { "range" } else { "melee" },
                lane_name(lane)?
            ))
        }
        UnitKind::Ancient => Some(format!("npc_dota_{faction}_fort")),
        UnitKind::Fountain => Some(format!("npc_dota_{faction}_healers")),
        UnitKind::Filler => Some(format!("npc_dota_{faction}_fillers")),
        _ => None,
    }
}

fn lane_name(lane: u8) -> Option<&'static str> {
    match lane {
        bota_core::rules::LANE_TOP => Some("top"),
        bota_core::rules::LANE_MID => Some("mid"),
        bota_core::rules::LANE_BOT => Some("bot"),
        _ => None,
    }
}
