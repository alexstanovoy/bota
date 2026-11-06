//! The Twin Gates at Dota 7.41f: a four-second warp at one gate carries its
//! caster out at the other gate's exit, for 30 mana given back if it is broken
//! off, with no cooldown, for either side and a courier alike; a stun or a
//! root breaks it, damage does not; both gates glow while it runs, seen only
//! by a side that sees them.

use bota_core::{
    DOTA_TWIN_GATES, ModifierKind, WARP_CHANNEL_TICKS, WARP_MANA, facing_towards, map_of,
};
use bota_proto::{
    AbilityId, EffectId, EventKind, Fixed, HeroId, MapId, ObjectiveKind, Order, Pick, RejectReason,
    SlotId, Target, Team, UnitKind, Vec2,
};

use crate::game::{Command, Entity, Event, UnitOrder, World, wire_id};
use crate::game_loop::visible_events;

use super::support::*;

/// A match on the Dota map with its default objectives, a Shadow Fiend for
/// each side given, at the horn.
fn gates_match(sides: &[Team]) -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = map_of(MapId(0)).default_objectives;
    cfg.picks = sides
        .iter()
        .enumerate()
        .map(|(slot, team)| Pick {
            slot: SlotId(slot as u8),
            team: *team,
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    let mut world = World::for_match(&cfg, cfg.rng());
    world.clock_shift.game = world.schedule().horn;
    world
}

fn hero(world: &World, slot: usize) -> Entity {
    world.seats[slot].unit.expect("standing")
}

/// The Twin Gate standing at a spot.
fn gate(world: &World, index: usize) -> Entity {
    let at = DOTA_TWIN_GATES[index].at;
    world
        .entities
        .iter()
        .find(|&entity| {
            world.kind.get(entity) == Some(&UnitKind::TwinGate)
                && world.transform.get(entity).is_some_and(|t| t.pos == at)
        })
        .expect("the gate stands there")
}

fn pos(world: &World, unit: Entity) -> Vec2 {
    world.transform.get(unit).expect("standing").pos
}

/// Puts a unit in front of gate `index`, within its reach, standing still.
fn before_gate(world: &mut World, unit: Entity, index: usize) {
    let at = DOTA_TWIN_GATES[index].at + Vec2::from_ints(0, -170);
    world.transform.get_mut(unit).expect("standing").pos = at;
    world.set_order(unit, UnitOrder::Stand);
    world.settle();
}

/// A seat orders one of its units, validated as a client's order is, and
/// the world advances a tick.
fn order(
    world: &mut World,
    slot: usize,
    unit: Option<Entity>,
    order: Order,
    queue: bool,
) -> Vec<Event> {
    let slot = world.seats[slot].slot;
    let named = unit.map(wire_id);
    assert_eq!(world.validate_order(slot, named, &order), Ok(()));
    world.advance_trusted(&[Command {
        slot,
        unit: named,
        order,
        queue,
        seq: 0,
    }])
}

/// A click on a gate: an attack order at it.
fn click(at: Entity) -> Order {
    Order::Attack {
        target: Target::Unit(wire_id(at)),
    }
}

fn mana(world: &World, unit: Entity) -> Fixed {
    world.mana.get(unit).expect("a pool").mana
}

/// Runs the world until a unit's warp has begun, at most a second.
fn until_channelling(world: &mut World, unit: Entity) {
    for _ in 0..30 {
        if world.is_channelling(unit) {
            return;
        }
        world.step();
    }
    assert!(world.is_channelling(unit), "the warp never began");
}

fn warped(events: &[Event]) -> Option<(bota_proto::EntityId, bota_proto::EntityId)> {
    events.iter().find_map(|event| match event.kind() {
        EventKind::GateWarped { from, to, .. } => Some((from, to)),
        _ => None,
    })
}

/// Runs the world until a warp lands, at most `limit` ticks; answers the
/// ticks it took and the landing's events.
fn until_warped(world: &mut World, limit: u32) -> Option<(u32, Vec<Event>)> {
    for ticks in 1..=limit {
        let events = world.step();
        if warped(&events).is_some() {
            return Some((ticks, events));
        }
    }
    None
}

fn glowing(world: &World, gate: Entity) -> bool {
    world.wears(gate, ModifierKind::TwinGateActive)
}

#[test]
fn the_dota_map_stands_its_two_gates_by_default_and_the_mid_only_map_only_when_asked() {
    let world = gates_match(&[Team::Radiant]);
    assert!(
        map_of(MapId(0))
            .default_objectives
            .has(ObjectiveKind::TwinGates)
    );
    for (index, def) in DOTA_TWIN_GATES.iter().enumerate() {
        let at = gate(&world, index);
        assert_eq!(world.team.get(at), Some(&Team::Neutral));
        assert_eq!(
            world.transform.get(at).expect("standing").facing,
            def.facing
        );
        assert_eq!(
            world.gate.get(at).expect("a gate").partner,
            gate(&world, 1 - index)
        );
    }
    let mut cfg = config();
    cfg.map = bota_core::MAP2_ID;
    cfg.objectives = map_of(bota_core::MAP2_ID).default_objectives;
    let skirmish = World::for_match(&cfg, cfg.rng());
    assert!(
        !skirmish
            .entities
            .iter()
            .any(|e| skirmish.kind.get(e) == Some(&UnitKind::TwinGate)),
        "the mid-only map stands none unless asked"
    );
    cfg.objectives = bota_proto::ObjectiveSet::of(ObjectiveKind::TwinGates);
    let asked = World::for_match(&cfg, cfg.rng());
    assert_eq!(
        asked
            .entities
            .iter()
            .filter(|e| asked.kind.get(*e) == Some(&UnitKind::TwinGate))
            .count(),
        2,
        "the mid-only map holds the full map's gates"
    );
}

#[test]
fn a_warp_channels_four_seconds_and_lands_on_the_other_gates_exit_facing_out_of_it() {
    let mut world = gates_match(&[Team::Radiant]);
    let (south_east, north_west) = (gate(&world, 1), gate(&world, 0));
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let before = mana(&world, caster);
    order(&mut world, 0, None, click(south_east), false);
    until_channelling(&mut world, caster);
    assert_eq!(mana(&world, caster), before - Fixed::from_int(WARP_MANA));
    assert!(
        glowing(&world, south_east) && glowing(&world, north_west),
        "both ends glow"
    );
    let (ticks, events) = until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    assert!(
        (WARP_CHANNEL_TICKS - 1..=WARP_CHANNEL_TICKS).contains(&ticks),
        "four seconds of channel: {ticks} ticks"
    );
    assert_eq!(
        warped(&events),
        Some((wire_id(south_east), wire_id(north_west)))
    );
    let exit = DOTA_TWIN_GATES[0].exit;
    assert_eq!(pos(&world, caster), exit, "on the exit to the hundredth");
    assert_eq!(
        world.transform.get(caster).expect("standing").facing,
        facing_towards(DOTA_TWIN_GATES[0].at, exit),
        "facing out of the gate"
    );
    assert!(!glowing(&world, south_east) && !glowing(&world, north_west));
    // Four seconds of regeneration came back meanwhile, the 30 did not.
    assert!(
        mana(&world, caster) < before - Fixed::from_int(WARP_MANA - 10),
        "the mana is spent: {:?} of {before:?}",
        mana(&world, caster)
    );
}

#[test]
fn the_exits_are_one_hundred_and_forty_south_of_the_install_spots() {
    // npc_dota_portal1 (-6457.690918, 7599.036621) and npc_dota_portal2
    // (6425.52832, -7313.782715), shifted by 9216.
    let hundredths = |v: Fixed| (i64::from(v.raw) * 100 + (1 << 15)) >> 16;
    let spots: Vec<(i64, i64, i64, i64)> = DOTA_TWIN_GATES
        .iter()
        .map(|g| {
            (
                hundredths(g.at.x),
                hundredths(g.at.y),
                hundredths(g.exit.x),
                hundredths(g.exit.y),
            )
        })
        .collect();
    assert_eq!(
        spots,
        vec![
            (275_831, 1_681_504, 275_831, 1_667_504),
            (1_564_153, 190_222, 1_564_153, 176_222)
        ]
    );
}

#[test]
fn a_warp_from_the_north_west_lands_at_the_south_east_exit_for_the_other_side() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let north_west = gate(&world, 0);
    let caster = hero(&world, 1);
    before_gate(&mut world, caster, 0);
    order(&mut world, 1, None, click(north_west), false);
    until_channelling(&mut world, caster);
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    assert_eq!(pos(&world, caster), DOTA_TWIN_GATES[1].exit);
}

