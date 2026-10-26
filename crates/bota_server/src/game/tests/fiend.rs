//! Shadow Fiend: the razes, the souls, the presence and the requiem.

use bota_core::{MELEE_CREEP, ModifierKind, isqrt64, presence_armor, rules, tower_def};

use crate::game::{ActionState, Command, Entity, UnitOrder, World, wire_id};
use bota_proto::{
    AbilityId, CastView, EffectId, EventKind, Fixed, HeroId, Inflictor, Order, SlotId, StatusFlags,
    Target, Team, Vec2,
};

use super::support::*;

/// The slot each of Shadow Fiend's abilities sits in.
const RAZE_NEAR_SLOT: u8 = 0;
const RAZE_MID_SLOT: u8 = 1;
const RAZE_FAR_SLOT: u8 = 2;
const FEAST_SLOT: u8 = 3;
const PRESENCE_SLOT: usize = 4;
const REQUIEM_SLOT: u8 = 5;

#[test]
fn the_kit_is_three_razes_the_feast_the_presence_and_the_requiem_with_no_slot_for_the_souls() {
    let (world, fiend, _mark) = fiend_and_a_mark(400);
    let ids: Vec<u16> = world
        .abilities
        .get(fiend)
        .expect("he carries a kit")
        .slots
        .iter()
        .map(|held| held.id.0)
        .collect();
    assert_eq!(ids, vec![9, 10, 11, 15, 14, 12]);
}

#[test]
fn the_necromastery_gathers_from_the_first_level_with_no_point_spent() {
    let mut world = World::new();
    let fiend = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let creep = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5400, 5000));
    world.settle();
    assert_eq!(world.points_spent(fiend), 0, "nothing learned");
    world.bury(vec![(creep, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), rules::SOULS_PER_UNIT);
    let shown = shown(&world, Some(Team::Radiant), fiend);
    assert!(
        shown
            .abilities
            .iter()
            .all(|slot| slot.id != AbilityId::NECROMASTERY),
        "and the necromastery is in no slot"
    );
    assert!(
        shown.abilities[usize::from(FEAST_SLOT)].learnable == Some(true),
        "while his first point may go into the feast"
    );
}

#[test]
fn a_raze_burns_what_stands_where_it_lands_and_nothing_else() {
    let mid = rules::RAZE_DISTANCE[1];
    // The mark stands `apart` east, another creep `other` east; one of them
    // stands where the raze lands.
    for (case, apart, other, slot, mark_struck) in [
        ("a creep past it", mid, 900, RAZE_MID_SLOT, true),
        (
            "one under his feet",
            50,
            rules::RAZE_DISTANCE[2],
            RAZE_FAR_SLOT,
            false,
        ),
        ("one behind him", mid, -mid, RAZE_MID_SLOT, true),
    ] {
        let (mut world, _fiend, mark) = fiend_and_a_mark(apart);
        let other = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5000 + other, 5000));
        let (struck, spared) = if mark_struck {
            (mark, other)
        } else {
            (other, mark)
        };
        let events = cast_through(&mut world, slot, rules::RAZE_CAST_POINT_MS);
        assert!(!burns_on(&events, struck).is_empty(), "{case}: felt");
        assert!(burns_on(&events, spared).is_empty(), "{case}: spared");
    }
}

