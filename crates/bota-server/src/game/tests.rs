//! Entity allocation and component storage.

use bota_proto::{Attributes, Fixed, Team};

use crate::game::rules;
use crate::game::{
    AbilityBook, AbilityState, Def, Entity, EntityAllocator, FLAGBEARER_CREEP, HERO, Health,
    Inventory, ItemStack, Level, MELEE_CREEP, Mana, NEUTRALS, NeutralKind, RANGED_CREEP, StackKind,
    Stats, Status, StatusKind, Statuses, Table, UnitDef, Upgrades, Visibility, World,
};

#[test]
fn global_random_streams_advance_between_draws() {
    let mut rng = crate::game::MatchRng::new(&[7; 32], 11);
    let first = rng.global(crate::game::Purpose::Wave).next_u32();
    let second = rng.global(crate::game::Purpose::Wave).next_u32();

    assert_ne!(
        first, second,
        "a global stream must not restart for every draw"
    );
}

#[test]
fn an_allocator_counts_what_is_live() {
    let mut entities = EntityAllocator::new();
    assert!(entities.is_empty());
    let first = entities.alloc();
    let second = entities.alloc();
    assert_eq!(entities.len(), 2);
    assert!(entities.contains(first) && entities.contains(second));
    assert!(entities.free(first));
    assert_eq!(entities.len(), 1);
    assert!(!entities.is_empty());
}

#[test]
fn a_handle_kept_past_a_death_names_nobody() {
    let mut entities = EntityAllocator::new();
    let dead = entities.alloc();
    assert!(entities.free(dead));
    assert!(!entities.contains(dead));
    assert!(!entities.free(dead), "freeing twice has to be refused");
}

#[test]
fn a_slot_handed_out_again_carries_a_raised_generation() {
    let mut entities = EntityAllocator::new();
    let first = entities.alloc();
    assert!(entities.free(first));
    let second = entities.alloc();
    assert_eq!(first.index(), second.index(), "the free slot is taken back");
    assert_ne!(first.generation(), second.generation());
    assert!(!entities.contains(first) && entities.contains(second));
}

#[test]
fn live_entities_come_out_in_slot_order() {
    let mut entities = EntityAllocator::new();
    let all: Vec<Entity> = (0..4).map(|_| entities.alloc()).collect();
    assert!(entities.free(all[1]));
    let live: Vec<Entity> = entities.iter().collect();
    assert_eq!(live, vec![all[0], all[2], all[3]]);
    let fresh = entities.alloc();
    let live: Vec<Entity> = entities.iter().collect();
    assert_eq!(
        live,
        vec![all[0], fresh, all[2], all[3]],
        "a reused slot is walked where it sits, not where it was made"
    );
}

#[test]
fn a_table_holds_a_component_for_the_entity_that_owns_it() {
    let mut entities = EntityAllocator::new();
    let mine = entities.alloc();
    let theirs = entities.alloc();
    let mut table: Table<i32> = Table::new();
    assert_eq!(table.insert(mine, 7), None);
    assert_eq!(table.get(mine), Some(&7));
    assert!(!table.contains(theirs));
    assert_eq!(table.insert(mine, 9), Some(7), "the old value comes back");
    *table.get_mut(mine).expect("just written") += 1;
    assert_eq!(table.get(mine), Some(&10));
    assert_eq!(table.remove(mine), Some(10));
    assert!(!table.contains(mine));
    assert_eq!(table.remove(mine), None);
}

#[test]
fn what_a_dead_entity_left_is_never_the_new_tenants() {
    let mut entities = EntityAllocator::new();
    let first = entities.alloc();
    let mut table: Table<i32> = Table::new();
    table.insert(first, 7);
    assert!(entities.free(first));
    let second = entities.alloc();
    assert_eq!(first.index(), second.index());
    assert_eq!(table.get(second), None, "the slot came empty");
    assert_eq!(
        table.insert(second, 3),
        None,
        "nothing of its own to return"
    );
    assert_eq!(table.get(second), Some(&3));
    assert_eq!(table.get(first), None, "the old handle reads nothing");
}

#[test]
fn a_table_answers_for_the_handle_it_is_given_not_for_the_living() {
    // Liveness is the allocator's to know. Until the slot changes hands the
    // table still answers the dead handle, so walking entities goes through
    // `EntityAllocator::iter` and never through a table alone.
    let mut entities = EntityAllocator::new();
    let gone = entities.alloc();
    let mut table: Table<i32> = Table::new();
    table.insert(gone, 7);
    assert!(entities.free(gone));
    assert!(!entities.contains(gone));
    assert_eq!(table.get(gone), Some(&7));
}

/// A stat block with every field named, so a new one has to be thought about
/// here before any test compiles again.
fn stats() -> Stats {
    Stats {
        max_hp: Fixed::from_int(20),
        max_mana: Fixed::from_int(20),
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        damage: 0,
        attack_range: Fixed::ZERO,
        acquisition: Fixed::ZERO,
        attack_interval: 30,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        attack_point: 0,
        attack_backswing: 0,
        projectile_speed: None,
        armor: Fixed::ZERO,
        magic_resist_pct: 0,
        move_speed: Fixed::ZERO,
        turn_rate: 0,
        damage_to_creeps: 0,
        vision: Fixed::ZERO,
        true_sight: Fixed::ZERO,
        hides: false,
        flies: false,
        phased: false,
        invulnerable: false,
    }
}

/// A pool holding a whole number of points.
fn health(hp: i32) -> Health {
    Health {
        hp: Fixed::from_int(hp),
    }
}

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
fn who_sees_an_entity_is_a_set_of_bits() {
    let mut seen = Visibility::default();
    assert!(!seen.by(Team::Radiant));
    seen.add(Team::Radiant);
    seen.add(Team::Radiant);
    assert!(seen.by(Team::Radiant), "naming a side twice sets one bit");
    assert!(!seen.by(Team::Dire), "and leaves the others alone");
    seen.add(Team::Dire);
    assert!(seen.by(Team::Radiant) && seen.by(Team::Dire));
    seen.clear();
    assert!(seen.is_empty() && !seen.by(Team::Dire));
}

/// A creep with nothing done to it.
fn plain_creep(world: &mut World) -> Entity {
    let creep = world.spawn();
    world.def.insert(creep, Def(&MELEE_CREEP));
    world.health.insert(creep, Health { hp: Fixed::ZERO });
    creep
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
    assert_eq!(stats.attack_interval, rules::CREEP_ATTACK_INTERVAL);
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

/// The health a hero of the plain kind holds so many levels past the first,
/// counting what its strength is worth.
fn body_at(levels: i32) -> Fixed {
    let strength = rules::HERO_ATTRIBUTES.strength
        + rules::HERO_ATTRIBUTES_PER_LEVEL.strength * Fixed::from_int(levels);
    Fixed::from_int(rules::HERO_HP + levels * rules::HERO_HP_PER_LEVEL)
        + Fixed::from_int(rules::HP_PER_STRENGTH) * strength
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
    world.statuses.insert(
        creep,
        Statuses(vec![Status {
            kind: StatusKind::Haste { speed: 40 },
            ticks_left: 5,
        }]),
    );
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.attack_interval),
        Some(rules::CREEP_ATTACK_INTERVAL * 100 / 140)
    );
    world.statuses.insert(creep, Statuses(Vec::new()));
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.attack_interval),
        Some(rules::CREEP_ATTACK_INTERVAL),
        "what is worked out afresh forgets what has lifted"
    );
}

#[test]
fn mending_adds_to_what_a_kind_regenerates() {
    let mut world = World::new();
    let creep = plain_creep(&mut world);
    world.statuses.insert(
        creep,
        Statuses(vec![Status {
            kind: StatusKind::Mending {
                per_tick: 25,
                breaks: false,
            },
            ticks_left: 5,
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
    assert_eq!(kobold.radius, rules::NEUTRAL_RADIUS);
    assert_eq!(kobold.vision, rules::NEUTRAL_VISION);
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

/// Buildings a map stands up: its towers, its barracks, both fountains, and
/// whatever Ancients it has.
fn map_buildings(map: &crate::game::MapDef) -> usize {
    map.radiant_towers.len()
        + map.dire_towers.len()
        + map.barracks[0].len()
        + map.barracks[1].len()
        + 2
        + map.ancients.iter().flatten().count()
}

#[test]
fn a_world_built_on_a_map_stands_its_buildings_full() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let world = World::on_map(map);
    assert_eq!(
        world.entities.len(),
        map_buildings(map),
        "every building and nothing else"
    );
    let view = world.view_full();
    assert_eq!(view.units.len(), world.entities.len());
    let tower = view
        .units
        .iter()
        .find(|u| u.kind == bota_proto::UnitKind::Tower)
        .expect("a tower stands");
    assert_eq!(tower.hp, rules::TOWER_TIER_HP[0], "built and full");
    assert_eq!(tower.max_hp, rules::TOWER_TIER_HP[0]);
    // The demo map raises no Ancients at all.
    assert!(
        !view
            .units
            .iter()
            .any(|u| u.kind == bota_proto::UnitKind::Ancient),
        "no Ancient stands on the demo map"
    );
}

#[test]
fn both_sides_are_always_told_of_every_building() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let world = World::on_map(map);
    let standing = map_buildings(map);
    let view = world.view(bota_proto::Team::Radiant);
    assert_eq!(
        view.units.len(),
        standing,
        "every building on the map is told, whosever it is"
    );
    let dire_fountain = map.fountains[1];
    assert!(
        view.units.iter().any(|u| u.pos == dire_fountain),
        "including the one across the map it has no eyes on"
    );
}

#[test]
fn a_unit_across_the_map_is_still_kept_from_a_side() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let far = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Dire, map.fountains[1]);
    world.settle();
    assert!(
        !world.can_see(bota_proto::Team::Radiant, far),
        "a creep is not a building"
    );
    let view = world.view(bota_proto::Team::Radiant);
    assert!(
        !view.units.iter().any(|u| u.id == crate::game::wire_id(far)),
        "and the fog keeps it back"
    );
}

#[test]
fn a_unit_still_standing_never_reads_as_empty() {
    let mut world = World::new();
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::ZERO,
    );
    world.settle();
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_ratio(1, 4),
        },
    );
    let view = world.view_full();
    let shown = view.units.first().expect("one unit").hp;
    assert_eq!(shown, 1, "a quarter of a point still shows as one");
}

#[test]
fn an_order_to_walk_moves_a_body_and_turns_it_first() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Move {
                pos: bota_proto::Vec2::from_ints(0, 1000),
            },
            cooldown: 0,
        },
    );
    // Facing east and sent west: it turns before it takes a step.
    let start = world.transform.get(hero).expect("placed").pos;
    world.step();
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(start),
        "the first tick is spent coming round"
    );
    for _ in 0..30 {
        world.step();
    }
    let now = world.transform.get(hero).expect("alive").pos;
    assert!(now.x < start.x, "it walks towards where it was sent");
    assert_eq!(now.y, start.y, "and holds the line it was sent along");
}

#[test]
fn a_side_sees_what_stands_inside_its_sight() {
    let mut world = World::new();
    let watcher = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1200, 1000),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(9000, 9000),
    );
    world.settle();
    world.step();
    let seen_near = world.visibility.get(near).expect("worked out this tick");
    assert!(seen_near.by(bota_proto::Team::Radiant), "close enough");
    assert!(seen_near.by(bota_proto::Team::Dire), "its own side always");
    let seen_far = world.visibility.get(far).expect("worked out this tick");
    assert!(!seen_far.by(bota_proto::Team::Radiant), "out of sight");
    let view = world.view(bota_proto::Team::Radiant);
    let ids: Vec<_> = view.units.iter().map(|u| u.id).collect();
    assert!(ids.contains(&crate::game::wire_id(watcher)));
    assert!(ids.contains(&crate::game::wire_id(near)));
    assert!(
        !ids.contains(&crate::game::wire_id(far)),
        "fog holds it back"
    );
}

#[test]
fn a_tower_takes_the_nearest_enemy_and_brings_it_down() {
    let mut world = World::new();
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1200, 1000),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1600, 1000),
    );
    world.settle();
    world.step();
    assert_eq!(
        world.target_of(tower),
        Some(near),
        "the nearer one is taken"
    );
    let mut ticks = 0;
    while world.alive(near) && ticks < 900 {
        world.step();
        ticks += 1;
    }
    assert!(!world.alive(near), "a tower brings a creep down");
    assert!(world.alive(far), "the other one was never in reach");
    assert!(!world.entities.contains(near), "what falls is cleared away");
}

#[test]
fn a_fallen_ancient_ends_the_match() {
    let mut world = World::new();
    let ancient = world.spawn_unit(
        &crate::game::ANCIENT,
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
fn a_missile_carries_the_hit_rather_than_landing_it_at_once() {
    let mut world = World::new();
    let archer = world.spawn_unit(
        &RANGED_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1400, 1000),
    );
    world.settle();
    let full = world.health.get(mark).expect("standing").hp;
    for _ in 0..20 {
        world.step();
    }
    assert!(
        world.projectile.get(archer).is_none(),
        "the archer is not its own missile"
    );
    let mut flying = 0;
    for entity in world.entities.iter() {
        if world.projectile.get(entity).is_some() {
            flying += 1;
        }
    }
    assert!(flying > 0 || world.health.get(mark).expect("standing").hp < full);
}

#[test]
fn a_wave_arrives_on_the_clock_and_walks_its_lane() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    let wave: Vec<_> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .collect();
    let plan = crate::game::wave_plan(1);
    let per_side = (plan.melee + plan.ranged + plan.siege) as usize;
    assert_eq!(
        wave.len(),
        per_side * 2 * usize::from(map.lanes),
        "one wave a lane a side"
    );
    let radiant: Vec<_> = wave
        .iter()
        .copied()
        .filter(|e| world.team.get(*e) == Some(&bota_proto::Team::Radiant))
        .collect();
    let start = world.transform.get(radiant[0]).expect("just placed").pos.x;
    for _ in 0..90 {
        world.step();
    }
    let now = world.transform.get(radiant[0]).expect("alive").pos.x;
    assert!(
        now > start,
        "a Radiant creep walks up its lane: {now:?} from {start:?}"
    );
}

#[test]
fn a_wave_carries_one_flag_from_the_fifth_wave_on() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let fifth = rules::FIRST_WAVE_TICK + 4 * rules::WAVE_PERIOD_TICKS;
    while world.tick < fifth {
        world.step();
    }
    let flags = world
        .entities
        .iter()
        .filter(|e| world.kind.get(*e) == Some(&bota_proto::UnitKind::CreepFlagbearer))
        .count();
    assert_eq!(flags, 2 * usize::from(map.lanes), "one a lane a side");
}

#[test]
fn a_unit_is_taken_before_a_building_and_a_siege_creep_takes_the_building_first() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(1000, 1000);
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    let siege = world.spawn_unit(&crate::game::SIEGE_CREEP, bota_proto::Team::Radiant, at);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1100, 1000),
    );
    let enemy = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1300, 1000),
    );
    world.settle();
    let reach = world.stats.get(creep).expect("settled").acquisition;
    assert_eq!(
        world.acquire(creep, reach, crate::game::PriorityOrder::Normal),
        Some(enemy),
        "a unit outranks a building however much nearer the building stands"
    );
    let siege_reach = world.stats.get(siege).expect("settled").acquisition;
    assert_eq!(
        world.acquire(siege, siege_reach, crate::game::PriorityOrder::SiegeFirst),
        Some(tower),
        "a siege creep goes for the building"
    );
    assert_eq!(
        world.priority_of(siege),
        crate::game::PriorityOrder::SiegeFirst
    );
    assert_eq!(world.priority_of(creep), crate::game::PriorityOrder::Normal);
}

#[test]
fn a_building_never_shoots_the_jungle_and_a_creep_only_at_a_pull_camp() {
    let mut world = World::new();
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let beast = world.spawn_unit(
        crate::game::NeutralKind::Kobold.def(),
        bota_proto::Team::Neutral,
        bota_proto::Vec2::from_ints(1100, 1000),
    );
    world.settle();
    assert!(!world.hostile(tower, beast), "a tower leaves the jungle be");
    assert!(world.hostile(hero, beast), "a hero may hit it");
    assert!(
        !world.hostile(creep, beast),
        "a lane creep leaves a camp it cannot be pulled to"
    );
    let pull = crate::game::CAMPS
        .iter()
        .find(|c| c.pullable)
        .expect("the map marks pull camps");
    world.camp_home.insert(
        beast,
        crate::game::CampHome {
            camp: 0,
            home: pull.pos,
        },
    );
    assert!(
        world.hostile(creep, beast),
        "at a pull camp it will fight after all"
    );
}

#[test]
fn a_body_does_not_walk_through_a_building() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let (_, _, tower) = map.radiant_towers[0];
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        tower - bota_proto::Vec2::from_ints(400, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Move {
                pos: tower + bota_proto::Vec2::from_ints(400, 0),
            },
            cooldown: 0,
        },
    );
    let mut nearest = i64::MAX;
    for _ in 0..300 {
        world.step();
        let at = world.transform.get(hero).expect("alive").pos;
        nearest = nearest.min(crate::game::isqrt64(at.distance_squared(tower)));
    }
    let hull = world.hull.get(hero).expect("has one").radius;
    assert!(
        nearest > i64::from(hull.raw),
        "it never stood inside the tower"
    );
    let end = world.transform.get(hero).expect("alive").pos;
    assert!(end.x > tower.x, "and it got past all the same: {end:?}");
}

#[test]
fn two_bodies_on_one_spot_are_eased_apart() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let one = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    let other = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    world.settle();
    for _ in 0..30 {
        world.step();
    }
    let (a, b) = (
        world.transform.get(one).expect("alive").pos,
        world.transform.get(other).expect("alive").pos,
    );
    let hulls = world.hull.get(one).expect("has one").radius
        + world.hull.get(other).expect("has one").radius;
    // Easing apart stops at the moment they stop overlapping, which leaves
    // them touching exactly.
    let apart = crate::game::isqrt64(a.distance_squared(b));
    assert!(
        apart >= i64::from(hulls.raw),
        "still inside one another: {a:?} {b:?}"
    );
}

#[test]
fn camps_fill_on_the_minute_and_stay_full() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let filled = world
        .entities
        .iter()
        .filter(|e| world.camp_home.get(*e).is_some())
        .count();
    assert!(filled > 0, "the jungle put something out");
    let before = filled;
    for _ in 0..rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
    }
    let now = world
        .entities
        .iter()
        .filter(|e| world.camp_home.get(*e).is_some())
        .count();
    assert_eq!(now, before, "a full camp puts out nothing more");
}

#[test]
fn a_neutral_led_too_far_gives_up_and_walks_home() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let beast = world
        .entities
        .iter()
        .find(|e| world.camp_home.get(*e).is_some())
        .expect("the jungle is full");
    let home = world.camp_home.get(beast).expect("it has one").home;
    // Carried well past its guard distance and held there.
    let away = home + bota_proto::Vec2::from_ints(1200, 0);
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = away;
    }
    for _ in 0..rules::NEUTRAL_AGGRO_WINDOW + 2 {
        world.step();
        if let Some(transform) = world.transform.get_mut(beast) {
            transform.pos = away;
        }
    }
    assert!(
        world.neutral_ai.get(beast).is_some_and(|ai| ai.going_home),
        "its patience ran out"
    );
    assert!(
        world.target.get(beast).is_none(),
        "and it takes nothing on while it walks back"
    );
    // Home again, it stands, and is led less far the next time.
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = home;
    }
    world.step();
    let ai = world.neutral_ai.get(beast).copied().expect("it has a mind");
    assert!(!ai.going_home, "back home it stands again");
    assert_eq!(
        ai.next_window,
        rules::NEUTRAL_SHORT_WINDOW,
        "and its patience is short from now on"
    );
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
        rules::HERO_KILL_XP_BASE + 4 * rules::HERO_KILL_XP_PER_LEVEL,
        "experience pays by the fallen hero's level"
    );
    assert_eq!(
        world.seats[1].gold,
        500 - 4 * rules::DEATH_GOLD_LOSS_PER_LEVEL,
        "dying costs gold by the level"
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
    world.seats[0].gold = 50;
    world.seats[0].net_worth = 50;
    world.seats[0].level = 10;
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(prey, None)], &mut events);
    assert_eq!(world.seats[0].gold, 0, "the purse is emptied, not owed");
    assert_eq!(world.seats[0].net_worth, 0);
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
    assert!(world.grid.walkable(at), "and on ground it can walk off");
    let full = world.stats.get(back).expect("settled").max_hp;
    assert_eq!(world.health.get(back).map(|h| h.hp), Some(full), "and full");
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

/// How long the salve mends for.
fn salve_ticks() -> u32 {
    match crate::game::ITEMS[usize::from(crate::game::ITEM_HEALING_SALVE)].active {
        Some(crate::game::ItemUse::Mend { ticks, .. }) => ticks,
        _ => panic!("the salve mends"),
    }
}

#[test]
fn a_salve_puts_mending_on_whoever_drinks_it_and_runs_out() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.step();
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE),
            charges: 1,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None),
        "it drinks"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the last charge takes the stack with it"
    );
    world.step();
    let plain = crate::game::HERO.hp_regen
        + rules::HP_REGEN_PER_STRENGTH * crate::game::HERO.attributes.strength;
    assert!(
        world.stats.get(hero).expect("settled").hp_regen > plain,
        "it mends faster while the salve holds"
    );
    for _ in 0..salve_ticks() + 1 {
        world.step();
    }
    assert_eq!(
        world.stats.get(hero).map(|s| s.hp_regen),
        Some(plain),
        "and back to its own once it runs out"
    );
}

