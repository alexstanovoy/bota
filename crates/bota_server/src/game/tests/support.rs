//! Fixtures and helpers the game tests share.

use bota_core::{
    CellGrid, ITEMS, MELEE_CREEP, MapDef, ModifierKind, NeutralKind, Odds, RANGED_CREEP, UnitDef,
    facing_towards, isqrt64, kind_def, map_of, rules,
};

use crate::game::{
    ActionPhase, ActionState, CampHome, Command, Def, Entity, Errand, Event, Ground, Health,
    HitEffect, ItemStack, Lane, LaneAi, Level, MatchConfig, Modifier, Modifiers, NeutralAi, Orders,
    Projectile, Seat, Stats, TERRAIN_CELLS, Transform, UnitOrder, World, plan_radius,
    tree_positions, wire_id,
};
use bota_proto::{
    AbilitySlot, Angle, Attributes, DamageKind, EventKind, Fixed, HeroId, ItemId, MapId, Order,
    Pick, RejectReason, SlotId, Target, Team, TickMode, UnitKind, UnitView, Vec2,
};

pub(crate) fn stats() -> Stats {
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
        magic_resist_bp: 0,
        status_resist_bp: 0,
        physical_amp_bp: rules::NOMINAL_BP,
        magic_amp_bp: rules::NOMINAL_BP,
        pure_amp_bp: rules::NOMINAL_BP,
        cooldown_rate_bp: rules::NOMINAL_BP,
        mana_cost_rate_bp: rules::NOMINAL_BP,
        move_speed: Fixed::ZERO,
        turn_rate: 0,
        damage_spread: 0,
        damage_to_creeps: 0,
        vision: Fixed::ZERO,
        true_sight: Fixed::ZERO,
        hides: false,
        flies: false,
        phased: false,
        invulnerable: false,
        glyph: false,
        evasion: Odds::NEVER,
        lifesteal_pct: 0,
        magic_taken_pct: 0,
        accuracy: Odds::NEVER,
    }
}

pub(crate) fn health(hp: i32) -> Health {
    Health {
        hp: Fixed::from_int(hp),
    }
}

/// A bare melee creep: its def, zero health and no modifiers, nothing else.
pub(crate) fn plain_creep(world: &mut World) -> Entity {
    let creep = world.spawn();
    world.def.insert(creep, Def(&MELEE_CREEP));
    world.health.insert(creep, Health { hp: Fixed::ZERO });
    world.modifiers.insert(creep, Modifiers(Vec::new()));
    creep
}

/// The health a hero of the plain kind holds so many levels past the first,
/// counting what its strength is worth.
pub(crate) fn body_at(levels: i32) -> Fixed {
    let strength = rules::HERO_ATTRIBUTES.strength
        + rules::HERO_ATTRIBUTES_PER_LEVEL.strength * Fixed::from_int(levels);
    Fixed::from_int(rules::HERO_HP + levels * rules::HERO_HP_PER_LEVEL)
        + Fixed::from_int(rules::HP_PER_STRENGTH) * strength
}

/// Buildings a map stands up: its towers, its barracks, both fountains, and
/// whatever Ancients it has.
pub(crate) fn map_buildings(map: &MapDef) -> usize {
    map.radiant_towers.len()
        + map.dire_towers.len()
        + map.barracks[0].len()
        + map.barracks[1].len()
        + map.fillers[0].len()
        + map.fillers[1].len()
        + 2
        + map.ancients.iter().flatten().count()
}

pub(crate) fn config() -> MatchConfig {
    MatchConfig {
        starting_gold_lost: Vec::new(),
        match_id: 7,
        stream: 7,
        master_key: [3; 32],
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::SHADOW_FIEND,
        }],
        map: MapId(1),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 30,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// Sets one of a hero's ability slots to a level.
pub(crate) fn set_level(world: &mut World, hero: Entity, slot: usize, level: u8) {
    world.abilities.get_mut(hero).expect("a kit").slots[slot].level = level;
}

/// Steps a world `ticks` times.
pub(crate) fn run(world: &mut World, ticks: u32) {
    for _ in 0..ticks {
        world.step();
    }
}

/// A match on [`config`] and its one hero, before any tick.
pub(crate) fn match_hero() -> (World, Entity) {
    let world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    (world, hero)
}

