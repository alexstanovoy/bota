//! Ethereal form: Dota's Ghost Scepter's Ghost Form, every number from the install's
//! `items.txt` through the catalogue.

use bota_core::{ModifierKind, item_def, mitigate, rules};
use bota_proto::{
    DamageKind, EffectId, EventKind, Fixed, ItemId, ItemSlot, Order, Target, Team, Vec2,
};

use crate::game::{Entity, UnitOrder, World, wire_id};

use super::item_numbers::{foe_hero, out_of_fountain_after_buying};
use super::support::*;

/// A hero that bought a Ghost Scepter through the shop and used it through its seat, out of
/// the fountain, a tick on: the world and the hero, ethereal.
fn a_ghost() -> (World, Entity) {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER]);
    let events = seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert!(
        events.iter().any(|event| matches!(
            event.kind(),
            EventKind::ItemUsed {
                item: ItemId::GHOST_SCEPTER,
                ..
            }
        )),
        "the use is told"
    );
    assert!(
        ghost_form(&world, hero).is_some(),
        "it wears Ghost Form at once"
    );
    world.step();
    assert!(ethereal(&world, hero), "and is ethereal");
    (world, hero)
}

/// Whether a unit is ethereal.
fn ethereal(world: &World, unit: Entity) -> bool {
    world.stats.get(unit).is_some_and(|stats| stats.ethereal)
}

/// The Ghost Form the unit wears, with the ticks it has left.
fn ghost_form(world: &World, unit: Entity) -> Option<u32> {
    world.modifiers.get(unit).and_then(|on_it| {
        on_it
            .active()
            .find(|held| matches!(held.kind, ModifierKind::Ethereal { .. }))
            .map(|held| held.ticks_left.unwrap_or(u32::MAX))
    })
}

/// An enemy hero at `at` carrying an Orb of Blight, whose Corruption lands with any attack of its
/// that reaches, physical damage or none.
fn a_corrupting_foe(world: &mut World, at: Vec2) -> Entity {
    let enemy = foe_hero(world, at);
    hand_item(world, enemy, ItemId::BLIGHT_STONE, 0);
    enemy
}

/// Whether `unit` wears an Orb of Blight's Corruption.
fn corrupted(world: &World, unit: Entity) -> bool {
    world.modifiers.get(unit).is_some_and(|on_it| {
        on_it.active().any(|held| {
            matches!(held.kind, ModifierKind::Laid { item, .. } if item == ItemId::BLIGHT_STONE)
        })
    })
}

/// Orders `by` to attack `on`.
fn order_attack(world: &mut World, by: Entity, on: Entity) {
    let last_seen = world.transform.get(on).expect("standing").pos;
    world.set_order(
        by,
        UnitOrder::Attack {
            target: on,
            last_seen,
        },
    );
}

/// Whether an attack's missile flies at `on`.
fn attack_flying_at(world: &World, on: Entity) -> bool {
    world.entities.iter().any(|missile| {
        world
            .projectile
            .get(missile)
            .is_some_and(|shot| shot.target == on && shot.striker.is_some())
    })
}

/// The health a unit lost to blows from `by` over `ticks` ticks, as the events tell it.
fn hurt_by(world: &mut World, by: Entity, on: Entity, ticks: u32) -> i32 {
    let mut lost = 0;
    for _ in 0..ticks {
        for event in world.step() {
            if let EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                ..
            } = event.kind()
                && source == wire_id(by)
                && target == wire_id(on)
            {
                lost += amount;
            }
        }
    }
    lost
}

