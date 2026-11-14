//! Team-only base exits, reached through the same orders and cheats as a client.
use super::support::config;
use crate::game::{Command, Entity, World};
use bota_core::{map_of, rules, segment_distance_squared};
use bota_proto::{Cheat, Fixed, HeroId, MapId, Order, Pick, SlotId, Target, Team, Vec2};

// The four installed map placements, rounded to whole world units for this independent oracle.
const GATES: [(Vec2, bool, Team); 4] = [
    (Vec2::from_ints(1523, 6287), false, Team::Radiant),
    (Vec2::from_ints(16576, 11648), false, Team::Dire),
    (Vec2::from_ints(5771, 1741), true, Team::Radiant),
    (Vec2::from_ints(12160, 16512), true, Team::Dire),
];

fn match_world() -> World {
    match_heroes([HeroId::SHADOW_FIEND; 2], true)
}

fn match_heroes(heroes: [HeroId; 2], immortal: bool) -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.cheats = true;
    cfg.runes = map_of(cfg.map).default_runes;
    cfg.immortal_heroes = immortal;
    cfg.picks = [Team::Radiant, Team::Dire]
        .into_iter()
        .enumerate()
        .map(|(i, team)| Pick {
            slot: SlotId(i as u8),
            team,
            hero: heroes[i],
        })
        .collect();
    cfg.objectives = map_of(cfg.map).default_objectives;
    let mut world = World::for_match(&cfg, cfg.rng());
    issue(
        &mut world,
        0,
        Order::Cheat {
            cheat: Cheat::StartGame,
        },
    );
    world
}

fn issue(world: &mut World, seat: usize, order: Order) {
    issue_to(world, seat, None, order);
}

fn issue_to(world: &mut World, seat: usize, unit: Option<Entity>, order: Order) {
    let slot = world.seats[seat].slot;
    let named = unit.map(crate::game::wire_id);
    assert_eq!(
        world.validate_order(slot, named, &order),
        Ok(()),
        "{order:?}"
    );
    world.advance_trusted(&[Command {
        slot,
        unit: named,
        order,
        queue: false,
        seq: 0,
    }]);
}

fn at(world: &World, unit: Entity) -> Vec2 {
    world.pos(unit).expect("standing")
}

fn collision_radius(world: &World, unit: Entity) -> Fixed {
    world.hull.get(unit).expect("a hull").collision
}

#[test]
fn enemy_ground_orders_never_cross_any_of_the_four_base_fields() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = if owner == Team::Radiant { 1 } else { 0 };
        let unit = world.seats[seat].unit.expect("hero");
        let across = if vertical {
            Vec2::from_ints(70, 0)
        } else {
            Vec2::from_ints(0, 70)
        };
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across,
                },
            },
        );
        issue(
            &mut world,
            seat,
            Order::Move {
                target: Target::Pos(centre + across),
            },
        );
        let mut previous = at(&world, unit);
        for _ in 0..180 {
            world.step();
            let next = at(&world, unit);
            assert_clear_field(
                centre,
                vertical,
                previous,
                next,
                collision_radius(&world, unit),
            );
            previous = next;
        }
    }
}

#[test]
fn allied_heroes_walk_both_ways_through_all_four_base_fields() {
    for (centre, vertical, owner) in GATES {
        for direction in [-1, 1] {
            let mut world = match_world();
            let seat = usize::from(owner == Team::Dire);
            let unit = world.seats[seat].unit.expect("hero");
            let across = if vertical {
                Vec2::from_ints(70 * direction, 0)
            } else {
                Vec2::from_ints(0, 70 * direction)
            };
            let destination = centre + across;
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport {
                        at: centre - across,
                    },
                },
            );
            issue(
                &mut world,
                seat,
                Order::Move {
                    target: Target::Pos(destination),
                },
            );
            for _ in 0..180 {
                world.step();
                if at(&world, unit).within(destination, rules::units(10)) {
                    break;
                }
            }
            assert!(
                at(&world, unit).within(destination, rules::units(10)),
                "{owner:?} must cross its field at{centre:?}: destination{destination:?}, actual{:?}",
                at(&world, unit)
            );
        }
    }
}

