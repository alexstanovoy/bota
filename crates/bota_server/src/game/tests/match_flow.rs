//! How a match ends, kills, experience, gold and respawns.

use bota_core::{
    MAPS, MELEE_CREEP, ModifierKind, SHADOW_FIEND, ancient_of, hero_kill_xp, hero_spawn_pos,
    map_of, passive_gold_by, respawn_wait, rules,
};

use crate::game::{Entity, Level, UnitOrder, World, wire_id};
use bota_proto::{
    AbilitySlot, EventKind, Fixed, HeroId, ItemId, ItemSlot, MapId, Order, SlotId, StatusFlags,
    Target, Team, Vec2,
};

use super::support::*;

#[test]
fn a_fallen_ancient_ends_the_match() {
    let mut world = World::new();
    let ancient = world.spawn_unit(
        ancient_of(Team::Dire),
        Team::Dire,
        Vec2::from_ints(1000, 1000),
    );
    world.settle();
    // Invulnerable while it stands, so it is brought down directly.
    world.health.insert(ancient, health(0));
    let mut events = Vec::new();
    world.bury(vec![(ancient, None)], &mut events);
    assert_eq!(world.winner, Some(Team::Radiant));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind(), EventKind::StructureDestroyed { .. }))
    );
}

#[test]
fn simultaneous_ancient_deaths_preserve_the_first_terminal_result() {
    let mut world = World::new();
    let radiant = world.spawn_unit(
        ancient_of(Team::Radiant),
        Team::Radiant,
        Vec2::from_ints(1000, 1000),
    );
    let dire = world.spawn_unit(
        ancient_of(Team::Dire),
        Team::Dire,
        Vec2::from_ints(2000, 2000),
    );
    world.settle();
    let mut events = Vec::new();

    world.bury(vec![(radiant, None), (dire, None)], &mut events);

    assert_eq!(world.victor(), Some(Team::Dire));
}

#[test]
fn every_map_sits_at_the_place_its_id_names() {
    // The per-map clearance base field is indexed by the id, not by the
    // position.
    for (at, map) in MAPS.iter().enumerate() {
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
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let prey = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5050, 5000));
    world.settle();
    world.health.insert(prey, health(0));
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(hero))], &mut events);
    let paid = world.seats[0].gold;
    assert!(
        (rules::MELEE_CREEP_BOUNTY.min..=rules::MELEE_CREEP_BOUNTY.max).contains(&paid),
        "paid {paid}, out of the melee creep's range"
    );
    assert_eq!(world.seats[0].last_hits, 1);
    assert_eq!(world.seats[0].xp, rules::MELEE_CREEP_XP);
}

#[test]
fn living_allied_heroes_nearby_split_a_units_experience_evenly() {
    let share = rules::MELEE_CREEP_XP / 2;
    for (second_dead, xp) in [(false, [share, share]), (true, [rules::MELEE_CREEP_XP, 0])] {
        let mut world = World::new();
        let first = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5_000, 5_000));
        let second = seat_hero(&mut world, 1, Team::Radiant, Vec2::from_ints(5_100, 5_000));
        let prey = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5_050, 5_000));
        world.settle();
        if second_dead {
            world.health.insert(second, health(0));
        }
        world.bury(vec![(prey, Some(first))], &mut Vec::new());
        assert_eq!(
            [world.seats[0].xp, world.seats[1].xp],
            xp,
            "second hero dead: {second_dead}"
        );
    }
}

#[test]
fn bringing_down_your_own_is_a_deny_that_feeds_only_nearby_enemies() {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5_000, 5_000),
        HeroId::SHADOW_FIEND,
    );
    seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5_100, 5_000));
    let own = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5_050, 5_000));
    world.settle();
    world.bury(vec![(own, Some(hero))], &mut Vec::new());
    assert_eq!(world.seats[0].denies, 1);
    assert_eq!(world.seats[0].gold, 0, "a deny pays no gold");
    assert_eq!(world.seats[0].xp, 0, "and no experience");
    assert_eq!(
        world.seats[1].xp,
        rules::MELEE_CREEP_XP * rules::DENIED_XP_PCT / 100,
        "the enemy beside it gets a reduced share"
    );
}

