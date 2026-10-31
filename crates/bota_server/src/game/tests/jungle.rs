//! Neutral camps.

use bota_core::{
    CAMPS, COURIER, CampDef, CampKind, MELEE_CREEP, NeutralAbility, NeutralKind, OBSERVER_WARD,
    SENTRY_WARD, SIEGE_CREEP, UnitDef, map_of, rules,
};

use crate::game::{Def, Entity, World};
use bota_proto::{DamageKind, EffectId, Fixed, ItemId, MapId, Order, Target, Team, UnitKind, Vec2};

use super::support::*;

#[test]
fn camps_fill_on_the_minute_and_stay_full() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < world.schedule().first_neutral() {
        world.step();
    }
    let neutrals = |world: &World| {
        world
            .entities
            .iter()
            .filter(|e| world.camp_home.get(*e).is_some())
            .count()
    };
    let filled = neutrals(&world);
    assert!(filled > 0, "the jungle put something out");
    run(&mut world, rules::NEUTRAL_SPAWN_PERIOD_TICKS);
    assert_eq!(
        neutrals(&world),
        filled,
        "a full camp puts out nothing more"
    );
}

#[test]
fn a_neutral_led_too_far_gives_up_and_walks_home() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < world.schedule().first_neutral() {
        world.step();
    }
    let beast = world
        .entities
        .iter()
        .find(|e| world.camp_home.get(*e).is_some())
        .expect("the jungle is full");
    let home = world.camp_home.get(beast).expect("it has one").home;
    // Carried well past its guard distance and held there.
    let away = home + Vec2::from_ints(1200, 0);
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
    // Home again, it stands, and may be led as far as before.
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = home;
    }
    world.step();
    let ai = world.neutral_ai.get(beast).copied().expect("it has a mind");
    assert!(!ai.going_home, "back home it stands again");
    assert_eq!(ai.leash_left, rules::NEUTRAL_AGGRO_WINDOW);
}

/// What the map's first dry camp holds after its first fill, with one thing
/// put into the world beforehand.
fn first_fill_with(put: impl FnOnce(&mut World)) -> usize {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    put(&mut world);
    world.settle();
    while world.tick < world.schedule().first_neutral() + 1 {
        world.step();
    }
    beasts_of(&world, CAMP)
}

/// A way to put a unit at a point.
type Blocker = fn(&mut World, Vec2);

/// The dry camp the box tests use.
const CAMP: usize = 1;

fn beasts_of(world: &World, camp: usize) -> usize {
    world
        .entities
        .iter()
        .filter(|e| {
            world
                .camp_home
                .get(*e)
                .is_some_and(|h| usize::from(h.camp) == camp)
        })
        .count()
}

fn inside() -> Vec2 {
    let b = CAMPS[CAMP].spawn_box;
    Vec2::from_ints(
        (b.min.x.to_int() + b.max.x.to_int()) / 2,
        (b.min.y.to_int() + b.max.y.to_int()) / 2,
    )
}

#[test]
fn an_empty_box_fills_and_what_stands_in_it_keeps_it_empty() {
    assert!(first_fill_with(|_| {}) > 0, "an empty box fills");
    let blockers: [(&str, Blocker); 6] = [
        ("an observer ward", |w, at| {
            w.spawn_unit(&OBSERVER_WARD, Team::Radiant, at);
        }),
        ("a sentry ward", |w, at| {
            w.spawn_unit(&SENTRY_WARD, Team::Dire, at);
        }),
        ("a hero", |w, at| {
            hero_at(w, at);
        }),
        ("a lane creep", |w, at| {
            w.spawn_unit(&MELEE_CREEP, Team::Dire, at);
        }),
        ("a siege creep that does not move", |w, at| {
            let siege = w.spawn_unit(&SIEGE_CREEP, Team::Dire, at);
            w.settle();
            w.stats.get_mut(siege).expect("stats").move_speed = Fixed::ZERO;
        }),
        ("a rooted hero", |w, at| {
            let hero = hero_at(w, at);
            w.settle();
            w.stats.get_mut(hero).expect("stats").move_speed = Fixed::ZERO;
        }),
    ];
    for (name, put) in blockers {
        let beasts = first_fill_with(|world| put(world, inside()));
        assert_eq!(beasts, 0, "{name} in the box keeps the camp empty");
    }
}

