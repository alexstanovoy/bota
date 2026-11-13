//! Placement regressions: new bodies are checked where they are created.
use super::*;
use bota_proto::AbilitySlot;

pub(super) fn perpendicular(vertical: bool, units: i32) -> Vec2 {
    if vertical {
        Vec2::from_ints(units, 0)
    } else {
        Vec2::from_ints(0, units)
    }
}

#[test]
fn rune_copies_spawn_with_clear_hulls_beside_every_enemy_base_field() {
    for (centre, vertical, owner) in GATES {
        for approach in [-1, 1] {
            let mut world = match_world();
            let seat = usize::from(owner == Team::Radiant);
            let hero = world.seats[seat].unit.expect("hero");
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport {
                        at: centre + perpendicular(vertical, 60 * approach),
                    },
                },
            );
            world.make_rune_illusions(hero);
            let copies: Vec<_> = world
                .entities
                .iter()
                .filter(|&unit| world.illusion.get(unit).is_some_and(|copy| copy.of == hero))
                .collect();
            assert_eq!(copies.len(), bota_core::ILLUSION_RUNE_COUNT);
            for copy in copies {
                assert_landing_outside_field(&world, copy, centre, vertical);
            }
            assert_landing_outside_field(&world, hero, centre, vertical);
        }
    }
}

#[test]
fn teleport_cheats_place_grounded_hulls_outside_enemy_base_fields() {
    for (centre, vertical, owner) in GATES {
        for team in [Team::Radiant, Team::Dire] {
            let mut world = match_world();
            let seat = usize::from(team == Team::Dire);
            let unit = world.seats[seat].unit.expect("hero");
            issue(
                &mut world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport { at: centre },
                },
            );
            if team == owner {
                assert!(at(&world, unit).within(centre, rules::units(1)));
            } else {
                assert_landing_outside_field(&world, unit, centre, vertical);
            }
        }
    }
}

#[test]
fn created_heroes_start_with_clear_hulls_beside_every_enemy_base_field() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        issue(
            &mut world,
            seat,
            Order::Cheat {
                cheat: Cheat::CreateHero {
                    hero: HeroId::SHADOW_FIEND,
                    enemy: false,
                    at: centre,
                },
            },
        );
        let unit = world
            .seats
            .last()
            .and_then(|seat| seat.unit)
            .expect("created hero");
        assert_landing_outside_field(&world, unit, centre, vertical);
    }
}

#[test]
fn raised_skeletons_start_with_clear_hulls_beside_both_teams_fields() {
    use bota_core::{NeutralAbility, NeutralKind};
    for (centre, vertical, _) in GATES {
        let mut world = match_world();
        let caster = world.spawn_neutral(
            NeutralKind::DarkTrollWarlord,
            centre - perpendicular(vertical, 60),
            0,
            0,
        );
        world.owner.insert(caster, world.seats[0].slot);
        world.settle();
        let slot = world
            .abilities
            .get(caster)
            .expect("abilities")
            .slots
            .iter()
            .position(|slot| slot.id == NeutralAbility::RaiseDead.id())
            .expect("Raise Dead");
        issue_to(
            &mut world,
            0,
            Some(caster),
            Order::Cast {
                slot: AbilitySlot(slot as u8),
                target: Target::None,
            },
        );
        let mut skeletons = Vec::new();
        for _ in 0..60 {
            world.step();
            skeletons = world
                .entities
                .iter()
                .filter(|&unit| world.neutral.get(unit) == Some(&NeutralKind::SkeletonWarrior))
                .collect();
            if !skeletons.is_empty() {
                break;
            }
        }
        assert!(!skeletons.is_empty(), "the real summon must create bodies");
        for unit in skeletons {
            assert_landing_outside_field(&world, unit, centre, vertical);
        }
    }
}

#[test]
fn returning_heroes_and_couriers_clear_enemy_base_field_hulls() {
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let seat = usize::from(owner == Team::Radiant);
        let hero = world.seats[seat].unit.expect("hero");
        let courier = world.seats[seat].courier.expect("courier");
        world.lay_hero_down(seat, 30);
        world.raise_fallen_hero(seat, centre);
        assert_landing_outside_field(&world, hero, centre, vertical);
        world.lay_courier_down(seat, 30);
        world.raise_fallen_courier(seat, centre);
        assert_landing_outside_field(&world, courier, centre, vertical);
    }
}

// An explicit capability fixture: no ability or modifier is fabricated.
#[test]
fn pathing_only_flight_crosses_fields_but_retains_grounded_vision() {
    const PATHING_FLIER: bota_core::UnitDef = bota_core::UnitDef {
        pathing_flight: true,
        flies: false,
        ..bota_core::MELEE_CREEP
    };
    for (centre, vertical, owner) in GATES {
        let mut world = match_world();
        let team = if owner == Team::Radiant {
            Team::Dire
        } else {
            Team::Radiant
        };
        let unit = world.spawn_unit(&PATHING_FLIER, team, centre - perpendicular(vertical, 70));
        world.settle();
        assert!(
            !world.stats.get(unit).expect("stats").flies,
            "vision must remain grounded"
        );
        world.set_order(
            unit,
            crate::game::UnitOrder::Move {
                pos: centre + perpendicular(vertical, 70),
            },
        );
        for _ in 0..120 {
            world.step();
            if at(&world, unit).within(centre + perpendicular(vertical, 70), rules::units(2)) {
                break;
            }
        }
        assert!(
            at(&world, unit).within(centre + perpendicular(vertical, 70), rules::units(2)),
            "pathing-only flight must cross"
        );
        assert_eq!(world.open_landing_for(unit, centre), Some(centre));
        let source = at(&world, unit);
        let target = source + Vec2::from_ints(256, 0);
        let middle = source + Vec2::from_ints(128, 0);
        let mut walls = bota_core::CellGrid::open();
        let (x, y) = bota_core::CellGrid::cell_of(middle).expect("wall cell");
        for row in y.saturating_sub(2)..=y + 2 {
            walls.close_cell(x, row);
        }
        world.sight_block = bota_core::SightBlock::lay(
            &|at| world.ground.tier(at),
            &walls,
            std::iter::empty(),
            std::iter::empty(),
        );
        world.settle();
        assert!(
            !world.can_see_point(team, target),
            "pathing flight must not see through the wall"
        );
        const TRUE_FLIER: bota_core::UnitDef = bota_core::UnitDef {
            flies: true,
            ..bota_core::MELEE_CREEP
        };
        world.spawn_unit(&TRUE_FLIER, team, source);
        world.settle();
        assert!(
            world.can_see_point(team, target),
            "true flying vision is the positive control"
        );
    }
}
