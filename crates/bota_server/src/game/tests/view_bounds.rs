//! A view never passes the protocol's bounds, however crowded the world: what does not fit is
//! left out the same way every time, the heroes and their side's units first.

use bota_proto::Team;
use bota_proto::limits::MAX_VIEW_UNITS;

use crate::game::wire_id;

use super::support::*;

#[test]
fn a_crowded_world_is_told_within_the_bounds_its_heroes_first() {
    let (mut world, hero) = match_hero();
    let spot = an_empty_spot(&world);
    for _ in 0..=MAX_VIEW_UNITS {
        thinking_creep_of(&mut world, Team::Dire, spot);
    }
    world.settle();
    for view in [world.view_full(), world.view(Team::Radiant)] {
        assert!(
            view.units.len() <= MAX_VIEW_UNITS,
            "{} units",
            view.units.len()
        );
        assert!(
            view.units.iter().any(|unit| unit.id == wire_id(hero)),
            "the hero is told"
        );
        assert!(
            view.units.windows(2).all(|pair| pair[0].id < pair[1].id),
            "the units stay sorted"
        );
    }
    assert_eq!(
        world.view_full(),
        world.view_full(),
        "the same world, the same view"
    );
}

/// A unit kept whose target is left out keeps its entry of whom it fights, the target absent:
/// what it does is told, at whom is not.
#[test]
fn a_unit_whose_target_is_left_out_keeps_its_aggro_without_the_target() {
    let (mut world, hero) = match_hero();
    let spot = an_empty_spot(&world);
    let mut last = None;
    for _ in 0..=MAX_VIEW_UNITS {
        last = Some(thinking_creep_of(&mut world, Team::Dire, spot));
    }
    world.settle();
    let target = last.expect("a creep");
    world.set_target(hero, target);
    let view = world.view_full();
    assert!(
        view.units.iter().all(|unit| unit.id != wire_id(target)),
        "the target is left out"
    );
    let entry = view
        .aggro
        .iter()
        .find(|entry| entry.unit == wire_id(hero))
        .expect("the hero's aggro is told");
    assert_eq!(entry.target, None);
    assert_eq!(entry.held_for, None, "no hold on nobody");
}