#[test]
fn a_side_is_told_only_of_what_it_could_see() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let far = bota_proto::Vec2::from_ints(9600, 12000);
    assert!(
        !world.can_see_point(bota_proto::Team::Radiant, far),
        "nothing of that side stands anywhere near"
    );
    assert_eq!(
        world.who_may_know(far, bota_proto::Team::Dire),
        crate::game::EventVisibility::OneTeam(bota_proto::Team::Dire),
        "only the side party to it is told"
    );
    let watcher = world.spawn_hero(
        bota_proto::Team::Radiant,
        far,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    assert!(world.can_see_point(bota_proto::Team::Radiant, far));
    assert_eq!(
        world.who_may_know(far, bota_proto::Team::Dire),
        crate::game::EventVisibility::Everyone
    );
    let _ = watcher;
}

#[test]
fn an_order_at_something_a_side_cannot_see_is_refused() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(6800, 9216),
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
    let hidden = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(13200, 9216),
    );
    world.settle();
    crate::game::visibility_system(crate::game::SightCx {
        entities: &world.entities,
        transform: &world.transform,
        team: &world.team,
        kind: &world.kind,
        stats: &world.stats,
        ground: &world.ground,
        sight_block: &world.sight_block,
        visibility: &mut world.visibility,
    });
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(hidden)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Err(bota_proto::RejectReason::UnknownTarget),
        "it is nowhere near and cannot be ordered at"
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(6900, 9216),
    );
    world.settle();
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(near)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
}

#[test]
fn a_seat_with_no_body_standing_may_order_nothing() {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let order = bota_proto::Order::Move {
        target: bota_proto::Target::Pos(bota_proto::Vec2::ZERO),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Err(bota_proto::RejectReason::HeroDead)
    );
}

/// A match config that always names the same numbers.
fn config() -> crate::game::MatchConfig {
    crate::game::MatchConfig {
        match_id: 7,
        master_key: [3; 32],
        picks: vec![bota_proto::Pick {
            slot: bota_proto::SlotId(0),
            team: bota_proto::Team::Radiant,
            hero: bota_proto::HeroId(0),
        }],
        map: bota_proto::MapId(1),
        tick_rate: 30,
        mode: bota_proto::TickMode::Lockstep,
        ack_timeout_ticks: 30,
    }
}

#[test]
fn two_runs_of_one_script_agree_at_every_checkpoint() {
    let mut first = World::for_match(&config(), config().rng());
    let mut second = World::for_match(&config(), config().rng());
    assert_eq!(first.hash(), second.hash(), "they start the same");
    for tick in 1..=600u32 {
        first.step();
        second.step();
        if tick % 60 == 0 {
            assert_eq!(first.hash(), second.hash(), "they parted at tick {tick}");
        }
    }
}

#[test]
fn world_hash_changes_when_hidden_random_state_advances() {
    let mut world = World::new();
    let before = world.hash();

    world.rng.global(crate::game::Purpose::Wave).next_u32();

    assert_ne!(world.hash(), before);
}

#[test]
fn world_hash_changes_when_uphill_prd_state_advances() {
    let mut world = World::new();
    let stream = world.rng.for_unit(
        crate::game::Purpose::Evasion,
        bota_proto::EntityId {
            idx: 0,
            generation: 1,
        },
        0,
    );
    world
        .uphill_miss
        .push(Some(crate::game::PseudoRandom25::new(stream)));
    let before = world.hash();

    world.uphill_miss[0].as_mut().expect("chance").roll();

    assert_ne!(world.hash(), before);
}

fn world_with_projectile_uphill_state(launch_tier: u8, can_miss_uphill: bool) -> World {
    let mut world = World::new();
    let target = world.spawn();
    let missile = world.spawn();
    world.projectile.insert(
        missile,
        crate::game::Projectile {
            speed: Fixed::ONE,
            source: None,
            target,
            damage: 1,
            kind: bota_proto::DamageKind::Physical,
            ability: None,
            launch_tier,
            can_miss_uphill,
            crit: false,
            bounces_left: 0,
            bounce_range: 0,
            bounced: Vec::new(),
        },
    );
    world
}

#[test]
fn world_hash_includes_projectile_uphill_state() {
    let level = world_with_projectile_uphill_state(1, true);
    let higher_launch = world_with_projectile_uphill_state(2, true);
    let cannot_miss = world_with_projectile_uphill_state(1, false);

    assert_ne!(level.hash(), higher_launch.hash());
    assert_ne!(level.hash(), cannot_miss.hash());
}

#[test]
fn the_fingerprint_moves_when_the_world_does() {
    let mut world = World::for_match(&config(), config().rng());
    let before = world.hash();
    world.step();
    assert_ne!(before, world.hash(), "a tick is a change");
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(9000, 9216),
    );
    world.settle();
    let with = world.hash();
    if let Some(health) = world.health.get_mut(creep) {
        health.hp -= Fixed::ONE;
    }
    assert_ne!(with, world.hash(), "so is a point of health");
}

/// A lane creep on clear ground with a mind of its own.
fn thinking_creep(world: &mut World, at: bota_proto::Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    world.lane_ai.insert(
        creep,
        crate::game::LaneAi {
            anchor: None,
            last_seen: None,
            keep_until: 0,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    creep
}

#[test]
fn a_creep_gives_up_a_chase_it_cannot_finish() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let prey = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.step();
    assert_eq!(world.target_of(creep), Some(prey), "it takes it");
    // Carried out of everything it can see, and the creep held where it was.
    let away = bota_proto::Vec2::from_ints(5900, 5000);
    for _ in 0..rules::CREEP_CHASE_TICKS + 2 {
        if let Some(at) = world.transform.get_mut(prey) {
            at.pos = away;
        }
        if let Some(at) = world.transform.get_mut(creep) {
            at.pos = bota_proto::Vec2::from_ints(5000, 5000);
        }
        world.step();
    }
    assert_eq!(
        world.target_of(creep),
        None,
        "the chase ran out and it let go"
    );
}

#[test]
fn a_creep_walks_back_to_where_it_left_its_route() {
    let mut world = World::new();
    let start = bota_proto::Vec2::from_ints(5000, 5000);
    let creep = thinking_creep(&mut world, start);
    world.settle();
    if let Some(ai) = world.lane_ai.get_mut(creep) {
        ai.anchor = Some(start);
    }
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos = bota_proto::Vec2::from_ints(5600, 5000);
    }
    world.step();
    let order = world.orders.get(creep).map(|o| o.current);
    assert_eq!(
        order,
        Some(crate::game::UnitOrder::AttackMove { pos: start }),
        "it is sent back to where it left"
    );
}

#[test]
fn an_attack_order_at_an_ally_never_hands_the_creep_the_one_who_gave_it() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5040, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.provoke(creep, hero, true);
    world.step();
    assert_eq!(
        world.target_of(creep),
        Some(other),
        "the nearer hero is put last, so the creep takes the creep"
    );
}

#[test]
fn an_attack_order_at_an_enemy_hands_the_creep_over_and_holds_it() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5040, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.provoke(creep, hero, false);
    let roused_at = world.tick;
    world.step();
    assert_eq!(world.target_of(creep), Some(hero), "handed over outright");
    assert_eq!(
        world.lane_ai.get(creep).map(|ai| ai.keep_until),
        Some(roused_at + rules::ORDER_AGGRO_HOLD_TICKS),
        "and held for two and a third seconds"
    );
}

#[test]
fn one_creep_answers_an_order_once_every_three_seconds() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let first = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5040, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let second = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5060, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.provoke(creep, first, false);
    world.step();
    assert_eq!(world.target_of(creep), Some(first));
    world.provoke(creep, second, false);
    world.step();
    assert_eq!(
        world.target_of(creep),
        Some(first),
        "a second order inside the wait passes it by"
    );
    // Waited out.
    world.tick += rules::ORDER_AGGRO_COOLDOWN_TICKS;
    if let Some(orders) = world.orders.get_mut(creep) {
        orders.cooldown = 0;
    }
    world.provoke(creep, second, false);
    world.step();
    assert_eq!(
        world.target_of(creep),
        Some(second),
        "once the wait is out it answers again"
    );
}

/// A hero that has learned one ability to its first level, with mana to spend.
fn caster(world: &mut World, at: bota_proto::Vec2, slot: usize) -> Entity {
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        at,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    // Levelled enough that the slot under test is one it may learn.
    world
        .level
        .insert(hero, crate::game::Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(hero);
    let mut events = Vec::new();
    assert!(world.learn(hero, slot, &mut events), "the slot is learned");
    hero
}

#[test]
fn frenzy_puts_haste_on_its_caster_and_spends_the_mana() {
    let mut world = World::new();
    let hero = caster(&mut world, bota_proto::Vec2::from_ints(5000, 5000), 1);
    let full = world.mana.get(hero).expect("has mana").mana;
    let plain = world.stats.get(hero).expect("settled").attack_interval;
    world.order_cast(
        hero,
        crate::game::PendingCast {
            slot: bota_proto::AbilitySlot(1),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert!(
        world.mana.get(hero).expect("has mana").mana < full,
        "a cast costs mana"
    );
    // Stats are worked out before casts run, so what a cast puts on shows
    // from the tick after.
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").attack_interval < plain,
        "and it swings faster while the haste holds"
    );
    assert!(
        world.abilities.get(hero).expect("has a book").slots[1].cooldown > 0,
        "and waits before casting again"
    );
}

#[test]
fn a_cast_with_no_mana_behind_it_does_nothing() {
    let mut world = World::new();
    let hero = caster(&mut world, bota_proto::Vec2::from_ints(5000, 5000), 1);
    if let Some(mana) = world.mana.get_mut(hero) {
        mana.mana = Fixed::ZERO;
    }
    world.order_cast(
        hero,
        crate::game::PendingCast {
            slot: bota_proto::AbilitySlot(1),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert_eq!(
        world.abilities.get(hero).expect("has a book").slots[1].cooldown,
        0,
        "nothing was spent and nothing began"
    );
}

#[test]
fn a_multishot_strikes_everything_around_and_leaves_allies_be() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let hero = caster(&mut world, at, 3);
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5200, 5000),
    );
    let ally = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5200, 5100),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 5000),
    );
    world.settle();
    world.fill_pools(near);
    world.fill_pools(ally);
    world.fill_pools(far);
    let (was_near, was_ally, was_far) = (
        world.health.get(near).expect("standing").hp,
        world.health.get(ally).expect("standing").hp,
        world.health.get(far).expect("standing").hp,
    );
    world.order_cast(
        hero,
        crate::game::PendingCast {
            slot: bota_proto::AbilitySlot(3),
            target: bota_proto::Target::None,
        },
    );
    world.step();
    assert!(
        world.health.get(near).expect("standing").hp < was_near,
        "the one in the ring is struck"
    );
    assert_eq!(
        world.health.get(ally).expect("standing").hp,
        was_ally,
        "its own side is left be"
    );
    assert_eq!(
        world.health.get(far).expect("standing").hp,
        was_far,
        "and the one outside the ring is untouched"
    );
}

#[test]
fn buying_needs_the_shop() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let away = bota_proto::Vec2::from_ints(9600, 9216);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        away,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mut seat = crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        rules::STARTING_GOLD,
        rules::STASH_SLOTS,
    );
    seat.unit = Some(hero);
    world.seats.push(seat);
    world.settle();
    let mut events = Vec::new();
    let salve = bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE);
    assert!(
        world.buy(bota_proto::SlotId(0), salve, &mut events),
        "out in the lane it buys all the same"
    );
    assert_eq!(
        world.seats[0].stash.slots[0].map(|stack| stack.id),
        Some(salve),
        "and what it bought waits in the stash"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").held().count() == 0,
        "nothing reaches its hands out there"
    );
    if let Some(at) = world.transform.get_mut(hero) {
        at.pos = crate::game::fountain_pos(map, bota_proto::Team::Radiant);
    }
    assert!(
        world.buy(bota_proto::SlotId(0), salve, &mut events),
        "at its own shop it may"
    );
    assert!(world.seats[0].gold < rules::STARTING_GOLD, "and it paid");
}

#[test]
fn buying_through_a_command_returns_the_purchase_event() {
    let mut world = World::for_match(&config(), config().rng());
    let item = bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE);

    let events = world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Buy { item },
    }]);

    assert!(events.iter().any(|event| {
        event.kind
            == bota_proto::EventKind::ItemBought {
                slot: bota_proto::SlotId(0),
                item,
            }
    }));
}

#[test]
fn critical_hits_keep_their_flag_in_damage_events() {
    let mut world = World::new();
    let source = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5_000, 5_000),
    );
    let target = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5_100, 5_000),
    );
    world.settle();
    world.hits.push_back(crate::game::Hit {
        source: Some(source),
        target,
        amount: 10,
        kind: bota_proto::DamageKind::Physical,
        crit: true,
    });

    let events = world.step();

    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            bota_proto::EventKind::Damaged {
                source: Some(_),
                target: _,
                amount: _,
                kind: bota_proto::DamageKind::Physical,
                crit: true,
            }
        )
    }));
}

#[test]
fn a_ranged_attack_puts_a_missile_where_a_side_can_see_it() {
    let mut world = World::new();
    let archer = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.fill_pools(archer);
    world.fill_pools(mark);
    let mut flew = false;
    for _ in 0..40 {
        world.step();
        if !world.view_full().projectiles.is_empty() {
            flew = true;
            break;
        }
    }
    assert!(
        flew,
        "a ranged hero throws something the client is told about"
    );
    let view = world.view(bota_proto::Team::Radiant);
    assert!(
        !view.projectiles.is_empty(),
        "and its own side is told of it"
    );
}

#[test]
fn a_creep_is_sent_one_way_at_a_time() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    let creep = world
        .entities
        .iter()
        .find(|e| {
            world.march.get(*e).is_some() && world.team.get(*e) == Some(&bota_proto::Team::Radiant)
        })
        .expect("a wave came out");
    // Left the route: the mark it was given must not change from tick to tick
    // while nothing about it changes.
    if let Some(ai) = world.lane_ai.get_mut(creep) {
        ai.anchor = Some(bota_proto::Vec2::from_ints(6800, 9600));
    }
    world.step();
    let first = world.orders.get(creep).map(|o| o.current);
    world.step();
    let second = world.orders.get(creep).map(|o| o.current);
    assert_eq!(first, second, "it is not pulled two ways in one breath");
    let x = world.transform.get(creep).expect("alive").pos.x.to_int();
    for _ in 0..60 {
        world.step();
    }
    let later = world.transform.get(creep).expect("alive").pos.x.to_int();
    assert!(
        (later - x).abs() > 40,
        "and it actually goes somewhere: {x} then {later}"
    );
}

#[test]
fn a_visibility_row_belongs_to_whatever_stands_on_a_side() {
    let mut world = World::new();
    let entity = world.spawn();
    assert_eq!(
        world.visibility.get(entity),
        None,
        "what stands on no side is not something sides see"
    );
    world.set_team(entity, Team::Radiant);
    assert!(
        world
            .visibility
            .get(entity)
            .is_some_and(|s| s.by(Team::Radiant)),
        "taking a side makes the row, and that side has it at once"
    );
    world.step();
    assert!(
        world
            .visibility
            .get(entity)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it from the first tick"
    );
    assert!(world.despawn(entity));
    assert_eq!(world.visibility.get(entity), None, "and given up with it");
    let next = world.spawn();
    assert_eq!(next.index(), entity.index(), "the slot came back round");
    assert_eq!(
        world.visibility.get(next),
        None,
        "and the new tenant inherits nothing"
    );
}

#[test]
fn a_missile_is_seen_from_the_tick_it_is_thrown() {
    let mut world = World::new();
    let archer = world.spawn_unit(
        &RANGED_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
    );
    let watcher = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    let mut thrown = None;
    for _ in 0..40 {
        world.step();
        thrown = world
            .entities
            .iter()
            .find(|e| world.projectile.get(*e).is_some());
        if thrown.is_some() {
            break;
        }
    }
    let missile = thrown.expect("the archer threw something");
    assert!(
        world
            .visibility
            .get(missile)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it"
    );
    let _ = (archer, watcher);
}

/// Runs one tick of the attack cycle and nothing else.
fn swing_once(world: &mut World) {
    crate::game::attacking_system(crate::game::AttackCx {
        entities: &mut world.entities,
        transform: &mut world.transform,
        hull: &world.hull,
        ground: &world.ground,
        kind: &world.kind,
        team: &mut world.team,
        health: &world.health,
        stats: &world.stats,
        visibility: &mut world.visibility,
        target: &world.target,
        statuses: &world.statuses,
        attacking: &mut world.attacking,
        hits: &mut world.hits,
        projectile: &mut world.projectile,
    });
}

/// An attacker and its mark, standing where they are put and nothing else.
fn duel(gap: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let attacker = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5000 + gap, 5000),
    );
    world.settle();
    (world, attacker, mark)
}

#[test]
fn a_swing_waits_on_the_angle_it_is_looking_at() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    // Turned right away from it: in reach, seen, but not looked at.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = bota_proto::Angle { brads: 32768 };
    }
    swing_once(&mut world);
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "nothing begins while it is looking the other way"
    );
    // Looking straight at it.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = bota_proto::Angle { brads: 0 };
    }
    swing_once(&mut world);
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some(),
        "and begins once it is"
    );
}

#[test]
fn a_swing_waits_on_being_able_to_see_at_all() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    swing_once(&mut world);
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "what a side has no eyes on it does not swing at"
    );
}

#[test]
fn a_swing_waits_on_reach() {
    let (mut world, attacker, mark) = duel(600);
    world.set_target(attacker, mark);
    swing_once(&mut world);
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "too far to touch"
    );
}

#[test]
fn a_swing_lands_on_whoever_it_began_against() {
    let (mut world, attacker, mark) = duel(100);
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.set_target(attacker, mark);
    world.step();
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some(),
        "the swing began"
    );
    // Set on somebody else halfway through.
    world.set_target(attacker, other);
    let was = world.health.get(mark).expect("standing").hp;
    for _ in 0..rules::MELEE_CREEP_ATTACK_POINT + 1 {
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "and landed on the one it began against"
    );
}

#[test]
fn a_hero_told_to_attack_comes_round_and_closes() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(4000, 5000),
    );
    world.settle();
    // Ordered at it, the way a player does.
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Attack {
                target: mark,
                last_seen: bota_proto::Vec2::from_ints(4000, 5000),
            },
            cooldown: 0,
        },
    );
    let start = world.transform.get(hero).expect("standing").pos;
    for _ in 0..60 {
        world.step();
    }
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(now.x < start.x, "it walked at what it was set on: {now:?}");
}

#[test]
fn a_swing_that_began_still_connects_a_little_past_reach() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some(),
        "the swing began in reach"
    );
    // Backs off by less than the leeway while the swing is under way.
    let just_out = rules::MELEE_CREEP_ATTACK_RANGE + 60;
    let was = world.health.get(mark).expect("standing").hp;
    for _ in 0..rules::MELEE_CREEP_ATTACK_POINT + 1 {
        if let Some(at) = world.transform.get_mut(mark) {
            at.pos = bota_proto::Vec2::from_ints(5000 + just_out, 5000);
        }
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "a step past reach does not shake it off"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_gets_away() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some(),
        "the swing began in reach"
    );
    let far = rules::MELEE_CREEP_ATTACK_RANGE + rules::ATTACK_RANGE_LEEWAY + 200;
    let was = world.health.get(mark).expect("standing").hp;
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(5000 + far, 5000);
    }
    world.step();
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "the swing is given up the moment it gets away"
    );
    assert_eq!(
        world.attacking.get(attacker).map(|a| a.cooldown),
        Some(0),
        "and costs nothing, so the next one may start at once"
    );
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        was,
        "nothing was struck"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_falls() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some()
    );
    world.health.insert(mark, Health { hp: Fixed::ZERO });
    world.step();
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "there is nothing left to strike"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_is_lost_from_sight() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        world
            .attacking
            .get(attacker)
            .and_then(|a| a.windup)
            .is_some()
    );
    // Blinded to it, the way stepping into fog would.
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    swing_once(&mut world);
    assert_eq!(
        world.attacking.get(attacker).and_then(|a| a.windup),
        None,
        "it does not finish a swing at what it can no longer see"
    );
}

/// A seated hero ordered at an enemy standing in plain sight `apart` away.
fn hero_ordered_at_an_enemy(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5000 + apart, 5000),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    world.settle();
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    (world, hero, mark)
}

#[test]
fn an_attack_order_at_what_slipped_into_fog_walks_to_where_it_was_last_seen() {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(1000);
    let seen_at = world.transform.get(mark).expect("standing").pos;
    // It slips away through the fog, far past the hero's sight.
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(6000, 12000);
    }
    for _ in 0..240 {
        world.step();
        let now = world.transform.get(hero).expect("standing").pos;
        assert!(
            now.y.to_int() < 5400,
            "its path never bends after what its side cannot see: {now:?}"
        );
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(seen_at, Fixed::from_int(400)),
        "it walked to where the enemy was last seen: {stood:?}"
    );
}

#[test]
fn the_fight_rolls_onto_the_closest_when_the_ordered_target_falls() {
    let (mut world, hero, first) = hero_ordered_at_an_enemy(300);
    let second = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5570, 5150),
    );
    world.settle();
    for _ in 0..5 {
        world.step();
    }
    // The one it was set on falls to a blow.
    world.push_hit(Some(hero), first, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(first), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::AttackMove { .. })
        ),
        "the order degrades to fighting from where it fell"
    );
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(second),
        "and the fight carries itself onto the closest"
    );
}

