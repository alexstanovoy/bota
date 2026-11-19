//! Fountain, tower and flagbearer auras.

use bota_core::{
    COURIER, FLAGBEARER_CREEP, FOUNTAIN, MELEE_CREEP, ModifierKind, OBSERVER_WARD, rules, tower_def,
};

use crate::game::{Level, World, wire_id};
use bota_proto::{EffectId, Fixed, HeroId, MapId, Pick, SlotId, Team, Vec2};

use super::support::*;

/// A tower's protection, whatever tier hands it out.
const GUARDED_EFFECT: ModifierKind = ModifierKind::Guarded {
    armor: rules::TOWER_AURA_ARMOR[0],
    hp_per_second: rules::TOWER_AURA_REGEN[0],
};

/// A flagbearer's inspiration.
const INSPIRED_EFFECT: ModifierKind = ModifierKind::Inspired {
    hp_per_second: rules::FLAGBEARER_AURA_REGEN,
    magic_resist_pct: 0,
};

#[test]
fn a_fountain_hands_out_mending_to_its_own_side_standing_in_it() {
    let (mut world, hero) = match_hero();
    world.health.insert(hero, health(100));
    let theirs = world.spawn_hero(
        Team::Dire,
        world.transform.get(hero).expect("standing").pos,
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "standing in it, the hero carries the effect"
    );
    let max_hp = world.stats.get(hero).expect("derived").max_hp;
    let a_tick = max_hp.to_int() * rules::FOUNTAIN_HEAL_HP_PCT / 100 / 30;
    let hp = world.health.get(hero).expect("alive").hp.to_int();
    assert!(
        (100 + a_tick..=101 + a_tick).contains(&hp),
        "and mends a twentieth of its maximum a second: {hp} of {max_hp}"
    );
    assert!(
        !carries(&world, theirs, FOUNTAIN_EFFECT),
        "an enemy standing in it mends no faster for it"
    );
    // Out of reach, nothing takes it off: it runs out on its own.
    world.transform.get_mut(hero).expect("hero").pos = Vec2::from_ints(9600, 9216);
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "one step out it is still running"
    );
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS);
    assert!(
        !carries(&world, hero, FOUNTAIN_EFFECT),
        "three seconds later it is gone"
    );
}

#[test]
fn the_fountain_fills_a_hero_in_twenty_seconds_not_one() {
    let (mut world, hero) = match_hero();
    world.health.insert(hero, health(1));
    world
        .mana
        .insert(hero, crate::game::Mana { mana: Fixed::ZERO });
    let full = |world: &World| {
        let stats = world.stats.get(hero).expect("derived");
        (
            world.health.get(hero).expect("alive").hp >= stats.max_hp,
            world.mana.get(hero).expect("alive").mana >= stats.max_mana,
        )
    };
    run(&mut world, rules::TICKS_PER_SECOND * 2);
    assert_eq!(
        full(&world),
        (false, false),
        "two seconds fill neither pool"
    );
    run(&mut world, rules::TICKS_PER_SECOND * 15);
    assert_eq!(
        full(&world),
        (false, true),
        "six percent fills mana in under 17 s"
    );
    run(&mut world, rules::TICKS_PER_SECOND * 3);
    assert_eq!(
        full(&world),
        (true, true),
        "five percent fills health in 20 s"
    );
}

#[test]
fn a_tower_guards_only_the_heroes_of_its_own_side_that_stand_by_it() {
    // Every tier, since what a tower adds is not the same at each of them.
    for tier in 1..=4u8 {
        let mut world = World::new();
        world.spawn_unit(tower_def(tier), Team::Radiant, Vec2::from_ints(5000, 5000));
        let hero = hero_at(&mut world, Vec2::from_ints(5400, 5000));
        let bare = world.spawn_hero(
            Team::Radiant,
            Vec2::from_ints(5000 + rules::TOWER_AURA_RADIUS + 200, 5000),
            SlotId(1),
            HeroId::SHADOW_FIEND,
        );
        let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5400, 4900));
        let theirs = world.spawn_hero(
            Team::Dire,
            Vec2::from_ints(5400, 5100),
            SlotId(2),
            HeroId::SHADOW_FIEND,
        );
        world.settle();
        world.step();
        let guarded = |unit| carries(&world, unit, GUARDED_EFFECT);
        assert!(guarded(hero), "tier {tier}: inside the reach");
        assert!(!guarded(bare), "tier {tier}: outside it");
        assert!(!guarded(creep), "tier {tier}: not a creep of its own side");
        assert!(!guarded(theirs), "tier {tier}: nor an enemy hero");
        let added = rules::TOWER_AURA_ARMOR[usize::from(tier) - 1];
        assert_eq!(
            world.stats.get(hero).map(|s| s.armor),
            world
                .stats
                .get(bare)
                .map(|s| s.armor + Fixed::from_int(added)),
            "tier {tier}: the armor it hands out is on the stat"
        );
    }
}