#[test]
fn whoever_sees_a_raze_in_its_cast_point_is_told_which_and_when_it_goes_off() {
    // Dota shows each raze's own animation to everyone who sees the caster.
    let (mut world, fiend, _mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
    a_creep_at(&mut world, Team::Dire, Vec2::from_ints(4600, 5000));
    world.step();
    cast(&mut world, RAZE_MID_SLOT, Target::None);
    let point = ticks_of(rules::RAZE_CAST_POINT_MS);
    for went in 0..point {
        let told = Some(CastView {
            ability: Some(AbilityId::RAZE_MID),
            goes_off_in: point - went,
        });
        for side in [Team::Radiant, Team::Dire] {
            assert_eq!(
                shown(&world, Some(side), fiend).cast,
                told,
                "{side:?}, {went} ticks in"
            );
        }
        world.step();
    }
    assert_eq!(
        shown(&world, Some(Team::Dire), fiend).cast,
        None,
        "and nothing once it has gone off"
    );
}

#[test]
fn a_raze_stands_through_its_cast_point_and_lands_when_it_is_over() {
    let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
    let stood = world.transform.get(fiend).expect("standing").pos;
    let mana = world.mana.get(fiend).expect("a pool").mana;
    cast(&mut world, RAZE_NEAR_SLOT, Target::None);
    let point = ticks_of(rules::RAZE_CAST_POINT_MS);
    assert_eq!(point, 17, "0.55 s at thirty ticks a second, rounded up");
    for tick in 1..point {
        let events = world.step();
        assert!(burns_on(&events, mark).is_empty(), "nothing yet at {tick}");
        assert_ne!(
            own_statuses(&world, fiend) & StatusFlags::CASTING,
            0,
            "casting on the wire at {tick}"
        );
        assert_eq!(world.mana.get(fiend).expect("a pool").mana, mana);
    }
    let events = world.step();
    assert_eq!(
        burns_on(&events, mark),
        vec![rules::RAZE_DAMAGE[0]],
        "it lands on the tick its cast point runs out"
    );
    assert!(
        events.iter().any(|event| matches!(
            event.kind,
            EventKind::AbilityCast {
                ability: AbilityId::RAZE_NEAR,
                ..
            }
        )),
        "and is told of as cast then"
    );
    assert_eq!(own_statuses(&world, fiend) & StatusFlags::CASTING, 0);
    assert_eq!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "a raze takes no aim, so there is nothing to walk into"
    );
    assert_eq!(
        cooldown(&world, fiend, RAZE_NEAR_SLOT),
        rules::RAZE_COOLDOWN[0]
    );
    assert_eq!(
        world.mana.get(fiend).expect("a pool").mana,
        mana - Fixed::from_int(rules::RAZE_MANA[0])
    );
}

#[test]
fn an_order_to_the_body_in_the_cast_point_breaks_the_raze_off_with_nothing_spent() {
    let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
    let mana = world.mana.get(fiend).expect("a pool").mana;
    cast(&mut world, RAZE_NEAR_SLOT, Target::None);
    run(&mut world, 5);
    stop_click(&mut world);
    let mut burnt = Vec::new();
    for _ in 0..30 {
        burnt.extend(burns_on(&world.step(), mark));
    }
    assert!(burnt.is_empty(), "the raze never goes off");
    assert_eq!(cooldown(&world, fiend, RAZE_NEAR_SLOT), 0);
    assert_eq!(world.mana.get(fiend).expect("a pool").mana, mana);
}

/// Seat 0 gives its hero an order, queued or not, and the world advances one tick.
fn give(world: &mut World, order: Order, queue: bool) -> Vec<crate::game::Event> {
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order,
        queue,
        seq: 0,
    }])
}

/// The abilities that went off in some events, in order.
fn went_off(events: &[crate::game::Event]) -> Vec<AbilityId> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::AbilityCast { ability, .. } => Some(ability),
            _ => None,
        })
        .collect()
}

fn walk_to_the_north() -> Order {
    Order::Move {
        target: Target::Pos(Vec2::from_ints(5000, 5400)),
    }
}

/// Dota keeps a cast point to the cast: only a stop or a hold, or a disable,
/// breaks it off. Any other order given inside it waits and runs once the
/// cast has gone off (Liquipedia, *Turn Rate*, "gets queued up and executed
/// once the spell is cast"; the Dota update of 2017-05-31), so a move given
/// right after a raze lets it land and walks from the tick it does, a raze
/// having no backswing.
#[test]
fn a_move_in_the_cast_point_lets_the_raze_land_and_walks_from_that_tick() {
    // An attack-move stops for the creep in reach, so it lands the raze and
    // stands to swing.
    let orders = [
        (walk_to_the_north(), true),
        (
            Order::MoveDirection {
                toward: Vec2::from_ints(5000, 5400),
            },
            true,
        ),
        (
            Order::Attack {
                target: Target::Pos(Vec2::from_ints(5000, 5400)),
            },
            false,
        ),
    ];
    for (order, walks) in orders {
        for queue in [false, true] {
            let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
            let (mana, stood) = (
                world.mana.get(fiend).expect("a pool").mana,
                world.transform.get(fiend).expect("standing").pos,
            );
            cast(&mut world, RAZE_NEAR_SLOT, Target::None);
            run(&mut world, 5);
            give(&mut world, order, queue);
            let mut burnt = Vec::new();
            let mut walked_by = None;
            for tick in 0..60 {
                burnt.extend(burns_on(&world.step(), mark));
                let at = world.transform.get(fiend).expect("standing").pos;
                if walked_by.is_none() && at != stood {
                    walked_by = Some(tick);
                }
            }
            assert_eq!(burnt, vec![rules::RAZE_DAMAGE[0]], "{order:?}: it lands");
            assert!(
                cooldown(&world, fiend, RAZE_NEAR_SLOT) > 0,
                "and is paid for"
            );
            assert!(world.mana.get(fiend).expect("a pool").mana <= mana);
            // The cast point has 17 - 6 ticks left; the order runs on the
            // tick it ends on, so the first step follows at once.
            let cast_tick = ticks_of(rules::RAZE_CAST_POINT_MS) - 6;
            assert!(
                !walks || walked_by.is_some_and(|tick| tick <= cast_tick + 1),
                "{order:?} queue {queue}: walked from {walked_by:?}, the cast at {cast_tick}"
            );
        }
    }
}

