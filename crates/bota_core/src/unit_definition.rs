//! Body catalogue queries from public unit identity and map data.
use crate::{MapDef, UnitDef};
use bota_proto::{Fixed, HeroId, NeutralId, Team, UnitKind, Vec2};

/// Identity data sufficient to select a catalogue body; no mutable world state is read.
#[derive(Clone, Copy, Debug)]
pub struct UnitIdentity {
    pub kind: UnitKind,
    pub hero: Option<HeroId>,
    pub neutral: Option<NeutralId>,
    pub team: Team,
    pub pos: Vec2,
    pub true_sight_radius: Fixed,
}
/// O(1) catalogue lookup; structure locations scan at most the configured building table.
pub fn unit_definition(map: &MapDef, unit: UnitIdentity) -> Option<&'static UnitDef> {
    if let Some(hero) = unit.hero {
        return crate::hero_def(hero).map(|hero| hero.unit);
    }
    if let Some(neutral) = unit.neutral {
        return crate::NEUTRALS
            .get(usize::from(neutral.0))
            .map(|row| &row.def);
    }
    if let Some(definition) = crate::lane_creep_def(unit.kind) {
        return Some(definition);
    }
    Some(match unit.kind {
        UnitKind::Tower => crate::tower_def(crate::tower_tier_at(map, unit.team, unit.pos)?),
        UnitKind::Barracks => {
            if crate::barracks_ranged_at(map, unit.team, unit.pos)? {
                &crate::BARRACKS_RANGED
            } else {
                &crate::BARRACKS_MELEE
            }
        }
        UnitKind::Ancient => crate::ancient_of(unit.team),
        UnitKind::Fountain => &crate::FOUNTAIN,
        UnitKind::Filler => &crate::FILLER,
        UnitKind::Courier => &crate::COURIER,
        UnitKind::Ward => {
            if unit.true_sight_radius > bota_proto::Fixed::ZERO {
                &crate::SENTRY_WARD
            } else {
                &crate::OBSERVER_WARD
            }
        }
        UnitKind::Outpost => &crate::OUTPOST,
        UnitKind::Watcher => &crate::WATCHER,
        UnitKind::Shrine => &crate::SHRINE,
        UnitKind::TwinGate => &crate::TWIN_GATE,
        _ => return None,
    })
}

/// Half the base attack-damage interval, including half-integer variances. O(1).
pub fn base_damage_variance(definition: &UnitDef) -> Fixed {
    Fixed::from_ratio(definition.damage_spread, 2)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base_damage_variance_preserves_half_integer_intervals() {
        let mut definition = *crate::hero_def(HeroId::SHADOW_FIEND).unwrap().unit;
        definition.damage_spread = 3;
        assert_eq!(base_damage_variance(&definition), Fixed::from_ratio(3, 2));
    }
}
