//! Buying, and knowing what is already owned.

use bota_proto::{HeroId, ItemId, SlotId, Team, Vec2};

use crate::tests::fixtures;
use crate::{
    BRANCH, CIRCLET, CLARITY, FIEND_GOODS, Field, MAGIC_STICK, MAGIC_WAND, MANTLE, NULL_TALISMAN,
    RECIPE_MAGIC_WAND, RECIPE_NULL_TALISMAN, Role, SCROLL, SHADOW_FIEND, SYLLA_GOODS, Stall, TANGO,
    shopping_list,
};

/// A shop holding everything the lists name, with the builds it puts together.
fn stall() -> Stall {
    Stall {
        entries: vec![
            fixtures::sold(TANGO, 90, vec![]),
            fixtures::sold(CLARITY, 50, vec![]),
            fixtures::sold(BRANCH, 50, vec![]),
            fixtures::sold(SCROLL, 100, vec![]),
            fixtures::sold(CIRCLET, 155, vec![]),
            fixtures::sold(MANTLE, 140, vec![]),
            fixtures::sold(RECIPE_NULL_TALISMAN, 210, vec![]),
            fixtures::sold(MAGIC_STICK, 200, vec![]),
            fixtures::sold(RECIPE_MAGIC_WAND, 150, vec![]),
            fixtures::sold(
                NULL_TALISMAN,
                505,
                vec![CIRCLET, MANTLE, RECIPE_NULL_TALISMAN],
            ),
            fixtures::sold(
                MAGIC_WAND,
                450,
                vec![MAGIC_STICK, BRANCH, BRANCH, RECIPE_MAGIC_WAND],
            ),
        ],
    }
}

fn field_holding(bag: Vec<ItemId>, gold: i32) -> bota_proto::WorldView {
    let mut me = fixtures::hero(1, Team::Radiant, Vec2::from_ints(1900, 2400));
    for (at, id) in bag.iter().enumerate() {
        me.items[at] = Some(fixtures::item(*id));
    }
    let mut units = fixtures::fountains();
    units.push(me);
    let mut seat = fixtures::seat(
        SlotId(0),
        Team::Radiant,
        SHADOW_FIEND,
        Some(fixtures::id(1)),
    );
    seat.gold = Some(gold);
    fixtures::tick(1000, units, vec![seat])
}

#[test]
fn a_part_inside_a_build_is_still_a_part_that_was_paid_for() {
    let stall = stall();
    assert_eq!(stall.how_many_within(NULL_TALISMAN, CIRCLET), 1);
    assert_eq!(stall.how_many_within(MAGIC_WAND, BRANCH), 2);
    assert_eq!(stall.how_many_within(MAGIC_WAND, MAGIC_STICK), 1);
    assert_eq!(stall.how_many_within(MAGIC_WAND, CIRCLET), 0);
    assert_eq!(stall.how_many_within(BRANCH, BRANCH), 1);
}

#[test]
fn the_first_thing_on_the_list_is_the_first_thing_bought() {
    let view = field_holding(vec![], 600);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), Some(TANGO));
}

#[test]
fn the_list_is_not_skipped_over_to_reach_what_is_affordable() {
    // Everything up to the talisman recipe is held, and there is not enough
    // for the recipe. Nothing further down is bought with the gold the recipe
    // is waiting for.
    let held = vec![TANGO, BRANCH, BRANCH, SCROLL, CIRCLET, MANTLE];
    let view = field_holding(held, 150);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), None);

    let view = field_holding(vec![TANGO, BRANCH, BRANCH, SCROLL, CIRCLET, MANTLE], 250);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), Some(RECIPE_NULL_TALISMAN));
}

#[test]
fn a_build_already_held_is_not_bought_over_again() {
    // The talisman swallowed the circlet, the mantle and the recipe; the list
    // moves on rather than buying a second circlet.
    let held = vec![TANGO, CLARITY, BRANCH, BRANCH, SCROLL, NULL_TALISMAN];
    let view = field_holding(held, 600);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), Some(crate::SAGES_MASK));
}

#[test]
fn what_waits_in_the_stash_counts_as_owned() {
    let mut view = field_holding(vec![], 600);
    view.players[0].stash = Some(vec![
        Some(fixtures::item(TANGO)),
        None,
        None,
        None,
        None,
        None,
    ]);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), Some(CLARITY));
}

#[test]
fn a_consumable_with_nowhere_to_go_is_stepped_over() {
    // Every working slot is full, so the clarity the list wants next has no
    // room. What holds it up is room and not gold, so the walk goes on to
    // what the gold is actually being saved for.
    let full = vec![TANGO, BRANCH, BRANCH, SCROLL, CIRCLET, MANTLE];
    let view = field_holding(full, 250);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(field.free_slots(), 0);
    assert_eq!(stall().next_buy(&field), Some(RECIPE_NULL_TALISMAN));
}

#[test]
fn a_drink_is_bought_again_once_it_has_been_drunk() {
    let view = field_holding(vec![TANGO, BRANCH, BRANCH, SCROLL], 600);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(stall().next_buy(&field), Some(CLARITY));
}

#[test]
fn a_hero_with_no_list_of_its_own_still_has_one() {
    assert_eq!(shopping_list(SHADOW_FIEND), &FIEND_GOODS);
    assert_eq!(shopping_list(crate::SYLLA), &SYLLA_GOODS);
    assert!(!shopping_list(HeroId(99)).is_empty());
}

#[test]
fn a_list_starts_with_what_the_starting_gold_pays_for() {
    let stall = stall();
    let start: i32 = FIEND_GOODS
        .iter()
        .take(6)
        .map(|item| stall.cost_of(*item))
        .sum();
    assert!(start <= 600, "the opening costs {start}");
}

#[test]
fn a_price_the_shop_does_not_name_is_nothing_at_all() {
    assert_eq!(stall().cost_of(ItemId(999)), 0);
    assert!(stall().parts_of(ItemId(999)).is_empty());
}

#[test]
fn the_shop_is_read_off_the_terms_of_the_match() {
    let info = fixtures::started(
        vec![
            fixtures::sold(TANGO, 90, vec![]),
            fixtures::sold(
                MAGIC_WAND,
                450,
                vec![MAGIC_STICK, BRANCH, BRANCH, RECIPE_MAGIC_WAND],
            ),
        ],
        Vec::new(),
    );
    let stall = Stall::of(&info);
    assert_eq!(stall.cost_of(TANGO), 90);
    assert_eq!(stall.parts_of(MAGIC_WAND).len(), 4);
}
