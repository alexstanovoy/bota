//! Item passives that lay something on whatever their carrier's attacks strike: Dota's attack
//! modifiers, every number from the install's `items.txt` through the catalogue.

use bota_core::{ModifierKind, item_def, mitigate, rules};
use bota_proto::{DamageKind, EffectId, EventKind, Fixed, ItemId, Team, Vec2};

use crate::game::{Entity, Hit, HitEffect, Laid, World, wire_id};

use super::item_numbers::{foe_hero, out_of_fountain_after_buying};
use super::support::*;

/// A world where the seat's hero bought `items` and its attack, given through the seat, has just
/// landed on what `on` put in its reach: the world, the hero, the struck and what the blow took.
fn first_landing(
    items: &[ItemId],
    on: impl FnOnce(&mut World, Vec2) -> Entity,
) -> (World, Entity, Entity, i32) {
    let (mut world, hero) = out_of_fountain_after_buying(items);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(120, 0));
    world.step();
    attack_click(&mut world, mark);
    for _ in 0..300 {
        let events = world.step();
        let taken = events.iter().find_map(|event| match event.kind() {
            EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                inflictor: None,
                ..
            } if source == wire_id(hero) && target == wire_id(mark) => Some(amount),
            _ => None,
        });
        if let Some(taken) = taken {
            return (world, hero, mark, taken);
        }
    }
    panic!("the attack never landed")
}

/// A swing's blow of 100 physical damage from `by` on `on`, laying `item`'s passive.
fn swing_laying(by: Entity, on: Entity, item: ItemId) -> Hit {
    let mut laid = Laid::NONE;
    laid.add(item);
    Hit {
        source: Some(by),
        inflictor: None,
        target: on,
        amount: 100,
        kind: DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: true,
        spell: false,
        striker: Some(bota_core::Striker::RangedHero),
        pierces: true,
        uphill: false,
        effect: HitEffect::None,
        laid,
    }
}

/// What a test puts at a spot.
type Put = fn(&mut World, Vec2) -> Entity;

/// How many of `item`'s modifiers `unit` wears, and the time the one left has.
fn laid_on(world: &World, unit: Entity, item: ItemId) -> (usize, Option<u32>) {
    let kind = ModifierKind::Laid { item };
    let held: Vec<_> = world
        .modifiers
        .get(unit)
        .into_iter()
        .flat_map(|on_it| on_it.active())
        .filter(|held| held.kind == kind)
        .collect();
    (held.len(), held.first().and_then(|held| held.ticks_left))
}

/// The armor `unit` stands at.
fn armor(world: &World, unit: Entity) -> Fixed {
    world.stats.get(unit).expect("derived").armor
}

/// The Orb of Blight's numbers, as the catalogue holds them from `item_blight_stone`.
fn corruption() -> bota_core::OnAttack {
    item_def(ItemId::BLIGHT_STONE)
        .and_then(|def| def.on_attack)
        .expect("the Orb of Blight lays its corruption")
}

/// An Orb of Blight bought through the shop: its carrier's attack leaves the struck wearing
/// Corruption (`modifier_blight_stone_buff`), 2 armor less for 3 seconds (`corruption_armor`,
/// `corruption_duration`), told to the struck's side; then it is gone and the armor back.
#[test]
fn an_orb_of_blight_takes_two_armor_off_what_its_carrier_strikes_for_three_seconds() {
    let laid = corruption();
    assert_eq!(laid.armor, Fixed::from_int(2));
    assert_eq!(laid.ticks, 3 * rules::TICKS_PER_SECOND);
    assert_eq!(laid.effect, EffectId::CORRUPTION);
    let (mut world, hero, mark, _) = first_landing(&[ItemId::BLIGHT_STONE], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    let (count, left) = laid_on(&world, mark, ItemId::BLIGHT_STONE);
    assert_eq!(count, 1, "the struck wears it");
    let bare = {
        let (world, _, mark, _) = first_landing(&[], foe_hero);
        armor(&world, mark)
    };
    world.step();
    assert_eq!(armor(&world, mark), bare - laid.armor);
    let shown = shown(&world, Some(Team::Dire), mark);
    assert!(
        shown
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::CORRUPTION),
        "the struck's side is told of it"
    );
    run(&mut world, left.expect("timed"));
    assert_eq!(
        laid_on(&world, mark, ItemId::BLIGHT_STONE).0,
        0,
        "and it runs out"
    );
    assert_eq!(armor(&world, mark), bare);
}