#[test]
fn a_hero_kill_pays_dotas_bounty_first_blood_and_assist_gold() {
    let mut world = World::new();
    let hunter = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let prey = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5050, 5000));
    world.seats[0].net_worth = 500;
    world.seats[1].gold = 500;
    world.seats[1].net_worth = 500;
    world.seats[1].level = 4;
    world.seats[1].streak = 3;
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(hunter))], &mut events);

    // 125 + 8 × level 4 + 60 for a streak of three + 135 First Blood, and
    // 15 + 50 + 0.037 × 500 assist gold at even net worth, rounded down.
    let head = 125 + 32 + 60 + 135 + 83;
    assert_eq!(world.seats[0].gold, head, "Dota 7.41f's head and assist");
    assert_eq!(world.seats[0].kills, 1);
    assert_eq!(
        world.seats[0].streak, 1,
        "and the kill starts a streak of the killer's own"
    );
    assert_eq!(world.seats[0].last_hits, 0, "a hero is not a last hit");
    assert_eq!(
        world.seats[0].xp,
        hero_kill_xp(0, 3, 4),
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
    assert_eq!(
        paid(&events),
        Some(head),
        "the event says what the kill paid"
    );

    // The second head pays no First Blood: a level-one hero on no streak
    // is 125 + 8 and 15 + 50 + 0.037 × its 1000, the killer side leading
    // 1435 to 1000 and paid 1000/1435 of it.
    let prey = seat_hero(&mut world, 2, Team::Dire, Vec2::from_ints(5050, 5000));
    world.seats[2].net_worth = 1000;
    world.seats[2].level = 1;
    world.seats[1].net_worth = 0;
    world.seats[0].net_worth = 1435;
    world.settle();
    let before = world.seats[0].gold;
    world.bury(vec![(prey, Some(hunter))], &mut Vec::new());
    let assist = (15_000 + 50_000 + 37 * 1000) * 1000 / 1435 / 1000;
    assert_eq!(world.seats[0].gold - before, 133 + assist);
}

#[test]
fn passive_gold_is_ninety_a_minute_from_the_horn_and_more_from_twelve() {
    let cfg = config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let start = world.seats[0].gold;
    let horn = world.schedule().horn;
    run(&mut world, horn);
    assert_eq!(world.seats[0].gold, start, "nothing before the horn");
    run(&mut world, 60 * rules::TICKS_PER_SECOND);
    assert_eq!(
        world.seats[0].gold - start,
        90,
        "ninety in the first minute"
    );
    let minute = 60 * rules::TICKS_PER_SECOND;
    assert_eq!(passive_gold_by(20), 1, "one gold every 0.67 s");
    assert_eq!(passive_gold_by(12 * minute), 12 * 90);
    assert_eq!(
        passive_gold_by(13 * minute) - passive_gold_by(12 * minute),
        94,
        "94.8 a minute from twelve, whole gold only"
    );
    assert_eq!(passive_gold_by(15 * minute), 1080 + 3 * 948 / 10);
}

#[test]
fn a_death_never_takes_more_gold_than_the_purse_holds() {
    let mut world = World::new();
    let prey = seat_hero(&mut world, 0, Team::Dire, Vec2::from_ints(5000, 5000));
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
    assert_eq!(paid(&events), Some(0), "and nobody was paid for the fall");
}

#[test]
fn a_fallen_hero_comes_back_at_its_fountain() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(9000, 9216),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.bury(vec![(hero, None)], &mut Vec::new());
    assert!(world.seats[0].dead(), "it lies dead");
    assert_eq!(world.seats[0].deaths, 1);
    assert!(world.seats[0].respawn_left > 0);
    let back = back_on_its_feet(&mut world);
    let at = world.transform.get(back).expect("standing").pos;
    assert_eq!(
        at,
        hero_spawn_pos(map, Team::Radiant),
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
        let map = map_of(MapId(id));
        let world = World::on_map(map);
        for team in [Team::Radiant, Team::Dire] {
            let at = hero_spawn_pos(map, team);
            assert!(
                world.clearance.walkable(at),
                "map {id}, {team:?}: a hero stands up on ground it can walk off"
            );
        }
    }
}