#[test]
fn a_dead_hero_a_courier_and_loot_do_not_block() {
    let dead = first_fill_with(|world| {
        let hero = hero_at(world, inside());
        world.settle();
        world.health.get_mut(hero).expect("health").hp = Fixed::ZERO;
    });
    assert!(dead > 0, "a dead hero does not keep the camp empty");
    let courier = first_fill_with(|world| {
        world.spawn_unit(&COURIER, Team::Radiant, inside());
    });
    assert!(courier > 0, "a courier does not");
    let loot = first_fill_with(|world| {
        world.lay_loot(a_stack_of(ItemId::IRON_BRANCH, 0), inside());
    });
    assert!(loot > 0, "an item on the ground does not");
}

#[test]
fn a_ward_blocks_to_the_edge_of_the_box_and_not_past_it() {
    let b = CAMPS[CAMP].spawn_box;
    let mid_y = (b.min.y.to_int() + b.max.y.to_int()) / 2;
    let cases = [
        (b.max.x.to_int(), mid_y, true),
        (b.max.x.to_int() + 1, mid_y, false),
        (b.min.x.to_int(), mid_y, true),
        (b.min.x.to_int() - 1, mid_y, false),
        (b.min.x.to_int() + 1, b.max.y.to_int(), true),
        (b.min.x.to_int() + 1, b.max.y.to_int() + 1, false),
        (b.min.x.to_int() + 1, b.min.y.to_int(), true),
        (b.min.x.to_int() + 1, b.min.y.to_int() - 1, false),
    ];
    for (x, y, blocks) in cases {
        let beasts = first_fill_with(|world| {
            world.spawn_unit(&OBSERVER_WARD, Team::Radiant, Vec2::from_ints(x, y));
        });
        assert_eq!(
            beasts == 0,
            blocks,
            "a ward at ({x}, {y}) against the box {b:?}"
        );
    }
}

#[test]
fn pulling_a_camp_out_of_its_box_before_the_minute_stacks_it() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    while world.tick < world.schedule().first_neutral() + 1 {
        world.step();
    }
    let first = beasts_of(&world, CAMP);
    assert!(first > 0);
    let b = CAMPS[CAMP].spawn_box;
    let out = Vec2::from_ints(b.max.x.to_int() + 200, b.max.y.to_int() + 200);
    let mine: Vec<_> = world
        .entities
        .iter()
        .filter(|e| {
            world
                .camp_home
                .get(*e)
                .is_some_and(|h| usize::from(h.camp) == CAMP)
        })
        .collect();
    // Held in the box, one stays and the camp stays closed.
    for beast in &mine[1..] {
        world.transform.get_mut(*beast).expect("stands").pos = out;
    }
    while world.tick < world.schedule().first_neutral() + rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
        for beast in &mine[1..] {
            world.transform.get_mut(*beast).expect("stands").pos = out;
        }
    }
    assert_eq!(beasts_of(&world, CAMP), first, "one left inside blocks it");
    // All of them out, the next minute puts a second camp beside them.
    world.transform.get_mut(mine[0]).expect("stands").pos = out;
    while world.tick < world.schedule().first_neutral() + 2 * rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
        for beast in &mine {
            world.transform.get_mut(*beast).expect("stands").pos = out;
        }
    }
    assert!(beasts_of(&world, CAMP) > first, "an empty box stacks");
}

