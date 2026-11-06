//! Pudge.

use bota_core::{ModifierKind, rules, tower_def};

use crate::game::{Entity, Inventory, Level, Modifier, Place, World, wire_id};
use bota_proto::{
    AbilityId, AbilitySlot, EffectId, Fixed, HeroId, Inflictor, ItemId, ItemSlot, Order,
    RejectReason, SlotId, StatusFlags, Target, Team, Vec2,
};

use super::support::*;

#[test]
fn a_hook_travels_its_full_range_for_nearby_self_and_far_aims() {
    for offset in [
        Vec2::ZERO,
        Vec2::from_ints(20, 0),
        Vec2::from_ints(100, 100),
        Vec2::from_ints(0, -200),
        Vec2::from_ints(rules::HOOK_RANGE + 200, 0),
        Vec2 {
            x: Fixed { raw: 1 },
            y: Fixed { raw: 1 },
        },
    ] {
        let (mut world, pudge, _) = pudge_and_a_mark(4000);
        let home = world.pos(pudge).expect("Pudge stands");
        cast(&mut world, 0, Target::Pos(home + offset));
        let mut furthest = 0;
        let mut launch = None;
        for _ in 0..100 {
            world.step();
            for entity in world
                .entities
                .iter()
                .filter(|entity| world.hook.get(*entity).is_some())
            {
                // A far point can first be approached under the existing
                // cast-range rules. Travel starts at the actual throw.
                let from =
                    *launch.get_or_insert_with(|| world.pos(pudge).expect("caster at launch"));
                furthest =
                    furthest.max(from.distance_squared(world.pos(entity).expect("Hook flies")));
            }
        }
        assert!(
            furthest >= Fixed::from_int(rules::HOOK_RANGE - 1).squared_raw(),
            "Hook reaches its full range for aim {offset:?}, not merely the clicked point"
        );
        assert!(
            furthest <= Fixed::from_int(rules::HOOK_RANGE + 1).squared_raw(),
            "Hook stays within its range"
        );
    }
}

#[test]
fn a_hook_catches_what_it_flies_into_and_drags_it_home() {
    let (mut world, pudge, mark) = pudge_and_a_hero_mark(600);
    let full = world.health.get(mark).expect("standing").hp;
    let home = world.transform.get(pudge).expect("standing").pos;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
    until_the_hook_flies(&mut world);
    for _ in 0..120 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
    }
    let landed = world.transform.get(mark).expect("standing").pos;
    assert!(
        landed.within(home, Fixed::from_int(200)),
        "what it caught is dragged to the one who threw it: {landed:?} against {home:?}"
    );
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "and an enemy feels it"
    );
}

#[test]
fn a_hook_executes_non_ancient_creeps_and_passes_ancients() {
    let lane = [
        &bota_core::MELEE_CREEP,
        &bota_core::FLAGBEARER_CREEP,
        &bota_core::RANGED_CREEP,
        &bota_core::SIEGE_CREEP,
        &bota_core::SUPER_MELEE_CREEP,
        &bota_core::MEGA_MELEE_CREEP,
        &bota_core::SUPER_RANGED_CREEP,
        &bota_core::MEGA_RANGED_CREEP,
        &bota_core::SUPER_SIEGE_CREEP,
    ];
    for def in lane
        .into_iter()
        .chain(bota_core::NEUTRALS.iter().map(|row| &row.def))
    {
        let (mut world, pudge, old) = pudge_and_a_mark(600);
        world.despawn(old);
        let creep = world.spawn_unit(def, Team::Dire, Vec2::from_ints(5600, 5000));
        world.put_modifier(
            creep,
            Modifier::new(ModifierKind::Stunned, None, None, Some(120)),
        );
        world.settle();
        let before = world.health.get(creep).expect("a live creep").hp;
        let damage_bounty = (!def.ancient).then(|| damage_kill_bounty(&world, pudge, creep));
        cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
        until_the_hook_flies(&mut world);
        world.put_modifier(
            pudge,
            Modifier::new(ModifierKind::Stunned, None, None, Some(120)),
        );
        let events: Vec<_> = (0..80).flat_map(|_| world.step()).collect();
        if def.ancient {
            assert_eq!(
                world.health.get(creep).expect("an ancient stays").hp,
                before
            );
            assert_eq!(world.pos(creep), Some(Vec2::from_ints(5600, 5000)));
        } else {
            assert!(
                !world.alive(creep),
                "Hook executes {:?}, even at {before:?} hp",
                def.kind
            );
            assert!(events.iter().any(|event| matches!(event.kind(),
                bota_proto::EventKind::Died { unit, killer: Some(killer), bounty: Some(bounty), .. }
                    if unit == wire_id(creep) && killer == wire_id(pudge) && Some(bounty) == damage_bounty
            )), "ordinary death credit and bounty");
            assert!(!events.iter().any(|event| matches!(event.kind(),
                bota_proto::EventKind::Damaged { target, inflictor: Some(Inflictor::Ability(AbilityId::MEAT_HOOK)), .. }
                    if target == wire_id(creep)
            )), "HP removal is not spell damage");
        }
    }
}

fn damage_kill_bounty(world: &World, source: Entity, target: Entity) -> i32 {
    let mut control = world.clone();
    let damage = control
        .health
        .get(target)
        .expect("a living target")
        .hp
        .to_int()
        + 1;
    control.push_hit(
        Some(source),
        target,
        damage,
        bota_proto::DamageKind::Pure,
        None,
    );
    control
        .step()
        .into_iter()
        .find_map(|event| match event.kind() {
            bota_proto::EventKind::Died {
                unit,
                killer: Some(killer),
                bounty,
                ..
            } if unit == wire_id(target) && killer == wire_id(source) => bounty,
            _ => None,
        })
        .expect("ordinary damage pays the killing hero")
}