#[test]
fn a_hero_keeps_what_it_learned_and_carried_through_a_death() {
    let (mut world, hero) = a_level_seven_hero();
    let mut events = Vec::new();
    assert!(world.learn(hero, 1));
    let boots = ItemId::BOOTS;
    assert!(world.buy(SlotId(0), boots, &mut events));
    world.bury(vec![(hero, None)], &mut events);
    assert!(world.seats[0].dead());
    assert_eq!(world.seats[0].unit, Some(hero), "the same hero lies dead");
    assert_eq!(world.pos(hero), None, "and stands nowhere");
    let back = back_on_its_feet(&mut world);
    assert_eq!(back, hero, "the same hero stands again, as Dota's");
    assert_eq!(world.seats[0].level, 7, "its level is its own");
    assert_eq!(
        world.abilities.get(back).expect("casts").slots[1].level,
        2,
        "and so is what it learned"
    );
    assert_eq!(
        slot_of(&world, back, 0).map(|s| s.id),
        Some(boots),
        "and what it carried came back with it"
    );
    world.step();
    assert_eq!(
        world.stats.get(back).map(|s| s.move_speed),
        Some(Fixed::from_int(SHADOW_FIEND.move_speed + 45)),
        "the boots work again at once"
    );
}

#[test]
fn what_a_hero_owes_runs_down_while_it_is_dead() {
    let (mut world, hero) = a_level_seven_hero();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].cooldown = 300;
    }
    hand_item(&mut world, hero, ItemId::QUELLING_BLADE, 0);
    if let Some(bag) = world.inventory.get_mut(hero)
        && let Some(Some(stack)) = bag.slots.get_mut(0)
    {
        stack.cooldown = 300;
    }
    world.bury(vec![(hero, None)], &mut Vec::new());
    assert!(
        world.seats[0].respawn_left > 300,
        "the wait outlasts the cooldowns"
    );
    let back = back_on_its_feet(&mut world);
    assert_eq!(
        world.abilities.get(back).expect("casts").slots[1].cooldown,
        0,
        "the ability came off its wait while the body was gone"
    );
    assert_eq!(
        slot_of(&world, back, 0).map(|s| s.cooldown),
        Some(0),
        "and so did the item"
    );
}

/// A streak of three ends for 13.75 experience a level, ten or more for 110,
/// and a thousand earned adds a tenth and a bit for the share.
/// Dota keeps a hero that neutral creeps brought down waiting at least 26
/// seconds (Liquipedia, Death); any other death waits its level's time. Every
/// side is told how long the fallen hero has left, as Dota shows every hero's
/// timer to everyone.
#[test]
fn a_hero_neutral_creeps_brought_down_waits_at_least_twenty_six_seconds() {
    for (level, by_neutrals, seconds) in [(1, true, 26), (1, false, 12), (8, true, 30)] {
        let map = map_of(MapId(1));
        let mut world = World::on_map(map);
        let hero = seated_hero(
            &mut world,
            Vec2::from_ints(9000, 9216),
            HeroId::SHADOW_FIEND,
        );
        world.seats[0].level = level;
        let side = if by_neutrals {
            Team::Neutral
        } else {
            Team::Dire
        };
        let killer = world.spawn_creep(&MELEE_CREEP, side, Vec2::from_ints(9200, 9216), 0, 0);
        world.settle();
        world.bury(vec![(hero, Some(killer))], &mut Vec::new());
        let wait = seconds * rules::TICKS_PER_SECOND;
        assert_eq!(world.seats[0].respawn_left, wait, "level {level}, {side:?}");
        for viewer in [Team::Radiant, Team::Dire] {
            let told = world
                .view(viewer)
                .players
                .iter()
                .find(|player| player.slot == world.seats[0].slot)
                .map(|player| player.respawn_left);
            assert_eq!(told, Some(wait), "told to {viewer:?}");
        }
    }
}