/// The Ghost Scepter adds 5 to every attribute, and its use puts the user in Ghost Form
/// (`modifier_ghost_state`) for 4 seconds, then waits 22 (`bonus_all_stats`, `duration`,
/// `AbilityCooldown`), told to the user's side.
#[test]
fn a_ghost_scepter_turns_its_user_ethereal_for_four_seconds() {
    let def = item_def(ItemId::GHOST_SCEPTER).expect("in the catalogue");
    assert_eq!(def.carried.attributes.strength, Fixed::from_int(5));
    assert_eq!(def.carried.attributes.agility, Fixed::from_int(5));
    assert_eq!(def.carried.attributes.intelligence, Fixed::from_int(5));
    assert_eq!(def.cooldown, 22 * rules::TICKS_PER_SECOND);
    let (mut world, hero) = a_ghost();
    let left = ghost_form(&world, hero).expect("it wears Ghost Form");
    assert!(
        left > 4 * rules::TICKS_PER_SECOND - 3 && left <= 4 * rules::TICKS_PER_SECOND,
        "{left}"
    );
    assert!(
        shown(&world, Some(Team::Radiant), hero)
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::GHOST_FORM),
        "its side is told"
    );
    let wait = slot_of(&world, hero, 0).expect("carried").cooldown;
    assert!(wait > 22 * rules::TICKS_PER_SECOND - 3, "{wait}");
    run(&mut world, left);
    assert_eq!(ghost_form(&world, hero), None, "and it runs out");
    assert!(!ethereal(&world, hero));
}

/// No attack reaches it (Dota: "cannot attack or be attacked"): an enemy hero ordered to attack
/// it never swings at it, so its Orb of Blight lays nothing, for the whole of the form; once the
/// form is over its blows land again.
#[test]
fn nothing_attacks_an_ethereal_hero() {
    let (mut world, hero) = a_ghost();
    let at = world.transform.get(hero).expect("standing").pos;
    let enemy = a_corrupting_foe(&mut world, at + Vec2::from_ints(150, 0));
    order_attack(&mut world, enemy, hero);
    for _ in 0..3 * rules::TICKS_PER_SECOND {
        world.step();
        assert_ne!(swinging(&world, enemy), Some(hero), "no swing at it");
    }
    assert!(!corrupted(&world, hero), "no attack reached it");
    run(&mut world, 2 * rules::TICKS_PER_SECOND);
    assert!(!ethereal(&world, hero));
    order_attack(&mut world, enemy, hero);
    assert!(hurt_by(&mut world, enemy, hero, 4 * rules::TICKS_PER_SECOND) > 0);
    assert!(corrupted(&world, hero));
}

/// An attack's missile already in flight is lost when its target turns ethereal (Dota 6.87:
/// "Ethereal now blocks in-flight attack projectiles fully, rather than just ones launched
/// before entering Ethereal form").
#[test]
fn an_attack_in_flight_is_lost_on_an_ethereal_target() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER]);
    let at = world.transform.get(hero).expect("standing").pos;
    // A Shadow Fiend throws its attack from afar; the Corruption its missile would lay shows
    // whether it arrived.
    let enemy = a_corrupting_foe(&mut world, at + Vec2::from_ints(450, 0));
    order_attack(&mut world, enemy, hero);
    let mut flying = false;
    for _ in 0..10 * rules::TICKS_PER_SECOND {
        world.step();
        flying = attack_flying_at(&world, hero);
        if flying {
            break;
        }
    }
    assert!(flying, "a missile flies at the hero");
    world.set_order(enemy, UnitOrder::Stand);
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    world.step();
    assert!(
        !attack_flying_at(&world, hero),
        "the missile is gone as the form takes hold"
    );
    run(&mut world, 2 * rules::TICKS_PER_SECOND);
    assert!(!corrupted(&world, hero), "and never arrived");
}

/// An ethereal unit attacks nothing (Dota: "unable to attack"): ordered to attack an enemy in
/// reach, it lands no blow.
#[test]
fn an_ethereal_hero_attacks_nothing() {
    let (mut world, hero) = a_ghost();
    let at = world.transform.get(hero).expect("standing").pos;
    let enemy = foe_hero(&mut world, at + Vec2::from_ints(150, 0));
    world.set_order(enemy, UnitOrder::Stand);
    attack_click(&mut world, enemy);
    assert_eq!(
        hurt_by(&mut world, hero, enemy, 3 * rules::TICKS_PER_SECOND),
        0
    );
}