fn assert_clear_field(centre: Vec2, vertical: bool, from: Vec2, to: Vec2, radius: Fixed) {
    let minimum = radius + Fixed::from_ratio(97, 4);
    for index in -7..=7 {
        let offset = if vertical {
            Vec2::from_ints(0, index * 25)
        } else {
            Vec2::from_ints(index * 25, 0)
        };
        assert!(
            segment_distance_squared(centre + offset, from, to) >= minimum.squared_raw(),
            "field{centre:?}: forbidden step{from:?} -> {to:?}"
        );
    }
}

#[test]
fn phased_directional_walks_stop_at_every_enemy_base_field() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        let unit = world.seats[seat].unit.expect("hero");
        let across = if vertical {
            Vec2::from_ints(70, 0)
        } else {
            Vec2::from_ints(0, 70)
        };
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across,
                },
            },
        );
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Item {
                    item: bota_proto::ItemId::PHASE_BOOTS,
                },
            },
        );
        issue(
            &mut world,
            seat,
            Order::Use {
                slot: bota_proto::ItemSlot(0),
                target: Target::None,
            },
        );
        for _ in 0..10 {
            world.step();
        }
        assert!(world.phased(unit), "the boots must really grant phase");
        let from = at(&world, unit);
        issue(
            &mut world,
            seat,
            Order::MoveDirection {
                toward: centre + across,
            },
        );
        assert_clear_field(
            centre,
            vertical,
            from,
            at(&world, unit),
            collision_radius(&world, unit),
        );
        let mut previous = at(&world, unit);
        for _ in 0..70 {
            world.step();
            let next = at(&world, unit);
            assert_clear_field(
                centre,
                vertical,
                previous,
                next,
                collision_radius(&world, unit),
            );
            previous = next;
        }
        assert!(
            previous.distance_squared(from) > rules::units(5).squared_raw(),
            "the hero must take a step before the field stops it"
        );
    }
}

#[test]
fn bounded_random_ground_orders_never_cross_enemy_base_fields() {
    let mut seed = 0x8cb9_a18f_u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for (centre, vertical, owner) in GATES {
        for direction in [-1, 1] {
            let mut world = match_world();
            let seat = usize::from(owner == Team::Radiant);
            let unit = world.seats[seat].unit.expect("hero");
            let across = if vertical {
                Vec2::from_ints(70 * direction, 0)
            } else {
                Vec2::from_ints(0, 70 * direction)
            };
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport {
                        at: centre - across,
                    },
                },
            );
            let mut previous = at(&world, unit);
            for step in 0..160 {
                if step % 4 == 0 {
                    let target = centre
                        + Vec2::from_ints(
                            (random() % 801) as i32 - 400,
                            (random() % 801) as i32 - 400,
                        );
                    let order = if random() & 1 == 0 {
                        Order::Move {
                            target: Target::Pos(target),
                        }
                    } else {
                        Order::MoveDirection { toward: target }
                    };
                    issue(&mut world, seat, order);
                } else {
                    world.step();
                }
                let next = at(&world, unit);
                assert_clear_field(
                    centre,
                    vertical,
                    previous,
                    next,
                    collision_radius(&world, unit),
                );
                previous = next;
            }
        }
    }
}

