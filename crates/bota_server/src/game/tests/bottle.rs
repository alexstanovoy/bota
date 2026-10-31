//! The Bottle at Dota 7.41f: bought, drunk, refilled at the fountain, and the
//! place a hero keeps the rune it takes.

use bota_core::{
    BOTTLE_CHARGES, BOTTLE_COOLDOWN, BOTTLE_COST, BOTTLE_RUNE_TICKS, ModifierKind, rules,
};
use bota_proto::{
    EventKind, ItemId, ItemSlot, Order, RejectReason, RuneKind, SlotId, Target, Team, Vec2,
};

use crate::game::{Entity, ItemStack, World, wire_id};

use super::runes::{fraction, hero, lying, place, rune_match, rune_under_hero, step_at_minute};
use super::support::*;

const RIVER_TOP: usize = 0;

fn sides() -> [Team; 2] {
    [Team::Radiant, Team::Dire]
}

/// A bottle in the hero's first slot, `charges` full.
fn bottled(world: &mut World, taker: Entity, charges: u8) {
    hand_item(world, taker, ItemId::BOTTLE, charges);
}

fn the_bottle(world: &World, hero: Entity) -> ItemStack {
    slot_of(world, hero, 0).expect("the bottle is in the first slot")
}

fn drink() -> Order {
    Order::Use {
        slot: ItemSlot(0),
        target: Target::None,
    }
}

fn hp_mana(world: &World, hero: Entity) -> (i32, i32) {
    (
        world.health.get(hero).expect("hp").hp.to_int(),
        world.mana.get(hero).expect("mana").mana.to_int(),
    )
}

/// A hero a long way from the fountain, half empty, with a bottle.
fn thirsty_hero() -> (World, Entity) {
    let mut world = rune_match(&sides());
    let taker = hero(&world, 0);
    bottled(&mut world, taker, BOTTLE_CHARGES);
    place(&mut world, taker, Vec2::from_ints(7000, 9000));
    let stats = *world.stats.get(taker).expect("stats");
    world.health.get_mut(taker).expect("hp").hp = fraction(stats.max_hp, 1, 2);
    world.mana.get_mut(taker).expect("mana").mana = fraction(stats.max_mana, 1, 2);
    world.step();
    (world, taker)
}

#[test]
fn a_bottle_is_bought_for_675_gold_with_three_charges() {
    let (mut world, hero) = a_hero_with_gold(BOTTLE_COST + 25);
    let events = advance_validated(
        &mut world,
        None,
        Order::Buy {
            item: ItemId::BOTTLE,
        },
    );
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::ItemBought {
            item: ItemId::BOTTLE,
            ..
        }
    )));
    let held = slot_of(&world, hero, 0).expect("bought into the first slot");
    assert_eq!(held.id, ItemId::BOTTLE);
    assert_eq!(held.charges, BOTTLE_CHARGES);
    assert_eq!(world.seats[0].gold, 25, "675 gold less");
    assert!(
        bota_core::shop_entries(bota_core::Catalogue::FULL)
            .iter()
            .any(|entry| entry.id == ItemId::BOTTLE && entry.cost == 675),
        "the shop sells it"
    );
}

#[test]
fn a_drink_mends_110_health_and_60_mana_over_2_7_seconds_and_costs_a_charge() {
    let (mut world, taker) = thirsty_hero();
    // The same hero left alone shows what its own regeneration adds.
    let mut control = world.clone();
    let (hp0, mana0) = hp_mana(&world, taker);
    let events = advance_validated(&mut world, None, drink());
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::ItemUsed {
            item: ItemId::BOTTLE,
            ..
        }
    )));
    assert_eq!(the_bottle(&world, taker).charges, BOTTLE_CHARGES - 1);
    assert!(carries(
        &world,
        taker,
        ModifierKind::BottleRegeneration {
            hp_per_tick: 0,
            mana_per_tick: 0
        }
    ));
    run(&mut world, 81);
    run(&mut control, 82);
    let (hp1, mana1) = hp_mana(&world, taker);
    let (hp2, mana2) = hp_mana(&control, taker);
    assert!(hp1 > hp0 && mana1 > mana0);
    assert!(
        (108..=111).contains(&(hp1 - hp2)),
        "110 health: {}",
        hp1 - hp2
    );
    assert!(
        (58..=61).contains(&(mana1 - mana2)),
        "60 mana: {}",
        mana1 - mana2
    );
    assert!(
        !carries(
            &world,
            taker,
            ModifierKind::BottleRegeneration {
                hp_per_tick: 0,
                mana_per_tick: 0
            }
        ),
        "over after the 2.7 seconds"
    );
}

