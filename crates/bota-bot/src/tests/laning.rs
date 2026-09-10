//! Laying a lane out from the buildings a snapshot shows.

use bota_proto::{SlotId, Team, UnitKind, Vec2};

use crate::tests::fixtures;
use crate::{Lane, Role, Which, span};

/// The Dota map's tier-one towers, both sides, all three lanes.
fn towers() -> Vec<bota_proto::UnitView> {
    vec![
        fixtures::works(
            10,
            UnitKind::Tower,
            Team::Radiant,
            Vec2::from_ints(7672, 7808),
        ),
        fixtures::works(
            11,
            UnitKind::Tower,
            Team::Radiant,
            Vec2::from_ints(2880, 11072),
        ),
        fixtures::works(
            12,
            UnitKind::Tower,
            Team::Radiant,
            Vec2::from_ints(14076, 2837),
        ),
        fixtures::works(13, UnitKind::Tower, Team::Dire, Vec2::from_ints(9740, 9868)),
        fixtures::works(
            14,
            UnitKind::Tower,
            Team::Dire,
            Vec2::from_ints(3941, 15252),
        ),
        fixtures::works(
            15,
            UnitKind::Tower,
            Team::Dire,
            Vec2::from_ints(15485, 6976),
        ),
    ]
}

fn view() -> bota_proto::WorldView {
    let mut units = fixtures::fountains();
    units.extend(towers());
    fixtures::tick(
        1000,
        units,
        vec![fixtures::seat(
            SlotId(0),
            Team::Radiant,
            bota_proto::HeroId(2),
            None,
        )],
    )
}

#[test]
fn a_tower_is_put_in_the_lane_it_stands_in() {
    let view = view();
    let mid = Lane::read(&view, Which::Mid, Team::Radiant).expect("fountains stand");
    assert_eq!(mid.mine, Some(Vec2::from_ints(7672, 7808)));
    assert_eq!(mid.theirs, Some(Vec2::from_ints(9740, 9868)));

    let top = Lane::read(&view, Which::Top, Team::Radiant).expect("fountains stand");
    assert_eq!(top.mine, Some(Vec2::from_ints(2880, 11072)));
    assert_eq!(top.theirs, Some(Vec2::from_ints(3941, 15252)));

    let bot = Lane::read(&view, Which::Bottom, Team::Radiant).expect("fountains stand");
    assert_eq!(bot.mine, Some(Vec2::from_ints(14076, 2837)));
    assert_eq!(bot.theirs, Some(Vec2::from_ints(15485, 6976)));
}

#[test]
fn the_middle_runs_straight_and_the_others_bend() {
    let view = view();
    let mid = Lane::read(&view, Which::Mid, Team::Radiant).expect("fountains stand");
    assert_eq!(mid.route.len(), 4);

    // The bend of the top lane sits where the map's own corner is, within a
    // tower's width of it.
    let top = Lane::read(&view, Which::Top, Team::Radiant).expect("fountains stand");
    let corner = top.route[2];
    assert!(
        span(corner, Vec2::from_ints(3050, 15150)) < 300.0,
        "corner at {corner:?}"
    );
}

#[test]
fn a_lane_reads_the_same_way_round_from_either_side() {
    let view = view();
    let ours = Lane::read(&view, Which::Mid, Team::Radiant).expect("fountains stand");
    let theirs = Lane::read(&view, Which::Mid, Team::Dire).expect("fountains stand");
    assert_eq!(ours.route.first(), Some(&Vec2::from_ints(1760, 2278)));
    assert_eq!(theirs.route.first(), Some(&Vec2::from_ints(16624, 16064)));
    assert_eq!(ours.where_they_meet(), theirs.where_they_meet());
}

#[test]
fn the_far_end_is_the_whole_of_the_way_along() {
    let view = view();
    let lane = Lane::read(&view, Which::Mid, Team::Radiant).expect("fountains stand");
    let home = lane.spot_along(0.0);
    let away = lane.spot_along(1.0);
    assert_eq!(home, Vec2::from_ints(1760, 2278));
    assert_eq!(away, Vec2::from_ints(16624, 16064));
    assert!(lane.how_far_along(away) > lane.how_far_along(home));
}

#[test]
fn a_step_back_along_the_lane_is_a_step_towards_home() {
    let view = view();
    let lane = Lane::read(&view, Which::Mid, Team::Radiant).expect("fountains stand");
    let meet = lane.where_they_meet();
    let back = lane.spot_from(meet, -500.0);
    assert!(lane.how_far_along(back) < lane.how_far_along(meet));
    assert!((span(back, meet) - 500.0).abs() < 60.0, "went {back:?}");
}

#[test]
fn a_role_takes_the_lane_its_side_farms_it_from() {
    assert_eq!(Role::Carry.lane(Team::Radiant), Which::Bottom);
    assert_eq!(Role::Carry.lane(Team::Dire), Which::Top);
    assert_eq!(Role::Offlane.lane(Team::Radiant), Which::Top);
    assert_eq!(Role::Mid.lane(Team::Dire), Which::Mid);
}

#[test]
fn a_role_is_numbered_as_it_is_spoken_of() {
    for number in 1..=5u8 {
        let role = Role::of(number).expect("five roles");
        assert_eq!(role.number(), number);
    }
    assert_eq!(Role::of(0), None);
    assert_eq!(Role::of(6), None);
}