#[test]
fn base_field_sight_is_grounded_700_for_its_side_day_and_night_in_server_and_client() {
    use bota_client_core::{MatchStatic, SightGround};
    use bota_core::{CellGrid, point_ahead};
    use bota_proto::Angle;
    let mut cfg = config();
    cfg.map = MapId(0);
    let statics = MatchStatic::new(&cfg.info()).expect("map statics");
    for night in [false, true] {
        let mut world = match_world();
        issue(
            &mut world,
            0,
            Order::Cheat {
                cheat: Cheat::TimeOfDay { night },
            },
        );
        let entities: Vec<_> = world.entities.iter().collect();
        for entity in entities {
            world.despawn(entity);
        }
        world.settle();
        for (centre, _, owner) in GATES {
            let view = world.view(owner);
            let ground = SightGround::lay(&statics, Some(&view)).expect("client sight");
            let mut lit = vec![false; rules::GRID_CELLS * rules::GRID_CELLS];
            ground.lit(&view, owner, &mut lit);
            let enemy = if owner == Team::Radiant {
                Team::Dire
            } else {
                Team::Radiant
            };
            let enemy_view = world.view(enemy);
            for heading in 0..16 {
                let point = point_ahead(
                    centre,
                    Angle {
                        brads: heading * 4096,
                    },
                    rules::units(100),
                );
                assert!(
                    !world.can_see_point(enemy, point),
                    "{enemy:?} must not see {point:?} through the gate of {owner:?}, night={night}"
                );
                assert!(
                    !ground.sees(&enemy_view, enemy, point),
                    "{enemy:?} client sight must not include the gate of {owner:?}, night={night}"
                );
            }
            let mut clear_inside = 0;
            for heading in 0..16 {
                let point = point_ahead(
                    centre,
                    Angle {
                        brads: heading * 4096,
                    },
                    rules::units(675),
                );
                let tier = world.ground.tier(centre);
                if world.ground.tier(point) <= tier && world.sight_block.clear(centre, tier, point)
                {
                    clear_inside += 1;
                    assert!(
                        world.can_see_point(owner, point),
                        "{owner:?} sight at{centre:?} stops short of675, night={night}"
                    );
                }
            }
            assert!(
                clear_inside > 0,
                "the reference gate must have an unobstructed line to test its700 radius"
            );
            let (cx, cy) = CellGrid::cell_of(centre).expect("map cell");
            for y in cy - 12..=cy + 12 {
                for x in cx - 12..=cx + 12 {
                    let point = CellGrid::cell_center((x, y));
                    let expected = world.can_see_point(owner, point);
                    assert_eq!(
                        ground.sees(&view, owner, point),
                        expected,
                        "point sight mismatch{point:?},night={night}"
                    );
                    assert_eq!(
                        lit[y * rules::GRID_CELLS + x],
                        expected,
                        "lit cell mismatch{x},{y},night={night}"
                    );
                    if point.distance_squared(centre) > rules::units(701).squared_raw() {
                        assert!(!expected, "the isolated gate sees beyond700");
                    }
                }
            }
        }
    }
}

#[test]
fn solo_mid_and_demo_maps_have_no_team_base_fields_or_field_sight() {
    for map in [MapId(1), MapId(2)] {
        assert!(map_of(map).base_blockers.is_empty());
    }
}

fn rune_copy(world: &mut World, seat: usize) -> Entity {
    let hero = world.seats[seat].unit.expect("hero");
    let rune = world.map.rune_spots[0].at;
    issue(
        world,
        seat,
        Order::Cheat {
            cheat: Cheat::Teleport { at: rune },
        },
    );
    issue(
        world,
        seat,
        Order::Cheat {
            cheat: Cheat::SpawnRune {
                kind: bota_proto::RuneKind::Illusion,
                at: rune,
            },
        },
    );
    for _ in 0..5 {
        if let Some(copy) = world
            .entities
            .iter()
            .find(|&unit| world.illusion.get(unit).is_some_and(|copy| copy.of == hero))
        {
            return copy;
        }
        world.step();
    }
    panic!("the cheat-spawned illusion rune never made its copies");
}