#[test]
fn a_heros_head_and_its_wait_are_priced_as_the_game_prices_them() {
    assert_eq!(hero_kill_xp(0, 0, 1), rules::HERO_KILL_XP_BASE);
    assert_eq!(hero_kill_xp(0, 2, 9), rules::HERO_KILL_XP_BASE);
    assert_eq!(hero_kill_xp(0, 3, 4), rules::HERO_KILL_XP_BASE + 55);
    assert_eq!(hero_kill_xp(0, 10, 2), rules::HERO_KILL_XP_BASE + 220);
    assert_eq!(hero_kill_xp(0, 14, 2), rules::HERO_KILL_XP_BASE + 220);
    assert_eq!(hero_kill_xp(1000, 0, 5), rules::HERO_KILL_XP_BASE + 130);
    assert_eq!(respawn_wait(1, false), 12 * rules::TICKS_PER_SECOND);
    assert_eq!(respawn_wait(12, false), 44 * rules::TICKS_PER_SECOND);
    assert_eq!(respawn_wait(25, false), 100 * rules::TICKS_PER_SECOND);
    assert_eq!(respawn_wait(30, false), 100 * rules::TICKS_PER_SECOND);
}

#[test]
fn a_heads_experience_stops_growing_at_level_twenty_five() {
    // Liquipedia, Experience: 100 + 0.13 × 34400 at level 25 and above.
    assert_eq!(hero_kill_xp(34_400, 0, 25), 4572);
    assert_eq!(hero_kill_xp(60_000, 0, 29), 4572);
}