#[test]
fn an_attack_order_whose_target_fell_keeps_the_hero_fighting_from_the_spot() {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(300);
    for _ in 0..5 {
        world.step();
    }
    world.push_hit(Some(hero), mark, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::AttackMove { .. })
        ),
        "the order degrades to fighting from where it fell"
    );
    for _ in 0..60 {
        world.step();
    }
    assert_eq!(world.target_of(hero), None, "with nobody around it waits");
    // The next one to come into acquisition is taken on unasked.
    let next = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5500, 5100),
    );
    world.settle();
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(next),
        "the auto attack carries on"
    );
}

/// The same seat and enemy, with a follow order in place of the attack.
fn hero_following_an_enemy(apart: i32) -> (World, Entity, Entity) {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(apart);
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    (world, hero, mark)
}

#[test]
fn a_follow_at_an_enemy_closes_and_never_swings() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let full = world.health.get(mark).expect("standing").hp;
    for _ in 0..240 {
        world.step();
        assert_eq!(world.target_of(hero), None, "a follow takes nothing on");
    }
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "the one followed was never struck"
    );
    let stood = world.transform.get(hero).expect("standing").pos;
    let theirs = world.transform.get(mark).expect("standing").pos;
    assert!(
        stood.within(theirs, Fixed::from_int(200)),
        "the follower closed until the bodies met: {stood:?}"
    );
}

#[test]
fn a_follow_at_what_slipped_into_fog_walks_to_where_it_was_last_seen() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let seen_at = world.transform.get(mark).expect("standing").pos;
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(6000, 12000);
    }
    for _ in 0..240 {
        world.step();
        let now = world.transform.get(hero).expect("standing").pos;
        assert!(
            now.y.to_int() < 5400,
            "its path never bends after what its side cannot see: {now:?}"
        );
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(seen_at, Fixed::from_int(400)),
        "it walked to where the one followed was last seen: {stood:?}"
    );
}

#[test]
fn a_follow_ends_where_the_one_followed_fell() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let theirs = world.transform.get(mark).expect("standing").pos;
    world.push_hit(None, mark, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::Move { .. })
        ),
        "the follow became a walk to where it fell"
    );
    for _ in 0..180 {
        world.step();
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(theirs, Fixed::from_int(400)),
        "and the walk ends there: {stood:?}"
    );
}

/// A hero, an enemy standing in its way, and the order it was given.
fn hero_past_an_enemy(order: crate::game::UnitOrder) -> (World, Entity, Entity) {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let enemy = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    world.settle();
    world.seats[0].unit = Some(hero);
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: order,
            cooldown: 0,
        },
    );
    (world, hero, enemy)
}

#[test]
fn a_hero_told_to_walk_walks_past_what_it_meets() {
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::Move {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
    });
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        None,
        "walking somewhere, it takes on nothing"
    );
    assert_eq!(
        world.health.get(enemy).expect("standing").hp,
        was,
        "and strikes nothing"
    );
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(
        now.x.to_int() > 6000,
        "it kept walking where it was sent: {now:?}"
    );
}

#[test]
fn a_hero_told_to_walk_and_attack_stops_for_what_it_meets() {
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::AttackMove {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
    });
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(world.target_of(hero), Some(enemy), "it took on what it met");
    assert!(
        world.health.get(enemy).expect("standing").hp < was,
        "and struck it"
    );
}

#[test]
fn a_hero_holding_comes_round_but_never_leaves_the_spot() {
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::Hold);
    // Out of reach, so anything that walked would walk.
    if let Some(at) = world.transform.get_mut(enemy) {
        at.pos = bota_proto::Vec2::from_ints(5500, 5000);
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    for _ in 0..60 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        Some(enemy),
        "holding, it still takes on what comes near"
    );
    assert_eq!(
        world.transform.get(hero).expect("standing").pos,
        stood,
        "but it does not go after it"
    );
}

#[test]
fn a_hero_told_to_stop_stands_and_takes_on_nothing() {
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::AttackMove {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
    });
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(enemy),
        "walking to attack, it took the enemy on"
    );
    // The stop key.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert_eq!(
        world.target_of(hero),
        None,
        "stopped, it gives up what it was on"
    );
    let stood = world.transform.get(hero).expect("standing").pos;
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(world.target_of(hero), None, "and takes on nothing more");
    assert_eq!(
        world.health.get(enemy).expect("standing").hp,
        was,
        "so it strikes nothing"
    );
    assert_eq!(
        world.transform.get(hero).expect("standing").pos,
        stood,
        "and keeps the ground it was left on"
    );
}

/// A hero of a side and one of its own creeps beside it.
fn hero_and_own_creep() -> (World, Entity, Entity) {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    let own = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(own);
    (world, hero, own)
}

#[test]
fn one_of_your_own_at_full_health_cannot_be_struck() {
    let (mut world, hero, own) = hero_and_own_creep();
    assert!(
        !world.may_attack_on_order(hero, own),
        "a creep at full health is nobody to strike"
    );
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(own)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(()),
        "the order may still be given: it is how creeps are shaken off"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
    let was = world.health.get(own).expect("standing").hp;
    for _ in 0..90 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        None,
        "it takes on nothing of its own"
    );
    assert_eq!(
        world.health.get(own).expect("standing").hp,
        was,
        "and strikes nothing"
    );
}

#[test]
fn one_of_your_own_worn_down_far_enough_may_be_put_out() {
    let (mut world, hero, own) = hero_and_own_creep();
    let max = world.stats.get(own).expect("settled").max_hp;
    // A shade under half of what it can hold.
    world.health.insert(
        own,
        Health {
            hp: Fixed {
                raw: max.raw * 49 / 100,
            },
        },
    );
    assert!(
        world.may_attack_on_order(hero, own),
        "worn down, it may be put out"
    );
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(own)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
    assert_eq!(world.target_of(hero), Some(own), "it takes it on");
    let was = world.health.get(own).expect("standing").hp;
    for _ in 0..90 {
        world.step();
    }
    let now = world
        .health
        .get(own)
        .map_or(Fixed::ZERO, |health| health.hp);
    assert!(now < was, "and strikes it");
}

#[test]
fn a_deny_is_given_up_when_the_creep_is_no_longer_worn_down() {
    let (mut world, hero, own) = hero_and_own_creep();
    let max = world.stats.get(own).expect("settled").max_hp;
    world.health.insert(
        own,
        Health {
            hp: Fixed {
                raw: max.raw * 49 / 100,
            },
        },
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(own)),
        },
    }]);
    assert_eq!(world.target_of(hero), Some(own));
    // Mended back over the line.
    world.health.insert(own, Health { hp: max });
    world.step();
    assert_eq!(
        world.target_of(hero),
        None,
        "back on its feet, it is nobody to strike again"
    );
}

#[test]
fn your_own_building_goes_only_at_a_tenth() {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        Team::Radiant,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(tower);
    let max = world.stats.get(tower).expect("settled").max_hp;
    assert!(
        !world.may_attack_on_order(hero, tower),
        "standing tall, it is nobody to strike"
    );
    // A tenth is still too much; a shade under is not.
    world.health.insert(
        tower,
        Health {
            hp: Fixed { raw: max.raw / 10 },
        },
    );
    assert!(
        !world.may_attack_on_order(hero, tower),
        "exactly a tenth is not below a tenth"
    );
    world.health.insert(
        tower,
        Health {
            hp: Fixed {
                raw: max.raw * 9 / 100,
            },
        },
    );
    assert!(
        world.may_attack_on_order(hero, tower),
        "worn past it, it may be put out"
    );
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(tower)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
    assert_eq!(world.target_of(hero), Some(tower));
}

#[test]
fn your_own_hero_is_never_struck_however_worn_down() {
    let mut world = World::new();
    let mine = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let theirs = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.health.insert(theirs, Health { hp: Fixed::ONE });
    assert!(
        !world.may_attack_on_order(mine, theirs),
        "one of your own heroes is nobody to strike, worn down or not"
    );
}

#[test]
fn a_creep_prefers_a_creep_to_a_hero_that_is_doing_nothing() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    // The hero stands nearer than the enemy creep.
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    world.settle();
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600)),
        Some(other),
        "what it is doing outranks how near it stands"
    );
    let _ = hero;
}

#[test]
fn a_hero_laying_into_your_side_counts_for_no_more_than_a_creep() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let friend = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5050, 5000),
    );
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.set_order(hero, crate::game::UnitOrder::Stand);
    assert_eq!(
        world.threat_priority(creep, hero),
        2,
        "doing nothing to this side, it comes after a plain unit"
    );
    // The hero lays into one of ours.
    world.set_order(
        hero,
        crate::game::UnitOrder::Attack {
            target: friend,
            last_seen: bota_proto::Vec2::from_ints(5050, 5000),
        },
    );
    assert_eq!(
        world.threat_priority(creep, hero),
        world.threat_priority(creep, other),
        "laying into this side, it counts the same as a creep"
    );
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600)),
        Some(other),
        "so the nearer of the two wins, and that is the creep"
    );
    // With the hero the nearer of the two, it is the one taken.
    if let Some(at) = world.transform.get_mut(hero) {
        at.pos = bota_proto::Vec2::from_ints(5040, 5000);
    }
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600)),
        Some(hero),
        "nearness decides between equals"
    );
}

#[test]
fn a_hero_putting_out_its_own_is_taken_on_last() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let theirs = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Attack {
                target: theirs,
                last_seen: bota_proto::Vec2::from_ints(5400, 5000),
            },
            cooldown: 0,
        },
    );
    assert_eq!(
        world.threat_priority(creep, hero),
        3,
        "putting out its own puts it last"
    );
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600)),
        Some(theirs),
        "so the creep it was denying is taken on instead"
    );
}

#[test]
fn what_is_in_reach_is_kept_unless_a_better_class_is_also_in_reach() {
    let mut world = World::new();
    let siege = world.spawn_unit(
        &crate::game::SIEGE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
    );
    world.lane_ai.insert(
        siege,
        crate::game::LaneAi {
            anchor: None,
            last_seen: None,
            keep_until: 0,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.set_target(siege, creep);
    assert_eq!(
        world.select_target(siege),
        Some(tower),
        "a siege creep turns from a unit to the building it prefers"
    );
}

/// A hero of one side, a creep of the other with a mind of its own, one of the
/// hero's own creeps for that creep to prefer, and an enemy hero to point at.
///
/// The enemy creep stands `apart` from the hero. Only an order at the enemy
/// hero calls creeps on, so that is what the pull tests click.
fn a_lane_with_a_hero(apart: i32) -> (World, Entity, Entity, Entity, Entity) {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    // Standing, so it takes nothing on of its own and is no threat to anybody
    // until it is told to be.
    world.set_order(hero, crate::game::UnitOrder::Stand);
    let theirs = thinking_creep_of(
        &mut world,
        Team::Dire,
        bota_proto::Vec2::from_ints(5000 + apart, 5000),
    );
    // One of the hero's own, standing right by the enemy creep, which is what
    // that creep would rather be fighting.
    let ours = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000 + apart + 60, 5000),
    );
    // Somebody worth pointing at: a last hit on a creep moves nobody.
    let foe = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5000 + apart + 200, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.set_order(foe, crate::game::UnitOrder::Stand);
    world.settle();
    world.step();
    (world, hero, theirs, ours, foe)
}

/// A creep of a side with a mind of its own.
fn thinking_creep_of(world: &mut World, team: Team, at: bota_proto::Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.lane_ai.insert(
        creep,
        crate::game::LaneAi {
            anchor: None,
            last_seen: None,
            keep_until: 0,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    creep
}

/// The order a player gives by clicking attack on somebody.
fn attack_click(world: &mut World, on: Entity) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(on)),
        },
    }]);
}

#[test]
fn attacking_an_enemy_pulls_the_creeps_near_you_onto_you() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "left alone, the creep fights the creep"
    );
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the click pulls it onto whoever gave it"
    );
}

#[test]
fn attacking_an_enemy_creep_pulls_nobody() {
    let (mut world, _hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Clicking the enemy creep is a last hit, and a last hit is not a call.
    attack_click(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "the creep goes on fighting what it was fighting"
    );
}

#[test]
fn a_creep_too_far_off_pays_the_order_no_mind() {
    // Past a melee creep's acquisition of 500.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(700);
    assert_eq!(world.target_of(theirs), Some(ours));
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "it never saw the order given"
    );
    let _ = hero;
}

#[test]
fn the_hold_lets_go_after_two_and_a_third_seconds() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero));
    // The hero stops, so it is no longer laying into that side.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS - 4 {
        world.step();
        // Held where they were put, so only the clock decides.
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the hold has not run out yet"
    );
    for _ in 0..6 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "past it, the ranking takes the creep back"
    );
}

#[test]
fn a_second_click_inside_the_wait_pulls_nothing() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero));
    // Stop, wait out the hold, and click again while the wait still runs.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS + 2 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(world.target_of(theirs), Some(ours), "it let go");
    assert!(
        world
            .orders
            .get(theirs)
            .is_some_and(|orders| orders.cooldown > 0),
        "and the wait before it answers again still runs"
    );
    attack_click(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "so a second click inside that wait pulls nothing"
    );
}

#[test]
fn clicking_your_own_does_not_pull_the_creeps_onto_you() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    let _ = foe;
    // Worn down far enough to be worth denying.
    let max = world.stats.get(ours).expect("settled").max_hp;
    world.health.insert(
        ours,
        Health {
            hp: Fixed {
                raw: max.raw * 40 / 100,
            },
        },
    );
    attack_click(&mut world, ours);
    assert_eq!(
        world.target_of(hero),
        Some(ours),
        "the hero does go for the deny"
    );
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "but the creep is not pulled onto the one who clicked"
    );
}

#[test]
fn a_hold_is_not_broken_by_clicking_your_own() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero), "pulled onto the hero");
    // Straight away, click one of your own: the hold does not give.
    attack_click(&mut world, ours);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "what pulled it keeps it for the whole span"
    );
    assert_eq!(
        world.target_of(hero),
        None,
        "and the hero strikes nothing of its own"
    );
    // Once the hold is out, the ranking takes the creep back on its own.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS + 2 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "and lets go in its time"
    );
}

#[test]
fn an_attack_order_at_one_of_your_own_walks_you_to_it_and_waits() {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    // Well out of reach, and at full health so it cannot be put out yet.
    let ours = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(6500, 5000),
    );
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(ours);
    attack_click(&mut world, ours);
    let start = world.transform.get(hero).expect("standing").pos;
    for _ in 0..90 {
        world.step();
    }
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(
        now.x > start.x,
        "it walks to what it was pointed at: {now:?} from {start:?}"
    );
    // Right up to it, not merely into reach: it has nothing to do from reach.
    let hulls = world.hull.get(hero).expect("has one").radius
        + world.hull.get(ours).expect("has one").radius;
    let apart = crate::game::isqrt64(
        now.distance_squared(world.transform.get(ours).expect("standing").pos),
    );
    assert!(
        apart < i64::from(rules::units(rules::HERO_ATTACK_RANGE).raw),
        "it came nearer than its reach: {apart}"
    );
    assert!(
        apart >= i64::from(hulls.raw) - i64::from(rules::units(4).raw),
        "and no nearer than the bodies allow"
    );
    assert_eq!(
        world.target_of(hero),
        None,
        "but takes nothing on while it cannot be struck"
    );
    let full = world.health.get(ours).expect("standing").hp;
    // Worn down past half, it may be put out after all.
    let max = world.stats.get(ours).expect("settled").max_hp;
    world.health.insert(
        ours,
        Health {
            hp: Fixed {
                raw: max.raw * 40 / 100,
            },
        },
    );
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        Some(ours),
        "and strikes the moment it may"
    );
    assert!(
        world.health.get(ours).map_or(Fixed::ZERO, |h| h.hp) < full,
        "it did put damage on it"
    );
}

/// How far apart two entities stand, along the lane.
fn gap_along_lane(world: &World, one: Entity, other: Entity) -> i32 {
    let at = |entity| {
        world
            .transform
            .get(entity)
            .expect("standing")
            .pos
            .x
            .to_int()
    };
    (at(one) - at(other)).abs()
}

#[test]
fn a_swing_costs_the_swinger_the_ground_it_stands_on() {
    let (mut world, hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Nothing else for it to fight, so it takes the hero and keeps after it.
    world.despawn(ours);
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(5280, 5000);
    attack_click(&mut world, theirs);
    world.advance(&[]);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the creep takes the one in front of it"
    );
    let before = gap_along_lane(&world, hero, theirs);
    for _ in 0..60 {
        world.advance(&[crate::game::Command {
            slot: bota_proto::SlotId(0),
            unit: None,
            order: bota_proto::Order::Move {
                target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(3000, 5000)),
            },
        }]);
    }
    let after = gap_along_lane(&world, hero, theirs);
    // The creep is the faster of the two: only the ticks it spends rooted in
    // its swings let the hero pull away at all.
    assert!(
        after > before + 200,
        "the hero should be pulling away: {before} then {after}"
    );
}

#[test]
fn an_order_to_break_off_gives_up_a_swing_that_has_not_landed() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..60 {
        if world
            .attacking
            .get(hero)
            .is_some_and(|state| state.windup.is_some())
        {
            break;
        }
        world.advance(&[]);
    }
    assert!(
        world
            .attacking
            .get(hero)
            .is_some_and(|state| state.windup.is_some()),
        "the hero should be mid-swing by now"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    let state = world.attacking.get(hero).copied().expect("attacking");
    assert!(state.windup.is_none(), "the swing is given up");
    assert_eq!(state.cooldown, 0, "and nothing was spent on it");
}

#[test]
fn an_order_after_a_swing_lands_does_not_hurry_the_next_one() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..120 {
        if world
            .attacking
            .get(hero)
            .is_some_and(|state| state.recovering > 0)
        {
            break;
        }
        world.advance(&[]);
    }
    let before = world
        .attacking
        .get(hero)
        .copied()
        .expect("attacking")
        .cooldown;
    assert!(
        before > 0,
        "the swing that landed spent the wait for the next"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    let state = world.attacking.get(hero).copied().expect("attacking");
    assert_eq!(state.recovering, 0, "the recovery is cancelled");
    assert_eq!(
        state.cooldown,
        before - 1,
        "but the wait for the next swing runs on"
    );
}

#[test]
fn a_hero_stands_up_beside_its_fountain_and_not_in_it() {
    for id in [0u16, 1] {
        let map = crate::game::map_of(bota_proto::MapId(id));
        let world = World::on_map(map);
        for team in [bota_proto::Team::Radiant, bota_proto::Team::Dire] {
            let at = crate::game::hero_spawn_pos(map, team);
            assert!(
                world.grid.walkable(at),
                "map {id}, {team:?}: a hero stands up on ground it can walk off"
            );
        }
    }
}

#[test]
fn a_hero_stands_up_beside_its_fountain_at_the_start_of_a_match() {
    let world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(crate::game::hero_spawn_pos(
            world.map,
            bota_proto::Team::Radiant
        )),
        "beside the fountain rather than inside it"
    );
}

/// The effect a fountain hands out.
const FOUNTAIN_EFFECT: crate::game::StatusKind = crate::game::StatusKind::Fountain {
    hp_per_tick: rules::FOUNTAIN_HEAL_HP_PER_TICK * 100,
    mana_per_tick: rules::FOUNTAIN_HEAL_MANA_PER_TICK * 100,
};

/// Whether a unit carries an effect of one kind right now, whatever there is
/// of it.
fn carries(world: &World, entity: Entity, kind: crate::game::StatusKind) -> bool {
    let same = std::mem::discriminant(&kind);
    world.statuses.get(entity).is_some_and(|on_it| {
        on_it
            .active()
            .any(|status| std::mem::discriminant(&status.kind) == same)
    })
}

#[test]
fn a_fountain_hands_out_mending_to_whoever_stands_in_it() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.health.insert(
        hero,
        Health {
            hp: Fixed::from_int(100),
        },
    );
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "standing in it, the hero carries the effect"
    );
    assert!(
        world.health.get(hero).map(|h| h.hp)
            >= Some(Fixed::from_int(100 + rules::FOUNTAIN_HEAL_HP_PER_TICK)),
        "and mends by at least what the fountain hands out"
    );
    // Walked out of reach, it runs out on its own: nothing takes it off, and
    // what it has left is what the fountain last handed it.
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(9600, 9216);
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "one step out it is still running"
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        !carries(&world, hero, FOUNTAIN_EFFECT),
        "a second later it is gone"
    );
}

#[test]
fn a_fountain_hands_out_nothing_to_the_other_side() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        world.transform.get(hero).expect("standing").pos,
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.step();
    assert!(
        !carries(&world, theirs, FOUNTAIN_EFFECT),
        "an enemy standing in it mends no faster for it"
    );
}

#[test]
fn what_a_hero_bought_is_in_the_view_its_side_is_sent() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), boots, &mut events),
        "standing at its own shop, it may buy"
    );
    let view = world.view(bota_proto::Team::Radiant);
    let mine = view
        .units
        .iter()
        .find(|unit| unit.id == crate::game::wire_id(hero))
        .expect("its own hero is in its own view");
    assert_eq!(
        mine.items.first().and_then(|slot| slot.map(|item| item.id)),
        Some(boots),
        "and what it bought is in the bag it is sent"
    );
    assert_eq!(
        mine.items.len(),
        rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS,
        "every slot keeps its place, held or not"
    );
    let seat = view
        .players
        .iter()
        .find(|player| player.slot == bota_proto::SlotId(0))
        .expect("its own seat");
    assert_eq!(
        seat.stash.as_ref().map(|stash| stash.len()),
        Some(rules::STASH_SLOTS),
        "and the stash is sent with its slots"
    );
}

/// A match world with one hero standing at its own shop, an item in its stash.
fn a_hero_at_the_shop() -> (World, Entity, bota_proto::ItemId) {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    (world, hero, boots)
}