#[test]
fn ground_couriers_and_rune_illusions_obey_their_own_teams_at_all_four_base_fields() {
    for (centre, vertical, owner) in GATES {
        for team in [Team::Radiant, Team::Dire] {
            for illusion in [false, true] {
                let mut world = match_world();
                let seat = usize::from(team == Team::Dire);
                let unit = if illusion {
                    rune_copy(&mut world, seat)
                } else {
                    world.seats[seat].courier.expect("courier")
                };
                let across = if vertical {
                    Vec2::from_ints(70, 0)
                } else {
                    Vec2::from_ints(0, 70)
                };
                let start = centre - across;
                let finish = centre + across;
                issue_to(
                    &mut world,
                    seat,
                    Some(unit),
                    Order::Cheat {
                        cheat: Cheat::Teleport { at: start },
                    },
                );
                assert!(
                    !world.stats.get(unit).expect("stats").flies,
                    "ground fixture"
                );
                let mut previous = at(&world, unit);
                issue_to(
                    &mut world,
                    seat,
                    Some(unit),
                    Order::Move {
                        target: Target::Pos(finish),
                    },
                );
                let mut reached = false;
                for _ in 0..180 {
                    let next = at(&world, unit);
                    if team != owner {
                        assert_clear_field(
                            centre,
                            vertical,
                            previous,
                            next,
                            collision_radius(&world, unit),
                        );
                    }
                    reached |= next.within(finish, rules::units(10));
                    previous = next;
                    world.step();
                }
                if team == owner {
                    assert!(
                        reached,
                        "allied unitillusion={illusion} never crossed{centre:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn lane_creep_bodies_respect_team_fields_while_using_the_same_walk_planner() {
    for (centre, vertical, owner) in GATES {
        for team in [Team::Radiant, Team::Dire] {
            for def in [&bota_core::MELEE_CREEP, &bota_core::RANGED_CREEP] {
                let mut world = match_world();
                let across = if vertical {
                    Vec2::from_ints(70, 0)
                } else {
                    Vec2::from_ints(0, 70)
                };
                let unit = world.spawn_unit(def, team, centre - across);
                world.settle();
                world.set_order(
                    unit,
                    crate::game::UnitOrder::Move {
                        pos: centre + across,
                    },
                );
                let mut previous = at(&world, unit);
                let mut reached = false;
                for _ in 0..120 {
                    world.step();
                    let next = at(&world, unit);
                    if team != owner {
                        assert_clear_field(
                            centre,
                            vertical,
                            previous,
                            next,
                            collision_radius(&world, unit),
                        );
                    }
                    reached |= next.within(centre + across, rules::units(10));
                    previous = next;
                }
                if team == owner {
                    assert!(reached, "allied{:?} never crossed{centre:?}", def.kind);
                }
            }
        }
    }
}

#[test]
fn blink_crosses_base_fields_but_cannot_land_inside_an_enemy_field() {
    for (centre, vertical, owner) in GATES {
        for team in [Team::Radiant, Team::Dire] {
            for into_field in [false, true] {
                let mut world = match_world();
                let seat = usize::from(team == Team::Dire);
                let unit = world.seats[seat].unit.expect("hero");
                let across = if vertical {
                    Vec2::from_ints(70, 0)
                } else {
                    Vec2::from_ints(0, 70)
                };
                let target = if into_field { centre } else { centre + across };
                issue(
                    &mut world,
                    seat,
                    Order::Cheat {
                        cheat: Cheat::Teleport {
                            at: centre - across,
                        },
                    },
                );
                issue(
                    &mut world,
                    seat,
                    Order::Cheat {
                        cheat: Cheat::Item {
                            item: bota_proto::ItemId::BLINK_DAGGER,
                        },
                    },
                );
                issue(
                    &mut world,
                    seat,
                    Order::Use {
                        slot: bota_proto::ItemSlot(0),
                        target: Target::Pos(target),
                    },
                );
                for _ in 0..5 {
                    world.step();
                }
                let landed = at(&world, unit);
                if into_field && team != owner {
                    assert_landing_outside_field(&world, unit, centre, vertical);
                } else {
                    assert!(
                        landed.within(target, rules::units(2)),
                        "blink should reach{target:?}, got{landed:?}"
                    );
                }
            }
        }
    }
}

fn assert_landing_outside_field(world: &World, unit: Entity, centre: Vec2, vertical: bool) {
    let at = at(world, unit);
    let minimum = world.hull.get(unit).expect("a hull").collision + rules::units(24);
    for index in -7..=7 {
        let shift = if vertical {
            Vec2::from_ints(0, index * 25)
        } else {
            Vec2::from_ints(index * 25, 0)
        };
        assert!(
            at.distance_squared(centre + shift) >= minimum.squared_raw(),
            "landed hull overlaps field{centre:?}: {at:?}"
        );
    }
}

#[test]
fn flying_couriers_cross_enemy_fields_and_can_stop_over_them() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Levels {
                    count: rules::COURIER_FLYING_LEVEL - 1,
                },
            },
        );
        let courier = world.seats[seat].courier.expect("courier");
        assert!(
            world.stats.get(courier).expect("stats").flies,
            "the courier really flies"
        );
        let across = if vertical {
            Vec2::from_ints(70, 0)
        } else {
            Vec2::from_ints(0, 70)
        };
        issue_to(
            &mut world,
            seat,
            Some(courier),
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across,
                },
            },
        );
        for destination in [centre, centre + across] {
            issue_to(
                &mut world,
                seat,
                Some(courier),
                Order::Move {
                    target: Target::Pos(destination),
                },
            );
            for _ in 0..90 {
                world.step();
                if at(&world, courier).within(destination, rules::units(2)) {
                    break;
                }
            }
            assert!(
                at(&world, courier).within(destination, rules::units(2)),
                "flying courier failed at{destination:?}"
            );
        }
        assert_eq!(
            world.open_landing_for(courier, centre),
            Some(centre),
            "flight also permits a forced release over the field"
        );
    }
}

