//! Planner output and stable endings are contracts in addition to safe steps.
use super::placements::perpendicular;
use super::*;

#[test]
fn a_plan_to_a_blocked_field_centre_ends_outside_the_field_and_stays_stopped() {
    use crate::game::{Planner, UnitOrder};
    for (centre, vertical, owner) in GATES {
        for approach in [-1, 1] {
            let mut world = match_world();
            let seat = usize::from(owner == Team::Radiant);
            let unit = world.seats[seat].unit.expect("hero");
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::CreepSpawning { on: false },
                },
            );
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::KillCreeps {
                        which: bota_proto::CreepSide::All,
                    },
                },
            );
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport {
                        at: centre + perpendicular(vertical, approach * 200),
                    },
                },
            );
            let from = at(&world, unit);
            let radius = collision_radius(&world, unit);
            let ob = world.walking_obstacles(unit, &[]);
            let corners = Planner::new().find_path(&ob, from, centre, radius);
            for point in corners {
                assert_clear_field(centre, vertical, point, point, radius);
            }
            issue(
                &mut world,
                seat,
                Order::Move {
                    target: Target::Pos(centre),
                },
            );
            for _ in 0..900 {
                world.step();
            }
            let stopped = at(&world, unit);
            for _ in 0..120 {
                world.step();
                assert!(
                    at(&world, unit).within(stopped, rules::units(1)),
                    "an unreachable goal must not keep oscillating"
                );
                assert_landing_outside_field(&world, unit, centre, vertical);
            }
            assert!(matches!(
                world.orders.get(unit).expect("order").current,
                UnitOrder::Move { .. }
            ));
        }
    }
}