#[test]
fn there_is_no_cooldown_a_warp_back_follows_at_once() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("there");
    let north_west = gate(&world, 0);
    order(&mut world, 0, None, click(north_west), false);
    until_channelling(&mut world, caster);
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("and back");
    assert_eq!(pos(&world, caster), DOTA_TWIN_GATES[1].exit);
}

#[test]
fn a_stun_breaks_the_warp_off_and_gives_its_mana_back() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let (from, before) = (pos(&world, caster), mana(&world, caster));
    {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    until_channelling(&mut world, caster);
    run(&mut world, WARP_CHANNEL_TICKS / 2);
    put_on(&mut world, caster, ModifierKind::Stunned, 10);
    world.step();
    assert!(!world.is_channelling(caster));
    assert_eq!(
        mana(&world, caster).to_int(),
        before.to_int(),
        "the 30 mana given back"
    );
    assert!(!glowing(&world, gate(&world, 0)) && !glowing(&world, gate(&world, 1)));
    assert!(until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).is_none());
    assert_eq!(pos(&world, caster), from, "it stays where it stood");
}

#[test]
fn a_root_breaks_the_warp_off_and_a_rooted_unit_cannot_begin_one() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let before = mana(&world, caster);
    {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    until_channelling(&mut world, caster);
    put_on(&mut world, caster, ModifierKind::Rooted, 60);
    world.step();
    assert!(
        !world.is_channelling(caster),
        "Dota 7.41e: a root interrupts it"
    );
    assert_eq!(mana(&world, caster).to_int(), before.to_int());
    let at = gate(&world, 1);
    assert_eq!(
        intake(&world, 0, None, &click(at)),
        Err(RejectReason::Disabled),
        "ROOT_DISABLES"
    );
    let left = mana(&world, caster);
    run(&mut world, 30);
    assert!(!world.is_channelling(caster), "nothing begins while rooted");
    assert!(
        mana(&world, caster) >= left,
        "nothing spent: {:?} of {left:?}",
        mana(&world, caster)
    );
}

