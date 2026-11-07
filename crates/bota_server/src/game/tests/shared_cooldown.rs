//! Items that wait as one: identical items, and items naming the same `AbilitySharedCooldown`,
//! every name from the install's `items.txt` through the catalogue.

use bota_core::rules;
use bota_proto::{EventKind, Fixed, ItemId, ItemSlot, Order, Target};

use crate::game::World;

use super::item_numbers::out_of_fountain_after_buying;
use super::support::*;

/// Two Ghost Scepters carried wait as one (Dota: identical items share their cooldown, and
/// `AbilitySharedCooldown` "ethereal" ties the Ghost Scepter's): one used, the other shows the
/// same 22 seconds and its use is refused.
#[test]
fn two_ghost_scepters_share_one_cooldown() {
    let (mut world, hero) =
        out_of_fountain_after_buying(&[ItemId::GHOST_SCEPTER, ItemId::GHOST_SCEPTER]);
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    let wait = |world: &World, at| slot_of(world, hero, at).expect("carried").cooldown;
    assert!(wait(&world, 0) > 0);
    assert_eq!(wait(&world, 1), wait(&world, 0), "the other waits as long");
    run(&mut world, 5 * rules::TICKS_PER_SECOND);
    let events = seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(1),
            target: Target::None,
        },
    );
    assert!(
        !events.iter().any(|event| matches!(
            event.kind(),
            EventKind::ItemUsed { item: ItemId::GHOST_SCEPTER, .. }
        )),
        "the second is not used while the first waits"
    );
}

/// A Magic Wand and a Magic Stick wait as one (`AbilitySharedCooldown` "magicwand" on both): the
/// wand used, the stick waits as long.
#[test]
fn a_magic_stick_and_a_magic_wand_share_one_cooldown() {
    // The wand first: bought beside a stick it would be built of it.
    let (mut world, hero) =
        out_of_fountain_after_buying(&[ItemId::MAGIC_WAND, ItemId::MAGIC_STICK]);
    for at in 0..2 {
        if let Some(Some(stack)) = world.inventory.get_mut(hero).map(|bag| &mut bag.slots[at]) {
            stack.charges = 5;
        }
    }
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(100);
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    let wand = slot_of(&world, hero, 0).expect("carried").cooldown;
    assert!(wand > 0, "the wand waits");
    assert_eq!(
        slot_of(&world, hero, 1).expect("carried").cooldown,
        wand,
        "and so does the stick"
    );
}
