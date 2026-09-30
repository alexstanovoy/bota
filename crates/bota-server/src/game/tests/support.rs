//! Fixtures and helpers the game tests share.

use crate::game::rules;
use crate::game::{
    Def, Entity, Health, MELEE_CREEP, Modifiers, RANGED_CREEP, Stats, UnitDef, World,
};
use bota_proto::{Attributes, Fixed, Team};

/// A stat block with every field named, so a new one has to be thought about
/// here before any test compiles again.
pub(super) fn stats() -> Stats {
    Stats {
        max_hp: Fixed::from_int(20),
        max_mana: Fixed::from_int(20),
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        damage: 0,
        attack_range: Fixed::ZERO,
        acquisition: Fixed::ZERO,
        attack_time: 1000,
        attack_speed: 100,
        attributes: Attributes::ZERO,
        primary: None,
        attack_point: 0,
        attack_backswing: 0,
        projectile_speed: None,
        armor: Fixed::ZERO,
        magic_resist_pct: 0,
        status_resist_bp: 0,
        physical_amp_bp: rules::NOMINAL_BP,
        magic_amp_bp: rules::NOMINAL_BP,
        pure_amp_bp: rules::NOMINAL_BP,
        cooldown_rate_bp: rules::NOMINAL_BP,
        mana_cost_rate_bp: rules::NOMINAL_BP,
        move_speed: Fixed::ZERO,
        turn_rate: 0,
        damage_to_creeps: 0,
        vision: Fixed::ZERO,
        true_sight: Fixed::ZERO,
        hides: false,
        flies: false,
        phased: false,
        invulnerable: false,
        evasion: crate::game::Ratio::NEVER,
        pierce: crate::game::Ratio::NEVER,
        pierce_damage: 0,
    }
}

/// A pool holding a whole number of points.
pub(super) fn health(hp: i32) -> Health {
    Health {
        hp: Fixed::from_int(hp),
    }
}

/// A creep with nothing done to it.
pub(super) fn plain_creep(world: &mut World) -> Entity {
    let creep = world.spawn();
    world.def.insert(creep, Def(&MELEE_CREEP));
    world.health.insert(creep, Health { hp: Fixed::ZERO });
    world.modifiers.insert(creep, Modifiers(Vec::new()));
    creep
}

/// The health a hero of the plain kind holds so many levels past the first,
/// counting what its strength is worth.
pub(super) fn body_at(levels: i32) -> Fixed {
    let strength = rules::HERO_ATTRIBUTES.strength
        + rules::HERO_ATTRIBUTES_PER_LEVEL.strength * Fixed::from_int(levels);
    Fixed::from_int(rules::HERO_HP + levels * rules::HERO_HP_PER_LEVEL)
        + Fixed::from_int(rules::HP_PER_STRENGTH) * strength
}

/// Buildings a map stands up: its towers, its barracks, both fountains, and
/// whatever Ancients it has.
pub(super) fn map_buildings(map: &crate::game::MapDef) -> usize {
    map.radiant_towers.len()
        + map.dire_towers.len()
        + map.barracks[0].len()
        + map.barracks[1].len()
        + 2
        + map.ancients.iter().flatten().count()
}

/// How long the salve mends for.
pub(super) fn salve_ticks() -> u32 {
    crate::game::SALVE_TICKS
}

/// A match config that always names the same numbers.
pub(super) fn config() -> crate::game::MatchConfig {
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
        cheats: false,
        spawn_modifiers: Vec::new(),
    }
}

pub(super) fn advance_validated(
    world: &mut World,
    unit: Option<Entity>,
    order: bota_proto::Order,
) -> Vec<crate::game::Event> {
    let command = crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: unit.map(crate::game::wire_id),
        order,
    };
    assert_eq!(
        world.validate_order(command.slot, command.unit, &command.order),
        Ok(())
    );
    let tick = world.tick;
    let events = world.advance(&[command]);
    assert_eq!(world.tick, tick + 1);
    events
}

