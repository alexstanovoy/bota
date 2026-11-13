//! Channel destinations deliberately near a base field, rather than a fountain.
use super::placements::perpendicular;
use super::*;
use crate::game::{Modifier, Place};
use bota_proto::{AbilitySlot, ItemId, ItemSlot};

fn scroll_near_field(centre: Vec2, vertical: bool, owner: Team, flight_expires: bool) {
    let mut world = match_world();
    let seat = usize::from(owner == Team::Radiant);
    let unit = world.seats[seat].unit.expect("hero");
    let team = world.seats[seat].team;
    world.spawn_building(
        bota_core::tower_def(1),
        team,
        centre - perpendicular(vertical, 220),
        Place::Tower {
            lane: rules::LANE_MID,
            tier: 1,
        },
    );
    world.settle();
    world.lay_passability();
    if flight_expires {
        world.put_modifier(
            unit,
            Modifier::new(bota_core::ModifierKind::Flying, Some(unit), None, Some(60)),
        );
        world.settle();
    }
    issue(
        &mut world,
        seat,
        Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::TOWN_PORTAL_SCROLL,
            },
        },
    );
    issue(
        &mut world,
        seat,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    issue(
        &mut world,
        seat,
        Order::Use {
            slot: ItemSlot::TOWN_PORTAL,
            target: Target::Pos(centre),
        },
    );
    for _ in 0..30 {
        if world.is_channelling(unit) {
            break;
        }
        world.step();
    }
    assert!(world.is_channelling(unit), "the real scroll must channel");
    let planned = world.seats[seat].teleport.expect("destination").landing;
    if flight_expires {
        assert!(planned.within(centre, rules::units(1)));
    } else {
        assert!(
            !planned.within(centre, rules::units(25)),
            "grounded destination must resolve before channel"
        );
    }
    for _ in 0..180 {
        world.step();
        if !world.is_channelling(unit) {
            break;
        }
    }
    assert!(!world.is_channelling(unit));
    assert!(!world.stats.get(unit).expect("stats").flies);
    assert_landing_outside_field(&world, unit, centre, vertical);
    assert!(
        at(&world, unit).within(centre, rules::units(500)),
        "the scroll lands near its actual target"
    );
}

#[test]
fn grounded_scroll_destinations_near_all_fields_clear_enemy_hulls_before_channel() {
    for (centre, vertical, owner) in GATES {
        scroll_near_field(centre, vertical, owner, false);
    }
}

// Existing flight with a bounded lifetime supplies a capability-change fixture.
#[test]
fn scroll_completion_revalidates_a_field_destination_after_flight_expires() {
    for (centre, vertical, owner) in GATES {
        scroll_near_field(centre, vertical, owner, true);
    }
}

#[test]
fn twin_gate_completion_revalidates_an_exit_beside_each_enemy_field() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        let unit = world.seats[seat].unit.expect("hero");
        let gate = world
            .entities
            .iter()
            .find(|&unit| world.gate.contains(unit))
            .expect("Twin Gate");
        let partner = world.gate.get(gate).expect("pair").partner;
        world.gate.get_mut(partner).expect("exit").exit = centre;
        let entrance = world.pos(gate).expect("entrance") + Vec2::from_ints(0, -170);
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Teleport { at: entrance },
            },
        );
        issue(
            &mut world,
            seat,
            Order::Attack {
                target: Target::Unit(crate::game::wire_id(gate)),
            },
        );
        let mut channelled = false;
        for _ in 0..180 {
            world.step();
            channelled |= world.warp.contains(unit);
            if channelled && !world.warp.contains(unit) {
                break;
            }
        }
        assert!(channelled, "a real Twin Gate channel must begin");
        assert!(!world.warp.contains(unit));
        assert!(
            at(&world, unit).within(centre, rules::units(500)),
            "the exit fixture is beside the field"
        );
        assert_landing_outside_field(&world, unit, centre, vertical);
    }
}

fn dismember_release(centre: Vec2, vertical: bool, owner: Team, interrupted: bool) {
    let caster_seat = usize::from(owner == Team::Dire);
    let victim_seat = 1 - caster_seat;
    let mut heroes = [HeroId::SHADOW_FIEND; 2];
    heroes[caster_seat] = HeroId::PUDGE;
    let mut world = match_heroes(heroes, false);
    let caster = world.seats[caster_seat].unit.expect("Pudge");
    let victim = world.seats[victim_seat].unit.expect("victim");
    issue(
        &mut world,
        caster_seat,
        Order::Cheat {
            cheat: Cheat::Levels { count: 8 },
        },
    );
    issue(
        &mut world,
        caster_seat,
        Order::Learn {
            slot: AbilitySlot(3),
        },
    );
    issue(
        &mut world,
        caster_seat,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: centre + perpendicular(vertical, 100),
            },
        },
    );
    issue(
        &mut world,
        victim_seat,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: centre - perpendicular(vertical, 70),
            },
        },
    );
    issue(
        &mut world,
        victim_seat,
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
    issue(
        &mut world,
        caster_seat,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    issue(
        &mut world,
        caster_seat,
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::Unit(crate::game::wire_id(victim)),
        },
    );
    let mut active = false;
    let mut overlapped = false;
    let mut stopped = false;
    for _ in 0..180 {
        world.step();
        active |= world.is_channelling(caster);
        let radius = collision_radius(&world, victim) + rules::units(24);
        let overlap = (-7..=7).any(|i| {
            let offset = if vertical {
                Vec2::from_ints(0, i * 25)
            } else {
                Vec2::from_ints(i * 25, 0)
            };
            at(&world, victim).within(centre + offset, radius)
        });
        overlapped |= overlap;
        if interrupted && overlap && world.is_channelling(caster) && !stopped {
            issue(
                &mut world,
                caster_seat,
                Order::Move {
                    target: Target::None,
                },
            );
            stopped = true;
        }
        if active && !world.is_channelling(caster) {
            break;
        }
    }
    assert!(
        active && overlapped,
        "the actual channel must pull a hull through the field"
    );
    assert_eq!(stopped, interrupted);
    assert!(!world.is_channelling(caster));
    assert_landing_outside_field(&world, victim, centre, vertical);
}

#[test]
fn dismember_completion_resolves_grounded_hulls_after_pull_segments_cross_fields() {
    for (centre, vertical, owner) in GATES {
        dismember_release(centre, vertical, owner, false);
    }
}

#[test]
fn interrupted_dismember_resolves_grounded_hulls_after_pull_segments_cross_fields() {
    for (centre, vertical, owner) in GATES {
        dismember_release(centre, vertical, owner, true);
    }
}