/// Shadow Fiend against Shadow Fiend: Radiant in seat 0, Dire in seat 1.
pub(crate) fn fiend_against_fiend() -> Vec<Pick> {
    let pick = |slot, team| Pick {
        slot: SlotId(slot),
        team,
        hero: HeroId::SHADOW_FIEND,
    };
    vec![pick(0, Team::Radiant), pick(1, Team::Dire)]
}

pub(crate) fn advance_validated(
    world: &mut World,
    unit: Option<Entity>,
    order: Order,
) -> Vec<Event> {
    let command = Command {
        slot: SlotId(0),
        unit: unit.map(wire_id),
        order,
        queue: false,
        seq: 0,
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

pub(crate) fn world_with_projectile_uphill_state(launch_tier: u8, can_miss_uphill: bool) -> World {
    let mut world = World::new();
    let target = world.spawn();
    let missile = world.spawn();
    world.projectile.insert(
        missile,
        Projectile {
            speed: Fixed::ONE,
            source: None,
            inflictor: None,
            target,
            damage: 1,
            kind: DamageKind::Physical,
            damage_amp_bp: rules::NOMINAL_BP,
            launch_tier,
            can_miss_uphill,
            pierces: false,
            pierced: Vec::new(),
            pierce_amp_bp: rules::NOMINAL_BP,
            effect: HitEffect::None,
        },
    );
    world
}

/// A Radiant melee creep with a lane mind of its own.
pub(crate) fn thinking_creep(world: &mut World, at: Vec2) -> Entity {
    thinking_creep_of(world, Team::Radiant, at)
}

/// A Radiant hero of the plain kind for seat 0, unseated.
pub(crate) fn hero_at(world: &mut World, at: Vec2) -> Entity {
    world.spawn_hero(Team::Radiant, at, SlotId(0), HeroId::SHADOW_FIEND)
}

/// A Radiant hero of one kind sat in a fresh seat 0 with no gold.
pub(crate) fn seated_hero(world: &mut World, at: Vec2, hero: HeroId) -> Entity {
    let unit = world.spawn_hero(Team::Radiant, at, SlotId(0), hero);
    let mut seat = Seat::new(SlotId(0), Team::Radiant, hero, 0, rules::STASH_SLOTS);
    seat.unit = Some(unit);
    world.seats.push(seat);
    unit
}

/// A hero of the plain kind sat in a fresh seat of a side with no gold.
pub(crate) fn seat_hero(world: &mut World, slot: u8, team: Team, at: Vec2) -> Entity {
    let hero = world.spawn_hero(team, at, SlotId(slot), HeroId::SHADOW_FIEND);
    let mut seat = Seat::new(
        SlotId(slot),
        team,
        HeroId::SHADOW_FIEND,
        0,
        rules::STASH_SLOTS,
    );
    seat.unit = Some(hero);
    world.seats.push(seat);
    hero
}

/// What the death the events tell of paid its killer.
pub(crate) fn paid(events: &[Event]) -> Option<i32> {
    events.iter().find_map(|event| match event.kind {
        EventKind::Died { bounty, .. } => bounty,
        _ => None,
    })
}

/// A hero that has learned one ability to its first level, with mana to spend.
pub(crate) fn caster(world: &mut World, at: Vec2, slot: usize) -> Entity {
    let hero = hero_at(world, at);
    // Levelled enough that the slot under test is one it may learn.
    world.level.insert(hero, Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(hero);
    assert!(world.learn(hero, slot), "the slot is learned");
    hero
}

/// A unit as a side's view shows it, or the full view for `None`.
pub(crate) fn shown(world: &World, side: Option<Team>, unit: Entity) -> UnitView {
    let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
    view.units
        .into_iter()
        .find(|shown| shown.id == wire_id(unit))
        .expect("in the view")
}

/// Who an entity is mid-swing at, if it is mid-swing at all.
pub(crate) fn swinging(world: &World, entity: Entity) -> Option<Entity> {
    match world.action.get(entity).map(|action| action.state) {
        Some(ActionState::Attack {
            target,
            phase: ActionPhase::Before { .. },
        }) => Some(target),
        _ => None,
    }
}

/// Whether an entity is recovering from a swing that landed.
pub(crate) fn recovering(world: &World, entity: Entity) -> bool {
    matches!(
        world.action.get(entity).map(|action| action.state),
        Some(ActionState::Attack {
            phase: ActionPhase::After { .. },
            ..
        })
    )
}

/// Ticks a span of milliseconds takes, rounded up.
pub(crate) fn ticks_of(ms: u32) -> u32 {
    (ms * rules::TICKS_PER_SECOND).div_ceil(1000)
}

/// A Radiant melee creep and a Dire one `gap` east of it, settled and
/// nothing else.
pub(crate) fn duel(gap: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let attacker = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5000 + gap, 5000));
    world.settle();
    (world, attacker, mark)
}

/// A seated hero one tick after being ordered to attack a Dire creep
/// `apart` east of it.
pub(crate) fn hero_ordered_at_an_enemy(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        Vec2::from_ints(5000 + apart, 5000),
    );
    world.settle();
    attack_click(&mut world, mark);
    (world, hero, mark)
}