#[test]
fn a_neutral_sleeps_until_something_comes_right_up_to_it() {
    let camp = Vec2::from_ints(5000, 5000);
    for (apart, wakes) in [
        (rules::NEUTRAL_AGGRO_RANGE + 120, false),
        (rules::NEUTRAL_AGGRO_RANGE - 60, true),
    ] {
        let mut world = World::new();
        world.tick = world.schedule().horn;
        let beasts = a_camp_at(&mut world, camp);
        let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
        world.settle();
        world.step();
        world.step();
        assert_eq!(
            world.target_of(beasts[0]) == Some(hero),
            wakes,
            "standing {apart} off by day, waking should be {wakes}"
        );
    }
}

#[test]
fn a_neutral_sleeps_through_the_night_with_an_enemy_at_its_side_and_wakes_at_dawn() {
    let camp = Vec2::from_ints(5000, 5000);
    let mut world = World::new();
    let dawn = world.schedule().horn + 2 * rules::HALF_DAY_TICKS;
    world.tick = dawn - 40;
    let beasts = a_camp_at(&mut world, camp);
    // An onlooker that never swings: an idle hero swings at what stands in
    // its reach, and the blow would wake the camp by itself.
    const ONLOOKER: UnitDef = UnitDef {
        acquisition: 0,
        ..MELEE_CREEP
    };
    let onlooker = world.spawn_unit(
        &ONLOOKER,
        Team::Radiant,
        camp + Vec2::from_ints(rules::NEUTRAL_AGGRO_RANGE - 60, 0),
    );
    world.settle();
    while world.tick < dawn - 1 {
        world.step();
        assert!(
            beasts.iter().all(|beast| world.target_of(*beast).is_none()),
            "asleep at night on tick {}, whoever stands beside it",
            world.tick
        );
    }
    world.step();
    world.step();
    assert_eq!(
        world.target_of(beasts[0]),
        Some(onlooker),
        "the day wakes it to what stands beside it"
    );
}

#[test]
fn a_blow_wakes_a_camp_from_further_than_it_can_see() {
    let mut world = World::new();
    let camp = Vec2::from_ints(5000, 5000);
    let beasts = a_camp_at(&mut world, camp);
    // Far past what wakes them by coming near, but inside the reach of a blow.
    let apart = rules::NEUTRAL_DAMAGE_AGGRO_RANGE - 100;
    let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
    world.settle();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "they sleep through a hero that far off"
    );
    world.push_hit(Some(hero), beasts[0], 10, DamageKind::Physical, None);
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
    let far = hero_at(
        &mut world,
        camp + Vec2::from_ints(rules::NEUTRAL_DAMAGE_AGGRO_RANGE + 400, 0),
    );
    world.settle();
    world.push_hit(Some(far), beasts[0], 10, DamageKind::Physical, None);
    world.step();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "a blow from beyond its reach wakes nobody"
    );
}

#[test]
fn a_neutral_slots_only_its_own_abilities_and_grows_with_the_clock_unseen() {
    // Dota's neutral_upgrade is an attribute-kind ability its HUD never
    // shows, and its modifier is hidden: a neutral's slots hold its own
    // abilities, and the clock's upgrades show only in its stats.
    let mut world = World::new();
    let horn = world.schedule().horn;
    world.tick = horn + rules::NEUTRAL_UPGRADE_PERIOD_TICKS - 2;
    let troll = world.spawn_neutral(
        NeutralKind::ForestTrollBerserker,
        Vec2::from_ints(5000, 5000),
        0,
        0,
    );
    world.settle();
    let before = shown(&world, None, troll);
    assert_eq!(
        before
            .abilities
            .iter()
            .map(|slot| slot.id)
            .collect::<Vec<_>>(),
        [NeutralAbility::Break.id(), NeutralAbility::Piercing.id()],
        "its slots are its own"
    );
    run(&mut world, 2);
    let after = shown(&world, None, troll);
    assert_eq!(
        (after.max_hp, after.attack_damage_min),
        (
            before.max_hp + rules::NEUTRAL_UPGRADE_HP,
            before.attack_damage_min + rules::NEUTRAL_UPGRADE_DAMAGE
        ),
        "the clock's upgrade raised it"
    );
    assert!(
        after
            .effects
            .iter()
            .all(|effect| effect.id != EffectId::NEUTRAL_UPGRADE),
        "and no view names the upgrade"
    );
}

