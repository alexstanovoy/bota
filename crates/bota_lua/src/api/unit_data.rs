//! Public catalogue bodies identified only by received unit metadata.

use crate::state::State;
use bota_core::UnitDef;
use bota_proto::{EntityId, UnitKind};

pub(super) fn base_definition(state: &State, id: EntityId) -> Option<&'static UnitDef> {
    if let Some(hero) = state.hero(id).and_then(bota_core::hero_def) {
        return Some(hero.unit);
    }
    let unit = state.unit(id)?;
    if let Some(neutral) = unit.neutral {
        return bota_core::NEUTRALS
            .get(usize::from(neutral.0))
            .map(|row| &row.def);
    }
    if let Some(definition) = bota_core::lane_creep_def(unit.kind) {
        return Some(definition);
    }
    Some(match unit.kind {
        UnitKind::Tower => bota_core::tower_def(bota_core::tower_tier_at(
            state.snapshot.as_ref()?.map,
            unit.team,
            unit.pos,
        )?),
        UnitKind::Barracks => {
            if bota_core::barracks_ranged_at(state.snapshot.as_ref()?.map, unit.team, unit.pos)? {
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
