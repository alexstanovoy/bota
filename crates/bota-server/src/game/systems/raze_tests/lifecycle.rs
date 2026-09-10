use bota_proto::{HeroId, SlotId, Team};

use crate::game::{Seat, rules, wire_id};

use super::fixtures::*;

#[test]
fn raze_already_queued_before_caster_death_keeps_its_damage_and_old_generation_effect() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.health.get_mut(caster).unwrap().hp = bota_proto::Fixed::ONE;
    world.push_hit(None, caster, 1, bota_proto::DamageKind::Pure);
    reveal(&mut world, target);
    assert!(world.cast_raze(caster, 0, 1));
    assert_eq!(damage(&world.step(), target), vec![140]);
    assert!(!world.alive(caster));
    assert_eq!(effects(&world, target), vec![effect(2, 240)]);
    let returned = hero(&mut world, Team::Radiant, ORIGIN, SlotId(0));
    assert_eq!(caster.index(), returned.index());
    assert_eq!(land(&mut world, returned, target, 0), 90);
}

#[test]
fn raze_target_death_and_respawn_do_not_restore_timed_stacks() {
    let (mut world, caster, target) = fixture();
    let mut seat = Seat::new(SlotId(1), Team::Dire, HeroId(2), 0, rules::STASH_SLOTS);
    seat.unit = Some(target);
    world.seats.push(seat);
    land(&mut world, caster, target, 0);
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
    world.bury(vec![(target, None)], &mut Vec::new());
    world.seats[0].respawn_left = 1;
    world.tick_respawns();
    let returned = world.seats[0].unit.unwrap();
    assert_eq!(target.index(), returned.index());
    assert_ne!(target.generation(), returned.generation());
    prepare(&mut world, returned, TARGET);
    assert!(effects(&world, returned).is_empty());
    assert_eq!(land(&mut world, caster, returned, 0), 90);
}

#[test]
fn raze_reused_caster_index_does_not_inherit_the_previous_generations_bonus() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
    assert!(world.despawn(caster));
    let returned = hero(&mut world, Team::Radiant, ORIGIN, SlotId(0));
    assert_eq!(caster.index(), returned.index());
    assert_ne!(caster.generation(), returned.generation());
    assert_eq!(land(&mut world, returned, target, 0), 90);
    assert_eq!(land(&mut world, returned, target, 0), 140);
}

#[test]
fn raze_caster_death_and_respawn_cannot_use_its_old_bodys_stacks() {
    let (mut world, caster, target) = fixture();
    let mut seat = Seat::new(SlotId(0), Team::Radiant, HeroId(2), 0, rules::STASH_SLOTS);
    seat.unit = Some(caster);
    world.seats.push(seat);
    land(&mut world, caster, target, 0);
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
    world.bury(vec![(caster, None)], &mut Vec::new());
    world.seats[0].respawn_left = 1;
    world.tick_respawns();
    let returned = world.seats[0].unit.unwrap();
    assert_ne!(caster, returned);
    prepare(&mut world, returned, ORIGIN);
    assert_eq!(land(&mut world, returned, target, 0), 90);
}

#[test]
fn raze_projection_exposes_only_existing_effect_fields_on_visible_targets() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.visibility.get_mut(caster).unwrap().clear();
    let visible = world.view(Team::Dire);
    assert!(!visible.units.iter().any(|unit| unit.id == wire_id(caster)));
    let shown = visible
        .units
        .iter()
        .find(|unit| unit.id == wire_id(target))
        .unwrap();
    assert_eq!(shown.effects, vec![effect(1, 240)]);
    world.visibility.get_mut(target).unwrap().clear();
    assert!(
        !world
            .view(Team::Radiant)
            .units
            .iter()
            .any(|unit| unit.id == wire_id(target))
    );
}

#[test]
fn raze_source_records_have_a_fixed_16_source_bound_with_oldest_expiry_eviction() {
    let (mut world, first, target) = fixture();
    land(&mut world, first, target, 0);
    let mut second = None;
    for index in 0..16 {
        let caster = hero(&mut world, Team::Radiant, ORIGIN, SlotId(index + 2));
        second = second.or(Some(caster));
        assert_eq!(land(&mut world, caster, target, 0), 90);
    }
    assert_eq!(effects(&world, target).len(), 16);
    assert_eq!(land(&mut world, second.unwrap(), target, 0), 140);
    assert_eq!(land(&mut world, first, target, 0), 90);
    assert_eq!(effects(&world, target).len(), 16);
}
