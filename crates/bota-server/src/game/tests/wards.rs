//! Observer and sentry wards, invisibility and true sight.

use crate::game::World;
use crate::game::rules;

use super::support::*;

#[test]
fn a_ward_stands_where_it_was_put_and_goes_when_its_time_is_up() {
    let (mut world, hero, spot) = a_hero_with_a_ward(crate::game::ITEM_OBSERVER_WARD);
    let at = spot + bota_proto::Vec2::from_ints(200, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(at), &mut Vec::new()),
        "within reach it may be put down"
    );
    let ward = the_ward(&world);
    assert_eq!(
        world.transform.get(ward).map(|t| t.pos),
        Some(at),
        "and it stands where it was aimed"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the last charge takes the stack with it"
    );
    world.step();
    assert!(
        world.stats.get(ward).map(|s| s.vision) > Some(bota_proto::Fixed::ZERO),
        "an observer sees"
    );
    let left = world
        .expiry
        .get(ward)
        .expect("stands for a time")
        .ticks_left;
    for _ in 0..=left {
        world.step();
    }
    assert!(!world.alive(ward), "and when its time is up it is gone");
}

#[test]
fn an_observer_is_hidden_from_the_other_side_until_a_sentry_finds_it() {
    let (mut world, hero, spot) = a_hero_with_a_ward(crate::game::ITEM_OBSERVER_WARD);
    let at = spot + bota_proto::Vec2::from_ints(200, 0);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(at), &mut Vec::new()));
    world.step();
    let ward = the_ward(&world);
    assert!(
        world.can_see(bota_proto::Team::Radiant, ward),
        "its own side sees it"
    );
    // An enemy hero standing on top of it makes no difference.
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        at,
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Dire, ward),
        "and the other side does not, however close it stands"
    );
    // A sentry of theirs beside it does.
    let sentry = world.spawn_unit(&crate::game::SENTRY_WARD, bota_proto::Team::Dire, at);
    world.settle();
    world.step();
    assert!(
        world.can_see(bota_proto::Team::Dire, ward),
        "true sight finds it"
    );
    assert!(
        world.alive(theirs),
        "and the hero standing there is none the wiser"
    );
    world.despawn(sentry);
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Dire, ward),
        "with the sentry gone it hides again"
    );
}

#[test]
fn a_ward_aimed_out_of_reach_is_not_put_down() {
    let (mut world, hero, spot) = a_hero_with_a_ward(crate::game::ITEM_SENTRY_WARD);
    let far = spot + bota_proto::Vec2::from_ints(2000, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(far), &mut Vec::new()),
        "further than it reaches, nothing is put down"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on it"
    );
}

#[test]
fn a_sentry_alone_takes_nothing_off_what_hides_in_the_dark() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let at = bota_proto::Vec2::from_ints(7000, 7000);
    let ward = world.spawn_unit(&crate::game::OBSERVER_WARD, bota_proto::Team::Radiant, at);
    world.spawn_unit(&crate::game::SENTRY_WARD, bota_proto::Team::Dire, at);
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Dire, ward),
        "true sight over ground nobody is looking at reveals nothing"
    );
}

#[test]
fn true_sight_reaches_only_so_far_over_ground_that_is_watched() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let at = bota_proto::Vec2::from_ints(7000, 7000);
    let ward = world.spawn_unit(&crate::game::OBSERVER_WARD, bota_proto::Team::Radiant, at);
    // Eyes of their own on the spot, so what is being measured is the reach
    // of the sentry and nothing else.
    world.spawn_hero(
        bota_proto::Team::Dire,
        at,
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    let reach = crate::game::SENTRY_WARD.true_sight;
    let near = world.spawn_unit(
        &crate::game::SENTRY_WARD,
        bota_proto::Team::Dire,
        at + bota_proto::Vec2::from_ints(reach - 10, 0),
    );
    world.settle();
    world.step();
    assert!(
        world.can_see(bota_proto::Team::Dire, ward),
        "inside its reach the sentry finds it"
    );
    world.despawn(near);
    world.spawn_unit(
        &crate::game::SENTRY_WARD,
        bota_proto::Team::Dire,
        at + bota_proto::Vec2::from_ints(reach + 10, 0),
    );
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Dire, ward),
        "a step outside it and the ward hides again, watched or not"
    );
}

#[test]
fn a_tower_reveals_what_hides_as_far_as_it_shoots() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let tower = world
        .entities
        .iter()
        .find(|entity| {
            world.kind.get(*entity) == Some(&bota_proto::UnitKind::Tower)
                && world.team.get(*entity) == Some(&bota_proto::Team::Dire)
        })
        .expect("the map has towers");
    let at = world.transform.get(tower).expect("standing").pos;
    let reach = rules::TOWER_ATTACK_RANGE;
    let under = world.spawn_unit(
        &crate::game::OBSERVER_WARD,
        bota_proto::Team::Radiant,
        at + bota_proto::Vec2::from_ints(reach - 10, 0),
    );
    let beyond = world.spawn_unit(
        &crate::game::OBSERVER_WARD,
        bota_proto::Team::Radiant,
        at + bota_proto::Vec2::from_ints(reach + 10, 0),
    );
    world.settle();
    world.step();
    assert!(
        world.can_see(bota_proto::Team::Dire, under),
        "what it could shoot it can also see"
    );
    assert!(
        !world.can_see(bota_proto::Team::Dire, beyond),
        "and what it could not, it cannot, however far it sees"
    );
    assert!(
        rules::TOWER_VISION > reach,
        "the two are not the same reach, or this test proves nothing"
    );
}

