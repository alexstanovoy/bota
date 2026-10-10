//! Shadow Fiend's talents: how a skill point buys one, and what each does.

use crate::game::{Command, Entity, Event, Level, ModifierKind, World, rules, wire_id};
use bota_proto::{
    DamageKind, EventKind, Fixed, HeroId, Order, Pick, RejectReason, SlotId, TalentSlot,
    TalentView, Talents, Team, Vec2,
};

use super::fiend::{burns_on, cast_through, souls};
use super::support::*;

const RAZE_NEAR_SLOT: u8 = 0;
const RAZE_MID_SLOT: u8 = 1;
const FEAST_SLOT: u8 = 3;
const REQUIEM_SLOT: u8 = 5;

/// A Map 1 match of two Shadow Fiends at their fountains, the Radiant one
/// raised to a level through the experience a level takes.
fn two_fiends_at(level: u8) -> World {
    let mut cfg = config();
    cfg.picks = vec![
        Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId(2),
        },
        Pick {
            slot: SlotId(1),
            team: Team::Dire,
            hero: HeroId(2),
        },
    ];
    let mut world = World::for_match(&cfg, cfg.rng());
    let xp = rules::XP_THRESHOLDS[usize::from(level - 1)];
    world.grant_xp(0, xp, &mut Vec::new());
    assert_eq!(world.seats[0].level, level);
    world.step();
    world
}

/// Seat 0 asks for a talent: what the gate says, and on yes the tick taken.
fn ask_talent(world: &mut World, slot: u8) -> Result<(), RejectReason> {
    let order = Order::LearnTalent {
        talent: TalentSlot(slot),
    };
    world.validate_order(SlotId(0), None, &order)?;
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order,
    }]);
    Ok(())
}

/// What a side is told of one seat's talents.
fn told(world: &World, side: Option<Team>, seat: u8) -> Option<TalentView> {
    let view = match side {
        Some(team) => world.view(team),
        None => world.view_full(),
    };
    view.talents
        .into_iter()
        .find(|talents| talents.slot == SlotId(seat))
}

fn set(slots: &[u8]) -> Talents {
    slots.iter().fold(Talents::default(), |held, slot| {
        held.with(TalentSlot(*slot))
    })
}

#[test]
fn a_tier_opens_at_its_level_and_takes_one_talent_of_two_for_a_point() {
    let mut world = two_fiends_at(9);
    assert_eq!(ask_talent(&mut world, 0), Err(RejectReason::CannotLearn));
    assert_eq!(
        told(&world, Some(Team::Radiant), 0).map(|view| view.learnable),
        Some(Talents::default()),
        "nothing is open below level ten"
    );
    let mut world = two_fiends_at(10);
    let hero = world.seats[0].unit.expect("standing");
    assert_eq!(
        told(&world, Some(Team::Radiant), 0),
        Some(TalentView {
            slot: SlotId(0),
            learned: Talents::default(),
            learnable: set(&[0, 1]),
        })
    );
    assert_eq!(ask_talent(&mut world, 2), Err(RejectReason::CannotLearn));
    assert_eq!(ask_talent(&mut world, 1), Ok(()));
    assert_eq!(world.points_spent(hero), 1, "it costs a point");
    assert_eq!(
        told(&world, Some(Team::Radiant), 0),
        Some(TalentView {
            slot: SlotId(0),
            learned: set(&[1]),
            learnable: Talents::default(),
        }),
        "and shuts its tier"
    );
    assert_eq!(ask_talent(&mut world, 1), Err(RejectReason::CannotLearn));
    assert_eq!(ask_talent(&mut world, 0), Err(RejectReason::CannotLearn));
}

#[test]
fn a_talent_waits_for_a_point_like_an_ability() {
    let mut world = two_fiends_at(10);
    let hero = world.seats[0].unit.expect("standing");
    for slot in [0, 3, 4, 5, 0, 3, 4, 0, 3, 4] {
        world.learn(hero, slot);
    }
    assert_eq!(world.points_spent(hero), 10, "every point is in the kit");
    assert_eq!(ask_talent(&mut world, 0), Err(RejectReason::CannotLearn));
}

#[test]
fn the_other_talent_of_a_tier_opens_at_twenty_seven() {
    let mut world = two_fiends_at(26);
    assert_eq!(ask_talent(&mut world, 0), Ok(()));
    assert_eq!(ask_talent(&mut world, 1), Err(RejectReason::CannotLearn));
    world.grant_xp(
        0,
        rules::XP_THRESHOLDS[26] - world.seats[0].xp,
        &mut Vec::new(),
    );
    world.step();
    assert_eq!(world.seats[0].level, 27);
    assert_eq!(
        told(&world, Some(Team::Radiant), 0).map(|view| view.learnable.has(TalentSlot(1))),
        Some(true)
    );
    assert_eq!(ask_talent(&mut world, 1), Ok(()));
    assert_eq!(ask_talent(&mut world, 2), Ok(()));
    assert_eq!(
        ask_talent(&mut world, 3),
        Err(RejectReason::CannotLearn),
        "the other of the second tier waits for twenty-eight"
    );
}