#[test]
fn a_bottle_waits_half_a_second_between_drinks_and_a_second_drink_refreshes_the_first() {
    let (mut world, taker) = thirsty_hero();
    advance_validated(&mut world, None, drink());
    assert_eq!(
        world.validate_order(SlotId(0), None, &drink()),
        Err(RejectReason::OnCooldown)
    );
    run(&mut world, BOTTLE_COOLDOWN);
    advance_validated(&mut world, None, drink());
    let worn = world
        .modifiers
        .get(taker)
        .expect("modifiers")
        .active()
        .filter(|held| matches!(held.kind, ModifierKind::BottleRegeneration { .. }))
        .count();
    assert_eq!(worn, 1, "refreshed, not stacked");
    assert_eq!(the_bottle(&world, taker).charges, BOTTLE_CHARGES - 2);
}

#[test]
fn a_blow_from_an_enemy_hero_ends_a_drink_but_a_creep_does_not() {
    let (mut world, taker) = thirsty_hero();
    let enemy = hero(&world, 1);
    advance_validated(&mut world, None, drink());
    let creep = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(100, 100));
    world.push_hit(
        Some(creep),
        taker,
        10,
        bota_proto::DamageKind::Physical,
        None,
    );
    run(&mut world, 2);
    assert!(carries(
        &world,
        taker,
        ModifierKind::BottleRegeneration {
            hp_per_tick: 0,
            mana_per_tick: 0
        }
    ));
    world.push_hit(
        Some(enemy),
        taker,
        10,
        bota_proto::DamageKind::Physical,
        None,
    );
    run(&mut world, 2);
    assert!(!carries(
        &world,
        taker,
        ModifierKind::BottleRegeneration {
            hp_per_tick: 0,
            mana_per_tick: 0
        }
    ));
}

#[test]
fn a_bottle_is_drunk_by_an_allied_hero_within_350_and_not_a_farther_one() {
    let mut world = rune_match(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let (one, ally) = (hero(&world, 0), hero(&world, 1));
    bottled(&mut world, one, BOTTLE_CHARGES);
    place(&mut world, one, Vec2::from_ints(7000, 9000));
    place(&mut world, ally, Vec2::from_ints(7300, 9000));
    run(&mut world, 3);
    let order = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(wire_id(ally)),
    };
    advance_validated(&mut world, None, order);
    assert!(carries(
        &world,
        ally,
        ModifierKind::BottleRegeneration {
            hp_per_tick: 0,
            mana_per_tick: 0
        }
    ));
    assert!(!carries(
        &world,
        one,
        ModifierKind::BottleRegeneration {
            hp_per_tick: 0,
            mana_per_tick: 0
        }
    ));
    place(&mut world, ally, Vec2::from_ints(7500, 9000));
    run(&mut world, BOTTLE_COOLDOWN + 85);
    let before = the_bottle(&world, one).charges;
    seat_order(&mut world, order);
    assert_eq!(
        the_bottle(&world, one).charges,
        before,
        "out of reach: nothing spent"
    );
}

