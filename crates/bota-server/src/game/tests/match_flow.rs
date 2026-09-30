//! How a match ends, kills, experience, gold and respawns.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn a_fallen_ancient_ends_the_match() {
    let mut world = World::new();
    let ancient = world.spawn_unit(
        crate::game::ancient_of(bota_proto::Team::Dire),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    world.settle();
    // An Ancient shrugs damage off while it stands invulnerable, so this one
    // is brought down directly.
    world.health.insert(ancient, Health { hp: Fixed::ZERO });
    let mut events = Vec::new();
    world.bury(vec![(ancient, None)], &mut events);
    assert_eq!(world.winner, Some(bota_proto::Team::Radiant));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, bota_proto::EventKind::StructureDestroyed { .. }))
    );
}

#[test]
fn simultaneous_ancient_deaths_preserve_the_first_terminal_result() {
    let mut world = World::new();
    let radiant = world.spawn_unit(
        crate::game::ancient_of(bota_proto::Team::Radiant),
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let dire = world.spawn_unit(
        crate::game::ancient_of(bota_proto::Team::Dire),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(2000, 2000),
    );
    world.settle();
    let mut events = Vec::new();

    world.bury(vec![(radiant, None), (dire, None)], &mut events);

    assert_eq!(world.victor(), Some(bota_proto::Team::Dire));
}

#[test]
fn every_map_sits_at_the_place_its_id_names() {
    // The per-map route cache is indexed by the id, not by the position, so
    // the two have to agree.
    for (at, map) in crate::game::MAPS.iter().enumerate() {
        assert_eq!(
            usize::from(map.id.0),
            at,
            "map {} is out of place",
            map.id.0
        );
        assert_eq!(map.index(), at);
    }
}

#[test]
fn a_kill_pays_the_one_who_struck_last_and_feeds_the_side() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    let prey = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5050, 5000),
    );
    world.settle();
    world.health.insert(prey, Health { hp: Fixed::ZERO });
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(hero))], &mut events);
    assert_eq!(world.seats[0].gold, rules::MELEE_CREEP_BOUNTY);
    assert_eq!(world.seats[0].last_hits, 1);
    assert_eq!(world.seats[0].xp, rules::MELEE_CREEP_XP);
}

#[test]
fn nearby_allied_heroes_split_a_units_experience_evenly() {
    let mut world = World::new();
    for (slot, x) in [
        (bota_proto::SlotId(0), 5_000),
        (bota_proto::SlotId(1), 5_100),
    ] {
        let hero = world.spawn_hero(
            Team::Radiant,
            bota_proto::Vec2::from_ints(x, 5_000),
            slot,
            bota_proto::HeroId(0),
        );
        let mut seat = crate::game::Seat::new(
            slot,
            Team::Radiant,
            bota_proto::HeroId(0),
            0,
            rules::STASH_SLOTS,
        );
        seat.unit = Some(hero);
        world.seats.push(seat);
    }
    let prey = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5_050, 5_000),
    );
    world.settle();
    let mut events = Vec::new();

    world.bury(vec![(prey, world.seats[0].unit)], &mut events);

    let share = rules::MELEE_CREEP_XP / 2;
    assert_eq!(world.seats[0].xp, share);
    assert_eq!(world.seats[1].xp, share);
}

#[test]
fn dead_allied_heroes_do_not_take_an_experience_share() {
    let mut world = World::new();
    for (slot, x) in [
        (bota_proto::SlotId(0), 5_000),
        (bota_proto::SlotId(1), 5_100),
    ] {
        let hero = world.spawn_hero(
            Team::Radiant,
            bota_proto::Vec2::from_ints(x, 5_000),
            slot,
            bota_proto::HeroId(0),
        );
        let mut seat = crate::game::Seat::new(
            slot,
            Team::Radiant,
            bota_proto::HeroId(0),
            0,
            rules::STASH_SLOTS,
        );
        seat.unit = Some(hero);
        world.seats.push(seat);
    }
    let prey = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5_050, 5_000),
    );
    world.settle();
    let dead = world.seats[1].unit.expect("second hero");
    world.health.get_mut(dead).expect("hero health").hp = Fixed::ZERO;
    let mut events = Vec::new();

    world.bury(vec![(prey, world.seats[0].unit)], &mut events);

    assert_eq!(world.seats[0].xp, rules::MELEE_CREEP_XP);
    assert_eq!(world.seats[1].xp, 0);
}