#[test]
fn damage_does_not_break_the_warp() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    until_channelling(&mut world, caster);
    let near = pos(&world, caster) + Vec2::from_ints(120, 0);
    let creep = thinking_creep_of(&mut world, Team::Dire, near);
    let full = world.health.get(caster).expect("health").hp;
    run(&mut world, WARP_CHANNEL_TICKS / 2);
    assert!(
        world.health.get(caster).expect("health").hp < full,
        "the creep's blows land"
    );
    assert!(world.is_channelling(caster), "and break nothing");
    assert!(world.alive(creep));
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands all the same");
    assert_eq!(pos(&world, caster), DOTA_TWIN_GATES[0].exit);
}

#[test]
fn a_courier_warps_for_nothing() {
    let mut world = gates_match(&[Team::Radiant]);
    let courier = the_courier(&world);
    before_gate(&mut world, courier, 1);
    {
        let at = gate(&world, 1);
        order(&mut world, 0, Some(courier), click(at), false)
    };
    until_channelling(&mut world, courier);
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("the courier lands");
    assert_eq!(pos(&world, courier), DOTA_TWIN_GATES[0].exit);
}

#[test]
fn queued_orders_run_on_after_the_warp() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    let onward = DOTA_TWIN_GATES[0].exit + Vec2::from_ints(0, -400);
    order(
        &mut world,
        0,
        None,
        Order::Move {
            target: Target::Pos(onward),
        },
        true,
    );
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    run(&mut world, 3 * 30);
    let walked = pos(&world, caster);
    assert!(
        walked.within(onward, bota_core::rules::units(50)),
        "the queued move is walked from the far gate: {walked:?}"
    );
}