// Fixtures select the shared motion controller directly: the game currently
// has no Force Staff item. Walking orders and the tick phases still run normally.
fn begin_motion(world: &mut World, on: Entity, aim: Vec2, arc: bool) {
    let source = world
        .seats
        .iter()
        .filter_map(|seat| seat.unit)
        .find(|unit| *unit != on)
        .expect("source");
    let from = at(world, on);
    let phase = if arc {
        crate::game::PushPhase::Arc {
            from,
            started: world.tick,
            duration: 12,
            height: 100,
            tree_radius: 0,
            landing: None,
        }
    } else {
        crate::game::PushPhase::Ground
    };
    world.set_order(on, crate::game::UnitOrder::Stand);
    world.push.insert(
        on,
        crate::game::Push {
            source,
            inflictor: bota_proto::Inflictor::Ability(bota_core::NeutralAbility::Hurricane.id()),
            team: *world.team.get(source).expect("team"),
            aim,
            speed: rules::units(800),
            clear: from,
            effect: None,
            phase,
        },
    );
    world.forget_walk(on);
}

#[test]
fn shared_push_and_arc_controllers_cross_fields_but_clear_grounded_release_hulls() {
    for (centre, vertical, owner) in GATES {
        for team in [Team::Radiant, Team::Dire] {
            for arc in [false, true] {
                for inside in [false, true] {
                    let mut world = match_world();
                    let seat = usize::from(team == Team::Dire);
                    let unit = world.seats[seat].unit.expect("hero");
                    let across = if vertical {
                        Vec2::from_ints(70, 0)
                    } else {
                        Vec2::from_ints(0, 70)
                    };
                    let destination = if inside { centre } else { centre + across };
                    issue(
                        &mut world,
                        seat,
                        Order::Cheat {
                            cheat: Cheat::Teleport {
                                at: centre - across,
                            },
                        },
                    );
                    begin_motion(&mut world, unit, destination, arc);
                    let mut passed_field = false;
                    for _ in 0..90 {
                        world.step();
                        let point = at(&world, unit);
                        let gap = if vertical {
                            (point.x - centre.x).abs()
                        } else {
                            (point.y - centre.y).abs()
                        };
                        passed_field |= gap < rules::units(25);
                        if !world.push.contains(unit) {
                            break;
                        }
                    }
                    assert!(!world.push.contains(unit), "motion must finish");
                    assert!(passed_field, "forcedsegments may enter the field");
                    if team != owner && inside {
                        assert_landing_outside_field(&world, unit, centre, vertical);
                    } else {
                        assert!(
                            at(&world, unit).within(destination, rules::units(2)),
                            "forcedmotion did not reach{destination:?}"
                        );
                    }
                }
            }
        }
    }
}