#[test]
fn a_hold_in_the_cast_point_breaks_the_raze_off_with_nothing_spent() {
    let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
    let mana = world.mana.get(fiend).expect("a pool").mana;
    cast(&mut world, RAZE_NEAR_SLOT, Target::None);
    run(&mut world, 5);
    give(
        &mut world,
        Order::Attack {
            target: Target::None,
        },
        false,
    );
    let mut burnt = Vec::new();
    for _ in 0..30 {
        burnt.extend(burns_on(&world.step(), mark));
    }
    assert!(burnt.is_empty(), "the raze never goes off");
    assert_eq!(cooldown(&world, fiend, RAZE_NEAR_SLOT), 0);
    assert!(world.mana.get(fiend).expect("a pool").mana >= mana);
}

/// Casts given inside a cast point go off in turn (the 2017 update): Q, W and E
/// pressed at once give three razes; a move given after them replaces the
/// ones still waiting, the cast in hand still landing; a cast given after a
/// move replaces the move.
#[test]
fn razes_given_in_a_cast_point_go_off_in_turn_and_a_move_drops_the_waiting_ones() {
    let razes = [RAZE_NEAR_SLOT, RAZE_MID_SLOT, RAZE_FAR_SLOT];
    let cast_order = |slot: u8| Order::Cast {
        slot: bota_proto::AbilitySlot(slot),
        target: Target::None,
    };
    let play = |orders: &[Order]| {
        let (mut world, fiend, _) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
        world.mana.get_mut(fiend).expect("a pool").mana = Fixed::from_int(2000);
        cast(&mut world, razes[0], Target::None);
        for order in orders {
            give(&mut world, *order, false);
        }
        let mut gone = Vec::new();
        for _ in 0..120 {
            gone.extend(went_off(&world.step()));
        }
        let at = world.transform.get(fiend).expect("standing").pos;
        (gone, at != Vec2::from_ints(5000, 5000))
    };
    let (gone, walked) = play(&[cast_order(razes[1]), cast_order(razes[2])]);
    assert_eq!(
        gone,
        vec![
            AbilityId::RAZE_NEAR,
            AbilityId::RAZE_MID,
            AbilityId::RAZE_FAR
        ],
        "the three go off in turn"
    );
    assert!(!walked);
    let (gone, walked) = play(&[cast_order(razes[1]), walk_to_the_north()]);
    assert_eq!(
        gone,
        vec![AbilityId::RAZE_NEAR],
        "the move replaced the waiting raze, the one in hand landed"
    );
    assert!(walked, "and runs after the one in hand");
    let (gone, walked) = play(&[walk_to_the_north(), cast_order(razes[2])]);
    assert_eq!(
        gone,
        vec![AbilityId::RAZE_NEAR, AbilityId::RAZE_FAR],
        "a cast replaces the move"
    );
    assert!(!walked);
}