#[test]
fn a_ward_takes_no_room_and_is_walked_straight_through() {
    let (mut world, hero, spot) = a_hero_with_a_ward(crate::game::ITEM_OBSERVER_WARD);
    let ahead = spot + bota_proto::Vec2::from_ints(300, 0);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(ahead), &mut Vec::new()));
    let ward = the_ward(&world);
    assert!(world.hull.get(ward).is_none(), "it has no hull to run into");
    // Told to walk to the far side of it, the hero passes over the spot.
    let beyond = spot + bota_proto::Vec2::from_ints(600, 0);
    world.set_order(hero, crate::game::UnitOrder::Move { pos: beyond });
    let mut over = false;
    for _ in 0..120 {
        world.step();
        let at = world.transform.get(hero).expect("standing").pos;
        if at.within(ahead, bota_proto::Fixed::from_int(20)) {
            over = true;
        }
        if at == beyond {
            break;
        }
    }
    assert!(over, "it walked over the spot the ward stands on");
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(beyond),
        "and got where it was going"
    );
    assert!(world.alive(ward), "with the ward none the worse for it");
}

#[test]
fn a_ward_cannot_be_put_where_nothing_may_walk() {
    let (mut world, hero, _spot) = a_hero_with_a_ward(crate::game::ITEM_SENTRY_WARD);
    // Closed ground with open ground to stand on beside it.
    let (stand, wall) = (0..200)
        .flat_map(|x| (0..200).map(move |y| bota_proto::Vec2::from_ints(x * 100, y * 100)))
        .filter(|at| !world.clearance.walkable(*at))
        .find_map(|wall| {
            let beside = wall + bota_proto::Vec2::from_ints(0, 300);
            world.clearance.walkable(beside).then_some((beside, wall))
        })
        .expect("the map has walls with room beside them");
    world.transform.get_mut(hero).expect("hero").pos = stand;
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(wall), &mut Vec::new()),
        "closed ground takes no ward"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on it"
    );
}

#[test]
fn each_ward_stands_up_what_its_own_item_names() {
    for (item, def) in [
        (crate::game::ITEM_OBSERVER_WARD, &crate::game::OBSERVER_WARD),
        (crate::game::ITEM_SENTRY_WARD, &crate::game::SENTRY_WARD),
    ] {
        let (mut world, hero, spot) = a_hero_with_a_ward(item);
        let at = spot + bota_proto::Vec2::from_ints(200, 0);
        assert!(world.use_item(hero, 0, bota_proto::Target::Pos(at), &mut Vec::new()));
        world.step();
        let ward = the_ward(&world);
        let stats = world.stats.get(ward).expect("settled");
        assert_eq!(
            stats.true_sight.to_int(),
            def.true_sight,
            "item {item} stands up what it names"
        );
        assert_eq!(
            stats.vision.to_int(),
            def.vision,
            "and it sees what it sees"
        );
        assert_eq!(stats.hides, def.hides, "and hides as it should");
        // What the wire carries has to tell the two apart, or nothing on
        // screen can.
        let view = world.view(bota_proto::Team::Radiant);
        let shown = view
            .units
            .iter()
            .find(|unit| unit.id == crate::game::wire_id(ward))
            .expect("its own side sees it");
        assert_eq!(shown.true_sight_radius.to_int(), def.true_sight);
    }
}

#[test]
fn both_wards_hide_and_each_reveals_the_other_side() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let at = bota_proto::Vec2::from_ints(7000, 7000);
    // A hero of each side standing right there, so ordinary sight is not what
    // is being measured.
    for (index, side) in [bota_proto::Team::Radiant, bota_proto::Team::Dire]
        .into_iter()
        .enumerate()
    {
        world.spawn_hero(
            side,
            at,
            bota_proto::SlotId(index as u8),
            bota_proto::HeroId(0),
        );
    }
    let theirs = world.spawn_unit(&crate::game::OBSERVER_WARD, bota_proto::Team::Dire, at);
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Radiant, theirs),
        "an observer hides from the other side, hero standing on it or not"
    );
    let ours = world.spawn_unit(&crate::game::SENTRY_WARD, bota_proto::Team::Radiant, at);
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Dire, ours),
        "and so does a sentry: what reveals is not itself revealed"
    );
    assert!(
        world.can_see(bota_proto::Team::Radiant, theirs),
        "the sentry finds theirs"
    );
    // One of their own sentries beside it finds ours in turn.
    let counter = world.spawn_unit(&crate::game::SENTRY_WARD, bota_proto::Team::Dire, at);
    world.settle();
    world.step();
    assert!(
        world.can_see(bota_proto::Team::Dire, ours),
        "a sentry of theirs finds ours"
    );
    assert!(
        world.can_see(bota_proto::Team::Radiant, counter),
        "and ours finds theirs"
    );
}
