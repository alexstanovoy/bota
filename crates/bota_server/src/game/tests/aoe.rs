//! Dota's Area of Effect bonus: a carried Chasm Stone widens every radius its carrier's scripts
//! mark `affected_by_aoe_increase`, every number from the install's scripts.

use bota_core::{ModifierKind, item_def, rules};
use bota_proto::{ItemId, Target, Team, Vec2};

use crate::game::{Entity, World};

use super::item_numbers::out_of_fountain_after_buying;
use super::support::*;

/// Puts a Chasm Stone in a hero's first slot and lets its stats take it in.
fn carry_a_chasm_stone(world: &mut World, hero: Entity) {
    hand_item(world, hero, ItemId::CHASM_STONE, 0);
    world.step();
}

/// A Chasm Stone adds 40 Area of Effect (`aoe_bonus`; Dota 7.41: "Provides +40 Area of Effect",
/// 7.41e: "Cost increased from 800 to 900"); two carried add 40, not 80 ("Area of Effect
/// bonuses from multiple Chasm Stones or its upgrades do not stack").
#[test]
fn a_chasm_stone_adds_forty_area_of_effect_and_two_do_not_stack() {
    let def = item_def(ItemId::CHASM_STONE).expect("in the catalogue");
    assert_eq!(def.carried.aoe_bonus, 40);
    assert_eq!(def.cost, 900);
    for carried in [
        vec![ItemId::CHASM_STONE],
        vec![ItemId::CHASM_STONE, ItemId::CHASM_STONE],
    ] {
        let (world, hero) = out_of_fountain_after_buying(&carried);
        assert_eq!(
            world.stats.get(hero).expect("derived").aoe_bonus,
            40,
            "{} carried",
            carried.len()
        );
    }
}

/// A raze reaches 40 further for a Chasm Stone (`shadowraze_radius` is marked): a creep standing
/// 20 past the edge of the nearest raze is burned only by a carrier's.
#[test]
fn a_chasm_stone_widens_a_raze() {
    let apart = rules::RAZE_DISTANCE[0] + rules::RAZE_RADIUS + 20;
    for carries in [false, true] {
        let (mut world, fiend, mark) = fiend_and_a_mark(apart);
        if carries {
            carry_a_chasm_stone(&mut world, fiend);
        }
        let events = cast_through(&mut world, 0, rules::RAZE_CAST_POINT_MS);
        assert_eq!(
            !burns_on(&events, mark).is_empty(),
            carries,
            "carrying one: {carries}"
        );
    }
}

/// Rot reaches 40 further for a Chasm Stone (`rot_radius` is marked).
#[test]
fn a_chasm_stone_widens_rot() {
    for carries in [false, true] {
        let (mut world, pudge, mark) = pudge_and_a_mark(rules::ROT_RADIUS + 20);
        // Both held where they stand: either would close in to fight of its own accord.
        put_on(&mut world, mark, ModifierKind::Stunned, 100);
        world.set_order(pudge, crate::game::UnitOrder::Stand);
        set_level(&mut world, pudge, 1, 1);
        if carries {
            carry_a_chasm_stone(&mut world, pudge);
        }
        cast(&mut world, 1, Target::None);
        run(&mut world, 3);
        let rotting = world.modifiers.get(mark).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| matches!(held.kind, ModifierKind::Rotting { .. }))
        });
        assert_eq!(rotting, carries, "carrying one: {carries}");
    }
}

/// An aura reaches 40 further for a Chasm Stone (Liquipedia, Area of Effect: "AoE Bonuses affect
/// all Auras"; `presence_radius` is marked): an enemy 20 past the edge of Presence of the Dark
/// Lord loses armor only to a carrier's.
#[test]
fn a_chasm_stone_widens_presence_of_the_dark_lord() {
    let radius = rules::PRESENCE_RADIUS;
    for carries in [false, true] {
        let (mut world, fiend, mark) = fiend_and_a_mark(radius + 20);
        let bare = world.stats.get(mark).expect("derived").armor;
        if carries {
            carry_a_chasm_stone(&mut world, fiend);
        }
        run(&mut world, 2);
        let armor = world.stats.get(mark).expect("derived").armor;
        assert_eq!(armor < bare, carries, "carrying one: {carries}");
    }
}

/// A Blood Grenade bursts 40 wider for its thrower's Chasm Stone (`radius` is marked).
#[test]
fn a_chasm_stone_widens_a_blood_grenades_burst() {
    let spec = item_def(ItemId::BLOOD_GRENADE)
        .and_then(bota_core::ItemDef::grenade)
        .expect("a grenade");
    for carries in [false, true] {
        let (mut world, hero) = out_of_fountain_after_buying(&[]);
        let at = world.transform.get(hero).expect("standing").pos;
        let spot = at + Vec2::from_ints(400, 0);
        let creep = a_creep_at(
            &mut world,
            Team::Dire,
            spot + Vec2::from_ints(spec.radius + 20, 0),
        );
        world.set_order(creep, crate::game::UnitOrder::Stand);
        if carries {
            carry_a_chasm_stone(&mut world, hero);
        }
        if let Some(bag) = world.inventory.get_mut(hero) {
            bag.slots[1] = Some(fresh_stack(ItemId::BLOOD_GRENADE, 1));
        }
        assert!(world.use_item(hero, 1, Target::Pos(spot), &mut Vec::new()));
        let mut burned = false;
        for _ in 0..3 * rules::TICKS_PER_SECOND {
            burned |= !burns_on(&world.step(), creep).is_empty();
        }
        assert_eq!(burned, carries, "carrying one: {carries}");
    }
}

/// What is not marked is not widened: a raze lands no further off for a Chasm Stone
/// (`shadowraze_range` is not marked), so a creep beyond the far edge of where it lands stays
/// out of it however wide it grows.
#[test]
fn a_chasm_stone_moves_no_raze() {
    let apart = rules::RAZE_DISTANCE[0] + rules::RAZE_RADIUS + 60;
    let (mut world, fiend, mark) = fiend_and_a_mark(apart);
    carry_a_chasm_stone(&mut world, fiend);
    let events = cast_through(&mut world, 0, rules::RAZE_CAST_POINT_MS);
    assert!(burns_on(&events, mark).is_empty());
}