#[test]
fn the_glow_shows_only_to_a_side_that_sees_the_gate_and_the_landing_only_to_who_sees_it() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let (south_east, north_west) = (gate(&world, 1), gate(&world, 0));
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    order(&mut world, 0, None, click(south_east), false);
    until_channelling(&mut world, caster);
    world.step();
    let glows = |world: &World, side: Option<Team>, gate: Entity| {
        shown(world, side, gate)
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::TWIN_GATE_ACTIVE)
    };
    assert!(glows(&world, None, north_west) && glows(&world, None, south_east));
    assert!(
        glows(&world, Some(Team::Radiant), south_east),
        "its own caster sees it"
    );
    assert!(
        !glows(&world, Some(Team::Radiant), north_west),
        "nobody of Radiant sees the far gate"
    );
    assert!(
        !glows(&world, Some(Team::Dire), north_west)
            && !glows(&world, Some(Team::Dire), south_east),
        "Dire, at its fountain, sees neither gate glow"
    );
    // The Dire hero comes to stand by the far gate.
    let dire = hero(&world, 1);
    before_gate(&mut world, dire, 0);
    world.transform.get_mut(dire).expect("standing").pos =
        DOTA_TWIN_GATES[0].at + Vec2::from_ints(300, -300);
    world.settle();
    world.step();
    assert!(
        glows(&world, Some(Team::Dire), north_west),
        "seen from beside it"
    );
    assert!(!glows(&world, Some(Team::Dire), south_east));
    let (_, events) = until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    let told = |side| {
        visible_events(&events, Some(side))
            .into_iter()
            .find_map(|kind| match kind {
                EventKind::GateWarped { unit, .. } => Some(unit),
                _ => None,
            })
    };
    assert_eq!(told(Team::Radiant), Some(Some(wire_id(caster))));
    assert_eq!(
        told(Team::Dire),
        Some(Some(wire_id(caster))),
        "it came out beside Dire"
    );
}

#[test]
fn a_side_that_sees_neither_gate_nor_the_unit_is_not_told_of_the_warp() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let mut events = {
        let at = gate(&world, 1);
        order(&mut world, 0, None, click(at), false)
    };
    for _ in 0..2 * WARP_CHANNEL_TICKS {
        let tick = world.step();
        let landed = warped(&tick).is_some();
        events.extend(tick);
        if landed {
            break;
        }
    }
    assert!(warped(&events).is_some(), "it lands");
    let dire = visible_events(&events, Some(Team::Dire));
    assert!(
        !dire
            .iter()
            .any(|kind| matches!(kind, EventKind::GateWarped { .. })),
        "Dire, at its fountain, hears nothing"
    );
    assert!(
        !dire.iter().any(|kind| matches!(
            kind,
            EventKind::AbilityCast { ability, .. } if *ability == AbilityId::TWIN_GATE_WARP
        )),
        "nor of the cast (Dota fixed its combat log telling an unseen channel)"
    );
    let radiant = visible_events(&events, Some(Team::Radiant));
    assert!(
        radiant
            .iter()
            .any(|kind| matches!(kind, EventKind::GateWarped { .. }))
    );
}

#[test]
fn a_side_by_the_exit_sees_its_gate_glow_through_the_whole_channel_but_not_who_channels() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let (south_east, north_west) = (gate(&world, 1), gate(&world, 0));
    let (caster, watcher) = (hero(&world, 0), hero(&world, 1));
    // Dire waits beside the exit the warp will come out of.
    world.transform.get_mut(watcher).expect("standing").pos =
        DOTA_TWIN_GATES[0].exit + Vec2::from_ints(250, -150);
    world.set_order(watcher, UnitOrder::Stand);
    world.settle();
    before_gate(&mut world, caster, 1);
    let glows = |world: &World, gate: Entity| {
        shown(world, Some(Team::Dire), gate)
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::TWIN_GATE_ACTIVE)
    };
    let in_dire_view = |world: &World| {
        world
            .view(Team::Dire)
            .units
            .iter()
            .any(|u| u.id == wire_id(caster))
    };
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    let mut channelled = 0;
    let landing = loop {
        assert!(
            channelled <= 2 * WARP_CHANNEL_TICKS,
            "the warp never landed"
        );
        assert!(
            glows(&world, north_west),
            "the exit glows at tick {channelled} of the channel"
        );
        assert!(!glows(&world, south_east), "the far gate is unseen");
        assert!(!in_dire_view(&world), "who channels is unseen");
        let events = world.step();
        channelled += 1;
        if warped(&events).is_some() {
            break events;
        }
    };
    assert!(!glows(&world, north_west), "dark once it has landed");
    assert!(in_dire_view(&world), "the unit seen coming out");
    let told = visible_events(&landing, Some(Team::Dire))
        .into_iter()
        .find_map(|kind| match kind {
            EventKind::GateWarped { unit, .. } => Some(unit),
            _ => None,
        });
    assert_eq!(told, Some(Some(wire_id(caster))));
}