#[test]
fn a_hero_without_a_tree_learns_none() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let xp = rules::XP_THRESHOLDS[24];
    world.grant_xp(0, xp, &mut Vec::new());
    world.step();
    assert_eq!(ask_talent(&mut world, 0), Err(RejectReason::CannotLearn));
    assert!(world.view_full().talents.is_empty(), "and shows no tree");
}

#[test]
fn learning_a_talent_leaves_a_cast_point_under_way() {
    let mut world = two_fiends_at(10);
    let hero = world.seats[0].unit.expect("standing");
    world.learn(hero, usize::from(RAZE_NEAR_SLOT));
    world.step();
    let events = world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cast {
            slot: bota_proto::AbilitySlot(RAZE_NEAR_SLOT),
            target: bota_proto::Target::None,
        },
    }]);
    assert!(events.is_empty());
    assert_eq!(ask_talent(&mut world, 1), Ok(()));
    let mut cast = false;
    for _ in 0..ticks_of(rules::RAZE_CAST_POINT_MS) {
        cast |= world
            .step()
            .iter()
            .any(|event| matches!(event.kind, EventKind::AbilityCast { .. }));
    }
    assert!(cast, "the raze still goes off");
}

#[test]
fn the_other_side_sees_learned_talents_while_it_sees_the_hero_and_its_own_through_death() {
    let mut world = two_fiends_at(10);
    assert_eq!(ask_talent(&mut world, 1), Ok(()));
    assert_eq!(
        told(&world, Some(Team::Dire), 0),
        None,
        "a hero in the fog shows nothing"
    );
    let dire = world.seats[1].unit.expect("standing");
    let radiant = world.seats[0].unit.expect("standing");
    let beside = world.transform.get(radiant).expect("standing").pos + Vec2::from_ints(100, 0);
    world.transform.get_mut(dire).expect("standing").pos = beside;
    world.step();
    assert_eq!(
        told(&world, Some(Team::Dire), 0),
        Some(TalentView {
            slot: SlotId(0),
            learned: set(&[1]),
            learnable: Talents::default(),
        }),
        "in sight, what is learned and nothing of what could be"
    );
    world.bury(vec![(radiant, Some(dire))], &mut Vec::new());
    world.step();
    assert_eq!(
        told(&world, Some(Team::Radiant), 0).map(|view| view.learned),
        Some(set(&[1])),
        "the fallen hero's side still sees it"
    );
    assert_eq!(told(&world, Some(Team::Dire), 0), None);
    assert_eq!(
        told(&world, None, 0).map(|view| view.learned),
        Some(set(&[1]))
    );
}

/// Shadow Fiend at level 25 and a creep `apart` east, every slot at its
/// first level, with or without one talent.
fn talented_fiend(apart: i32, talent: Option<u8>) -> (World, Entity, Entity) {
    let (mut world, fiend, mark) = fiend_and_a_mark(apart);
    world.level.insert(fiend, Level(25));
    if let Some(slot) = talent {
        assert!(world.learn_talent(fiend, TalentSlot(slot)));
    }
    // Standing, he swings at nothing: every blow told of is a raze's.
    stop_click(&mut world);
    (world, fiend, mark)
}

/// Physical damage told of on one unit.
fn blows_on(events: &[Event], unit: Entity) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged {
                target,
                amount,
                kind: DamageKind::Physical,
                ..
            } if target == wire_id(unit) => Some(amount),
            _ => None,
        })
        .collect()
}

#[test]
fn the_stack_damage_talent_adds_thirty_a_stack() {
    for (talent, stack) in [(None, 35), (Some(1), 65)] {
        // Both the near and the middle raze land 325 east.
        let (mut world, _fiend, mark) = talented_fiend(325, talent);
        let first = cast_through(&mut world, RAZE_NEAR_SLOT, rules::RAZE_CAST_POINT_MS);
        assert_eq!(burns_on(&first, mark), vec![85]);
        let second = cast_through(&mut world, RAZE_MID_SLOT, rules::RAZE_CAST_POINT_MS);
        assert_eq!(burns_on(&second, mark), vec![85 + stack], "{talent:?}");
    }
}

#[test]
fn the_feast_attack_speed_talent_adds_thirty() {
    for (talent, added) in [(None, 35), (Some(0), 65)] {
        let (mut world, fiend, _mark) = talented_fiend(1500, talent);
        let before = world.stats.get(fiend).expect("settled").attack_speed;
        let_go(&mut world, FEAST_SLOT);
        world.step();
        let during = world.stats.get(fiend).expect("settled").attack_speed;
        assert_eq!(during - before, added, "{talent:?}");
    }
}

