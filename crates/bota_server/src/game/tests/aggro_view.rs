//! What a side is told of whom units fight, read back off the wire.

use bota_proto::{
    AggroView, EventKind, FrameReader, ServerMsg, Team, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Entity, MELEE_CREEP, World, rules, wire_id};

use super::support::*;

/// The view a viewer is sent, as a client decodes it.
fn received(world: &World, viewer: Option<Team>) -> WorldView {
    let view = match viewer {
        Some(team) => world.view(team),
        None => world.view_full(),
    };
    let frame = encode_frame_to_vec(&ServerMsg::Snapshot { view }).expect("encode");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    match reader.next_message::<ServerMsg>() {
        Ok(Some(ServerMsg::Snapshot { view })) => view,
        other => panic!("a snapshot comes back, not {other:?}"),
    }
}

/// Steps the world one tick and puts a creep back where the lane fixture
/// stood it, so only the clock decides what it does.
fn step_held(world: &mut World, creep: Entity) {
    world.step();
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos = Vec2::from_ints(5300, 5000);
    }
}

/// What a view says of one unit's fighting, if it says anything.
fn told_of(view: &WorldView, unit: Entity) -> Option<AggroView> {
    view.aggro
        .iter()
        .find(|entry| entry.unit == wire_id(unit))
        .copied()
}

#[test]
fn a_draw_is_told_to_both_sides_and_runs_out_on_the_wire() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    for side in [Team::Radiant, Team::Dire] {
        let entry = told_of(&received(&world, Some(side)), theirs).expect("it fights");
        assert_eq!(
            entry.target,
            Some(wire_id(ours)),
            "the creep fights the creep"
        );
        assert_eq!(entry.hold_left, 0, "nothing has drawn it");
        assert_eq!(entry.draw_cooldown, 0, "the next click draws it");
    }
    attack_click(&mut world, foe);
    let drawn = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("it fights");
    assert_eq!(drawn.target, Some(wire_id(hero)), "the click draws it");
    assert_eq!(
        Some(drawn),
        told_of(&received(&world, Some(Team::Dire)), theirs),
        "both sides see the same creep fight the same hero"
    );
    assert!(0 < drawn.hold_left && drawn.hold_left <= rules::ORDER_AGGRO_HOLD_TICKS);
    assert_eq!(drawn.draw_cooldown, 0, "a creep keeps no wait of its own");
    let wait = |world: &World, side| {
        told_of(&received(world, Some(side)), hero).map_or(0, |entry| entry.draw_cooldown)
    };
    assert!(drawn.hold_left < wait(&world, Team::Radiant));
    assert!(wait(&world, Team::Radiant) <= rules::ORDER_AGGRO_COOLDOWN_TICKS);
    assert_eq!(
        wait(&world, Team::Dire),
        0,
        "the hero's wait is its own side's"
    );
    stop_click(&mut world);
    let mut last = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("held");
    while last.hold_left > 0 {
        let waited = wait(&world, Team::Radiant);
        assert_eq!(last.target, Some(wire_id(hero)), "the hold keeps the hero");
        step_held(&mut world, theirs);
        let now = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("held");
        assert_eq!(
            now.hold_left,
            last.hold_left - 1,
            "the hold counts down a tick a tick"
        );
        assert_eq!(
            wait(&world, Team::Radiant),
            waited - 1,
            "so does the hero's wait"
        );
        last = now;
    }
    step_held(&mut world, theirs);
    let let_go = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("it fights");
    assert_eq!(
        let_go.target,
        Some(wire_id(ours)),
        "the tick after the hold runs out, the ranking takes the creep back"
    );
}

#[test]
fn a_swing_strikes_on_the_tick_the_wire_said() {
    let (mut world, _hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    let mut ticks = 0;
    let swing = loop {
        let told = told_of(&received(&world, Some(Team::Radiant)), theirs);
        if let Some(swing) = told.and_then(|entry| entry.swing) {
            break swing;
        }
        assert!(ticks < 90, "a creep in reach swings within three seconds");
        world.step();
        ticks += 1;
    };
    assert_eq!(swing.target, wire_id(ours));
    for left in (1..swing.strike_in).rev() {
        world.step();
        assert_eq!(swinging(&world, theirs), Some(ours), "still in the windup");
        let told = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("it fights");
        assert_eq!(told.swing.map(|swing| swing.strike_in), Some(left));
    }
    let struck = world.step().into_iter().any(|event| {
        matches!(
            event.kind,
            EventKind::Damaged { source: Some(by), target, .. }
                if by == wire_id(theirs) && target == wire_id(ours)
        )
    });
    assert!(struck, "the blow lands on the tick it was due");
}

#[test]
fn a_target_in_the_fog_is_never_named() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, Vec2::from_ints(5000, 5000));
    let prey = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.step();
    assert_eq!(world.target_of(creep), Some(prey));
    // Carried out of everything Radiant sees; the creep chases it.
    if let Some(at) = world.transform.get_mut(prey) {
        at.pos = Vec2::from_ints(5900, 5000);
    }
    world.step();
    assert_eq!(world.target_of(creep), Some(prey), "the chase goes on");
    let radiant = received(&world, Some(Team::Radiant));
    assert!(!radiant.units.iter().any(|unit| unit.id == wire_id(prey)));
    let entry = told_of(&radiant, creep).expect("a chase is told");
    assert_eq!(entry.target, None, "whom it chases is not");
    assert!(entry.chase_left > 0);
    let full = received(&world, None);
    assert_eq!(
        told_of(&full, creep).and_then(|entry| entry.target),
        Some(wire_id(prey)),
        "a view of everything names it"
    );
}