/// The corruption is on before the blow that lays it lands (Dota: "The attacks first apply the
/// debuff, then their own attack damage"): the first blow already meets the lowered armor.
#[test]
fn the_first_blow_of_an_orb_of_blight_lands_on_the_corrupted_armor() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(300, 0));
    world.step();
    let before = armor(&world, mark);
    world
        .hits
        .push_back(swing_laying(hero, mark, ItemId::BLIGHT_STONE));
    let events = world.step();
    let taken = events
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Damaged { target, amount, .. } if target == wire_id(mark) => Some(amount),
            _ => None,
        })
        .expect("the blow lands");
    let corrupted = before - corruption().armor;
    assert_eq!(taken, mitigate(100, DamageKind::Physical, corrupted, 0));
    assert_ne!(taken, mitigate(100, DamageKind::Physical, before, 0));
}

/// Two Orbs of Blight carried corrupt as one: one modifier, two armor.
#[test]
fn two_orbs_of_blight_corrupt_as_one() {
    let (mut world, _, mark, _) =
        first_landing(&[ItemId::BLIGHT_STONE, ItemId::BLIGHT_STONE], foe_hero);
    assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 1);
    let bare = {
        let (world, _, mark, _) = first_landing(&[], foe_hero);
        armor(&world, mark)
    };
    world.step();
    assert_eq!(armor(&world, mark), bare - corruption().armor);
}

/// Corruption goes through spell immunity (`SpellImmunityType` `SPELL_IMMUNITY_ENEMIES_YES`) and
/// lands on whatever an attack strikes (Liquipedia, *Orb of Blight*: it affects "Allied units",
/// "Buildings", "Ward-type units"): a courier, immune to spells, a tower, a ward and an allied creep
/// struck by a carrier of an Orb of Blight wear it.
#[test]
fn an_orb_of_blight_corrupts_through_spell_immunity_and_on_buildings_wards_and_allies() {
    let marks: [(&str, Put); 4] = [
        ("a courier", |world, at| {
            world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
        }),
        ("a tower", |world, at| {
            world.spawn_unit(bota_core::tower_def(1), Team::Dire, at)
        }),
        ("a ward", |world, at| {
            world.spawn_unit(&bota_core::OBSERVER_WARD, Team::Dire, at)
        }),
        ("an allied creep", |world, at| {
            world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, at)
        }),
    ];
    for (case, mark) in marks {
        let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
        let at = world.transform.get(hero).expect("standing").pos;
        let mark = mark(&mut world, at + Vec2::from_ints(300, 0));
        world.step();
        world
            .hits
            .push_back(swing_laying(hero, mark, ItemId::BLIGHT_STONE));
        world.step();
        assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 1, "{case}");
    }
}

/// A basic dispel takes Corruption off (`SpellDispellableType` `SPELL_DISPELLABLE_YES`).
#[test]
fn a_basic_dispel_takes_corruption_off() {
    let (mut world, _, mark, _) = first_landing(&[ItemId::BLIGHT_STONE], foe_hero);
    world.dispel_basic(mark);
    assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 0);
}

/// An illusion of a hero carrying an Orb of Blight lays nothing with its attacks (Liquipedia,
/// *Orb of Blight*: "This ability is not bestowed by Illusions").
#[test]
fn an_illusion_lays_no_corruption() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&unit| world.illusion.get(unit).is_some_and(|made| made.of == hero))
        .expect("an illusion");
    let at = world.transform.get(illusion).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(120, 0));
    world.step();
    world.advance_trusted(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: Some(wire_id(illusion)),
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(wire_id(mark)),
        },
        queue: false,
        seq: 0,
    }]);
    for _ in 0..300 {
        let events = world.step();
        let struck = events.iter().any(|event| {
            matches!(event.kind(), EventKind::Damaged { source: Some(source), target, .. }
                if source == wire_id(illusion) && target == wire_id(mark))
        });
        if struck {
            assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 0);
            return;
        }
    }
    panic!("the illusion never struck")
}