/// Liquipedia, *Creep pulling*: a hero strikes a pull camp as its lane wave comes up and runs out
/// onto the lane ahead of it; the neutrals chase it into the creeps' acquisition range, the creeps
/// take them on (*Creep aggro*: a lane creep fights the neutrals of a camp it can be pulled to)
/// and follow them home (*Lane Creeps*: a creep chases its target indefinitely), and the wave
/// is fought out in the jungle. Radiant's safe lane small camp, pulled by a scripted hero from
/// the horn on, every step an order of its seat, with the swings recovering for each kind's own
/// time and for Dota's.
#[test]
fn a_hero_pulls_its_safe_lane_wave_into_the_small_camp() {
    for recovery in [Recovery::OWN, Recovery::DOTA] {
        let pulled = pull(&waiting_by_the_camp(recovery), PULL_REACH, true);
        assert!(pulled.fought, "the wave takes the neutrals on");
        assert!(
            pulled.leashed,
            "the neutrals, fought past their guard distance, turn for home"
        );
        assert!(pulled.dragged, "and the wave follows them into the camp");
    }
}

/// A pull that leads the neutrals onto the lane ahead of the wave holds whenever in the wave's
/// approach the hero strikes and however long creeps and neutrals recover from a swing: no tick
/// of timing decides it.
#[test]
fn a_pull_holds_for_any_strike_moment_and_any_recovery() {
    let recoveries = [
        Recovery::OWN,
        Recovery::DOTA,
        Recovery {
            melee: Some(700),
            ranged: Some(700),
            neutral: Some(|_| 700),
        },
        Recovery {
            melee: Some(300),
            ranged: Some(300),
            neutral: Some(|_| 0),
        },
    ];
    for recovery in recoveries {
        let by = waiting_by_the_camp(recovery);
        for reach in [1100, 1500, 1900, 2300] {
            let pulled = pull(&by, reach, true);
            let all = Pulled {
                fought: true,
                leashed: true,
                dragged: true,
            };
            assert_eq!(pulled, all, "struck at {reach} with {recovery:?}");
        }
    }
}

/// A wave nobody pulls walks its lane past the camp: the same hero walking out onto the lane
/// without striking the camp leaves the neutrals asleep and the wave on its way.
#[test]
fn a_wave_nobody_pulls_walks_on_past_the_camp() {
    let by = waiting_by_the_camp(Recovery::OWN);
    let pulled = pull(&by, PULL_REACH, false);
    assert!(!pulled.fought, "the wave leaves the camp be");
    assert!(!pulled.dragged, "and never sets foot in it");
}

/// What a scripted pull came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pulled {
    /// A creep of the wave swung at one of the camp.
    fought: bool,
    /// One of the camp turned for home after the wave took it on.
    leashed: bool,
    /// A creep of the wave stood in the camp's spawn box.
    dragged: bool,
}

/// How long lane creeps and neutrals recover from a swing in a scripted
/// pull, in milliseconds: `None` keeps a kind's own.
#[derive(Clone, Copy, Debug)]
struct Recovery {
    melee: Option<u32>,
    ranged: Option<u32>,
    neutral: Option<fn(NeutralKind) -> u32>,
}

impl Recovery {
    const OWN: Recovery = Recovery {
        melee: None,
        ranged: None,
        neutral: None,
    };

    /// Liquipedia, *Attack Animation*: melee creeps 0.533 s, ranged 0.3 s,
    /// neutrals by kind.
    const DOTA: Recovery = Recovery {
        melee: Some(533),
        ranged: Some(300),
        neutral: Some(dota_neutral_recovery),
    };
}

