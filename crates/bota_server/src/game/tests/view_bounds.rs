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

/// Reach a real attack windup or flight without projecting intermediate frames.
fn a_ground_attack(flying: bool, discarded: bool) -> (crate::game::World, crate::game::Entity, crate::game::Entity) {
    use bota_proto::{ItemId, Order, Target, Vec2};
    use crate::game::{ActionState, ItemStack};
    let (mut world, hero) = match_hero();
    world.creeps_halted = true;
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    let at = spot + Vec2::from_ints(350, 0);
    if discarded {
        for _ in 0..bota_proto::limits::MAX_SNAPSHOT_LOOT {
            world.lay_loot(ItemStack::dropped(ItemId::CHEESE).expect("cheese"), at);
        }
    }
    let target = world.lay_loot(ItemStack::dropped(ItemId::AEGIS).expect("aegis"), at);
    world.settle();
    seat_order(&mut world, Order::Attack { target: Target::Unit(wire_id(target)) });
    let mut reached = false;
    for _ in 0..2 * bota_core::rules::TICKS_PER_SECOND {
        let in_flight = world.entities.iter().any(|entity| world.projectile.get(entity)
            .is_some_and(|shot| shot.source == Some(hero) && shot.target == target));
        let winding = world.action.get(hero).is_some_and(|action| matches!(action.state,
            ActionState::Attack { target: on, phase: crate::game::ActionPhase::Before { .. } } if on == target));
        if if flying { in_flight } else { winding } {
            reached = true;
            break;
        }
        world.step();
    }
    assert!(reached, "the real order reaches its requested attack phase");
    (world, hero, target)
}

fn assert_ground_target(view: &bota_proto::WorldView, hero: crate::game::Entity, target: crate::game::Entity, flying: bool) {
    let target = wire_id(target);
    assert!(view.loot.iter().any(|loot| loot.id == target), "ground target retained");
    assert!(view.units.iter().all(|unit| unit.id != target), "loot is not a unit row");
    let aggro = view.aggro.iter().find(|entry| entry.unit == wire_id(hero)).expect("attacker");
    assert_eq!(aggro.target, Some(target));
    if flying {
        assert!(view.projectiles.iter().any(|shot| shot.source == Some(wire_id(hero)) && shot.target == Some(target)), "flight keeps its ground target");
    } else {
        assert_eq!(aggro.swing.as_ref().map(|swing| swing.target), Some(target));
    }
    let statics = bota_client_core::MatchStatic::new(&config().info()).expect("match metadata");
    let mut client = bota_client_core::ClientSession::spectator(statics,
        bota_client_core::SessionOptions { ack: bota_client_core::AckPolicy::Never,
            ..bota_client_core::SessionOptions::default() });
    client.ingest_snapshot(view.clone(), &mut ()).expect("client accepts the ground attack snapshot");
}

#[test]
fn ground_item_attack_snapshots_keep_the_real_order_target_in_windup_and_flight() {
    for flying in [false, true] {
        let (world, hero, target) = a_ground_attack(flying, false);
        for view in [world.view_full(), world.view(Team::Radiant)] {
            assert_ground_target(&view, hero, target, flying);
        }
    }
}

#[test]
fn ground_item_attack_snapshots_keep_loot_targets_when_unit_rows_overflow() {
    for flying in [false, true] {
        let (mut world, hero, target) = a_ground_attack(flying, false);
        let spot = world.pos(hero).expect("hero");
        for _ in 0..=MAX_VIEW_UNITS {
            world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, spot);
        }
        world.settle();
        let view = world.view_full();
        assert_eq!(view.units.len(), MAX_VIEW_UNITS);
        assert_ground_target(&view, hero, target, flying);
    }
}

#[test]
fn ground_item_attack_snapshots_remove_references_to_loot_discarded_by_the_bound() {
    for flying in [false, true] {
        let (world, hero, target) = a_ground_attack(flying, true);
        let view = world.view_full();
        assert_eq!(view.loot.len(), bota_proto::limits::MAX_SNAPSHOT_LOOT);
        assert!(view.loot.iter().all(|loot| loot.id != wire_id(target)));
        let aggro = view.aggro.iter().find(|entry| entry.unit == wire_id(hero)).expect("attacker");
        assert_eq!(aggro.target, None);
        assert_eq!(aggro.held_for, None);
        assert_eq!(aggro.swing, None);
        assert!(view.projectiles.iter().all(|shot| shot.target != Some(wire_id(target))), "discarded target cannot leave a dangling flight");
    }
}

#[test]
fn ground_item_attack_snapshots_keep_retained_targets_when_other_loot_overflows() {
    use bota_proto::ItemId;
    for flying in [false, true] {
        let (mut world, hero, target) = a_ground_attack(flying, false);
        let at = world.pos(target).expect("loot");
        for _ in 0..bota_proto::limits::MAX_SNAPSHOT_LOOT {
            world.lay_loot(crate::game::ItemStack::dropped(ItemId::CHEESE).expect("cheese"), at);
        }
        let view = world.view_full();
        assert_eq!(view.loot.len(), bota_proto::limits::MAX_SNAPSHOT_LOOT);
        assert_ground_target(&view, hero, target, flying);
    }
}
