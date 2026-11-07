//! Attack-point facts follow the attacker's visibility, independently of the victim's.

use super::support::{advance_validated, seated_hero};
use crate::game::{Entity, Event, World, wire_id};
use bota_core::{MELEE_CREEP, UnitDef};
use bota_proto::{
    EventKind, FrameReader, HeroId, Order, ServerMsg, Target, Team, Vec2, encode_frame_to_vec,
};

const OBSERVER: UnitDef = UnitDef {
    max_hp: 30_000,
    damage: 0,
    acquisition: 0,
    move_speed: 0,
    day_vision: 1000,
    night_vision: 1000,
    ..MELEE_CREEP
};
const VICTIM: UnitDef = UnitDef {
    day_vision: 0,
    night_vision: 0,
    ..OBSERVER
};
const NEAR_OBSERVER: UnitDef = UnitDef {
    day_vision: 200,
    night_vision: 200,
    ..OBSERVER
};

fn launched(hero: HeroId, watcher: Vec2) -> (World, Entity, Entity, Vec<Event>) {
    launched_with_observer(hero, watcher, &OBSERVER)
}

fn launched_with_observer(
    hero: HeroId,
    watcher: Vec2,
    observer: &'static UnitDef,
) -> (World, Entity, Entity, Vec<Event>) {
    let mut world = World::new();
    let attacker = seated_hero(&mut world, Vec2::from_ints(1000, 1000), hero);
    let position = if hero == HeroId::PUDGE {
        Vec2::from_ints(1100, 1000)
    } else {
        Vec2::from_ints(1500, 1000)
    };
    let target = world.spawn_unit(&VICTIM, Team::Neutral, position);
    world.spawn_unit(observer, Team::Dire, watcher);
    world.settle();
    let mut events = advance_validated(
        &mut world,
        None,
        Order::Attack {
            target: Target::Unit(wire_id(target)),
        },
    );
    for _ in 0..60 {
        if events.iter().any(|event| is_launch(&event.kind())) {
            return (world, attacker, target, events);
        }
        events = world.step();
    }
    panic!("an attack reaching its attack point must emit AttackLaunched");
}

fn is_launch(event: &EventKind) -> bool {
    matches!(event, EventKind::AttackLaunched { .. })
}

fn received(world: &World, events: &[Event], viewer: Option<Team>) -> Vec<EventKind> {
    let events = crate::game_loop::visible_events(events, viewer);
    let frame = encode_frame_to_vec(&ServerMsg::Events {
        tick: world.tick,
        events,
    })
    .expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    match reader.next_message::<ServerMsg>().expect("decode") {
        Some(ServerMsg::Events { tick, events }) => {
            assert_eq!(tick, world.tick);
            events.into_iter().filter(is_launch).collect()
        }
        other => panic!("expected event frame, got {other:?}"),
    }
}

#[test]
fn ranged_and_melee_attack_points_are_told_to_the_attackers_side() {
    for hero in [HeroId::SHADOW_FIEND, HeroId::PUDGE] {
        let (world, attacker, target, events) = launched(hero, Vec2::from_ints(5000, 5000));
        assert_eq!(
            received(&world, &events, Some(Team::Radiant)),
            vec![EventKind::AttackLaunched {
                attacker: wire_id(attacker),
                target: Some(wire_id(target))
            }]
        );
    }
}

#[test]
fn a_side_that_cannot_see_the_attacker_receives_no_launch_fact() {
    let (world, attacker, _, events) = launched(HeroId::SHADOW_FIEND, Vec2::from_ints(5000, 5000));
    assert!(!world.can_see(Team::Dire, attacker));
    assert!(received(&world, &events, Some(Team::Dire)).is_empty());
}

#[test]
fn a_side_that_sees_the_attacker_receives_the_launch_fact() {
    let (world, attacker, _, events) = launched(HeroId::SHADOW_FIEND, Vec2::from_ints(1000, 1500));
    assert!(world.can_see(Team::Dire, attacker));
    assert_eq!(received(&world, &events, Some(Team::Dire)).len(), 1);
}

#[test]
fn a_full_spectator_or_replay_receives_the_launch_fact() {
    let (world, attacker, target, events) =
        launched(HeroId::SHADOW_FIEND, Vec2::from_ints(5000, 5000));
    assert_eq!(
        received(&world, &events, None),
        vec![EventKind::AttackLaunched {
            attacker: wire_id(attacker),
            target: Some(wire_id(target))
        }]
    );
}

#[test]
fn attacker_visibility_does_not_reveal_a_target_hidden_from_the_viewer() {
    let (world, attacker, target, events) = launched_with_observer(
        HeroId::SHADOW_FIEND,
        Vec2::from_ints(900, 1000),
        &NEAR_OBSERVER,
    );
    assert!(world.can_see(Team::Dire, attacker));
    assert!(!world.can_see(Team::Dire, target));
    assert_eq!(
        received(&world, &events, Some(Team::Dire)),
        vec![EventKind::AttackLaunched {
            attacker: wire_id(attacker),
            target: None
        }]
    );
}