#[test]
fn bringing_down_your_own_is_a_deny_and_pays_the_other_side_nothing() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    let own = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5050, 5000),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(own, Some(hero))], &mut events);
    assert_eq!(world.seats[0].denies, 1);
    assert_eq!(world.seats[0].gold, 0, "a deny pays no gold");
    assert_eq!(world.seats[0].xp, 0, "and no experience");
}

#[test]
fn denied_lane_creeps_give_reduced_experience_to_nearby_enemies() {
    let mut world = World::new();
    for (slot, team, x) in [
        (bota_proto::SlotId(0), Team::Radiant, 5_000),
        (bota_proto::SlotId(1), Team::Dire, 5_100),
    ] {
        let hero = world.spawn_hero(
            team,
            bota_proto::Vec2::from_ints(x, 5_000),
            slot,
            bota_proto::HeroId(0),
        );
        let mut seat =
            crate::game::Seat::new(slot, team, bota_proto::HeroId(0), 0, rules::STASH_SLOTS);
        seat.unit = Some(hero);
        world.seats.push(seat);
    }
    let denied = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5_050, 5_000),
    );
    world.settle();
    let mut events = Vec::new();

    world.bury(vec![(denied, world.seats[0].unit)], &mut events);

    assert_eq!(world.seats[0].xp, 0, "the denying side gains no experience");
    assert_eq!(
        world.seats[1].xp,
        rules::MELEE_CREEP_XP * rules::DENIED_XP_PCT / 100
    );
}

#[test]
fn a_hero_kill_pays_by_the_streak_and_a_death_costs_gold_by_the_level() {
    let mut world = World::new();
    let hunter = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let prey = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5050, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    for (slot, team) in [
        (bota_proto::SlotId(0), bota_proto::Team::Radiant),
        (bota_proto::SlotId(1), bota_proto::Team::Dire),
    ] {
        world.seats.push(crate::game::Seat::new(
            slot,
            team,
            bota_proto::HeroId(0),
            0,
            rules::STASH_SLOTS,
        ));
    }
    world.seats[0].unit = Some(hunter);
    world.seats[1].unit = Some(prey);
    world.seats[1].gold = 500;
    world.seats[1].net_worth = 500;
    world.seats[1].level = 4;
    world.seats[1].streak = 3;
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(hunter))], &mut events);

    let head = rules::HERO_KILL_BOUNTY_BASE + 3 * rules::HERO_KILL_BOUNTY_PER_STREAK;
    assert_eq!(
        world.seats[0].gold, head,
        "the head is priced by the streak it wore"
    );
    assert_eq!(world.seats[0].kills, 1);
    assert_eq!(
        world.seats[0].streak, 1,
        "and the kill starts a streak of the killer's own"
    );
    assert_eq!(world.seats[0].last_hits, 0, "a hero is not a last hit");
    assert_eq!(
        world.seats[0].xp,
        World::hero_kill_xp(0, 3, 4),
        "experience pays by what the fallen had earned and the streak it wore"
    );
    assert_eq!(
        world.seats[1].gold,
        500 - 500 / rules::DEATH_GOLD_LOSS_SHARE,
        "dying costs a share of the net worth"
    );
    assert_eq!(
        world.seats[1].streak, 0,
        "and the streak ends with the body"
    );
    let told = events.iter().find_map(|event| match event.kind {
        bota_proto::EventKind::Died { gold, .. } => Some(gold),
        _ => None,
    });
    assert_eq!(told, Some(head), "the event says what the kill paid");
}

