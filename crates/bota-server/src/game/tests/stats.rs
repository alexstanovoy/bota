//! Pools, regeneration, and the stats worked out every tick.

use crate::game::rules;
use crate::game::{
    AbilityBook, AbilityState, Def, FLAGBEARER_CREEP, HERO, Health, Inventory, ItemStack, Level,
    MELEE_CREEP, Mana, Modifier, ModifierKind, Modifiers, NEUTRALS, NeutralKind, RANGED_CREEP,
    Stats, Upgrades, World,
};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_tick_mends_what_can_mend() {
    let mut world = World::new();
    let hurt = world.spawn();
    world.health.insert(hurt, health(10));
    world.stats.insert(
        hurt,
        Stats {
            hp_regen: Fixed::from_int(3),
            ..stats()
        },
    );
    world.step();
    assert_eq!(world.tick, 1);
    assert_eq!(world.health.get(hurt).map(|h| h.hp.to_int()), Some(13));
    for _ in 0..10 {
        world.step();
    }
    assert_eq!(
        world.health.get(hurt).map(|h| h.hp.to_int()),
        Some(20),
        "mending stops at the maximum"
    );
}

#[test]
fn mending_finer_than_a_point_gathers_until_it_is_worth_one() {
    let mut world = World::new();
    let hurt = world.spawn();
    world.health.insert(hurt, health(10));
    world.stats.insert(
        hurt,
        Stats {
            hp_regen: Fixed::from_ratio(1, 4),
            ..stats()
        },
    );
    for _ in 0..3 {
        world.step();
    }
    assert_eq!(
        world.health.get(hurt).map(|h| h.hp.to_int()),
        Some(10),
        "three quarters of a point is not a point"
    );
    world.step();
    assert_eq!(world.health.get(hurt).map(|h| h.hp.to_int()), Some(11));
    for _ in 0..4 {
        world.step();
    }
    assert_eq!(
        world.health.get(hurt).map(|h| h.hp.to_int()),
        Some(12),
        "nothing is lost between whole points"
    );
}

#[test]
fn mana_mends_the_same_way() {
    let mut world = World::new();
    let caster = world.spawn();
    world.mana.insert(caster, Mana { mana: Fixed::ZERO });
    world.stats.insert(
        caster,
        Stats {
            mana_regen: Fixed::from_ratio(1, 2),
            ..stats()
        },
    );
    for _ in 0..2 {
        world.step();
    }
    assert_eq!(world.mana.get(caster).map(|m| m.mana.to_int()), Some(1));
}

#[test]
fn the_dead_mend_nothing() {
    let mut world = World::new();
    let dead = world.spawn();
    world.health.insert(dead, health(0));
    world.stats.insert(
        dead,
        Stats {
            hp_regen: Fixed::from_int(3),
            ..stats()
        },
    );
    world.step();
    assert_eq!(world.health.get(dead).map(|h| h.hp.to_int()), Some(0));
}

#[test]
fn a_tick_leaves_alone_what_carries_no_stats() {
    let mut world = World::new();
    let stone = world.spawn();
    world.health.insert(stone, health(10));
    world.step();
    assert_eq!(world.health.get(stone).map(|h| h.hp.to_int()), Some(10));
}

#[test]
fn a_tick_leaves_alone_what_is_no_longer_in_the_world() {
    let mut world = World::new();
    let gone = world.spawn();
    world.health.insert(gone, health(10));
    world.stats.insert(
        gone,
        Stats {
            hp_regen: Fixed::from_int(3),
            ..stats()
        },
    );
    assert!(world.despawn(gone));
    world.step();
    // The row is still there, untouched: the tick walks the allocator, and
    // the allocator no longer names it.
    assert_eq!(world.health.get(gone).map(|h| h.hp.to_int()), Some(10));
}

