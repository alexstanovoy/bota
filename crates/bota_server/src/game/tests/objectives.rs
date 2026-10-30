//! The map's objectives at Dota 7.41f: Outposts captured by a channel and
//! taken to by scroll, Watchers lit for a side's sight and put out by the
//! other, Shrines of Wisdom gathered, and the rules that stand them up.

use bota_core::{
    CAPTURE_TICKS, LAMP_CHANNEL_TICKS, LAMP_LIT_TICKS, LAMP_OFF_TICKS, MELEE_CREEP, ModifierKind,
    NeutralKind, OBJECTIVES_OPEN_TICK, SHRINE_COUNTDOWN_TICKS, SHRINE_FIRST_TICK,
    SHRINE_PERIOD_TICKS, WATCHER, WATCHER_DAY_VISION, WATCHER_NIGHT_VISION, map_of, rules,
};
use bota_proto::{
    AbilityId, EntityId, EventKind, HeroId, ItemId, ItemSlot, MapId, ObjectiveKind, ObjectiveSet,
    Order, Pick, SlotId, StatusFlags, Target, Team, UnitKind, Vec2,
};

use crate::game::{Command, Entity, Event, UnitOrder, World, wire_id};
use crate::game_loop::visible_events;

use super::support::*;

/// A match on the Dota map with every objective it plays, its seats
/// Shadow Fiends of the sides given, and the clock at the horn.
fn objectives_match(sides: &[Team]) -> World {
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
    world.clock_shift.game = OBJECTIVES_OPEN_TICK;
    world
}

/// The hero of a seat.
fn hero(world: &World, slot: usize) -> Entity {
    world.seats[slot].unit.expect("standing")
}

/// The unit of a kind standing at a spot.
fn the(world: &World, kind: UnitKind, at: Vec2) -> Entity {
    world
        .entities
        .iter()
        .find(|&entity| {
            world.kind.get(entity) == Some(&kind)
                && world.transform.get(entity).is_some_and(|t| t.pos == at)
        })
        .expect("it stands there")
}

/// Puts a hero at a spot, standing still.
fn place(world: &mut World, unit: Entity, at: Vec2) {
    world.transform.get_mut(unit).expect("standing").pos = at;
    world.set_order(unit, UnitOrder::Stand);
    world.settle();
}

/// Puts a seat's hero at a spot, standing still.
fn place_hero(world: &mut World, slot: usize, at: Vec2) {
    let unit = hero(world, slot);
    place(world, unit, at);
}

/// Seats give their heroes orders, validated as a client's are, and the
/// world advances one tick.
fn give(world: &mut World, orders: &[(usize, Order)]) -> Vec<Event> {
    let commands: Vec<Command> = orders
        .iter()
        .map(|(slot, order)| {
            let slot = world.seats[*slot].slot;
            assert_eq!(world.validate_order(slot, None, order), Ok(()));
            Command {
                slot,
                unit: None,
                order: *order,
                queue: false,
                seq: 0,
            }
        })
        .collect();
    world.advance(&commands)
}

/// An attack order at a unit.
fn attack(at: Entity) -> Order {
    Order::Attack {
        target: Target::Unit(wire_id(at)),
    }
}

/// Runs the world until a tick's events tell of something, at most
/// `limit` ticks; answers how many ticks it took and those events.
fn until(
    world: &mut World,
    limit: u32,
    told: impl Fn(&EventKind) -> bool,
) -> Option<(u32, Vec<Event>)> {
    for ticks in 1..=limit {
        let events = world.step();
        if events.iter().any(|event| told(&event.kind)) {
            return Some((ticks, events));
        }
    }
    None
}

fn is_capture(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Captured { .. })
}

/// Gives capture orders and runs the world to the capture: answers the
/// ticks from the channel's start and the events of the capture.
fn capture_by(world: &mut World, orders: &[(usize, Order)]) -> Option<(u32, Vec<Event>)> {
    let begun = |kind: &EventKind| matches!(kind, EventKind::AbilityCast { ability, .. } if *ability == AbilityId::CAPTURE);
    if !give(world, orders).iter().any(|event| begun(&event.kind)) {
        until(world, rules::TICKS_PER_SECOND, begun)?;
    }
    until(world, 2 * CAPTURE_TICKS, is_capture)
}

