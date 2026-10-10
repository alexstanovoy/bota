//! Shadow Fiend: the razes, the souls, the presence and the requiem.

use crate::game::{
    ActionState, Entity, Event, ModifierKind, StackKind, UnitOrder, World, ability, isqrt64, rules,
    tower_def, wire_id,
};
use bota_proto::{
    DamageKind, EffectId, EventKind, Fixed, HeroId, Order, SlotId, StatusFlags, Target, Team, Vec2,
};

use super::support::*;

/// The slot each of Shadow Fiend's abilities sits in.
const RAZE_NEAR_SLOT: u8 = 0;
const RAZE_MID_SLOT: u8 = 1;
const RAZE_FAR_SLOT: u8 = 2;
const FEAST_SLOT: u8 = 3;
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
    assert_eq!(ids, vec![13, 14, 15, 19, 18, 16]);
}

#[test]
fn the_necromastery_gathers_from_the_first_level_with_no_point_spent() {
    let mut world = World::new();
    let fiend = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId(2));
    let creep = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5400, 5000));
    world.settle();
    assert_eq!(world.points_spent(fiend), 0, "nothing learned");
    world.bury(vec![(creep, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), rules::SOULS_PER_UNIT);
    let view = world.view(Team::Radiant);
    let shown = view
        .units
        .iter()
        .find(|unit| unit.id == wire_id(fiend))
        .expect("he is in the view");
    assert!(
        shown
            .abilities
            .iter()
            .all(|slot| slot.id != ability::NECROMASTERY),
        "and the necromastery is in no slot"
    );
    assert!(
        shown.abilities[usize::from(FEAST_SLOT)].can_level,
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
fn a_raze_stands_through_its_cast_point_and_lands_when_it_is_over() {
    let (mut world, fiend, mark) = fiend_and_a_mark(rules::RAZE_DISTANCE[0]);
    let stood = world.transform.get(fiend).expect("standing").pos;
    let mana = world.mana.get(fiend).expect("a pool").mana;
    let_go(&mut world, RAZE_NEAR_SLOT);
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
                ability: ability::RAZE_NEAR,
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
    let_go(&mut world, RAZE_NEAR_SLOT);
    for _ in 0..5 {
        world.step();
    }
    stop_click(&mut world);
    let mut burnt = Vec::new();
    for _ in 0..30 {
        burnt.extend(burns_on(&world.step(), mark));
    }
    assert!(burnt.is_empty(), "the raze never goes off");
    assert_eq!(cooldown(&world, fiend, RAZE_NEAR_SLOT), 0);
    assert_eq!(world.mana.get(fiend).expect("a pool").mana, mana);
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
        for _ in 0..ticks {
            world.step();
        }
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
fn a_hero_brought_down_is_worth_four_souls() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let victim = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5400, 5000),
        SlotId(1),
        HeroId(0),
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

#[test]
fn the_presence_wears_down_the_armor_of_enemies_that_see_its_carrier() {
    // He stands on high ground, which the creep 400 east of him is below.
    let (mut world, fiend, below) = fiend_and_a_mark(400);
    // Standing, he strikes nothing, and a strike would show him.
    world.set_order(fiend, UnitOrder::Stand);
    let out = a_creep_at(
        &mut world,
        Team::Dire,
        Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
    );
    for _ in 0..rules::PRESENCE_LINGER_TICKS {
        world.step();
    }
    assert!(
        !world.can_see(Team::Dire, fiend),
        "the Dire side is blind to him"
    );
    let armor = |world: &World, unit| world.stats.get(unit).expect("standing").armor;
    assert_eq!(
        armor(&world, below),
        armor(&world, out),
        "and what cannot see him wears nothing"
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
        "for the whole side: two and a half armor at its first level"
    );
}

#[test]
fn armor_the_presence_takes_below_zero_lets_his_swings_land_harder() {
    // A melee creep's 2 armor less the fourth level's 7 is -5: Dota's
    // 2 - 1 / (1 + 0.06 * 5) of a swing comes through.
    for (presence, taken) in [(0, 44 * 100 / 112), (4, 44 * 160 / 130)] {
        let (mut world, fiend, mark) = fiend_and_a_mark(-400);
        if let Some(book) = world.abilities.get_mut(fiend) {
            book.slots[4].level = presence;
        }
        world.step();
        assert_eq!(
            one_swing_takes(&mut world, fiend, mark),
            taken,
            "presence at level {presence}"
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
        .find(|shown| shown.ability == Some(ability::RAZE_FAR))
        .expect("the Dire side sees the raze where it landed");
    assert_eq!(
        landed.pos,
        Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000)
    );
    assert!(
        dire.units.iter().all(|shown| shown.hero != Some(HeroId(2))),
        "and still does not see the fiend"
    );
    for _ in 0..rules::MARK_TICKS {
        world.step();
    }
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .all(|shown| shown.ability != Some(ability::RAZE_FAR)),
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
        .find(|u| u.hero == Some(HeroId(2)))
        .expect("he is in the view");
    let souls = shown
        .effects
        .iter()
        .find(|e| e.id == EffectId(11))
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
            .filter(|shown| shown.ability == Some(ability::REQUIEM))
            .count()
    };
    let_go(&mut world, REQUIEM_SLOT);
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
    let projected = world
        .view(Team::Dire)
        .units
        .into_iter()
        .find(|unit| unit.id == wire_id(mark))
        .expect("the frightened creep is projected");
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
        HeroId(0),
    );
    world.settle();
    hand_souls(&mut world, fiend, 1);
    world.step();
    let resist = |world: &World| {
        world
            .view_full()
            .units
            .into_iter()
            .find(|unit| unit.id == wire_id(foe))
            .expect("the foe is projected")
            .magic_resist
    };
    assert_eq!(resist(&world), Fixed::from_ratio(1, 4), "a hero's own");
    cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
    for _ in 0..10 {
        world.step();
    }
    // 1 - (1 - 0.25) * 1.05: what came through grows by five percent.
    assert_eq!(resist(&world), Fixed::from_ratio(2125, 10_000));
    let shown = world
        .view_full()
        .units
        .into_iter()
        .find(|unit| unit.id == wire_id(foe))
        .expect("the foe is projected");
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
        .filter(|shown| shown.ability == Some(ability::REQUIEM))
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
    if let Some(book) = world.abilities.get_mut(fiend) {
        book.slots[usize::from(REQUIEM_SLOT)].level = 0;
    }
    world.bury(vec![(fiend, None)], &mut Vec::new());
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .all(|shown| shown.ability != Some(ability::REQUIEM))
    );
}

