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