#[test]
fn a_death_never_takes_more_gold_than_the_purse_holds() {
    let mut world = World::new();
    let prey = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Dire,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(prey);
    // Fifty in the purse and the rest in items: a fortieth of the worth is
    // a hundred, and only the fifty is there to take.
    world.seats[0].gold = 50;
    world.seats[0].net_worth = 4000;
    world.seats[0].level = 10;
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, None)], &mut events);
    assert_eq!(world.seats[0].gold, 0, "the purse is emptied, not owed");
    assert_eq!(
        world.seats[0].net_worth, 3950,
        "and the items keep their worth"
    );
    let told = events.iter().find_map(|event| match event.kind {
        bota_proto::EventKind::Died { gold, .. } => Some(gold),
        _ => None,
    });
    assert_eq!(told, Some(0), "and nobody was paid for the fall");
}

#[test]
fn a_fallen_hero_comes_back_at_its_fountain() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(9000, 9216),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    assert!(world.seats[0].unit.is_none(), "its body is gone");
    assert_eq!(world.seats[0].deaths, 1);
    let wait = world.seats[0].respawn_left;
    assert!(wait > 0);
    for _ in 0..=wait {
        world.step();
    }
    let back = world.seats[0].unit.expect("it came back");
    let at = world.transform.get(back).expect("standing").pos;
    // It comes back beside the fountain, on the spot it first stood up on.
    assert_eq!(
        at,
        crate::game::hero_spawn_pos(map, bota_proto::Team::Radiant),
        "it came back somewhere else"
    );
    assert!(
        world.clearance.walkable(at),
        "and on ground it can walk off"
    );
    let full = world.stats.get(back).expect("settled").max_hp;
    assert_eq!(world.health.get(back).map(|h| h.hp), Some(full), "and full");
}

#[test]
fn a_hero_stands_up_beside_its_fountain_and_not_in_it() {
    for id in [0u16, 1] {
        let map = crate::game::map_of(bota_proto::MapId(id));
        let world = World::on_map(map);
        for team in [bota_proto::Team::Radiant, bota_proto::Team::Dire] {
            let at = crate::game::hero_spawn_pos(map, team);
            assert!(
                world.clearance.walkable(at),
                "map {id}, {team:?}: a hero stands up on ground it can walk off"
            );
        }
    }
}

#[test]
fn a_hero_keeps_what_it_learned_and_carried_through_a_death() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.level.insert(hero, crate::game::Level(7));
    world.seats[0].level = 7;
    let mut events = Vec::new();
    assert!(world.learn(hero, 1, &mut events));
    assert!(world.learn(hero, 1, &mut events));
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    assert!(world.buy(bota_proto::SlotId(0), boots, &mut events));
    world.bury(vec![(hero, None)], &mut events);
    assert!(
        world.seats[0].kept.is_some(),
        "what it had waits with its seat while the body is gone"
    );
    for _ in 0..=World::respawn_wait(7) {
        world.step();
    }
    let back = world.seats[0].unit.expect("it came back");
    assert_eq!(world.seats[0].level, 7, "its level is its own");
    assert_eq!(
        world.abilities.get(back).expect("casts").slots[1].level,
        2,
        "and so is what it learned"
    );
    assert_eq!(
        world
            .inventory
            .get(back)
            .and_then(|bag| bag.slots[0])
            .map(|s| s.id),
        Some(boots),
        "and what it carried came back with it"
    );
    assert!(
        world.seats[0].kept.is_none(),
        "and the seat holds nothing of it any more"
    );
    world.step();
    assert_eq!(
        world.stats.get(back).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(
            crate::game::HERO.move_speed + 45
        )),
        "the boots work again at once"
    );
}

#[test]
fn what_a_hero_owes_runs_down_while_it_is_dead() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.level.insert(hero, crate::game::Level(7));
    world.seats[0].level = 7;
    let mut events = Vec::new();
    assert!(world.learn(hero, 1, &mut events));
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].cooldown = 300;
    }
    hand_item(&mut world, hero, crate::game::ITEM_QUELLING_BLADE, 0);
    if let Some(bag) = world.inventory.get_mut(hero)
        && let Some(Some(stack)) = bag.slots.get_mut(0)
    {
        stack.cooldown = 300;
    }
    world.bury(vec![(hero, None)], &mut events);
    let wait = World::respawn_wait(7);
    assert!(wait > 200, "the wait is long enough to measure against");
    for _ in 0..=wait {
        world.step();
    }
    let back = world.seats[0].unit.expect("it came back");
    assert_eq!(
        world.abilities.get(back).expect("casts").slots[1].cooldown,
        0,
        "the ability came off its wait while the body was gone"
    );
    assert_eq!(
        world.inventory.get(back).expect("has a bag").slots[0].map(|s| s.cooldown),
        Some(0),
        "and so did the item"
    );
}