#[test]
fn a_warp_whose_exit_is_walled_off_lands_nowhere_and_gives_its_mana_back() {
    let mut world = gates_match(&[Team::Radiant]);
    stand_a_wall(
        &mut world,
        DOTA_TWIN_GATES[0].exit,
        bota_core::rules::units(800),
    );
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let (from, before) = (pos(&world, caster), mana(&world, caster));
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    assert!(
        until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).is_none(),
        "no landing is told"
    );
    assert!(!world.is_channelling(caster));
    assert_eq!(pos(&world, caster), from, "it stays where it channelled");
    assert!(
        mana(&world, caster) >= before,
        "the 30 mana given back: {:?} of {before:?}",
        mana(&world, caster)
    );
    assert!(!glowing(&world, gate(&world, 0)) && !glowing(&world, gate(&world, 1)));
}

#[test]
fn a_warp_shows_its_caster_to_the_gates_neutral_side_as_any_cast_at_a_unit_does() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    assert!(
        world
            .exposure
            .get(caster)
            .is_some_and(|exposure| exposure.0[Team::Neutral as usize].is_some()),
        "the neutral side is shown the caster: its sight wakes the camps"
    );
}

/// What a seat's order is refused for, or `Ok` when it is taken.
fn intake(
    world: &World,
    slot: usize,
    unit: Option<Entity>,
    order: &Order,
) -> Result<(), RejectReason> {
    world.validate_order(world.seats[slot].slot, unit.map(wire_id), order)
}

#[test]
fn a_click_on_a_gate_is_refused_to_a_stunned_or_rooted_hero_and_never_warps_it_later() {
    for disable in [ModifierKind::Stunned, ModifierKind::Rooted] {
        let mut world = gates_match(&[Team::Radiant]);
        let caster = hero(&world, 0);
        before_gate(&mut world, caster, 1);
        put_on(&mut world, caster, disable, 15);
        let at = gate(&world, 1);
        assert_eq!(
            intake(&world, 0, None, &click(at)),
            Err(RejectReason::Disabled),
            "{disable:?}: Dota refuses the cast"
        );
        // Refused at intake, the order never reaches the world: nothing is pending for when the
        // disable lifts.
        assert!(world.pending_cast(caster).is_none());
        assert!(
            until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).is_none(),
            "{disable:?}: no warp once it lifts"
        );
        assert!(!world.is_channelling(caster));
    }
}

#[test]
fn a_click_on_a_watcher_is_refused_to_a_stunned_hero() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    let at = map_of(MapId(0)).watchers[4];
    world.transform.get_mut(caster).expect("standing").pos = at + Vec2::from_ints(100, 0);
    world.settle();
    let watcher = world
        .entities
        .iter()
        .find(|e| world.kind.get(*e) == Some(&UnitKind::Watcher) && world.pos(*e) == Some(at))
        .expect("the watcher");
    put_on(&mut world, caster, ModifierKind::Stunned, 15);
    assert_eq!(
        intake(&world, 0, None, &click(watcher)),
        Err(RejectReason::Disabled)
    );
}

#[test]
fn a_hero_short_of_mana_is_refused_at_the_click() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    world.mana.get_mut(caster).expect("a pool").mana = Fixed::from_int(WARP_MANA - 1);
    let at = gate(&world, 1);
    assert_eq!(
        intake(&world, 0, None, &click(at)),
        Err(RejectReason::NotEnoughMana)
    );
}

#[test]
fn an_illusion_ordered_to_a_gate_is_refused_and_never_warps() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    world.make_rune_illusions(caster);
    let illusion = world
        .entities
        .iter()
        .find(|e| world.illusion.get(*e).is_some_and(|i| i.of == caster))
        .expect("an illusion");
    let at = gate(&world, 1);
    assert_eq!(
        intake(&world, 0, Some(illusion), &click(at)),
        Err(RejectReason::NotCastable),
        "an illusion casts nothing (Liquipedia, Twin Gate)"
    );
}

#[test]
fn a_warp_begins_only_within_reach_of_a_standing_gate_for_a_living_caster() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    let at = gate(&world, 1);
    let target = Target::Unit(wire_id(at));
    world.transform.get_mut(caster).expect("standing").pos =
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(0, -600);
    assert!(!world.warp_begins(caster, target), "out of reach");
    assert!(world.warp.get(caster).is_none());
    before_gate(&mut world, caster, 1);
    assert!(world.warp_begins(caster, target), "within reach");
    world.warp.remove(caster);
}