/// Physical damage passes it by, whatever deals it (Dota 2011-02-04: "Fixed cases where units
/// affected by Ghost Form could still take physical damage"), and magic resistance lets 30% more
/// through (`extra_spell_damage_percent` -30; Dota 7.40: "decreased from 40% to 30%"), multiplied
/// into what it lets through already.
#[test]
fn an_ethereal_hero_takes_no_physical_damage_and_thirty_percent_more_magic() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER]);
    let at = world.transform.get(hero).expect("standing").pos;
    let enemy = foe_hero(&mut world, at + Vec2::from_ints(3000, 0));
    world.step();
    let resist = world.stats.get(hero).expect("derived").magic_resist_bp;
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    world.push_hit(Some(enemy), hero, 200, DamageKind::Physical, None);
    world.push_splash_hit(Some(enemy), hero, 200, None, crate::game::Category::Plain);
    assert_eq!(hurt_by(&mut world, enemy, hero, 1), 0, "no physical damage");
    let let_through = (rules::NOMINAL_BP - resist) * 130 / 100;
    assert_eq!(
        world.stats.get(hero).expect("derived").magic_resist_bp,
        rules::NOMINAL_BP - let_through
    );
    world.push_hit(Some(enemy), hero, 200, DamageKind::Magical, None);
    assert_eq!(
        hurt_by(&mut world, enemy, hero, 1),
        mitigate(
            200,
            DamageKind::Magical,
            Fixed::ZERO,
            rules::NOMINAL_BP - let_through
        )
    );
}

/// A tower drops an ethereal hero for another target and takes it on again once the form is
/// over.
#[test]
fn a_tower_leaves_an_ethereal_hero_alone() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER]);
    let at = world.transform.get(hero).expect("standing").pos;
    let tower = world.spawn_unit(
        bota_core::tower_def(1),
        Team::Dire,
        at + Vec2::from_ints(300, 0),
    );
    world.set_order(hero, UnitOrder::Stand);
    assert!(
        hurt_by(&mut world, tower, hero, 3 * rules::TICKS_PER_SECOND) > 0,
        "the tower shoots it"
    );
    world.fill_pools(hero);
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    world.set_order(hero, UnitOrder::Stand);
    assert_eq!(
        hurt_by(&mut world, tower, hero, 3 * rules::TICKS_PER_SECOND),
        0
    );
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(
        hurt_by(&mut world, tower, hero, 4 * rules::TICKS_PER_SECOND) > 0,
        "and again"
    );
}

/// A basic dispel ends Ghost Form (`SpellDispellableType` `SPELL_DISPELLABLE_YES`).
#[test]
fn a_basic_dispel_ends_ghost_form() {
    let (mut world, hero) = a_ghost();
    world.dispel(
        hero,
        crate::game::Dispel::Buffs,
        crate::game::DispelPower::Basic,
    );
    world.step();
    assert_eq!(ghost_form(&world, hero), None);
    assert!(!ethereal(&world, hero));
}

/// Ghost Form holds on the tick it is used: a physical blow landing that very tick passes the
/// user by (the form is on before the tick's blows are felt, as Dota's is from its cast).
#[test]
fn ghost_form_holds_on_the_tick_it_is_used() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER]);
    let at = world.transform.get(hero).expect("standing").pos;
    let enemy = foe_hero(&mut world, at + Vec2::from_ints(3000, 0));
    world.step();
    world.push_hit(Some(enemy), hero, 200, DamageKind::Physical, None);
    let events = seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    let hurt = events.iter().any(|event| {
        matches!(event.kind(), EventKind::Damaged { target, .. } if target == wire_id(hero))
    });
    assert!(!hurt, "the blow passes the ghost by");
}
