//! The other side cannot tell a hero from its illusions by anything it is
//! told; the hero's own side can.

use bota_core::ModifierKind;
use bota_proto::{
    Cheat, EffectId, EntityId, EventKind, Fixed, Order, OutgoingOrder, RejectReason, ServerMsg,
    SlotId, Target, Team, Vec2, WorldView,
};

use super::local::mid_config;
use super::support::an_empty_spot;
use crate::game::{Entity, Illusion, MatchConfig, Modifier, OrderDelay, UnitOrder, World};
use crate::local::LocalMatch;

/// How long a stand-in illusion lasts.
const STAND_IN_TICKS: u32 = 20 * bota_core::rules::TICKS_PER_SECOND;

/// A stand-in for what an illusion maker makes: a body of the hero's seat
/// and kind at `at`, with the hero's level, abilities, items, modifiers
/// (its souls among them), health and mana, facing as it does and standing
/// still, an illusion of it for [`STAND_IN_TICKS`], the other side's aims at
/// the hero called off.
fn stand_in_illusion(world: &mut World, hero: Entity, at: Vec2) -> Entity {
    let team = *world.team.get(hero).expect("a side");
    let slot = *world.owner.get(hero).expect("a seat");
    let kind = *world.hero.get(hero).expect("a hero");
    let illusion = world.spawn_hero(team, at, slot, kind);
    let level = *world.level.get(hero).expect("a level");
    let book = world.abilities.get(hero).expect("a kit").clone();
    let bag = world.inventory.get(hero).expect("a bag").clone();
    let modifiers = world.modifiers.get(hero).cloned();
    let health = *world.health.get(hero).expect("health");
    let mana = *world.mana.get(hero).expect("mana");
    let facing = world.transform.get(hero).expect("standing").facing;
    world.level.insert(illusion, level);
    world.abilities.insert(illusion, book);
    world.inventory.insert(illusion, bag);
    if let Some(modifiers) = modifiers {
        world.modifiers.insert(illusion, modifiers);
    }
    world.health.insert(illusion, health);
    world.mana.insert(illusion, mana);
    world.transform.get_mut(illusion).expect("standing").facing = facing;
    world.set_order(illusion, UnitOrder::Stand);
    world.illusion.insert(
        illusion,
        Illusion {
            of: hero,
            made: world.tick,
        },
    );
    world.put_modifier(
        illusion,
        Modifier::new(
            ModifierKind::Illusion,
            Some(hero),
            None,
            Some(STAND_IN_TICKS),
        ),
    );
    world.call_off_aims_at(hero);
    world.settle();
    illusion
}

/// A match whose heroes stand still in sight of each other on open ground,
/// a second and a half after they were put there, long enough for whatever
/// the fountain left on them to wear off: the radiant hero and where it
/// stands.
fn face_off(cfg: &MatchConfig) -> (LocalMatch, Entity, Vec2) {
    let (mut local, _) = LocalMatch::new(cfg, &[OrderDelay::NONE]).expect("a match");
    let stop = |seq| OutgoingOrder {
        seq,
        unit: None,
        order: Order::Move {
            target: Target::None,
        },
        seen: local.tick(),
        queue: false,
    };
    local.step(&[&[stop(1)], &[stop(1)]]).expect("a step");
    let at = an_empty_spot(local.world());
    let world = local.world_mut();
    let [radiant, dire] = [0, 1].map(|seat| world.seats[seat].unit.expect("standing"));
    world.transform.get_mut(radiant).expect("standing").pos = at;
    world.transform.get_mut(dire).expect("standing").pos = at + Vec2::from_ints(0, 500);
    world.settle();
    for _ in 0..45 {
        local.step(&[&[], &[]]).expect("a step");
    }
    assert!(
        local.handle_of(1, radiant).is_some(),
        "the dire side saw it"
    );
    (local, radiant, at)
}

/// The view a step's messages hold.
fn view_in(stream: &[ServerMsg]) -> WorldView {
    stream
        .iter()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => Some(view.clone()),
            _ => None,
        })
        .expect("a snapshot")
}