/// The game's own numbers for a hero's head and its wait, as the wiki gives
/// them: a streak of three ends for 13.75 experience a level, ten or more
/// for 110, a tenth of a thousand earned and a bit besides for the share.
#[test]
fn a_heros_head_and_its_wait_are_priced_as_the_game_prices_them() {
    assert_eq!(World::hero_kill_xp(0, 0, 1), rules::HERO_KILL_XP_BASE);
    assert_eq!(World::hero_kill_xp(0, 2, 9), rules::HERO_KILL_XP_BASE);
    assert_eq!(World::hero_kill_xp(0, 3, 4), rules::HERO_KILL_XP_BASE + 55);
    assert_eq!(
        World::hero_kill_xp(0, 10, 2),
        rules::HERO_KILL_XP_BASE + 220
    );
    assert_eq!(
        World::hero_kill_xp(0, 14, 2),
        rules::HERO_KILL_XP_BASE + 220
    );
    assert_eq!(
        World::hero_kill_xp(1000, 0, 5),
        rules::HERO_KILL_XP_BASE + 130
    );
    assert_eq!(World::respawn_wait(1), 12 * rules::TICKS_PER_SECOND);
    assert_eq!(World::respawn_wait(12), 44 * rules::TICKS_PER_SECOND);
    assert_eq!(World::respawn_wait(25), 100 * rules::TICKS_PER_SECOND);
    assert_eq!(World::respawn_wait(30), 100 * rules::TICKS_PER_SECOND);
    assert_eq!(
        rules::XP_THRESHOLDS[5],
        2440,
        "the sixth level, the ultimate's"
    );
    assert_eq!(rules::XP_THRESHOLDS[29], 63900, "the last");
}

#[test]
fn a_heros_blows_on_enemy_heroes_and_structures_are_counted_in_the_final_stats() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let hero = world.spawn_hero(
        Team::Radiant,
        at,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let foe = world.spawn_hero(
        Team::Dire,
        at + bota_proto::Vec2::from_ints(100, 0),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    let ancient = world.spawn_unit(
        crate::game::ancient_of(Team::Dire),
        Team::Dire,
        at + bota_proto::Vec2::from_ints(0, 100),
    );
    for (slot, team, unit) in [(0, Team::Radiant, hero), (1, Team::Dire, foe)] {
        world.seats.push(crate::game::Seat::new(
            bota_proto::SlotId(slot),
            team,
            bota_proto::HeroId(0),
            0,
            rules::STASH_SLOTS,
        ));
        world.seats[usize::from(slot)].unit = Some(unit);
    }
    world.settle();
    let (mut to_hero, mut to_ancient) = (0, 0);
    for (aim, tally) in [(foe, &mut to_hero), (ancient, &mut to_ancient)] {
        world.set_order(
            hero,
            crate::game::UnitOrder::Attack {
                target: aim,
                last_seen: at,
            },
        );
        for _ in 0..120 {
            for event in world.advance(&[]) {
                if let bota_proto::EventKind::Damaged {
                    source: Some(source),
                    target,
                    amount,
                    ..
                } = event.kind
                    && source == crate::game::wire_id(hero)
                    && target == crate::game::wire_id(aim)
                {
                    *tally += amount;
                }
            }
        }
    }
    assert!(to_hero > 0 && to_ancient > 0, "both were struck");
    let stats = world.match_stats();
    assert_eq!(stats.slots[0].hero_damage, to_hero);
    assert_eq!(stats.slots[0].structure_damage, to_ancient);
    assert_eq!(
        stats.slots[1].structure_damage, 0,
        "the foe hit no building"
    );
}