#[test]
fn what_a_despawned_entity_left_is_not_inherited() {
    let mut world = World::new();
    let first = world.spawn();
    world.health.insert(first, health(10));
    world.stats.insert(
        first,
        Stats {
            hp_regen: Fixed::from_int(3),
            ..stats()
        },
    );
    assert!(world.despawn(first));
    let second = world.spawn();
    assert_eq!(first.index(), second.index());
    assert_eq!(world.health.get(second), None);
    world.step();
    assert_eq!(
        world.health.get(second),
        None,
        "it mends nothing it never had"
    );
}

#[test]
fn a_plain_creep_gets_the_numbers_of_its_kind() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    world.step();
    let stats = world.stats.get(creep).expect("worked out this tick");
    assert_eq!(stats.max_hp, Fixed::from_int(rules::MELEE_CREEP_HP));
    assert_eq!(stats.damage, rules::MELEE_CREEP_ATTACK_DAMAGE);
    assert_eq!(stats.armor, Fixed::from_int(rules::MELEE_CREEP_ARMOR));
    assert_eq!(stats.attack_time, rules::CREEP_ATTACK_TIME);
    assert_eq!(stats.projectile_speed, None, "a melee creep throws nothing");
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
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_int(100),
        },
    );
    world.step();
    assert_eq!(
        world.health.get(creep).map(|h| h.hp),
        Some(Fixed::from_int(100)),
        "what it has left is left alone"
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
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let hurt = world
        .entities
        .iter()
        .find(|e| world.kind.get(*e) == Some(&bota_proto::UnitKind::Tower));
    let hurt = hurt.expect("the map has towers");
    world.health.insert(
        hurt,
        Health {
            hp: Fixed::from_int(100),
        },
    );
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
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_int(rules::MELEE_CREEP_HP),
        },
    );
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
fn a_flag_carrier_takes_no_upgrades() {
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
        Some(Fixed::from_int(rules::MELEE_CREEP_HP)),
        "upgrades pass a flag carrier by"
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
    world.modifiers.insert(
        creep,
        Modifiers(vec![Modifier {
            kind: ModifierKind::Haste { speed: 40 },
            source: None,
            ticks_left: Some(5),
        }]),
    );
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
    world.modifiers.insert(
        creep,
        Modifiers(vec![Modifier {
            kind: ModifierKind::Mending {
                per_tick: 25,
                breaks: false,
            },
            source: None,
            ticks_left: Some(5),
        }]),
    );
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.hp_regen),
        Some(Fixed::from_ratio(25, 100)),
        "a creep mends nothing of its own, so this is all of it"
    );
}

#[test]
fn every_neutral_kind_has_numbers_of_its_own() {
    let dragon = NeutralKind::BlackDragon.def();
    assert_eq!(dragon.max_hp, 2000);
    assert_eq!(
        dragon.projectile_speed,
        Some(Fixed::from_int(1500).to_int())
    );
    assert!(dragon.ancient, "a dragon is an ancient creep");
    let kobold = NeutralKind::Kobold.def();
    assert_eq!(kobold.max_hp, 240);
    assert_eq!(kobold.projectile_speed, None, "a kobold swings");
    assert!(!kobold.ancient);
    assert_eq!(kobold.collision, rules::NEUTRAL_COLLISION);
    assert_eq!(kobold.bound, rules::NEUTRAL_BOUND);
    assert_eq!(kobold.vision, 1400, "a kobold sees further than most camps");
    assert_eq!(NeutralKind::GnollAssassin.def().vision, 400);
    assert_eq!(
        NeutralKind::OgreMauler.def().vision,
        rules::NEUTRAL_VISION,
        "a camp with no sight of its own sees the usual distance"
    );
    assert_eq!(kobold.per_upgrade.hp, rules::NEUTRAL_UPGRADE_HP);
    assert_eq!(NEUTRALS.len(), 36);
}