/// The same seat and enemy, one tick after a move order at the enemy.
pub(crate) fn hero_following_an_enemy(apart: i32) -> (World, Entity, Entity) {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(apart);
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Unit(wire_id(mark)),
        },
    );
    (world, hero, mark)
}

/// A seated hero holding `order`, and a Dire creep 400 east of it.
pub(crate) fn hero_past_an_enemy(order: UnitOrder) -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let enemy = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5400, 5000));
    world.settle();
    world.orders.insert(
        hero,
        Orders {
            current: order,
            cooldown: 0,
            release_cooldown: 0,
            pending: None,
            immediate: None,
        },
    );
    (world, hero, enemy)
}

/// A hero of a side and one of its own creeps beside it.
pub(crate) fn hero_and_own_creep() -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let own = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5100, 5000));
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(own);
    (world, hero, own)
}

/// A standing Radiant hero, a Dire lane creep `apart` east of it, a Radiant
/// creep beside that creep, and a standing Dire hero to aim orders at; one
/// tick has run.
pub(crate) fn a_lane_with_a_hero(apart: i32) -> (World, Entity, Entity, Entity, Entity) {
    let mut world = World::new();
    let hero = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId::PUDGE);
    // Standing, so it takes nothing on of its own.
    world.set_order(hero, UnitOrder::Stand);
    let theirs = thinking_creep_of(&mut world, Team::Dire, Vec2::from_ints(5000 + apart, 5000));
    let ours = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        Vec2::from_ints(5000 + apart + 60, 5000),
    );
    // Only an order at an enemy hero calls creeps on.
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5000 + apart + 200, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.set_order(foe, UnitOrder::Stand);
    world.settle();
    world.step();
    (world, hero, theirs, ours, foe)
}

/// A creep of a side with a mind of its own.
pub(crate) fn thinking_creep_of(world: &mut World, team: Team, at: Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.lane_ai.insert(
        creep,
        LaneAi {
            last_seen: None,
            keep_until: 0,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    creep
}

/// Seat 0 gives its hero an order, and the world advances one tick.
pub(crate) fn seat_order(world: &mut World, order: Order) -> Vec<Event> {
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order,
        queue: false,
        seq: 0,
    }])
}

/// Seat 0 casts one of its hero's slots, and the world advances one tick.
pub(crate) fn cast(world: &mut World, slot: u8, target: Target) -> Vec<Event> {
    let slot = AbilitySlot(slot);
    seat_order(world, Order::Cast { slot, target })
}

/// Seat 0 clicks attack on somebody, and the world advances one tick.
pub(crate) fn attack_click(world: &mut World, on: Entity) {
    let target = Target::Unit(wire_id(on));
    seat_order(world, Order::Attack { target });
}

