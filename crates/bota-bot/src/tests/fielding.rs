//! Reading one tick into the shape the policy weighs.

use bota_proto::{HeroId, SlotId, Team, UnitKind, Vec2};

use crate::tests::fixtures;
use crate::{Field, NEARBY, RAZE_NEAR, Role, SALVE, TANGO};

const MID: Vec2 = Vec2::from_ints(8700, 8800);

fn field_of(units: Vec<bota_proto::UnitView>, me: Option<u32>) -> bota_proto::WorldView {
    let mut all = fixtures::fountains();
    all.extend(units);
    let seat = fixtures::seat(SlotId(0), Team::Radiant, HeroId(2), me.map(fixtures::id));
    fixtures::tick(1000, all, vec![seat])
}

#[test]
fn creeps_come_out_nearest_first() {
    let view = field_of(
        vec![
            fixtures::hero(1, Team::Radiant, MID),
            fixtures::creep(2, Team::Dire, Vec2::from_ints(9200, 9300), 550),
            fixtures::creep(3, Team::Dire, Vec2::from_ints(8900, 9000), 550),
        ],
        Some(1),
    );
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(field.creeps.len(), 2);
    assert_eq!(field.creeps[0].id, fixtures::id(3));
    assert_eq!(field.creeps[1].id, fixtures::id(2));
}

#[test]
fn a_wave_on_another_lane_is_not_this_hero_s_business() {
    let far = Vec2::from_ints(MID.x.to_int() + NEARBY + 500, MID.y.to_int());
    let view = field_of(
        vec![
            fixtures::hero(1, Team::Radiant, MID),
            fixtures::creep(2, Team::Dire, far, 550),
        ],
        Some(1),
    );
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert!(field.creeps.is_empty());
}

#[test]
fn a_seat_with_no_body_standing_reads_as_one() {
    let view = field_of(vec![], None);
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert!(!field.alive());
    assert_eq!(field.at(), Vec2::from_ints(1760, 2278));
}

#[test]
fn the_shop_reaches_as_far_as_it_reaches() {
    let at_home = field_of(
        vec![fixtures::hero(
            1,
            Team::Radiant,
            Vec2::from_ints(2000, 2400),
        )],
        Some(1),
    );
    let field = Field::of(&at_home, SlotId(0), Role::Mid).expect("a seat in the view");
    assert!(field.at_shop());

    let in_lane = field_of(vec![fixtures::hero(1, Team::Radiant, MID)], Some(1));
    let field = Field::of(&in_lane, SlotId(0), Role::Mid).expect("a seat in the view");
    assert!(!field.at_shop());
}

#[test]
fn an_ability_is_found_by_what_sits_in_the_slot() {
    let mut me = fixtures::hero(1, Team::Radiant, MID);
    me.abilities = vec![
        fixtures::ability(crate::NECROMASTERY, 1, 0),
        fixtures::ability(RAZE_NEAR, 2, 200),
    ];
    let view = field_of(vec![me], Some(1));
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    let (slot, held) = field.ability(RAZE_NEAR).expect("the raze is carried");
    assert_eq!(slot, bota_proto::AbilitySlot(1));
    assert_eq!(held.level, 2);
    assert_eq!(field.ability(crate::MEAT_HOOK), None);
}

#[test]
fn only_the_working_slots_of_a_bag_are_reached_into() {
    let mut me = fixtures::hero(1, Team::Radiant, MID);
    me.items[0] = Some(fixtures::item(TANGO));
    me.items[7] = Some(fixtures::item(SALVE));
    let view = field_of(vec![me], Some(1));
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(
        field.item(TANGO).map(|(slot, _)| slot),
        Some(bota_proto::ItemSlot(0))
    );
    assert_eq!(field.item(SALVE), None, "the backpack works nothing");
}

#[test]
fn souls_are_counted_off_what_shows_on_the_hero() {
    let mut me = fixtures::hero(1, Team::Radiant, MID);
    me.effects = vec![fixtures::souls(7)];
    let view = field_of(vec![me], Some(1));
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(field.souls(), 7);
}

#[test]
fn towers_of_each_side_are_told_apart() {
    let view = field_of(
        vec![
            fixtures::hero(1, Team::Radiant, MID),
            fixtures::works(
                2,
                UnitKind::Tower,
                Team::Radiant,
                Vec2::from_ints(7672, 7808),
            ),
            fixtures::works(3, UnitKind::Tower, Team::Dire, Vec2::from_ints(9740, 9868)),
            fixtures::works(
                4,
                UnitKind::Ancient,
                Team::Dire,
                Vec2::from_ints(14744, 14216),
            ),
        ],
        Some(1),
    );
    let field = Field::of(&view, SlotId(0), Role::Mid).expect("a seat in the view");
    assert_eq!(field.own_towers.len(), 1);
    assert_eq!(field.enemy_towers.len(), 1);
    assert_eq!(field.enemy_works.len(), 2);
    assert_eq!(field.enemy_works[0].kind, UnitKind::Tower);
}