#[test]
fn a_hook_execution_leaves_camp_mates_asleep_but_a_damage_kill_wakes_them() {
    let (mut world, pudge, old) = pudge_and_a_mark(4000);
    world.despawn(old);
    let center = Vec2::from_ints(6000, 5000);
    let victim = world.spawn_neutral(bota_core::NeutralKind::Kobold, center, 0, 0);
    let mates = [-200, 200].map(|offset| {
        world.spawn_neutral(
            bota_core::NeutralKind::Kobold,
            center + Vec2::from_ints(0, offset),
            0,
            0,
        )
    });
    world.settle();
    assert!(
        mates
            .iter()
            .all(|mate| !world.neutral_ai.get(*mate).expect("camp AI").awake)
    );
    let mut damage = world.clone();
    damage.push_hit(
        Some(pudge),
        victim,
        100_000,
        bota_proto::DamageKind::Pure,
        None,
    );
    damage.step();
    let damage_wake = mates.map(|mate| damage.neutral_ai.get(mate).expect("camp AI").awake);
    assert_eq!(
        damage_wake,
        [true, true],
        "a damage kill wakes nearby camp mates"
    );
    cast(&mut world, 0, Target::Pos(center));
    for _ in 0..80 {
        if !world.alive(victim) {
            break;
        }
        world.step();
    }
    assert!(!world.alive(victim), "Hook executed the neutral");
    assert_eq!(
        mates.map(|mate| world.neutral_ai.get(mate).expect("camp AI").awake),
        [false, false],
        "point-target HP removal leaves camp mates asleep"
    );
}

#[test]
fn a_hook_saves_an_allied_creep_without_damage_or_execution() {
    let (mut world, pudge, creep) = pudge_and_a_mark(600);
    world.set_team(creep, Team::Radiant);
    world.put_modifier(
        creep,
        Modifier::new(ModifierKind::Stunned, None, None, Some(120)),
    );
    let before = world.health.get(creep).expect("a live ally").hp;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
    run(&mut world, 80);
    assert_eq!(world.health.get(creep).expect("ally survives").hp, before);
    assert!(world.pos(creep).expect("ally stands").within(
        world.pos(pudge).expect("Pudge stands"),
        Fixed::from_int(200)
    ));
}

/// A hook pulls what it caught to where it was thrown from, wherever its thrower has gone
/// since (Valve 2013-09-23: "Meat Hook now pulls the target to the initial hook position
/// regardless of where Pudge is"; Liquipedia: to "the ability cast location").
#[test]
fn a_hook_pulls_to_where_it_was_thrown_from_not_to_its_thrower() {
    let (mut world, pudge, mark) = pudge_and_a_hero_mark(1200);
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(1200, 0)));
    until_the_hook_flies(&mut world);
    for _ in 0..rules::TICKS_PER_SECOND {
        if hook_holds(&world) {
            break;
        }
        world.step();
    }
    assert!(hook_holds(&world), "the hook latches");
    // Freed by the latch, Pudge walks off north while the hook comes back.
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Pos(home + Vec2::from_ints(0, 600)),
        },
    );
    for _ in 0..rules::TICKS_PER_SECOND * 2 {
        if !hook_flying(&world) {
            break;
        }
        world.step();
    }
    assert!(!hook_flying(&world), "the hook is back");
    let landed = world.pos(mark).expect("standing");
    assert!(
        landed.within(home, Fixed::from_int(60)),
        "the mark lands where the hook was thrown: {landed:?} from {home:?}"
    );
    assert!(
        !world
            .pos(pudge)
            .expect("standing")
            .within(home, Fixed::from_int(150)),
        "while Pudge has walked off"
    );
}

/// A hooked ally is dragged, not stunned: whatever it channels goes on (Valve 2011-11-17:
/// "Fixed Meat Hook interrupting allied channel abilities"; Liquipedia: the hook "neither
/// stuns nor damages the affected allied unit").
#[test]
fn a_hook_drags_an_ally_without_stunning_it() {
    let (mut world, pudge, ally) = pudge_and_a_hero_mark(600);
    world.set_team(ally, Team::Radiant);
    world.settle();
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(600, 0)));
    until_the_hook_flies(&mut world);
    let mut dragged = false;
    for _ in 0..rules::TICKS_PER_SECOND * 2 {
        world.step();
        dragged |= hook_holds(&world);
        assert!(
            !carries(&world, ally, ModifierKind::Stunned),
            "the ally is never stunned"
        );
    }
    assert!(dragged, "and was dragged");
}

#[test]
fn a_hook_passes_invulnerable_and_fortified_creeps_of_either_side() {
    for protection in [ModifierKind::Shielded, ModifierKind::Fortified] {
        for side in [Team::Radiant, Team::Dire] {
            let (mut world, pudge, creep) = pudge_and_a_mark(600);
            world.set_team(creep, side);
            for kind in [protection, ModifierKind::Stunned] {
                world.put_modifier(creep, Modifier::new(kind, None, None, Some(120)));
            }
            world.settle();
            let before = world.pos(creep);
            cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
            until_the_hook_flies(&mut world);
            world.put_modifier(
                pudge,
                Modifier::new(ModifierKind::Stunned, None, None, Some(120)),
            );
            run(&mut world, 80);
            assert!(world.alive(creep), "protected creep lives");
            assert_eq!(world.pos(creep), before, "Hook passes protected creeps");
        }
    }
}

#[test]
fn a_hook_that_catches_nothing_comes_back_by_itself_with_its_chain() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    // Thrown the other way, so it flies out over open ground and returns.
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(4000, 5000)));
    until_the_hook_flies(&mut world);
    let chained = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::MEAT_HOOK)))
            .count()
    };
    assert_eq!(
        chained(&world),
        rules::HOOK_LINKS + 1,
        "the hook and every link of its chain are in the view"
    );
    let mut caught = false;
    for _ in 0..200 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
        caught |= world.entities.iter().any(|entity| {
            world
                .hook
                .get(entity)
                .is_some_and(|hook| hook.caught.is_some())
        });
    }
    assert!(!caught, "it caught nobody over open ground");
    assert!(!hook_flying(&world), "and came back");
    assert_eq!(chained(&world), 0, "the chain went home with the hook");
    assert!(world.alive(pudge));
}

#[test]
fn a_hook_flies_no_further_than_it_reaches() {
    let (mut world, _pudge, _mark) = pudge_and_a_mark(4000);
    // At the very edge of its reach: further off, the caster walks in first.
    cast(
        &mut world,
        0,
        Target::Pos(Vec2::from_ints(5000 + rules::HOOK_RANGE, 5000)),
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
fn the_rot_burns_and_slows_what_stands_in_it_and_lifts_its_linger_after_switched_off() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    let full = world.health.get(mark).expect("standing").hp;
    cast(&mut world, 1, Target::None);
    assert!(
        carries(&world, pudge, ModifierKind::Rot { level: 0, since: 0 }),
        "it is switched on"
    );
    run(&mut world, rules::BURN_PERIOD_TICKS * 4);
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "what stands in it burns"
    );
    assert!(
        carries(
            &world,
            mark,
            ModifierKind::Rotting {
                level: 0,
                slow_pct: 14,
                since: 0,
            }
        ),
        "and is slowed while it stands there"
    );
    cast(&mut world, 1, Target::None);
    assert!(
        !carries(&world, pudge, ModifierKind::Rot { level: 0, since: 0 }),
        "it is switched off"
    );
    run(&mut world, rules::ROT_LINGER_TICKS + 1);
    assert!(
        !carries(
            &world,
            mark,
            ModifierKind::Rotting {
                level: 0,
                slow_pct: 14,
                since: 0,
            }
        ),
        "and nothing is left slowed"
    );
}

