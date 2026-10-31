//! Trusted local setup's economic events: gold and items reach a seat at the
//! named tick, through the shop's own delivery, deterministically; a setup
//! naming one out of bounds is refused.

use bota_proto::{ItemId, ItemSlot, Order, OutgoingOrder, SlotId};

use crate::game::{
    EconomyEvent, EconomyEventError, MAX_ECONOMY_EVENT_GOLD, MAX_ECONOMY_EVENT_ITEMS,
    MAX_ECONOMY_EVENTS, MatchConfig, MatchConfigError, OrderDelay,
};
use crate::local::{LocalError, LocalMatch};

use super::local::mid_config;

/// The tick the test's event happens at.
const AT: u32 = 60;

fn gift(slot: u8, tick: u32, gold: i32, items: Vec<ItemId>) -> EconomyEvent {
    EconomyEvent {
        slot: SlotId(slot),
        tick,
        gold,
        items,
    }
}

fn with_events(events: Vec<EconomyEvent>) -> MatchConfig {
    MatchConfig {
        economy_events: events,
        ..mid_config()
    }
}

/// Steps the match with no orders until it has run `tick` ticks.
fn run_to(local: &mut LocalMatch, tick: u32) {
    while local.tick() < tick {
        local.step(&[&[], &[]]).expect("a step");
    }
}

fn gold(local: &LocalMatch, seat: usize) -> i32 {
    local.world().seats[seat].gold
}

/// Every item the seat holds in the hero's bag and its stash.
fn held(local: &LocalMatch, seat: usize) -> Vec<ItemId> {
    let world = local.world();
    let hero = local.hero_of(seat).expect("a hero");
    let bag = world.inventory.get(hero).expect("a bag");
    bag.held()
        .chain(world.seats[seat].stash.held())
        .map(|stack| stack.id)
        .collect()
}

#[test]
fn an_economic_event_hands_out_gold_and_items_at_its_tick_and_they_sell_like_bought_ones() {
    let cfg = with_events(vec![gift(
        0,
        AT,
        1_000,
        vec![ItemId::CIRCLET, ItemId::BOOTS],
    )]);
    let (mut local, _) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    run_to(&mut local, AT - 1);
    // Both heroes idle at their fountains: the same gold, nothing given yet.
    assert_eq!(gold(&local, 0), gold(&local, 1));
    assert!(!held(&local, 0).contains(&ItemId::BOOTS));
    run_to(&mut local, AT + 1);
    assert_eq!(gold(&local, 0), gold(&local, 1) + 1_000);
    let items = held(&local, 0);
    assert!(
        items.contains(&ItemId::BOOTS) && items.contains(&ItemId::CIRCLET),
        "{items:?}"
    );
    assert!(!held(&local, 1).contains(&ItemId::BOOTS));
    // Sold back through the real order path like a bought item.
    let hero = local.hero_of(0).expect("a hero");
    let at = local
        .world()
        .inventory
        .get(hero)
        .expect("a bag")
        .slots
        .iter()
        .position(|slot| slot.as_ref().is_some_and(|stack| stack.id == ItemId::BOOTS))
        .expect("the Boots in the bag");
    let before = gold(&local, 0);
    let sell = OutgoingOrder {
        seq: 1,
        unit: None,
        order: Order::Sell {
            slot: ItemSlot(u8::try_from(at).expect("a bag slot")),
        },
        seen: local.tick(),
        queue: false,
    };
    local.step(&[&[sell], &[]]).expect("a step");
    local.step(&[&[], &[]]).expect("a step");
    assert!(!held(&local, 0).contains(&ItemId::BOOTS));
    assert!(gold(&local, 0) > before, "the sale pays");
}

#[test]
fn a_match_with_economic_events_plays_out_the_same_every_time() {
    let cfg = with_events(vec![
        gift(1, 0, 300, vec![ItemId::SLIPPERS]),
        gift(0, AT, -200, vec![ItemId::MANGO, ItemId::CIRCLET]),
    ]);
    let hashes = || {
        let (mut local, _) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
        (0..3)
            .map(|leg| {
                run_to(&mut local, (leg + 1) * AT);
                local.world().hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(hashes(), hashes());
    let (mut plain, _) = LocalMatch::new(&mid_config(), &[OrderDelay::NONE]).expect("a match");
    run_to(&mut plain, 3 * AT);
    assert_ne!(
        hashes()[2],
        plain.world().hash(),
        "the events changed the match"
    );
}

#[test]
fn a_setup_with_an_economic_event_out_of_bounds_is_refused() {
    let refused = |events: Vec<EconomyEvent>| match LocalMatch::new(
        &with_events(events),
        &[OrderDelay::NONE],
    ) {
        Err(LocalError::Config { message }) => message,
        Ok(_) => panic!("refused"),
        Err(other) => panic!("refused as a setup: {other}"),
    };
    let check = |events: Vec<EconomyEvent>, expected: MatchConfigError| {
        assert_eq!(with_events(events.clone()).validate(), Err(expected));
        assert!(!refused(events).is_empty());
    };
    let error = |error| MatchConfigError::EconomyEvent { at: 0, error };
    check(
        vec![gift(7, 0, 0, Vec::new())],
        error(EconomyEventError::UnknownSeat),
    );
    check(
        vec![gift(0, 0, MAX_ECONOMY_EVENT_GOLD + 1, Vec::new())],
        error(EconomyEventError::TooMuchGold),
    );
    check(
        vec![gift(0, 0, -MAX_ECONOMY_EVENT_GOLD - 1, Vec::new())],
        error(EconomyEventError::TooMuchGold),
    );
    check(
        vec![gift(0, u32::MAX, 0, Vec::new())],
        error(EconomyEventError::TooLate),
    );
    check(
        vec![gift(
            0,
            0,
            0,
            vec![ItemId::MANGO; MAX_ECONOMY_EVENT_ITEMS + 1],
        )],
        error(EconomyEventError::TooManyItems),
    );
    let unsold = ItemId(u16::MAX);
    check(
        vec![gift(0, 0, 0, vec![unsold])],
        error(EconomyEventError::NotSold { item: unsold }),
    );
    check(
        vec![gift(0, 0, 0, Vec::new()); MAX_ECONOMY_EVENTS + 1],
        MatchConfigError::TooManyEconomyEvents {
            count: MAX_ECONOMY_EVENTS + 1,
        },
    );
    assert_eq!(
        with_events(vec![gift(0, 0, 0, Vec::new()); MAX_ECONOMY_EVENTS]).validate(),
        Ok(())
    );
}
