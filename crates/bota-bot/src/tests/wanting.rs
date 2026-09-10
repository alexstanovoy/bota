//! Saying a want, and not saying it again straight away.

use bota_proto::{AbilitySlot, Order, Target, Vec2};

use crate::tests::fixtures;
use crate::{Ask, RESEND_TICKS, Steady, same_want};

#[test]
fn the_first_want_of_all_goes_out() {
    let mut steady = Steady::new();
    assert!(steady.worth_sending(0, Ask::walk_to(Vec2::from_ints(100, 100))));
}

#[test]
fn the_want_in_hand_is_not_said_again_straight_away() {
    let mut steady = Steady::new();
    let ask = Ask::swing_at(fixtures::id(4));
    assert!(steady.worth_sending(10, ask));
    assert!(!steady.worth_sending(11, ask));
    assert!(!steady.worth_sending(10 + RESEND_TICKS - 1, ask));
    assert!(steady.worth_sending(10 + RESEND_TICKS, ask));
}

#[test]
fn a_different_want_goes_out_at_once() {
    let mut steady = Steady::new();
    assert!(steady.worth_sending(10, Ask::swing_at(fixtures::id(4))));
    assert!(steady.worth_sending(11, Ask::swing_at(fixtures::id(5))));
}

#[test]
fn two_walks_a_step_apart_are_one_want() {
    let mut steady = Steady::new();
    assert!(steady.worth_sending(10, Ask::walk_to(Vec2::from_ints(4000, 4000))));
    assert!(!steady.worth_sending(11, Ask::walk_to(Vec2::from_ints(4050, 4000))));
    assert!(steady.worth_sending(12, Ask::walk_to(Vec2::from_ints(4400, 4000))));
}

#[test]
fn a_walk_and_a_fight_to_the_same_spot_are_not_one_want() {
    let spot = Vec2::from_ints(4000, 4000);
    assert!(!same_want(Ask::walk_to(spot), Ask::fight_towards(spot)));
    assert!(same_want(Ask::walk_to(spot), Ask::walk_to(spot)));
}

#[test]
fn a_want_for_the_courier_is_not_a_want_for_the_hero() {
    let order = Order::Cast {
        slot: AbilitySlot(0),
        target: Target::None,
    };
    assert!(!same_want(
        Ask::mine(order),
        Ask::of(fixtures::id(9), order)
    ));
}

#[test]
fn forgetting_lets_the_same_want_out_again() {
    let mut steady = Steady::new();
    let ask = Ask::swing_at(fixtures::id(4));
    assert!(steady.worth_sending(10, ask));
    assert!(!steady.worth_sending(11, ask));
    steady.forget();
    assert!(steady.worth_sending(12, ask));
}