/// The ticks at which Rot's damage landed on `unit`, of `events` told by tick, with the
/// amounts.
fn rot_blows(events: &[(u32, crate::game::Event)], unit: Entity) -> Vec<(u32, i32)> {
    events
        .iter()
        .filter_map(|(tick, event)| match event.kind() {
            bota_proto::EventKind::Damaged {
                target,
                amount,
                inflictor: Some(Inflictor::Ability(AbilityId::ROT)),
                ..
            } if target == wire_id(unit) => Some((*tick, amount)),
            _ => None,
        })
        .collect()
}

/// Rot burns as it goes on, and every 0.2 s (`rot_tick`) after, its owner and what stands
/// in it alike: 6 a time at its first level (Liquipedia, *Pudge*: "starting immediately upon
/// debuff application"; Valve 2014-05-19: "Fixed Rot dealing one instance of self-damage
/// while dealing two to the enemy").
#[test]
fn the_rot_burns_at_once_and_every_fifth_of_a_second_its_owner_and_enemies_alike() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    let mut events: Vec<(u32, crate::game::Event)> = Vec::new();
    for event in cast(&mut world, 1, Target::None) {
        events.push((0, event));
    }
    for tick in 1..13 {
        for event in world.step() {
            events.push((tick, event));
        }
    }
    let times = |blows: Vec<(u32, i32)>| blows.into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(times(rot_blows(&events, pudge)), [0, 6, 12], "its owner");
    assert_eq!(
        rot_blows(&events, mark),
        [(0, 6), (6, 6), (12, 6)],
        "and the creep in it, six a time"
    );
}

/// Whether a unit is slowed by a rot.
fn rotting(world: &World, unit: Entity) -> bool {
    carries(
        world,
        unit,
        ModifierKind::Rotting {
            level: 0,
            slow_pct: 0,
            since: 0,
        },
    )
}

/// What steps out of Rot stays slowed for its 0.5 s aura linger, but is burned no more
/// (Liquipedia, *Pudge*: "Aura Linger Duration: 0.5"; "The lingering debuff only applies the
/// movement speed slow component").
#[test]
fn the_rot_slows_half_a_second_past_leaving_it_and_burns_no_more() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    cast(&mut world, 1, Target::None);
    run(&mut world, 3);
    assert!(rotting(&world, mark), "slowed in it");
    world.transform.get_mut(mark).expect("standing").pos =
        world.pos(pudge).expect("Pudge stands") + Vec2::from_ints(rules::ROT_RADIUS + 200, 0);
    // Held there, it walks back into nothing.
    world.put_modifier(
        mark,
        Modifier::new(ModifierKind::Stunned, None, None, Some(60)),
    );
    let left = world.health.get(mark).expect("standing").hp;
    run(&mut world, 13);
    assert!(rotting(&world, mark), "still slowed a while out of it");
    assert!(
        world.health.get(mark).expect("standing").hp >= left,
        "but burned no more"
    );
    run(&mut world, 5);
    assert!(!rotting(&world, mark), "and freed half a second on");
}

/// Switching Rot on or off charges no enemy Magic Stick; casting the meat shield does
/// (Liquipedia, *Magic Stick*: no charge from "toggle-able abilities"; Valve 2011-07-01:
/// "Toggling On / Off Rot no longer proc and grant charges to Magic Stick-based items").
#[test]
fn switching_rot_charges_no_enemy_stick_and_a_cast_does() {
    let (mut world, pudge, foe) = pudge_and_a_hero_mark(400);
    set_level(&mut world, pudge, 1, 1);
    set_level(&mut world, pudge, 2, 1);
    world.fill_pools(pudge);
    hand_item(&mut world, foe, ItemId::MAGIC_STICK, 0);
    let charges = |world: &World| {
        world.inventory.get(foe).expect("a bag").slots[0]
            .map(|stack| stack.charges)
            .expect("the stick")
    };
    cast(&mut world, 1, Target::None);
    cast(&mut world, 1, Target::None);
    assert_eq!(charges(&world), 0, "on and off, no charge");
    cast(&mut world, 2, Target::None);
    assert_eq!(charges(&world), 1, "a meat shield is a charge");
}

/// Switched on and straight off, Rot still burns its owner once (Valve 2012-06-28: "Fixed a
/// bug that allowed Rot not to deal damage to self by quickly toggling On / Off").
#[test]
fn the_rot_switched_on_and_off_at_once_still_burns_its_owner() {
    // Whatever tick it is switched on at.
    for wait in 0..3 {
        let (mut world, pudge, _mark) = pudge_and_a_mark(1500);
        set_level(&mut world, pudge, 1, 1);
        run(&mut world, wait);
        let mut events: Vec<(u32, crate::game::Event)> = Vec::new();
        for event in cast(&mut world, 1, Target::None) {
            events.push((0, event));
        }
        for event in cast(&mut world, 1, Target::None) {
            events.push((1, event));
        }
        for tick in 2..10 {
            for event in world.step() {
                events.push((tick, event));
            }
        }
        assert_eq!(
            rot_blows(&events, pudge).len(),
            1,
            "one burn, {wait} ticks in"
        );
    }
}

