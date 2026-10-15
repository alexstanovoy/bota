//! Pools, regeneration, and the stats worked out every tick.

use bota_core::{
    FLAGBEARER_CREEP, HERO, ITEMS, MEGA_MELEE_CREEP, MEGA_RANGED_CREEP, MELEE_CREEP, ModifierKind,
    RANGED_CREEP, SUPER_MELEE_CREEP, SUPER_RANGED_CREEP, UnitDef, attack_gain, map_of, rules,
};

use crate::game::{Def, Health, Level, Mana, Modifiers, Stats, UnitOrder, Upgrades, World};
use bota_proto::{DamageKind, Fixed, ItemId, MapId, Team, UnitKind, Vec2};

use super::support::*;

/// A pool a tick regenerates.
#[derive(Clone, Copy, Debug)]
enum Pool {
    Health,
    Mana,
}

/// Regeneration cases: the pool, its rate a tick (none for a body with no
/// stats), where it starts, the ticks run, and where it stands after.
const REGENERATION: [(Pool, Option<Fixed>, i32, u32, i32); 8] = [
    (Pool::Health, Some(Fixed::from_int(3)), 10, 1, 13),
    (Pool::Health, Some(Fixed::from_int(3)), 10, 11, 20),
    (Pool::Health, Some(Fixed::from_ratio(1, 4)), 10, 3, 10),
    (Pool::Health, Some(Fixed::from_ratio(1, 4)), 10, 4, 11),
    (Pool::Health, Some(Fixed::from_ratio(1, 4)), 10, 8, 12),
    (Pool::Mana, Some(Fixed::from_ratio(1, 2)), 0, 2, 1),
    (Pool::Health, Some(Fixed::from_int(3)), 0, 1, 0),
    (Pool::Health, None, 10, 1, 10),
];

#[test]
fn a_pool_regenerates_in_whole_points_up_to_its_maximum_and_not_once_empty() {
    for (pool, rate, start, ticks, expected) in REGENERATION {
        let mut world = World::new();
        let body = world.spawn();
        match pool {
            Pool::Health => {
                world.health.insert(body, health(start));
            }
            Pool::Mana => {
                world.mana.insert(
                    body,
                    Mana {
                        mana: Fixed::from_int(start),
                    },
                );
            }
        }
        if let Some(rate) = rate {
            let (hp_regen, mana_regen) = match pool {
                Pool::Health => (rate, Fixed::ZERO),
                Pool::Mana => (Fixed::ZERO, rate),
            };
            world.stats.insert(
                body,
                Stats {
                    hp_regen,
                    mana_regen,
                    ..stats()
                },
            );
        }
        for _ in 0..ticks {
            world.step();
        }
        let left = match pool {
            Pool::Health => world.health.get(body).map(|h| h.hp.to_int()),
            Pool::Mana => world.mana.get(body).map(|m| m.mana.to_int()),
        };
        assert_eq!(
            left,
            Some(expected),
            "{pool:?} from {start} at {rate:?} a tick for {ticks} ticks"
        );
    }
}

#[test]
fn a_pool_stands_up_full_and_is_only_capped_after() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    let full = Fixed::from_int(rules::MELEE_CREEP_HP);
    world.step();
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(full),
        "with no stats behind it, it has just been stood up"
    );
    world.health.insert(creep, health(100));
    world.step();
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(Fixed::from_int(100) + rules::MELEE_CREEP_HP_REGEN),
        "what it has left is left alone but for its own mending"
    );
    world.health.insert(
        creep,
        Health {
            hp: full + Fixed::from_int(400),
        },
    );
    world.step();
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(full),
        "and never more than the maximum"
    );
}

#[test]
fn a_wave_coming_out_does_not_mend_what_already_stands() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let hurt = world
        .entities
        .iter()
        .find(|e| world.kind.get(*e) == Some(&UnitKind::Tower))
        .expect("the map has towers");
    world.health.insert(hurt, health(100));
    while world.tick <= rules::FIRST_WAVE_TICK {
        world.step();
    }
    assert!(
        world.entities.iter().any(|e| world.march.get(e).is_some()),
        "a wave came out"
    );
    assert_eq!(
        world.health.get(hurt).map(|h| h.hp),
        Some(Fixed::from_int(100)),
        "the tower is no better off for it"
    );
}

