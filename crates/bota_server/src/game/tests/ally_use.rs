//! What an item used on an allied unit does where Dota lets it: wards and Tango are given one
//! charge at a time, Mango, Healing Salve and Clarity land on the ally (Salve at half).

use bota_core::{ModifierKind, rules};
use bota_proto::{ItemId, ItemSlot, Order, RejectReason, SlotId, Target, Team, UnitKind, Vec2};

use crate::game::{Entity, ItemStack, World, wire_id};

use super::support::*;

fn use_on(world: &mut World, hero: Entity, slot: u8, ally: Entity) {
    let order = Order::Use {
        slot: ItemSlot(slot),
        target: Target::Unit(wire_id(ally)),
    };
    advance_validated(world, Some(hero), order);
    // Long enough to walk across a screen; the use is over by then.
    run(world, 200);
}

/// A hero with a stack in its first slot and an allied hero `apart` units off in the open.
fn a_hero_and_an_ally(item: ItemId, charges: u8, apart: i32) -> (World, Entity, Entity) {
    let (mut world, hero) = match_hero();
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    hand_item(&mut world, hero, item, charges);
    let ally = seat_hero(
        &mut world,
        1,
        Team::Radiant,
        spot + Vec2::from_ints(apart, 0),
    );
    world.settle();
    (world, hero, ally)
}

fn held(world: &World, who: Entity) -> Vec<ItemStack> {
    world
        .inventory
        .get(who)
        .expect("bag")
        .held()
        .copied()
        .collect()
}

fn stacks(world: &World, who: Entity) -> Vec<(ItemId, u8, u8)> {
    held(world, who)
        .iter()
        .map(|stack| (stack.id, stack.charges, stack.sentries))
        .collect()
}

fn buy(world: &mut World, item: ItemId) {
    assert!(world.buy(SlotId(0), item, &mut Vec::new()));
}

#[test]
fn a_ward_cast_on_an_ally_gives_one_ward_and_a_dispenser_the_kind_it_is_switched_to() {
    let (mut world, hero) = match_hero();
    for item in [
        ItemId::OBSERVER_WARD,
        ItemId::SENTRY_WARD,
        ItemId::SENTRY_WARD,
    ] {
        buy(&mut world, item);
    }
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    // Farther than a hand-over, within a ward's cast range of 500 once walked to.
    let ally = seat_hero(&mut world, 1, Team::Radiant, spot + Vec2::from_ints(900, 0));
    world.settle();
    seat_order(&mut world, Order::SwitchWard { slot: ItemSlot(0) });
    use_on(&mut world, hero, 0, ally);
    assert_eq!(stacks(&world, ally), vec![(ItemId::SENTRY_WARD, 1, 0)]);
    assert_eq!(stacks(&world, hero), vec![(ItemId::WARD_DISPENSER, 1, 1)]);
    // The last sentry leaves the observer as the plain ward it is, and the wards of one giver
    // join in the ally's bag.
    use_on(&mut world, hero, 0, ally);
    assert_eq!(stacks(&world, ally), vec![(ItemId::SENTRY_WARD, 2, 0)]);
    assert_eq!(stacks(&world, hero), vec![(ItemId::OBSERVER_WARD, 1, 0)]);
    // A lone ward goes whole, and the two kinds meet in a dispenser.
    use_on(&mut world, hero, 0, ally);
    assert!(held(&world, hero).is_empty());
    assert_eq!(stacks(&world, ally), vec![(ItemId::WARD_DISPENSER, 1, 2)]);
    assert_eq!(standing(&world), 0, "nothing was planted");
}

fn standing(world: &World) -> usize {
    world
        .entities
        .iter()
        .filter(|entity| world.kind.get(*entity) == Some(&UnitKind::Ward))
        .count()
}

#[test]
fn a_give_waits_for_no_cooldown_and_a_bag_with_no_room_leaves_the_ward_at_its_feet() {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::OBSERVER_WARD, 3, 100);
    let at = world.pos(hero).expect("placed");
    assert!(world.use_item(
        hero,
        0,
        Target::Pos(at + Vec2::from_ints(200, 0)),
        &mut Vec::new()
    ));
    assert!(
        slot_of(&world, hero, 0).expect("held").cooldown > 0,
        "the plant started its wait"
    );
    for slot in world.inventory.get_mut(ally).expect("bag").slots.iter_mut() {
        *slot = Some(a_stack_of(ItemId::BOOTS, 0));
    }
    use_on(&mut world, hero, 0, ally);
    assert_eq!(stacks(&world, hero), vec![(ItemId::OBSERVER_WARD, 1, 0)]);
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1, "it lies where the ally stands");
    assert_eq!(world.pos(lying[0]), world.pos(ally));
}

#[test]
fn a_ward_cast_on_no_one_it_may_be_given_to_is_refused() {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::OBSERVER_WARD, 2, 100);
    let at = world.pos(hero).expect("placed");
    let enemy = seat_hero(&mut world, 2, Team::Dire, at);
    world.settle();
    let check = |world: &World, who: Entity| {
        world.check_order(
            SlotId(0),
            None,
            &Order::Use {
                slot: ItemSlot(0),
                target: Target::Unit(wire_id(who)),
            },
            false,
        )
    };
    assert_eq!(check(&world, ally), Ok(()));
    assert_eq!(check(&world, hero), Err(RejectReason::WrongTargetKind));
    assert_eq!(check(&world, enemy), Err(RejectReason::WrongTargetKind));
}