#[test]
fn the_rot_shows_as_one_rot_on_its_owner_and_on_what_stands_in_it() {
    // Dota shows modifier_pudge_rot, "Rot", on Pudge and on the enemies in
    // it: one icon each, not a burn and a slow, and the toggle is on while
    // Pudge wears the rot he put on.
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    cast(&mut world, 1, Target::None);
    run(&mut world, 2);
    let rot = Some(Inflictor::Ability(AbilityId::ROT));
    let view = world.view_full();
    let shown = |unit| {
        view.units
            .iter()
            .find(|shown| shown.id == wire_id(unit))
            .expect("in the view")
    };
    let own: Vec<_> = shown(pudge)
        .effects
        .iter()
        .filter(|e| e.inflictor == rot)
        .collect();
    assert_eq!(own.len(), 1, "one rot on Pudge: {own:?}");
    assert_eq!(own[0].id, EffectId::ROT);
    assert_eq!(own[0].caster, Some(wire_id(pudge)));
    assert_eq!(own[0].ticks_left, None, "a toggle runs until switched off");
    let theirs: Vec<_> = shown(mark)
        .effects
        .iter()
        .filter(|e| e.inflictor == rot)
        .collect();
    assert_eq!(theirs.len(), 1, "one rot on what stands in it: {theirs:?}");
    assert_eq!(theirs[0].id, EffectId::ROT);
    assert_eq!(theirs[0].caster, Some(wire_id(pudge)));
    let bits = shown(mark).statuses.bits;
    assert_eq!(
        bits & (StatusFlags::SLOWED | StatusFlags::DOT),
        StatusFlags::SLOWED | StatusFlags::DOT,
        "it slows and burns"
    );
    assert!(
        shown(pudge)
            .abilities
            .iter()
            .any(|ability| ability.id == AbilityId::ROT && ability.on),
        "the toggle shows on"
    );
    cast(&mut world, 1, Target::None);
    run(&mut world, 3);
    let view = world.view_full();
    let pudge_view = view
        .units
        .iter()
        .find(|shown| shown.id == wire_id(pudge))
        .expect("in the view");
    assert!(
        pudge_view
            .abilities
            .iter()
            .any(|ability| ability.id == AbilityId::ROT && !ability.on),
        "and off again"
    );
    assert!(pudge_view.effects.iter().all(|e| e.inflictor != rot));
}

#[test]
fn the_rot_shows_its_cloud_where_its_owner_stands_while_it_is_on() {
    // The mark stands too far off to be taken on, so he only moves when
    // moved.
    let (mut world, pudge, _mark) = pudge_and_a_mark(2000);
    set_level(&mut world, pudge, 1, 1);
    cast(&mut world, 1, Target::None);
    let cloud = world
        .mark_of(pudge, AbilityId::ROT)
        .expect("switched on, the cloud shows");
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert_eq!(world.transform.get(cloud).map(|t| t.pos), Some(stood));
    world.transform.get_mut(pudge).expect("standing").pos = stood + Vec2::from_ints(0, 90);
    world.step();
    assert_eq!(
        world.transform.get(cloud).map(|t| t.pos),
        world.transform.get(pudge).map(|t| t.pos),
        "and follows him"
    );
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .any(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::ROT))),
        "and is in the view"
    );
    cast(&mut world, 1, Target::None);
    assert_eq!(
        world.mark_of(pudge, AbilityId::ROT),
        None,
        "switched off, it goes"
    );
    cast(&mut world, 1, Target::None);
    assert!(world.mark_of(pudge, AbilityId::ROT).is_some());
    world.bury(vec![(pudge, None)], &mut Vec::new());
    assert!(
        world.entities.iter().all(|entity| world
            .mark
            .get(entity)
            .is_none_or(|shown| shown.ability != AbilityId::ROT)),
        "and it goes with him when he falls"
    );
}

#[test]
fn the_rot_never_kills_the_one_carrying_it() {
    let (mut world, pudge) = a_lone_pudge();
    set_level(&mut world, pudge, 1, 3);
    world.put_modifier(
        pudge,
        Modifier::new(
            ModifierKind::Rot { level: 3, since: 0 },
            Some(pudge),
            None,
            None,
        ),
    );
    world.health.insert(pudge, health(20));
    run(&mut world, rules::BURN_PERIOD_TICKS * 40);
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
    set_level(&mut world, pudge, 3, 1);
    world.health.insert(pudge, health(300));
    world.step();
    let hurt = world.health.get(pudge).expect("standing").hp;
    let full = world.health.get(mark).expect("standing").hp;
    dismember(&mut world, mark);
    assert!(world.is_channelling(pudge), "it takes hold");
    run(&mut world, 30);
    assert!(
        carries(&world, mark, ModifierKind::Stunned),
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
    stop_click(&mut world);
    assert!(!world.is_channelling(pudge), "an order of any kind lets go");
}

#[test]
fn a_dismember_shows_its_hold_on_what_it_eats_until_it_lets_go() {
    let (mut world, pudge, mark) = pudge_and_a_mark(100);
    set_level(&mut world, pudge, 3, 1);
    dismember(&mut world, mark);
    let shown = world
        .mark_of(pudge, AbilityId::DISMEMBER)
        .expect("the hold is shown");
    let eaten = world.transform.get(mark).expect("standing").pos;
    assert_eq!(world.transform.get(shown).map(|t| t.pos), Some(eaten));
    world.transform.get_mut(mark).expect("standing").pos = eaten + Vec2::from_ints(40, 0);
    world.step();
    assert_eq!(
        world.transform.get(shown).map(|t| t.pos),
        world.transform.get(mark).map(|t| t.pos),
        "and follows what is eaten"
    );
    stop_click(&mut world);
    assert_eq!(
        world.mark_of(pudge, AbilityId::DISMEMBER),
        None,
        "letting go takes the hold away"
    );
}

#[test]
fn a_flesh_heap_keeps_every_enemy_hero_that_falls_near_it_and_nothing_else() {
    let (mut world, pudge, mark) = pudge_and_a_mark(200);
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    let friend = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5300),
        SlotId(2),
        HeroId::SHADOW_FIEND,
    );
    let far = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5000 + rules::FLESH_HEAP_RANGE + 200, 5000),
        SlotId(3),
        HeroId::SHADOW_FIEND,
    );
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5200, 5000));
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(mark, None), (tower, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        0,
        "a creep or a structure falling beside it feeds it nothing"
    );
    world.bury(vec![(friend, None), (far, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        0,
        "neither a hero of its own side, nor an enemy hero falling too far off"
    );
    // Brought down the way a fight brings a hero down: dead before it is
    // buried.
    world.health.insert(foe, health(0));
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        1,
        "an enemy hero falling beside it feeds it"
    );
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK),
        "and one stack is worth its strength"
    );
}

/// Pudge has no mana regeneration of his own (`npc_dota_hero_base`'s `StatusManaRegen` 0;
/// his script sets none): at level one only his 16 intelligence mends it, 0.8 a second
/// (the dota2.com hero data gives 0.8).
#[test]
fn pudge_mends_only_the_mana_his_intelligence_gives_eight_in_ten_seconds() {
    let (mut world, pudge) = a_lone_pudge();
    world.mana.get_mut(pudge).expect("a hero has mana").mana = Fixed::ZERO;
    run(&mut world, 10 * rules::TICKS_PER_SECOND);
    let mended = world.mana.get(pudge).expect("a hero has mana").mana;
    // A tick's share of 0.8 is rounded down, so ten seconds fall a hair short of 8.
    assert!(
        mended > Fixed::from_ratio(79, 10) && mended <= Fixed::from_int(8),
        "0.8 mana a second, none of it his own: {mended}"
    );
}

