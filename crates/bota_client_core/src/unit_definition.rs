//! Base bodies selected only from public map and unit identity data.

use bota_core::{MapDef, UnitDef};
use bota_proto::{UnitKind, UnitView};

/// The catalogue body underlying a received unit, before temporary stat changes.
pub fn unit_definition(map: &MapDef, unit: &UnitView) -> Option<&'static UnitDef> {
    if let Some(hero) = unit.hero {
        return bota_core::hero_def(hero).map(|hero| hero.unit);
    }
    if let Some(neutral) = unit.neutral {
        return bota_core::NEUTRALS
            .get(usize::from(neutral.0))
            .map(|row| &row.def);
    }
    if let Some(definition) = bota_core::lane_creep_def(unit.kind) {
        return Some(definition);
    }
    Some(match unit.kind {
        UnitKind::Tower => {
            bota_core::tower_def(bota_core::tower_tier_at(map, unit.team, unit.pos)?)
        }
        UnitKind::Barracks => {
            if bota_core::barracks_ranged_at(map, unit.team, unit.pos)? {
                &bota_core::BARRACKS_RANGED
            } else {
                &bota_core::BARRACKS_MELEE
            }
        }
        UnitKind::Ancient => bota_core::ancient_of(unit.team),
        UnitKind::Fountain => &bota_core::FOUNTAIN,
        UnitKind::Filler => &bota_core::FILLER,
        UnitKind::Courier => &bota_core::COURIER,
        UnitKind::Ward => {
            if unit.true_sight_radius > bota_proto::Fixed::ZERO {
                &bota_core::SENTRY_WARD
            } else {
                &bota_core::OBSERVER_WARD
            }
        }
        UnitKind::Outpost => &bota_core::OUTPOST,
        UnitKind::Watcher => &bota_core::WATCHER,
        UnitKind::Shrine => &bota_core::SHRINE,
        UnitKind::TwinGate => &bota_core::TWIN_GATE,
        _ => return None,
    })
}
