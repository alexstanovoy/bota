//! Failure and side choice are explicit, rather than a same-side promise.
use super::placements::perpendicular;
use super::*;

#[test]
fn forced_release_side_is_deterministic_and_does_not_depend_on_the_approach_side() {
    for (centre, vertical, owner) in GATES {
        for arc in [false, true] {
            let mut endings = Vec::new();
            for approach in [-1, 1] {
                let mut world = match_world();
                let seat = usize::from(owner == Team::Radiant);
                let unit = world.seats[seat].unit.expect("hero");
                issue(
                    &mut world,
                    seat,
                    Order::Cheat {
                        cheat: Cheat::Teleport {
                            at: centre + perpendicular(vertical, 70 * approach),
                        },
                    },
                );
                begin_motion(&mut world, unit, centre, arc);
                for _ in 0..60 {
                    world.step();
                    if !world.push.contains(unit) {
                        break;
                    }
                }
                assert!(!world.push.contains(unit));
                assert_landing_outside_field(&world, unit, centre, vertical);
                endings.push(at(&world, unit));
            }
            assert_eq!(
                endings[0], endings[1],
                "a desired-point search must not impose the old side"
            );
        }
    }
}

#[test]
fn exhausted_clear_space_reports_failure_and_removes_the_arc_controller() {
    use crate::game::Clearance;
    let (centre, vertical, owner) = GATES[0];
    let mut world = match_world();
    let seat = usize::from(owner == Team::Radiant);
    let unit = world.seats[seat].unit.expect("hero");
    issue(
        &mut world,
        seat,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: centre - perpendicular(vertical, 70),
            },
        },
    );
    begin_motion(&mut world, unit, centre, true);
    let mut cells = bota_core::CellGrid::open();
    for y in 0..rules::GRID_CELLS {
        for x in 0..rules::GRID_CELLS {
            cells.close_cell(x, y);
        }
    }
    world.clearance = Clearance::from_cells(cells);
    let result = world.resolve_landing_for(unit, centre);
    assert!(!result.cleared);
    assert_eq!(
        result.at, centre,
        "the fallback is map-bounded, not certified field clearance"
    );
    for _ in 0..20 {
        world.step();
        if !world.push.contains(unit) {
            break;
        }
    }
    assert!(
        !world.push.contains(unit),
        "an exhausted clear-space search does not retain the controller (established from the installed game)"
    );
    assert_eq!(at(&world, unit), centre);
}