/// A disable breaks the cast off as before; what was given in the cast point
/// runs when the disable ends.
#[test]
fn a_stun_in_the_cast_point_breaks_the_raze_off_and_the_waiting_move_runs_after() {
    let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
    let (mana, stood) = (
        world.mana.get(fiend).expect("a pool").mana,
        world.transform.get(fiend).expect("standing").pos,
    );
    cast(&mut world, RAZE_NEAR_SLOT, Target::None);
    run(&mut world, 3);
    give(&mut world, walk_to_the_north(), false);
    put_on(&mut world, fiend, ModifierKind::Stunned, 60);
    let mut burnt = Vec::new();
    for _ in 0..50 {
        burnt.extend(burns_on(&world.step(), mark));
    }
    assert!(burnt.is_empty(), "the raze never goes off");
    assert_eq!(cooldown(&world, fiend, RAZE_NEAR_SLOT), 0);
    assert!(world.mana.get(fiend).expect("a pool").mana >= mana);
    assert_eq!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "stunned, it has not walked"
    );
    run(&mut world, 60);
    assert_ne!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "and walks when the stun is over"
    );
}

#[test]
fn every_soul_held_adds_two_to_a_raze() {
    for held in [0, 7, rules::NECRO_SOUL_CAP] {
        let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
        hand_souls(&mut world, fiend, held);
        let events = cast_through(&mut world, RAZE_MID_SLOT, rules::RAZE_CAST_POINT_MS);
        assert_eq!(
            burns_on(&events, mark),
            vec![85 + 2 * held as i32],
            "{held} souls, a creep with no magic resistance"
        );
    }
}

#[test]
fn a_raze_or_a_requiem_burns_the_creep_at_a_tower_and_leaves_the_tower_whole() {
    let mid = rules::RAZE_DISTANCE[1];
    for (case, slot, point, apart, tower_at, ticks) in [
        (
            "raze",
            RAZE_MID_SLOT,
            rules::RAZE_CAST_POINT_MS,
            mid,
            Vec2::from_ints(5000 + mid, 5200),
            0,
        ),
        (
            "requiem",
            REQUIEM_SLOT,
            rules::REQUIEM_CAST_POINT_MS,
            400,
            Vec2::from_ints(5700, 5000),
            60,
        ),
    ] {
        let (mut world, fiend, creep) = fiend_and_a_mark(apart);
        let tower = world.spawn_unit(tower_def(1), Team::Dire, tower_at);
        world.settle();
        hand_souls(&mut world, fiend, 5);
        world.step();
        let (was_creep, was_tower) = (hp(&world, creep), hp(&world, tower));
        cast_through(&mut world, slot, point);
        run(&mut world, ticks);
        assert!(
            hp(&world, creep) < was_creep,
            "{case}: the creep at the tower feels it"
        );
        assert_eq!(hp(&world, tower), was_tower, "{case}: the tower does not");
        assert!(
            world
                .modifiers
                .get(tower)
                .is_none_or(|on_it| on_it.active().all(|held| held.source != Some(fiend))),
            "{case}: nor does it keep a stack, a fear or a slow"
        );
    }
}

#[test]
fn souls_come_only_from_the_units_the_gatherer_brings_down() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let other = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5400, 5000));
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5600, 5000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(other, None)], &mut events);
    assert_eq!(
        souls(&world, fiend),
        0,
        "a death nobody is answerable for is worth nothing"
    );
    world.bury(vec![(tower, Some(fiend))], &mut events);
    assert_eq!(souls(&world, fiend), 0, "nor is a structure");
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        souls(&world, fiend),
        rules::SOULS_PER_UNIT,
        "and a creep he brought down is worth its soul"
    );
}

#[test]
fn a_broken_fiend_keeps_what_his_souls_are_worth_but_gathers_no_new_one() {
    // Liquipedia, Break: Necromastery "Does not grant stacks while disabled
    // by Break. However, their already existing stacks still fully work."
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 10);
    world.step();
    let worth = world.stats.get(fiend).map(|s| s.damage);
    put_on(&mut world, fiend, ModifierKind::Broken, 600);
    world.step();
    assert_eq!(
        world.stats.get(fiend).map(|s| s.damage),
        worth,
        "broken, his ten souls deal their damage on"
    );
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        souls(&world, fiend),
        10,
        "but the creep he brings down is worth none"
    );
}

#[test]
fn a_hero_brought_down_is_worth_four_souls() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let victim = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5400, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.bury(vec![(victim, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), 4);
}

#[test]
fn souls_stop_at_twenty_whatever_the_level() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    assert_eq!(world.soul_cap(fiend), 20);
    hand_souls(&mut world, fiend, 20);
    world.bury(vec![(mark, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), 20, "no more are held than the cap");
}