fn hook_case(centre: Vec2, vertical: bool, owner: Team, into_field: bool) {
    use bota_proto::AbilitySlot;
    let shooter = usize::from(owner == Team::Dire);
    let victim = 1 - shooter;
    let mut heroes = [HeroId::SHADOW_FIEND; 2];
    heroes[shooter] = HeroId::PUDGE;
    let mut world = match_heroes(heroes, false);
    let across = if vertical {
        Vec2::from_ints(70, 0)
    } else {
        Vec2::from_ints(0, 70)
    };
    let home = if into_field { centre } else { centre - across };
    issue(
        &mut world,
        shooter,
        Order::Cheat {
            cheat: Cheat::Teleport { at: home },
        },
    );
    issue(
        &mut world,
        victim,
        Order::Cheat {
            cheat: Cheat::Teleport {
                at: centre + across,
            },
        },
    );
    issue(
        &mut world,
        shooter,
        Order::Learn {
            slot: AbilitySlot(0),
        },
    );
    issue(
        &mut world,
        shooter,
        Order::Cast {
            slot: AbilitySlot(0),
            target: Target::Pos(centre + across),
        },
    );
    let unit = world.seats[victim].unit.expect("victim");
    let mut caught = false;
    let mut crossed = false;
    let mut previous = at(&world, unit);
    for _ in 0..180 {
        world.step();
        caught |= world
            .entities
            .iter()
            .filter_map(|entity| world.hook.get(entity))
            .any(|hook| hook.caught == Some(unit));
        let point = at(&world, unit);
        crossed |=
            segment_distance_squared(centre, previous, point) < rules::units(25).squared_raw();
        previous = point;
        if caught && world.hook.is_empty() {
            break;
        }
    }
    assert!(caught, "the actual ability must latch onto its target");
    assert!(world.hook.is_empty(), "the hook must return");
    if !into_field {
        assert!(crossed, "the pull passes through field{centre:?}");
    }
    assert_landing_outside_field(&world, unit, centre, vertical);
    if !into_field {
        assert!(
            at(&world, unit).within(home, rules::units(20)),
            "the pull reaches the far side"
        );
    }
}

#[test]
fn actual_hook_pulls_cross_every_field_and_release_outside_enemy_field_hulls() {
    for (centre, vertical, owner) in GATES {
        for inside in [false, true] {
            hook_case(centre, vertical, owner, inside);
        }
    }
}

#[test]
fn actual_ranged_attacks_fire_and_damage_targets_across_every_base_field() {
    use bota_proto::EventKind;
    for (centre, vertical, _) in GATES {
        let mut world = match_heroes([HeroId::SHADOW_FIEND; 2], false);
        let across = if vertical {
            Vec2::from_ints(70, 0)
        } else {
            Vec2::from_ints(0, 70)
        };
        issue(
            &mut world,
            0,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across,
                },
            },
        );
        issue(
            &mut world,
            1,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre + across,
                },
            },
        );
        let shooter = world.seats[0].unit.expect("shooter");
        let target = world.seats[1].unit.expect("target");
        let shooter = crate::game::wire_id(shooter);
        let target = crate::game::wire_id(target);
        issue(
            &mut world,
            0,
            Order::Attack {
                target: Target::Unit(target),
            },
        );
        let mut fired = false;
        let mut damaged = false;
        for _ in 0..120 {
            let events = world.step();
            fired |= world
                .view_full()
                .projectiles
                .iter()
                .any(|shot| shot.source == Some(shooter) && shot.target == Some(target));
            damaged|=events.iter().any(|event|matches!(event.kind(),EventKind::Damaged {source:Some(source),target:hit,..} if source==shooter&&hit==target));
            if fired && damaged {
                break;
            }
        }
        assert!(fired, "a real rangedattack projectile must cross the field");
        assert!(
            damaged,
            "the projectile must hit its target across the field"
        );
    }
}

#[test]
fn neutral_skeletons_and_tree_walkers_respect_both_teams_base_fields() {
    const TREE_WALKER: bota_core::UnitDef = bota_core::UnitDef {
        tree_walking: true,
        ..bota_core::MELEE_CREEP
    };
    let kinds = [
        &bota_core::MELEE_CREEP,
        bota_core::NeutralKind::SkeletonWarrior.def(),
        &TREE_WALKER,
    ];
    for (centre, vertical, _) in GATES {
        for def in kinds {
            let mut world = World::on_map(map_of(MapId(0)));
            let across = if vertical {
                Vec2::from_ints(70, 0)
            } else {
                Vec2::from_ints(0, 70)
            };
            let unit = world.spawn_unit(def, Team::Neutral, centre - across);
            world.settle();
            world.set_order(
                unit,
                crate::game::UnitOrder::Move {
                    pos: centre + across,
                },
            );
            let mut previous = at(&world, unit);
            for _ in 0..120 {
                world.step();
                let next = at(&world, unit);
                assert_clear_field(
                    centre,
                    vertical,
                    previous,
                    next,
                    collision_radius(&world, unit),
                );
                previous = next;
            }
        }
    }
}