#[test]
fn a_caster_dying_mid_channel_puts_the_gates_out_and_lands_nowhere() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    let killer = hero(&world, 1);
    world.kill_by_hp_removal(killer, caster);
    assert!(world.warp.get(caster).is_none(), "the warp is gone with it");
    assert!(!glowing(&world, gate(&world, 0)) && !glowing(&world, gate(&world, 1)));
    assert!(until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).is_none());
}

#[test]
fn two_warps_through_one_gate_keep_it_lit_until_both_end() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let (first, second) = (hero(&world, 0), hero(&world, 1));
    before_gate(&mut world, first, 1);
    world.transform.get_mut(second).expect("standing").pos =
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(150, -120);
    world.set_order(second, UnitOrder::Stand);
    world.settle();
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    order(&mut world, 1, None, click(at), false);
    until_channelling(&mut world, first);
    until_channelling(&mut world, second);
    put_on(&mut world, first, ModifierKind::Stunned, 10);
    world.step();
    assert!(!world.is_channelling(first));
    assert!(glowing(&world, at), "the other warp still runs through it");
    let (_, events) = until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("the second lands");
    assert_eq!(
        events.iter().find_map(|e| match e.kind() {
            EventKind::GateWarped { unit, .. } => unit,
            _ => None,
        }),
        Some(wire_id(second))
    );
    assert!(!glowing(&world, at) && !glowing(&world, gate(&world, 0)));
}

/// A hero struck dead after its click was taken but before its cast begins: its health is
/// gone this tick, its body not yet buried.
fn dead_in_reach(world: &mut World, caster: Entity, at: Vec2) {
    world.health.get_mut(caster).expect("health").hp = Fixed::ZERO;
    world.transform.get_mut(caster).expect("standing").pos = at;
}

#[test]
fn a_caster_dead_before_its_warp_begins_never_begins_it() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    world.transform.get_mut(caster).expect("standing").pos =
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(0, -600);
    world.set_order(caster, UnitOrder::Stand);
    world.settle();
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    assert!(world.pending_cast(caster).is_some(), "walking in to it");
    dead_in_reach(
        &mut world,
        caster,
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(0, -170),
    );
    world.step();
    assert!(!world.is_channelling(caster));
    assert!(world.warp.get(caster).is_none(), "no warp entry");
    assert!(!glowing(&world, at) && !glowing(&world, gate(&world, 0)));
    assert!(
        until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).is_none(),
        "no landing"
    );
}

#[test]
fn a_caster_dead_before_its_lamp_begins_never_lights_the_watcher() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    let at = map_of(MapId(0)).watchers[4];
    world.transform.get_mut(caster).expect("standing").pos = at + Vec2::from_ints(600, 0);
    world.set_order(caster, UnitOrder::Stand);
    world.settle();
    let watcher = world
        .entities
        .iter()
        .find(|e| world.kind.get(*e) == Some(&UnitKind::Watcher) && world.pos(*e) == Some(at))
        .expect("the watcher");
    order(&mut world, 0, None, click(watcher), false);
    assert!(world.pending_cast(caster).is_some(), "walking in to it");
    dead_in_reach(&mut world, caster, at + Vec2::from_ints(100, 0));
    let events = world.step();
    assert!(
        !events.iter().any(|e| matches!(
            e.kind(),
            EventKind::AbilityCast { ability, .. } if ability == AbilityId::LAMP_USE
        )),
        "a dead hero casts nothing"
    );
    assert!(!world.is_channelling(caster));
    assert!(
        world.pending_cast(caster).is_none(),
        "its cast is called off"
    );
}

#[test]
fn a_refused_click_is_told_back_and_leaves_nothing_pending() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    put_on(&mut world, caster, ModifierKind::Stunned, 15);
    let at = gate(&world, 1);
    world.advance_checked(&[Command {
        slot: world.seats[0].slot,
        unit: None,
        order: click(at),
        queue: false,
        seq: 7,
    }]);
    assert_eq!(
        world
            .refused
            .iter()
            .map(|refused| (refused.seq, refused.reason))
            .collect::<Vec<_>>(),
        vec![(7, RejectReason::Disabled)]
    );
    assert!(world.pending_cast(caster).is_none());
    run(&mut world, 30);
    assert!(
        !world.is_channelling(caster),
        "nothing goes off once the stun lifts"
    );
}