#[test]
fn a_soul_is_worth_one_damage_and_point_eight_more_every_six_levels() {
    // Hero level, souls held, attack damage they add: 1 + 0.8 per six whole
    // levels a soul, the sum rounded down.
    for (level, held, added) in [
        (1, 5, 5),
        (5, 20, 20),
        (6, 20, 36),
        (6, 7, 12),
        (12, 20, 52),
        (18, 20, 68),
        (24, 20, 84),
        (30, 20, 100),
    ] {
        let (mut world, fiend, _mark) = fiend_and_a_mark(400);
        world.level.insert(fiend, crate::game::Level(level));
        world.step();
        let bare = world.stats.get(fiend).expect("settled").damage;
        hand_souls(&mut world, fiend, held);
        world.step();
        assert_eq!(
            world.stats.get(fiend).map(|s| s.damage),
            Some(bare + added),
            "level {level}, {held} souls"
        );
    }
}

/// Dota's presence asks no sight: an enemy within its reach wears it whether
/// its side sees him or not, and one beyond its reach wears nothing.
#[test]
fn the_presence_wears_down_the_armor_of_enemies_whether_they_see_its_carrier_or_not() {
    // He stands on high ground, which the creep 400 east of him is below.
    let (mut world, fiend, below) = fiend_and_a_mark(400);
    // Standing, he strikes nothing, and a strike would show him.
    world.set_order(fiend, UnitOrder::Stand);
    let out = a_creep_at(
        &mut world,
        Team::Dire,
        Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
    );
    run(&mut world, rules::PRESENCE_LINGER_TICKS);
    assert!(
        !world.can_see(Team::Dire, fiend),
        "the Dire side is blind to him"
    );
    let armor = |world: &World, unit| world.stats.get(unit).expect("standing").armor;
    assert_eq!(
        armor(&world, below),
        armor(&world, out) - Fixed::from_ratio(5, 2),
        "blind to him, it still wears two and a half armor off at his first level"
    );
    a_creep_at(&mut world, Team::Dire, Vec2::from_ints(4600, 5000));
    world.step();
    world.step();
    assert!(
        world.can_see(Team::Dire, fiend),
        "a creep beside him sees him"
    );
    assert_eq!(
        armor(&world, below),
        armor(&world, out) - Fixed::from_ratio(5, 2),
        "and the same once its side sees him"
    );
}

#[test]
fn a_broken_fiend_wears_nobody_down_until_the_break_lifts() {
    // A Hill Troll Berserker's Break turns his passives off, the presence's
    // aura with them, as Dota's does.
    let (mut world, fiend, below) = fiend_and_a_mark(400);
    world.set_order(fiend, UnitOrder::Stand);
    a_creep_at(&mut world, Team::Dire, Vec2::from_ints(4600, 5000));
    let out = a_creep_at(
        &mut world,
        Team::Dire,
        Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
    );
    run(&mut world, 2);
    let armor = |world: &World, unit| world.stats.get(unit).expect("standing").armor;
    let torn = armor(&world, out) - Fixed::from_ratio(5, 2);
    assert_eq!(armor(&world, below), torn, "the presence holds");
    let broken = 4 * rules::PRESENCE_LINGER_TICKS;
    put_on(&mut world, fiend, ModifierKind::Broken, broken);
    run(&mut world, rules::PRESENCE_LINGER_TICKS + 1);
    assert_eq!(
        armor(&world, below),
        armor(&world, out),
        "broken, he lays nothing, and what he laid runs out"
    );
    run(&mut world, broken);
    assert_eq!(
        armor(&world, below),
        torn,
        "and it is back once the break lifts"
    );
}

#[test]
fn of_two_presences_on_one_enemy_the_one_tearing_more_armor_holds() {
    // Two instances of one aura do not stack and the stronger applies
    // (Liquipedia, Aura). The weaker fiend stands later in the slots, so his
    // presence goes on after the stronger one's every tick.
    let mut world = World::new();
    let stronger = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let weaker = hero_at(&mut world, Vec2::from_ints(4800, 5000));
    set_level(&mut world, stronger, PRESENCE_SLOT, 4);
    set_level(&mut world, weaker, PRESENCE_SLOT, 1);
    for fiend in [stronger, weaker] {
        world.set_order(fiend, UnitOrder::Stand);
    }
    let creep = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(4600, 5000));
    run(&mut world, 2);
    assert!(
        [stronger, weaker]
            .iter()
            .all(|fiend| world.can_see(Team::Dire, *fiend)),
        "the creep sees both"
    );
    assert_eq!(
        world.stats.get(creep).expect("standing").armor,
        Fixed::from_int(MELEE_CREEP.armor) - presence_armor(4, false),
        "the fourth level's seven armor, not the first's two and a half"
    );
}