#[test]
fn actual_town_portal_channels_jump_across_enemy_fields_to_their_own_fountain() {
    use bota_proto::{ItemId, ItemSlot};
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        let team = world.seats[seat].team;
        let unit = world.seats[seat].unit.expect("hero");
        let destination = bota_core::hero_spawn_pos(world.map, team);
        let sign = if vertical {
            (destination.x - centre.x).raw.signum()
        } else {
            (destination.y - centre.y).raw.signum()
        };
        let start = centre
            + if vertical {
                Vec2::from_ints(-70 * sign, 0)
            } else {
                Vec2::from_ints(0, -70 * sign)
            };
        assert!(
            (-7..=7).any(|index| {
                let offset = if vertical {
                    Vec2::from_ints(0, index * 25)
                } else {
                    Vec2::from_ints(index * 25, 0)
                };
                segment_distance_squared(centre + offset, start, destination)
                    < rules::units(25).squared_raw()
            }),
            "the teleport fixture must span the gate field"
        );
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::Teleport { at: start },
            },
        );
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
                target: Target::None,
            },
        );
        let mut channelled = false;
        for _ in 0..180 {
            world.step();
            channelled |= world.is_channelling(unit);
            if at(&world, unit).within(destination, rules::units(10)) {
                break;
            }
        }
        assert!(channelled, "a real scroll channel must begin");
        assert!(
            at(&world, unit).within(destination, rules::units(10)),
            "the field must not stop the scroll's jump"
        );
    }
}

#[test]
fn body_overlap_corrections_never_push_an_enemy_hull_inside_a_base_field() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let enemy = usize::from(owner == Team::Radiant);
        let ally = 1 - enemy;
        let unit = world.seats[enemy].unit.expect("enemy hero");
        let across = if vertical {
            Vec2::from_ints(1, 0)
        } else {
            Vec2::from_ints(0, 1)
        };
        issue(
            &mut world,
            enemy,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across * rules::units(54),
                },
            },
        );
        issue(
            &mut world,
            enemy,
            Order::Move {
                target: Target::None,
            },
        );
        issue(
            &mut world,
            ally,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across * rules::units(70),
                },
            },
        );
        let collider = world.seats[ally].unit.expect("other hero");
        assert!(
            at(&world, unit).within(at(&world, collider), rules::units(54)),
            "the fixture must have overlapping hulls"
        );
        for _ in 0..30 {
            world.step();
            assert_landing_outside_field(&world, unit, centre, vertical);
        }
    }
}

#[test]
fn fields_remain_team_conditional_after_glyph_building_loss_and_late_night() {
    use bota_proto::UnitKind;
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        issue(
            &mut world,
            0,
            Order::Cheat {
                cheat: Cheat::Refresh,
            },
        );
        issue(&mut world, 0, Order::Glyph);
        issue(&mut world, 1, Order::Glyph);
        let buildings: Vec<_> = world
            .entities
            .iter()
            .filter(|&unit| {
                matches!(
                    world.kind.get(unit),
                    Some(UnitKind::Tower | UnitKind::Barracks)
                )
            })
            .collect();
        for unit in buildings {
            world.despawn(unit);
        }
        world.clock_shift.game = world.schedule().at_minute(60).saturating_sub(world.tick);
        issue(
            &mut world,
            0,
            Order::Cheat {
                cheat: Cheat::TimeOfDay { night: true },
            },
        );
        let enemy = usize::from(owner == Team::Radiant);
        let unit = world.seats[enemy].unit.expect("hero");
        let across = if vertical {
            Vec2::from_ints(70, 0)
        } else {
            Vec2::from_ints(0, 70)
        };
        issue(
            &mut world,
            enemy,
            Order::Cheat {
                cheat: Cheat::Teleport {
                    at: centre - across,
                },
            },
        );
        let mut previous = at(&world, unit);
        issue(
            &mut world,
            enemy,
            Order::Move {
                target: Target::Pos(centre + across),
            },
        );
        for _ in 0..120 {
            let next = at(&world, unit);
            assert_clear_field(
                centre,
                vertical,
                previous,
                next,
                collision_radius(&world, unit),
            );
            previous = next;
            world.step();
        }
    }
}

mod placements;

mod channels;
mod routing;

mod property;

mod resolution;
