//! Where the skill points go.

use bota_proto::{AbilityId, AbilitySlot, HeroId, SlotId, Team, Vec2};

use crate::tests::fixtures;
use crate::{
    BOUNCE, CRIT, FIEND_PLAN, Field, NECROMASTERY, RAZE_FAR, RAZE_MID, RAZE_NEAR, REQUIEM, Role,
    SHADOW_FIEND, SYLLA, SYLLA_PLAN, VOLLEY, next_point, plan_of,
};

/// Shadow Fiend's book, with the razes all standing at one level.
fn fiend_book(raze: u8, necro: u8, requiem: u8, points: bool) -> Vec<bota_proto::AbilityView> {
    let slot = |id: AbilityId, level: u8, max: u8| {
        let mut view = fixtures::ability(id, level, 0);
        view.max_level = max;
        view.can_level = points && level < max;
        view
    };
    vec![
        slot(RAZE_NEAR, raze, 4),
        slot(RAZE_MID, raze, 4),
        slot(RAZE_FAR, raze, 4),
        slot(NECROMASTERY, necro, 4),
        slot(crate::PRESENCE, 0, 4),
        slot(REQUIEM, requiem, 3),
    ]
}

fn field_with(book: Vec<bota_proto::AbilityView>, hero: HeroId) -> bota_proto::WorldView {
    let mut me = fixtures::hero(1, Team::Radiant, Vec2::from_ints(8700, 8800));
    me.hero = Some(hero);
    me.abilities = book;
    let mut units = fixtures::fountains();
    units.push(me);
    let seat = fixtures::seat(SlotId(0), Team::Radiant, hero, Some(fixtures::id(1)));
    fixtures::tick(1000, units, vec![seat])
}

#[test]
fn a_fresh_fiend_puts_its_first_point_in_a_raze() {
    let view = field_with(fiend_book(0, 0, 0, true), SHADOW_FIEND);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), Some(AbilitySlot(0)));
}

#[test]
fn the_second_point_goes_to_the_souls() {
    let view = field_with(fiend_book(1, 0, 0, true), SHADOW_FIEND);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), Some(AbilitySlot(3)));
}

#[test]
fn the_requiem_is_taken_the_moment_it_opens() {
    let view = field_with(fiend_book(3, 2, 0, true), SHADOW_FIEND);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), Some(AbilitySlot(5)));
}

#[test]
fn a_point_that_cannot_be_spent_is_not_spent() {
    // The plan wants the requiem, but the hero is not high enough for it and
    // nothing else will take a point either.
    let mut book = fiend_book(3, 2, 0, false);
    for slot in &mut book {
        slot.can_level = false;
    }
    let view = field_with(book, SHADOW_FIEND);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), None);
}

#[test]
fn a_point_the_plan_has_nowhere_for_still_goes_somewhere() {
    // Every raze is full and the plan's next entry is a raze; the point falls
    // through to whatever will take it.
    let mut book = fiend_book(4, 4, 3, false);
    book[4].can_level = true;
    let view = field_with(book, SHADOW_FIEND);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), Some(AbilitySlot(4)));
}

#[test]
fn sylla_starts_with_the_bolt() {
    let book = vec![
        {
            let mut one = fixtures::ability(CRIT, 0, 0);
            one.can_level = true;
            one
        },
        fixtures::ability(crate::FRENZY, 0, 0),
        {
            let mut one = fixtures::ability(BOUNCE, 0, 550);
            one.can_level = true;
            one
        },
        fixtures::ability(VOLLEY, 0, 700),
    ];
    let view = field_with(book, SYLLA);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(next_point(&field), Some(AbilitySlot(2)));
}

#[test]
fn a_plan_spends_one_point_per_hero_level() {
    assert_eq!(FIEND_PLAN.len(), 10);
    assert_eq!(SYLLA_PLAN.len(), 10);
    assert_eq!(plan_of(SHADOW_FIEND), &FIEND_PLAN);
    assert_eq!(plan_of(SYLLA), &SYLLA_PLAN);
    assert!(plan_of(HeroId(99)).is_empty());
}

#[test]
fn no_plan_asks_for_more_of_an_ability_than_it_holds() {
    let counted = |plan: &[AbilityId], id: AbilityId| plan.iter().filter(|had| **had == id).count();
    assert!(counted(&FIEND_PLAN, RAZE_NEAR) <= 4);
    assert!(counted(&FIEND_PLAN, NECROMASTERY) <= 4);
    assert!(counted(&FIEND_PLAN, REQUIEM) <= 3);
    assert!(counted(&SYLLA_PLAN, BOUNCE) <= 4);
    assert!(counted(&SYLLA_PLAN, CRIT) <= 4);
    assert!(counted(&SYLLA_PLAN, VOLLEY) <= 3);
}

#[test]
fn an_ultimate_is_never_asked_for_before_its_level() {
    // The ultimate opens at hero levels six, eight and ten, which are the
    // sixth, eighth and tenth points.
    for (at, id) in FIEND_PLAN.iter().enumerate() {
        if *id == REQUIEM {
            assert!(at + 1 >= 6, "the requiem is asked for at point {}", at + 1);
        }
    }
    for (at, id) in SYLLA_PLAN.iter().enumerate() {
        if *id == VOLLEY {
            assert!(at + 1 >= 6, "the volley is asked for at point {}", at + 1);
        }
    }
}