#[test]
fn upgrades_raise_a_creep_and_carry_its_health_with_them() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    world.health.insert(creep, health(rules::MELEE_CREEP_HP));
    world.step();
    let full = world.health.get(creep).expect("alive").hp;
    world.upgrades.insert(creep, Upgrades(3));
    world.step();
    let stats = world.stats.get(creep).expect("worked out this tick");
    assert_eq!(
        stats.max_hp,
        Fixed::from_int(rules::MELEE_CREEP_HP + 3 * rules::MELEE_UPGRADE_HP)
    );
    assert_eq!(stats.damage, rules::MELEE_CREEP_ATTACK_DAMAGE + 3);
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(full + Fixed::from_int(3 * rules::MELEE_UPGRADE_HP)),
        "the health gained is the maximum gained"
    );
}

#[test]
fn a_flag_carrier_takes_the_melee_upgrades() {
    let mut world = World::new();
    let ranged = world.spawn();
    world.def.insert(ranged, Def(&RANGED_CREEP));
    world.upgrades.insert(ranged, Upgrades(3));
    let flag = world.spawn();
    world.def.insert(flag, Def(&FLAGBEARER_CREEP));
    world.upgrades.insert(flag, Upgrades(3));
    world.step();
    assert_eq!(
        world.stats.get(ranged).map(|s| s.max_hp),
        Some(Fixed::from_int(
            rules::RANGED_CREEP_HP + 3 * rules::RANGED_UPGRADE_HP
        ))
    );
    assert_eq!(
        world.stats.get(flag).map(|s| s.max_hp),
        Some(Fixed::from_int(
            rules::MELEE_CREEP_HP + 3 * rules::MELEE_UPGRADE_HP
        )),
        "a flag carrier grows as a melee creep does"
    );
}

#[test]
fn levels_raise_a_hero() {
    let mut world = World::new();
    let hero = world.spawn();
    world.def.insert(hero, Def(&HERO));
    world.level.insert(hero, Level(1));
    world.mana.insert(hero, Mana { mana: Fixed::ZERO });
    world.step();
    let first = world.stats.get(hero).expect("worked out this tick").max_hp;
    assert_eq!(first, body_at(0), "level one is plain");
    world.level.insert(hero, Level(4));
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.max_hp),
        Some(body_at(3)),
        "three levels past the first"
    );
}

#[test]
fn haste_shortens_the_wait_between_attacks_while_it_lasts() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    put_on(&mut world, creep, ModifierKind::Haste { speed: 40 }, 5);
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.attack_speed),
        Some(rules::BASE_ATTACK_SPEED + 40)
    );
    world.modifiers.insert(creep, Modifiers(Vec::new()));
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.attack_speed),
        Some(rules::BASE_ATTACK_SPEED),
        "what is worked out afresh forgets what has lifted"
    );
}

#[test]
fn mending_adds_to_what_a_kind_regenerates() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    let mending = ModifierKind::Mending {
        per_tick: 25,
        breaks: false,
    };
    put_on(&mut world, creep, mending, 5);
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.hp_regen),
        Some(rules::MELEE_CREEP_HP_REGEN + Fixed::from_ratio(25, 100)),
        "on top of the half point a second a melee creep mends"
    );
}

#[test]
fn what_a_hero_carries_shows_up_in_its_stats() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.step();
    let bare = world.stats.get(hero).expect("settled").damage;
    // The first item in the table that carries damage rather than charges.
    let (id, def) = ITEMS
        .iter()
        .enumerate()
        .find(|(_, d)| d.carried.damage > 0)
        .expect("some item adds damage");
    hand_item(&mut world, hero, ItemId(id as u16), def.charges);
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.damage),
        Some(bare + def.carried.damage),
        "the damage it carries is added"
    );
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = None;
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.damage),
        Some(bare),
        "and dropping it takes the damage away, with nothing to unapply"
    );
}

