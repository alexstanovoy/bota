//! What a kind of unit is, by its kind alone.

use bota_proto::UnitKind;

/// Whether a kind of unit is a building: it stands still, blocks the ground,
/// and both sides always know where it is.
#[inline]
pub fn is_structure(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::Tower
            | UnitKind::Ancient
            | UnitKind::Barracks
            | UnitKind::Fountain
            | UnitKind::Filler
    )
}

/// Whether a kind of unit leaves anything to what is gathered from deaths:
/// the flesh heap and the souls. A structure or a ward leaves nothing.
#[inline]
pub fn leaves_a_death(kind: UnitKind) -> bool {
    !is_structure(kind) && kind != UnitKind::Ward
}

/// Whether a kind of unit is a lane creep: one of what a wave is made of.
#[inline]
pub fn is_lane_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepRanged
            | UnitKind::CreepSiege
    )
}

/// Whether a kind of unit is a creep: what a wave is made of, and what the
/// jungle grows.
#[inline]
pub fn is_creep(kind: UnitKind) -> bool {
    is_lane_creep(kind) || kind == UnitKind::CreepNeutral
}
