//! Fixed-seed movement contracts across starts, owners, body kinds and order families.
use super::*;
use crate::game::UnitOrder;
use bota_core::NeutralKind;

fn random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn offset(state: &mut u32, span: u32) -> Vec2 {
    let coordinate = |state: &mut u32| (random(state) % (2 * span + 1)) as i32 - span as i32;
    Vec2::from_ints(coordinate(state), coordinate(state))
}

fn check_step(
    world: &World,
    unit: Entity,
    centre: Vec2,
    vertical: bool,
    blocked: bool,
    previous: &mut Vec2,
) {
    let current = at(world, unit);
    if blocked {
        assert_clear_field(
            centre,
            vertical,
            *previous,
            current,
            collision_radius(world, unit),
        );
    }
    *previous = current;
}

fn random_start(world: &World, unit: Entity, centre: Vec2, state: &mut u32) -> Vec2 {
    let body = world.landing_body(unit);
    for _ in 0..32 {
        let candidate = centre + offset(state, 350);
        if world.clearance.stands_clear(candidate) && world.field_placement_clear(body, candidate) {
            return candidate;
        }
    }
    world
        .open_landing_for(unit, centre)
        .expect("the map supplies a legal bounded fallback")
}

fn random_orders(
    gate: (Vec2, bool, Team),
    team: Team,
    kind: usize,
    state: &mut u32,
    seen: &mut [u32; 6],
) {
    let (centre, vertical, owner) = gate;
    let mut world = match_heroes([HeroId::SHADOW_FIEND; 2], false);
    let seat = usize::from(team == Team::Dire);
    let other = 1 - seat;
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
            cheat: Cheat::AllVision { on: true },
        },
    );
    let home = world
        .pos(world.seats[seat].unit.expect("hero"))
        .expect("fountain");
    let unit = match kind {
        0 => world.seats[seat].unit.expect("hero"),
        1 => world.spawn_unit(&bota_core::MELEE_CREEP, team, home),
        _ => world.spawn_unit(NeutralKind::SkeletonWarrior.def(), team, home),
    };
    // Owned body fixtures exercise the same public order validation as heroes.
    world.owner.insert(unit, world.seats[seat].slot);
    world.march.remove(unit);
    world.set_order(unit, UnitOrder::Stand);
    // Isolate movement from incidental combat deaths; collision and flight are unchanged.
    world.put_modifier(
        unit,
        crate::game::Modifier::new(bota_core::ModifierKind::Shielded, None, None, None),
    );
    issue(
        &mut world,
        other,
        Order::Cheat {
            cheat: Cheat::ApplyModifier {
                target: Target::None,
                spec: bota_proto::ModifierSpec {
                    max_hp: 40000,
                    ..bota_proto::ModifierSpec::NOMINAL
                },
                ticks: 600,
            },
        },
    );
    world.settle();
    let start = random_start(&world, unit, centre, state);
    issue_to(
        &mut world,
        seat,
        Some(unit),
        Order::Cheat {
            cheat: Cheat::Teleport { at: start },
        },
    );
    let ally = world.spawn_unit(
        &bota_core::MELEE_CREEP,
        team,
        start + Vec2::from_ints(150, 0),
    );
    world.owner.insert(ally, world.seats[seat].slot);
    world.march.remove(ally);
    world.set_order(ally, UnitOrder::Stand);
    world.put_modifier(
        ally,
        crate::game::Modifier::new(bota_core::ModifierKind::Shielded, None, None, None),
    );
    world.settle();
    let enemy = world.seats[other].unit.expect("enemy hero");
    let mut previous = at(&world, unit);
    for step in 0..96 {
        if step % 4 == 0 {
            let ally_at = random_start(&world, ally, centre, state);
            issue_to(
                &mut world,
                seat,
                Some(ally),
                Order::Cheat {
                    cheat: Cheat::Teleport { at: ally_at },
                },
            );
            check_step(&world, unit, centre, vertical, team != owner, &mut previous);
            issue(
                &mut world,
                other,
                Order::Cheat {
                    cheat: Cheat::Teleport {
                        at: centre + offset(state, 450),
                    },
                },
            );
            check_step(&world, unit, centre, vertical, team != owner, &mut previous);
            issue(
                &mut world,
                other,
                Order::Move {
                    target: Target::None,
                },
            );
            check_step(&world, unit, centre, vertical, team != owner, &mut previous);
            let target = centre + offset(state, 500);
            let choice = (random(state) % 6) as usize;
            seen[choice] += 1;
            let order = match choice {
                0 => Order::Move {
                    target: Target::Pos(target),
                },
                1 => Order::MoveDirection { toward: target },
                2 => Order::Attack {
                    target: Target::Pos(target),
                },
                3 => Order::Attack {
                    target: Target::Unit(crate::game::wire_id(ally)),
                },
                4 => Order::Move {
                    target: Target::Unit(crate::game::wire_id(ally)),
                },
                _ => Order::Attack {
                    target: Target::Unit(crate::game::wire_id(enemy)),
                },
            };
            issue_to(&mut world, seat, Some(unit), order);
            check_step(&world, unit, centre, vertical, team != owner, &mut previous);
        }
        world.step();
        check_step(&world, unit, centre, vertical, team != owner, &mut previous);
    }
}

#[test]
fn bounded_random_starts_and_all_movement_orders_keep_hero_creep_and_summon_hulls_clear() {
    let mut state = 0x4d39_81b7_u32;
    let mut seen = [0u32; 6];
    for gate in GATES {
        for team in [Team::Radiant, Team::Dire] {
            for kind in 0..3 {
                for _ in 0..6 {
                    random_orders(gate, team, kind, &mut state, &mut seen);
                }
            }
        }
    }
    assert!(
        seen.iter().all(|count| *count > 0),
        "all six order families must run: {seen:?}"
    );
}