/// The Radiant Outpost, at its home in the south-west, and the Dire
/// hero standing within reach of it; a Radiant tier-two tower down when
/// `opened`.
fn dire_at_the_radiant_outpost(sides: &[Team], opened: bool) -> (World, Entity) {
    let mut world = objectives_match(sides);
    let outpost = map_of(MapId(0)).outposts[0];
    assert_eq!(outpost.home, Team::Radiant);
    let it = the(&world, UnitKind::Outpost, outpost.at);
    if opened {
        let (_, _, tier_two) = *map_of(MapId(0))
            .radiant_towers
            .iter()
            .find(|(lane, tier, _)| *lane == rules::LANE_MID && *tier == 2)
            .expect("a mid tier two");
        fell_at(&mut world, tier_two);
    }
    for slot in 0..world.seats.len() {
        if world.seats[slot].team == Team::Dire {
            let spot = outpost.at + Vec2::from_ints(100 + 40 * slot as i32, 100);
            place_hero(&mut world, slot, spot);
            let dire = hero(&world, slot);
            world.transform.get_mut(dire).expect("standing").facing =
                bota_core::facing_towards(spot, outpost.at);
        }
    }
    world.step();
    (world, it)
}

#[test]
fn an_outpost_is_captured_by_a_six_second_channel_once_a_tier_two_tower_has_fallen() {
    let (mut world, outpost) = dire_at_the_radiant_outpost(&[Team::Radiant, Team::Dire], false);
    assert_eq!(world.team.get(outpost), Some(&Team::Radiant), "home side");
    assert!(world.wears(outpost, ModifierKind::CaptureGuard));
    let dire = hero(&world, 1);
    give(&mut world, &[(1, attack(outpost))]);
    assert!(
        until(&mut world, 2 * CAPTURE_TICKS, is_capture).is_none(),
        "every Radiant tier two stands: no capture"
    );
    assert_eq!(world.team.get(outpost), Some(&Team::Radiant));

    let (mut world, outpost) = dire_at_the_radiant_outpost(&[Team::Radiant, Team::Dire], true);
    assert!(!world.wears(outpost, ModifierKind::CaptureGuard));
    give(&mut world, &[(1, attack(outpost))]);
    let shown = shown(&world, Some(Team::Dire), dire);
    assert!(shown.statuses.bits & StatusFlags::CHANNELLING != 0);
    assert!(
        shown.abilities.iter().all(|a| a.id != AbilityId::CAPTURE),
        "the hidden ability has no slot in a view"
    );
    give(
        &mut world,
        &[(
            1,
            Order::Move {
                target: Target::None,
            },
        )],
    );
    let (ticks, events) = capture_by(&mut world, &[(1, attack(outpost))]).expect("captured");
    assert_eq!(ticks, CAPTURE_TICKS, "six seconds of channel");
    assert_eq!(world.team.get(outpost), Some(&Team::Dire));
    let told = |side| {
        visible_events(&events, Some(side))
            .into_iter()
            .find(is_capture)
    };
    assert_eq!(
        told(Team::Dire),
        Some(EventKind::Captured {
            unit: wire_id(outpost),
            team: Team::Dire,
            by: Some(wire_id(dire)),
        })
    );
    assert_eq!(
        told(Team::Radiant),
        Some(EventKind::Captured {
            unit: wire_id(outpost),
            team: Team::Dire,
            by: None,
        }),
        "the other side is told it was lost, not by whom"
    );
    assert!(!world.is_channelling(dire), "the channel ends with it");
}

#[test]
fn two_capturers_share_the_six_seconds() {
    let sides = [Team::Radiant, Team::Dire, Team::Dire];
    let (mut world, outpost) = dire_at_the_radiant_outpost(&sides, true);
    let both = [(1, attack(outpost)), (2, attack(outpost))];
    let (ticks, _) = capture_by(&mut world, &both).expect("captured");
    assert_eq!(ticks, CAPTURE_TICKS / 2, "three seconds for two");
}

#[test]
fn a_stun_or_a_move_breaks_the_capture_and_its_progress_is_lost() {
    let (mut world, outpost) = dire_at_the_radiant_outpost(&[Team::Radiant, Team::Dire], true);
    let dire = hero(&world, 1);
    give(&mut world, &[(1, attack(outpost))]);
    run(&mut world, CAPTURE_TICKS / 2);
    put_on(&mut world, dire, ModifierKind::Stunned, 10);
    world.step();
    assert!(!world.is_channelling(dire));
    assert!(world.capture.get(outpost).is_none(), "progress lost");
    run(&mut world, 10);
    give(&mut world, &[(1, attack(outpost))]);
    run(&mut world, CAPTURE_TICKS / 2);
    let spot = world.transform.get(dire).expect("standing").pos;
    let away = Order::Move {
        target: Target::Pos(spot + Vec2::from_ints(0, 200)),
    };
    give(&mut world, &[(1, away)]);
    assert!(!world.is_channelling(dire));
    assert!(
        until(&mut world, CAPTURE_TICKS, is_capture).is_none(),
        "neither half channel took it"
    );
    let (ticks, _) = capture_by(&mut world, &[(1, attack(outpost))]).expect("captured");
    assert_eq!(ticks, CAPTURE_TICKS, "a whole channel again");
}