#[test]
fn armor_the_presence_takes_below_zero_lets_his_swings_land_harder() {
    // A melee creep's 2 armor less the fourth level's 7 is -5: Dota's
    // 2 - 1 / (1 + 0.06 * 5) of a swing comes through. A swing is 41 to 47:
    // 16 to 22 base and 25 agility.
    for (presence, over, under) in [(0, 100, 112), (4, 160, 130)] {
        let (mut world, fiend, mark) = fiend_and_a_mark(-400);
        set_level(&mut world, fiend, 4, presence);
        world.step();
        let taken = one_swing_takes(&mut world, fiend, mark);
        assert!(
            (41 * over / under..=47 * over / under).contains(&taken),
            "presence at level {presence}: {taken}"
        );
    }
}

#[test]
fn souls_outlive_the_death_of_the_one_who_gathered_them_but_for_a_share() {
    for (held, kept) in [(rules::SOULS_PER_UNIT, rules::SOULS_PER_UNIT), (10, 7)] {
        let (mut world, fiend, _mark) = fiend_and_a_mark(400);
        hand_souls(&mut world, fiend, held);
        world.bury(vec![(fiend, None)], &mut Vec::new());
        assert_eq!(world.seats[0].unit, None, "the body is gone");
        let back = back_on_its_feet(&mut world);
        assert_eq!(souls(&world, back), kept, "{held} held, {kept} come back");
    }
}

#[test]
fn a_raze_leaves_its_mark_where_it_lands_for_whoever_sees_the_spot() {
    // The creep stands past its own sight of the fiend, and the far raze
    // lands beside it.
    let apart = rules::RAZE_DISTANCE[2] + 150;
    assert!(
        apart > rules::CREEP_VISION,
        "the fiend is out of the creep's sight"
    );
    let (mut world, _fiend, _mark) = fiend_and_a_mark(apart);
    cast_through(&mut world, RAZE_FAR_SLOT, rules::RAZE_CAST_POINT_MS);
    // What a side sees is worked out before a cast goes off.
    world.step();
    let dire = world.view(Team::Dire);
    let landed = dire
        .projectiles
        .iter()
        .find(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::RAZE_FAR)))
        .expect("the Dire side sees the raze where it landed");
    assert_eq!(
        landed.pos,
        Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000)
    );
    assert!(
        dire.units
            .iter()
            .all(|shown| shown.hero != Some(HeroId::SHADOW_FIEND)),
        "and still does not see the fiend"
    );
    run(&mut world, rules::MARK_TICKS);
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .all(|shown| shown.inflictor != Some(Inflictor::Ability(AbilityId::RAZE_FAR))),
        "the mark is gone once its ticks have run"
    );
}

#[test]
fn the_view_carries_what_has_been_gathered_as_a_counted_effect() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.step();
    let view = world.view(Team::Radiant);
    let shown = view
        .units
        .iter()
        .find(|u| u.hero == Some(HeroId::SHADOW_FIEND))
        .expect("he is in the view");
    let souls = shown
        .effects
        .iter()
        .find(|e| e.id == EffectId::SOULS)
        .expect("what is gathered is on him");
    assert_eq!(souls.stacks, Some(rules::SOULS_PER_UNIT));
    assert_eq!(souls.ticks_left, None, "and nothing counts it down");
}

#[test]
fn a_requiem_lets_a_line_fly_for_every_soul_and_a_line_burns_what_it_crosses_once() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = hp(&world, mark);
    let flying = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::REQUIEM)))
            .count()
    };
    cast(&mut world, REQUIEM_SLOT, Target::None);
    for _ in 1..ticks_of(rules::REQUIEM_CAST_POINT_MS) {
        world.step();
        assert_eq!(flying(&world), 0, "nothing flies in the cast point");
    }
    // The first line flies due east, where the mark stands; the others fly
    // wide of it. His swings at it are not the requiem's and are left out.
    let mut burns = burns_on(&world.step(), mark);
    assert_eq!(flying(&world), 5, "one line to a soul");
    assert_eq!(souls(&world, fiend), 5, "and the souls are kept");
    for _ in 0..60 {
        burns.extend(burns_on(&world.step(), mark));
    }
    assert_eq!(
        burns,
        vec![rules::REQUIEM_LINE_DAMAGE[0]],
        "one line's worth, once"
    );
    assert!(hp(&world, mark) < full, "and it was felt");
    assert_eq!(flying(&world), 0, "and the lines have flown out");
}

