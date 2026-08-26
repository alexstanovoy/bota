//! The contract between the numbers, the deeds and the game.

use bota_proto::{SlotId, Team};

use crate::tests::{a_busy_tick, a_tick, unit};
use crate::{Aim, BLOCKS, DEEDS, Deed, Field, LAYOUT, NUMBERS, Role, allowed, shown, sight};

/// The field of the busy tick, for the seat this crate plays.
fn busy() -> bota_proto::WorldView {
    a_busy_tick()
}

#[test]
fn every_number_has_a_place_and_the_layout_adds_up() {
    let view = busy();
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
    let numbers = sight(&field);
    assert_eq!(
        numbers.len(),
        NUMBERS,
        "what was filled in is not the length the layout claims"
    );
    assert_eq!(
        LAYOUT.iter().map(|(_, size)| size).sum::<usize>(),
        NUMBERS,
        "the layout does not add up to its own total"
    );
    assert!(
        numbers.iter().all(|number| number.is_finite()),
        "a number that is not a number would poison every weight it touches"
    );
}

#[test]
fn every_deed_number_means_one_deed_and_the_same_one_back() {
    assert_eq!(
        BLOCKS.iter().map(|(_, size)| size).sum::<usize>(),
        DEEDS,
        "the blocks do not add up to their own total"
    );
    for index in 0..DEEDS {
        let deed = Deed::at(index).expect("every number in the list is a deed");
        assert_eq!(
            deed.index(),
            index,
            "{deed:?} does not go back to the number it came from"
        );
    }
    assert_eq!(Deed::at(DEEDS), None, "there is nothing past the end");
}

#[test]
fn a_deed_that_is_allowed_can_be_carried_out() {
    // The one thing that must hold on every tick: a flag saying yes and a
    // decode saying nothing would throw the tick away, and there is one order
    // a tick.
    for view in [busy(), a_tick(Vec::new(), 0), a_tick(Vec::new(), 600)] {
        let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
        let may = allowed(&field);
        assert_eq!(may.len(), DEEDS, "a flag per deed");
        for (index, allowed) in may.iter().enumerate() {
            if !allowed {
                continue;
            }
            let deed = Deed::at(index).expect("a flag belongs to a deed");
            assert!(
                deed.into_ask(&field).is_some(),
                "{deed:?} is allowed but turns into no order"
            );
        }
    }
}

#[test]
fn a_cast_at_a_body_is_offered_only_within_its_reach() {
    // A cast in reach goes off on the tick it is asked for; one aimed past
    // its reach would be walked in by the server, and the next tick's deed
    // would call the walk off.
    let mut view = busy();
    for body in &mut view.units {
        if body.kind == bota_proto::UnitKind::Hero && body.owner == Some(SlotId(0)) {
            for held in &mut body.abilities {
                held.range = 550;
            }
        }
    }
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
    let may = allowed(&field);
    // The other hero stands past 550 away, the nearest creep inside it.
    assert!(
        !may[Deed::Cast(2, Aim::Hero).index()],
        "a cast at a hero out of reach is withheld"
    );
    assert!(
        may[Deed::Cast(2, Aim::Creep).index()],
        "and one at a creep in reach is offered"
    );
}

#[test]
fn a_tick_with_a_body_standing_always_has_something_to_do() {
    let view = busy();
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
    let seen = shown(&field);
    assert!(
        seen.anything_to_do(),
        "a bot that may do nothing at all has nothing to learn from"
    );
    assert!(
        seen.allowed[Deed::Stand.index()],
        "standing is always among them, so there is never an empty choice"
    );
}

#[test]
fn nothing_standing_means_nothing_allowed() {
    let mut view = a_tick(Vec::new(), 0);
    view.players[0].unit = None;
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is still in the tick");
    let seen = shown(&field);
    assert!(
        !seen.anything_to_do(),
        "a dead hero decides nothing, and saying so is better than a stray order"
    );
    assert_eq!(
        seen.numbers.len(),
        NUMBERS,
        "the numbers are still the numbers"
    );
}

#[test]
fn the_bodies_named_are_the_bodies_shown() {
    // The whole reason the reading of a tick is one place: creep three in the
    // numbers and creep three in "swing at creep three" have to be one creep.
    let view = busy();
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
    for at in 0..field.creeps.len() {
        let named = field.creeps[at].id;
        let ask = Deed::Swing(at)
            .into_ask(&field)
            .expect("a creep in the list can be swung at");
        assert_eq!(
            ask.order,
            bota_proto::Order::Attack {
                target: bota_proto::Target::Unit(named)
            },
            "swinging at the {at}th creep did not name the {at}th creep"
        );
    }
}

#[test]
fn the_order_of_bodies_does_not_shift_under_a_deed() {
    // Two creeps the same distance off must not swap places from tick to tick,
    // or the number that named one names the other next time.
    let same = |idx: u32| {
        unit(
            idx,
            bota_proto::UnitKind::CreepMelee,
            Team::Dire,
            (7400, 7400),
            100,
        )
    };
    let one = a_tick(vec![same(30), same(31)], 0);
    let other = a_tick(vec![same(31), same(30)], 0);
    let named = |view: &bota_proto::WorldView| {
        let field = Field::of(view, SlotId(0), Role::Mid).expect("the seat is in the tick");
        field
            .creeps
            .iter()
            .map(|creep| creep.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        named(&one),
        named(&other),
        "the same tick read twice gave two different orders of the same creeps"
    );
}

#[test]
fn what_the_bot_is_shown_is_the_same_from_either_end_of_the_map() {
    // The point of turning everything about: a mirrored tick should read
    // nearly the same, so one set of weights serves both sides.
    let view = busy();
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("the seat is in the tick");
    let creep = field.creeps.first().expect("there are creeps");
    let (forward, _) = field.seen_from_here(creep.pos);
    assert!(
        forward > 0.0,
        "a creep of the other side stands forward of the bot, whichever end it started at"
    );
    let home = field.home.expect("its own fountain is always in sight");
    let (back, _) = field.seen_from_here(home);
    assert!(back < 0.0, "and its own fountain is behind it");
}
