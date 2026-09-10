//! Finding a tree to eat.

use bota_proto::{SlotId, Team, Vec2};

use crate::Forest;
use crate::tests::fixtures;

fn forest() -> Forest {
    Forest::of(&fixtures::started(
        Vec::new(),
        vec![
            Vec2::from_ints(4000, 4000),
            Vec2::from_ints(4100, 4000),
            Vec2::from_ints(9000, 9000),
        ],
    ))
}

fn empty_tick() -> bota_proto::WorldView {
    fixtures::tick(
        1000,
        fixtures::fountains(),
        vec![fixtures::seat(
            SlotId(0),
            Team::Radiant,
            bota_proto::HeroId(2),
            None,
        )],
    )
}

#[test]
fn the_nearest_tree_within_reach_is_the_one_taken() {
    let view = empty_tick();
    let at = Vec2::from_ints(4050, 4000);
    assert_eq!(
        forest().nearest_standing(&view, at, 200.0),
        Some(Vec2::from_ints(4000, 4000))
    );
}

#[test]
fn a_tree_out_of_reach_is_no_tree_at_all() {
    let view = empty_tick();
    let at = Vec2::from_ints(4050, 4000);
    assert_eq!(forest().nearest_standing(&view, at, 20.0), None);
}

#[test]
fn a_tree_already_down_is_not_offered() {
    let mut view = empty_tick();
    view.felled_trees = vec![0, 1];
    let at = Vec2::from_ints(4050, 4000);
    assert_eq!(forest().nearest_standing(&view, at, 200.0), None);
}

#[test]
fn a_tree_put_up_during_the_match_counts() {
    let mut view = empty_tick();
    view.planted_trees = vec![Vec2::from_ints(4060, 4000)];
    let at = Vec2::from_ints(4050, 4000);
    assert_eq!(
        forest().nearest_standing(&view, at, 200.0),
        Some(Vec2::from_ints(4060, 4000))
    );
}