#[test]
fn a_flagbearer_inspires_the_creeps_of_its_own_side_around_it() {
    let mut world = World::new();
    world.spawn_unit(
        &FLAGBEARER_CREEP,
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5300, 5000));
    let far = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        Vec2::from_ints(5000 + rules::FLAGBEARER_AURA_RADIUS + 200, 5000),
    );
    let hero = hero_at(&mut world, Vec2::from_ints(5300, 5100));
    let theirs = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 4900),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.step();
    let inspired = |unit| carries(&world, unit, INSPIRED_EFFECT);
    assert!(inspired(creep), "inside the reach");
    assert!(!inspired(far), "outside it");
    assert!(!inspired(hero), "never a hero, since 7.40");
    assert!(!inspired(theirs), "and its own side only");
}

#[test]
fn a_broken_flagbearers_inspiration_stays_on_its_creeps_doing_nothing_and_a_tower_guards_on() {
    // Dota's flagbearer_creep_aura_effect is a passive a Break disables: "The
    // aura itself continues to place its buff or debuff on units, but ...
    // the buff or debuff is disabled for the duration. If the aura has a
    // status icon in the HUD, they show 0" (Liquipedia, Break). A tower's
    // aura is the building's, no ability.
    let mut world = World::new();
    let at = |x| Vec2::from_ints(x, 5000);
    let flag = world.spawn_creep(&FLAGBEARER_CREEP, Team::Radiant, at(5000), 0, 3);
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at(5300));
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, at(8000));
    let hero = hero_at(&mut world, at(8400));
    world.settle();
    world.step();
    let resist = |world: &World| world.stats.get(creep).map(|s| s.magic_resist_bp);
    let shown = |world: &World| {
        world
            .view(Team::Radiant)
            .units
            .iter()
            .find(|unit| unit.id == wire_id(creep))
            .and_then(|unit| {
                unit.effects
                    .iter()
                    .find(|effect| effect.id == EffectId::INSPIRED)
                    .map(|effect| effect.stacks)
            })
    };
    assert_eq!(resist(&world), Some(1200), "inspired at first");
    let broken = 4 * rules::AURA_LINGER_TICKS;
    put_on(&mut world, flag, ModifierKind::Broken, broken);
    put_on(&mut world, tower, ModifierKind::Broken, broken);
    run(&mut world, rules::AURA_LINGER_TICKS + 1);
    assert!(
        carries(&world, creep, INSPIRED_EFFECT),
        "broken, the flagbearer's inspiration stays on"
    );
    assert_eq!(resist(&world), Some(0), "and lends nothing");
    assert_eq!(shown(&world), Some(Some(0)), "shown with a zero");
    assert!(
        carries(&world, hero, GUARDED_EFFECT),
        "a broken tower guards as before"
    );
    run(&mut world, broken);
    assert_eq!(resist(&world), Some(1200), "back once the break lifts");
    assert_eq!(shown(&world), Some(None), "and shown as before");
}

#[test]
fn the_fountain_mends_its_heroes_creeps_and_couriers_but_no_building_and_no_ward() {
    let mut world = World::new();
    let at = |x| Vec2::from_ints(x, 5000);
    world.spawn_unit(&FOUNTAIN, Team::Radiant, at(5000));
    let hero = hero_at(&mut world, at(5300));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at(5000 + 400));
    let courier = world.spawn_unit(&COURIER, Team::Radiant, at(5000 + 500));
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, at(5000 + 800));
    let ward = world.spawn_unit(&OBSERVER_WARD, Team::Radiant, at(5000 - 600));
    world.settle();
    world.step();
    let mended = |unit| carries(&world, unit, FOUNTAIN_EFFECT);
    assert!(
        mended(hero) && mended(creep) && mended(courier),
        "heroes, creeps and couriers"
    );
    assert!(!mended(tower), "no building");
    assert!(!mended(ward), "and no ward");
}