#[test]
fn what_waits_in_the_stash_can_be_taken_into_the_bag() {
    let (mut world, hero, boots) = a_hero_at_the_shop();
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "at its own shop the stash takes part"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|stack| stack.id),
        Some(boots),
        "and the item is in hand"
    );
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "with nothing left behind it"
    );
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(
            crate::game::HERO.move_speed + 45
        )),
        "and it works at once, being no backpack it came from"
    );
}

#[test]
fn the_backpack_takes_from_the_stash_too() {
    let (mut world, hero, boots) = a_hero_at_the_shop();
    let pocket = rules::INVENTORY_SLOTS;
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, pocket),
        "the pocket is a place like any other"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[pocket].map(|stack| stack.id),
        Some(boots),
        "and holds it"
    );
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(crate::game::HERO.move_speed)),
        "carried inert, it adds nothing"
    );
    // Out of the pocket into the inventory, it waits before it works.
    assert!(world.move_item(bota_proto::SlotId(0), hero, pocket, 0));
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(crate::game::HERO.move_speed)),
        "just out of the pocket it is still inert"
    );
    for _ in 0..rules::BACKPACK_MUTE_TICKS {
        world.step();
    }
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(
            crate::game::HERO.move_speed + 45
        )),
        "and works once the wait is out"
    );
}

#[test]
fn the_stash_is_out_of_reach_away_from_the_shop() {
    let (mut world, hero, _boots) = a_hero_at_the_shop();
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(9600, 9216);
    assert!(
        !world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "out in the lane the stash cannot be reached into"
    );
}

#[test]
fn selling_soon_after_buying_pays_the_whole_price_back() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let purse = world.seats[0].gold;
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let mut events = Vec::new();
    assert!(world.buy(bota_proto::SlotId(0), boots, &mut events));
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "and sells it back"
    );
    assert_eq!(world.seats[0].gold, purse, "nothing was lost on it");
}

/// Walkable ground out in the open: nothing standing near it, and the
/// ground clear for a short walk in every direction it is put to.
fn an_empty_spot(world: &World) -> bota_proto::Vec2 {
    for row in 0..16 {
        for step in 0..60 {
            let at = bota_proto::Vec2::from_ints(5600 + step * 100, 6000 + row * 200);
            let clear = world.entities.iter().all(|entity| {
                !world
                    .transform
                    .get(entity)
                    .is_some_and(|t| t.pos.within(at, bota_proto::Fixed::from_int(800)))
            });
            let open = (-2..=14).all(|dx: i32| {
                (-2..=2).all(|dy: i32| {
                    world
                        .grid
                        .walkable(at + bota_proto::Vec2::from_ints(dx * 50, dy * 100))
                })
            });
            if clear && open {
                return at;
            }
        }
    }
    panic!("the map has room somewhere")
}

/// A hero out in the lane with a scroll in its first slot.
fn a_hero_with_a_scroll() -> (World, Entity) {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.transform.get_mut(hero).expect("hero").pos = an_empty_spot(&world);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_TOWN_PORTAL_SCROLL),
            charges: 1,
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
    (world, hero)
}

/// Where the scroll is aimed: beside a tower of one's own.
fn beside_own_tower(world: &World) -> bota_proto::Vec2 {
    let tower = world
        .entities
        .iter()
        .find(|entity| {
            world.kind.get(*entity) == Some(&bota_proto::UnitKind::Tower)
                && world.team.get(*entity) == Some(&bota_proto::Team::Radiant)
        })
        .expect("its side has towers");
    let at = world.transform.get(tower).expect("standing").pos;
    let beside = at + bota_proto::Vec2::from_ints(300, 0);
    if world.grid.walkable(beside) {
        beside
    } else {
        at + bota_proto::Vec2::from_ints(-300, 0)
    }
}

#[test]
fn a_scroll_carries_its_user_once_the_channel_runs_out() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let from = world.transform.get(hero).expect("standing").pos;
    // Told to walk somewhere before it read the scroll: the order stays
    // behind with the spot it was given in.
    world.set_order(
        hero,
        crate::game::UnitOrder::Move {
            pos: bota_proto::Vec2::from_ints(8000, 12000),
        },
    );
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(to)),
        "beside a building of its own it may go"
    );
    assert!(world.is_channelling(hero), "and stands through the channel");
    world.step();
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(from),
        "still where it was while it channels"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and the scroll is not spent yet"
    );
    for _ in 0..90 {
        world.step();
    }
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(to),
        "then it is there"
    );
    for _ in 0..30 {
        world.step();
    }
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(to),
        "and stays there rather than walking back to what it was told"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "and the scroll went with it"
    );
}

#[test]
fn a_scroll_aimed_where_nothing_of_its_own_stands_does_nothing() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let nowhere = bota_proto::Vec2::from_ints(14000, 9216);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(nowhere)),
        "the middle of the map is nothing to go to"
    );
    assert!(!world.is_channelling(hero));
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on it"
    );
}

#[test]
fn an_order_takes_a_channel_away_and_leaves_the_scroll() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to)));
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert!(!world.is_channelling(hero), "the order took it away");
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and the scroll is still there to use again"
    );
}

#[test]
fn business_with_the_bag_and_the_shop_takes_no_channel_away() {
    let (mut world, hero) = a_hero_with_a_scroll();
    world.seats[0].gold = 5000;
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to)));
    let asks = [
        bota_proto::Order::Buy {
            item: bota_proto::ItemId(crate::game::ITEM_BOOTS),
        },
        bota_proto::Order::Learn {
            slot: bota_proto::AbilitySlot(0),
        },
        bota_proto::Order::Sell {
            slot: bota_proto::ItemSlot(9),
        },
    ];
    for order in asks {
        world.advance(&[crate::game::Command {
            slot: bota_proto::SlotId(0),
            unit: None,
            order,
        }]);
        assert!(
            world.is_channelling(hero),
            "the channel stands through {order:?}"
        );
    }
}

/// A hero out in the open with one ward of a kind in its first slot.
fn a_hero_with_a_ward(item: u16) -> (World, Entity, bota_proto::Vec2) {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(item),
            charges: 1,
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
    (world, hero, spot)
}

/// The one ward standing on the map.
fn the_ward(world: &World) -> Entity {
    world
        .entities
        .iter()
        .find(|entity| world.kind.get(*entity) == Some(&bota_proto::UnitKind::Ward))
        .expect("a ward stands")
}

#[test]
fn a_ward_stands_where_it_was_put_and_goes_when_its_time_is_up() {
    let (mut world, hero, spot) = a_hero_with_a_ward(crate::game::ITEM_OBSERVER_WARD);
    let at = spot + bota_proto::Vec2::from_ints(200, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(at)),
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
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(at)));
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
        !world.use_item(hero, 0, bota_proto::Target::Pos(far)),
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
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(ahead)));
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
        .filter(|at| !world.grid.walkable(*at))
        .find_map(|wall| {
            let beside = wall + bota_proto::Vec2::from_ints(0, 300);
            world.grid.walkable(beside).then_some((beside, wall))
        })
        .expect("the map has walls with room beside them");
    world.transform.get_mut(hero).expect("hero").pos = stand;
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(wall)),
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
        assert!(world.use_item(hero, 0, bota_proto::Target::Pos(at)));
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

#[test]
fn a_drink_may_be_aimed_at_the_one_drinking_it() {
    for item in [crate::game::ITEM_HEALING_SALVE, crate::game::ITEM_CLARITY] {
        let mut world = World::for_match(&config(), config().rng());
        let hero = world.seats[0].unit.expect("stood up");
        if let Some(bag) = world.inventory.get_mut(hero) {
            bag.slots[0] = Some(crate::game::ItemStack {
                id: bota_proto::ItemId(item),
                charges: 1,
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
        // Aimed at itself by name, the way a click on one's own hero sends it.
        assert!(
            world.use_item(
                hero,
                0,
                bota_proto::Target::Unit(crate::game::wire_id(hero))
            ),
            "item {item} may be drunk by whoever holds it"
        );
        assert!(
            world.statuses.get(hero).is_some_and(|on_it| on_it
                .active()
                .any(|status| !matches!(status.kind, crate::game::StatusKind::Fountain { .. }))),
            "item {item} leaves its effect on the one who drank it"
        );
    }
}

/// A hero on the big map standing beside the forest, with one item in hand.
fn a_hero_by_the_trees(item: u16, charges: u8) -> (World, Entity, bota_proto::Vec2) {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let tree = crate::game::tree_positions(map)
        .into_iter()
        .find(|at| {
            world
                .grid
                .walkable(*at + bota_proto::Vec2::from_ints(120, 0))
                && world
                    .grid
                    .walkable(*at + bota_proto::Vec2::from_ints(240, 0))
        })
        .expect("some tree has open ground beside it");
    let stand = tree + bota_proto::Vec2::from_ints(120, 0);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        stand,
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
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(item),
            charges,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    world.settle();
    world.step();
    (world, hero, tree)
}

/// Whether the cell a spot falls in still stops a sight line.
fn sight_stopped_at(world: &World, at: bota_proto::Vec2) -> bool {
    crate::game::PassGrid::cell_of(at).is_some_and(|(cx, cy)| !world.sight_block.cell_open(cx, cy))
}

#[test]
fn a_quelling_blade_takes_a_tree_down_and_the_tree_comes_back() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    assert!(sight_stopped_at(&world, tree), "the tree stops a look");
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(tree)),
        "the blade reaches it"
    );
    assert_eq!(world.trees.felled().count(), 1, "one tree is down");
    assert!(
        !sight_stopped_at(&world, tree),
        "and what it stopped is stopped no longer"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "the blade itself is not spent on it"
    );
    for _ in 0..rules::TREE_REGROW_TICKS + 1 {
        world.step();
    }
    assert_eq!(world.trees.felled().count(), 0, "in time it comes back");
    assert!(sight_stopped_at(&world, tree), "and stops a look again");
}

#[test]
fn a_tango_eats_a_tree_and_without_one_eats_nothing() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_TANGO, 3);
    let far = tree + bota_proto::Vec2::from_ints(4000, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(far)),
        "aimed where no tree stands it does nothing"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|s| s.charges),
        Some(3),
        "and spends no charge on it"
    );
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(tree)));
    assert_eq!(world.trees.felled().count(), 1, "the tree it ate is gone");
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|s| s.charges),
        Some(2),
        "and one charge with it"
    );
    assert!(
        carries(
            &world,
            hero,
            crate::game::StatusKind::Mending {
                per_tick: 0,
                breaks: false
            }
        ),
        "the one who ate it mends"
    );
}

#[test]
fn a_branch_puts_a_tree_up_and_eating_that_one_feeds_twice_as_long() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_IRON_BRANCH, 1);
    let spot = tree + bota_proto::Vec2::from_ints(240, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(spot)),
        "the branch goes into open ground"
    );
    assert_eq!(world.trees.planted().len(), 1, "and a tree stands there");
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the branch is spent on it"
    );
    assert!(
        sight_stopped_at(&world, spot),
        "what it put up stops a look"
    );
    // The same tango, eaten off the map's own tree and off a planted one.
    let plain = tango_ticks(&mut world, hero, tree);
    let put_up = tango_ticks(&mut world, hero, spot);
    assert_eq!(put_up, plain * 2, "a tree put up feeds twice as long");
    // What is left of it goes on its own.
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_IRON_BRANCH, 1);
    let spot = tree + bota_proto::Vec2::from_ints(240, 0);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(spot)));
    for _ in 0..rules::PLANTED_TREE_TICKS + 1 {
        world.step();
    }
    assert!(world.trees.planted().is_empty(), "in time it goes");
    assert!(!sight_stopped_at(&world, spot), "and stops nothing");
}

/// How long a tango eaten off the tree at a spot mends for.
fn tango_ticks(world: &mut World, hero: Entity, at: bota_proto::Vec2) -> u32 {
    world.statuses.remove(hero);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_TANGO),
            charges: 1,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    assert!(world.use_item(hero, 1, bota_proto::Target::Pos(at)));
    world
        .statuses
        .get(hero)
        .expect("it mends")
        .active()
        .find(|status| matches!(status.kind, crate::game::StatusKind::Mending { .. }))
        .expect("of health")
        .ticks_left
}

#[test]
fn a_blade_takes_the_tree_it_was_pointed_at_and_no_other() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    // Open ground a step off the trunk, still well inside the blade's reach.
    let beside = tree + bota_proto::Vec2::from_ints(rules::TREE_RADIUS + 20, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(beside)),
        "pointed at ground beside a tree it takes nothing"
    );
    assert_eq!(world.trees.felled().count(), 0);
    let on_it = tree + bota_proto::Vec2::from_ints(rules::TREE_RADIUS - 10, 0);
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(on_it)),
        "pointed at the trunk it takes that tree"
    );
    assert_eq!(world.trees.felled().count(), 1);
}

#[test]
fn a_blade_cannot_reach_a_tree_it_was_pointed_at_from_far_off() {
    let (mut world, hero, tree) = a_hero_by_the_trees(crate::game::ITEM_QUELLING_BLADE, 0);
    world.transform.get_mut(hero).expect("hero").pos = tree + bota_proto::Vec2::from_ints(2000, 0);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(tree)),
        "the tree is the one pointed at, but it is out of reach"
    );
    assert_eq!(world.trees.felled().count(), 0);
}

/// What one swing of an entity takes off another, run to the blow itself.
fn one_swing_takes(world: &mut World, from: Entity, on: Entity) -> i32 {
    world.set_target(from, on);
    if let Some(state) = world.attacking.get_mut(from) {
        state.cooldown = 0;
        state.windup = None;
        state.recovering = 0;
    }
    let before = world.health.get(on).expect("standing").hp;
    for _ in 0..120 {
        world.step();
        let now = world.health.get(on).expect("standing").hp;
        if now < before {
            return (before - now).to_int();
        }
    }
    panic!("the swing never landed")
}

#[test]
fn a_quelling_blade_is_worth_something_against_a_creep_and_nothing_against_a_hero() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000, 5100),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    let bare_creep = one_swing_takes(&mut world, hero, creep);
    let bare_hero = one_swing_takes(&mut world, hero, theirs);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_QUELLING_BLADE),
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
    let blade = crate::game::ITEMS[usize::from(crate::game::ITEM_QUELLING_BLADE)]
        .carried
        .damage_to_creeps;
    assert_eq!(
        world.stats.get(hero).map(|s| s.damage_to_creeps),
        Some(blade),
        "what it carries against creeps is worked out"
    );
    let with_creep = one_swing_takes(&mut world, hero, creep);
    let with_hero = one_swing_takes(&mut world, hero, theirs);
    // Armor takes its share of the blade as it does of everything else, so
    // what lands is somewhere between one point and the whole of it.
    let felt = with_creep - bare_creep;
    assert!(
        (1..=blade).contains(&felt),
        "the creep feels the blade: {bare_creep} then {with_creep}, of {blade} carried"
    );
    // A hero mends while it is being measured, so what it took can read a
    // point light; what it must not do is read heavier.
    assert!(
        with_hero <= bare_hero,
        "the hero feels none of it: {bare_hero} then {with_hero}"
    );
}

/// Puts an effect on a unit for a while.
fn put_on(world: &mut World, entity: Entity, kind: crate::game::StatusKind, ticks: u32) {
    let mut on_it = world.statuses.remove(entity).unwrap_or_default();
    on_it.put(crate::game::Status {
        kind,
        ticks_left: ticks,
    });
    world.statuses.insert(entity, on_it);
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
    put_on(&mut world, hero, crate::game::StatusKind::Stunned, 60);
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
        crate::game::StatusKind::Slowed { pct: 25 },
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
        crate::game::StatusKind::Burning {
            amount: 5,
            kind: bota_proto::DamageKind::Pure,
            from: None,
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
        crate::game::StatusKind::Burning {
            amount: 100,
            kind: bota_proto::DamageKind::Pure,
            from: None,
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
fn a_point_is_spent_only_when_there_is_one_and_the_level_allows_it() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    world.settle();
    let mut events = Vec::new();
    assert!(world.learn(hero, 0, &mut events), "the first point goes in");
    assert!(
        !world.learn(hero, 1, &mut events),
        "and the second waits for a second level"
    );
    assert!(
        !world.learn(hero, 0, &mut events),
        "one ability twice over is no different"
    );
    // Levelled up, the point is there, but the ultimate still waits.
    world.level.insert(hero, crate::game::Level(2));
    assert!(
        !world.learn(hero, 3, &mut events),
        "the ultimate waits for the level it asks for"
    );
    assert!(world.learn(hero, 1, &mut events), "a basic one does not");
    // A passive takes points like anything else.
    world.level.insert(hero, crate::game::Level(3));
    assert!(
        world.learn(hero, 2, &mut events),
        "a passive is learned, not cast"
    );
    // Right up to the level it opens on, the ultimate answers.
    world
        .level
        .insert(hero, crate::game::Level(rules::ULT_LEVEL_FLOORS[0]));
    assert!(world.learn(hero, 3, &mut events), "and then it opens");
    // Nothing goes past its own cap, however many levels are had.
    world.level.insert(hero, crate::game::Level(50));
    for _ in 0..10 {
        world.learn(hero, 0, &mut events);
    }
    assert_eq!(
        world.abilities.get(hero).expect("casts").slots[0].level,
        rules::ABILITY_MAX_LEVEL,
        "the hook stops at its own cap"
    );
}

/// Pudge standing in the open with his hook learned, and an enemy creep a
/// way off in front of him.
fn pudge_and_a_mark(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + apart, 5000),
    );
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[0].level = 1;
    }
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(1),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(pudge);
    world.settle();
    world.step();
    (world, pudge, mark)
}

/// Sends Pudge's hook at a spot the way a player does.
fn throw_hook(world: &mut World, at: bota_proto::Vec2) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(0),
            target: bota_proto::Target::Pos(at),
        },
    }]);
}

#[test]
fn a_hook_catches_what_it_flies_into_and_drags_it_home() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    let full = world.health.get(mark).expect("standing").hp.to_int();
    let home = world.transform.get(pudge).expect("standing").pos;
    throw_hook(&mut world, bota_proto::Vec2::from_ints(6000, 5000));
    for _ in 0..120 {
        world.step();
        if world.entities.iter().all(|e| world.hook.get(e).is_none()) {
            break;
        }
    }
    let landed = world.transform.get(mark).expect("standing").pos;
    assert!(
        landed.within(home, bota_proto::Fixed::from_int(200)),
        "what it caught is dragged to the one who threw it: {landed:?} against {home:?}"
    );
    assert!(
        world.health.get(mark).expect("standing").hp.to_int() < full,
        "and an enemy feels it"
    );
}

#[test]
fn a_hook_that_catches_nothing_comes_back_by_itself() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    // Thrown the other way, so it flies out over open ground and returns.
    throw_hook(&mut world, bota_proto::Vec2::from_ints(4000, 5000));
    let (mut flew, mut caught) = (false, false);
    for _ in 0..200 {
        world.step();
        let mut flying = false;
        for entity in world.entities.iter() {
            if let Some(hook) = world.hook.get(entity) {
                flying = true;
                caught |= hook.caught.is_some();
            }
        }
        if flying {
            flew = true;
        } else if flew {
            break;
        }
    }
    assert!(flew, "it was thrown");
    assert!(!caught, "and caught nobody over open ground");
    assert!(
        world.entities.iter().all(|e| world.hook.get(e).is_none()),
        "and came back"
    );
    assert!(world.alive(pudge));
}

#[test]
fn a_hook_flies_no_further_than_it_reaches() {
    let (mut world, _pudge, _mark) = pudge_and_a_mark(4000);
    // Aimed at the very edge of its reach: further off than that the caster
    // walks in first, which is movement's business rather than the hook's.
    throw_hook(
        &mut world,
        bota_proto::Vec2::from_ints(5000 + rules::HOOK_RANGE, 5000),
    );
    let mut furthest = 0;
    for _ in 0..200 {
        world.step();
        for entity in world.entities.iter() {
            if world.hook.get(entity).is_some()
                && let Some(at) = world.transform.get(entity)
            {
                furthest = furthest.max(at.pos.x.to_int() - 5000);
            }
        }
    }
    assert!(
        furthest <= rules::HOOK_RANGE,
        "it stops at its own reach: {furthest} of {}",
        rules::HOOK_RANGE
    );
    assert!(
        furthest > rules::HOOK_RANGE - 100,
        "and gets there: {furthest}"
    );
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

/// Sends one of Pudge's abilities the way a player does.
fn pudge_casts(world: &mut World, slot: u8, target: bota_proto::Target) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(slot),
            target,
        },
    }]);
}

#[test]
fn the_rot_burns_and_slows_what_stands_in_it_and_lifts_when_switched_off() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[1].level = 1;
    }
    let full = world.health.get(mark).expect("standing").hp;
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert!(world.rotting.get(pudge).is_some(), "it is switched on");
    for _ in 0..rules::BURN_PERIOD_TICKS * 4 {
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "what stands in it burns"
    );
    assert!(
        carries(&world, mark, crate::game::StatusKind::Slowed { pct: 0 }),
        "and is slowed while it stands there"
    );
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert!(world.rotting.get(pudge).is_none(), "it is switched off");
    for _ in 0..3 {
        world.step();
    }
    assert!(
        !carries(&world, mark, crate::game::StatusKind::Slowed { pct: 0 }),
        "and nothing is left slowed"
    );
}

#[test]
fn the_rot_never_kills_the_one_carrying_it() {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[1].level = 3;
    }
    world.settle();
    world.step();
    world
        .rotting
        .insert(pudge, crate::game::Rotting { level: 3 });
    world.health.insert(
        pudge,
        Health {
            hp: Fixed::from_int(20),
        },
    );
    for _ in 0..rules::BURN_PERIOD_TICKS * 40 {
        world.step();
    }
    assert!(
        world.health.get(pudge).expect("standing").hp.to_int() < 20,
        "it does burn its owner"
    );
    assert!(world.alive(pudge), "but never kills him");
    assert!(
        world.health.get(pudge).expect("standing").hp >= Fixed::from_int(1),
        "and never takes his last point"
    );
}