/// Casts one of the fiend's slots and runs the world through its cast point:
/// every event on the way, the tick it goes off included.
pub(super) fn cast_through(world: &mut World, slot: u8, point_ms: u32) -> Vec<Event> {
    let mut events = world.advance(&[crate::game::Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cast {
            slot: bota_proto::AbilitySlot(slot),
            target: Target::None,
        },
    }]);
    for _ in 0..ticks_of(point_ms) {
        events.extend(world.step());
    }
    events
}

/// Magic damage told of on one unit.
pub(super) fn burns_on(events: &[Event], unit: Entity) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged {
                target,
                amount,
                kind: DamageKind::Magical,
                ..
            } if target == wire_id(unit) => Some(amount),
            _ => None,
        })
        .collect()
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

/// Ticks one of a hero's slots waits before it may be cast again.
pub(super) fn cooldown(world: &World, hero: Entity, slot: u8) -> u32 {
    world
        .abilities
        .get(hero)
        .map_or(0, |book| book.slots[usize::from(slot)].cooldown)
}

/// What a unit has left of its health.
pub(super) fn hp(world: &World, unit: Entity) -> Fixed {
    world.health.get(unit).expect("standing").hp
}

/// Souls a hero holds.
pub(super) fn souls(world: &World, hero: Entity) -> u32 {
    world
        .stacks
        .get(hero)
        .map_or(0, |kept| kept.of(StackKind::Souls))
}