/// How far apart two entities stand along x, in whole units.
pub(crate) fn gap_along_lane(world: &World, one: Entity, other: Entity) -> i32 {
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

pub(crate) const FOUNTAIN_EFFECT: ModifierKind = ModifierKind::Fountain {
    hp_pct: rules::FOUNTAIN_HEAL_HP_PCT,
    mana_pct: rules::FOUNTAIN_HEAL_MANA_PCT,
};

/// Whether a unit carries an effect of one kind right now, whatever there is
/// of it.
pub(crate) fn carries(world: &World, entity: Entity, kind: ModifierKind) -> bool {
    let same = std::mem::discriminant(&kind);
    world.modifiers.get(entity).is_some_and(|on_it| {
        on_it
            .active()
            .any(|held| std::mem::discriminant(&held.kind) == same)
    })
}

/// A match world with its one hero beside its fountain and boots in the
/// first stash slot.
pub(crate) fn a_hero_at_the_shop() -> (World, Entity, ItemId) {
    let (mut world, hero) = match_hero();
    let boots = ItemId::BOOTS;
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    (world, hero, boots)
}

/// A spot with nothing standing within 800 units, and hero-sized room from
/// 100 west to 700 east of it, 200 either side.
pub(crate) fn an_empty_spot(world: &World) -> Vec2 {
    for row in 0..16 {
        for step in 0..60 {
            let at = Vec2::from_ints(5600 + step * 100, 6000 + row * 200);
            let clear = world.entities.iter().all(|entity| {
                !world
                    .transform
                    .get(entity)
                    .is_some_and(|t| t.pos.within(at, Fixed::from_int(800)))
            });
            let room = plan_radius(rules::units(rules::HERO_COLLISION));
            let open = (-2..=14).all(|dx: i32| {
                (-2..=2).all(|dy: i32| {
                    world
                        .clearance
                        .point_clear(at + Vec2::from_ints(dx * 50, dy * 100), room)
                })
            });
            if clear && open {
                return at;
            }
        }
    }
    panic!("the map has room somewhere")
}

/// A match hero moved to [`an_empty_spot`] with the scroll it started with
/// ready to read, one tick on.
pub(crate) fn a_hero_with_a_scroll() -> (World, Entity) {
    let (mut world, hero) = match_hero();
    world.transform.get_mut(hero).expect("hero").pos = an_empty_spot(&world);
    world.seats[0].item_clocks.clear();
    world.step();
    (world, hero)
}

/// 300 east of the first Radiant tower, or 300 west when east is not
/// walkable.
pub(crate) fn beside_own_tower(world: &World) -> Vec2 {
    let tower = world
        .entities
        .iter()
        .find(|entity| {
            world.kind.get(*entity) == Some(&UnitKind::Tower)
                && world.team.get(*entity) == Some(&Team::Radiant)
        })
        .expect("its side has towers");
    let at = world.transform.get(tower).expect("standing").pos;
    let beside = at + Vec2::from_ints(300, 0);
    if world.clearance.walkable(beside) {
        beside
    } else {
        at + Vec2::from_ints(-300, 0)
    }
}

/// A match hero moved to [`an_empty_spot`] with one charge of `item` in its
/// first slot, one tick on.
pub(crate) fn a_hero_with_a_ward(item: ItemId) -> (World, Entity, Vec2) {
    let (mut world, hero) = match_hero();
    let spot = an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = spot;
    hand_item(&mut world, hero, item, 1);
    world.step();
    (world, hero, spot)
}

/// The one ward standing on the map.
pub(crate) fn the_ward(world: &World) -> Entity {
    world
        .entities
        .iter()
        .find(|entity| world.kind.get(*entity) == Some(&UnitKind::Ward))
        .expect("a ward stands")
}

/// A hero on the big map standing beside the forest, with one item in hand.
pub(crate) fn a_hero_by_the_trees(item: ItemId, charges: u8) -> (World, Entity, Vec2) {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    let tree = tree_positions(map, crate::game::terrain_of(map))
        .into_iter()
        .find(|at| {
            world.clearance.stands_clear(*at + Vec2::from_ints(120, 0))
                && world.clearance.stands_clear(*at + Vec2::from_ints(240, 0))
        })
        .expect("some tree has open ground beside it");
    let stand = tree + Vec2::from_ints(120, 0);
    let hero = seated_hero(&mut world, stand, HeroId::SHADOW_FIEND);
    hand_item(&mut world, hero, item, charges);
    world.settle();
    world.step();
    (world, hero, tree)
}

/// Whether the cell a spot falls in still stops a sight line along the
/// ground there.
pub(crate) fn sight_stopped_at(world: &World, at: Vec2) -> bool {
    CellGrid::cell_of(at).is_some_and(|cell| world.sight_block.stops(cell, world.ground.tier(at)))
}

/// How long a tango eaten off the tree at a spot mends for.
pub(crate) fn tango_ticks(world: &mut World, hero: Entity, at: Vec2) -> u32 {
    world.modifiers.insert(hero, Modifiers(Vec::new()));
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(fresh_stack(ItemId::TANGO, 1));
    }
    assert!(world.use_item(hero, 1, Target::Pos(at), &mut Vec::new()));
    world
        .modifiers
        .get(hero)
        .expect("it mends")
        .active()
        .find(|held| matches!(held.kind, ModifierKind::Mending { .. }))
        .expect("of health")
        .ticks_left
        .expect("for a while")
}