pub(super) fn world_with_projectile_uphill_state(launch_tier: u8, can_miss_uphill: bool) -> World {
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
            damage_amp_bp: rules::NOMINAL_BP,
            ability: None,
            launch_tier,
            can_miss_uphill,
            crit: false,
            pierces: false,
            pierce_damage: 0,
            pierce_amp_bp: rules::NOMINAL_BP,
            bounces_left: 0,
            bounce_range: 0,
            bounced: Vec::new(),
        },
    );
    world
}

/// A lane creep on clear ground with a mind of its own.
pub(super) fn thinking_creep(world: &mut World, at: bota_proto::Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    world.lane_ai.insert(
        creep,
        crate::game::LaneAi {
            last_seen: None,
            keep_until: 0,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    creep
}

/// A hero that has learned one ability to its first level, with mana to spend.
pub(super) fn caster(world: &mut World, at: bota_proto::Vec2, slot: usize) -> Entity {
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

/// Runs one tick of the attack cycle and nothing else.
pub(super) fn swing_once(world: &mut World) {
    world.run_actions();
}

/// Who an entity is mid-swing at, if it is mid-swing at all.
pub(super) fn swinging(world: &World, entity: Entity) -> Option<Entity> {
    match world.action.get(entity).map(|action| action.state) {
        Some(crate::game::ActionState::Attack {
            target,
            phase: crate::game::ActionPhase::Before { .. },
        }) => Some(target),
        _ => None,
    }
}

/// Whether an entity is recovering from a swing that landed.
pub(super) fn recovering(world: &World, entity: Entity) -> bool {
    matches!(
        world.action.get(entity).map(|action| action.state),
        Some(crate::game::ActionState::Attack {
            phase: crate::game::ActionPhase::After { .. },
            ..
        })
    )
}

/// The tick a span of milliseconds is crossed on at the base attack speed.
pub(super) fn ticks_of(ms: u32) -> u32 {
    (ms * rules::TICKS_PER_SECOND).div_ceil(1000)
}

/// An attacker and its mark, standing where they are put and nothing else.
pub(super) fn duel(gap: i32) -> (World, Entity, Entity) {
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

/// A seated hero ordered at an enemy standing in plain sight `apart` away.
pub(super) fn hero_ordered_at_an_enemy(apart: i32) -> (World, Entity, Entity) {
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

/// The same seat and enemy, with a follow order in place of the attack.
pub(super) fn hero_following_an_enemy(apart: i32) -> (World, Entity, Entity) {
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

/// A hero, an enemy standing in its way, and the order it was given.
pub(super) fn hero_past_an_enemy(order: crate::game::UnitOrder) -> (World, Entity, Entity) {
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
            pending: None,
        },
    );
    (world, hero, enemy)
}

/// A hero of a side and one of its own creeps beside it.
pub(super) fn hero_and_own_creep() -> (World, Entity, Entity) {
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

/// A hero of one side, a creep of the other with a mind of its own, one of the
/// hero's own creeps for that creep to prefer, and an enemy hero to point at.
///
/// The enemy creep stands `apart` from the hero. Only an order at the enemy
/// hero calls creeps on, so that is what the pull tests click.
pub(super) fn a_lane_with_a_hero(apart: i32) -> (World, Entity, Entity, Entity, Entity) {
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
pub(super) fn thinking_creep_of(world: &mut World, team: Team, at: bota_proto::Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.lane_ai.insert(
        creep,
        crate::game::LaneAi {
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
pub(super) fn attack_click(world: &mut World, on: Entity) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(on)),
        },
    }]);
}

/// How far apart two entities stand, along the lane.
pub(super) fn gap_along_lane(world: &World, one: Entity, other: Entity) -> i32 {
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

/// The effect a fountain hands out.
pub(super) const FOUNTAIN_EFFECT: crate::game::ModifierKind = crate::game::ModifierKind::Fountain {
    hp_per_tick: rules::FOUNTAIN_HEAL_HP_PER_TICK * 100,
    mana_per_tick: rules::FOUNTAIN_HEAL_MANA_PER_TICK * 100,
};

/// Whether a unit carries an effect of one kind right now, whatever there is
/// of it.
pub(super) fn carries(world: &World, entity: Entity, kind: crate::game::ModifierKind) -> bool {
    let same = std::mem::discriminant(&kind);
    world.modifiers.get(entity).is_some_and(|on_it| {
        on_it
            .active()
            .any(|held| std::mem::discriminant(&held.kind) == same)
    })
}

/// Whether a unit has its rot switched on.
pub(super) fn rotting(world: &World, entity: Entity) -> bool {
    carries(world, entity, crate::game::ModifierKind::Rot { level: 0 })
}

/// A tower's protection, whatever tier hands it out.
pub(super) const GUARDED_EFFECT: crate::game::ModifierKind = crate::game::ModifierKind::Guarded {
    armor: rules::TOWER_AURA_ARMOR[0],
    hp_per_second: rules::TOWER_AURA_REGEN[0],
};

/// A flagbearer's inspiration.
pub(super) const INSPIRED_EFFECT: crate::game::ModifierKind = crate::game::ModifierKind::Inspired {
    hp_per_second: rules::FLAGBEARER_AURA_REGEN,
};

/// A match world with one hero standing at its own shop, an item in its stash.
pub(super) fn a_hero_at_the_shop() -> (World, Entity, bota_proto::ItemId) {
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

/// Walkable ground out in the open: nothing standing near it, and the
/// ground clear for a short walk in every direction it is put to.
pub(super) fn an_empty_spot(world: &World) -> bota_proto::Vec2 {
    for row in 0..16 {
        for step in 0..60 {
            let at = bota_proto::Vec2::from_ints(5600 + step * 100, 6000 + row * 200);
            let clear = world.entities.iter().all(|entity| {
                !world
                    .transform
                    .get(entity)
                    .is_some_and(|t| t.pos.within(at, bota_proto::Fixed::from_int(800)))
            });
            let room = crate::game::plan_radius(rules::units(rules::HERO_COLLISION));
            let open = (-2..=14).all(|dx: i32| {
                (-2..=2).all(|dy: i32| {
                    world
                        .clearance
                        .point_clear(at + bota_proto::Vec2::from_ints(dx * 50, dy * 100), room)
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
pub(super) fn a_hero_with_a_scroll() -> (World, Entity) {
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
pub(super) fn beside_own_tower(world: &World) -> bota_proto::Vec2 {
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
    if world.clearance.walkable(beside) {
        beside
    } else {
        at + bota_proto::Vec2::from_ints(-300, 0)
    }
}

/// A hero out in the open with one ward of a kind in its first slot.
pub(super) fn a_hero_with_a_ward(item: u16) -> (World, Entity, bota_proto::Vec2) {
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
pub(super) fn the_ward(world: &World) -> Entity {
    world
        .entities
        .iter()
        .find(|entity| world.kind.get(*entity) == Some(&bota_proto::UnitKind::Ward))
        .expect("a ward stands")
}

/// A hero on the big map standing beside the forest, with one item in hand.
pub(super) fn a_hero_by_the_trees(item: u16, charges: u8) -> (World, Entity, bota_proto::Vec2) {
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let tree = crate::game::tree_positions(map)
        .into_iter()
        .find(|at| {
            world
                .clearance
                .stands_clear(*at + bota_proto::Vec2::from_ints(120, 0))
                && world
                    .clearance
                    .stands_clear(*at + bota_proto::Vec2::from_ints(240, 0))
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
pub(super) fn sight_stopped_at(world: &World, at: bota_proto::Vec2) -> bool {
    crate::game::CellGrid::cell_of(at).is_some_and(|(cx, cy)| !world.sight_block.cell_open(cx, cy))
}

/// How long a tango eaten off the tree at a spot mends for.
pub(super) fn tango_ticks(world: &mut World, hero: Entity, at: bota_proto::Vec2) -> u32 {
    world
        .modifiers
        .insert(hero, crate::game::Modifiers(Vec::new()));
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
    assert!(world.use_item(hero, 1, bota_proto::Target::Pos(at), &mut Vec::new()));
    world
        .modifiers
        .get(hero)
        .expect("it mends")
        .active()
        .find(|held| matches!(held.kind, crate::game::ModifierKind::Mending { .. }))
        .expect("of health")
        .ticks_left
        .expect("for a while")
}

/// What one swing of an entity takes off another, run to the blow itself.
pub(super) fn one_swing_takes(world: &mut World, from: Entity, on: Entity) -> i32 {
    world.set_target(from, on);
    if let Some(action) = world.action.get_mut(from) {
        action.state = crate::game::ActionState::Ready;
        action.attack_cooldown = 0;
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

/// Puts an effect on a unit for a while.
pub(super) fn put_on(
    world: &mut World,
    entity: Entity,
    kind: crate::game::ModifierKind,
    ticks: u32,
) {
    world.put_modifier(
        entity,
        crate::game::Modifier {
            kind,
            source: None,
            ticks_left: Some(ticks),
        },
    );
}

/// Pudge standing in the open with his hook learned, and an enemy creep a
/// way off in front of him.
pub(super) fn pudge_and_a_mark(apart: i32) -> (World, Entity, Entity) {
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
pub(super) fn throw_hook(world: &mut World, at: bota_proto::Vec2) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(0),
            target: bota_proto::Target::Pos(at),
        },
    }]);
}

/// Sends one of Pudge's abilities the way a player does.
pub(super) fn pudge_casts(world: &mut World, slot: u8, target: bota_proto::Target) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(slot),
            target,
        },
    }]);
}

/// Lays one item in a hero's first slot.
pub(super) fn hand_item(world: &mut World, hero: Entity, item: u16, charges: u8) {
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

/// A camp of two kobolds at a spot, asleep and at home.
pub(super) fn a_camp_at(world: &mut World, at: bota_proto::Vec2) -> Vec<Entity> {
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

/// Sends the bolt at somebody the way a player does.
pub(super) fn cast_at(world: &mut World, mark: Entity) {
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Cast {
            slot: bota_proto::AbilitySlot(2),
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
}

/// The courier of the first seat, while one stands.
pub(super) fn the_courier(world: &World) -> Entity {
    world.seats[0].courier.expect("a seat has a courier")
}

pub(super) fn courier_on_delivery() -> (World, Entity, Entity) {
    use bota_proto::{AbilitySlot, Order, Target};

    let (mut world, hero) = a_hero_with_gold(0);
    let courier = the_courier(&world);
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    hand_item(&mut world, courier, crate::game::ITEM_BOOTS, 0);
    advance_validated(
        &mut world,
        Some(courier),
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::None,
        },
    );
    world.advance(&[]);
    assert_eq!(
        world.errand.get(courier),
        Some(&crate::game::Errand::ToOwner)
    );
    assert!(slot_of(&world, courier, 0).is_some());
    (world, hero, courier)
}

/// A hero of the plain kind, standing at its own shop with gold in hand.
pub(super) fn a_hero_with_gold(gold: i32) -> (World, Entity) {
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
pub(super) fn slot_of(world: &World, hero: Entity, at: usize) -> Option<crate::game::ItemStack> {
    world.inventory.get(hero).and_then(|bag| bag.slots[at])
}

/// What one item of the catalog costs.
pub(super) fn price_of(item: u16) -> i32 {
    crate::game::ITEMS[usize::from(item)].cost
}

pub(super) fn reject_purchase_without_mutation(
    world: &mut World,
    hero: Entity,
    item: bota_proto::ItemId,
    reason: bota_proto::RejectReason,
) {
    let bag = world.inventory.get(hero).expect("bag").clone();
    let stash = world.seats[0].stash.clone();
    let gold = world.seats[0].gold;
    let order = bota_proto::Order::Buy { item };
    for unit in [None, Some(crate::game::wire_id(the_courier(world)))] {
        assert_eq!(
            world.validate_order(bota_proto::SlotId(0), unit, &order),
            Err(reason)
        );
    }
    let mut events = Vec::new();
    assert!(!world.buy(bota_proto::SlotId(0), item, &mut events));
    assert_eq!(world.inventory.get(hero), Some(&bag));
    assert_eq!(world.seats[0].stash, stash);
    assert_eq!(world.seats[0].gold, gold);
    assert!(events.is_empty());
}

/// Every item lying on the ground.
pub(super) fn on_the_ground(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| world.loot.get(*e).is_some())
        .collect()
}

/// A plain stack of one item for a seat, already touched.
pub(super) fn a_stack_of(item: u16, owner: u8) -> crate::game::ItemStack {
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

/// A world with a wall of closed ground between two spots, so the way from
/// one to the other has to be found rather than walked straight.
pub(super) fn a_world_with_a_wall(from: bota_proto::Vec2, to: bota_proto::Vec2) -> World {
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
    stand_a_wall(&mut world, middle, rules::units(600));
    assert!(
        !world.clearance.capsule_clear(from, to, Fixed::ZERO),
        "the wall stands in the way"
    );
    world
}

/// Stands a circle of closed ground on a world, beside everything else
/// standing on it.
pub(super) fn stand_a_wall(world: &mut World, at: bota_proto::Vec2, radius: Fixed) {
    let mut circles = world.clearance.circles().to_vec();
    circles.push((at, radius));
    world.clearance.set_circles(circles);
}

/// Shadow Fiend at the middle of the map, facing east, with a creep `apart`
/// to the east of him and his whole kit learned to its first level.
pub(super) fn fiend_and_a_mark(apart: i32) -> (World, Entity, Entity) {
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
pub(super) fn let_go(world: &mut World, slot: u8) {
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
pub(super) fn a_creep_at(
    world: &mut World,
    team: bota_proto::Team,
    at: bota_proto::Vec2,
) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.settle();
    creep
}

/// Lays a count of souls on a hero the way killing for them would.
pub(super) fn hand_souls(world: &mut World, hero: Entity, many: u32) {
    let mut kept = world.stacks.get(hero).copied().unwrap_or_default();
    kept.set(crate::game::StackKind::Souls, many);
    world.stacks.insert(hero, kept);
}

/// The standing structure at a spot, however it is guarded.
pub(super) fn structure_at(world: &World, pos: bota_proto::Vec2) -> Entity {
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
pub(super) fn open_at(world: &World, pos: bota_proto::Vec2) -> bool {
    let it = structure_at(world, pos);
    !world.stats.get(it).expect("settled").invulnerable
}

/// Takes a structure at a spot down the way a fight would.
pub(super) fn fell_at(world: &mut World, pos: bota_proto::Vec2) {
    let it = structure_at(world, pos);
    let mut events = Vec::new();
    world.bury(vec![(it, None)], &mut events);
    world.step();
}

/// Steps the world to the next wave spawn and hands back that wave's mid
/// creeps of one side.
pub(super) fn next_mid_wave(world: &mut World, team: bota_proto::Team) -> Vec<Entity> {
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

/// How far along a lane's centerline a point stands, in world units: the
/// length of the line up to the point's foot on its nearest segment.
pub(super) fn lane_progress(line: &[bota_proto::Vec2], pos: bota_proto::Vec2) -> i64 {
    let at = |v: bota_proto::Vec2| (i64::from(v.x.to_int()), i64::from(v.y.to_int()));
    let (px, py) = at(pos);
    let mut nearest: Option<(i64, i64)> = None;
    let mut walked = 0;
    for seg in line.windows(2) {
        let (ax, ay) = at(seg[0]);
        let (bx, by) = at(seg[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1);
        let len = len2.isqrt();
        let t = ((px - ax) * dx + (py - ay) * dy).clamp(0, len2);
        let (fx, fy) = (ax + dx * t / len2, ay + dy * t / len2);
        let off = (px - fx).pow(2) + (py - fy).pow(2);
        if nearest.is_none_or(|(had, _)| off < had) {
            nearest = Some((off, walked + t / len));
        }
        walked += len;
    }
    nearest.map_or(0, |(_, progress)| progress)
}

pub(super) fn nearby_elevations(
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

pub(super) const UPHILL_TEST_ATTACKER: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 1,
    attack_time: 33,
    attack_point: 33,
    attack_backswing: 0,
    projectile_speed: Some(6_000),
    move_speed: 0,
    ..RANGED_CREEP
};

pub(super) const UPHILL_TEST_FLYING_ATTACKER: UnitDef = UnitDef {
    flies: true,
    ..UPHILL_TEST_ATTACKER
};

pub(super) const UPHILL_TEST_TARGET: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 0,
    move_speed: 0,
    armor: 0,
    ..MELEE_CREEP
};

pub(super) const UPHILL_TEST_BUILDING: UnitDef = UnitDef {
    kind: bota_proto::UnitKind::Tower,
    move_speed: 0,
    ..UPHILL_TEST_TARGET
};

pub(super) fn ranged_damage_after_ticks(
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

/// Blows an attacker landed and missed on a target over the same run as
/// [`ranged_damage_after_ticks`], counted from what the ticks told of.
pub(super) fn ranged_swings_after_ticks(
    source: bota_proto::Vec2,
    target: bota_proto::Vec2,
    attacker_def: &'static UnitDef,
    target_def: &'static UnitDef,
) -> (usize, usize) {
    const OBSERVER: UnitDef = UnitDef {
        max_hp: 8_000,
        damage: 0,
        move_speed: 0,
        vision: 2_000,
        ..MELEE_CREEP
    };
    let mut world = World::new();
    let attacker = world.spawn_unit(attacker_def, Team::Radiant, source);
    let mark = world.spawn_unit(target_def, Team::Dire, target);
    world.spawn_unit(&OBSERVER, Team::Radiant, target);
    world.settle();
    world.set_target(attacker, mark);
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = crate::game::facing_towards(source, target);
    }
    let (mut landed, mut missed) = (0, 0);
    for _ in 0..512 {
        for event in world.step() {
            match event.kind {
                bota_proto::EventKind::Damaged { source, target, .. }
                    if source == Some(crate::game::wire_id(attacker))
                        && target == crate::game::wire_id(mark) =>
                {
                    landed += 1;
                }
                bota_proto::EventKind::Missed { source, target }
                    if source == Some(crate::game::wire_id(attacker))
                        && target == crate::game::wire_id(mark) =>
                {
                    missed += 1;
                }
                _ => {}
            }
        }
    }
    (landed, missed)
}

pub(super) fn manual_projectile_damage(
    source_pos: bota_proto::Vec2,
    target_pos: bota_proto::Vec2,
    launch_tier: u8,
    pierces: bool,
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
                damage_amp_bp: rules::NOMINAL_BP,
                ability: None,
                launch_tier,
                can_miss_uphill: true,
                crit: false,
                pierces,
                pierce_damage: 0,
                pierce_amp_bp: rules::NOMINAL_BP,
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

/// Steps a world until a mover stands within fifty units of a spot, and
/// how many ticks that took. None when it never got there in time.
pub(super) fn ticks_until_near(
    world: &mut World,
    mover: Entity,
    goal: bota_proto::Vec2,
    within: u32,
) -> Option<u32> {
    for t in 0..within {
        world.step();
        let at = world.transform.get(mover).expect("standing").pos;
        if at.within(goal, rules::units(50)) {
            return Some(t + 1);
        }
    }
    None
}

/// How far along the lane the Radiant wave stands: its front, and all of
/// it on average.
pub(super) fn wave_progress(world: &World, route: &[bota_proto::Vec2]) -> (i64, i64) {
    let each: Vec<i64> = world
        .entities
        .iter()
        .filter(|e| {
            world.march.get(*e).is_some() && world.team.get(*e) == Some(&bota_proto::Team::Radiant)
        })
        .map(|e| lane_progress(route, world.transform.get(e).expect("standing").pos))
        .collect();
    let front = each.iter().copied().max().unwrap_or(0);
    let mean = if each.is_empty() {
        0
    } else {
        each.iter().sum::<i64>() / each.len() as i64
    };
    (front, mean)
}

/// The spot a progress along a polyline lands on, shifted sideways.
pub(super) fn along_lane(
    route: &[bota_proto::Vec2],
    progress: i64,
    aside: i64,
) -> bota_proto::Vec2 {
    let mut left = progress;
    for (i, seg) in route.windows(2).enumerate() {
        let (a, b) = (seg[0], seg[1]);
        let len = crate::game::isqrt64(a.distance_squared(b)) >> 16;
        if len >= left || i + 2 == route.len() {
            let dx = i64::from(b.x.to_int() - a.x.to_int());
            let dy = i64::from(b.y.to_int() - a.y.to_int());
            let len = len.max(1);
            let x = i64::from(a.x.to_int()) + dx * left / len - dy * aside / len;
            let y = i64::from(a.y.to_int()) + dy * left / len + dx * aside / len;
            return bota_proto::Vec2::from_ints(x as i32, y as i32);
        }
        left -= len;
    }
    route[route.len() - 1]
}