#[test]
fn a_held_unit_neither_walks_nor_swings_nor_casts() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, spot + Vec2::from_ints(100, 0));
    world.settle();
    world.step();
    let full = world.health.get(creep).expect("standing").hp;
    put_on(&mut world, hero, ModifierKind::Stunned, 60);
    let stood = world.transform.get(hero).expect("standing").pos;
    world.set_order(
        hero,
        UnitOrder::Move {
            pos: spot + Vec2::from_ints(1000, 0),
        },
    );
    for _ in 0..40 {
        world.step();
    }
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(stood),
        "held, it does not walk however it is ordered"
    );
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(full),
        "and nothing it stood beside was struck"
    );
    // Once it lifts, the order it was given all along is carried out.
    for _ in 0..30 {
        world.step();
    }
    assert!(
        world.transform.get(hero).map(|t| t.pos) != Some(stood),
        "and once it lifts the hero goes where it was told"
    );
}

#[test]
fn a_slow_takes_its_share_of_the_speed() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    world.step();
    let full = world.stats.get(creep).expect("settled").move_speed;
    put_on(&mut world, creep, ModifierKind::Slowed { pct: 25 }, 60);
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.move_speed),
        Some(Fixed {
            raw: full.raw * 75 / 100
        }),
        "three quarters of what it had"
    );
}

#[test]
fn a_burn_takes_health_on_the_beat_and_may_be_told_to_leave_one() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    world.step();
    let full = world.health.get(creep).expect("standing").hp.to_int();
    put_on(
        &mut world,
        creep,
        ModifierKind::Burning {
            amount: 5,
            kind: DamageKind::Pure,
            lethal: true,
        },
        rules::BURN_PERIOD_TICKS * 4,
    );
    for _ in 0..rules::BURN_PERIOD_TICKS * 4 {
        world.step();
    }
    let left = world.health.get(creep).expect("standing").hp.to_int();
    assert!(
        left <= full - 5 && left >= full - 25,
        "it burns on the beat, not every tick: {full} then {left}"
    );
    // One that may not take the last point stops one short of it.
    world.health.insert(creep, health(3));
    put_on(
        &mut world,
        creep,
        ModifierKind::Burning {
            amount: 100,
            kind: DamageKind::Pure,
            lethal: false,
        },
        rules::BURN_PERIOD_TICKS * 10,
    );
    for _ in 0..rules::BURN_PERIOD_TICKS * 10 {
        world.step();
    }
    assert!(world.alive(creep), "it is still standing");
    assert_eq!(
        world.health.get(creep).map(|h| h.hp.to_int()),
        Some(1),
        "on its last point"
    );
}

#[test]
fn what_a_courier_carries_is_worth_nothing_to_the_courier() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    world.step();
    let plain = world.stats.get(courier).expect("settled").move_speed;
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    world.step();
    assert_eq!(
        world.stats.get(courier).map(|stats| stats.move_speed),
        Some(plain),
        "it carries the boots, it does not wear them"
    );
}

#[test]
fn every_attribute_pays_for_what_it_is_worth() {
    let (mut world, hero) = a_hero_with_gold(0);
    let before = *world.stats.get(hero).expect("settled");
    let six = Fixed::from_int(6);
    hand_item(&mut world, hero, ItemId::BELT, 0);
    world.step();
    let with_belt = *world.stats.get(hero).expect("settled");
    assert_eq!(
        with_belt.attributes.strength - before.attributes.strength,
        six,
        "the belt is worth six points of strength"
    );
    assert_eq!(
        with_belt.max_hp - before.max_hp,
        Fixed::from_int(rules::HP_PER_STRENGTH) * six,
        "and strength is worth health"
    );
    assert_eq!(
        with_belt.hp_regen - before.hp_regen,
        rules::HP_REGEN_PER_STRENGTH * six,
        "and mending"
    );
    hand_item(&mut world, hero, ItemId::ROBE, 0);
    world.step();
    let with_robe = *world.stats.get(hero).expect("settled");
    assert_eq!(
        with_robe.max_mana - before.max_mana,
        Fixed::from_int(rules::MANA_PER_INTELLIGENCE) * six,
        "intelligence is worth mana"
    );
    hand_item(&mut world, hero, ItemId::BAND, 0);
    world.step();
    let with_band = *world.stats.get(hero).expect("settled");
    assert_eq!(
        with_band.armor - before.armor,
        rules::ARMOR_PER_AGILITY * six,
        "agility is worth armor"
    );
    assert_eq!(
        with_band.damage - before.damage,
        6 * rules::DAMAGE_PER_PRIMARY,
        "and it is what this one pays its damage with"
    );
}