#[test]
fn the_home_side_retakes_its_outpost_with_no_tower_down() {
    let (mut world, outpost) = dire_at_the_radiant_outpost(&[Team::Radiant, Team::Dire], true);
    give(&mut world, &[(1, attack(outpost))]);
    until(&mut world, 2 * CAPTURE_TICKS, is_capture).expect("captured");
    let at = world.transform.get(outpost).expect("standing").pos;
    place_hero(&mut world, 0, at + Vec2::from_ints(-150, -150));
    // Every Dire tower stands, and Radiant takes it back all the same.
    give(&mut world, &[(0, attack(outpost))]);
    until(&mut world, 2 * CAPTURE_TICKS, is_capture).expect("retaken");
    assert_eq!(world.team.get(outpost), Some(&Team::Radiant));
    // Its own: an attack at it is no capture, and a move follows it.
    let radiant = hero(&world, 0);
    give(&mut world, &[(0, attack(outpost))]);
    world.step();
    assert!(!world.is_channelling(radiant));
}

#[test]
fn a_scroll_carries_to_an_outpost_its_side_holds() {
    let (mut world, outpost) = dire_at_the_radiant_outpost(&[Team::Radiant, Team::Dire], true);
    let at = world.transform.get(outpost).expect("standing").pos;
    let dire = hero(&world, 1);
    world.seats[1].item_clocks.clear();
    let home = map_of(MapId(0)).fountains[1];
    let read = Order::Use {
        slot: ItemSlot::TOWN_PORTAL,
        target: Target::Pos(at),
    };
    let carried = |world: &mut World, dire: Entity| {
        world.inventory.get_mut(dire).expect("a bag").town_portal =
            Some(Some(crate::game::ItemStack {
                owner: SlotId(1),
                ..fresh_stack(ItemId::TOWN_PORTAL_SCROLL, 1)
            }));
        world.seats[1].item_clocks.clear();
        place(world, dire, home + Vec2::from_ints(-600, -600));
        give(world, &[(1, read)]);
        run(world, scroll_def().duration + 1);
        world.transform.get(dire).expect("standing").pos
    };
    let landed = carried(&mut world, dire);
    assert!(
        !landed.within(at, rules::units(scroll().max_distance)),
        "Radiant's Outpost takes no Dire scroll"
    );
    place(&mut world, dire, at + Vec2::from_ints(150, 150));
    give(&mut world, &[(1, attack(outpost))]);
    until(&mut world, 2 * CAPTURE_TICKS, is_capture).expect("captured");
    world.seats[1].item_clocks.clear();
    let landed = carried(&mut world, dire);
    assert!(landed.within(at, rules::units(scroll().max_distance)));
}

/// The Watcher of the Radiant jungle above the bounty rune, the Radiant
/// hero beside it and the Dire hero at its fountain.
fn a_watcher_and_its_lighter() -> (World, Entity) {
    let mut world = objectives_match(&[Team::Radiant, Team::Dire]);
    let at = map_of(MapId(0)).watchers[4];
    let watcher = the(&world, UnitKind::Watcher, at);
    place_hero(&mut world, 0, at + Vec2::from_ints(100, 0));
    world.step();
    (world, watcher)
}

/// Lights a Watcher by seat `slot`'s attack order and the channel run out.
fn light(world: &mut World, slot: usize, watcher: Entity) {
    give(world, &[(slot, attack(watcher))]);
    run(world, LAMP_CHANNEL_TICKS + 1);
}

/// The side a Watcher is lit for.
fn lit_for(world: &World, watcher: Entity) -> Option<Team> {
    world
        .modifiers
        .get(watcher)?
        .active()
        .find_map(|held| match held.kind {
            ModifierKind::VisionGrant { side, .. } => Some(side),
            _ => None,
        })
}