/// Liquipedia, *Attack Animation*: how long each neutral recovers from a
/// swing; 0.3 s for the kinds the table leaves out.
fn dota_neutral_recovery(kind: NeutralKind) -> u32 {
    match kind {
        NeutralKind::Kobold | NeutralKind::KoboldTunneler | NeutralKind::KoboldTaskmaster => 600,
        NeutralKind::GiantWolf | NeutralKind::AlphaWolf => 640,
        NeutralKind::BlackDrake
        | NeutralKind::BlackDragon
        | NeutralKind::SmallThunderLizard
        | NeutralKind::BigThunderLizard => 560,
        NeutralKind::GraniteGolem => 500,
        NeutralKind::SkeletonWarrior => 440,
        _ => 300,
    }
}

/// How far off the wave's front creep is when the scripted hero strikes.
const PULL_REACH: i32 = 1300;
/// How far down the lane past the camp the scripted hero leads the
/// neutrals.
const PULL_AHEAD: i32 = 1500;

/// A match on the Dota map whose Radiant hero waits out of the safe lane
/// small camp's box, the camp filled and the first wave after its fill on
/// its way, with a [`Recovery`] on every creep.
#[derive(Clone)]
struct ByTheCamp {
    world: World,
    hero: Entity,
    paced: Paced,
}

fn waiting_by_the_camp(recovery: Recovery) -> ByTheCamp {
    let (mut world, hero) = a_full_map_hero();
    let (_, camp) = safe_lane_small_camp();
    let wait = Vec2::from_ints(camp.pos.x.to_int(), camp.spawn_box.min.y.to_int() - 250);
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Pos(wait),
        },
    );
    let mut paced = Paced::new(recovery);
    while world.tick < world.schedule().first_neutral() + rules::WAVE_PERIOD_TICKS / 2 {
        paced.step(&mut world);
    }
    ByTheCamp { world, hero, paced }
}

/// Pulls the wave from [`waiting_by_the_camp`]: once the wave's front creep
/// comes within `reach`, the hero strikes the nearest neutral (unless
/// `strike` is false) and walks out onto the lane ahead of the wave and on
/// down it, the neutrals in tow, for the wave to walk into them.
fn pull(by: &ByTheCamp, reach: i32, strike: bool) -> Pulled {
    let ByTheCamp {
        mut world,
        hero,
        mut paced,
    } = by.clone();
    let (index, camp) = safe_lane_small_camp();
    let front = loop {
        paced.step(&mut world);
        if let Some(front) = front_within(&world, hero, reach) {
            break pos_of(&world, front);
        }
        assert!(
            world.tick < world.schedule().first_neutral() + 2 * rules::WAVE_PERIOD_TICKS,
            "no wave came"
        );
    };
    if strike {
        let beast = *camp_beasts(&world, index)
            .iter()
            .min_by_key(|b| pos_of(&world, **b).distance_squared(pos_of(&world, hero)))
            .expect("the camp is full");
        attack_click(&mut world, beast);
        let max_hp = world.stats.get(beast).expect("a beast").max_hp;
        while world.health.get(beast).is_some_and(|h| h.hp >= max_hp) {
            paced.step(&mut world);
            assert!(
                world.tick < world.schedule().first_neutral() + 3 * rules::WAVE_PERIOD_TICKS,
                "never struck"
            );
        }
    }
    let ahead = Vec2 {
        x: camp.pos.x + Fixed::from_int(PULL_AHEAD),
        y: front.y,
    };
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Pos(ahead),
        },
    );
    let mut pulled = Pulled {
        fought: false,
        leashed: false,
        dragged: false,
    };
    for _ in 0..20 * rules::TICKS_PER_SECOND {
        paced.step(&mut world);
        let wave = lane_creeps(&world);
        let neutrals = camp_beasts(&world, index);
        pulled.fought |= wave
            .iter()
            .any(|c| swinging(&world, *c).is_some_and(|t| neutrals.contains(&t)));
        pulled.dragged |= wave
            .iter()
            .any(|c| camp.spawn_box.contains(pos_of(&world, *c)));
        pulled.leashed |= pulled.fought
            && neutrals
                .iter()
                .any(|b| world.neutral_ai.get(*b).is_some_and(|ai| ai.going_home));
    }
    pulled
}