#[test]
fn a_heros_blows_on_enemy_heroes_and_structures_are_counted_in_the_final_stats() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let hero = seated_hero(&mut world, at, HeroId::SHADOW_FIEND);
    let foe = seat_hero(&mut world, 1, Team::Dire, at + Vec2::from_ints(100, 0));
    let ancient = world.spawn_unit(
        ancient_of(Team::Dire),
        Team::Dire,
        at + Vec2::from_ints(0, 100),
    );
    world.settle();
    let (mut to_hero, mut to_ancient) = (0, 0);
    // The foe is hit for a few swings only, so it does not fall mid-count.
    for (aim, tally, ticks) in [(foe, &mut to_hero, 60), (ancient, &mut to_ancient, 120)] {
        world.set_order(
            hero,
            UnitOrder::Attack {
                target: aim,
                last_seen: at,
            },
        );
        for _ in 0..ticks {
            for event in world.advance(&[]) {
                if let EventKind::Damaged {
                    source: Some(source),
                    target,
                    amount,
                    ..
                } = event.kind()
                    && source == wire_id(hero)
                    && target == wire_id(aim)
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

/// The match hero at the seventh level with its second ability learned.
/// Dota's hero is one entity all match ("After the first spawn, a hero is
/// not created again. However, it respawns", Liquipedia, *Hero (unit
/// type)*). Dead, it is the unit its seat names, on the wire too; it stands
/// nowhere, so no view carries it among its units and nothing sees it; it
/// mends nothing, and its cooldowns keep running.
#[test]
fn a_dead_hero_is_the_same_unit_standing_nowhere_mending_nothing_its_cooldowns_running() {
    let (mut world, hero) = a_level_seven_hero();
    world.abilities.get_mut(hero).expect("a book").slots[1].cooldown = 300;
    world.bury(vec![(hero, None)], &mut Vec::new());
    world.step();
    for team in [Team::Radiant, Team::Dire] {
        let view = world.view(team);
        let seat = view.player(SlotId(0)).expect("the seat");
        assert_eq!(seat.unit, Some(wire_id(hero)), "{team:?} is told its hero");
        assert!(seat.respawn_left > 0);
        assert!(view.units.iter().all(|unit| unit.id != wire_id(hero)));
        assert_eq!(
            seat.kit.is_some(),
            team == Team::Radiant,
            "its kit, own side alone"
        );
    }
    assert!(!world.can_see(Team::Dire, hero) && world.pos(hero).is_none());
    let cooldown = |world: &World| world.abilities.get(hero).expect("a book").slots[1].cooldown;
    let before = cooldown(&world);
    run(&mut world, 30);
    assert_eq!(
        cooldown(&world),
        before - 30,
        "its waits run while it lies dead"
    );
    assert_eq!(world.health.get(hero).map(|h| h.hp), Some(Fixed::ZERO));
    assert_eq!(
        world.mana.get(hero).map(|m| m.mana),
        Some(Fixed::ZERO),
        "it mends nothing"
    );
    let back = back_on_its_feet(&mut world);
    assert_eq!(back, hero);
    let full = world.stats.get(hero).expect("settled");
    assert_eq!(
        world.mana.get(hero).map(|m| m.mana),
        Some(full.max_mana),
        "it stands up full"
    );
}

fn a_level_seven_hero() -> (World, Entity) {
    let (mut world, hero) = match_hero();
    world.level.insert(hero, Level(7));
    world.seats[0].level = 7;
    assert!(world.learn(hero, 1));
    (world, hero)
}

/// A hero risen at its fountain is invulnerable and untargetable "until the
/// respawning hero is issued a command", moving, attacking, casting and shop
/// dealings alike (Liquipedia, *Hero (unit type)*, Respawning; the fountain's
/// `modifier_fountain_invulnerability`); the Glyph is the side's, no command
/// to the hero. It also ends once the fountain's Rejuvenation, which provides
/// it, no longer holds on the hero.
#[test]
fn a_risen_hero_is_untouchable_at_its_fountain_until_it_is_given_a_command() {
    let risen = || {
        let (mut world, hero) = match_hero();
        world.settle();
        world.step();
        assert!(
            !carries(&world, hero, ModifierKind::FountainInvulnerable),
            "a hero standing at the start does not wear it"
        );
        world.bury(vec![(hero, None)], &mut Vec::new());
        world.seats[0].respawn_left = 1;
        world.step();
        let back = world.seats[0].unit.expect("risen");
        run(&mut world, 30);
        assert!(
            carries(&world, back, ModifierKind::FountainInvulnerable),
            "risen and waiting at its fountain"
        );
        assert!(world.stats.get(back).expect("stats").invulnerable);
        assert!(
            shown(&world, Some(Team::Radiant), back).statuses.bits & StatusFlags::INVULNERABLE != 0,
            "shown untouchable"
        );
        seat_order(&mut world, Order::Glyph);
        assert!(
            carries(&world, back, ModifierKind::FountainInvulnerable),
            "the Glyph is no command to the hero"
        );
        hand_item(&mut world, back, ItemId::IRON_BRANCH, 0);
        (world, back)
    };
    // Shop dealings are named; the swap and the learning are unit orders as
    // any other ("any regular unit orders").
    let walk = Order::Move {
        target: Target::Pos(Vec2::from_ints(0, 0)),
    };
    for (order, queue) in [
        (walk, false),
        (walk, true),
        (
            Order::Buy {
                item: ItemId::TANGO,
            },
            false,
        ),
        (Order::Sell { slot: ItemSlot(0) }, false),
        (
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(1),
            },
            false,
        ),
        (
            Order::Learn {
                slot: AbilitySlot(0),
            },
            false,
        ),
    ] {
        let (mut world, back) = risen();
        world.advance(&[crate::game::Command {
            slot: SlotId(0),
            unit: None,
            order,
            queue,
            seq: 0,
        }]);
        assert!(
            !carries(&world, back, ModifierKind::FountainInvulnerable),
            "{order:?} (queued: {queue}) ends it"
        );
        assert!(!world.stats.get(back).expect("stats").invulnerable);
    }
    // Carried out of its fountain's reach, it stays untouchable for as long
    // as the Rejuvenation lingers on it, and not a tick longer.
    let (mut world, back) = risen();
    world.transform.get_mut(back).expect("standing").pos = hero_spawn_pos(world.map, Team::Dire);
    let mut lingered = 0;
    while carries(&world, back, FOUNTAIN_EFFECT) {
        assert!(
            carries(&world, back, ModifierKind::FountainInvulnerable),
            "untouchable while the Rejuvenation lingers"
        );
        world.step();
        lingered += 1;
        assert!(lingered <= rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    }
    assert!(
        lingered + 1 >= rules::FOUNTAIN_HEAL_LINGER_TICKS,
        "{lingered}"
    );
    assert!(
        !carries(&world, back, ModifierKind::FountainInvulnerable),
        "out of its fountain's reach once the Rejuvenation lifts"
    );
    assert!(!world.stats.get(back).expect("stats").invulnerable);
}

/// "Players with the Fountain Invulnerablility buff will no longer
/// auto-attack and break the buff when an enemy is within range" (Dota,
/// Spring Cleaning 2022): a risen hero left to itself takes nobody on while
/// it wears the buff, and does again once the buff is gone.
#[test]
fn a_risen_hero_attacks_nobody_of_its_own_accord_while_untouchable() {
    let (mut world, hero) = match_hero();
    world.settle();
    world.bury(vec![(hero, None)], &mut Vec::new());
    world.seats[0].respawn_left = 1;
    world.step();
    let back = world.seats[0].unit.expect("risen");
    // Out of every tower's and fountain's reach, an enemy beside it, while
    // the Rejuvenation lingers.
    let at = an_empty_spot(&world);
    world.transform.get_mut(back).expect("standing").pos = at;
    let enemy = world.spawn_unit(&MELEE_CREEP, Team::Dire, at + Vec2::from_ints(150, 0));
    world.settle();
    while carries(&world, back, ModifierKind::FountainInvulnerable) {
        assert_eq!(
            world.orders.get(back).map(|o| o.current),
            Some(UnitOrder::Idle)
        );
        assert_eq!(
            world.target_of(back),
            None,
            "no auto-attack while untouchable"
        );
        world.step();
    }
    run(&mut world, 2);
    assert_eq!(
        world.target_of(back),
        Some(enemy),
        "left to itself, it takes the enemy on once the buff is gone"
    );
}

/// A rune's cut of the cooldowns and mana costs is told to its bearer's side and to a view of the
/// whole map, never to the other side, which sees the hero all the same.
#[test]
fn cast_rates_are_told_to_the_own_side_and_the_whole_map_only() {
    let mut world = World::new();
    let bearer = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5_000, 5_000));
    seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5_100, 5_000));
    put_on(&mut world, bearer, ModifierKind::RuneArcane, 600);
    world.settle();
    world.step();
    let rates = |view: bota_proto::WorldView| {
        view.units
            .iter()
            .find(|unit| unit.id == wire_id(bearer))
            .map(|unit| unit.cast_rates)
    };
    assert_eq!(
        rates(world.view(Team::Radiant)),
        Some(Some((7_500, 7_000))),
        "its own side"
    );
    assert_eq!(
        rates(world.view_full()),
        Some(Some((7_500, 7_000))),
        "the whole map"
    );
    assert_eq!(
        rates(world.view(Team::Dire)),
        Some(None),
        "the other side sees the hero, not the rates"
    );
}