#[test]
fn a_dismember_holds_what_it_eats_and_feeds_the_one_eating() {
    let (mut world, pudge, mark) = pudge_and_a_mark(100);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    world.health.insert(
        pudge,
        Health {
            hp: Fixed::from_int(300),
        },
    );
    world.step();
    let hurt = world.health.get(pudge).expect("standing").hp;
    let full = world.health.get(mark).expect("standing").hp;
    pudge_casts(
        &mut world,
        3,
        bota_proto::Target::Unit(crate::game::wire_id(mark)),
    );
    assert!(world.dismember.get(pudge).is_some(), "it takes hold");
    for _ in 0..30 {
        world.step();
    }
    assert!(
        carries(&world, mark, crate::game::StatusKind::Stunned),
        "what it holds cannot act"
    );
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "and is eaten"
    );
    assert!(
        world.health.get(pudge).expect("standing").hp > hurt,
        "while the one eating mends"
    );
    // An order of any kind lets go.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert!(world.dismember.get(pudge).is_none(), "an order lets go");
}

#[test]
fn a_flesh_heap_keeps_what_dies_near_it_and_the_keeping_outlives_a_death() {
    let (mut world, pudge, mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    world.step();
    let bare = world.stats.get(pudge).expect("settled").max_hp;
    let mut events = Vec::new();
    world.bury(vec![(mark, None)], &mut events);
    world.step();
    assert_eq!(
        world
            .stacks
            .get(pudge)
            .map(|kept| kept.of(StackKind::FleshHeap)),
        Some(1),
        "a death beside it feeds it"
    );
    assert_eq!(
        world.stats.get(pudge).map(|s| s.max_hp),
        Some(bare + Fixed::from_int(rules::FLESH_HEAP_HP)),
        "and one stack is worth its health"
    );
    assert!(
        world.stats.get(pudge).expect("settled").magic_resist_pct
            > crate::game::PUDGE.magic_resist_pct,
        "and knowing it at all holds magic off"
    );
}

/// Lays one item in a hero's first slot.
fn hand_item(world: &mut World, hero: Entity, item: u16, charges: u8) {
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(item),
            charges,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
}

#[test]
fn a_scroll_read_is_owed_by_the_hero_and_not_by_the_scroll() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(to)));
    for _ in 0..91 {
        world.step();
    }
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_none(),
        "the scroll went with the teleport"
    );
    // A fresh scroll, bought after the first was spent, is still on the wait.
    hand_item(&mut world, hero, crate::game::ITEM_TOWN_PORTAL_SCROLL, 1);
    let there = beside_own_tower(&world);
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::Pos(there)),
        "a new scroll does not buy a new wait"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").slots[0].is_some(),
        "and nothing was spent on the attempt"
    );
    for _ in 0..rules::SCROLL_WAIT_TICKS {
        world.step();
    }
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(there)),
        "once the wait is out it reads again"
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

#[test]
fn a_hero_hit_loses_its_drink_but_never_what_a_tree_bought() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(7000, 7000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 7000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(7000, 7000),
    );
    world.settle();
    world.step();
    let salve = crate::game::StatusKind::Mending {
        per_tick: 0,
        breaks: false,
    };
    // A creep may hit all day and the drink holds.
    hand_item(&mut world, hero, crate::game::ITEM_HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, bota_proto::Target::None));
    world.push_hit(Some(creep), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(carries(&world, hero, salve), "a creep does not break it");
    // A hero's blow puts it out.
    world.push_hit(Some(theirs), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(!carries(&world, hero, salve), "a hero does");
    // What a tree bought is broken by nothing.
    let tree = crate::game::tree_positions(map)
        .into_iter()
        .find(|at| {
            at.within(
                bota_proto::Vec2::from_ints(7000, 7000),
                bota_proto::Fixed::from_int(4000),
            )
        })
        .expect("the forest reaches here");
    world.transform.get_mut(hero).expect("hero").pos = tree + bota_proto::Vec2::from_ints(120, 0);
    hand_item(&mut world, hero, crate::game::ITEM_TANGO, 1);
    assert!(world.use_item(hero, 0, bota_proto::Target::Pos(tree)));
    world.push_hit(Some(theirs), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    assert!(
        carries(&world, hero, salve),
        "a tango holds through a hero's blow"
    );
}

#[test]
fn a_ward_in_a_camp_keeps_it_empty() {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let camp = crate::game::CAMPS[0].pos;
    world.spawn_unit(&crate::game::OBSERVER_WARD, bota_proto::Team::Radiant, camp);
    world.settle();
    while world.tick < rules::FIRST_NEUTRAL_TICK + 1 {
        world.step();
    }
    let box_radius = rules::units(rules::CAMP_BOX_RADIUS);
    let filled = world.entities.iter().any(|entity| {
        world.team.get(entity) == Some(&bota_proto::Team::Neutral)
            && world
                .transform
                .get(entity)
                .is_some_and(|t| t.pos.within(camp, box_radius))
    });
    assert!(!filled, "a ward standing in the box keeps the camp empty");
}

#[test]
fn a_creep_that_has_stood_long_enough_shoves_through() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    world.march.insert(
        creep,
        crate::game::March {
            route_step: 0,
            trace: None,
            shove: 0,
        },
    );
    // A body right in front of it, close enough that a step enters its hull.
    world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        at + bota_proto::Vec2::from_ints(40, 0),
    );
    world.settle();
    let aim = at + bota_proto::Vec2::from_ints(400, 0);
    let step = bota_proto::Fixed::from_int(10);
    assert_eq!(
        world.march_step(creep, aim, step),
        at,
        "with a body in the way it does not move"
    );
    if let Some(march) = world.march.get_mut(creep) {
        march.shove = rules::MARCH_SHOVE_TICKS;
    }
    assert_ne!(
        world.march_step(creep, aim, step),
        at,
        "but once it has stood long enough it shoves through"
    );
}

#[test]
fn a_camp_struck_answers_as_one() {
    let mut world = World::new();
    let camp = bota_proto::Vec2::from_ints(5000, 5000);
    let mut beasts = Vec::new();
    for step in 0..2 {
        let beast = world.spawn_unit(
            crate::game::NeutralKind::Kobold.def(),
            bota_proto::Team::Neutral,
            camp + bota_proto::Vec2::from_ints(60 * step, 0),
        );
        world.camp_home.insert(
            beast,
            crate::game::CampHome {
                camp: 0,
                home: camp,
            },
        );
        world.neutral_ai.insert(
            beast,
            crate::game::NeutralAi {
                leash_left: rules::NEUTRAL_AGGRO_WINDOW,
                reaggro_block: 0,
                next_window: rules::NEUTRAL_AGGRO_WINDOW,
                going_home: false,
                roused_by: None,
                awake: false,
            },
        );
        beasts.push(beast);
    }
    // Far enough off that neither would notice a hero standing there.
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        camp + bota_proto::Vec2::from_ints(700, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "left alone the camp takes nobody on"
    );
    // One of them is struck from out there.
    world.push_hit(Some(hero), beasts[0], 10, bota_proto::DamageKind::Physical);
    world.step();
    world.step();
    for beast in &beasts {
        assert_eq!(
            world.target_of(*beast),
            Some(hero),
            "every one of the camp answers, not only the one struck"
        );
    }
}

/// A camp of two kobolds at a spot, asleep and at home.
fn a_camp_at(world: &mut World, at: bota_proto::Vec2) -> Vec<Entity> {
    let mut beasts = Vec::new();
    for step in 0..2 {
        let beast = world.spawn_unit(
            crate::game::NeutralKind::Kobold.def(),
            bota_proto::Team::Neutral,
            at + bota_proto::Vec2::from_ints(60 * step, 0),
        );
        world
            .camp_home
            .insert(beast, crate::game::CampHome { camp: 0, home: at });
        world.neutral_ai.insert(
            beast,
            crate::game::NeutralAi {
                leash_left: rules::NEUTRAL_AGGRO_WINDOW,
                reaggro_block: 0,
                next_window: rules::NEUTRAL_AGGRO_WINDOW,
                going_home: false,
                roused_by: None,
                awake: false,
            },
        );
        beasts.push(beast);
    }
    beasts
}

#[test]
fn a_neutral_sleeps_until_something_comes_right_up_to_it() {
    let camp = bota_proto::Vec2::from_ints(5000, 5000);
    for (apart, wakes) in [
        (rules::NEUTRAL_AGGRO_RANGE + 120, false),
        (rules::NEUTRAL_AGGRO_RANGE - 60, true),
    ] {
        let mut world = World::new();
        let beasts = a_camp_at(&mut world, camp);
        let hero = world.spawn_hero(
            bota_proto::Team::Radiant,
            camp + bota_proto::Vec2::from_ints(apart, 0),
            bota_proto::SlotId(0),
            bota_proto::HeroId(0),
        );
        world.settle();
        world.step();
        world.step();
        assert_eq!(
            world.target_of(beasts[0]) == Some(hero),
            wakes,
            "standing {apart} off, waking should be {wakes}"
        );
    }
}

#[test]
fn a_blow_wakes_a_camp_from_further_than_it_can_see() {
    let mut world = World::new();
    let camp = bota_proto::Vec2::from_ints(5000, 5000);
    let beasts = a_camp_at(&mut world, camp);
    // Far past anything they could see, but inside the reach of a blow.
    let apart = rules::NEUTRAL_DAMAGE_AGGRO_RANGE - 100;
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        camp + bota_proto::Vec2::from_ints(apart, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Neutral, hero),
        "they cannot see that far, which is the point"
    );
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "and they sleep through it"
    );
    world.push_hit(Some(hero), beasts[0], 10, bota_proto::DamageKind::Physical);
    world.step();
    world.step();
    for beast in &beasts {
        assert_eq!(
            world.target_of(*beast),
            Some(hero),
            "a blow wakes the whole camp, seen or not"
        );
    }
    // Further off than a blow carries, it wakes nobody.
    let mut world = World::new();
    let beasts = a_camp_at(&mut world, camp);
    let far = world.spawn_hero(
        bota_proto::Team::Radiant,
        camp + bota_proto::Vec2::from_ints(rules::NEUTRAL_DAMAGE_AGGRO_RANGE + 400, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.push_hit(Some(far), beasts[0], 10, bota_proto::DamageKind::Physical);
    world.step();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "a blow from beyond its reach wakes nobody"
    );
}

#[test]
fn a_cast_aimed_out_of_reach_walks_the_caster_in_and_then_goes_off() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(4000);
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(rules::HOOK_RANGE + 900, 0);
    throw_hook(&mut world, far);
    assert!(
        world.casting.get(pudge).is_some(),
        "out of reach the cast is held rather than dropped"
    );
    let mut thrown = false;
    for _ in 0..400 {
        world.step();
        if world.entities.iter().any(|e| world.hook.get(e).is_some()) {
            thrown = true;
            break;
        }
    }
    assert!(thrown, "and goes off once the caster has walked in");
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert!(
        stood.x.to_int() > from.x.to_int(),
        "the caster walked at it: {} then {}",
        from.x.to_int(),
        stood.x.to_int()
    );
    assert!(
        stood.within(far, bota_proto::Fixed::from_int(rules::HOOK_RANGE)),
        "and no further than it had to"
    );
}

#[test]
fn a_later_order_calls_a_held_cast_off() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(4000);
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(rules::HOOK_RANGE + 900, 0);
    throw_hook(&mut world, far);
    assert!(
        world.casting.get(pudge).is_some(),
        "out of reach the cast is held rather than dropped"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(4200, 5000)),
        },
    }]);
    assert_eq!(
        world.casting.get(pudge),
        None,
        "the later order took the held cast away"
    );
    for _ in 0..120 {
        world.step();
        assert!(
            !world.entities.iter().any(|e| world.hook.get(e).is_some()),
            "and it never goes off"
        );
    }
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert!(
        stood.x < from.x,
        "the body answers the order it was given instead: {stood:?}"
    );
}

#[test]
fn a_cast_walked_towards_a_target_that_fell_is_given_up_and_costs_nothing() {
    let (mut world, pudge, mark) = pudge_and_a_mark(4000);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    world.mana.insert(
        pudge,
        crate::game::Mana {
            mana: Fixed::from_int(110),
        },
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(3),
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    assert!(
        world.casting.get(pudge).is_some(),
        "out of reach the cast is held while the caster walks in"
    );
    world.push_hit(None, mark, 10_000, bota_proto::DamageKind::Pure);
    for _ in 0..3 {
        world.step();
        assert!(
            world.dismember.get(pudge).is_none(),
            "a cast at what has fallen never goes off"
        );
    }
    assert_eq!(
        world.casting.get(pudge),
        None,
        "the held cast is given up with its target"
    );
    assert_eq!(
        world
            .abilities
            .get(pudge)
            .map(|book| book.slots[3].cooldown),
        Some(0),
        "and nothing was spent on it"
    );
}

#[test]
fn an_aimed_cast_takes_the_bodys_order_over() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    assert!(
        matches!(
            world.orders.get(pudge).map(|o| o.current),
            Some(crate::game::UnitOrder::Attack { .. })
        ),
        "the attack order stands"
    );
    let from = world.transform.get(pudge).expect("standing").pos;
    throw_hook(&mut world, from + bota_proto::Vec2::from_ints(400, 0));
    assert!(
        matches!(
            world.orders.get(pudge).map(|o| o.current),
            Some(crate::game::UnitOrder::Idle)
        ),
        "the cast took the order's place and the attack is not returned to"
    );
}

#[test]
fn a_body_walking_into_a_cast_swings_at_nothing_on_the_way() {
    let (mut world, pudge, mark) = pudge_and_a_mark(300);
    let full = world.health.get(mark).expect("standing").hp;
    let from = world.transform.get(pudge).expect("standing").pos;
    let far = from + bota_proto::Vec2::from_ints(0, rules::HOOK_RANGE + 900);
    throw_hook(&mut world, far);
    for _ in 0..30 {
        world.step();
        assert_eq!(
            world.target_of(pudge),
            None,
            "the body is the cast's until it goes off"
        );
    }
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "nothing was swung at on the way"
    );
}

#[test]
fn a_cast_with_no_mana_is_named_and_refused() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    let aim = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(0),
        target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(5600, 5000)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &aim),
        Ok(()),
        "with the mana for it, it is allowed"
    );
    world
        .mana
        .insert(pudge, crate::game::Mana { mana: Fixed::ZERO });
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &aim),
        Err(bota_proto::RejectReason::NotEnoughMana),
        "and without it the seat is told why"
    );
    // An unlearned slot and a wrongly aimed one are named too.
    let unlearned = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(3),
        target: bota_proto::Target::Unit(crate::game::wire_id(pudge)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &unlearned),
        Err(bota_proto::RejectReason::NotLearned),
        "a slot with no points in it says so"
    );
    // A passive is not a slot with nothing in it: it is one that is never
    // cast at all.
    let passive = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(2),
        target: bota_proto::Target::None,
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &passive),
        Err(bota_proto::RejectReason::NotCastable),
        "and a passive says that instead"
    );
    let wrongly_aimed = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(0),
        target: bota_proto::Target::None,
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &wrongly_aimed),
        Err(bota_proto::RejectReason::WrongTargetKind)
    );
}

#[test]
fn a_spell_answers_the_way_a_swing_does_and_never_lets_go() {
    // At an enemy hero it calls them on.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "left alone, creep on creep"
    );
    cast_at(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "a spell at an enemy hero calls them on"
    );
    // At an enemy creep it moves nobody.
    let (mut world, _hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    cast_at(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "a spell at an enemy creep is a spell like any other"
    );
    // At one of your own it is not their business: what holds them holds.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    cast_at(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero), "called on");
    cast_at(&mut world, ours);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "and a spell at one of your own does not let them go"
    );
}

/// Sends the bolt at somebody the way a player does.
fn cast_at(world: &mut World, mark: Entity) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(2),
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
}

#[test]
fn a_bolt_goes_on_to_the_next_and_never_back_to_the_same_one() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let hero = caster(&mut world, at, 2);
    // Three enemies in a row, each within a bounce of the last.
    let marks: Vec<Entity> = (0..3)
        .map(|step| {
            world.spawn_unit(
                &MELEE_CREEP,
                bota_proto::Team::Dire,
                at + bota_proto::Vec2::from_ints(300 + 300 * step, 0),
            )
        })
        .collect();
    world.settle();
    world.step();
    let full: Vec<i32> = marks
        .iter()
        .map(|mark| world.health.get(*mark).expect("standing").hp.to_int())
        .collect();
    world.order_cast(
        hero,
        crate::game::PendingCast {
            slot: bota_proto::AbilitySlot(2),
            target: bota_proto::Target::Unit(crate::game::wire_id(marks[0])),
        },
    );
    for _ in 0..120 {
        world.step();
    }
    for (mark, was) in marks.iter().zip(full) {
        assert!(
            world.health.get(*mark).expect("standing").hp.to_int() < was,
            "the bolt reached every one of them"
        );
    }
    assert!(
        world
            .entities
            .iter()
            .all(|entity| world.projectile.get(entity).is_none()),
        "and is gone once it runs out of places to go"
    );
}

#[test]
fn a_spell_aimed_at_what_it_cannot_take_is_named_and_refused() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world
        .level
        .insert(hero, crate::game::Level(rules::HERO_MAX_LEVEL));
    let mut events = Vec::new();
    assert!(world.learn(hero, 2, &mut events), "the bolt is learned");
    let ally = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        world.transform.get(hero).expect("standing").pos,
    );
    world.settle();
    world.step();
    let at_an_ally = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(2),
        target: bota_proto::Target::Unit(crate::game::wire_id(ally)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &at_an_ally),
        Err(bota_proto::RejectReason::WrongTargetKind),
        "a bolt at one of your own is refused and named"
    );
}

/// The courier of the first seat, while one stands.
fn the_courier(world: &World) -> Entity {
    world.seats[0].courier.expect("a seat has a courier")
}

#[test]
fn a_seat_stands_up_with_a_courier_of_its_own() {
    let world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    assert_eq!(
        world.kind.get(courier),
        Some(&bota_proto::UnitKind::Courier),
        "it is a courier"
    );
    assert_eq!(
        world.owner.get(courier),
        Some(&bota_proto::SlotId(0)),
        "and it belongs to the seat"
    );
    assert!(
        world.inventory.get(courier).is_some(),
        "and it has room to carry"
    );
    assert_eq!(
        world.stats.get(courier).map(|stats| stats.damage),
        Some(0),
        "a courier does not fight"
    );
}

#[test]
fn a_courier_brought_down_comes_back_in_its_own_time() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let mut events = Vec::new();
    world.bury(vec![(courier, None)], &mut events);
    world.step();
    assert!(world.seats[0].courier.is_none(), "it is gone");
    assert!(world.seats[0].courier_left > 0, "and a wait has started");
    for _ in 0..rules::COURIER_RESPAWN_TICKS {
        world.step();
    }
    let back = world.seats[0].courier.expect("it came back");
    assert_ne!(back, courier, "as a new body");
    assert_eq!(
        world.transform.get(back).map(|at| at.pos),
        Some(world.courier_home(bota_proto::Team::Radiant)),
        "at its own fountain"
    );
}

#[test]
fn a_courier_fetches_the_stash_and_hands_it_to_its_owner() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    // Standing at the fountain, it takes what waits there on the next tick.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    assert_eq!(
        world.inventory.get(courier).expect("carries").slots[0].map(|held| held.id),
        Some(boots),
        "and the courier holds it"
    );
    // Sent out to its owner, it walks there and hands it over.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(900, 0);
    assert!(world.courier_deliver(courier));
    for _ in 0..400 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|held| held.id),
        Some(boots),
        "what it carried is in its owner's hands"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .expect("carries")
            .held()
            .count(),
        0,
        "and the courier carries nothing now"
    );
}

#[test]
fn a_burst_makes_a_courier_fly_faster_and_only_one_at_a_time() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    world.step();
    let plain = world.stats.get(courier).expect("settled").move_speed;
    assert!(world.courier_burst(courier));
    world.step();
    assert!(
        world.stats.get(courier).expect("settled").move_speed > plain,
        "it flies faster"
    );
    assert!(
        !world.courier_burst(courier),
        "and one burst at a time is all it has"
    );
}

#[test]
fn an_order_goes_to_the_unit_it_names_and_only_to_ones_this_seat_drives() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let to = world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(600, 0);
    let walk = bota_proto::Order::Move {
        target: bota_proto::Target::Pos(to),
    };
    // Naming nobody is the hero.
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &walk),
        Ok(())
    );
    // Naming its own courier is allowed, and the order lands on the courier.
    let named = Some(crate::game::wire_id(courier));
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), named, &walk),
        Ok(()),
        "a seat drives its own courier"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: named,
        order: walk,
    }]);
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { pos }) if pos == to
        ),
        "the courier was told, not the hero"
    );
    assert!(
        !matches!(
            world.orders.get(hero).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { .. })
        ),
        "and the hero was left alone"
    );
    // Anything else is nobody this seat drives.
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(6000, 6000),
    );
    world.settle();
    assert_eq!(
        world.validate_order(
            bota_proto::SlotId(0),
            Some(crate::game::wire_id(creep)),
            &walk
        ),
        Err(bota_proto::RejectReason::NotYourUnit),
        "a creep of its own side is still not its to drive"
    );
}

