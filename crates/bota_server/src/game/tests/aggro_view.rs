//! What a side is told of whom units fight, read back off the wire.

use bota_core::{MELEE_CREEP, rules};
use bota_proto::{
    AggroView, EventKind, FrameReader, ServerMsg, Team, Vec2, WorldView, encode_frame_to_vec,
};

use crate::game::{Entity, World, wire_id};

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
        assert_eq!(entry.held_for, Some(0), "nothing has drawn it");
        assert_eq!(entry.draw_wait, Some(0), "the next click draws it");
    }
    attack_click(&mut world, foe);
    let drawn = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("it fights");
    assert_eq!(drawn.target, Some(wire_id(hero)), "the click draws it");
    assert_eq!(
        Some(drawn),
        told_of(&received(&world, Some(Team::Dire)), theirs),
        "both sides see the same creep fight the same hero"
    );
    let held = drawn.held_for.expect("a hold on a hero in view is told");
    assert!(0 < held && held <= rules::ORDER_AGGRO_HOLD_TICKS);
    assert_eq!(drawn.draw_wait, Some(0), "a creep keeps no wait of its own");
    let wait = |world: &World, side| {
        told_of(&received(world, Some(side)), hero).and_then(|entry| entry.draw_wait)
    };
    assert!(Some(held) < wait(&world, Team::Radiant));
    assert!(wait(&world, Team::Radiant) <= Some(rules::ORDER_AGGRO_COOLDOWN_TICKS));
    assert_eq!(
        wait(&world, Team::Dire),
        None,
        "the hero's wait is its own side's: withheld from the other"
    );
    stop_click(&mut world);
    let mut last = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("held");
    while last.held_for > Some(0) {
        let waited = wait(&world, Team::Radiant).expect("its own side is told");
        assert_eq!(last.target, Some(wire_id(hero)), "the hold keeps the hero");
        step_held(&mut world, theirs);
        let now = told_of(&received(&world, Some(Team::Radiant)), theirs).expect("held");
        assert_eq!(
            now.held_for,
            last.held_for.map(|held| held - 1),
            "the hold counts down a tick a tick"
        );
        assert_eq!(
            wait(&world, Team::Radiant),
            Some(waited - 1),
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
            event.kind(),
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
    assert_eq!(
        told_of(&radiant, creep),
        None,
        "nothing of a chase after what the side cannot see is told, nor how long it goes on"
    );
    let full = received(&world, None);
    assert_eq!(
        told_of(&full, creep).and_then(|entry| entry.target),
        Some(wire_id(prey)),
        "a view of everything names it"
    );
}

#[test]
fn phase_observations_ignore_hidden_target_properties_for_every_stage() {
    use crate::game::{Action, ActionPhase, ActionState};
    use bota_proto::{AbilityId, AbilitySlot, HeroId, ItemSlot, Target};
    const EYE: bota_core::UnitDef = bota_core::UnitDef {
        damage: 0,
        acquisition: 0,
        move_speed: 0,
        day_vision: 200,
        night_vision: 200,
        ..MELEE_CREEP
    };
    let mut world = World::new();
    let actor = seated_hero(&mut world, Vec2::from_ints(1000, 1000), HeroId::PUDGE);
    let target = world.spawn_unit(&MELEE_CREEP, Team::Neutral, Vec2::from_ints(1500, 1000));
    world.spawn_unit(&EYE, Team::Dire, Vec2::from_ints(1000, 1150));
    world.settle();
    let slot = AbilitySlot(
        world
            .abilities
            .get(actor)
            .expect("book")
            .slots
            .iter()
            .position(|slot| slot.id == AbilityId::DISMEMBER)
            .expect("slot") as u8,
    );
    let at = Target::Unit(wire_id(target));
    for phase in [
        ActionPhase::Before { progress: 1000 },
        ActionPhase::During { progress: 1000 },
        ActionPhase::After { progress: 1000 },
    ] {
        for state in [
            ActionState::Ready,
            ActionState::Attack { target, phase },
            ActionState::CastAbility {
                slot,
                target: at,
                phase,
            },
        ] {
            world.action.insert(
                actor,
                Action {
                    state,
                    attack_cooldown: 0,
                },
            );
            assert!(world.shows(Team::Dire, actor));
            assert!(!world.shows(Team::Dire, target));
            let before = encode_frame_to_vec(&ServerMsg::Snapshot {
                view: world.view(Team::Dire),
            })
            .expect("frame");
            for change in 0..3 {
                let mut changed = world.clone();
                match change {
                    0 => {
                        changed
                            .stats
                            .get_mut(target)
                            .expect("stats")
                            .status_resist_bp = 5000
                    }
                    1 => {
                        changed.kind.insert(target, bota_proto::UnitKind::Hero);
                        changed.hero.insert(target, HeroId::SHADOW_FIEND);
                    }
                    _ => {
                        changed.transform.get_mut(target).expect("position").pos =
                            Vec2::from_ints(1750, 1000);
                    }
                }
                assert!(!changed.shows(Team::Dire, target));
                assert_eq!(
                    before,
                    encode_frame_to_vec(&ServerMsg::Snapshot {
                        view: changed.view(Team::Dire)
                    })
                    .expect("frame"),
                    "hidden property {change}, state {state:?}"
                );
            }
        }
    }
    {
        let bag = world.inventory.get_mut(actor).expect("inventory");
        bag.slots[0] = Some(fresh_stack(bota_proto::ItemId::HEALING_SALVE, 1));
        bag.slots[1] = Some(fresh_stack(bota_proto::ItemId::TOWN_PORTAL_SCROLL, 1));
    }
    // Choosing a different item may not change the newly added observation.
    for phase in [
        ActionPhase::Before { progress: 1000 },
        ActionPhase::During { progress: 1000 },
        ActionPhase::After { progress: 1000 },
    ] {
        let mut observed = None;
        for slot in [ItemSlot(0), ItemSlot(1)] {
            world.action.insert(
                actor,
                Action {
                    state: ActionState::UseItem {
                        slot,
                        target: at,
                        phase,
                    },
                    attack_cooldown: 0,
                },
            );
            let view = world.view(Team::Dire);
            let now = view.unit(wire_id(actor)).expect("actor").action_phase;
            if let Some(previous) = observed {
                assert_eq!(
                    previous, now,
                    "item choice must not enter animation progress"
                );
            } else {
                observed = Some(now);
            }
        }
    }
}

#[test]
fn sixty_idle_visible_units_pay_only_the_absent_phase_tag() {
    let mut world = World::new();
    for n in 0..60 {
        world.spawn_unit(
            &MELEE_CREEP,
            Team::Radiant,
            Vec2::from_ints(1000 + n * 40, 1000),
        );
    }
    world.settle();
    let view = world.view(Team::Radiant);
    assert_eq!(view.units.len(), 60);
    let actual: usize = view
        .units
        .iter()
        .map(|unit| encode_frame_to_vec(unit).expect("unit").len())
        .sum();
    let legacy: usize = view
        .units
        .iter()
        .map(|unit| {
            let mut unit = unit.clone();
            unit.action_phase = None;
            encode_frame_to_vec(&unit).expect("unit").len() - 1
        })
        .sum();
    assert_eq!(
        actual - legacy,
        60,
        "one byte per real idle unit, not a populated idle record"
    );
}