/// Seats 0 and 1 on Radiant and seat 2 on Dire, all in reach of one another, and the hero of seat
/// 1 having struck the Dire hero with real swings until it was hurt.
fn a_hero_the_second_seat_has_hurt() -> (World, Entity, Entity, Entity) {
    let mut world = World::new();
    let killer = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5_000, 5_000));
    let helper = seat_hero(&mut world, 1, Team::Radiant, Vec2::from_ints(5_040, 5_000));
    let prey = seat_hero(&mut world, 2, Team::Dire, Vec2::from_ints(5_080, 5_000));
    world.settle();
    let whole = world.health.get(prey).expect("alive").hp;
    world.set_target(helper, prey);
    for _ in 0..90 {
        world.step();
        if world.health.get(prey).is_some_and(|h| h.hp < whole) {
            break;
        }
    }
    assert!(
        world.health.get(prey).expect("alive").hp < whole,
        "it was hurt"
    );
    (world, killer, helper, prey)
}

fn the_assists(events: &[crate::game::Event]) -> Option<u16> {
    events.iter().find_map(|event| match event.kind() {
        EventKind::Died { assists, .. } => Some(assists),
        _ => None,
    })
}

/// Dota's wiki: damage to a hero credits its dealer an assist when an ally kills it soon after.
#[test]
fn an_ally_that_hurt_a_hero_a_moment_before_it_fell_is_told_as_an_assist() {
    let (mut world, killer, _, prey) = a_hero_the_second_seat_has_hurt();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(killer))], &mut events);
    assert_eq!(
        the_assists(&events),
        Some(0b10),
        "seat 1 assisted, seat 0 killed"
    );
    assert_eq!(world.seats[1].assists, 1, "and the scoreboard counts it");
    assert_eq!(world.seats[0].assists, 0, "the killer is no assist");
}