/// A hero Pudge brings down feeds his heap however far off it falls (Liquipedia, *Pudge*:
/// "Grants Pudge a stack if the enemy hero kill is credited to him, regardless of
/// distance"; the dota2.com hero data: "every time he kills an enemy Hero").
#[test]
fn a_flesh_heap_keeps_an_enemy_hero_its_bearer_kills_however_far_off() {
    let (mut world, pudge, hero) = pudge_and_a_hero_mark(rules::FLESH_HEAP_RANGE + 1000);
    world.health.insert(hero, health(0));
    let mut events = Vec::new();
    world.bury(vec![(hero, Some(pudge))], &mut events);
    world.step();
    assert_eq!(heap(&world, pudge), 1, "his own kill feeds it");
}

/// A fallen Pudge keeps no stack, not even of a hero whose death is credited to him after his
/// own, a blow he left landing late (Liquipedia, *Pudge*: stacks come "as long as Pudge is
/// alive").
#[test]
fn a_dead_flesh_heap_keeps_nothing_even_of_its_bearers_late_kill() {
    let (mut world, pudge, hero) = pudge_and_a_hero_mark(200);
    world.health.insert(pudge, health(0));
    let mut events = Vec::new();
    world.bury(vec![(pudge, Some(hero))], &mut events);
    world.health.insert(hero, health(0));
    world.bury(vec![(hero, Some(pudge))], &mut events);
    world.step();
    assert_eq!(heap(&world, pudge), 0, "the dead keep nothing");
}

/// An enemy illusion falling beside Pudge is no hero to the heap (Liquipedia, *Pudge*: Flesh
/// Heap "considers ... enemy illusions ... as creeps").
#[test]
fn a_flesh_heap_keeps_nothing_of_an_enemy_illusion_falling_near_it() {
    let (mut world, pudge, hero) = pudge_and_a_hero_mark(200);
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&e| world.illusion.get(e).is_some_and(|made| made.of == hero))
        .expect("a rune makes illusions");
    world.transform.get_mut(illusion).expect("standing").pos =
        world.pos(pudge).expect("Pudge stands") + Vec2::from_ints(150, 0);
    world.settle();
    world.health.insert(illusion, health(0));
    let mut events = Vec::new();
    world.bury(vec![(illusion, Some(pudge))], &mut events);
    world.step();
    assert_eq!(heap(&world, pudge), 0, "an illusion feeds it nothing");
}

/// A stack is worth two strength flat, whatever the hero's level, and the heap
/// leaves magic resistance alone.
#[test]
fn a_stack_of_the_heap_is_worth_two_strength_and_nothing_else() {
    let (mut world, pudge) = a_lone_pudge();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let resist = world.stats.get(pudge).map(|s| s.magic_resist_bp);
    world
        .modifiers
        .get_mut(pudge)
        .expect("a hero wears modifiers")
        .set_count(ModifierKind::flesh_heap, 3, (pudge, AbilityId::FLESH_HEAP));
    world.step();
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + Fixed::from_int(6)),
        "three stacks are worth two strength each"
    );
    assert_eq!(world.stats.get(pudge).map(|s| s.magic_resist_bp), resist);
}

#[test]
fn a_break_leaves_the_flesh_heap_whole() {
    // Liquipedia, Break, among the abilities not disabled by it: "Pudge -
    // Flesh Heap".
    let (mut world, pudge) = a_lone_pudge();
    world
        .modifiers
        .get_mut(pudge)
        .expect("a hero wears modifiers")
        .set_count(ModifierKind::flesh_heap, 3, (pudge, AbilityId::FLESH_HEAP));
    world.step();
    let strength = world.stats.get(pudge).map(|s| s.attributes.strength);
    put_on(&mut world, pudge, ModifierKind::Broken, 600);
    world.step();
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        strength,
        "broken, his three stacks are worth their strength on"
    );
}

#[test]
fn a_flesh_heap_outlives_the_death_of_the_one_carrying_it() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5200),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(heap(&world, pudge), 1, "one death has fed it");
    world.bury(vec![(pudge, None)], &mut events);
    assert!(world.seats[0].dead(), "it lies dead");
    let back = back_on_its_feet(&mut world);
    assert_eq!(heap(&world, back), 1, "and what it kept comes back with it");
    assert_eq!(
        world.stats.get(back).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK),
        "worth as much strength as it was before"
    );
}

/// Ticks a hook's followthrough holds its thrower, less one to spare: the hook's
/// reach over its speed, times its `self_stun_duration_percent` of 65 (Liquipedia, *Pudge*:
/// "MaxTravelDistance / HookSpeed × 0.65", 0.53 s).
const FOLLOWTHROUGH_SURE_TICKS: u32 = 14;

/// Throws a hook from Pudge east into nothing and steps until it flies.
fn a_hook_into_nothing() -> (World, Entity) {
    let (mut world, pudge, creep) = pudge_and_a_mark(600);
    world.despawn(creep);
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(600, 0)));
    until_the_hook_flies(&mut world);
    (world, pudge)
}

/// Meat Hook's followthrough (`modifier_pudge_meat_hook_followthrough`): rooted with his facing
/// locked until it ends (Liquipedia, *Pudge*; Valve 2012-04-12 "Reduced the delay before being
/// able to issue orders after casting").
#[test]
fn a_hook_thrower_neither_walks_nor_turns_during_its_followthrough() {
    let (mut world, pudge) = a_hook_into_nothing();
    let (home, facing) = {
        let at = world.transform.get(pudge).expect("standing");
        (at.pos, at.facing)
    };
    let behind = Target::Pos(home - Vec2::from_ints(600, 0));
    seat_order(&mut world, Order::Move { target: behind });
    run(&mut world, FOLLOWTHROUGH_SURE_TICKS - 1);
    let at = *world.transform.get(pudge).expect("standing");
    assert_eq!(at.pos, home, "he stands where he threw");
    assert_eq!(at.facing, facing, "facing where he threw");
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(
        world.pos(pudge).expect("standing").x < home.x,
        "and walks once it ends"
    );
}