#[test]
fn attack_speed_shortens_the_wait_between_attacks() {
    let (mut world, hero) = a_hero_with_gold(0);
    let plain = *world.stats.get(hero).expect("settled");
    hand_item(&mut world, hero, ItemId::GLOVES, 0);
    world.step();
    let hasted = *world.stats.get(hero).expect("settled");
    assert_eq!(
        hasted.attack_speed,
        plain.attack_speed + 20,
        "the gloves are worth twenty"
    );
    assert_eq!(
        hasted.attack_time, plain.attack_time,
        "the cycle itself is the kind's own"
    );
    assert!(
        attack_gain(hasted.attack_speed) > attack_gain(plain.attack_speed),
        "and it runs faster for the speed"
    );
}

#[test]
fn shadow_fiend_stands_at_dota_numbers_from_the_first_level_to_the_last() {
    // Dota 7.41f with nothing bought: level, health, mana, attack damage
    // (the middle of the spread), milliseconds between swings, and armor in
    // thousandths. Health and mana show their whole points.
    for (level, hp, mana, damage, swing, armor) in [
        (1, 538, 267, 44, 1280, 4_166),
        (15, 1369, 636, 94, 914, 12_565),
        (30, 2260, 1032, 148, 698, 21_565),
    ] {
        let mut world = World::new();
        let fiend = seated_hero(
            &mut world,
            Vec2::from_ints(5000, 5000),
            bota_proto::HeroId(1),
        );
        world.level.insert(fiend, Level(level));
        world.settle();
        let shown = world
            .view_full()
            .units
            .into_iter()
            .find(|unit| unit.hero == Some(bota_proto::HeroId(1)))
            .expect("he is projected");
        assert_eq!(shown.max_hp, hp, "level {level} health");
        assert_eq!(shown.max_mana, mana, "level {level} mana");
        assert_eq!(shown.attack_damage, damage, "level {level} damage");
        assert_eq!(shown.attack_time, swing, "level {level} swing");
        assert_eq!(
            (shown.armor * Fixed::from_int(1000)).to_int(),
            armor,
            "level {level} armor"
        );
        assert_eq!(shown.magic_resist, Fixed::from_ratio(1, 4));
        assert_eq!(shown.move_speed, Fixed::from_int(305));
        assert_eq!(shown.attack_range, Fixed::from_int(525));
    }
}

/// A creep kind, its health, its damage range and its gold range.
type Row = (&'static UnitDef, i32, (i32, i32), (i32, i32));

#[test]
fn super_and_mega_creeps_match_the_table_at_fifteen_minutes() {
    // Liquipedia, Lane Creeps, the upgrade table's 15:00 row (two upgrades):
    // health, damage range, gold range. Mega creeps grow only their bounty;
    // a super or mega melee creep's 1.5 gold a step is paid in whole gold.
    let rows: [Row; 4] = [
        (&SUPER_MELEE_CREEP, 738, (40, 48), (23, 29)),
        (&MEGA_MELEE_CREEP, 1270, (96, 104), (23, 29)),
        (&SUPER_RANGED_CREEP, 511, (47, 52), (31, 37)),
        (&MEGA_RANGED_CREEP, 1015, (131, 136), (31, 37)),
    ];
    for (def, hp, damage, gold) in rows {
        let mut world = World::new();
        let creep = world.spawn_creep(def, Team::Dire, Vec2::from_ints(5000, 5000), 0, 2);
        world.settle();
        let stats = world.stats.get(creep).expect("settled");
        let bounty = world.bounty.get(creep).expect("it pays").gold;
        assert_eq!(
            (
                stats.max_hp,
                (stats.damage, stats.damage + stats.damage_spread),
                (bounty.min, bounty.max)
            ),
            (Fixed::from_int(hp), damage, gold),
            "{:?} of {hp} health",
            def.kind
        );
    }
}