#[test]
fn a_lit_watcher_sees_for_its_side_by_day_and_night_for_seven_minutes() {
    let (mut world, watcher) = a_watcher_and_its_lighter();
    let at = world.transform.get(watcher).expect("standing").pos;
    light(&mut world, 0, watcher);
    assert_eq!(lit_for(&world, watcher), Some(Team::Radiant));
    let shown = shown(&world, Some(Team::Dire), watcher);
    let lit = shown
        .effects
        .iter()
        .find(|effect| effect.id == bota_proto::EffectId::VISION_GRANT)
        .expect("both sides see it lit");
    assert_eq!(lit.ticks_total, Some(LAMP_LIT_TICKS));
    // The lighter goes home: the Watcher alone sees for Radiant.
    let radiant = hero(&world, 0);
    place(&mut world, radiant, map_of(MapId(0)).fountains[0]);
    world.step();
    assert!(!world.time_of_day().night);
    let ring = |reach: i32| {
        [(1, 0), (0, 1), (-1, 0), (0, -1)]
            .map(|(dx, dy)| at + Vec2::from_ints(dx * reach, dy * reach))
    };
    let sees = |world: &World, reach: i32| {
        ring(reach)
            .iter()
            .filter(|&&spot| world.can_see_point(Team::Radiant, spot))
            .count()
    };
    assert!(sees(&world, WATCHER_DAY_VISION - 50) > 0, "800 by day");
    assert_eq!(sees(&world, WATCHER_DAY_VISION + 50), 0);
    assert!(sees(&world, WATCHER_NIGHT_VISION + 100) > 0);
    world.clock_shift.day = 5 * 60 * rules::TICKS_PER_SECOND;
    world.step();
    assert!(world.time_of_day().night);
    assert_eq!(sees(&world, WATCHER_NIGHT_VISION + 100), 0, "450 by night");
    assert!(sees(&world, WATCHER_NIGHT_VISION - 50) > 0);
    // A Dire creep in the light is seen by Radiant, as a unit.
    let spot = ring(WATCHER_NIGHT_VISION - 50)
        .into_iter()
        .find(|&spot| world.can_see_point(Team::Radiant, spot))
        .expect("a lit spot");
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, spot);
    world.settle();
    assert!(world.shows(Team::Radiant, creep));
    run(&mut world, LAMP_LIT_TICKS);
    assert_eq!(
        lit_for(&world, watcher),
        None,
        "seven minutes, then at rest"
    );
    world.transform.get_mut(creep).expect("standing").pos = spot;
    world.settle();
    assert!(!world.shows(Team::Radiant, creep));
}

#[test]
fn the_other_side_puts_a_lit_watcher_out_for_two_minutes() {
    let (mut world, watcher) = a_watcher_and_its_lighter();
    let at = world.transform.get(watcher).expect("standing").pos;
    light(&mut world, 0, watcher);
    place_hero(&mut world, 1, at + Vec2::from_ints(-100, 0));
    light(&mut world, 1, watcher);
    assert_eq!(lit_for(&world, watcher), None, "out");
    assert!(world.wears(watcher, ModifierKind::LampOff));
    light(&mut world, 0, watcher);
    assert_eq!(
        lit_for(&world, watcher),
        None,
        "nobody lights it while it is out"
    );
    run(&mut world, LAMP_OFF_TICKS);
    assert!(
        !world.wears(watcher, ModifierKind::LampOff),
        "at rest again"
    );
    light(&mut world, 1, watcher);
    assert_eq!(lit_for(&world, watcher), Some(Team::Dire));
}

/// The western Shrine, the clock `ahead` ticks before its activation `n`,
/// and every hero far from it.
fn the_shrine_before(activation: u32, ahead: u32, sides: &[Team]) -> (World, Entity) {
    let mut world = objectives_match(sides);
    let at = map_of(MapId(0)).shrines[0];
    let shrine = the(&world, UnitKind::Shrine, at);
    let due = SHRINE_FIRST_TICK + (activation - 1) * SHRINE_PERIOD_TICKS;
    world.clock_shift.game = due - world.tick - ahead - 1;
    world.step();
    assert_eq!(world.clock() + ahead, due);
    (world, shrine)
}

fn is_gathered(kind: &EventKind) -> bool {
    matches!(kind, EventKind::ShrineActivated { .. })
}

/// The experience a gathering told.
fn gathered(events: &[Event]) -> Option<(Team, i32, Option<EntityId>)> {
    events.iter().find_map(|event| match event.kind {
        EventKind::ShrineActivated { team, xp, by, .. } => Some((team, xp, by)),
        _ => None,
    })
}

