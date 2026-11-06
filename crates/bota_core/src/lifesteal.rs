//! Target-dependent lifesteal effectiveness. Roshan is a creep since 7.40.
use bota_proto::UnitKind;

/// Percent effectiveness after the damage actually taken: physical attacks or spell lifesteal.
/// The creep factors are the documented lifesteal mechanics; wards/buildings are filtered by
/// the damage consumer before this is read.
pub fn lifesteal_factor(kind: UnitKind, spell: bool) -> i32 {
    if crate::kind_def(kind).creep {
        if spell { 20 } else { 60 }
    } else {
        100
    }
}