#[test]
fn a_courier_carries_its_errands_as_abilities() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let book = world.abilities.get(courier).expect("a courier casts");
    let carried: Vec<_> = book.slots.iter().map(|slot| slot.id).collect();
    assert_eq!(
        carried,
        vec![
            crate::game::ability::TAKE_STASH,
            crate::game::ability::RETURN_ITEMS,
            crate::game::ability::BURST,
            crate::game::ability::DELIVER,
            crate::game::ability::SHIELD,
        ],
        "it knows what a courier knows"
    );
    assert!(
        book.slots.iter().all(|slot| slot.level == 1),
        "and knows them from the start"
    );
    // Sent through the wire the way a player sends it.
    let named = Some(crate::game::wire_id(courier));
    // The burst sits third, after the two that fetch and put back.
    let burst = bota_proto::Order::Cast {
        slot: bota_proto::AbilitySlot(2),
        target: bota_proto::Target::None,
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), named, &burst),
        Ok(())
    );
    world.step();
    let plain = world.stats.get(courier).expect("settled").move_speed;
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: named,
        order: burst,
    }]);
    world.step();
    assert!(
        world.stats.get(courier).expect("settled").move_speed > plain,
        "the burst went off"
    );
}

#[test]
fn a_courier_told_to_go_at_a_unit_follows_it() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    // Something of its own side standing a way off, that then walks further.
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        home + bota_proto::Vec2::from_ints(700, 0),
    );
    world.settle();
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: Some(crate::game::wire_id(courier)),
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    for _ in 0..120 {
        world.step();
    }
    let near = |world: &World| {
        let at = world.transform.get(courier).expect("standing").pos;
        let to = world.transform.get(mark).expect("standing").pos;
        crate::game::isqrt64(at.distance_squared(to))
            <= i64::from(bota_proto::Fixed::from_int(150).raw)
    };
    assert!(near(&world), "it went to it");
    // Moved on, it is followed rather than left behind.
    world.transform.get_mut(mark).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(700, 900);
    for _ in 0..200 {
        world.step();
    }
    assert!(near(&world), "and it keeps up when the mark moves");
}

#[test]
fn a_courier_at_the_fountain_reaches_the_stash_itself() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    // The hero is out in the lane, so the stash is nothing to it.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(4000, 0);
    assert!(
        !world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "out in the lane a hero cannot reach into the stash"
    );
    // The courier is standing at the fountain, so for it the stash is right
    // there.
    assert!(
        world.move_item(bota_proto::SlotId(0), courier, crate::game::BAG_SLOTS, 0),
        "the courier at the fountain reaches it"
    );
    assert_eq!(
        world.inventory.get(courier).expect("carries").slots[0].map(|held| held.id),
        Some(boots),
        "and what waited there is in its hands"
    );
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    // And back again.
    assert!(world.move_item(bota_proto::SlotId(0), courier, 0, crate::game::BAG_SLOTS));
    assert_eq!(
        world.seats[0].stash.slots[0].map(|held| held.id),
        Some(boots),
        "it goes back the same way"
    );
}

#[test]
fn an_order_takes_a_courier_off_its_errand() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(3000, 0);
    // Something to carry, or it would simply go home instead.
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
    // Sent to its owner, it is on its way.
    assert!(world.courier_deliver(courier));
    for _ in 0..30 {
        world.step();
    }
    assert_eq!(
        world.errand.get(courier),
        Some(&crate::game::Errand::ToOwner),
        "it is on the errand"
    );
    // Told to go somewhere else, it goes there instead.
    let aside = home + bota_proto::Vec2::from_ints(0, 800);
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: Some(crate::game::wire_id(courier)),
        order: bota_proto::Order::Move {
            target: bota_proto::Target::Pos(aside),
        },
    }]);
    assert_eq!(
        world.errand.get(courier),
        Some(&crate::game::Errand::None),
        "the order took it off the errand"
    );
    for _ in 0..300 {
        world.step();
    }
    assert_eq!(
        world.transform.get(courier).map(|at| at.pos),
        Some(aside),
        "and it went where it was told"
    );
}

#[test]
fn a_courier_that_has_handed_over_turns_for_home() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(900, 0);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
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
    assert!(world.courier_take_stash(courier));
    world.step();
    assert!(world.courier_deliver(courier));
    for _ in 0..300 {
        world.step();
        if world.errand.get(courier) == Some(&crate::game::Errand::None) {
            break;
        }
    }
    assert!(
        world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0),
        "it handed over"
    );
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { pos }) if pos == home
        ),
        "and turned for home on its own"
    );
}

#[test]
fn taking_the_stash_carries_it_on_without_being_asked_twice() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(900, 0);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
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
    assert!(world.courier_take_stash(courier));
    for _ in 0..300 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert!(
        world.inventory.get(hero).expect("has a bag").held().count() > 0,
        "one press fetched it and brought it"
    );
}

#[test]
fn an_errand_with_nothing_to_do_sends_the_courier_home() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(courier).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(1500, 0);
    // Nothing in the stash and nothing in its hands.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert_eq!(
        world.errand.get(courier),
        Some(&crate::game::Errand::GoingHome),
        "with nothing to take it goes home"
    );
    for _ in 0..400 {
        world.step();
    }
    assert_eq!(
        world.transform.get(courier).map(|at| at.pos),
        Some(home),
        "and gets there"
    );
}

#[test]
fn a_courier_whose_owner_fell_puts_what_it_carries_back() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
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
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    assert!(world.courier_deliver(courier));
    for _ in 0..300 {
        world.step();
        if world.seats[0].stash.held().count() > 0 {
            break;
        }
    }
    assert!(
        world.seats[0].stash.held().count() > 0,
        "with nobody to hand to, it put it back in the stash"
    );
}

#[test]
fn a_shielded_courier_takes_nothing() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    world.step();
    let full = world.health.get(courier).expect("standing").hp;
    assert!(world.courier_shield(courier));
    world.step();
    world.push_hit(None, courier, 100, bota_proto::DamageKind::Pure);
    world.step();
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(full),
        "nothing gets through while it holds"
    );
    for _ in 0..rules::COURIER_SHIELD_TICKS {
        world.step();
    }
    world.push_hit(None, courier, 100, bota_proto::DamageKind::Pure);
    world.step();
    assert!(
        world.health.get(courier).expect("standing").hp < full,
        "and once it lifts the courier is a courier again"
    );
}

#[test]
fn the_stash_sells_from_anywhere_and_a_bag_far_out_only_marks() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let stack = crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    };
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(stack);
    }
    world.seats[0].stash.slots[0] = Some(stack);
    // Out in the lane, well away from its own shop.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(4000, 0);
    world.settle();
    let purse = world.seats[0].gold;
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "the ask is taken out there"
    );
    assert_eq!(world.seats[0].gold, purse, "but as a mark, not a sale");
    assert!(
        world
            .inventory
            .get(hero)
            .and_then(|bag| bag.slots[0])
            .is_some_and(|held| held.for_sale),
        "the stack stays in hand, marked"
    );
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS),
        "what waits in the stash is already at the shop"
    );
    assert!(world.seats[0].gold > purse, "and paid for");
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "and gone from the stash"
    );
    // Out in the lane the order is allowed now: it marks.
    let sell_bag = bota_proto::Order::Sell {
        slot: bota_proto::ItemSlot(0),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &sell_bag),
        Ok(())
    );
}

#[test]
fn the_jungle_pays_a_courier_no_mind_but_the_other_side_does_not() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let at = world.transform.get(courier).expect("standing").pos;
    // A neutral and a creep of the other side, both standing on top of it.
    let beast = world.spawn_unit(
        crate::game::NeutralKind::Kobold.def(),
        bota_proto::Team::Neutral,
        at,
    );
    world
        .camp_home
        .insert(beast, crate::game::CampHome { camp: 0, home: at });
    world.neutral_ai.insert(
        beast,
        crate::game::NeutralAi {
            leash_left: rules::NEUTRAL_AGGRO_WINDOW,
            reaggro_block: 0,
            next_window: rules::NEUTRAL_AGGRO_WINDOW,
            going_home: false,
            roused_by: None,
            awake: true,
        },
    );
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Dire, at);
    world.settle();
    for _ in 0..10 {
        world.step();
    }
    assert!(
        !world.hostile(beast, courier),
        "the jungle does not take a courier on"
    );
    assert_ne!(world.target_of(beast), Some(courier));
    assert!(
        world.hostile(creep, courier),
        "a creep of the other side does"
    );
    assert_eq!(world.target_of(creep), Some(courier), "and goes for it");
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

/// A hero of the plain kind, standing at its own shop with gold in hand.
fn a_hero_with_gold(gold: i32) -> (World, Entity) {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.seats[0].gold = gold;
    world.settle();
    // A tick past settling, so what the fountain hands out is already on and
    // does not read as a change of its own.
    world.step();
    (world, hero)
}

/// What sits in one of a hero's inventory slots.
fn slot_of(world: &World, hero: Entity, at: usize) -> Option<crate::game::ItemStack> {
    world.inventory.get(hero).and_then(|bag| bag.slots[at])
}

/// What one item of the catalog costs.
fn price_of(item: u16) -> i32 {
    crate::game::ITEMS[usize::from(item)].cost
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
    assert!(
        hasted.attack_interval < plain.attack_interval,
        "and the wait is shorter for it"
    );
    assert_eq!(
        hasted.attack_interval,
        crate::game::HERO.attack_interval * rules::BASE_ATTACK_SPEED as u32
            / hasted.attack_speed as u32,
        "by exactly what the speed says"
    );
}

#[test]
fn buying_a_built_item_buys_only_the_parts_it_lacks() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let before = world.seats[0].gold;
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought"
    );
    assert_eq!(
        before - world.seats[0].gold,
        price_of(crate::game::ITEM_POWER_TREADS) - price_of(crate::game::ITEM_BOOTS),
        "the boots already in hand are not paid for twice"
    );
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(treads),
        "and the parts build themselves into the whole"
    );
    assert!(
        world
            .inventory
            .get(hero)
            .expect("has a bag")
            .slots
            .iter()
            .skip(1)
            .all(|slot| slot.is_none()),
        "leaving nothing of what went into it"
    );
}

#[test]
fn what_an_item_is_set_to_is_worth_points_of_that_attribute() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought"
    );
    world.step();
    let bonus =
        Fixed::from_int(crate::game::ITEMS[usize::from(crate::game::ITEM_POWER_TREADS)].mode_bonus);
    let on_strength = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Strength),
        "they come set to strength"
    );
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None),
        "and using them sets them over"
    );
    world.step();
    let on_agility = *world.stats.get(hero).expect("settled");
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Agility),
        "to the attribute after the one they were on"
    );
    assert_eq!(
        on_agility.attributes.strength + bonus,
        on_strength.attributes.strength,
        "what they were worth in strength is gone"
    );
    assert_eq!(
        on_agility.attributes.agility,
        on_strength.attributes.agility + bonus,
        "and worth the same in agility instead"
    );
}

#[test]
fn treads_switched_round_the_wheel_mend_nothing() {
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
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_POWER_TREADS),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: Some(bota_proto::Attribute::Strength),
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    world.settle();
    world.step();
    world.health.insert(
        hero,
        Health {
            hp: Fixed::from_int(100),
        },
    );
    world.mana.insert(
        hero,
        Mana {
            mana: Fixed::from_int(30),
        },
    );
    let switches = 12;
    for _ in 0..switches {
        assert!(
            world.use_item(hero, 0, bota_proto::Target::None),
            "switched"
        );
        world.step();
    }
    assert_eq!(
        slot_of(&world, hero, 0).and_then(|stack| stack.mode),
        Some(bota_proto::Attribute::Strength),
        "four full turns of the wheel end where they began"
    );
    // What the pools may gain over the wheel is what regeneration mends and
    // not a drop more, bounded by the highest rate any mode pays.
    let bonus =
        Fixed::from_int(crate::game::ITEMS[usize::from(crate::game::ITEM_POWER_TREADS)].mode_bonus);
    let ticks = Fixed::from_int(switches);
    let mended = (rules::HERO_HP_REGEN
        + rules::HP_REGEN_PER_STRENGTH * (rules::HERO_ATTRIBUTES.strength + bonus))
        * ticks;
    assert!(
        world.health.get(hero).expect("standing").hp <= Fixed::from_int(100) + mended,
        "no health is minted"
    );
    let cleared = (rules::HERO_MANA_REGEN
        + rules::MANA_REGEN_PER_INTELLIGENCE * (rules::HERO_ATTRIBUTES.intelligence + bonus))
        * ticks;
    assert!(
        world.mana.get(hero).expect("has a pool").mana <= Fixed::from_int(30) + cleared,
        "and no mana"
    );
}

/// Every item lying on the ground.
fn on_the_ground(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| world.loot.get(*e).is_some())
        .collect()
}

/// A plain stack of one item for a seat, already touched.
fn a_stack_of(item: u16, owner: u8) -> crate::game::ItemStack {
    crate::game::ItemStack {
        id: bota_proto::ItemId(item),
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: bota_proto::SlotId(owner),
        for_sale: false,
    }
}

#[test]
fn an_item_laid_down_lies_where_it_was_aimed() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(100),
        y: from.y,
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "the bag slot gave it up"
    );
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1, "one item lies on the ground");
    assert_eq!(
        world.transform.get(lying[0]).map(|t| t.pos),
        Some(spot),
        "where it was aimed"
    );
}

#[test]
fn an_item_aimed_past_reach_walks_its_carrier_in_first() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(1000),
        y: from.y + rules::units(1000),
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.step();
    assert!(on_the_ground(&world).is_empty(), "too far to lay at once");
    for _ in 0..300 {
        world.step();
    }
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1, "walked over and laid it");
    assert_eq!(world.transform.get(lying[0]).map(|t| t.pos), Some(spot));
    let stood = world.transform.get(hero).expect("stands").pos;
    assert!(
        stood.within(spot, rules::units(rules::PUT_ITEM_RANGE)),
        "from within reach"
    );
}

#[test]
fn what_is_handed_over_lands_in_the_first_free_slot() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    assert!(world.put_item(
        hero,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(courier))
    ));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "out of the hero's hands"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .and_then(|bag| bag.slots[0])
            .map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "and into the courier's"
    );
    // And back again the same way.
    assert!(world.put_item(
        courier,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(hero))
    ));
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "handed back"
    );
    assert!(
        world
            .inventory
            .get(courier)
            .is_some_and(|bag| bag.held().count() == 0),
        "and the courier's hands are empty"
    );
}

#[test]
fn a_bag_with_no_room_is_handed_nothing() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(courier) {
        for slot in bag.slots.iter_mut() {
            *slot = Some(a_stack_of(crate::game::ITEM_IRON_BRANCH, 0));
        }
    }
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    assert!(world.put_item(
        hero,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(courier))
    ));
    world.step();
    assert!(slot_of(&world, hero, 0).is_some(), "kept where it was");
    assert!(world.handling.get(hero).is_none(), "and the errand is over");
}

#[test]
fn an_enemy_may_take_what_lies_on_the_ground_but_never_sell_it() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let mid = bota_proto::Vec2::from_ints(8000, 8000);
    let enemy = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2 {
            x: mid.x + rules::units(100),
            y: mid.y,
        },
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(1),
        bota_proto::Team::Dire,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[1].unit = Some(enemy);
    world.settle();
    let lying = world.lay_loot(a_stack_of(crate::game::ITEM_BOOTS, 0), mid);
    assert!(world.take_item(enemy, crate::game::wire_id(lying)));
    world.step();
    assert_eq!(
        slot_of(&world, enemy, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "anybody with a bag may take it"
    );
    assert!(
        on_the_ground(&world).is_empty(),
        "and it is gone from the ground"
    );
    // At its own shop it is still not the enemy's to cash.
    if let Some(t) = world.transform.get_mut(enemy) {
        t.pos = crate::game::fountain_pos(world.map, bota_proto::Team::Dire);
    }
    let before = world.seats[1].gold;
    assert!(
        !world.sell_item(bota_proto::SlotId(1), enemy, 0),
        "not this seat's to sell"
    );
    assert!(slot_of(&world, enemy, 0).is_some(), "so it stays in hand");
    assert_eq!(world.seats[1].gold, before, "and pays nothing");
}

#[test]
fn what_was_muted_stays_muted_across_the_ground() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            mute: 100,
            ..a_stack_of(crate::game::ITEM_BOOTS, 0)
        });
    }
    assert!(world.put_item(hero, 0, bota_proto::Target::None));
    world.step();
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1);
    assert!(world.take_item(hero, crate::game::wire_id(lying[0])));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.mute > 90),
        "the ground is no way around the backpack mute"
    );
}

#[test]
fn selling_away_from_the_shop_marks_the_stack_and_a_second_ask_unmarks_it() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    let before = world.seats[0].gold;
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "away from the shop the ask is taken"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.for_sale),
        "as a mark rather than a sale"
    );
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_some(),
        "and nothing sells this far out"
    );
    assert_eq!(world.seats[0].gold, before);
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "asked again"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| !stack.for_sale),
        "the mark is off"
    );
}

#[test]
fn a_marked_stack_sells_the_moment_it_reaches_the_shop() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    assert!(world.sell_item(bota_proto::SlotId(0), hero, 0), "marked");
    let before = world.seats[0].gold;
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    }
    world.step();
    assert!(slot_of(&world, hero, 0).is_none(), "sold on arrival");
    let half = price_of(crate::game::ITEM_BOOTS) * rules::SELL_PCT / 100;
    let gained = world.seats[0].gold - before;
    assert!(
        gained >= half && gained <= half + 1,
        "for its part of the price, {gained} against {half}"
    );
}

#[test]
fn a_courier_called_empty_still_collects_what_is_marked() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    let shop = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = shop + bota_proto::Vec2::from_ints(1500, 1500);
    }
    assert!(world.sell_item(bota_proto::SlotId(0), hero, 0), "marked");
    let before = world.seats[0].gold;
    let half = price_of(crate::game::ITEM_BOOTS) * rules::SELL_PCT / 100;
    assert!(world.courier_deliver(courier), "called with an empty bag");
    for _ in 0..1500 {
        world.step();
        if world.seats[0].gold >= before + half {
            break;
        }
    }
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "the mark went with the bird"
    );
    assert!(
        world.seats[0].gold >= before + half,
        "and came back as gold"
    );
    assert!(
        world.seats[0].stash.held().count() == 0,
        "sold at the shop rather than shelved"
    );
}

#[test]
fn what_another_seat_bought_is_not_yours_to_sell() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 1));
    }
    let before = world.seats[0].gold;
    assert!(
        !world.sell_item(bota_proto::SlotId(0), hero, 0),
        "the other seat bought it"
    );
    assert!(slot_of(&world, hero, 0).is_some(), "so it stays");
    assert_eq!(world.seats[0].gold, before);
}

#[test]
fn a_marked_part_or_a_borrowed_part_builds_nothing() {
    let (mut world, hero) = a_hero_with_gold(0);
    // Out of the shop's reach, or the marked part would sell before the
    // build ever looked at it.
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            for_sale: true,
            ..a_stack_of(crate::game::ITEM_BOOTS, 0)
        });
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 0));
        bag.slots[2] = Some(a_stack_of(crate::game::ITEM_BELT, 0));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "a part marked for sale does not vanish into a build"
    );
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 1));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "nor does a part somebody else bought"
    );
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 0));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_POWER_TREADS)),
        "whole and owned, the parts come together"
    );
}

#[test]
fn a_courier_keeps_its_load_through_death() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    let mut events = Vec::new();
    world.bury(vec![(courier, None)], &mut events);
    assert!(
        world.seats[0]
            .courier_kept
            .as_ref()
            .is_some_and(|bag| bag.held().count() == 1),
        "the load waits on the seat"
    );
    for _ in 0..rules::COURIER_RESPAWN_TICKS + 2 {
        world.step();
    }
    let back = world.seats[0].courier.expect("stands again");
    assert_ne!(back, courier, "in a new body");
    assert_eq!(
        world.inventory.get(back).map(|bag| bag.held().count()),
        Some(1),
        "with the load back aboard"
    );
}

#[test]
fn a_later_order_calls_an_item_errand_off() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(1000),
        y: from.y + rules::units(1000),
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..200 {
        world.step();
    }
    assert!(on_the_ground(&world).is_empty(), "nothing was laid");
    assert!(
        slot_of(&world, hero, 0).is_some(),
        "and the item never left"
    );
}

#[test]
fn what_lies_on_the_ground_is_seen_through_the_fog() {
    let (mut world, hero) = a_hero_with_gold(0);
    let beside = world.transform.get(hero).expect("stands").pos;
    world.lay_loot(a_stack_of(crate::game::ITEM_BOOTS, 0), beside);
    world.step();
    let ours = world.view(bota_proto::Team::Radiant);
    assert_eq!(ours.loot.len(), 1, "lying in our own light");
    assert_eq!(
        ours.loot[0].item,
        bota_proto::ItemId(crate::game::ITEM_BOOTS)
    );
    assert_eq!(ours.loot[0].charges, None, "boots hold no charges");
    let theirs = world.view(bota_proto::Team::Dire);
    assert!(theirs.loot.is_empty(), "the far side has no eyes on it");
}

#[test]
fn a_blink_carries_no_further_than_it_reaches() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    let range = match crate::game::ITEMS[usize::from(crate::game::ITEM_BLINK_DAGGER)].active {
        Some(crate::game::ItemUse::Blink { range }) => range,
        _ => panic!("the dagger blinks"),
    };
    let far = bota_proto::Vec2 {
        x: from.x + rules::units(range * 4),
        y: from.y,
    };
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(far)),
        "it goes"
    );
    let landed = world.transform.get(hero).expect("stands somewhere").pos;
    assert!(landed != from, "and it carried");
    assert!(
        landed.within(from, rules::units(range)),
        "no further than it reaches"
    );
}