/// The followthrough disarms: a foe right in front of him is not swung at until it ends.
#[test]
fn a_hook_thrower_swings_at_nothing_during_its_followthrough() {
    let (mut world, pudge) = a_hook_into_nothing();
    let home = world.pos(pudge).expect("Pudge stands");
    // In front, in reach, and passed by the hook already: a courier, which it never catches.
    let foe = world.spawn_courier(
        Team::Dire,
        home + Vec2::from_ints(120, 0),
        SlotId(1),
        Inventory::empty(6),
    );
    world.settle();
    attack_click(&mut world, foe);
    for _ in 0..FOLLOWTHROUGH_SURE_TICKS - 1 {
        assert_eq!(swinging(&world, pudge), None, "no swing begins");
        world.step();
    }
    run(&mut world, rules::TICKS_PER_SECOND / 2);
    assert_eq!(swinging(&world, pudge), Some(foe), "until it ends");
}

/// The followthrough mutes: no item is used until it ends.
#[test]
fn a_hook_thrower_uses_no_item_during_its_followthrough() {
    let (mut world, pudge) = a_hook_into_nothing();
    hand_item(&mut world, pudge, ItemId::CLARITY, 1);
    let drink = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(wire_id(pudge)),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &drink),
        Err(RejectReason::Disabled),
        "an item is refused"
    );
    run(&mut world, FOLLOWTHROUGH_SURE_TICKS + 3);
    assert_eq!(
        world.validate_order(SlotId(0), None, &drink),
        Ok(()),
        "until it ends"
    );
}

/// A hook that latches lifts the followthrough at once (Valve 7.34: "Cast stun on Pudge is
/// now removed if the hook finds a target").
#[test]
fn a_hook_that_latches_frees_its_thrower_at_once() {
    let (mut world, pudge, _mark) = pudge_and_a_hero_mark(250);
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(250, 0)));
    until_the_hook_flies(&mut world);
    for _ in 0..6 {
        if hook_holds(&world) {
            break;
        }
        world.step();
    }
    assert!(hook_holds(&world), "the hook latches at once");
    let facing = world.transform.get(pudge).expect("standing").facing;
    seat_order(
        &mut world,
        Order::Move {
            target: Target::Pos(home - Vec2::from_ints(600, 0)),
        },
    );
    run(&mut world, 3);
    assert_ne!(
        world.transform.get(pudge).expect("standing").facing,
        facing,
        "and its thrower turns before the followthrough would have ended"
    );
}

/// Meat Hook kills an enemy illusion outright, as it does a creep (Valve 7.34: "Now
/// instantly kills illusions"; Liquipedia: it "considers ... illusions as creeps").
#[test]
fn a_hook_kills_an_enemy_illusion_outright() {
    let (mut world, pudge, hero) = pudge_and_a_hero_mark(600);
    let mark = world.pos(hero).expect("standing");
    // Sturdy enough that no hook's damage, nor anything else in the time, brings it down.
    world.level.insert(hero, Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(hero);
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&e| world.illusion.get(e).is_some_and(|made| made.of == hero))
        .expect("a rune makes illusions");
    // The hero itself steps far out of the hook's way; one illusion stands where it stood.
    world.transform.get_mut(hero).expect("standing").pos = mark + Vec2::from_ints(0, 3000);
    world.transform.get_mut(illusion).expect("standing").pos = mark;
    for other in world.entities.iter().collect::<Vec<_>>() {
        if other != illusion && world.illusion.get(other).is_some() {
            world.transform.get_mut(other).expect("standing").pos =
                mark + Vec2::from_ints(0, -3000);
        }
    }
    world.settle();
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(600, 0)));
    until_the_hook_flies(&mut world);
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(!world.alive(illusion), "the illusion it caught is dead");
}

/// A hook passes an invulnerable unit by, a hero as much as a creep (Valve 7.03: "Can no
/// longer latch on invulnerable units"): a hero risen in its fountain stays where it is.
#[test]
fn a_hook_passes_an_invulnerable_hero_by() {
    let (mut world, pudge, hero) = pudge_and_a_hero_mark(600);
    world.put_modifier(
        hero,
        Modifier::new(ModifierKind::FountainInvulnerable, None, None, None),
    );
    world.step();
    assert!(
        world.stats.get(hero).expect("settled").invulnerable,
        "the hero is invulnerable"
    );
    let at = world.pos(hero).expect("standing");
    let home = world.pos(pudge).expect("Pudge stands");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(600, 0)));
    until_the_hook_flies(&mut world);
    for _ in 0..rules::TICKS_PER_SECOND * 2 {
        assert!(!hook_holds(&world), "the hook never holds it");
        world.step();
    }
    assert_eq!(world.pos(hero), Some(at), "and it stands where it stood");
}

/// Whether a flying hook holds something.
fn hook_holds(world: &World) -> bool {
    world
        .entities
        .iter()
        .any(|e| world.hook.get(e).is_some_and(|hook| hook.caught.is_some()))
}

/// Dismember takes `DOTA_UNIT_TARGET_HERO | DOTA_UNIT_TARGET_BASIC`
/// (`npc_dota_hero_pudge.txt`): a tower that may be struck is no target.
#[test]
fn a_dismember_at_a_tower_is_refused_and_holds_nothing() {
    let (mut world, pudge, tower) = pudge_and_a_tower(150);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    let at_the_tower = Order::Cast {
        slot: AbilitySlot(3),
        target: Target::Unit(wire_id(tower)),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &at_the_tower),
        Err(RejectReason::WrongTargetKind),
        "a building is not a unit Dismember takes"
    );
    cast(&mut world, 3, Target::Unit(wire_id(tower)));
    run(&mut world, rules::DISMEMBER_CAST_POINT_MS / 30 + 5);
    assert!(!world.is_channelling(pudge), "and nothing is held");
}

/// Rot burns and slows Dota's heroes and basic units: a building in it
/// stands untouched.
#[test]
fn the_rot_leaves_a_tower_standing_in_it_untouched() {
    let (mut world, pudge, tower) = pudge_and_a_tower(150);
    set_level(&mut world, pudge, 1, 4);
    // Only the rot may touch the tower: Pudge's own swings would.
    world.put_modifier(
        pudge,
        Modifier::new(ModifierKind::Disarmed, None, None, None),
    );
    let full = world.health.get(tower).expect("standing").hp;
    cast(&mut world, 1, Target::None);
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(
        !carries(
            &world,
            tower,
            ModifierKind::Rotting {
                level: 3,
                slow_pct: 32,
                since: 0,
            }
        ),
        "the rot lays nothing on a building"
    );
    assert_eq!(
        world.health.get(tower).expect("standing").hp,
        full,
        "nor burns it"
    );
}

