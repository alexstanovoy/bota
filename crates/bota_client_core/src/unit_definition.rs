//! Body catalogue selection for a received observation.
use bota_core::{MapDef, UnitDef, UnitIdentity};
use bota_proto::UnitView;
/// The catalogue body underlying a received unit, before temporary stat changes.
pub fn unit_definition(map: &MapDef, unit: &UnitView) -> Option<&'static UnitDef> {
    bota_core::unit_definition(
        map,
        UnitIdentity {
            kind: unit.kind,
            hero: unit.hero,
            neutral: unit.neutral,
            team: unit.team,
            pos: unit.pos,
            true_sight_radius: unit.true_sight_radius,
        },
    )
}