#[test]
fn a_blink_aimed_at_closed_ground_steps_back_to_open() {
    let (mut world, hero) = a_hero_with_gold(0);
    let from = world.transform.get(hero).expect("stands somewhere").pos;
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    let aim = bota_proto::Vec2 {
        x: from.x + rules::units(600),
        y: from.y,
    };
    world
        .grid
        .block_circle(aim, rules::units(rules::BLINK_STEP_BACK * 2));
    assert!(
        world.use_item(hero, 0, bota_proto::Target::Pos(aim)),
        "it goes"
    );
    let landed = world.transform.get(hero).expect("stands somewhere").pos;
    assert!(world.grid.walkable(landed), "and it lands on open ground");
    assert!(landed != aim, "short of what it was aimed at");
}

#[test]
fn a_blow_from_a_hero_sets_a_blink_back() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BLINK_DAGGER, 0);
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.cooldown),
        Some(0),
        "it is ready"
    );
    let hitter = world.spawn();
    world.kind.insert(hitter, bota_proto::UnitKind::Hero);
    world.push_hit(Some(hitter), hero, 10, bota_proto::DamageKind::Physical);
    world.step();
    let wait = crate::game::ITEMS[usize::from(crate::game::ITEM_BLINK_DAGGER)].breaks_on_damage;
    assert!(
        slot_of(&world, hero, 0)
            .map(|stack| stack.cooldown)
            .is_some_and(|left| left > 0 && left <= wait),
        "and a blow from a hero sets it back"
    );
}

#[test]
fn a_magic_stick_gains_charges_and_is_kept_when_it_spends_them() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_MAGIC_STICK, 0);
    world.step();
    assert!(
        !world.use_item(hero, 0, bota_proto::Target::None),
        "with no charge there is nothing to spend"
    );
    if let Some(bag) = world.inventory.get_mut(hero)
        && let Some(stack) = bag.slots[0].as_mut()
    {
        stack.charges = 4;
    }
    if let Some(pool) = world.health.get_mut(hero) {
        pool.hp = Fixed::from_int(100);
    }
    world.step();
    let before = world.health.get(hero).expect("has health").hp;
    assert!(
        world.use_item(hero, 0, bota_proto::Target::None),
        "with charges it mends"
    );
    assert!(
        world.health.get(hero).expect("has health").hp > before,
        "and health comes back"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.charges == 0),
        "every charge goes at once, and the stack stays"
    );
}

#[test]
fn phase_walks_a_body_through_another() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_PHASE_BOOTS, 0);
    world.step();
    assert!(
        !world.stats.get(hero).expect("settled").phased,
        "it walks round what is in the way until the boots are used"
    );
    assert!(world.use_item(hero, 0, bota_proto::Target::None), "it goes");
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").phased,
        "and then walks through it"
    );
}

#[test]
fn buying_a_second_of_something_buys_a_second_of_it() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let branch = bota_proto::ItemId(crate::game::ITEM_IRON_BRANCH);
    let mut events = Vec::new();
    for held in 1..=3 {
        let before = world.seats[0].gold;
        assert!(
            world.buy(bota_proto::SlotId(0), branch, &mut events),
            "bought the branch"
        );
        assert_eq!(
            before - world.seats[0].gold,
            price_of(crate::game::ITEM_IRON_BRANCH),
            "and paid for it"
        );
        assert_eq!(
            world
                .inventory
                .get(hero)
                .expect("has a bag")
                .slots
                .iter()
                .flatten()
                .filter(|stack| stack.id == branch)
                .count(),
            held,
            "one more branch in hand than before"
        );
    }
}

#[test]
fn buying_a_built_item_a_second_time_buys_its_parts_again() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought the first pair"
    );
    world.step();
    let before = world.seats[0].gold;
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought a second pair"
    );
    assert_eq!(
        before - world.seats[0].gold,
        price_of(crate::game::ITEM_POWER_TREADS),
        "the pair already worn is no part of the new one"
    );
    world.step();
    assert_eq!(
        world
            .inventory
            .get(hero)
            .expect("has a bag")
            .slots
            .iter()
            .flatten()
            .filter(|stack| stack.id == treads)
            .count(),
        2,
        "and both pairs are in hand"
    );
}

#[test]
fn walking_at_an_ally_stops_where_the_bodies_meet() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let at = world.transform.get(hero).expect("standing").pos;
    let ally = world.spawn_unit(
        &crate::game::MELEE_CREEP,
        bota_proto::Team::Radiant,
        at + bota_proto::Vec2::from_ints(600, 0),
    );
    world.settle();
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(ally)),
        },
    }]);
    let mut seen: Vec<bota_proto::Vec2> = Vec::new();
    for _ in 0..240 {
        world.advance(&[]);
        seen.push(world.transform.get(hero).expect("standing").pos);
    }
    let theirs = world.transform.get(ally).expect("standing").pos;
    let hulls = world.hull.get(hero).expect("has one").radius
        + world.hull.get(ally).expect("has one").radius;
    let last = *seen.last().expect("walked");
    assert!(
        last.within(theirs, hulls + rules::units(rules::STEER_MARGIN * 2)),
        "it comes right up to what it was pointed at"
    );
    // Once it has arrived it stays arrived: what used to happen is that it
    // pressed into the body, was eased out, and walked in again for ever.
    let settled = &seen[seen.len() - 60..];
    let drift = settled
        .iter()
        .map(|spot| {
            let (dx, dy) = (spot.x - last.x, spot.y - last.y);
            dx.raw.abs().max(dy.raw.abs())
        })
        .max()
        .expect("some ticks");
    assert!(
        drift < rules::units(24).raw,
        "and it stands there rather than circling"
    );
}

#[test]
fn a_courier_sent_for_the_stash_carries_on_what_it_already_holds() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: boots,
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
    let home = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(2500, 0);
    // The stash is empty, so the old answer was to fly home with the goods.
    assert!(world.courier_take_stash(courier));
    for _ in 0..600 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|held| held.id),
        Some(boots),
        "what it was already carrying reaches its owner"
    );
}

#[test]
fn what_an_owner_has_no_room_for_goes_back_to_the_stash() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let branch = bota_proto::ItemId(crate::game::ITEM_IRON_BRANCH);
    let stack = crate::game::ItemStack {
        id: branch,
        charges: 1,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    };
    // Every slot the hero has is taken, so there is nowhere to hand it.
    if let Some(bag) = world.inventory.get_mut(hero) {
        for slot in bag.slots.iter_mut() {
            *slot = Some(stack);
        }
    }
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(stack);
    }
    let home = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(1500, 0);
    assert!(world.courier_deliver(courier));
    for _ in 0..900 {
        world.step();
        if world.seats[0].stash.held().count() > 0 {
            break;
        }
    }
    assert_eq!(
        world.seats[0].stash.held().count(),
        1,
        "with nowhere to put it, it is carried back to the stash"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .expect("carries")
            .held()
            .count(),
        0,
        "and the courier is empty again"
    );
}

/// A world with a wall of closed ground between two spots, so the way from
/// one to the other has to be found rather than walked straight.
fn a_world_with_a_wall(from: bota_proto::Vec2, to: bota_proto::Vec2) -> World {
    let mut world = World::for_match(&config(), config().rng());
    world.settle();
    let middle = bota_proto::Vec2 {
        x: bota_proto::Fixed {
            raw: (from.x.raw / 2).saturating_add(to.x.raw / 2),
        },
        y: bota_proto::Fixed {
            raw: (from.y.raw / 2).saturating_add(to.y.raw / 2),
        },
    };
    world.grid.block_circle(middle, rules::units(600));
    assert!(
        !crate::game::grid_los(&world.grid, from, to),
        "the wall stands in the way"
    );
    world
}

#[test]
fn what_flies_goes_straight_over_what_a_walker_goes_round() {
    let hero_at = bota_proto::Vec2::from_ints(6000, 9216);
    let far = bota_proto::Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, far);
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    world.transform.get_mut(hero).expect("standing").pos = far;
    world.transform.get_mut(courier).expect("flies").pos = hero_at;
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: boots,
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
    assert!(world.courier_deliver(courier));
    // Straight there means the line it walks never wanders off the line it
    // was on: a flier routed like a walker swings wide round the wall.
    let mut widest = 0;
    for _ in 0..400 {
        world.step();
        let now = world.transform.get(courier).expect("flies").pos;
        widest = widest.max((now.y.raw - hero_at.y.raw).abs());
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert!(
        world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0),
        "it got there"
    );
    assert!(
        widest < rules::units(200).raw,
        "and it went over the wall rather than round it"
    );
}

#[test]
fn a_way_found_to_a_spot_that_walks_away_is_found_again() {
    let hero_at = bota_proto::Vec2::from_ints(6000, 9216);
    let goal_at = bota_proto::Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, goal_at);
    let hero = world.seats[0].unit.expect("stood up");
    world.transform.get_mut(hero).expect("standing").pos = hero_at;
    let mut goal = goal_at;
    world.set_order(hero, crate::game::UnitOrder::Move { pos: goal });
    world.step();
    assert!(
        world
            .route
            .get(hero)
            .is_some_and(|route| !route.path.is_empty()),
        "a way round the wall was found"
    );
    // The spot creeps away, a little every tick, exactly as a walking hero
    // does to a courier chasing it. Never enough in one tick to look like a
    // new goal, and after a while far beyond where the way was laid to.
    for _ in 0..60 {
        goal += bota_proto::Vec2::from_ints(0, 20);
        world.set_order(hero, crate::game::UnitOrder::Move { pos: goal });
        world.step();
    }
    let end = world
        .route
        .get(hero)
        .and_then(|route| route.path.last().copied())
        .expect("still walking a way round");
    assert!(
        end.within(goal, rules::units(600)),
        "the way it walks leads where the spot is now, not where it was"
    );
}

#[test]
fn a_flesh_heap_outlives_the_death_of_the_one_carrying_it() {
    let (mut world, pudge, mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    world.step();
    let bare = world.stats.get(pudge).expect("settled").max_hp;
    let mut events = Vec::new();
    world.bury(vec![(mark, None)], &mut events);
    world.step();
    assert_eq!(
        world
            .stacks
            .get(pudge)
            .map(|kept| kept.of(StackKind::FleshHeap)),
        Some(1),
        "one death has fed it"
    );
    world.bury(vec![(pudge, None)], &mut events);
    assert_eq!(world.seats[0].unit, None, "the body is gone");
    for _ in 0..world.seats[0].respawn_left {
        world.step();
    }
    let back = world.seats[0].unit.expect("stands again");
    assert_eq!(
        world
            .stacks
            .get(back)
            .map(|kept| kept.of(StackKind::FleshHeap)),
        Some(1),
        "and what it kept comes back with it"
    );
    assert_eq!(
        world.stats.get(back).map(|s| s.max_hp),
        Some(bare + Fixed::from_int(rules::FLESH_HEAP_HP)),
        "worth as much health as it was before"
    );
}

/// Shadow Fiend at the middle of the map, facing east, with a creep `apart`
/// to the east of him and his whole kit learned to its first level.
fn fiend_and_a_mark(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let fiend = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(2),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + apart, 5000),
    );
    if let Some(book) = world.abilities.get_mut(fiend) {
        for held in book.slots.iter_mut() {
            held.level = 1;
        }
    }
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(2),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(fiend);
    world.settle();
    world.step();
    (world, fiend, mark)
}

/// Casts one of the slots at nothing, the way a player does.
fn let_go(world: &mut World, slot: u8) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(slot),
            target: bota_proto::Target::None,
        },
    }]);
}

/// Stands a creep of one side up at a spot.
fn a_creep_at(world: &mut World, team: bota_proto::Team, at: bota_proto::Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.settle();
    creep
}

#[test]
fn a_raze_burns_what_stands_where_it_lands_and_nothing_else() {
    let (mut world, _fiend, near) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
    let far = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + 900, 5000),
    );
    let (was_near, was_far) = (
        world.health.get(near).expect("standing").hp,
        world.health.get(far).expect("standing").hp,
    );
    let_go(&mut world, 1);
    world.step();
    assert!(
        world.health.get(near).expect("standing").hp < was_near,
        "what stands where the raze lands feels it"
    );
    assert_eq!(
        world.health.get(far).expect("standing").hp,
        was_far,
        "and what stands past it does not"
    );
}

#[test]
fn a_raze_lands_at_its_own_reach_however_near_the_enemy_stands() {
    let (mut world, _fiend, under) = fiend_and_a_mark(50);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000),
    );
    let (was_under, was_out) = (
        world.health.get(under).expect("standing").hp,
        world.health.get(out).expect("standing").hp,
    );
    // The farthest raze lands at its own reach, over the head of what stands
    // right under his feet.
    let_go(&mut world, 2);
    world.step();
    assert!(
        world.health.get(out).expect("standing").hp < was_out,
        "the raze lands at its own reach"
    );
    assert_eq!(
        world.health.get(under).expect("standing").hp,
        was_under,
        "and nowhere nearer"
    );
}

#[test]
fn a_raze_lays_itself_along_the_facing() {
    let (mut world, _fiend, ahead) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
    let behind = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 - rules::RAZE_DISTANCE[1], 5000),
    );
    let (was_ahead, was_behind) = (
        world.health.get(ahead).expect("standing").hp,
        world.health.get(behind).expect("standing").hp,
    );
    let_go(&mut world, 1);
    world.step();
    assert!(
        world.health.get(ahead).expect("standing").hp < was_ahead,
        "what stands where he faces feels it"
    );
    assert_eq!(
        world.health.get(behind).expect("standing").hp,
        was_behind,
        "and what stands behind him does not"
    );
}

#[test]
fn souls_come_only_from_what_the_gatherer_brings_down() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    let other = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    world.bury(vec![(other, None)], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map_or(0, |kept| kept.of(StackKind::Souls)),
        0,
        "a death nobody is answerable for is worth nothing"
    );
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_UNIT),
        "and one he brought down is worth its soul"
    );
}

#[test]
fn a_hero_brought_down_is_worth_more_souls_than_a_creep() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let victim = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(victim, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_HERO),
    );
}

#[test]
fn souls_stop_at_what_the_necromastery_holds() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let cap = world.soul_cap(fiend);
    assert_eq!(
        cap,
        rules::NECRO_SOUL_CAP[0],
        "the necromastery at its first level"
    );
    hand_souls(&mut world, fiend, cap);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(cap),
        "no more are held than the level allows"
    );
}

#[test]
fn every_soul_held_is_worth_attack_damage() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    world.step();
    let bare = world.stats.get(fiend).expect("settled").damage;
    hand_souls(&mut world, fiend, 5);
    world.step();
    assert_eq!(
        world.stats.get(fiend).map(|s| s.damage),
        Some(bare + rules::DAMAGE_PER_SOUL * 5),
    );
}

#[test]
fn a_raze_goes_off_where_it_is_asked_for_and_walks_the_caster_nowhere() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(50);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(6000, 5000),
    );
    let stood = world.transform.get(fiend).expect("standing").pos;
    let full = world.health.get(out).expect("standing").hp;
    let_go(&mut world, 1);
    world.step();
    assert_eq!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "a raze takes no aim, so there is nothing to walk into"
    );
    assert_eq!(
        world.health.get(out).expect("standing").hp,
        full,
        "and what stands past its reach is missed, not chased"
    );
    assert!(
        world
            .abilities
            .get(fiend)
            .is_some_and(|book| book.slots[1].cooldown > 0),
        "the cast itself went off"
    );
}

#[test]
fn one_point_levels_every_raze_at_once_and_costs_one() {
    let mut world = World::new();
    let fiend = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(2),
    );
    world.settle();
    let mut events = Vec::new();
    assert!(world.learn(fiend, 2, &mut events), "the point goes in");
    let levels: Vec<u8> = world
        .abilities
        .get(fiend)
        .expect("casts")
        .slots
        .iter()
        .map(|held| held.level)
        .collect();
    assert_eq!(
        levels,
        vec![1, 1, 1, 0, 0, 0],
        "every raze stands at one, and nothing else moved"
    );
    assert_eq!(
        world.points_spent(fiend),
        1,
        "the three cost one point together"
    );
    assert!(
        !world.learn(fiend, 3, &mut events),
        "and there is nothing left to spend"
    );
    // The trio waits for hero levels the same as any one ability would.
    world.level.insert(fiend, crate::game::Level(2));
    assert!(
        !world.learn(fiend, 0, &mut events),
        "the second raze level waits for hero level three"
    );
    assert!(
        world.learn(fiend, 3, &mut events),
        "while the necromastery is open to the spare point"
    );
}

#[test]
fn the_presence_wears_down_the_armor_of_enemies_near_its_carrier() {
    let (mut world, _fiend, near) = fiend_and_a_mark(400);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
    );
    world.step();
    let (worn, whole) = (
        world.stats.get(near).expect("standing").armor,
        world.stats.get(out).expect("standing").armor,
    );
    assert_eq!(
        worn,
        whole - Fixed::from_int(rules::PRESENCE_ARMOR[0]),
        "standing in the presence costs its armor"
    );
}

#[test]
fn souls_outlive_the_death_of_the_one_who_gathered_them() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.bury(vec![(fiend, None)], &mut events);
    assert_eq!(world.seats[0].unit, None, "the body is gone");
    for _ in 0..world.seats[0].respawn_left {
        world.step();
    }
    let back = world.seats[0].unit.expect("stands again");
    assert_eq!(
        world.stacks.get(back).map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_UNIT),
        "what was gathered comes back with him"
    );
}

#[test]
fn a_requiem_grows_with_the_souls_held_and_spends_none_of_them() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = world.health.get(mark).expect("standing").hp;
    let_go(&mut world, 5);
    world.step();
    world.step();
    let taken = full - world.health.get(mark).expect("standing").hp;
    assert!(taken > Fixed::ZERO, "everything near feels it: {taken:?}");
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(5),
        "and the souls are kept"
    );
    assert!(
        world.statuses.get(mark).is_some_and(|on_it| on_it
            .active()
            .any(|status| matches!(status.kind, StatusKind::Slowed { .. }))),
        "what it caught walks slower"
    );
}

#[test]
fn a_requiem_with_no_souls_gathered_touches_nobody() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let full = world.health.get(mark).expect("standing").hp;
    let_go(&mut world, 5);
    world.step();
    world.step();
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "nothing gathered is nothing let go"
    );
    assert!(
        world.statuses.get(mark).is_none_or(|on_it| on_it
            .active()
            .all(|status| !matches!(status.kind, StatusKind::Slowed { .. }))),
        "not even the slow"
    );
    let spent = world
        .abilities
        .get(fiend)
        .map_or(0, |book| book.slots[5].cooldown);
    assert!(spent > 0, "though the cast itself happened");
}

#[test]
fn a_structure_falling_beside_a_flesh_heap_feeds_it_nothing() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5200, 5000),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(tower, None)], &mut events);
    assert_eq!(
        world
            .stacks
            .get(pudge)
            .map_or(0, |kept| kept.of(StackKind::FleshHeap)),
        0,
    );
}

#[test]
fn a_structure_brought_down_is_worth_no_soul() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5600, 5000),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(tower, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map_or(0, |kept| kept.of(StackKind::Souls)),
        0,
    );
}

#[test]
fn the_view_carries_what_has_been_gathered_as_a_counted_effect() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.step();
    let view = world.view(bota_proto::Team::Radiant);
    let shown = view
        .units
        .iter()
        .find(|u| u.hero == Some(bota_proto::HeroId(2)))
        .expect("he is in the view");
    let souls = shown
        .effects
        .iter()
        .find(|e| e.stacks.is_some())
        .expect("what is gathered is on him");
    assert_eq!(souls.stacks, Some(rules::SOULS_PER_UNIT));
    assert_eq!(souls.ticks_left, None, "and nothing counts it down");
}

/// Lays a count of souls on a hero the way killing for them would.
fn hand_souls(world: &mut World, hero: Entity, many: u32) {
    let mut kept = world.stacks.get(hero).copied().unwrap_or_default();
    kept.set(crate::game::StackKind::Souls, many);
    world.stacks.insert(hero, kept);
}