#[test]
fn an_ally_that_hurt_a_hero_long_before_it_fell_is_no_assist() {
    let (mut world, killer, _, prey) = a_hero_the_second_seat_has_hurt();
    world.tick += crate::game::KILL_CREDIT_WINDOW_TICKS + 1;
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(killer))], &mut events);
    assert_eq!(the_assists(&events), Some(0));
    assert_eq!(world.seats[1].assists, 0);
}

/// Liquipedia credits a creep finish after one hero damager to that hero.
/// No self-assist follows from that credit; it is not a separate quoted rule.
/// https://liquipedia.net/dota2/Gold#Hero_kills
#[test]
fn a_hero_a_creep_finishes_credits_its_sole_damager_without_a_self_assist() {
    let (mut world, _, damager, prey) = a_hero_the_second_seat_has_hurt();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(1_000, 1_000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(creep))], &mut events);
    assert_eq!(the_assists(&events), Some(0));
    assert_eq!(world.seats[1].kills, 1);
    assert_eq!(world.seats[1].assists, 0);
    for viewer in [None, Some(Team::Radiant), Some(Team::Dire)] {
        let projected = crate::game_loop::visible_events(&events, viewer);
        let (physical, credited, assists) = projected
            .iter()
            .find_map(|event| match event {
                EventKind::Died {
                    killer,
                    credited_hero,
                    assists,
                    ..
                } => Some((*killer, credited_hero.as_deref().copied(), *assists)),
                _ => None,
            })
            .expect("public hero death");
        assert_eq!(
            physical,
            if viewer == Some(Team::Dire) {
                None
            } else {
                Some(wire_id(creep))
            }
        );
        assert_eq!(
            credited,
            Some(wire_id(damager)),
            "the kill feed announces the credited hero even with a hidden physical finisher"
        );
        assert_eq!(assists, 0, "the credited killer never assists its own kill");
    }
}

/// The page's no-recent-damage branch states a 20 s window. The credit code
/// applies that same window to sole-damager attribution.
/// https://liquipedia.net/dota2/Gold#Hero_kills
#[test]
fn a_creep_finish_after_the_sole_damagers_window_has_no_credited_hero() {
    let (mut world, _, _, prey) = a_hero_the_second_seat_has_hurt();
    world.tick += crate::game::KILL_CREDIT_WINDOW_TICKS + 1;
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5_100, 5_000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(creep))], &mut events);
    let (physical, credited, assists) = events
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Died {
                killer,
                credited_hero,
                assists,
                ..
            } => Some((killer, credited_hero, assists)),
            _ => None,
        })
        .expect("the expired-damager creep finish emits a death");
    assert_eq!(
        physical,
        Some(wire_id(creep)),
        "the physical finisher remains the creep"
    );
    assert_eq!(
        credited, None,
        "expired damage cannot credit the former sole damager"
    );
    assert_eq!(
        assists, 0,
        "expired damage does not count as an assist either"
    );
    for seat in &world.seats[..2] {
        assert_eq!(seat.kills, 0);
        assert_eq!(seat.assists, 0);
    }
}