#[test]
fn the_feast_souls_talent_takes_five_from_a_hero() {
    for (talent, taken) in [(None, 3), (Some(2), 5)] {
        let (mut world, fiend, _mark) = talented_fiend(1500, talent);
        let foe = world.spawn_hero(
            Team::Dire,
            Vec2::from_ints(5300, 5000),
            SlotId(1),
            HeroId(0),
        );
        put_on(&mut world, foe, ModifierKind::Stunned, 10_000);
        world.settle();
        stop_click(&mut world);
        let_go(&mut world, FEAST_SLOT);
        for _ in 0..rules::FEAST_PERIOD_TICKS {
            world.step();
        }
        assert_eq!(souls(&world, fiend), taken, "{talent:?}");
    }
}

#[test]
fn the_presence_talent_takes_one_and_a_half_armor_more() {
    let armor = |talent| {
        let (mut world, _fiend, mark) = talented_fiend(100, talent);
        world.step();
        world.stats.get(mark).expect("standing").armor
    };
    assert_eq!(armor(None) - armor(Some(3)), Fixed::from_ratio(3, 2));
}

#[test]
fn the_requiem_talent_holds_a_line_longer_and_lifts_the_most() {
    for (talent, held) in [(None, [18, 65]), (Some(4), [24, 78])] {
        for (lines, ticks) in [1, 5].into_iter().zip(held) {
            let (mut world, fiend, mark) = talented_fiend(60, talent);
            hand_souls(&mut world, fiend, lines);
            world.step();
            cast_through(&mut world, REQUIEM_SLOT, rules::REQUIEM_CAST_POINT_MS);
            world.step();
            let feared = world.modifiers.get(mark).and_then(|on_it| {
                on_it
                    .active()
                    .find(|held| held.kind == ModifierKind::Feared)
                    .and_then(|held| held.ticks_left)
            });
            assert_eq!(feared, Some(ticks), "{talent:?}, {lines} lines");
        }
    }
}

#[test]
fn the_souls_talent_holds_twenty_five() {
    for (talent, cap) in [(None, 20), (Some(5), 25)] {
        let (mut world, fiend, _mark) = talented_fiend(1500, talent);
        for nth in 0..30 {
            let fallen = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(7000 + nth, 7000));
            world.bury(vec![(fallen, Some(fiend))], &mut Vec::new());
        }
        assert_eq!(souls(&world, fiend), cap, "{talent:?}");
        assert_eq!(world.soul_cap(fiend), cap);
    }
}

#[test]
fn the_raze_attack_talent_lands_a_swing_with_every_raze() {
    let (mut world, _fiend, mark) = talented_fiend(rules::RAZE_DISTANCE[0], None);
    let events = cast_through(&mut world, RAZE_NEAR_SLOT, rules::RAZE_CAST_POINT_MS);
    assert!(
        blows_on(&events, mark).is_empty(),
        "a plain raze only burns"
    );
    let (mut world, fiend, mark) = talented_fiend(rules::RAZE_DISTANCE[0], Some(6));
    let stats = *world.stats.get(fiend).expect("settled");
    let armor = world.stats.get(mark).expect("standing").armor;
    let events = cast_through(&mut world, RAZE_NEAR_SLOT, rules::RAZE_CAST_POINT_MS);
    assert_eq!(burns_on(&events, mark), vec![85], "the burn is the same");
    let blows = blows_on(&events, mark);
    assert_eq!(blows.len(), 1, "and one swing's damage rides with it");
    // A creep's armor lets 100 / (100 + 6 armor) through.
    let through = |damage: i32| {
        (i64::from(damage) * 100 * 65536 / (100 * 65536 + 6 * i64::from(armor.raw))) as i32
    };
    assert!(
        (through(stats.damage)..=through(stats.damage + stats.damage_spread)).contains(&blows[0]),
        "{} within a swing's roll",
        blows[0]
    );
}

#[test]
fn the_cast_speed_talent_shortens_a_cast_point_while_the_feast_runs() {
    for (talent, feasting, point) in [(None, true, 17), (Some(7), false, 17), (Some(7), true, 12)] {
        let (mut world, _fiend, mark) = talented_fiend(rules::RAZE_DISTANCE[0], talent);
        if feasting {
            let_go(&mut world, FEAST_SLOT);
        }
        let_go(&mut world, RAZE_NEAR_SLOT);
        let landed = (1..=20).find(|_| !burns_on(&world.step(), mark).is_empty());
        assert_eq!(landed, Some(point), "{talent:?}, feasting {feasting}");
    }
}