/// The bug this guards against: a waypoint that routes a wave around its own
/// tower was cleared straight through the tower, and one Radiant mid creep of
/// every wave spent fifteen seconds wrestling its own tier three.
#[test]
fn no_creep_of_the_first_waves_is_left_wrestling_its_own_base() {
    let cfg = crate::game::MatchConfig {
        match_id: 7,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    for _ in 0..=rules::FIRST_WAVE_TICK {
        world.advance(&[]);
    }
    let first: Vec<(Entity, bota_proto::Vec2)> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .map(|e| {
            let team = world.team.get(e).copied().expect("has a side");
            let lane = world.lane.get(e).copied().expect("has a lane");
            let spawn = crate::game::creep_spawn_pos(world.map, team, lane.0);
            (e, spawn)
        })
        .collect();
    assert_eq!(
        first.len(),
        24,
        "four creeps a lane, three lanes, two sides"
    );
    for _ in 0..(30 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
    }
    // Half a minute in, everything still standing has long left its base;
    // what died, died out on the lane.
    for (creep, spawn) in &first {
        if !world.entities.iter().any(|e| e == *creep) {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            !at.within(*spawn, rules::units(1500)),
            "a creep is still beside its spawner at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
    // And the mid waves in particular met in the middle of no-man's land,
    // four against four.
    let meet = bota_proto::Vec2::from_ints(8706, 8838);
    for (creep, _) in &first {
        if !world.entities.iter().any(|e| e == *creep)
            || world.lane.get(*creep).copied() != Some(crate::game::Lane(rules::LANE_MID))
        {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            at.within(meet, rules::units(900)),
            "a mid creep never reached the river: ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
}

/// The standing structure at a spot, however it is guarded.
fn structure_at(world: &World, pos: bota_proto::Vec2) -> Entity {
    world
        .entities
        .iter()
        .find(|e| {
            world.transform.get(*e).is_some_and(|t| t.pos == pos)
                && world
                    .kind
                    .get(*e)
                    .copied()
                    .is_some_and(crate::game::is_structure)
        })
        .expect("a structure stands there")
}

/// Whether the structure at a spot may be struck.
fn open_at(world: &World, pos: bota_proto::Vec2) -> bool {
    let it = structure_at(world, pos);
    !world.stats.get(it).expect("settled").invulnerable
}

/// Takes a structure at a spot down the way a fight would.
fn fell_at(world: &mut World, pos: bota_proto::Vec2) {
    let it = structure_at(world, pos);
    let mut events = Vec::new();
    world.bury(vec![(it, None)], &mut events);
    world.step();
}

#[test]
fn a_lane_opens_tower_by_tower_into_its_barracks() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let t1 = rules::RADIANT_TOWERS[0].2;
    let t2 = rules::RADIANT_TOWERS[1].2;
    let t3 = rules::RADIANT_TOWERS[2].2;
    let melee_rax = rules::RADIANT_BARRACKS[0].2;
    let ranged_rax = rules::RADIANT_BARRACKS[1].2;
    assert!(open_at(&world, t1), "the first tower is open from the horn");
    assert!(!open_at(&world, t2), "the second waits on the first");
    assert!(!open_at(&world, t3), "the third waits on the second");
    assert!(
        !open_at(&world, melee_rax),
        "the barracks wait on the third"
    );
    fell_at(&mut world, t1);
    assert!(open_at(&world, t2), "the first fallen opens the second");
    assert!(!open_at(&world, t3), "and only the second");
    fell_at(&mut world, t2);
    assert!(open_at(&world, t3));
    assert!(!open_at(&world, melee_rax), "the barracks still wait");
    fell_at(&mut world, t3);
    assert!(open_at(&world, melee_rax), "the third fallen opens both");
    assert!(open_at(&world, ranged_rax));
}

#[test]
fn a_destroyed_structure_reopens_the_ground_it_blocked() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let tower = rules::RADIANT_TOWERS[0].2;
    assert!(
        !world.grid.walkable(tower),
        "a standing tower blocks its cell"
    );

    fell_at(&mut world, tower);

    assert!(
        world.grid.walkable(tower),
        "the tower's walkable terrain reopens after its destruction"
    );
}

#[test]
fn passability_changes_invalidate_cached_routes() {
    let mut world = World::new();
    let entity = world.spawn();
    world.route.insert(
        entity,
        crate::game::Route {
            path: vec![bota_proto::Vec2::from_ints(5_000, 5_000)],
            goal: bota_proto::Vec2::from_ints(6_000, 5_000),
            trace: Some(crate::game::TraceSide::Left),
        },
    );
    world.march.insert(
        entity,
        crate::game::March {
            route_step: 0,
            trace: Some(crate::game::TraceSide::Right),
            shove: 0,
        },
    );

    world.lay_passability();

    let route = world.route.get(entity).expect("route remains");
    assert!(route.path.is_empty());
    assert_eq!(route.trace, None);
    assert_eq!(world.march.get(entity).expect("march remains").trace, None);
}

#[test]
fn the_ancient_waits_for_both_tier_fours() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let t4_near = rules::RADIANT_TOWERS[9].2;
    let t4_far = rules::RADIANT_TOWERS[10].2;
    let ancient = rules::RADIANT_ANCIENT_POS;
    assert!(
        !open_at(&world, t4_near),
        "the tier fours wait on a broken lane"
    );
    fell_at(&mut world, rules::RADIANT_TOWERS[3].2);
    fell_at(&mut world, rules::RADIANT_TOWERS[4].2);
    fell_at(&mut world, rules::RADIANT_TOWERS[5].2);
    assert!(open_at(&world, t4_near), "any tier three fallen opens them");
    assert!(open_at(&world, t4_far));
    assert!(!open_at(&world, ancient), "the Ancient stands guarded");
    fell_at(&mut world, t4_near);
    assert!(
        !open_at(&world, ancient),
        "one tier four fallen is not enough"
    );
    fell_at(&mut world, t4_far);
    assert!(open_at(&world, ancient), "both fallen open the Ancient");
}

/// Steps the world to the next wave spawn and hands back that wave's mid
/// creeps of one side.
fn next_mid_wave(world: &mut World, team: bota_proto::Team) -> Vec<Entity> {
    let before: Vec<Entity> = world.entities.iter().collect();
    loop {
        world.advance(&[]);
        let fresh: Vec<Entity> = world
            .entities
            .iter()
            .filter(|e| {
                !before.contains(e)
                    && world.march.get(*e).is_some()
                    && world.lane.get(*e).copied() == Some(crate::game::Lane(rules::LANE_MID))
                    && world.team.get(*e).copied() == Some(team)
            })
            .collect();
        if !fresh.is_empty() {
            return fresh;
        }
    }
}

#[test]
fn a_fallen_barracks_turns_the_waves_against_it_super_and_all_of_them_mega() {
    let cfg = crate::game::MatchConfig {
        match_id: 9,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    // Whole barracks: plain waves.
    let plain = next_mid_wave(&mut world, bota_proto::Team::Dire);
    assert!(
        plain
            .iter()
            .all(|e| world.stats.get(*e).expect("settled").max_hp
                <= Fixed::from_int(rules::MELEE_CREEP_HP)),
        "no barracks down, nothing spawns super"
    );
    // The Radiant mid melee barracks falls: Dire mid melee go super, the
    // ranged stay plain, and Radiant's own creeps are untouched.
    fell_at(&mut world, rules::RADIANT_BARRACKS[0].2);
    let dire = next_mid_wave(&mut world, bota_proto::Team::Dire);
    let hp_of = |world: &World, e: Entity| world.stats.get(e).expect("settled").max_hp;
    assert!(
        dire.iter().any(
            |e| hp_of(&world, *e) >= Fixed::from_int(rules::SUPER_MELEE_HP)
                && world.kind.get(*e) == Some(&bota_proto::UnitKind::CreepMelee)
        ),
        "the melee spawn super"
    );
    assert!(
        dire.iter()
            .filter(|e| world.kind.get(**e) == Some(&bota_proto::UnitKind::CreepRanged))
            .all(|e| hp_of(&world, *e) < Fixed::from_int(rules::SUPER_RANGED_HP)),
        "the ranged do not"
    );
    let radiant = next_mid_wave(&mut world, bota_proto::Team::Radiant);
    assert!(
        radiant
            .iter()
            .all(|e| hp_of(&world, *e) <= Fixed::from_int(rules::MELEE_CREEP_HP)),
        "losing a barracks strengthens nobody's own creeps"
    );
    // Every Radiant barracks falls: Dire's waves go mega everywhere.
    for (_, _, at) in rules::RADIANT_BARRACKS.iter().skip(1) {
        fell_at(&mut world, *at);
    }
    let mega = next_mid_wave(&mut world, bota_proto::Team::Dire);
    let mega_melee = mega
        .iter()
        .find(|e| world.kind.get(**e) == Some(&bota_proto::UnitKind::CreepMelee))
        .expect("a melee creep spawned");
    assert_eq!(
        world
            .stats
            .get(*mega_melee)
            .expect("settled")
            .attack_interval,
        rules::MEGA_MELEE_ATTACK_INTERVAL,
        "a mega melee swings faster than a super one"
    );
}

#[test]
fn the_demo_waves_march_out_and_meet_between_the_towers() {
    let cfg = crate::game::MatchConfig {
        match_id: 11,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(1),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    for _ in 0..=rules::FIRST_WAVE_TICK {
        world.advance(&[]);
    }
    let first: Vec<(Entity, bota_proto::Vec2)> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .map(|e| {
            let team = world.team.get(e).copied().expect("has a side");
            (e, crate::game::creep_spawn_pos(world.map, team, 0))
        })
        .collect();
    assert_eq!(first.len(), 8, "one wave a side on the one lane");
    for _ in 0..(30 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
    }
    // Half a minute in the survivors are grinding in the middle of the
    // lane, nobody is left wrestling its own base, and with no Ancient
    // standing there is nothing to win by.
    let meet = bota_proto::Vec2::from_ints(8850, 9020);
    for (creep, spawn) in &first {
        if !world.entities.iter().any(|e| e == *creep) {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            !at.within(*spawn, rules::units(600)),
            "a creep is still beside its spawner at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
        assert!(
            at.within(meet, rules::units(900)),
            "a creep never reached the meet: ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
    assert_eq!(world.victor(), None, "the demo map has nothing to win by");
}

#[test]
fn the_fountain_melts_whoever_steps_into_its_reach_and_spares_who_stays_out() {
    let cfg = crate::game::MatchConfig {
        match_id: 13,
        master_key: [0; 32],
        picks: vec![bota_proto::Pick {
            slot: bota_proto::SlotId(0),
            team: bota_proto::Team::Dire,
            hero: bota_proto::HeroId(1),
        }],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.level.insert(hero, crate::game::Level(10));
    world.settle();
    world.fill_pools(hero);
    let fountain = rules::RADIANT_FOUNTAIN_POS;
    let hold = |world: &mut World, hero, at, ticks| {
        for _ in 0..ticks {
            world.advance(&[]);
            if let Some(t) = world.transform.get_mut(hero) {
                t.pos = at;
            }
        }
    };
    // Past its reach, nothing comes out.
    let outside = fountain + bota_proto::Vec2::from_ints(1400, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = outside;
    }
    let full = world.health.get(hero).expect("standing").hp;
    hold(&mut world, hero, outside, 45);
    assert_eq!(
        world.health.get(hero).expect("standing").hp,
        full,
        "out of reach the fountain leaves it be"
    );
    // Inside, a hero of the tenth level is torn apart within seconds.
    let inside = fountain + bota_proto::Vec2::from_ints(1000, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = inside;
    }
    // The first shots are still in the air: the reading starts once the
    // missiles have begun to land.
    hold(&mut world, hero, inside, 30);
    let before = world.health.get(hero).expect("standing").hp.to_int();
    hold(&mut world, hero, inside, 30);
    let after = world.health.get(hero).map_or(0, |h| h.hp.to_int());
    assert!(
        before - after > 1200,
        "a second under the fountain costs over 1200 health, not {}",
        before - after
    );
}

/// The bug this guards against: the demo map's shore rocks were read as
/// walkable ground, and the central water could be crossed anywhere rather
/// than through its two openings.
#[test]
fn the_demo_lake_is_walled_by_its_shore_and_crossed_at_its_ford() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let ground = crate::game::Ground::of(map);
    let cell = |x: i32, y: i32| {
        (
            (x / rules::GRID_CELL_SIZE) as usize,
            (y / rules::GRID_CELL_SIZE) as usize,
        )
    };
    // The lane's ford: shallow water, walked through.
    let (fx, fy) = cell(8768, 9024);
    assert!(ground.cell_walkable(fx, fy), "the ford is walked");
    assert!(
        ground.water(bota_proto::Vec2::from_ints(8768, 9024)),
        "and it is water underfoot"
    );
    // The rocks ringing the lake bar the way outside the openings.
    for (x, y) in [(7968, 8864), (8032, 8928)] {
        let (cx, cy) = cell(x, y);
        assert!(
            !ground.cell_walkable(cx, cy),
            "the shore at ({x},{y}) is not walked over"
        );
    }
    // The fountain structures close their own ground too.
    for at in map.fountains {
        let (cx, cy) = cell(at.x.to_int(), at.y.to_int());
        assert!(
            !ground.cell_walkable(cx, cy),
            "a fountain is stood beside, not inside"
        );
    }
}

/// The bug this guards against: the demo lane was drawn through the towers
/// the way the big map's lanes are, and every wave hooked around its own
/// tower instead of walking the road past it.
#[test]
fn the_demo_waves_walk_the_road_and_not_through_their_towers() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let towers = [
        rules::DEMO_RADIANT_TOWERS[0].2,
        rules::DEMO_DIRE_TOWERS[0].2,
    ];
    for team_at in 0..2 {
        let route = &crate::game::lane_routes(map)[team_at][0];
        assert!(!route.is_empty(), "the lane is walked");
        for waypoint in route {
            for tower in towers {
                assert!(
                    !waypoint.within(tower, rules::units(250)),
                    "a waypoint at ({},{}) aims into the tower at ({},{})",
                    waypoint.x.to_int(),
                    waypoint.y.to_int(),
                    tower.x.to_int(),
                    tower.y.to_int()
                );
            }
        }
    }
}

/// The bug this guards against: on lanes whose spawner stands ahead of its
/// own rearmost tower, the route began behind the wave, and every fresh
/// wave walked back to its own tower before turning around.
#[test]
fn no_route_on_any_map_walks_a_wave_backwards() {
    for map_id in [bota_proto::MapId(0), bota_proto::MapId(1)] {
        let map = crate::game::map_of(map_id);
        for (ti, team) in [bota_proto::Team::Radiant, bota_proto::Team::Dire]
            .into_iter()
            .enumerate()
        {
            for lane in map.lanes() {
                let route = &crate::game::lane_routes(map)[ti][usize::from(lane)];
                assert!(!route.is_empty(), "the lane is walked");
                let mut at = crate::game::creep_spawn_pos(map, team, lane);
                let mut last: Option<(i64, i64)> = None;
                for (i, w) in route.iter().enumerate() {
                    let d = (
                        i64::from(w.x.raw) - i64::from(at.x.raw),
                        i64::from(w.y.raw) - i64::from(at.y.raw),
                    );
                    if d == (0, 0) {
                        continue;
                    }
                    if let Some(p) = last {
                        assert!(
                            p.0 * d.0 + p.1 * d.1 >= 0,
                            "{team:?} lane {lane} on map {map_id:?} turns                              right around at waypoint {i} ({},{})",
                            w.x.to_int(),
                            w.y.to_int()
                        );
                    }
                    last = Some(d);
                    at = *w;
                }
            }
        }
    }
}

/// The bugs this guards against: a walker pressed into a knot of creeps and
/// crawled along it at slide speed, and a marcher wiggled at a wall of
/// bodies for ever, flipping sides every tick and never working round.
#[test]
fn walkers_and_marchers_both_work_round_a_wall_of_bodies() {
    let wall = |world: &mut World, x: i32, y: i32| {
        for i in -2..=2i32 {
            world.spawn_unit(
                &MELEE_CREEP,
                bota_proto::Team::Radiant,
                bota_proto::Vec2::from_ints(x, y + i * 40),
            );
        }
    };
    let arrives_by = |world: &mut World, mover: Entity, goal: bota_proto::Vec2, within: u32| {
        for t in 0..within {
            world.step();
            let at = world.transform.get(mover).expect("standing").pos;
            if at.within(goal, rules::units(50)) {
                return Some(t + 1);
            }
        }
        None
    };
    // A hero ordered through the knot gets round it briskly.
    let mut world = World::new();
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    wall(&mut world, 5400, 5000);
    world.settle();
    let goal = bota_proto::Vec2::from_ints(5900, 5000);
    world.set_order(hero, crate::game::UnitOrder::Move { pos: goal });
    let took = arrives_by(&mut world, hero, goal, 130);
    assert!(took.is_some(), "the hero works round the wall");
    // A marching creep held off any lane does the same.
    let mut world = World::new();
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 6000),
    );
    world.march.insert(
        creep,
        crate::game::March {
            route_step: 0,
            trace: None,
            shove: 0,
        },
    );
    wall(&mut world, 5400, 6000);
    world.settle();
    let goal = bota_proto::Vec2::from_ints(5900, 6000);
    world.set_order(creep, crate::game::UnitOrder::AttackMove { pos: goal });
    let took = arrives_by(&mut world, creep, goal, 130);
    assert!(took.is_some(), "the creep works round the wall");
}

fn nearby_elevations(
    ground: &crate::game::Ground,
) -> (bota_proto::Vec2, bota_proto::Vec2, bota_proto::Vec2) {
    const RADIUS: usize = 6;
    for sy in RADIUS..crate::game::TERRAIN_CELLS - RADIUS {
        for sx in RADIUS..crate::game::TERRAIN_CELLS - RADIUS {
            if !ground.cell_walkable(sx, sy) {
                continue;
            }
            let source = bota_proto::Vec2::from_ints(
                sx as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
                sy as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
            );
            let source_tier = ground.tier(source);
            let mut level = None;
            let mut uphill = None;
            for ty in sy - RADIUS..=sy + RADIUS {
                for tx in sx - RADIUS..=sx + RADIUS {
                    if (tx == sx && ty == sy) || !ground.cell_walkable(tx, ty) {
                        continue;
                    }
                    let target = bota_proto::Vec2::from_ints(
                        tx as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
                        ty as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
                    );
                    match ground.tier(target).cmp(&source_tier) {
                        std::cmp::Ordering::Equal => level = Some(target),
                        std::cmp::Ordering::Greater => uphill = Some(target),
                        std::cmp::Ordering::Less => {}
                    }
                    if let (Some(level), Some(uphill)) = (level, uphill) {
                        return (source, level, uphill);
                    }
                }
            }
        }
    }
    panic!("the Dota terrain needs a walkable elevation boundary");
}

const UPHILL_TEST_ATTACKER: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 1,
    attack_interval: 1,
    attack_point: 1,
    projectile_speed: Some(6_000),
    move_speed: 0,
    ..RANGED_CREEP
};
const UPHILL_TEST_FLYING_ATTACKER: UnitDef = UnitDef {
    flies: true,
    ..UPHILL_TEST_ATTACKER
};
const UPHILL_TEST_TARGET: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 0,
    move_speed: 0,
    armor: 0,
    ..MELEE_CREEP
};
const UPHILL_TEST_BUILDING: UnitDef = UnitDef {
    kind: bota_proto::UnitKind::Tower,
    move_speed: 0,
    ..UPHILL_TEST_TARGET
};

fn ranged_damage_after_ticks(
    source: bota_proto::Vec2,
    target: bota_proto::Vec2,
    attacker_def: &'static UnitDef,
    target_def: &'static UnitDef,
) -> i32 {
    const OBSERVER: UnitDef = UnitDef {
        max_hp: 8_000,
        damage: 0,
        move_speed: 0,
        vision: 2_000,
        ..MELEE_CREEP
    };
    const TICKS: u32 = 512;

    let mut world = World::new();
    let attacker = world.spawn_unit(attacker_def, Team::Radiant, source);
    let mark = world.spawn_unit(target_def, Team::Dire, target);
    world.spawn_unit(&OBSERVER, Team::Radiant, target);
    world.settle();
    world.set_target(attacker, mark);
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = crate::game::facing_towards(source, target);
    }
    let before = world.health.get(mark).expect("target health").hp;
    for _ in 0..TICKS {
        world.step();
    }
    let after = world.health.get(mark).expect("target survives").hp;
    (before - after).to_int()
}

#[test]
fn ranged_attacks_can_miss_uphill_but_not_on_level_ground() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (source, level, uphill) = nearby_elevations(&ground);

    let level_damage =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    let uphill_damage =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);

    assert!(level_damage > 0, "the level-ground control must attack");
    assert!(uphill_damage < level_damage, "uphill attacks must miss");
    assert!(
        uphill_damage > level_damage / 2,
        "the uphill miss rate must stay near one quarter"
    );
}

#[test]
fn buildings_and_flying_attackers_are_exempt_from_uphill_misses() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (source, level, uphill) = nearby_elevations(&ground);

    let building_level =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let building_uphill =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let flying_level = ranged_damage_after_ticks(
        source,
        level,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );
    let flying_uphill = ranged_damage_after_ticks(
        source,
        uphill,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );

    assert_eq!(building_uphill, building_level, "buildings do not evade");
    assert_eq!(flying_uphill, flying_level, "flying attacks do not miss");
}

fn manual_projectile_damage(
    source_pos: bota_proto::Vec2,
    target_pos: bota_proto::Vec2,
    launch_tier: u8,
) -> i32 {
    const SHOTS: usize = 512;
    let mut world = World::new();
    let source = world.spawn_unit(&UPHILL_TEST_TARGET, Team::Radiant, source_pos);
    let target = world.spawn_unit(&UPHILL_TEST_TARGET, Team::Dire, target_pos);
    world.settle();
    world.hull.remove(source);
    let before = world.health.get(target).expect("target health").hp;
    for _ in 0..SHOTS {
        let missile = world.spawn();
        world.transform.insert(
            missile,
            crate::game::Transform {
                pos: target_pos,
                facing: bota_proto::Angle::default(),
            },
        );
        world.set_team(missile, Team::Radiant);
        world.projectile.insert(
            missile,
            crate::game::Projectile {
                speed: Fixed::ONE,
                source: Some(source),
                target,
                damage: 1,
                kind: bota_proto::DamageKind::Physical,
                ability: None,
                launch_tier,
                can_miss_uphill: true,
                crit: false,
                bounces_left: 0,
                bounce_range: 0,
                bounced: Vec::new(),
            },
        );
    }

    world.step();

    let after = world.health.get(target).expect("target survives").hp;
    (before - after).to_int()
}

#[test]
fn uphill_eligibility_uses_attacker_and_target_elevation_at_impact() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (low, _, high) = nearby_elevations(&ground);
    let low_tier = ground.tier(low);
    let high_tier = ground.tier(high);

    let moved_up = manual_projectile_damage(high, high, low_tier);
    let moved_down = manual_projectile_damage(low, high, high_tier);

    assert_eq!(
        moved_up, 512,
        "an attacker now level with its target does not miss"
    );
    assert!(
        moved_down < moved_up,
        "an attacker now below its target can miss"
    );
    assert!(
        moved_down > moved_up / 2,
        "the miss rate stays near one quarter"
    );
}