/// A view with every handle renamed by where its unit stands, then by the
/// order first met: two views that differ only in which of two names went
/// to which body compare equal.
fn by_place(mut view: WorldView) -> WorldView {
    let mut places: Vec<(Vec2, EntityId)> = view.units.iter().map(|u| (u.pos, u.id)).collect();
    places.sort_unstable_by_key(|(at, _)| (at.x, at.y));
    let mut met: Vec<EntityId> = places.into_iter().map(|(_, id)| id).collect();
    view.visit_ids(&mut |id| {
        let at = met.iter().position(|seen| seen == id).unwrap_or_else(|| {
            met.push(*id);
            met.len() - 1
        });
        *id = EntityId {
            idx: at as u32,
            generation: 0,
        };
    });
    view.units.sort_unstable_by_key(|unit| unit.id);
    view
}

#[test]
fn the_other_side_sees_the_same_whichever_body_is_the_hero() {
    let cfg = mid_config();
    let (local, hero, at) = face_off(&cfg);
    let beside = at + Vec2::from_ints(200, 0);
    // In one match the illusion stands beside the hero, in the other the
    // hero stands beside it. The hero spent mana its illusion did not.
    let play = |hero_at: Vec2, illusion_at: Vec2| {
        let mut local = local.clone();
        let world = local.world_mut();
        world.transform.get_mut(hero).expect("standing").pos = hero_at;
        let illusion = stand_in_illusion(world, hero, illusion_at);
        world.mana.get_mut(hero).expect("mana").mana = Fixed::from_int(40);
        world.settle();
        let step = local.step(&[&[], &[]]).expect("a step");
        (local, illusion, view_in(&step[0]), view_in(&step[1]))
    };
    let (local_a, illusion, own_a, other_a) = play(at, beside);
    let (_, _, own_b, other_b) = play(beside, at);
    assert_eq!(
        by_place(other_a.clone()),
        by_place(other_b),
        "the dire side"
    );
    assert_ne!(by_place(own_a.clone()), by_place(own_b), "the radiant side");

    let names = [hero, illusion].map(|unit| local_a.handle_of(1, unit).expect("told"));
    let shown = names.map(|name| other_a.unit(name).expect("in sight").clone());
    assert_eq!(shown[0].mana, shown[1].mana, "the hero's sheet on both");
    assert!(
        shown
            .iter()
            .all(|unit| unit.effects.iter().all(|e| e.id != EffectId::ILLUSION))
    );
    let seat = other_a.player(SlotId(0)).expect("the radiant seat");
    assert_eq!(seat.unit, None, "never told which unit is the hero");
    let own = local_a
        .handle_of(0, illusion)
        .expect("its own side knows it");
    let marked = own_a
        .unit(own)
        .expect("its own")
        .effects
        .iter()
        .any(|e| e.id == EffectId::ILLUSION);
    assert!(marked, "its own side is told it is an illusion");
}

/// An illusion lasts as long as what made it says, as Dota's
/// `modifier_illusion` does: its own side is told the time it has left and
/// its full length, the other side nothing of it, and when the time runs out
/// the illusion is gone.
#[test]
fn an_illusion_runs_out_and_only_its_own_side_is_told_its_time() {
    let cfg = mid_config();
    let (mut local, hero, at) = face_off(&cfg);
    let illusion = stand_in_illusion(local.world_mut(), hero, at + Vec2::from_ints(200, 0));
    let step = local.step(&[&[], &[]]).expect("a step");
    let (own, other) = (view_in(&step[0]), view_in(&step[1]));
    let mark = own
        .unit(local.handle_of(0, illusion).expect("its own side knows it"))
        .expect("its own")
        .effects
        .iter()
        .find(|e| e.id == EffectId::ILLUSION)
        .copied()
        .expect("its own side is told it is an illusion");
    assert_eq!(mark.ticks_total, Some(STAND_IN_TICKS));
    assert!(mark.ticks_left.is_some_and(|left| left < STAND_IN_TICKS));
    assert!(
        other
            .units
            .iter()
            .all(|unit| unit.effects.iter().all(|e| e.id != EffectId::ILLUSION)),
        "the other side is told nothing of it"
    );

    for _ in 0..STAND_IN_TICKS {
        local.step(&[&[], &[]]).expect("a step");
    }
    assert!(!local.world().alive(illusion), "the illusion is gone");
    assert!(local.world().alive(hero), "the hero stands");
}