#[test]
fn what_an_entity_carries_and_casts_keeps_its_slots() {
    let mut inventory = Inventory::empty(3);
    assert_eq!(inventory.held().count(), 0);
    inventory.slots[1] = Some(ItemStack {
        id: bota_proto::ItemId(7),
        charges: 2,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    assert_eq!(inventory.held().count(), 1);
    assert_eq!(inventory.slots.len(), 3, "an empty slot keeps its number");

    let book = AbilityBook {
        slots: vec![
            AbilityState {
                id: bota_proto::AbilityId(1),
                level: 0,
                cooldown: 0,
            },
            AbilityState {
                id: bota_proto::AbilityId(2),
                level: 3,
                cooldown: 0,
            },
        ],
    };
    assert_eq!(
        book.learned().count(),
        1,
        "an unlearned ability is not held"
    );
    assert_eq!(book.slot(1).map(|a| a.level), Some(3));
}

#[test]
fn what_a_hero_carries_shows_up_in_its_stats() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.step();
    let bare = world.stats.get(hero).expect("settled").damage;
    // The first item in the table that carries damage rather than charges.
    let (id, def) = crate::game::ITEMS
        .iter()
        .enumerate()
        .find(|(_, d)| d.carried.damage > 0)
        .expect("some item adds damage");
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(id as u16),
            charges: def.charges,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.damage),
        Some(bare + def.carried.damage),
        "the damage it carries is added"
    );
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = None;
    }
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
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        spot + bota_proto::Vec2::from_ints(100, 0),
    );
    world.settle();
    world.step();
    let full = world.health.get(creep).expect("standing").hp;
    put_on(&mut world, hero, crate::game::ModifierKind::Stunned, 60);
    let stood = world.transform.get(hero).expect("standing").pos;
    world.set_order(
        hero,
        crate::game::UnitOrder::Move {
            pos: spot + bota_proto::Vec2::from_ints(1000, 0),
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
    put_on(
        &mut world,
        creep,
        crate::game::ModifierKind::Slowed { pct: 25 },
        60,
    );
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.move_speed),
        Some(bota_proto::Fixed {
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
        crate::game::ModifierKind::Burning {
            amount: 5,
            kind: bota_proto::DamageKind::Pure,
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
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_int(3),
        },
    );
    put_on(
        &mut world,
        creep,
        crate::game::ModifierKind::Burning {
            amount: 100,
            kind: bota_proto::DamageKind::Pure,
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
fn each_hero_stands_up_with_what_its_own_kind_carries() {
    for (id, def) in crate::game::HEROES.iter().enumerate() {
        let mut world = World::new();
        let hero = world.spawn_hero(
            bota_proto::Team::Radiant,
            bota_proto::Vec2::from_ints(5000, 5000),
            bota_proto::SlotId(0),
            bota_proto::HeroId(id as u16),
        );
        world.settle();
        let book = world.abilities.get(hero).expect("a hero casts");
        let carried: Vec<_> = book.slots.iter().map(|slot| slot.id).collect();
        assert_eq!(
            carried,
            def.abilities.to_vec(),
            "{} carries its own",
            def.name
        );
        assert_eq!(
            world.stats.get(hero).map(|s| s.max_hp),
            Some(
                Fixed::from_int(def.unit.max_hp)
                    + Fixed::from_int(rules::HP_PER_STRENGTH) * def.unit.attributes.strength
            ),
            "{} stands up in its own body",
            def.name
        );
    }
}

#[test]
fn what_a_courier_carries_is_worth_nothing_to_the_courier() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    world.step();
    let plain = world.stats.get(courier).expect("settled").move_speed;
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_BOOTS),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
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
    hand_item(&mut world, hero, crate::game::ITEM_BELT, 0);
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
    hand_item(&mut world, hero, crate::game::ITEM_ROBE, 0);
    world.step();
    let with_robe = *world.stats.get(hero).expect("settled");
    assert_eq!(
        with_robe.max_mana - before.max_mana,
        Fixed::from_int(rules::MANA_PER_INTELLIGENCE) * six,
        "intelligence is worth mana"
    );
    hand_item(&mut world, hero, crate::game::ITEM_BAND, 0);
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
    hand_item(&mut world, hero, crate::game::ITEM_GLOVES, 0);
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
        crate::game::attack_gain(hasted.attack_speed)
            > crate::game::attack_gain(plain.attack_speed),
        "and it runs faster for the speed"
    );
}