#[test]
fn an_empty_bottle_is_kept_and_filled_again_inside_the_fountain_one_second_after_the_other() {
    let (mut world, taker) = thirsty_hero();
    bottled(&mut world, taker, 0);
    assert_eq!(
        world.validate_order(SlotId(0), None, &drink()),
        Err(RejectReason::NoCharges),
        "kept, and useless while empty"
    );
    // Away from the fountain nothing fills it, however long the wait.
    run(&mut world, 5 * rules::TICKS_PER_SECOND);
    assert_eq!(the_bottle(&world, taker).charges, 0);
    let fountain = world.map.fountains[0];
    place(
        &mut world,
        taker,
        Vec2::from_ints(fountain.x.to_int() + 200, fountain.y.to_int()),
    );
    run(&mut world, rules::TICKS_PER_SECOND + 1);
    assert_eq!(
        the_bottle(&world, taker).charges,
        BOTTLE_CHARGES,
        "full again"
    );
}

#[test]
fn a_courier_does_not_fill_a_bottle_it_carries() {
    let mut world = rune_match(&sides());
    let courier = the_courier(&world);
    hand_item(&mut world, courier, ItemId::BOTTLE, 0);
    let fountain = world.map.fountains[0];
    world.transform.get_mut(courier).expect("standing").pos =
        Vec2::from_ints(fountain.x.to_int() + 100, fountain.y.to_int());
    run(&mut world, 3 * rules::TICKS_PER_SECOND);
    assert_eq!(slot_of(&world, courier, 0).expect("kept").charges, 0);
}

#[test]
fn a_hero_with_a_bottle_stores_the_rune_it_takes_and_lets_it_out_when_it_uses_the_bottle() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Haste);
    bottled(&mut world, taker, 1);
    let events = world.step();
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::RuneBottled {
            kind: RuneKind::Haste,
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::RuneActivated { .. })),
        "stored, not activated"
    );
    assert!(lying(&world).is_empty());
    assert!(!carries(&world, taker, ModifierKind::RuneHaste));
    // The hero's own side is told what the bottle holds; the other is not.
    let shown = |team| {
        world
            .view(team)
            .units
            .iter()
            .find(|unit| unit.id == wire_id(taker))
            .map(|unit| unit.items[0].expect("bottle").rune)
    };
    let own = shown(Team::Radiant).expect("it sees itself");
    assert_eq!(own.map(|rune| rune.kind), Some(RuneKind::Haste));
    assert_eq!(own.map(|rune| rune.ticks_left), Some(BOTTLE_RUNE_TICKS));
    // Using the bottle lets the rune out and fills the bottle.
    let events = advance_validated(&mut world, None, drink());
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::RuneActivated {
            kind: RuneKind::Haste,
            bottled: true,
            ..
        }
    )));
    assert!(carries(&world, taker, ModifierKind::RuneHaste));
    let after = the_bottle(&world, taker);
    assert_eq!(after.charges, BOTTLE_CHARGES, "a power rune fills it");
    assert!(after.rune.is_none());
    assert!(
        !carries(
            &world,
            taker,
            ModifierKind::BottleRegeneration {
                hp_per_tick: 0,
                mana_per_tick: 0
            }
        ),
        "it did not drink"
    );
}

#[test]
fn a_bottle_with_no_charge_left_still_lets_a_rune_out() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Arcane);
    bottled(&mut world, taker, 0);
    world.step();
    advance_validated(&mut world, None, drink());
    assert!(carries(&world, taker, ModifierKind::RuneArcane));
    assert_eq!(the_bottle(&world, taker).charges, BOTTLE_CHARGES);
}

#[test]
fn a_stored_water_rune_fills_two_charges_unless_the_bottle_was_full() {
    for (held, want) in [(0u8, 2u8), (2, 3), (3, 3)] {
        let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Water);
        bottled(&mut world, taker, held);
        world.step();
        advance_validated(&mut world, None, drink());
        assert_eq!(
            the_bottle(&world, taker).charges,
            want,
            "{held} charges when the water was stored"
        );
    }
}

#[test]
fn a_stored_bounty_rune_pays_what_it_was_worth_when_it_appeared() {
    let mut world = rune_match(&[Team::Radiant, Team::Dire]);
    let taker = hero(&world, 0);
    bottled(&mut world, taker, BOTTLE_CHARGES);
    step_at_minute(&mut world, 4, 0);
    let jungle = world.map.rune_spots[3].at;
    place(&mut world, taker, jungle);
    let events = world.step();
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::RuneBottled {
            kind: RuneKind::Bounty,
            ..
        }
    )));
    let before = world.seats[0].gold;
    // Used at 6:00, long after the rune appeared at 4:00: paid 46, not 40.
    world.tick = world.schedule().at_minute(6) - 1;
    let dire_before = world.seats[1].gold;
    advance_validated(&mut world, None, drink());
    let dire_gain = world.seats[1].gold - dire_before;
    assert_eq!(world.seats[0].gold - before - dire_gain, 46);
}

