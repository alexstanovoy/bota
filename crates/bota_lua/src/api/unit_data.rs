//! Public catalogue bodies identified only by received unit metadata.

use crate::state::State;
use bota_core::UnitDef;
use bota_proto::EntityId;

pub(super) fn base_definition(state: &State, id: EntityId) -> Option<&'static UnitDef> {
    if let Some(hero) = state.hero(id).and_then(bota_core::hero_def) {
        return Some(hero.unit);
    }
    bota_client_core::unit_definition(state.snapshot.as_ref()?.map, state.unit(id)?)
}
