//! The bounds an applied modifier spec must stay within.

use crate::*;

/// One spec per scale field, all others nominal, the named one set to `scale`.
fn with_scale(scale: i32) -> [ModifierSpec; 9] {
    let mut specs = [ModifierSpec::NOMINAL; 9];
    specs[0].physical_damage = scale;
    specs[1].magic_damage = scale;
    specs[2].pure_damage = scale;
    specs[3].cooldown_rate = scale;
    specs[4].mana_cost_rate = scale;
    specs[5].move_speed = scale;
    specs[6].max_hp = scale;
    specs[7].max_mana = scale;
    specs[8].gold_income = scale;
    specs
}

#[test]
fn a_nominal_spec_is_bounded_and_changes_nothing() {
    assert!(ModifierSpec::NOMINAL.is_bounded());
    assert!(ModifierSpec::NOMINAL.is_nominal());
}

#[test]
fn a_resistance_delta_at_its_farthest_is_still_bounded() {
    let mut spec = ModifierSpec::NOMINAL;
    spec.magic_resist = ModifierSpec::MAX_RESIST;
    assert!(spec.is_bounded());
    spec.magic_resist = -ModifierSpec::MAX_RESIST;
    assert!(spec.is_bounded());
    spec = ModifierSpec::NOMINAL;
    spec.status_resist = ModifierSpec::MAX_STATUS_RESIST;
    assert!(spec.is_bounded());
}

#[test]
fn a_resistance_delta_beyond_its_bound_is_rejected() {
    let mut spec = ModifierSpec::NOMINAL;
    spec.magic_resist = ModifierSpec::MAX_RESIST + 1;
    assert!(!spec.is_bounded(), "{spec:?} must be rejected");
    spec.magic_resist = -ModifierSpec::MAX_RESIST - 1;
    assert!(!spec.is_bounded(), "{spec:?} must be rejected");
    spec = ModifierSpec::NOMINAL;
    spec.status_resist = -1;
    assert!(!spec.is_bounded(), "{spec:?} must be rejected");
    spec.status_resist = ModifierSpec::MAX_STATUS_RESIST + 1;
    assert!(!spec.is_bounded(), "{spec:?} must be rejected");
}

#[test]
fn a_scale_at_either_bound_is_bounded_in_every_field() {
    for scale in [ModifierSpec::MIN_SCALE, ModifierSpec::MAX_SCALE] {
        for spec in with_scale(scale) {
            assert!(spec.is_bounded(), "{spec:?} must be accepted");
        }
    }
}

#[test]
fn a_scale_beyond_its_bounds_is_rejected_in_every_field() {
    for scale in [ModifierSpec::MIN_SCALE - 1, ModifierSpec::MAX_SCALE + 1] {
        for spec in with_scale(scale) {
            assert!(!spec.is_bounded(), "{spec:?} must be rejected");
        }
    }
}