#[test]
fn what_stands_at_the_fiend_is_crossed_by_every_line_and_held_the_longest() {
    let (mut world, fiend, mark) = fiend_and_a_mark(60);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = hp(&world, mark);
    cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
    world.step();
    let taken = full - hp(&world, mark);
    assert_eq!(
        taken,
        Fixed::from_int(5 * rules::REQUIEM_LINE_DAMAGE[0]),
        "five lines, five burns"
    );
    let held = |kind: ModifierKind| {
        world.modifiers.get(mark).and_then(|on_it| {
            on_it
                .active()
                .find(|held| {
                    std::mem::discriminant(&held.kind) == std::mem::discriminant(&kind)
                        && held.source == Some(fiend)
                })
                .and_then(|held| held.ticks_left)
        })
    };
    for kind in [
        ModifierKind::Feared,
        ModifierKind::Slowed { pct: 0 },
        ModifierKind::MagicBroken { pct: 0 },
    ] {
        assert_eq!(held(kind), Some(65), "five lines' worth, held 2.15 s");
    }
    let projected = shown(&world, Some(Team::Dire), mark);
    assert_ne!(
        projected.statuses.bits & StatusFlags::FEARED,
        0,
        "and the fear is on the wire"
    );
}

#[test]
fn a_requiem_line_tears_magic_resistance_by_a_share_of_what_it_keeps_out() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(1500);
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    hand_souls(&mut world, fiend, 1);
    world.step();
    let resist = |world: &World| shown(world, None, foe).magic_resist;
    let own = world.stats.get(foe).expect("settled").magic_resist_bp;
    assert!(own > 2_500, "a hero's quarter and its intelligence's");
    assert_eq!(resist(&world), Fixed::from_ratio(own, rules::NOMINAL_BP));
    cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
    run(&mut world, 10);
    // 1 - (1 - own) * 1.05: what came through grows by five percent.
    let torn = rules::NOMINAL_BP - (rules::NOMINAL_BP - own) * 105 / 100;
    assert_eq!(resist(&world), Fixed::from_ratio(torn, rules::NOMINAL_BP));
    let shown = shown(&world, None, foe);
    assert!(
        shown
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::MAGIC_BROKEN && effect.ticks_left.is_some())
    );
}

#[test]
fn what_the_requiem_frightens_runs_from_the_fiend_and_swings_at_nothing() {
    let (mut world, fiend, mark) = fiend_and_a_mark(150);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let apart = |world: &World| {
        isqrt64(
            world
                .transform
                .get(fiend)
                .expect("standing")
                .pos
                .distance_squared(world.transform.get(mark).expect("standing").pos),
        )
    };
    cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
    world.step();
    assert!(
        world.feared(mark),
        "the first line crossing it frightens it"
    );
    let before = apart(&world);
    for _ in 0..10 {
        let blows = world
            .step()
            .into_iter()
            .filter(|event| {
                matches!(event.kind, EventKind::Damaged { source, .. } if source == Some(wire_id(mark)))
            })
            .count();
        assert_eq!(blows, 0, "feared, it lands no blow");
        assert!(
            !matches!(
                world.action.get(mark).map(|action| action.state),
                Some(ActionState::Attack {
                    phase: crate::game::ActionPhase::Before { .. },
                    ..
                })
            ),
            "and begins no swing"
        );
    }
    let after = apart(&world);
    // Ten ticks, a few of them spent turning round.
    assert!(
        after > before + Fixed::from_int(40).raw as i64,
        "and it has run: {before} then {after}"
    );
}

#[test]
fn a_requiem_with_no_souls_gathered_touches_nobody() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let full = hp(&world, mark);
    cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
    world.step();
    assert_eq!(hp(&world, mark), full, "nothing gathered is nothing let go");
    assert!(
        !carries(&world, mark, ModifierKind::Slowed { pct: 0 }),
        "not even the slow"
    );
    assert!(
        cooldown(&world, fiend, REQUIEM_SLOT) > 0,
        "though the cast itself happened"
    );
}

