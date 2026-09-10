//! Shadow Fiend's spellwork: what a raze is cast at, and what turns first.

use bota_proto::{AbilitySlot, Angle, HeroId, Order, SlotId, Target, Team, Vec2, WorldView};

use crate::tests::fixtures;
use crate::{
    Ask, Beat, Field, NECROMASTERY, PRESENCE, RAZE_FAR, RAZE_MID, RAZE_NEAR, REQUIEM, Role,
    SHADOW_FIEND, fiend_aim, fiend_spell,
};

const HOME: Vec2 = Vec2::from_ints(8000, 8000);

/// Shadow Fiend's book, every raze ready at one level and the rest unlearned.
fn book(raze: u8, requiem: u8) -> Vec<bota_proto::AbilityView> {
    let reach = [200, 450, 700];
    let mut slots: Vec<bota_proto::AbilityView> = [RAZE_NEAR, RAZE_MID, RAZE_FAR]
        .iter()
        .enumerate()
        .map(|(at, id)| fixtures::ability(*id, raze, reach[at]))
        .collect();
    slots.push(fixtures::ability(NECROMASTERY, 1, 0));
    slots.push(fixtures::ability(PRESENCE, 0, 900));
    let mut ult = fixtures::ability(REQUIEM, requiem, 900);
    ult.max_level = 3;
    ult.mana_cost = 150;
    slots.push(ult);
    slots
}

/// A tick with Shadow Fiend looking one way and the units given standing.
fn tick(
    facing: u16,
    raze: u8,
    requiem: u8,
    souls: u32,
    others: Vec<bota_proto::UnitView>,
) -> WorldView {
    let mut me = fixtures::hero(1, Team::Radiant, HOME);
    me.hero = Some(SHADOW_FIEND);
    me.facing = Angle { brads: facing };
    me.abilities = book(raze, requiem);
    me.effects = vec![fixtures::souls(souls)];
    let mut units = fixtures::fountains();
    units.push(me);
    units.extend(others);
    let seat = fixtures::seat(
        SlotId(0),
        Team::Radiant,
        SHADOW_FIEND,
        Some(fixtures::id(1)),
    );
    fixtures::tick(1000, units, vec![seat])
}

fn read(view: &WorldView) -> Field<'_> {
    Field::of(view, SlotId(0), Role::Mid).expect("a seat in the view")
}

#[test]
fn a_raze_goes_off_at_a_foe_the_hero_is_already_looking_at() {
    // Looking east, with an enemy hero four hundred and fifty units east:
    // the middle raze lands on him.
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8450, 8000));
    let view = tick(0, 1, 0, 0, vec![foe]);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(
        fiend_spell(&field, &beat),
        Some(Ask::cast(AbilitySlot(1), Target::None))
    );
}

#[test]
fn nothing_is_cast_down_an_empty_line() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8000, 8600));
    let view = tick(0, 1, 0, 0, vec![foe]);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(fiend_spell(&field, &beat), None);
}

#[test]
fn a_foe_off_the_line_is_turned_towards() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8000, 8600));
    let view = tick(0, 1, 0, 0, vec![foe]);
    let field = read(&view);
    assert_eq!(fiend_aim(&field), Some(Vec2::from_ints(8000, 8600)));
}

#[test]
fn a_foe_already_in_the_burn_is_not_turned_towards() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8450, 8000));
    let view = tick(0, 1, 0, 0, vec![foe]);
    let field = read(&view);
    assert_eq!(fiend_aim(&field), None);
}

#[test]
fn a_foe_beyond_every_raze_is_not_turned_towards() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8000, 9400));
    let view = tick(0, 1, 0, 0, vec![foe]);
    let field = read(&view);
    assert_eq!(fiend_aim(&field), None);
}

#[test]
fn an_unlearned_raze_is_not_cast() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8450, 8000));
    let view = tick(0, 0, 0, 0, vec![foe]);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(fiend_spell(&field, &beat), None);
    assert_eq!(fiend_aim(&field), None);
}

#[test]
fn a_raze_still_waiting_is_not_cast() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8450, 8000));
    let mut view = tick(0, 1, 0, 0, vec![foe]);
    for slot in &mut view.units[2].abilities {
        slot.cooldown_left = 40;
    }
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(fiend_spell(&field, &beat), None);
}

#[test]
fn a_raze_there_is_no_mana_for_is_not_cast() {
    let foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8450, 8000));
    let mut view = tick(0, 1, 0, 0, vec![foe]);
    view.units[2].mana = 10;
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(fiend_spell(&field, &beat), None);
}

#[test]
fn the_requiem_goes_off_for_a_crowd() {
    let foes = vec![
        fixtures::hero(5, Team::Dire, Vec2::from_ints(8300, 8000)),
        fixtures::hero(6, Team::Dire, Vec2::from_ints(8000, 8300)),
    ];
    let view = tick(16384, 1, 1, 6, foes);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(
        fiend_spell(&field, &beat),
        Some(Ask::cast(AbilitySlot(5), Target::None))
    );
}

#[test]
fn the_requiem_is_not_let_go_with_no_souls_gathered() {
    let foes = vec![
        fixtures::hero(5, Team::Dire, Vec2::from_ints(8300, 8000)),
        fixtures::hero(6, Team::Dire, Vec2::from_ints(8000, 8300)),
    ];
    let view = tick(16384, 0, 1, 0, foes);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert_eq!(fiend_spell(&field, &beat), None);
}

#[test]
fn the_requiem_is_let_go_at_one_it_would_kill() {
    let mut foe = fixtures::hero(5, Team::Dire, Vec2::from_ints(8300, 8000));
    foe.hp = 30;
    // Looking away from him, so no raze is offered instead.
    let view = tick(32768, 0, 1, 6, vec![foe]);
    let field = read(&view);
    let beat = Beat::new(SHADOW_FIEND, 30);
    assert!(matches!(
        fiend_spell(&field, &beat),
        Some(Ask {
            unit: None,
            order: Order::Cast {
                slot: AbilitySlot(5),
                ..
            }
        })
    ));
}

#[test]
fn another_hero_s_spellwork_is_none_of_the_fiend_s() {
    let mut view = tick(0, 1, 0, 0, vec![]);
    view.units[2].hero = Some(HeroId(0));
    view.players[0].hero = HeroId(0);
    let field = read(&view);
    assert_eq!(field.hero, HeroId(0));
}