/// Pudge beside an enemy tier-1 tower, which may be struck, `apart` units
/// off along x.
fn pudge_and_a_tower(apart: i32) -> (World, Entity, Entity) {
    let (mut world, pudge, creep) = pudge_and_a_mark(apart);
    world.despawn(creep);
    let at = world.pos(pudge).expect("Pudge stands") + Vec2::from_ints(apart, 0);
    let tower = world.spawn_building(
        tower_def(1),
        Team::Dire,
        at,
        Place::Tower { lane: 1, tier: 1 },
    );
    world.settle();
    world.step();
    assert!(
        !world.stats.get(tower).expect("settled").invulnerable,
        "a first tower may be struck"
    );
    (world, pudge, tower)
}

/// A hero target exercises Hook's damage and pull instead of creep execution.
pub(super) fn pudge_and_a_hero_mark(apart: i32) -> (World, Entity, Entity) {
    let (mut world, pudge, creep) = pudge_and_a_mark(apart);
    world.despawn(creep);
    let at = world.pos(pudge).expect("Pudge stands") + Vec2::from_ints(apart, 0);
    let hero = world.spawn_hero(Team::Dire, at, SlotId(1), HeroId::SHADOW_FIEND);
    world.settle();
    (world, pudge, hero)
}

/// Steps until the hook has left Pudge: its cast point is over.
pub(super) fn until_the_hook_flies(world: &mut World) {
    for _ in 0..rules::HOOK_CAST_POINT_MS / 30 + 2 {
        if hook_flying(world) {
            return;
        }
        world.step();
    }
    assert!(hook_flying(world), "the hook leaves after its cast point");
}

/// Orders a dismember and steps until its cast point is over and it holds.
pub(super) fn dismember(world: &mut World, mark: Entity) {
    cast(world, 3, Target::Unit(wire_id(mark)));
    for _ in 0..rules::DISMEMBER_CAST_POINT_MS / 30 + 2 {
        if mark_is_held(world, mark) {
            return;
        }
        world.step();
    }
    assert!(
        mark_is_held(world, mark),
        "it takes hold after its cast point"
    );
}

fn mark_is_held(world: &World, mark: Entity) -> bool {
    carries(world, mark, ModifierKind::Stunned)
}

/// Pudge alone on an empty world at (5000, 5000), settled and one tick on.
pub(super) fn a_lone_pudge() -> (World, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId::PUDGE,
    );
    world.settle();
    world.step();
    (world, pudge)
}

pub(super) fn heap(world: &World, hero: Entity) -> u32 {
    world
        .modifiers
        .get(hero)
        .map_or(0, |on_it| on_it.count_of(ModifierKind::flesh_heap))
}

#[test]
fn a_hook_costs_120_mana_waits_18_seconds_and_deals_150_pure_damage_at_its_first_level() {
    let (mut world, pudge, mark) = pudge_and_a_hero_mark(600);
    let full = world.health.get(mark).expect("standing").hp.to_int();
    world.fill_pools(pudge);
    let mana = world.mana.get(pudge).expect("has mana").mana;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(5600, 5000)));
    until_the_hook_flies(&mut world);
    assert_eq!(
        mana - world.mana.get(pudge).expect("has mana").mana,
        Fixed::from_int(120),
        "the hook costs 120 mana"
    );
    let mut impact = None;
    for _ in 0..200 {
        for event in world.step() {
            if let bota_proto::EventKind::Damaged {
                target,
                amount,
                kind,
                inflictor: Some(Inflictor::Ability(AbilityId::MEAT_HOOK)),
                ..
            } = event.kind()
                && target == wire_id(mark)
            {
                impact = Some((
                    amount,
                    kind,
                    world.health.get(mark).expect("standing").hp.to_int(),
                ));
            }
        }
        if !hook_flying(&world) {
            break;
        }
    }
    assert_eq!(
        impact,
        Some((150, bota_proto::DamageKind::Pure, full - 150)),
        "a hero takes 150 pure damage at impact, before its later regeneration"
    );
    let waiting = world.abilities.get(pudge).expect("a kit").slots[0].cooldown;
    assert!(
        (540 - 40..=540).contains(&waiting),
        "and the next hook is 18 seconds off, less the flight: {waiting} ticks"
    );
}

/// The dismember bites six times, the first as it takes hold: each bite is
/// the level's damage a second and 0.3 a point of strength, over 2.75 seconds
/// shared six ways, and the one holding mends by every bite. It pulls
/// what it holds to 125 units away at 75 a second.
#[test]
fn a_dismember_bites_six_times_by_strength_pulls_and_heals_by_every_bite() {
    let (mut world, pudge, mark) = pudge_and_a_mark(190);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    world.health.insert(pudge, health(300));
    world.step();
    let strength = world.stats.get(pudge).expect("settled").attributes.strength;
    let bite = bota_core::dismember_bite_damage(
        bota_core::Dec::of("80"),
        bota_core::Dec::of("0.3"),
        strength,
        bota_core::Ms::secs("2.75"),
        6,
    );
    assert_eq!(
        bite,
        ((Fixed::from_int(80) + strength * Fixed::from_ratio(3, 10))
            * Fixed::from_ratio(2750, 6000))
        .to_int(),
        "a bite is the second's damage over 2.75 seconds in six"
    );
    let start = world.health.get(pudge).expect("standing").hp.to_int();
    let mut left = world.health.get(mark).expect("standing").hp.to_int();
    let mut bites = Vec::new();
    cast(&mut world, 3, Target::Unit(wire_id(mark)));
    for tick in 0..rules::DISMEMBER_TICKS + 5 {
        let now = world.health.get(mark).expect("standing").hp.to_int();
        // Its own regeneration may give back a point between two readings.
        if left - now >= bite - 1 {
            assert!(left - now <= bite, "each bite takes the same");
            bites.push(tick);
        }
        left = now;
        world.step();
    }
    assert_eq!(
        bites.len(),
        rules::DISMEMBER_BITES as usize,
        "it bites six times in all"
    );
    assert_eq!(
        bites[0],
        ticks_of(rules::DISMEMBER_CAST_POINT_MS),
        "the first as it takes hold, once the cast point is over"
    );
    let gaps: Vec<u32> = bites.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert_eq!(gaps, [14, 14, 14, 13, 14], "evenly over 2.75 seconds");
    let healed = world.health.get(pudge).expect("standing").hp.to_int() - start;
    let regen = world.stats.get(pudge).expect("settled").hp_regen
        * Fixed::from_int((rules::DISMEMBER_TICKS + 5) as i32);
    assert!(
        (bite * 6..=bite * 6 + regen.to_int() + 1).contains(&healed),
        "and the one eating mends by all of it, his own regeneration aside: {healed}"
    );
    let apart = world.transform.get(mark).expect("standing").pos
        - world.transform.get(pudge).expect("standing").pos;
    assert!(
        (124..=126).contains(&apart.x.to_int()),
        "what it held was pulled in to 125: {apart:?}"
    );
}