#[test]
fn a_fallen_fiend_lets_half_his_kept_souls_fly_whatever_the_cooldown_and_they_only_burn() {
    let (mut world, fiend, mark) = fiend_and_a_mark(60);
    hand_souls(&mut world, fiend, 10);
    if let Some(book) = world.abilities.get_mut(fiend) {
        book.slots[usize::from(REQUIEM_SLOT)].cooldown = 1000;
    }
    world.step();
    let full = hp(&world, mark);
    world.bury(vec![(fiend, None)], &mut Vec::new());
    let lines = world
        .view_full()
        .projectiles
        .iter()
        .filter(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::REQUIEM)))
        .count();
    // Ten held, three lost to the death, half of the seven kept fly.
    assert_eq!(lines, 3);
    world.step();
    assert_eq!(
        full - hp(&world, mark),
        Fixed::from_int(3 * rules::REQUIEM_LINE_DAMAGE[0]),
        "what stands where he fell is crossed by every line"
    );
    assert!(!world.feared(mark), "and is not frightened");
    assert!(!carries(&world, mark, ModifierKind::Slowed { pct: 0 }));
    assert!(!carries(&world, mark, ModifierKind::MagicBroken { pct: 0 }));
}

#[test]
fn a_fallen_fiend_with_no_requiem_learned_lets_nothing_fly() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(60);
    hand_souls(&mut world, fiend, 20);
    set_level(&mut world, fiend, usize::from(REQUIEM_SLOT), 0);
    world.bury(vec![(fiend, None)], &mut Vec::new());
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .all(|shown| shown.inflictor != Some(Inflictor::Ability(AbilityId::REQUIEM)))
    );
}

/// The statuses a unit shows the whole world.
fn own_statuses(world: &World, unit: Entity) -> u16 {
    world
        .view_full()
        .units
        .into_iter()
        .find(|shown| shown.id == wire_id(unit))
        .map_or(0, |shown| shown.statuses.bits)
}

fn cooldown(world: &World, hero: Entity, slot: u8) -> u32 {
    world
        .abilities
        .get(hero)
        .map_or(0, |book| book.slots[usize::from(slot)].cooldown)
}

fn hp(world: &World, unit: Entity) -> Fixed {
    world.health.get(unit).expect("standing").hp
}

#[test]
fn a_raze_burns_what_its_side_cannot_see_but_never_a_ward() {
    let (mut world, _, spot, slot) = fiend_across_a_cliff(true);
    let creep = world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, spot);
    let ward = world.spawn_ward(
        &bota_core::OBSERVER_WARD,
        (Team::Radiant, spot),
        1_000,
        (None, None),
    );
    world.settle();
    assert!(
        !world.can_see(Team::Dire, creep),
        "from below, the creep up the cliff is fog"
    );
    let hp = |world: &World, unit| world.health.get(unit).expect("standing").hp;
    let (creep_before, ward_before) = (hp(&world, creep), hp(&world, ward));
    cast(&mut world, slot, Target::None);
    let struck = (0..60).any(|_| {
        world.step();
        hp(&world, creep) < creep_before
    });
    assert!(struck, "an area strikes what stands in it, seen or not");
    assert_eq!(
        hp(&world, ward),
        ward_before,
        "a ward is brought down by attacks alone"
    );
}

#[test]
fn a_blood_grenade_bursts_on_what_its_side_cannot_see() {
    let (mut world, fiend, spot, _) = fiend_across_a_cliff(true);
    let creep = world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, spot);
    hand_item(&mut world, fiend, bota_proto::ItemId::BLOOD_GRENADE, 1);
    world.settle();
    assert!(
        !world.can_see(Team::Dire, creep),
        "from below, the creep up the cliff is fog"
    );
    let hp = |world: &World| world.health.get(creep).expect("standing").hp;
    let before = hp(&world);
    seat_order(
        &mut world,
        bota_proto::Order::Use {
            slot: bota_proto::ItemSlot(0),
            target: Target::Pos(spot),
        },
    );
    let struck = (0..90).any(|_| {
        world.step();
        hp(&world) < before
    });
    assert!(struck, "an area strikes what stands in it, seen or not");
}