/// What one swing of an entity takes off another, run to the blow itself.
pub(crate) fn one_swing_takes(world: &mut World, from: Entity, on: Entity) -> i32 {
    world.set_target(from, on);
    if let Some(action) = world.action.get_mut(from) {
        action.state = ActionState::Ready;
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

/// Puts an effect with no source on a unit for `ticks` ticks.
pub(crate) fn put_on(world: &mut World, entity: Entity, kind: ModifierKind, ticks: u32) {
    world.put_modifier(entity, Modifier::new(kind, None, None, Some(ticks)));
}

/// Pudge with his hook learned and a Dire creep `apart` east of him, one
/// tick on.
pub(crate) fn pudge_and_a_mark(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let pudge = seated_hero(&mut world, Vec2::from_ints(5000, 5000), HeroId::PUDGE);
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        Vec2::from_ints(5000 + apart, 5000),
    );
    set_level(&mut world, pudge, 0, 1);
    world.settle();
    world.step();
    (world, pudge, mark)
}

pub(crate) fn hook_flying(world: &World) -> bool {
    world.entities.iter().any(|e| world.hook.get(e).is_some())
}

/// Lays one item in a hero's first slot.
pub(crate) fn hand_item(world: &mut World, hero: Entity, item: ItemId, charges: u8) {
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(fresh_stack(item, charges));
    }
}

/// An untouched stack of one item for seat 0.
pub(crate) fn fresh_stack(item: ItemId, charges: u8) -> ItemStack {
    ItemStack {
        id: item,
        charges,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: SlotId(0),
        for_sale: false,
        free_charges: 0,
    }
}