/// Radiant's safe lane creeps.
fn lane_creeps(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| {
            world.team.get(*e) == Some(&Team::Radiant)
                && world.march.get(*e).is_some()
                && world
                    .lane
                    .get(*e)
                    .is_some_and(|lane| lane.0 == rules::LANE_BOT)
        })
        .collect()
}

/// The wave's front creep, the lane creep nearest the hero, once it is
/// within `reach` of the hero.
fn front_within(world: &World, hero: Entity, reach: i32) -> Option<Entity> {
    let hero_at = pos_of(world, hero);
    lane_creeps(world)
        .into_iter()
        .min_by_key(|c| pos_of(world, *c).distance_squared(hero_at))
        .filter(|c| pos_of(world, *c).within(hero_at, Fixed::from_int(reach)))
}

/// Puts a [`Recovery`] on every lane creep and neutral as it comes: its
/// kind's entry, copied with the recovery asked for.
#[derive(Clone)]
struct Paced {
    recovery: Recovery,
    copies: Vec<(&'static UnitDef, u32, &'static UnitDef)>,
}

impl Paced {
    fn new(recovery: Recovery) -> Paced {
        Paced {
            recovery,
            copies: Vec::new(),
        }
    }

    /// Steps the world and paces whatever it brought.
    fn step(&mut self, world: &mut World) {
        world.step();
        let entities: Vec<Entity> = world.entities.iter().collect();
        for entity in entities {
            let Some(Def(def)) = world.def.get(entity).copied() else {
                continue;
            };
            let wanted = match (world.kind.get(entity), world.neutral.get(entity)) {
                (_, Some(kind)) => self.recovery.neutral.map(|of| of(*kind)),
                (Some(UnitKind::CreepMelee | UnitKind::CreepFlagbearer), _) => self.recovery.melee,
                (Some(UnitKind::CreepRanged), _) => self.recovery.ranged,
                _ => None,
            };
            if let Some(ms) = wanted.filter(|ms| *ms != def.attack_backswing) {
                let copy = self.copy(def, ms);
                world.def.insert(entity, Def(copy));
            }
        }
    }

    /// A kind's entry recovering for `ms`: one copy per entry and time,
    /// made once for the test's run.
    fn copy(&mut self, def: &'static UnitDef, ms: u32) -> &'static UnitDef {
        let made = self
            .copies
            .iter()
            .find(|(of, at, _)| std::ptr::eq(*of, def) && *at == ms);
        if let Some((_, _, copy)) = made {
            return copy;
        }
        let copy: &'static UnitDef = Box::leak(Box::new(UnitDef {
            attack_backswing: ms,
            ..*def
        }));
        self.copies.push((def, ms, copy));
        copy
    }
}

/// A match on the Dota map with one Radiant hero, every step an order of its seat.
fn a_full_map_hero() -> (World, Entity) {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = map_of(MapId(0)).default_objectives;
    cfg.runes = map_of(MapId(0)).default_runes;
    let world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    (world, hero)
}

/// Radiant's safe lane small camp: its index and its definition.
fn safe_lane_small_camp() -> (usize, &'static CampDef) {
    let index = CAMPS
        .iter()
        .position(|c| {
            c.pullable
                && c.kind == CampKind::Small
                && c.pos.x > Fixed::from_int(12_000)
                && c.pos.y < Fixed::from_int(5_000)
        })
        .expect("Radiant's safe lane small camp");
    (index, &CAMPS[index])
}

/// The living neutrals of a camp.
fn camp_beasts(world: &World, camp: usize) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|e| {
            world
                .camp_home
                .get(*e)
                .is_some_and(|h| usize::from(h.camp) == camp)
        })
        .collect()
}

fn pos_of(world: &World, entity: Entity) -> Vec2 {
    world.transform.get(entity).expect("stands").pos
}

fn put(world: &mut World, entity: Entity, at: Vec2) {
    world.transform.get_mut(entity).expect("stands").pos = at;
}