#[test]
fn a_tango_cast_on_an_ally_leaves_a_shared_one_that_is_gone_in_forty_seconds() {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::TANGO, 3, 100);
    use_on(&mut world, hero, 0, ally);
    assert_eq!(stacks(&world, hero), vec![(ItemId::TANGO, 2, 0)]);
    let shared = held(&world, ally);
    assert_eq!(shared.len(), 1);
    assert_eq!((shared[0].id, shared[0].charges), (ItemId::TANGO_SHARED, 1));
    assert!(shared[0].free_charges > 0, "no shop buys it back");
    // The forty seconds run from the give.
    let life = shared[0].life;
    assert!(life <= 40 * rules::TICKS_PER_SECOND && life > 30 * rules::TICKS_PER_SECOND);
    run(&mut world, life - 1);
    assert_eq!(held(&world, ally).len(), 1, "still there a tick before");
    run(&mut world, 1);
    assert!(held(&world, ally).is_empty(), "gone with its forty seconds");
}

#[test]
fn a_shared_tango_does_not_join_another_and_lies_out_its_time_on_the_ground() {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::TANGO, 3, 100);
    use_on(&mut world, hero, 0, ally);
    use_on(&mut world, hero, 0, ally);
    assert_eq!(held(&world, ally).len(), 2, "one slot each");
    assert!(world.put_item(ally, 0, Target::None));
    run(&mut world, 5);
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1);
    let life = world.loot.get(lying[0]).expect("loot").0.life;
    assert!(life > 0 && life < 40 * rules::TICKS_PER_SECOND);
    run(&mut world, life);
    assert!(on_the_ground(&world).is_empty(), "and goes there too");
}

fn mending_per_tick(world: &World, who: Entity) -> Option<bota_proto::Fixed> {
    world
        .modifiers
        .get(who)?
        .active()
        .find_map(|held| match held.kind {
            ModifierKind::Mending { per_tick, .. } | ModifierKind::Clarity { per_tick, .. } => {
                Some(per_tick)
            }
            _ => None,
        })
}

#[test]
fn a_salve_cast_on_an_ally_heals_for_half_and_a_clarity_for_the_whole() {
    for (item, percent) in [(ItemId::HEALING_SALVE, 50), (ItemId::CLARITY, 100)] {
        let (mut world, hero, ally) = a_hero_and_an_ally(item, 1, 100);
        use_on(&mut world, hero, 0, ally);
        let theirs = mending_per_tick(&world, ally).expect("the ally mends");
        let (mut alone, solo, _) = a_hero_and_an_ally(item, 1, 100);
        assert!(alone.use_item(solo, 0, Target::None, &mut Vec::new()));
        let own = mending_per_tick(&alone, solo).expect("the user mends");
        assert!(
            (theirs.raw * 100 / percent - own.raw).abs() <= 2,
            "{item:?}: {theirs} against {own}"
        );
        assert!(
            mending_per_tick(&world, hero).is_none(),
            "and the user does not"
        );
    }
}

#[test]
fn a_mango_cast_on_an_ally_restores_its_mana_walks_up_to_four_hundred_and_takes_only_what_is_missing()
 {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::MANGO, 2, 350);
    let mana = |world: &World, who: Entity| world.mana.get(who).expect("mana").mana.to_int();
    world.mana.get_mut(ally).expect("mana").mana = bota_proto::Fixed::from_int(50);
    world.mana.get_mut(hero).expect("mana").mana = bota_proto::Fixed::from_int(50);
    let check = |world: &World, who: Entity| {
        world.check_order(
            SlotId(0),
            None,
            &Order::Use {
                slot: ItemSlot(0),
                target: Target::Unit(wire_id(who)),
            },
            false,
        )
    };
    assert_eq!(check(&world, ally), Ok(()));
    let user_before = mana(&world, hero);
    advance_validated(
        &mut world,
        Some(hero),
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Unit(wire_id(ally)),
        },
    );
    assert_eq!(
        mana(&world, ally) - 50,
        100,
        "a Mango's hundred reached the ally"
    );
    assert!(mana(&world, hero) - user_before < 10, "and none the user");
    assert_eq!(
        slot_of(&world, hero, 0).map(|s| s.charges),
        Some(1),
        "at the price of a charge"
    );
    // A full pool takes nothing: the use is refused, as for its user.
    world.mana.get_mut(ally).expect("mana").mana = world.stats.get(ally).expect("stats").max_mana;
    assert_eq!(check(&world, ally), Err(RejectReason::NotReady));
}

#[test]
fn a_mango_cast_on_an_ally_out_of_reach_walks_up_to_it_first() {
    let (mut world, hero, ally) = a_hero_and_an_ally(ItemId::MANGO, 1, 1500);
    world.mana.get_mut(ally).expect("mana").mana = bota_proto::Fixed::from_int(0);
    use_on(&mut world, hero, 0, ally);
    assert!(world.mana.get(ally).expect("mana").mana.to_int() >= 100);
    assert!(
        world.pos(hero).expect("placed").within(
            world.pos(ally).expect("placed"),
            rules::units(400) + bota_proto::Fixed::from_int(50)
        ),
        "the user stood within four hundred"
    );
    assert!(slot_of(&world, hero, 0).is_none(), "the charge is spent");
}