/// Both heroes struck the victim: a creep finish leaves no credited hero,
/// so both remain assistants [Liquipedia Gold, Hero kills].
fn a_hero_both_radiant_seats_have_hurt() -> (World, Entity, Entity, Entity) {
    let (mut world, first, second, prey) = a_hero_the_second_seat_has_hurt();
    world.set_target(first, prey);
    for _ in 0..90 {
        world.step();
        if world.seats[2]
            .hurt_by
            .iter()
            .any(|entry| entry.by == 0 && entry.damage_at.is_some())
        {
            break;
        }
    }
    assert!(
        world.seats[2]
            .hurt_by
            .iter()
            .any(|entry| entry.by == 0 && entry.damage_at.is_some()),
        "the first hero actually damaged the victim too"
    );
    assert!(world.alive(prey));
    world.set_order(first, UnitOrder::Stand);
    world.set_order(second, UnitOrder::Stand);
    (world, first, second, prey)
}

/// https://liquipedia.net/dota2/Gold#Hero_kills distinguishes one hero
/// damager (a credited kill) from several (a shared creep kill with assists).
#[test]
fn a_hero_a_creep_finishes_is_assisted_by_those_who_hurt_it() {
    let (mut world, _, _, prey) = a_hero_both_radiant_seats_have_hurt();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5_100, 5_000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(creep))], &mut events);
    assert_eq!(
        the_assists(&events),
        Some(0b11),
        "both genuine damagers assist the shared kill"
    );
    for seat in &world.seats[..2] {
        assert_eq!(seat.kills, 0);
        assert_eq!(seat.assists, 1);
    }
    let (physical, credited) = events
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Died {
                killer,
                credited_hero,
                ..
            } => Some((killer, credited_hero)),
            _ => None,
        })
        .expect("the shared creep finish emits a death");
    assert_eq!(
        physical,
        Some(wire_id(creep)),
        "the creep dealt the killing blow"
    );
    assert_eq!(
        credited, None,
        "two hero damagers leave no sole credited hero"
    );
}

#[test]
fn what_hurt_a_hero_in_its_last_life_credits_nothing_in_the_next() {
    let (mut world, killer, _, prey) = a_hero_the_second_seat_has_hurt();
    world.bury(vec![(prey, Some(killer))], &mut Vec::new());
    assert!(world.seats[2].hurt_by.is_empty());
}

/// The fall of a building tells what it was, for a feed to word as Dota does.
#[test]
fn a_fallen_tower_tells_its_kind_lane_and_tier() {
    let cfg = config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let (tower, lane, tier) = world
        .entities
        .iter()
        .find_map(|entity| {
            if world.kind.get(entity) != Some(&bota_proto::UnitKind::Tower) {
                return None;
            }
            Some((entity, world.lane.get(entity)?.0, world.tier.get(entity)?.0))
        })
        .expect("a tower with a lane and a tier");
    let mut events = Vec::new();
    world.bury(vec![(tower, None)], &mut events);
    let told = events.iter().find_map(|event| match event.kind() {
        EventKind::StructureDestroyed {
            kind,
            lane,
            tier,
            ranged,
            ..
        } => Some((kind, lane, tier, ranged)),
        _ => None,
    });
    assert_eq!(
        told,
        Some((bota_proto::UnitKind::Tower, Some(lane), Some(tier), None))
    );
}

/// A side not told who struck the blow is not told who helped: the helpers would
/// name the unseen killer's side's heroes. This shared creep kill has genuine
/// assistants; the sole-damager case above checks public credited-hero disclosure.
#[test]
fn a_side_that_cannot_see_the_killer_is_told_no_assists() {
    let (mut world, _, _, prey) = a_hero_both_radiant_seats_have_hurt();
    // A Radiant creep out of every Dire eye finishes the hero.
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(1_000, 1_000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(creep))], &mut events);
    let told = |team| {
        crate::game_loop::visible_events(&events, Some(team))
            .into_iter()
            .find_map(|event| match event {
                EventKind::Died {
                    killer, assists, ..
                } => Some((killer.is_some(), assists)),
                _ => None,
            })
    };
    assert_eq!(
        told(Team::Radiant),
        Some((true, 0b11)),
        "its own side knows both"
    );
    assert_eq!(
        told(Team::Dire),
        Some((false, 0)),
        "the fallen's side knows neither"
    );
}