/// Liquipedia, *Neutral Creeps*, Losing Aggro: a neutral led more than 400 from its spot gives
/// up five seconds later and walks back, and only coming back within 400 starts those seconds
/// again. A hero that keeps striking it as it runs (a kite) holds it no longer: the neutral
/// turns for home five seconds after it passed its guard distance, and stands at home again.
#[test]
fn a_kited_neutral_gives_up_five_seconds_past_its_guard_distance() {
    let (mut world, hero) = a_full_map_hero();
    let (index, _) = safe_lane_small_camp();
    while world.tick <= world.schedule().first_neutral() {
        world.step();
    }
    let beast = camp_beasts(&world, index)[0];
    let home = world.camp_home.get(beast).expect("a camp's").home;
    put(&mut world, hero, home + Vec2::from_ints(0, -250));
    let away = Target::Pos(home + Vec2::from_ints(-3000, -1200));
    seat_order(&mut world, Order::Move { target: away });
    let guard = Fixed::from_int(rules::NEUTRAL_GUARD_DISTANCE);
    let (mut passed, mut gave_up) = (None, None);
    for _ in 0..20 * rules::TICKS_PER_SECOND {
        let going_home = world.neutral_ai.get(beast).is_some_and(|ai| ai.going_home);
        if gave_up.is_none() && world.tick.is_multiple_of(30) {
            world.push_hit(Some(hero), beast, 1, DamageKind::Physical, None);
        }
        world.step();
        if passed.is_none() && !pos_of(&world, beast).within(home, guard) {
            passed = Some(world.tick);
        }
        if gave_up.is_none()
            && !going_home
            && world.neutral_ai.get(beast).is_some_and(|ai| ai.going_home)
        {
            gave_up = Some(world.tick);
        }
    }
    let (passed, gave_up) = (passed.expect("it chased"), gave_up.expect("it gave up"));
    assert_eq!(
        gave_up - passed,
        rules::NEUTRAL_AGGRO_WINDOW,
        "five seconds past 400"
    );
    let ai = world.neutral_ai.get(beast).copied().expect("a neutral");
    assert!(
        !ai.going_home
            && pos_of(&world, beast).within(home, Fixed::from_int(rules::NEUTRAL_RETURN)),
        "home again"
    );
    assert!(world.target_of(beast).is_none(), "and nobody's");
}

/// A blow wakes the neutrals around the one struck, not its whole camp (Liquipedia, *Neutral
/// Creeps*: "another neutral creep within a 500 radius"): a neutral led far off and struck there
/// turns on its striker with whatever stands near it, while the rest of its stack stays at home.
#[test]
fn a_blow_on_a_neutral_led_far_off_leaves_its_stack_at_home() {
    let (mut world, hero) = a_full_map_hero();
    let (index, camp) = safe_lane_small_camp();
    while world.tick <= world.schedule().first_neutral() {
        world.step();
    }
    let beasts = camp_beasts(&world, index);
    assert!(beasts.len() >= 3, "a full small camp");
    let far = camp.pos + Vec2::from_ints(-1400, -900);
    put(&mut world, beasts[0], far);
    put(&mut world, beasts[1], far + Vec2::from_ints(300, 0));
    put(&mut world, hero, far + Vec2::from_ints(0, -300));
    world.settle();
    attack_click(&mut world, beasts[0]);
    let max_hp = world.stats.get(beasts[0]).expect("a beast").max_hp;
    while world.health.get(beasts[0]).is_some_and(|h| h.hp >= max_hp) {
        world.step();
        assert!(
            world.tick < world.schedule().first_neutral() + 300,
            "never struck"
        );
    }
    world.step();
    world.step();
    assert_eq!(
        world.target_of(beasts[0]),
        Some(hero),
        "the struck one turns on its striker"
    );
    assert_eq!(
        world.target_of(beasts[1]),
        Some(hero),
        "so does one standing by it"
    );
    for beast in &beasts[2..] {
        let ai = world.neutral_ai.get(*beast).copied().expect("a neutral");
        assert!(
            !ai.awake && world.target_of(*beast).is_none(),
            "the stack at home sleeps on"
        );
    }
}