#[test]
fn a_second_rune_takes_effect_at_once_while_the_bottle_holds_one() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Haste);
    bottled(&mut world, taker, 3);
    world.step();
    world.put_rune(RuneKind::Arcane, RIVER_TOP);
    let events = world.step();
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::RuneActivated {
            kind: RuneKind::Arcane,
            bottled: false,
            ..
        }
    )));
    assert!(carries(&world, taker, ModifierKind::RuneArcane));
    assert_eq!(
        the_bottle(&world, taker).rune.map(|rune| rune.kind),
        Some(RuneKind::Haste),
        "the first stays where it was"
    );
}

#[test]
fn a_bottle_in_the_backpack_stores_nothing() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Haste);
    if let Some(bag) = world.inventory.get_mut(taker) {
        bag.slots[rules::INVENTORY_SLOTS] = Some(fresh_stack(ItemId::BOTTLE, 3));
    }
    let events = world.step();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::RuneActivated { bottled: false, .. }))
    );
    assert!(carries(&world, taker, ModifierKind::RuneHaste));
}

#[test]
fn a_stored_rune_takes_effect_by_itself_after_90_seconds() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Arcane);
    bottled(&mut world, taker, 1);
    world.step();
    run(&mut world, BOTTLE_RUNE_TICKS - 2);
    assert!(!carries(&world, taker, ModifierKind::RuneArcane));
    assert!(the_bottle(&world, taker).rune.is_some());
    let events = {
        let mut all = Vec::new();
        for _ in 0..4 {
            all.extend(world.step());
        }
        all
    };
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::RuneActivated {
            kind: RuneKind::Arcane,
            bottled: true,
            ..
        }
    )));
    assert!(carries(&world, taker, ModifierKind::RuneArcane));
    assert_eq!(the_bottle(&world, taker).charges, BOTTLE_CHARGES);
}

#[test]
fn a_bottle_holding_a_rune_stays_in_hand() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Haste);
    bottled(&mut world, taker, 3);
    world.step();
    let held = Some(the_bottle(&world, taker));
    let order = |order| world.validate_order(SlotId(0), None, &order);
    assert_eq!(
        order(Order::Put {
            slot: ItemSlot(0),
            target: Target::None
        }),
        Err(RejectReason::HoldsRune),
        "cannot be laid down"
    );
    assert_eq!(
        order(Order::Sell { slot: ItemSlot(0) }),
        Err(RejectReason::HoldsRune)
    );
    assert_eq!(
        order(Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(rules::INVENTORY_SLOTS as u8)
        }),
        Err(RejectReason::HoldsRune),
        "nor put in the backpack"
    );
    assert_eq!(
        order(Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(1)
        }),
        Ok(()),
        "but moved among the others"
    );
    assert_eq!(slot_of(&world, taker, 0), held);
}

#[test]
fn a_hero_that_dies_with_a_stored_rune_loses_it_when_its_time_is_out() {
    let (mut world, taker) = rune_under_hero(&sides(), RuneKind::Haste);
    bottled(&mut world, taker, 3);
    world.step();
    let seat = 0;
    world.health.get_mut(taker).expect("hp").hp = bota_proto::Fixed::from_int(1);
    world.push_hit(None, taker, 100, bota_proto::DamageKind::Pure, None);
    world.step();
    assert!(world.seats[seat].dead(), "fallen");
    run(&mut world, BOTTLE_RUNE_TICKS + 5);
    assert!(
        world
            .inventory
            .get(taker)
            .is_none_or(|bag| bag.held().all(|stack| stack.rune.is_none())),
        "the rune ran out unused"
    );
}