#[test]
fn of_the_auras_only_the_fountains_and_the_towers_reach_the_invulnerable() {
    // Liquipedia: the Inspiration Aura "Does not affect invulnerable nor
    // hidden units", the Rejuvenation Aura "Affects invulnerable, but not
    // hidden units", Tower Protection "Affects invulnerable heroes".
    let mut world = World::new();
    let at = |x| Vec2::from_ints(x, 5000);
    world.spawn_unit(&FLAGBEARER_CREEP, Team::Radiant, at(5000));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at(5300));
    world.spawn_unit(tower_def(1), Team::Radiant, at(8000));
    let guarded = hero_at(&mut world, at(8400));
    world.spawn_unit(&FOUNTAIN, Team::Radiant, at(11_000));
    let mended = hero_at(&mut world, at(11_300));
    for unit in [creep, guarded, mended] {
        put_on(&mut world, unit, ModifierKind::Shielded, 60);
    }
    world.settle();
    world.step();
    assert!(
        world
            .stats
            .get(creep)
            .is_some_and(|stats| stats.invulnerable),
        "the shield makes it invulnerable"
    );
    assert!(
        !carries(&world, creep, INSPIRED_EFFECT),
        "the flagbearer passes the invulnerable creep by"
    );
    assert!(
        carries(&world, guarded, GUARDED_EFFECT),
        "the tower guards the invulnerable hero"
    );
    assert!(
        carries(&world, mended, FOUNTAIN_EFFECT),
        "and the fountain mends one"
    );
}

#[test]
fn of_two_towers_the_stronger_protection_holds() {
    // "Multiple instances of Tower Protection aura do not stack" (Liquipedia,
    // Buildings); the stronger applies. The tier one tower stands later in
    // the slots, so its protection goes on after the tier two's every tick.
    let mut world = World::new();
    world.spawn_unit(tower_def(2), Team::Radiant, Vec2::from_ints(5000, 5000));
    world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(6600, 5000));
    let hero = hero_at(&mut world, Vec2::from_ints(5800, 5000));
    world.settle();
    world.step();
    let held = world.modifiers.get(hero).and_then(|on_it| {
        on_it
            .active()
            .find(|held| matches!(held.kind, ModifierKind::Guarded { .. }))
            .map(|held| held.kind)
    });
    assert_eq!(
        held,
        Some(ModifierKind::Guarded {
            armor: rules::TOWER_AURA_ARMOR[1],
            hp_per_second: rules::TOWER_AURA_REGEN[1],
        }),
        "the tier two tower's five armor and three regeneration"
    );
}

#[test]
fn of_two_flagbearers_the_one_lending_more_magic_resistance_holds() {
    // The one that spawned three intervals in lends 12 %; the fresh one,
    // later in the slots, would lend nothing.
    let mut world = World::new();
    let at = |x| Vec2::from_ints(x, 5000);
    world.spawn_creep(&FLAGBEARER_CREEP, Team::Radiant, at(5000), 0, 3);
    world.spawn_creep(&FLAGBEARER_CREEP, Team::Radiant, at(5600), 0, 0);
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at(5300));
    world.settle();
    world.step();
    assert_eq!(
        world.stats.get(creep).map(|s| s.magic_resist_bp),
        Some(12 * 100),
        "the stronger inspiration"
    );
}

#[test]
fn a_flagbearer_spawned_later_lends_its_creeps_more_magic_resistance() {
    // Three upgrade intervals in, the inspiration is worth 12 %, and it
    // multiplies with what the creep already shrugs off.
    for (steps, pct) in [(0, 0), (3, 12), (40, 60)] {
        let mut world = World::new();
        let flag = world.spawn_creep(
            &FLAGBEARER_CREEP,
            Team::Radiant,
            Vec2::from_ints(5000, 5000),
            0,
            steps,
        );
        let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5300, 5000));
        world.settle();
        world.step();
        assert_eq!(
            world.stats.get(creep).map(|s| s.magic_resist_bp),
            Some(pct * 100),
            "{steps} intervals: the creep beside it"
        );
        let own = rules::FLAGBEARER_MAGIC_RESIST_PCT;
        assert_eq!(
            world.stats.get(flag).map(|s| s.magic_resist_bp),
            Some(10_000 - (100 - own) * (100 - pct)),
            "{steps} intervals: the flagbearer itself, multiplied"
        );
    }
}

#[test]
fn the_fountain_melts_whoever_steps_into_its_reach_and_spares_who_stays_out() {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.picks = vec![Pick {
        slot: SlotId(0),
        team: Team::Dire,
        hero: HeroId::PUDGE,
    }];
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.level.insert(hero, Level(20));
    world.settle();
    world.fill_pools(hero);
    let hold = |world: &mut World, at, ticks| {
        for _ in 0..ticks {
            if let Some(t) = world.transform.get_mut(hero) {
                t.pos = at;
            }
            world.advance_trusted(&[]);
        }
    };
    let full = world.health.get(hero).expect("standing").hp;
    hold(
        &mut world,
        rules::RADIANT_FOUNTAIN_POS + Vec2::from_ints(1400, 0),
        45,
    );
    assert_eq!(
        world.health.get(hero).expect("standing").hp,
        full,
        "out of reach the fountain leaves it be"
    );
    // A hero of the twentieth level is torn apart within seconds. The reading
    // starts once the first missiles have begun to land.
    let inside = rules::RADIANT_FOUNTAIN_POS + Vec2::from_ints(1000, 0);
    hold(&mut world, inside, 30);
    let before = world.health.get(hero).expect("standing").hp.to_int();
    hold(&mut world, inside, 30);
    let after = world.health.get(hero).map_or(0, |h| h.hp.to_int());
    assert!(
        before - after > 1200,
        "a second under the fountain costs over 1200 health, not {}",
        before - after
    );
}

