//! What a lane creep's fall pays: a fresh draw from its gold range every
//! kill, the range raised by the upgrades it spawned after, and a
//! flagbearer's bonus to the heroes around it.

use bota_core::{FLAGBEARER_CREEP, GoldRange, MELEE_CREEP, RANGED_CREEP, UnitDef, rules};

use crate::game::Entity;
use crate::game::World;
use crate::game_loop::visible_events;
use bota_proto::{EventKind, Team, Vec2};

use super::support::*;

/// Kills one creep of a kind after so many upgrades with a seated Radiant
/// hero standing beside it, and answers the gold and experience it paid.
fn one_kill(world: &mut World, def: &'static UnitDef, upgrades: u32) -> (i32, i32) {
    let killer = world.seats[0].unit.expect("seated");
    let (gold, xp) = (world.seats[0].gold, world.seats[0].xp);
    let prey = world.spawn_creep(def, Team::Dire, Vec2::from_ints(5050, 5000), 0, upgrades);
    world.settle();
    world.health.insert(prey, health(0));
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(killer))], &mut events);
    let gained = world.seats[0].gold - gold;
    assert_eq!(paid(&events), Some(gained), "the event tells what was paid");
    (gained, world.seats[0].xp - xp)
}

/// The lowest, highest and mean gold of many kills, the mean in hundredths.
fn spread(def: &'static UnitDef, upgrades: u32, kills: i32) -> (i32, i32, i32, i32) {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5000, 5000));
    let (mut low, mut high, mut sum, mut xp) = (i32::MAX, i32::MIN, 0, 0);
    for _ in 0..kills {
        let (gold, gained_xp) = one_kill(&mut world, def, upgrades);
        low = low.min(gold);
        high = high.max(gold);
        sum += gold;
        xp = gained_xp;
    }
    (low, high, sum * 100 / kills, xp)
}

#[test]
fn creep_gold_is_drawn_evenly_from_its_range_every_kill() {
    // 7.41: melee 34-39 (mean 36.5), ranged 43-52 (mean 47.5); two upgrade
    // intervals move both ends, melee by 1 and ranged by 3 each, and add
    // 8 experience each to a ranged creep alone.
    let cases: [(&UnitDef, u32, GoldRange, i32); 4] = [
        (&MELEE_CREEP, 0, GoldRange::new(34, 39), 57),
        (&RANGED_CREEP, 0, GoldRange::new(43, 52), 69),
        (&MELEE_CREEP, 2, GoldRange::new(36, 41), 57),
        (&RANGED_CREEP, 2, GoldRange::new(49, 58), 85),
    ];
    for (def, upgrades, range, xp) in cases {
        let (low, high, mean, gained_xp) = spread(def, upgrades, 20_000);
        let label = format!("{:?} after {upgrades} upgrades", def.kind);
        assert_eq!((low, high), (range.min, range.max), "{label}: every end");
        // Over 20 000 draws the mean wanders a hundredth or two; a range
        // paid at a rounded midpoint or drawn with a bias is off by tens.
        let expected = (range.min + range.max) * 50;
        let leeway = 3 * (range.max - range.min) / 2;
        assert!(
            (mean - expected).abs() <= leeway,
            "{label}: mean {mean} against {expected} hundredths"
        );
        assert_eq!(gained_xp, xp, "{label}: experience");
    }
}

#[test]
fn a_fallen_flagbearer_pays_its_bonus_to_the_heroes_around_it_and_the_killer() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let killer = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(3000, 5000));
    seat_hero(&mut world, 1, Team::Radiant, Vec2::from_ints(6400, 5000));
    seat_hero(&mut world, 2, Team::Radiant, Vec2::from_ints(6600, 5000));
    seat_hero(&mut world, 3, Team::Dire, Vec2::from_ints(5100, 5000));
    let flag = world.spawn_creep(&FLAGBEARER_CREEP, Team::Dire, at, 0, 0);
    world.settle();
    let bonus = world.bounty.get(flag).expect("it pays").bonus;
    let range = rules::MELEE_CREEP_BOUNTY;
    assert!(
        (rules::FLAGBEARER_BONUS_GOLD + range.min..=rules::FLAGBEARER_BONUS_GOLD + range.max)
            .contains(&bonus),
        "ten and one draw of its own range: {bonus}"
    );
    world.health.insert(flag, health(0));
    let mut events = Vec::new();
    world.bury(vec![(flag, Some(killer))], &mut events);
    let gold: Vec<i32> = world.seats.iter().map(|seat| seat.gold).collect();
    let kill = gold[0] - bonus;
    assert!(
        (range.min..=range.max).contains(&kill),
        "the killer, 2000 away, gets its own draw and the bonus: {gold:?}"
    );
    assert_eq!(paid(&events), Some(gold[0]), "the event tells both");
    assert_eq!(gold[1], bonus, "an ally within 1500 gets the bonus");
    assert_eq!(gold[2], 0, "one 1600 away gets nothing");
    assert_eq!(gold[3], 0, "nor does its own side");
    assert_eq!(world.seats[0].xp, 0, "experience stays with who stood near");
}

#[test]
fn a_denied_flagbearer_pays_nobody() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5200, 5000));
    let denier = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5100, 5000));
    let flag = world.spawn_creep(&FLAGBEARER_CREEP, Team::Dire, at, 0, 0);
    world.settle();
    world.health.insert(flag, health(0));
    let mut events = Vec::new();
    world.bury(vec![(flag, Some(denier))], &mut events);
    assert_eq!(paid(&events), Some(0));
    assert_eq!([world.seats[0].gold, world.seats[1].gold], [0, 0]);
    assert_eq!(
        world.seats[0].xp,
        rules::FLAGBEARER_XP * rules::DENIED_XP_PCT / 100,
        "the other side still learns half"
    );
}

/// Buries `prey` at zero health under `killer` and answers the gold each
/// audience is told: the earner's side, the other side, a full-world stream.
fn told_gold(world: &mut World, prey: Entity, killer: Entity) -> (i32, [Option<Option<i32>>; 3]) {
    world.settle();
    world.health.insert(prey, health(0));
    let mut events = Vec::new();
    world.bury(vec![(prey, Some(killer))], &mut events);
    let earned = paid(&events).expect("a death is told");
    assert!(earned > 0);
    let told = |side| {
        visible_events(&events, side)
            .into_iter()
            .find_map(|kind| match kind {
                EventKind::Died { bounty, .. } => Some(bounty),
                _ => None,
            })
    };
    (
        earned,
        [Some(Team::Radiant), Some(Team::Dire), None].map(told),
    )
}

#[test]
fn a_creep_kills_gold_is_told_only_to_the_side_that_earned_it() {
    let mut world = World::new();
    let killer = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5000, 5000));
    seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5100, 5000));
    let prey = world.spawn_creep(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5050, 5000), 0, 0);
    let (earned, told) = told_gold(&mut world, prey, killer);
    assert_eq!(
        told,
        [Some(Some(earned)), Some(None), Some(Some(earned))],
        "told to the side that earned it and the whole map, withheld from the other"
    );
}

#[test]
fn a_hero_kills_gold_is_public() {
    let mut world = World::new();
    let killer = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5000, 5000));
    let prey = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(5100, 5000));
    let (earned, told) = told_gold(&mut world, prey, killer);
    assert_eq!(told, [Some(Some(earned)); 3]);
}