#[test]
fn the_hero_is_named_afresh_and_its_old_name_names_nothing() {
    let mut lower = [false; 2];
    for seed in 0..16 {
        let cfg = MatchConfig {
            match_id: 100 + seed,
            stream: 100 + seed,
            cheats: true,
            ..mid_config()
        };
        let (mut local, hero, at) = face_off(&cfg);
        let old = local.handle_of(1, hero).expect("named before");
        let world = local.world_mut();
        let dire = world.seats[1].unit.expect("standing");
        let last_seen = at;
        world.set_order(
            dire,
            UnitOrder::Attack {
                target: hero,
                last_seen,
            },
        );
        let illusion = stand_in_illusion(world, hero, at + Vec2::from_ints(200, 0));
        let current = world.orders.get(dire).expect("orders").current;
        assert_eq!(
            current,
            UnitOrder::Idle,
            "its attack on the hero is called off"
        );
        let level_up = OutgoingOrder {
            seq: 2,
            unit: None,
            order: Order::Cheat {
                cheat: Cheat::Levels { count: 1 },
            },
            seen: local.tick(),
            queue: false,
        };
        let hit_old = OutgoingOrder {
            seq: 3,
            order: Order::Attack {
                target: Target::Unit(old),
            },
            ..level_up
        };
        let step = local.step(&[&[level_up], &[]]).expect("a step");
        let names = [hero, illusion].map(|unit| local.handle_of(1, unit).expect("told"));
        assert!(!names.contains(&old), "the hero is named afresh");
        lower[usize::from(names[0] < names[1])] = true;
        let after = local.step(&[&[], &[hit_old]]).expect("a step");
        let refused = after[1].iter().any(|msg| {
            matches!(
                msg,
                ServerMsg::OrderRejected {
                    seq: 3,
                    reason: RejectReason::UnknownTarget
                }
            )
        });
        assert!(refused, "the old name names nothing");
        let level_ups: Vec<EntityId> = step[1]
            .iter()
            .filter_map(|msg| match msg {
                ServerMsg::Events { events, .. } => Some(events),
                _ => None,
            })
            .flatten()
            .filter_map(|event| match event {
                EventKind::LevelUp { unit, .. } => Some(*unit),
                _ => None,
            })
            .collect();
        let mut sorted = names.to_vec();
        sorted.sort_unstable();
        assert_eq!(level_ups, sorted, "a level up for each body, by name");
        let view = view_in(&step[1]);
        let levels = names.map(|name| view.unit(name).expect("in sight").level);
        assert_eq!(levels[0], levels[1], "both show the hero's level");
        assert_eq!(view.viewer, Some(Team::Dire));
    }
    assert_eq!(
        lower, [true; 2],
        "the hidden stream, not the setup, orders the names"
    );
}

#[test]
fn an_illusion_falling_out_of_sight_is_not_announced_as_a_hero_kill() {
    let mut world = World::new();
    super::support::seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let hero = super::support::seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();
    let illusion = stand_in_illusion(&mut world, hero, Vec2::from_ints(9200, 9000));
    let mut events = Vec::new();
    world.bury(vec![(illusion, None)], &mut events);
    let fell = |team| {
        crate::game_loop::visible_events(&events, Some(team))
            .iter()
            .any(|kind| matches!(kind, EventKind::Died { unit, .. } if *unit == crate::game::wire_id(illusion)))
    };
    assert!(fell(Team::Dire), "its own side is told");
    assert!(!fell(Team::Radiant), "the feed announces no illusion");
}