#[test]
fn buckler_armor_is_suppressed_when_vladmir_or_assault_reaches_the_ally() {
    // “Buckler Aura no longer stacks with Vladmir's Aura or Assault Aura”.
    use bota_core::{Stat, item_by_key, item_def};

    let buckler = item_by_key("item_buckler").expect("catalogue");
    let gear = |key| item_by_key(key).expect("catalogue");
    for other in [gear("item_vladmir"), gear("item_assault")] {
        for buckler_first in [false, true] {
            let mut world = World::new();
            let at = Vec2::from_ints(5500, 5500);
            let ally = hero_at(&mut world, at);
            let bearer = world.spawn_hero(
                Team::Radiant,
                at + Vec2::from_ints(120, 0),
                SlotId(1),
                HeroId::SHADOW_FIEND,
            );
            world.settle();
            let base = world.stats.get(ally).unwrap().armor;
            let first = if buckler_first { buckler } else { other };
            let second = if buckler_first { other } else { buckler };
            world.inventory.get_mut(bearer).unwrap().slots[0] = Some(fresh_stack(first, 1));
            world.step();
            world.inventory.get_mut(bearer).unwrap().slots[1] = Some(fresh_stack(second, 1));
            world.step();
            let expected = item_def(other).unwrap().modifiers[0]
                .buff
                .get(Stat::Armor, 1);
            assert_eq!(
                world.stats.get(ally).unwrap().armor,
                base + Fixed::from_int(expected)
            );
        }
    }
}

#[test]
fn headdress_aura_reaches_owned_units_but_passes_over_ordinary_lane_creeps() {
    // “Regeneration Aura now only applies to player controlled units”.
    let mut world = World::new();
    let at = Vec2::from_ints(5500, 5500);
    let hero = hero_at(&mut world, at);
    let lane = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at + Vec2::from_ints(140, 0));
    let controlled = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at + Vec2::from_ints(-140, 0));
    world.owner.insert(controlled, SlotId(0));
    let item = bota_core::item_by_key("item_headdress").expect("catalogue");
    let effect = bota_core::item_def(item).unwrap().modifiers[0].effect;
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(fresh_stack(item, 0));
    world.settle();
    world.step();
    let wears = |unit| {
        world.modifiers.get(unit).is_some_and(|modifiers| {
            modifiers
                .active()
                .any(|held| held.kind.effect_id() == effect)
        })
    };
    assert!(wears(hero));
    assert!(wears(controlled));
    assert!(!wears(lane));
}

#[test]
fn pipe_barrier_blocks_magic_until_a_dispel_of_buffs() {
    // “Barrier is now dispellable”.
    use crate::game::{Dispel, DispelPower, wire_id};
    use bota_proto::{DamageKind, EventKind};

    for power in [DispelPower::Basic, DispelPower::Strong] {
        let mut world = World::new();
        let hero = hero_at(&mut world, Vec2::from_ints(5500, 5500));
        let item = bota_core::item_by_key("item_pipe").expect("catalogue");
        hand_item(&mut world, hero, item, 0);
        world.settle();
        assert!(world.use_item(hero, 0, bota_proto::Target::None, &mut Vec::new()));
        let magic_hit = |world: &mut World| {
            world.push_hit(None, hero, 100, DamageKind::Magical, None);
            world
                .step()
                .iter()
                .filter_map(|event| match event.kind() {
                    EventKind::Damaged { target, amount, .. } if target == wire_id(hero) => {
                        Some(amount)
                    }
                    _ => None,
                })
                .sum::<i32>()
        };
        assert_eq!(magic_hit(&mut world), 0, "the barrier absorbs magic");
        world.dispel(hero, Dispel::Debuffs, power);
        assert_eq!(
            magic_hit(&mut world),
            0,
            "dispelling debuffs preserves the barrier"
        );
        world.dispel(hero, Dispel::Buffs, power);
        assert!(
            magic_hit(&mut world) > 0,
            "magic reaches the hero after a buff dispel"
        );
    }
}