#[test]
fn a_shrine_gathered_in_three_seconds_gives_two_hundred_then_five_hundred() {
    let ahead = SHRINE_COUNTDOWN_TICKS + 10;
    let (mut world, shrine) = the_shrine_before(1, ahead, &[Team::Radiant, Team::Dire]);
    let at = world.transform.get(shrine).expect("standing").pos;
    let radiant = hero(&world, 0);
    place(&mut world, radiant, at + Vec2::from_ints(200, 0));
    run(&mut world, ahead - 1);
    assert!(
        !world.wears(shrine, ModifierKind::ShrineReady),
        "not before 7:00"
    );
    assert!(
        world
            .zone
            .get(shrine)
            .is_some_and(|zone| zone.progress == 0)
    );
    world.step();
    assert!(world.wears(shrine, ModifierKind::ShrineReady), "7:00");
    let xp = world.seats[0].xp;
    let (ticks, events) =
        until(&mut world, 2 * SHRINE_COUNTDOWN_TICKS, is_gathered).expect("gathered");
    assert_eq!(ticks + 1, SHRINE_COUNTDOWN_TICKS, "three seconds standing");
    assert_eq!(
        gathered(&events),
        Some((Team::Radiant, 200, Some(wire_id(radiant))))
    );
    assert_eq!(world.seats[0].xp - xp, 200, "the only Radiant hero, once");
    assert!(
        visible_events(&events, Some(Team::Dire))
            .iter()
            .any(|kind| matches!(
                kind,
                EventKind::ShrineActivated {
                    by: None,
                    xp: 200,
                    ..
                }
            )),
        "told to the other side, without the gatherer"
    );
    assert!(
        !world.wears(shrine, ModifierKind::ShrineReady),
        "one charge"
    );

    let (mut world, shrine) = the_shrine_before(2, 0, &[Team::Radiant, Team::Dire]);
    place_hero(&mut world, 0, at + Vec2::from_ints(200, 0));
    let (_, events) =
        until(&mut world, 2 * SHRINE_COUNTDOWN_TICKS + 2, is_gathered).expect("gathered");
    assert_eq!(gathered(&events).map(|(_, xp, _)| xp), Some(500), "14:00");
    assert!(world.zone.get(shrine).is_some());
}

#[test]
fn a_shrine_rewards_a_gatherer_and_the_least_experienced_ally() {
    let (mut world, shrine) = the_shrine_before(1, 0, &[Team::Radiant, Team::Radiant, Team::Dire]);
    let at = world.transform.get(shrine).expect("standing").pos;
    place_hero(&mut world, 0, at + Vec2::from_ints(200, 0));
    let mut events = Vec::new();
    world.grant_xp(0, 100, &mut events);
    let before = [world.seats[0].xp, world.seats[1].xp];
    until(&mut world, 3 * SHRINE_COUNTDOWN_TICKS, is_gathered).expect("gathered");
    assert_eq!(
        [world.seats[0].xp - before[0], world.seats[1].xp - before[1]],
        [200, 200],
        "the gatherer, and the ally with the least experience but it"
    );
}

#[test]
fn an_enemy_in_the_shrine_runs_its_countdown_back() {
    let (mut world, shrine) = the_shrine_before(1, 0, &[Team::Radiant, Team::Dire]);
    let at = world.transform.get(shrine).expect("standing").pos;
    let (radiant, dire) = (hero(&world, 0), hero(&world, 1));
    place(&mut world, radiant, at + Vec2::from_ints(200, 0));
    run(&mut world, 1 + SHRINE_COUNTDOWN_TICKS / 2);
    let half = world.zone.get(shrine).expect("a zone").progress;
    assert_eq!(
        world.zone.get(shrine).and_then(|z| z.side),
        Some(Team::Radiant)
    );
    place(&mut world, dire, at + Vec2::from_ints(0, 200));
    run(&mut world, half / 2);
    assert_eq!(
        world.zone.get(shrine).map(|zone| zone.progress),
        Some(half - half / 2),
        "contested, it runs back"
    );
    place(&mut world, radiant, map_of(MapId(0)).fountains[0]);
    let (ticks, events) =
        until(&mut world, 3 * SHRINE_COUNTDOWN_TICKS, is_gathered).expect("gathered");
    assert_eq!(gathered(&events).map(|(team, ..)| team), Some(Team::Dire));
    assert_eq!(
        ticks,
        half - half / 2 + SHRINE_COUNTDOWN_TICKS,
        "back down to nothing for Radiant, then three seconds for Dire"
    );
}