/// A camp of two kobolds at a spot, asleep and at home.
pub(crate) fn a_camp_at(world: &mut World, at: Vec2) -> Vec<Entity> {
    let mut beasts = Vec::new();
    for step in 0..2 {
        let beast = world.spawn_unit(
            NeutralKind::Kobold.def(),
            Team::Neutral,
            at + Vec2::from_ints(60 * step, 0),
        );
        world
            .camp_home
            .insert(beast, CampHome { camp: 1, home: at });
        world.neutral_ai.insert(
            beast,
            NeutralAi {
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

pub(crate) fn the_courier(world: &World) -> Entity {
    world.seats[0].courier.expect("a seat has a courier")
}

pub(crate) fn courier_on_delivery() -> (World, Entity, Entity) {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = the_courier(&world);
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    advance_validated(
        &mut world,
        Some(courier),
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::None,
        },
    );
    world.advance(&[]);
    assert_eq!(world.errand.get(courier), Some(&Errand::ToOwner));
    assert!(slot_of(&world, courier, 0).is_some());
    (world, hero, courier)
}

/// A match hero of the plain kind beside its fountain, its seat holding
/// `gold`, one tick on.
pub(crate) fn a_hero_with_gold(gold: i32) -> (World, Entity) {
    let (mut world, hero) = match_hero();
    world.seats[0].gold = gold;
    world.settle();
    // A tick past settling, so the fountain's effect is already on.
    world.step();
    (world, hero)
}

pub(crate) fn slot_of(world: &World, hero: Entity, at: usize) -> Option<ItemStack> {
    world.inventory.get(hero).and_then(|bag| bag.slots[at])
}

pub(crate) fn price_of(item: ItemId) -> i32 {
    ITEMS[usize::from(item.0)].cost
}

pub(crate) fn reject_purchase_without_mutation(
    world: &mut World,
    hero: Entity,
    item: ItemId,
    reason: RejectReason,
) {
    let bag = world.inventory.get(hero).expect("bag").clone();
    let stash = world.seats[0].stash.clone();
    let gold = world.seats[0].gold;
    let order = Order::Buy { item };
    for unit in [None, Some(wire_id(the_courier(world)))] {
        assert_eq!(world.validate_order(SlotId(0), unit, &order), Err(reason));
    }
    let mut events = Vec::new();
    assert!(!world.buy(SlotId(0), item, &mut events));
    assert_eq!(world.inventory.get(hero), Some(&bag));
    assert_eq!(world.seats[0].stash, stash);
    assert_eq!(world.seats[0].gold, gold);
    assert!(events.is_empty());
}

pub(crate) fn on_the_ground(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| world.loot.get(*e).is_some())
        .collect()
}

/// A plain stack of one item for a seat, already touched.
pub(crate) fn a_stack_of(item: ItemId, owner: u8) -> ItemStack {
    ItemStack {
        id: item,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: SlotId(owner),
        for_sale: false,
        free_charges: 0,
    }
}

/// A world with a wall of closed ground between two spots, so the way from
/// one to the other has to be found rather than walked straight.
pub(crate) fn a_world_with_a_wall(from: Vec2, to: Vec2) -> World {
    let mut world = World::for_match(&config(), config().rng());
    world.settle();
    let middle = Vec2 {
        x: Fixed {
            raw: (from.x.raw / 2).saturating_add(to.x.raw / 2),
        },
        y: Fixed {
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
pub(crate) fn stand_a_wall(world: &mut World, at: Vec2, radius: Fixed) {
    let mut circles = world.clearance.circles().to_vec();
    circles.push((at, radius));
    world.clearance.set_circles(circles);
}

/// Shadow Fiend at (5000, 5000), facing east, with his whole kit at level
/// one and a Dire creep `apart` east of him, one tick on.
pub(crate) fn fiend_and_a_mark(apart: i32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let fiend = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        Vec2::from_ints(5000 + apart, 5000),
    );
    if let Some(book) = world.abilities.get_mut(fiend) {
        for held in book.slots.iter_mut() {
            held.level = 1;
        }
    }
    world.settle();
    world.step();
    (world, fiend, mark)
}

/// Shadow Fiend in Dire's seat 0 with his kit learned and his mana full, on
/// one side of a cliff (below it when `uphill`), facing a spot on the other
/// side; with the slot of the raze that lands on that spot.
pub(crate) fn fiend_across_a_cliff(uphill: bool) -> (World, Entity, Vec2, u8) {
    let mut world = World::new();
    let (low, _, high) = nearby_elevations(&world.ground);
    let (stand, spot) = if uphill { (low, high) } else { (high, low) };
    let fiend = seat_hero(&mut world, 0, Team::Dire, stand);
    if let Some(book) = world.abilities.get_mut(fiend) {
        for held in book.slots.iter_mut() {
            held.level = 1;
        }
    }
    let facing = facing_towards(stand, spot);
    if let Some(at) = world.transform.get_mut(fiend) {
        at.facing = facing;
    }
    world.settle();
    world.fill_pools(fiend);
    let gap = isqrt64(stand.distance_squared(spot)) / i64::from(Fixed::ONE.raw);
    let reach = if gap <= 325 { 0 } else { 1 };
    assert!(
        bota_core::raze_landing(stand, facing, reach)
            .within(spot, rules::units(rules::RAZE_RADIUS)),
        "the raze lands on the spot"
    );
    (world, fiend, spot, reach as u8)
}

pub(crate) fn a_creep_at(world: &mut World, team: Team, at: Vec2) -> Entity {
    let creep = world.spawn_unit(&MELEE_CREEP, team, at);
    world.settle();
    creep
}

pub(crate) fn hand_souls(world: &mut World, hero: Entity, many: u32) {
    world.set_souls(hero, many);
}

/// The standing structure at a spot, however it is guarded.
pub(crate) fn structure_at(world: &World, pos: Vec2) -> Entity {
    world
        .entities
        .iter()
        .find(|e| {
            world.transform.get(*e).is_some_and(|t| t.pos == pos)
                && world
                    .kind
                    .get(*e)
                    .copied()
                    .is_some_and(|kind| kind_def(kind).structure)
        })
        .expect("a structure stands there")
}

/// Whether the structure at a spot may be struck.
pub(crate) fn open_at(world: &World, pos: Vec2) -> bool {
    let it = structure_at(world, pos);
    !world.stats.get(it).expect("settled").invulnerable
}

/// Takes a structure at a spot down the way a fight would.
pub(crate) fn fell_at(world: &mut World, pos: Vec2) {
    let it = structure_at(world, pos);
    let mut events = Vec::new();
    world.bury(vec![(it, None)], &mut events);
    world.step();
}

/// Steps the world to the next wave spawn and hands back that wave's mid
/// creeps of one side.
pub(crate) fn next_mid_wave(world: &mut World, team: Team) -> Vec<Entity> {
    let before: Vec<Entity> = world.entities.iter().collect();
    loop {
        world.advance(&[]);
        let fresh: Vec<Entity> = world
            .entities
            .iter()
            .filter(|e| {
                !before.contains(e)
                    && world.march.get(*e).is_some()
                    && world.lane.get(*e).copied() == Some(Lane(rules::LANE_MID))
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
pub(crate) fn lane_progress(line: &[Vec2], pos: Vec2) -> i64 {
    let at = |v: Vec2| (i64::from(v.x.to_int()), i64::from(v.y.to_int()));
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

pub(crate) fn nearby_elevations(ground: &Ground) -> (Vec2, Vec2, Vec2) {
    const RADIUS: usize = 6;
    for sy in RADIUS..TERRAIN_CELLS - RADIUS {
        for sx in RADIUS..TERRAIN_CELLS - RADIUS {
            if !ground.cell_walkable(sx, sy) {
                continue;
            }
            let source = Vec2::from_ints(
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
                    let target = Vec2::from_ints(
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

pub(crate) const UPHILL_TEST_ATTACKER: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 1,
    attack_time: 33,
    attack_point: 33,
    attack_backswing: 0,
    projectile_speed: Some(6_000),
    move_speed: 0,
    ..RANGED_CREEP
};

pub(crate) const UPHILL_TEST_FLYING_ATTACKER: UnitDef = UnitDef {
    flies: true,
    ..UPHILL_TEST_ATTACKER
};

pub(crate) const UPHILL_TEST_TARGET: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 0,
    move_speed: 0,
    armor: 0,
    ..MELEE_CREEP
};

pub(crate) const UPHILL_TEST_BUILDING: UnitDef = UnitDef {
    kind: UnitKind::Tower,
    move_speed: 0,
    ..UPHILL_TEST_TARGET
};

/// A Radiant spotter that keeps the target of a ranged run in sight.
const UPHILL_TEST_OBSERVER: UnitDef = UnitDef {
    max_hp: 8_000,
    damage: 0,
    move_speed: 0,
    day_vision: 2_000,
    night_vision: 2_000,
    ..MELEE_CREEP
};

/// A Radiant attacker set on a Dire target, with a spotter standing on the
/// target, settled and facing it.
fn ranged_duel(
    source: Vec2,
    target: Vec2,
    attacker_def: &'static UnitDef,
    target_def: &'static UnitDef,
) -> (World, Entity, Entity) {
    let mut world = World::new();
    let attacker = world.spawn_unit(attacker_def, Team::Radiant, source);
    let mark = world.spawn_unit(target_def, Team::Dire, target);
    world.spawn_unit(&UPHILL_TEST_OBSERVER, Team::Radiant, target);
    world.settle();
    world.set_target(attacker, mark);
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = facing_towards(source, target);
    }
    (world, attacker, mark)
}

pub(crate) fn ranged_damage_after_ticks(
    source: Vec2,
    target: Vec2,
    attacker_def: &'static UnitDef,
    target_def: &'static UnitDef,
) -> i32 {
    let (mut world, _, mark) = ranged_duel(source, target, attacker_def, target_def);
    let before = world.health.get(mark).expect("target health").hp;
    run(&mut world, 512);
    let after = world.health.get(mark).expect("target survives").hp;
    (before - after).to_int()
}

/// Blows an attacker landed and missed on a target over the same run as
/// [`ranged_damage_after_ticks`], counted from what the ticks told of.
pub(crate) fn ranged_swings_after_ticks(
    source: Vec2,
    target: Vec2,
    attacker_def: &'static UnitDef,
    target_def: &'static UnitDef,
) -> (usize, usize) {
    let (mut world, attacker, mark) = ranged_duel(source, target, attacker_def, target_def);
    let (mut landed, mut missed) = (0, 0);
    for _ in 0..512 {
        for event in world.step() {
            match event.kind {
                EventKind::Damaged { source, target, .. }
                    if source == Some(wire_id(attacker)) && target == wire_id(mark) =>
                {
                    landed += 1;
                }
                EventKind::Missed { source, target }
                    if source == Some(wire_id(attacker)) && target == wire_id(mark) =>
                {
                    missed += 1;
                }
                _ => {}
            }
        }
    }
    (landed, missed)
}

pub(crate) fn manual_projectile_damage(
    source_pos: Vec2,
    target_pos: Vec2,
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
            Transform {
                pos: target_pos,
                facing: Angle::default(),
            },
        );
        world.set_team(missile, Team::Radiant);
        world.projectile.insert(
            missile,
            Projectile {
                speed: Fixed::ONE,
                source: Some(source),
                inflictor: None,
                target,
                damage: 1,
                kind: DamageKind::Physical,
                damage_amp_bp: rules::NOMINAL_BP,
                launch_tier,
                can_miss_uphill: true,
                pierces,
                pierced: Vec::new(),
                pierce_amp_bp: rules::NOMINAL_BP,
                effect: HitEffect::None,
            },
        );
    }

    world.step();

    let after = world.health.get(target).expect("target survives").hp;
    (before - after).to_int()
}

/// Steps a world until a mover stands within fifty units of a spot, and
/// how many ticks that took. None when it never got there in time.
pub(crate) fn ticks_until_near(
    world: &mut World,
    mover: Entity,
    goal: Vec2,
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

/// The spot a progress along a polyline lands on, shifted sideways.
pub(crate) fn along_lane(route: &[Vec2], progress: i64, aside: i64) -> Vec2 {
    let mut left = progress;
    for (i, seg) in route.windows(2).enumerate() {
        let (a, b) = (seg[0], seg[1]);
        let len = isqrt64(a.distance_squared(b)) >> 16;
        if len >= left || i + 2 == route.len() {
            let dx = i64::from(b.x.to_int() - a.x.to_int());
            let dy = i64::from(b.y.to_int() - a.y.to_int());
            let len = len.max(1);
            let x = i64::from(a.x.to_int()) + dx * left / len - dy * aside / len;
            let y = i64::from(a.y.to_int()) + dy * left / len + dx * aside / len;
            return Vec2::from_ints(x as i32, y as i32);
        }
        left -= len;
    }
    route[route.len() - 1]
}

/// Steps the world through seat 0's respawn wait and hands back its new body.
pub(crate) fn back_on_its_feet(world: &mut World) -> Entity {
    let wait = world.seats[0].respawn_left;
    run(world, wait);
    world.seats[0].unit.expect("it came back")
}

/// Seat 0 presses the stop key, and the world advances one tick.
pub(crate) fn stop_click(world: &mut World) {
    let target = Target::None;
    seat_order(world, Order::Move { target });
}

/// Sets a unit's health to so many hundredths of its most.
pub(crate) fn wear_down(world: &mut World, unit: Entity, hundredths: i32) {
    let max = world.stats.get(unit).expect("settled").max_hp;
    world.health.insert(
        unit,
        Health {
            hp: Fixed {
                raw: max.raw * hundredths / 100,
            },
        },
    );
}

/// A realtime match on a map with nobody seated.
pub(crate) fn a_match_with_nobody_in_it(match_id: u64, map: MapId) -> World {
    let config = MatchConfig {
        starting_gold_lost: Vec::new(),
        match_id,
        stream: match_id,
        master_key: [0; 32],
        picks: vec![],
        map,
        tick_rate: 30,
        mode: TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    };
    World::for_match(&config, config.rng())
}

/// The demo map stepped to the tick its first waves come out.
pub(crate) fn the_demo_lane_at_its_first_wave() -> World {
    let mut world = World::on_map(map_of(MapId(1)));
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    world
}

/// Every Radiant lane creep standing.
pub(crate) fn radiant_marchers(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some() && world.team.get(*e) == Some(&Team::Radiant))
        .collect()
}

/// Casts one of the fiend's slots and runs the world through its cast point:
/// every event on the way, the tick it goes off included.
pub(crate) fn cast_through(world: &mut World, slot: u8, point_ms: u32) -> Vec<Event> {
    let mut events = cast(world, slot, Target::None);
    for _ in 0..ticks_of(point_ms) {
        events.extend(world.step());
    }
    events
}

/// Magic damage told of on one unit.
pub(crate) fn burns_on(events: &[Event], unit: Entity) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged {
                target,
                amount,
                kind: DamageKind::Magical,
                ..
            } if target == wire_id(unit) => Some(amount),
            _ => None,
        })
        .collect()
}

pub(crate) fn souls(world: &World, hero: Entity) -> u32 {
    world.souls_of(hero)
}

/// Ticks a unit that stands for a while has left, from its timed life.
pub(crate) fn life_left(world: &World, unit: Entity) -> Option<u32> {
    world
        .modifiers
        .get(unit)?
        .active()
        .find(|held| held.kind == ModifierKind::TimedLife)?
        .ticks_left
}