#[test]
fn a_second_click_on_the_gate_leaves_the_warp_running() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    let mut ticks = 0;
    for _ in 0..WARP_CHANNEL_TICKS / 2 {
        world.step();
        ticks += 1;
    }
    // DOTA_ABILITY_BEHAVIOR_DONT_CANCEL_CHANNEL: clicking the gate again changes nothing.
    order(&mut world, 0, None, click(at), false);
    ticks += 1;
    assert!(world.is_channelling(caster), "the channel runs on");
    let (rest, _) = until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    assert!(
        ticks + rest <= WARP_CHANNEL_TICKS + 1,
        "on the first click's time: {ticks} + {rest} ticks"
    );
}

#[test]
fn objective_channels_charge_no_enemy_magic_stick() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let caster = hero(&world, 0);
    let enemy = hero(&world, 1);
    before_gate(&mut world, caster, 1);
    world.transform.get_mut(enemy).expect("standing").pos =
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(400, -300);
    world.set_order(enemy, UnitOrder::Stand);
    world.settle();
    hand_item(&mut world, enemy, bota_proto::ItemId::MAGIC_STICK, 0);
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    let charges = world.inventory.get(enemy).expect("a bag").slots[0]
        .expect("the stick")
        .charges;
    assert_eq!(
        charges, 0,
        "Valve: objective interactions do not charge a Magic Stick (Dota2-Gameplay issue \
         32850); the 7.41 bug fix \"Glacier Guard triggering when using the Twin Gates\""
    );
}

#[test]
fn a_stunned_caster_begins_no_warp_by_any_path() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let at = gate(&world, 1);
    let slot = world
        .interaction_at(caster, wire_id(at))
        .expect("the warp's slot");
    put_on(&mut world, caster, ModifierKind::Stunned, 15);
    assert!(!world.begin_ability(caster, slot, Target::Unit(wire_id(at))));
    assert!(world.warp.get(caster).is_none());
}

#[test]
fn a_warp_begins_for_no_caster_that_has_fallen() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let killer = hero(&world, 1);
    world.kill_by_hp_removal(killer, caster);
    let at = gate(&world, 1);
    assert!(!world.warp_begins(caster, Target::Unit(wire_id(at))));
    assert!(world.warp.get(caster).is_none());
}

#[test]
fn a_click_taken_before_a_stun_waits_for_it_to_lift() {
    let mut world = gates_match(&[Team::Radiant]);
    let caster = hero(&world, 0);
    world.transform.get_mut(caster).expect("standing").pos =
        DOTA_TWIN_GATES[1].at + Vec2::from_ints(0, -700);
    world.set_order(caster, UnitOrder::Stand);
    world.settle();
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    put_on(&mut world, caster, ModifierKind::Stunned, 20);
    run(&mut world, 20);
    assert!(world.pending_cast(caster).is_some(), "the cast still waits");
    for _ in 0..4 * 30 {
        if world.is_channelling(caster) {
            break;
        }
        world.step();
    }
    assert!(
        world.is_channelling(caster),
        "walked in once the stun lifted"
    );
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands once walked in");
}

/// The warp a unit channels is told with the unit, as Dota plays its channel for whoever sees it:
/// a client knows the channel for a teleport from the view alone, after a reconnect as well.
#[test]
fn the_warp_channelled_is_told_with_the_unit() {
    let mut world = gates_match(&[Team::Radiant, Team::Dire]);
    let caster = hero(&world, 0);
    before_gate(&mut world, caster, 1);
    let channel = |world: &World, side| {
        shown(world, side, caster)
            .channel
            .map(|channel| channel.ability)
    };
    assert_eq!(channel(&world, Some(Team::Radiant)), None);
    let at = gate(&world, 1);
    order(&mut world, 0, None, click(at), false);
    until_channelling(&mut world, caster);
    world.step();
    assert_eq!(
        channel(&world, Some(Team::Radiant)),
        Some(AbilityId::TWIN_GATE_WARP)
    );
    assert_eq!(channel(&world, None), Some(AbilityId::TWIN_GATE_WARP));
    until_warped(&mut world, 2 * WARP_CHANNEL_TICKS).expect("it lands");
    assert_eq!(
        channel(&world, Some(Team::Radiant)),
        None,
        "the channel is over"
    );
}