#[test]
fn the_mid_only_map_stands_no_objective_and_its_heroes_hold_no_hidden_slot() {
    for (map, objectives) in [
        (MapId(2), map_of(MapId(2)).default_objectives),
        (MapId(0), ObjectiveSet::NONE),
    ] {
        let mut cfg = config();
        cfg.map = map;
        cfg.objectives = objectives;
        cfg.picks = fiend_against_fiend();
        let world = World::for_match(&cfg, cfg.rng());
        assert!(world.entities.iter().all(|entity| {
            !matches!(
                world.kind.get(entity),
                Some(UnitKind::Outpost | UnitKind::Watcher | UnitKind::Shrine)
            )
        }));
        let book = world.abilities.get(hero(&world, 0)).expect("a book");
        assert_eq!(book.slots.len(), 6, "{map:?}: the kit and nothing more");
    }
    let world = objectives_match(&[Team::Radiant, Team::Dire]);
    let count = |kind| {
        world
            .entities
            .iter()
            .filter(|&entity| world.kind.get(entity) == Some(&kind))
            .count()
    };
    assert_eq!(
        [
            count(UnitKind::Outpost),
            count(UnitKind::Watcher),
            count(UnitKind::Shrine)
        ],
        [2, 10, 2]
    );
    assert!(
        map_of(MapId(0))
            .default_objectives
            .has(ObjectiveKind::Outposts)
    );
    let holders: Vec<Team> = map_of(MapId(0))
        .outposts
        .iter()
        .map(|outpost| {
            let it = the(&world, UnitKind::Outpost, outpost.at);
            *world.team.get(it).expect("a side")
        })
        .collect();
    assert_eq!(holders, [Team::Radiant, Team::Dire], "each holds its home");
    let fiend = shown(&world, Some(Team::Radiant), hero(&world, 0));
    assert_eq!(
        fiend.abilities.len(),
        6,
        "the hidden slots are shown to no one"
    );
}

#[test]
fn a_watcher_by_a_lane_creep_opens_no_early_aggro_as_a_neutral_creep_does() {
    // Before the free-aggro mark a lane creep is called on only when it is
    // already busy: an enemy lane creep or a neutral creep within reach.
    let aggroable = |beside: fn(&mut World, Vec2)| {
        let mut world = World::new();
        let at = an_empty_spot(&world);
        let creep = thinking_creep_of(&mut world, Team::Radiant, at);
        beside(&mut world, at + Vec2::from_ints(0, 150));
        world.settle();
        assert!(world.clock() < rules::FREE_AGGRO_TICK);
        world.aggroable_yet(creep)
    };
    assert!(!aggroable(|_, _| {}), "alone");
    assert!(aggroable(|world, at| {
        world.spawn_neutral(NeutralKind::Kobold, at, 0, 0);
    }));
    assert!(!aggroable(|world, at| {
        world.spawn_objective(&WATCHER, Team::Neutral, at);
    }));
}

#[test]
fn a_glyph_fortifies_the_buildings_and_leaves_a_held_outpost_bare() {
    let mut world = objectives_match(&[Team::Radiant, Team::Dire]);
    let map = map_of(MapId(0));
    let outpost = the(&world, UnitKind::Outpost, map.outposts[0].at);
    assert_eq!(world.team.get(outpost), Some(&Team::Radiant), "held");
    let tower = structure_at(&world, map.radiant_towers[0].2);
    world.glyphs[0].ready_at = 0;
    give(&mut world, &[(0, Order::Glyph)]);
    assert!(world.wears(tower, ModifierKind::Fortified), "a tower");
    for (kind, at) in [
        (UnitKind::Filler, map.fillers[0][0]),
        (UnitKind::Fountain, map.fountains[0]),
    ] {
        let it = the(&world, kind, at);
        assert!(world.wears(it, ModifierKind::Fortified), "{kind:?}");
    }
    assert!(
        !world.wears(outpost, ModifierKind::Fortified),
        "not the Outpost"
    );
    for (kind, at) in [
        (UnitKind::Watcher, map.watchers[4]),
        (UnitKind::Shrine, map.shrines[0]),
    ] {
        let it = the(&world, kind, at);
        assert!(!world.wears(it, ModifierKind::Fortified), "{kind:?}");
    }
}