/// A dismember heals its holder by each bite's full damage, whatever the magic resistance
/// of what it eats takes off (Liquipedia, *Pudge*: it "heals for the full value regardless
/// of the damage instance value"; the hero data: "heals him for the total amount").
#[test]
fn a_dismember_heals_by_each_bites_full_damage_not_by_what_resistance_leaves() {
    let (mut world, pudge, mark) = pudge_and_a_hero_mark(150);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    world.health.insert(pudge, health(200));
    world.step();
    assert!(
        world.stats.get(mark).expect("settled").magic_resist_bp > 0,
        "a hero resists magic"
    );
    let strength = world.stats.get(pudge).expect("settled").attributes.strength;
    let bite = bota_core::dismember_bite_damage(
        bota_core::Dec::of("80"),
        bota_core::Dec::of("0.3"),
        strength,
        bota_core::Ms::secs("2.75"),
        6,
    );
    let start = world.health.get(pudge).expect("standing").hp;
    dismember(&mut world, mark);
    let healed = (world.health.get(pudge).expect("standing").hp - start).to_int();
    // The cast point's few ticks of his own regeneration come with it.
    assert!(
        (bite..=bite + 2).contains(&healed),
        "the take-hold bite heals {bite}: healed {healed}"
    );
}

/// A dismember ends the moment what it eats turns invulnerable, a creep its side's Glyph
/// fortifies (Liquipedia, *Pudge*: invulnerability "interrupts the channeling").
#[test]
fn a_dismember_lets_go_of_a_creep_its_glyph_fortifies() {
    let (mut world, pudge, creep) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    dismember(&mut world, creep);
    assert!(world.is_channelling(pudge), "it takes hold");
    world.put_modifier(
        creep,
        Modifier::new(ModifierKind::Fortified, None, None, Some(150)),
    );
    run(&mut world, 2);
    assert!(
        !world.is_channelling(pudge),
        "and lets go once it is fortified"
    );
}

/// A dismember ends when its hold is taken off what it eats (Valve 2019-08-08: "Fixed
/// Dismember continuing to channel when the target had broken out of the ability").
#[test]
fn a_dismember_ends_when_its_hold_is_taken_off() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    dismember(&mut world, mark);
    assert!(world.is_channelling(pudge), "it takes hold");
    let any = ModifierKind::Dismembered {
        level: 0,
        bites: 0,
        channel_ms: 0,
    };
    world.take_modifier(mark, any, Some(pudge));
    run(&mut world, 2);
    assert!(!world.is_channelling(pudge), "and the channel ends with it");
}

#[test]
fn a_meat_shield_takes_its_block_off_every_blow_for_four_seconds() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(2000);
    set_level(&mut world, pudge, 2, 1);
    world.fill_pools(pudge);
    let mana = world.mana.get(pudge).expect("has mana").mana;
    cast(&mut world, 2, Target::None);
    world.step();
    let spent = mana - world.mana.get(pudge).expect("has mana").mana;
    assert!(
        (64..=65).contains(&spent.to_int()),
        "it costs 65 mana, less a tick of regeneration: {spent}"
    );
    let hp = |world: &World| world.health.get(pudge).expect("standing").hp.to_int();
    let before = hp(&world);
    world.push_hit(None, pudge, 30, bota_proto::DamageKind::Pure, None);
    world.push_hit(None, pudge, 5, bota_proto::DamageKind::Pure, None);
    world.step();
    assert_eq!(
        before - hp(&world),
        30 - 8,
        "every blow loses 8: a blow of 30 costs 22 and one of 5 costs nothing"
    );
    run(&mut world, 4 * rules::TICKS_PER_SECOND);
    let before = hp(&world);
    world.push_hit(None, pudge, 30, bota_proto::DamageKind::Pure, None);
    world.step();
    assert!(
        before - hp(&world) >= 29,
        "and after four seconds it is gone (a point of regeneration aside)"
    );
}

#[test]
fn a_courier_is_spell_immune_rot_and_hook_pass_it_by_and_only_dismember_holds_it() {
    let courier_at = Vec2::from_ints(5150, 5000);
    let enemy_courier = |world: &mut World| {
        let courier = world.spawn_courier(Team::Dire, courier_at, SlotId(1), Inventory::empty(6));
        world.settle();
        courier
    };
    // The hook flies through the courier and executes the non-ancient creep beyond.
    let (mut world, _pudge, mark) = pudge_and_a_mark(600);
    let courier = enemy_courier(&mut world);
    assert!(
        shown(&world, Some(Team::Radiant), courier).statuses.bits & StatusFlags::MAGIC_IMMUNE != 0,
        "shown spell immune"
    );
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
    until_the_hook_flies(&mut world);
    for _ in 0..120 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
    }
    assert_eq!(
        world.pos(courier),
        Some(courier_at),
        "the hook passed it by"
    );
    assert!(
        !world.alive(mark),
        "and executed the non-ancient creep beyond"
    );
    // The rot neither slows nor burns it.
    let (mut world, pudge, _) = pudge_and_a_mark(600);
    let courier = enemy_courier(&mut world);
    set_level(&mut world, pudge, 1, 1);
    let pace = world.stats.get(courier).expect("settled").move_speed;
    cast(&mut world, 1, Target::None);
    run(&mut world, rules::BURN_PERIOD_TICKS * 4);
    assert!(!carries(
        &world,
        courier,
        ModifierKind::Rotting {
            level: 0,
            slow_pct: 14,
            since: 0,
        }
    ));
    assert_eq!(
        world.stats.get(courier).map(|stats| stats.move_speed),
        Some(pace)
    );
    // The dismember holds it, and its bites take none of its hits.
    let (mut world, pudge, _) = pudge_and_a_mark(600);
    let courier = enemy_courier(&mut world);
    set_level(&mut world, pudge, 3, 1);
    dismember(&mut world, courier);
    run(&mut world, rules::DISMEMBER_TICKS / 2);
    assert!(mark_is_held(&world, courier), "held");
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(Fixed::from_int(rules::COURIER_HP))
    );
}
