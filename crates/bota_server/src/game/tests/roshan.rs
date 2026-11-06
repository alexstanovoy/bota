//! Roshan through the match and order paths.
use super::support::*;
use crate::game::World;
use bota_proto::{MapId, ObjectiveKind, ObjectiveSet};

#[test]
fn roshan_stands_at_match_start_only_when_the_map_and_objectives_allow_him() {
    for (map, enabled, expected) in [(0, true, 1), (2, true, 1), (1, true, 0), (0, false, 0)] {
        let mut cfg = config();
        cfg.map = MapId(map);
        cfg.objectives = if enabled {
            ObjectiveSet::of(ObjectiveKind::Roshan)
        } else {
            ObjectiveSet::NONE
        };
        if map == 1 {
            assert!(cfg.validate().is_err());
            continue;
        }
        let world = World::for_match(&cfg, cfg.rng());
        let count = world
            .entities
            .iter()
            .filter(|unit| {
                world
                    .neutral
                    .get(*unit)
                    .is_some_and(|kind| kind.neutral().unit == "npc_dota_roshan")
            })
            .count();
        assert_eq!(count, expected, "map {map}, enabled {enabled}");
    }
}

fn match_with_roshan() -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    world
}

fn boss(world: &World) -> crate::game::Entity {
    world.lairs[0].unit.expect("Roshan standing")
}

#[test]
fn roshan_waits_until_fifteen_minutes_and_walks_between_pits_on_natural_transitions() {
    use bota_core::{ROSHAN_DAY_PIT, ROSHAN_NIGHT_PIT, rules};
    let mut world = match_with_roshan();
    let it = boss(&world);
    assert_eq!(world.pos(it), Some(ROSHAN_DAY_PIT));
    world.clock_shift.game = world.schedule().horn + 7 * 60 * rules::TICKS_PER_SECOND;
    world.step();
    assert_eq!(world.camp_home.get(it).expect("home").home, ROSHAN_DAY_PIT);
    world.clock_shift.game =
        world.schedule().horn + 15 * 60 * rules::TICKS_PER_SECOND - world.tick - 1;
    world.step();
    assert_eq!(
        world.camp_home.get(it).expect("new home").home,
        ROSHAN_NIGHT_PIT
    );
    assert!(
        world
            .pos(it)
            .expect("walking")
            .within(ROSHAN_DAY_PIT, rules::units(100)),
        "does not teleport"
    );
    for _ in 0..90 * rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        world
            .pos(it)
            .expect("arrived")
            .within(ROSHAN_NIGHT_PIT, rules::units(100))
    );
    world.clock_shift.game =
        world.schedule().horn + 20 * 60 * rules::TICKS_PER_SECOND - world.tick - 1;
    world.step();
    assert_eq!(
        world.camp_home.get(it).expect("day home").home,
        ROSHAN_DAY_PIT
    );
}

#[test]
fn roshan_respawns_within_eight_to_eleven_minutes_in_the_current_pit() {
    use bota_core::{ROSHAN_NIGHT_PIT, rules};
    use bota_proto::DamageKind;
    let mut waits = Vec::new();
    for stream in 0..4 {
        let mut world = match_with_roshan();
        world.rng = crate::game::MatchRng::new(&[3; 32], stream);
        let it = boss(&world);
        let hero = world.seats[0].unit.expect("hero");
        world.push_hit(Some(hero), it, 100_000, DamageKind::Pure, None);
        world.step();
        assert!(world.lairs[0].unit.is_none());
        let at = world.lairs[0].respawn_at;
        let delay = at - world.clock();
        assert!(
            (8 * 60 * rules::TICKS_PER_SECOND..=11 * 60 * rules::TICKS_PER_SECOND).contains(&delay)
        );
        waits.push(delay);
        world.clock_shift.game = at - world.tick - 2;
        world.step();
        assert!(world.lairs[0].unit.is_none());
        world.step();
        assert!(world.lairs[0].unit.is_some());
        // A later death at 15:00 returns during the night of 25:00-30:00.
        world.clock_shift.game =
            world.schedule().horn + 16 * 60 * rules::TICKS_PER_SECOND - world.tick;
        let it = boss(&world);
        world.push_hit(Some(hero), it, 100_000, DamageKind::Pure, None);
        world.step();
        let at = world.lairs[0].respawn_at;
        world.clock_shift.game = at - world.tick - 1;
        world.step();
        let expected = if bota_core::time_of_day(world.clock(), world.schedule()).night {
            ROSHAN_NIGHT_PIT
        } else {
            bota_core::ROSHAN_DAY_PIT
        };
        assert_eq!(world.pos(boss(&world)), Some(expected));
    }
    assert!(waits.windows(2).any(|w| w[0] != w[1]));
}

#[test]
fn roshan_can_be_attacked_from_inside_either_pit_and_ignores_outside_attacks() {
    use crate::game::{Command, wire_id};
    use bota_core::rules;
    use bota_proto::{Fixed, Order, Target, Team, Vec2};
    for inside in [false, true] {
        let mut world = match_with_roshan();
        let it = boss(&world);
        let at = world.pos(it).expect("boss");
        let hero = world.seats[0].unit.expect("hero");
        // A ranged hero has reach in both cases; a ward grants sight inside.
        let spot = at + Vec2::from_ints(if inside { 200 } else { -450 }, 0);
        assert_eq!(
            world.map.boss_zones.iter().any(|zone| zone.contains(spot)),
            inside
        );
        world.transform.get_mut(hero).expect("hero position").pos = spot;
        world.spawn_unit(
            &bota_core::OBSERVER_WARD,
            Team::Radiant,
            at + Vec2::from_ints(0, 50),
        );
        world.settle();
        let order = Order::Attack {
            target: Target::Unit(wire_id(it)),
        };
        assert_eq!(
            world.validate_order(world.seats[0].slot, None, &order),
            Ok(())
        );
        world.advance(&[Command {
            slot: world.seats[0].slot,
            unit: None,
            order,
            seq: 0,
            queue: false,
        }]);
        let mut blows = 0;
        for _ in 0..4 * rules::TICKS_PER_SECOND {
            blows += world.step().iter().filter(|event| matches!(event.kind(),bota_proto::EventKind::Damaged { target, inflictor:None, .. } if target==wire_id(it))).count();
        }
        assert_eq!(blows > 0, inside);
        assert!(world.health.get(it).expect("alive").hp > Fixed::ZERO);
    }
}

#[test]
fn flying_vision_cannot_cross_a_pit_boundary_in_either_direction() {
    use bota_core::{RANGED_CREEP, ROSHAN_DAY_PIT, UnitDef, rules};
    use bota_proto::{Team, Vec2};
    const FLYER: UnitDef = UnitDef {
        flies: true,
        day_vision: 2000,
        night_vision: 2000,
        ..RANGED_CREEP
    };
    for (from, to) in [
        (ROSHAN_DAY_PIT + Vec2::from_ints(-700, 0), ROSHAN_DAY_PIT),
        (ROSHAN_DAY_PIT, ROSHAN_DAY_PIT + Vec2::from_ints(-700, 0)),
    ] {
        let mut world = match_with_roshan();
        let eye = world.spawn_unit(&FLYER, Team::Radiant, from);
        let mark = world.spawn_unit(&RANGED_CREEP, Team::Dire, to);
        world.settle();
        assert!(
            !world.can_see(Team::Radiant, mark),
            "flying viewer {eye:?} across the boundary"
        );
        world.transform.get_mut(eye).expect("eye").pos = to + Vec2::from_ints(0, 50);
        world.settle();
        assert!(
            world.can_see(Team::Radiant, mark),
            "within the same zone, in range {}",
            rules::TICKS_PER_SECOND
        );
    }
}

#[test]
fn hurting_a_camp_creep_does_not_wake_an_independent_boss() {
    use bota_core::{NeutralKind, ROSHAN_DAY_PIT};
    use bota_proto::{DamageKind, Vec2};
    let mut world = match_with_roshan();
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(1000, 0);
    let creep = world.spawn_neutral(
        NeutralKind::Kobold,
        ROSHAN_DAY_PIT + Vec2::from_ints(200, 0),
        0,
        0,
    );
    world.settle();
    world.push_hit(Some(hero), creep, 1, DamageKind::Pure, None);
    world.step();
    assert!(!world.neutral_ai.get(boss(&world)).expect("boss mind").awake);
    assert!(world.neutral_ai.get(creep).expect("camp mind").awake);
}

#[test]
fn a_blow_does_not_interrupt_roshans_walk_to_the_other_pit() {
    use bota_core::{ROSHAN_DAY_PIT, rules};
    use bota_proto::{DamageKind, Vec2};
    let mut world = match_with_roshan();
    let it = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(500, 0);
    world.clock_shift.game = world.schedule().horn + 15 * 60 * rules::TICKS_PER_SECOND - 1;
    world.step();
    world.push_hit(Some(hero), it, 1, DamageKind::Pure, None);
    world.step();
    let ai = world.neutral_ai.get(it).expect("neutral mind");
    assert!(ai.going_home);
    assert!(!ai.awake);
}

#[test]
fn roshan_wakes_at_night_and_returns_home_after_being_led_beyond_his_leash() {
    use bota_core::{ROSHAN_DAY_PIT, rules};
    use bota_proto::Vec2;
    let mut world = match_with_roshan();
    world.immortal_heroes = true;
    let it = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(90, 0);
    world.clock_shift.game = world.schedule().horn + 7 * 60 * rules::TICKS_PER_SECOND;
    world.settle();
    world.step();
    assert!(
        world.neutral_ai.get(it).expect("mind").awake,
        "does not sleep by night"
    );
    world.step();
    assert_eq!(world.target_of(it), Some(hero));
    world.transform.get_mut(hero).expect("hero position").pos = world.map.fountains[0];
    world.transform.get_mut(it).expect("boss position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(1000, 0);
    world.target.remove(it);
    world.set_order(it, crate::game::UnitOrder::Idle);
    world.settle();
    run(&mut world, bota_core::ROSHAN_HOME.window + 1);
    assert!(world.neutral_ai.get(it).expect("leashed").going_home);
    run(&mut world, 15 * rules::TICKS_PER_SECOND);
    assert!(
        world
            .pos(it)
            .expect("home")
            .within(ROSHAN_DAY_PIT, rules::units(100))
    );
}

#[test]
fn roshan_grows_health_damage_experience_from_one_minute_and_aura_armor_from_the_horn() {
    use bota_core::rules;
    use bota_proto::Fixed;
    let mut world = match_with_roshan();
    let boss = boss(&world);
    assert_eq!(
        world.stats.get(boss).expect("boss").max_hp,
        Fixed::from_int(6000)
    );
    for (seconds, hp, armor_tenths, damage, xp) in [
        (0, 6000, 304, 75, 400),
        (59, 6000, 304, 75, 400),
        (60, 6130, 308, 82, 420),
        (600, 7300, 344, 145, 600),
    ] {
        world.clock_shift.game =
            world.schedule().horn + seconds * rules::TICKS_PER_SECOND - world.tick - 1;
        world.step();
        let stats = world.stats.get(boss).expect("boss");
        assert_eq!(stats.max_hp, Fixed::from_int(hp), "at {seconds}s");
        assert_eq!(
            world.health.get(boss).expect("living boss").hp,
            Fixed::from_int(hp),
            "live current health updates at {seconds}s"
        );
        assert_eq!(
            stats.armor,
            Fixed::from_ratio(armor_tenths, 10),
            "at {seconds}s"
        );
        assert_eq!(stats.damage, damage, "at {seconds}s");
        assert_eq!(
            world.bounty.get(boss).expect("bounty").xp,
            xp,
            "at {seconds}s"
        );
    }
}

#[test]
fn roshans_clock_keeps_growing_while_he_is_dead_and_respawns_at_current_strength() {
    use bota_core::rules;
    use bota_proto::{DamageKind, Fixed};
    let mut world = match_with_roshan();
    let old = boss(&world);
    world.push_hit(world.seats[0].unit, old, 100_000, DamageKind::Pure, None);
    world.step();
    assert!(world.lairs[0].unit.is_none());
    world.clock_shift.game =
        world.schedule().horn + 12 * 60 * rules::TICKS_PER_SECOND - world.tick - 1;
    world.step();
    world.step(); // The aura heartbeat reaches the newly stood-up body.
    let new = boss(&world);
    assert_ne!(new, old);
    let stats = world.stats.get(new).expect("boss");
    assert_eq!(stats.max_hp, Fixed::from_int(7560));
    assert_eq!(stats.damage, 159);
    assert_eq!(stats.armor, Fixed::from_ratio(352, 10));
    assert_eq!(world.bounty.get(new).expect("bounty").xp, 640);
    assert_eq!(world.health.get(new).expect("alive").hp, stats.max_hp);
}

#[test]
fn devotion_only_spreads_its_fractional_armor_to_close_vulnerable_allies_and_lingers() {
    use crate::game::Modifier;
    use bota_core::{ModifierKind, NeutralKind, rules};
    use bota_proto::{Fixed, Team, Vec2};
    let mut world = match_with_roshan();
    let roshan = boss(&world);
    let at = world.pos(roshan).expect("boss");
    world.clock_shift.game = world.schedule().horn - world.tick - 1;
    world.step();
    let ally = world.spawn_neutral(NeutralKind::Kobold, at + Vec2::from_ints(1, 0), 0, 0);
    let shielded = world.spawn_neutral(NeutralKind::Kobold, at + Vec2::from_ints(0, 1), 0, 0);
    world.put_modifier(
        shielded,
        Modifier::new(ModifierKind::Shielded, None, None, Some(100)),
    );
    let enemy = world.spawn_unit(NeutralKind::Kobold.def(), Team::Radiant, at);
    world.settle();
    world.tick_auras();
    world.settle();
    assert_eq!(
        world.stats.get(ally).expect("ally").armor,
        Fixed::from_ratio(4, 10)
    );
    assert_eq!(
        world.stats.get(shielded).expect("invulnerable").armor,
        Fixed::ZERO
    );
    assert_eq!(world.stats.get(enemy).expect("enemy").armor, Fixed::ZERO);
    world.transform.get_mut(ally).expect("ally").pos = at + Vec2::from_ints(500, 0);
    run(&mut world, rules::TICKS_PER_SECOND / 2 + 1);
    assert_eq!(world.stats.get(ally).expect("away").armor, Fixed::ZERO);
}

#[test]
fn roshans_npc_level_attack_speed_and_bounty_are_the_boss_rows() {
    use bota_proto::{DamageKind, Fixed, Vec2};
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    cfg.picks.push(bota_proto::Pick {
        slot: bota_proto::SlotId(1),
        team: bota_proto::Team::Dire,
        hero: bota_proto::HeroId::PUDGE,
    });
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    let roshan = boss(&world);
    let at = world.pos(roshan).expect("boss");
    assert_eq!(world.level.get(roshan).expect("NPC level").0, 30);
    assert_eq!(world.stats.get(roshan).expect("boss").attack_speed, 200);
    for (i, seat) in world.seats.clone().into_iter().enumerate() {
        if let Some(hero) = seat.unit {
            world.transform.get_mut(hero).expect("hero").pos =
                at + Vec2::from_ints(100 + i as i32 * 100, 0);
        }
    }
    world.settle();
    let before: Vec<_> = world.seats.iter().map(|s| s.xp).collect();
    let killer = world.seats[0].unit;
    world.push_hit(killer, roshan, 100_000, DamageKind::Pure, None);
    world.step();
    assert_eq!(world.seats[0].xp - before[0], 400);
    assert_eq!(world.seats[1].xp - before[1], 0);
    assert!(world.health.get(killer.expect("hero")).expect("alive").hp > Fixed::ZERO);
}

#[test]
fn roshans_status_resistance_and_slow_resistance_reduce_different_parts_of_a_slow() {
    use crate::game::Modifier;
    use bota_core::ModifierKind;
    use bota_proto::Fixed;
    let mut world = match_with_roshan();
    let boss = boss(&world);
    assert_eq!(world.stats.get(boss).expect("boss").status_resist_bp, 2500);
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Slowed { pct: 50 }, None, None, Some(100)),
    );
    world.settle();
    let slow = world
        .modifiers
        .get(boss)
        .expect("mods")
        .active()
        .find(|m| matches!(m.kind, ModifierKind::Slowed { .. }))
        .expect("slow");
    assert_eq!(slow.ticks_left, Some(75));
    assert_eq!(
        world.stats.get(boss).expect("slow strength").move_speed,
        Fixed::from_ratio(675, 4)
    );
}

#[test]
fn spell_block_spends_dismembers_mana_and_cooldown_without_starting_a_channel() {
    use crate::game::Modifier;
    use bota_core::ModifierKind;
    use bota_proto::{AbilitySlot, EventKind, Fixed, HeroId, Order, Target};
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    cfg.picks[0].hero = HeroId::PUDGE;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    let hero = world.seats[0].unit.expect("Pudge");
    let boss = boss(&world);
    world.transform.get_mut(hero).expect("hero").pos =
        world.pos(boss).expect("boss") + bota_proto::Vec2::from_ints(100, 0);
    set_level(&mut world, hero, 3, 1);
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Stunned, None, None, Some(1000)),
    );
    world.settle();
    let mana = world.mana.get(hero).expect("mana").mana;
    seat_order(
        &mut world,
        Order::Cast {
            slot: AbilitySlot(3),
            target: Target::Unit(crate::game::wire_id(boss)),
        },
    );
    let mut events = Vec::new();
    for _ in 0..30 {
        events.extend(world.step());
    }
    assert!(world.mana.get(hero).expect("paid").mana < mana - Fixed::from_int(90));
    assert!(world.abilities.get(hero).expect("book").slots[3].cooldown > 0);
    assert!(!world.wears(boss, ModifierKind::Dismembered { level: 0 }));
    assert!(!world.is_casting(hero));
    assert!(events.iter().any(|e| matches!(
        e.kind(),
        EventKind::AbilityCast {
            ability: bota_proto::AbilityId::DISMEMBER,
            ..
        }
    )));
}

fn roshan_spell_block_slot(world: &World, boss: crate::game::Entity) -> usize {
    world
        .abilities
        .get(boss)
        .expect("book")
        .slots
        .iter()
        .position(|s| s.id == bota_core::NeutralAbility::SpellBlock.id())
        .expect("spell block")
}

#[test]
fn spell_block_absorbs_one_of_simultaneous_zero_damage_bolts_at_impact_even_if_the_caster_died() {
    use crate::game::Modifier;
    use bota_core::{ModifierKind, NeutralAbility, NeutralKind};
    use bota_proto::{DamageKind, EventKind, Target, Team, Vec2};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let caster = world.spawn_unit(
        NeutralKind::Kobold.def(),
        Team::Radiant,
        at + Vec2::from_ints(400, 0),
    );
    assert!(world.cast_spell(
        caster,
        NeutralAbility::Ensnare,
        1,
        Target::Unit(crate::game::wire_id(boss))
    ));
    assert!(world.cast_spell(
        caster,
        NeutralAbility::Ensnare,
        1,
        Target::Unit(crate::game::wire_id(boss))
    ));
    world.despawn(caster);
    world.settle();
    let mut activations = 0;
    for _ in 0..20 {
        activations+=world.step().iter().filter(|event|matches!(event.kind(),EventKind::AbilityCast {ability,..} if ability==NeutralAbility::SpellBlock.id())).count();
    }
    assert_eq!(activations, 1);
    assert!(
        world.wears(boss, ModifierKind::Rooted),
        "the other bolt still roots"
    );
    assert!(
        world.abilities.get(boss).expect("book").slots[roshan_spell_block_slot(&world, boss)]
            .cooldown
            > 400
    );
    // A non-targeted spell hit does not consume another ready block.
    let slot = roshan_spell_block_slot(&world, boss);
    world.abilities.get_mut(boss).expect("book").slots[slot].cooldown = 0;
    world.push_hit(
        None,
        boss,
        100,
        DamageKind::Pure,
        Some(bota_proto::Inflictor::Ability(
            bota_proto::AbilityId::RAZE_NEAR,
        )),
    );
    world.step();
    assert_eq!(
        world.abilities.get(boss).expect("book").slots[slot].cooldown,
        0
    );
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Broken, None, None, Some(100)),
    );
    world.settle();
    assert_eq!(
        world
            .stats
            .get(boss)
            .expect("unbreakable resistance")
            .status_resist_bp,
        2500
    );
}

#[test]
fn illusion_attack_reduction_does_not_reduce_spell_damage() {
    use crate::game::Modifier;
    use bota_core::{ModifierKind, Striker};
    use bota_proto::{AbilityId, DamageKind, Inflictor};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let hero = world.seats[0].unit;
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Stunned, None, None, Some(1000)),
    );
    world.push_hit(hero, boss, 100, DamageKind::Pure, None);
    let hit = world.hits.back_mut().expect("attack");
    hit.attack = true;
    hit.striker = Some(Striker::Illusion);
    let attack = world
        .step()
        .into_iter()
        .find_map(|e| match e.kind() {
            bota_proto::EventKind::Damaged { target, amount, .. }
                if target == crate::game::wire_id(boss) =>
            {
                Some(amount)
            }
            _ => None,
        })
        .expect("attack");
    assert_eq!(attack, 20);
    world.push_hit(
        hero,
        boss,
        100,
        DamageKind::Pure,
        Some(Inflictor::Ability(AbilityId::RAZE_NEAR)),
    );
    let spell = world
        .step()
        .into_iter()
        .find_map(|e| match e.kind() {
            bota_proto::EventKind::Damaged { target, amount, .. }
                if target == crate::game::wire_id(boss) =>
            {
                Some(amount)
            }
            _ => None,
        })
        .expect("spell");
    assert_eq!(spell, 100);
}

#[test]
fn roshan_keeps_its_phased_body_when_an_enemy_stands_in_the_same_spot() {
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    let at = world.pos(boss).expect("boss");
    world.transform.get_mut(hero).expect("hero").pos = at;
    world.settle();
    world.push_apart();
    assert!(world.stats.get(boss).expect("phased").phased);
    assert_eq!(world.pos(boss), Some(at));
    assert_eq!(world.pos(hero), Some(at));
}

#[test]
fn lifesteal_from_roshan_uses_the_creep_factor_on_damage_actually_dealt() {
    use crate::game::{Modifier, UnitOrder};
    use bota_core::{MELEE_CREEP, ModifierKind, NeutralAbility};
    use bota_proto::{EventKind, Fixed, Team, Vec2};
    const ATTACKER: bota_core::UnitDef = bota_core::UnitDef {
        kind: bota_proto::UnitKind::Hero,
        max_hp: 1000,
        damage: 1000,
        hp_regen: Fixed::ZERO,
        ..MELEE_CREEP
    };
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let attacker = world.spawn_unit(&ATTACKER, Team::Radiant, at - Vec2::from_ints(100, 0));
    world.put_modifier(
        attacker,
        Modifier::new(
            ModifierKind::Spell {
                ability: NeutralAbility::ProwlerAura,
                level: 1,
                amount: 0,
            },
            Some(attacker),
            None,
            None,
        ),
    );
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Stunned, None, None, Some(1000)),
    );
    world.settle();
    world.health.get_mut(attacker).expect("attacker").hp = Fixed::from_int(50);
    world.orders.get_mut(attacker).expect("orders").current = UnitOrder::Attack {
        target: boss,
        last_seen: at,
    };
    for _ in 0..90 {
        let before = world.health.get(attacker).expect("attacker").hp;
        let regen = world.stats.get(attacker).expect("stats").hp_regen;
        let pct = world.stats.get(attacker).expect("lifesteal").lifesteal_pct;
        let events = world.step();
        if let Some(damage) = events.into_iter().find_map(|e| match e.kind() {
            EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                ..
            } if source == crate::game::wire_id(attacker)
                && target == crate::game::wire_id(boss) =>
            {
                Some(amount)
            }
            _ => None,
        }) {
            assert!(pct > 0);
            let healed = world.health.get(attacker).expect("attacker").hp - before - regen;
            assert_eq!(healed, Fixed::from_int(damage * pct * 60 / 10000));
            return;
        }
    }
    panic!("the normal attack did not land");
}

#[test]
fn roshan_can_target_a_courier_and_his_melee_attack_counts_as_a_heros() {
    use bota_core::COURIER;
    use bota_proto::{Team, Vec2};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let courier = world.spawn_unit(&COURIER, Team::Radiant, at + Vec2::from_ints(100, 0));
    world.settle();
    assert!(world.hostile_unseen(boss, courier));
    let mut hits = 0;
    for _ in 0..90 {
        hits+=world.step().iter().filter(|e|matches!(e.kind(),bota_proto::EventKind::Damaged {target,..} if target==crate::game::wire_id(courier))).count();
        if !world.alive(courier) {
            assert_eq!(hits, 1);
            return;
        }
    }
    panic!("Roshan did not bring down the courier");
}

#[test]
fn an_illusion_arrow_keeps_its_attack_classification_after_the_illusion_is_gone() {
    use crate::game::{Modifier, UnitOrder};
    use bota_core::{ModifierKind, Striker, mitigate, rules};
    use bota_proto::{EventKind, Vec2};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    let at = world.pos(boss).expect("boss");
    world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(300, 0);
    world.make_rune_illusions(hero);
    let illusions: Vec<_> = world
        .entities
        .iter()
        .filter(|e| world.illusion.get(*e).is_some())
        .collect();
    let illusion = illusions[0];
    for other in illusions {
        world.set_order(other, UnitOrder::Stand);
    }
    world.set_order(
        illusion,
        UnitOrder::Attack {
            target: boss,
            last_seen: at,
        },
    );
    world.transform.get_mut(hero).expect("hero").pos = world.map.fountains[0];
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Stunned, None, None, Some(1000)),
    );
    world.settle();
    let mut damage = None;
    for _ in 0..90 {
        world.step();
        if let Some(shot) = world
            .entities
            .iter()
            .filter_map(|e| world.projectile.get(e))
            .find(|p| p.source == Some(illusion))
        {
            assert_eq!(shot.striker, Some(Striker::Illusion));
            let stats = world.stats.get(boss).expect("boss");
            damage = Some(
                mitigate(
                    bota_core::amplify(shot.damage, shot.damage_amp_bp),
                    shot.kind,
                    stats.armor,
                    stats.magic_resist_bp,
                ) * 20
                    / 100,
            );
            break;
        }
    }
    let expected = damage.expect("a normal ranged attack was launched");
    world.despawn(illusion);
    for _ in 0..60 {
        if let Some(amount) = world.step().into_iter().find_map(|event| match event.kind() {
            EventKind::Damaged { target, amount, .. } if target == crate::game::wire_id(boss) => {
                Some(amount)
            }
            _ => None,
        }) {
            assert_eq!(amount, expected);
            assert!(
                world
                    .stats
                    .get(boss)
                    .expect("resistance")
                    .illusion_attack_taken_bp
                    < rules::NOMINAL_BP
            );
            return;
        }
    }
    panic!("the in-flight arrow did not land");
}

#[test]
fn a_spell_bolt_arrives_as_a_spell_instead_of_an_attack() {
    use bota_core::{NeutralAbility, NeutralKind};
    use bota_proto::{Target, Team, Vec2};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let caster = world.spawn_unit(
        NeutralKind::Kobold.def(),
        Team::Radiant,
        at + Vec2::from_ints(100, 0),
    );
    assert!(world.cast_spell(
        caster,
        NeutralAbility::HurlBoulder,
        1,
        Target::Unit(crate::game::wire_id(boss))
    ));
    for _ in 0..20 {
        crate::game::missile_system(crate::game::MissileCx {
            entities: &mut world.entities,
            projectile: &mut world.projectile,
            transform: &mut world.transform,
            team: &mut world.team,
            visibility: &mut world.visibility,
            health: &world.health,
            kind: &world.kind,
            ground: &world.ground,
            hits: &mut world.hits,
        });
        if let Some(hit) = world.hits.front() {
            assert!(!hit.attack);
            assert!(hit.striker.is_none());
            assert_eq!(hit.blockable_source_team, Some(Team::Radiant));
            return;
        }
    }
    panic!("the bolt did not arrive");
}

#[test]
fn spell_block_ignores_invalid_targets_and_becomes_ready_after_fifteen_seconds() {
    use crate::game::Modifier;
    use bota_core::ModifierKind;
    use bota_proto::{AbilityId, Fixed, HeroId, Target, Team, Vec2};
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    cfg.picks[0].hero = HeroId::PUDGE;
    cfg.immortal_heroes = true;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    let hero = world.seats[0].unit.expect("hero");
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
    world.settle();
    let target = Target::Unit(crate::game::wire_id(boss));
    let slot = roshan_spell_block_slot(&world, boss);
    world.set_team(boss, Team::Radiant);
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        None
    );
    world.set_team(boss, Team::Neutral);
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Shielded, None, None, Some(100)),
    );
    world.settle();
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        None
    );
    world.take_modifier(boss, ModifierKind::Shielded, None);
    world.settle();
    let hp = world.health.get(boss).expect("boss").hp;
    world.health.get_mut(boss).expect("boss").hp = Fixed::ZERO;
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        None
    );
    world.health.get_mut(boss).expect("boss").hp = hp;
    let caster_hp = world.health.get(hero).expect("hero").hp;
    world.health.get_mut(hero).expect("hero").hp = Fixed::ZERO;
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        None
    );
    world.health.get_mut(hero).expect("hero").hp = caster_hp;
    assert_eq!(
        world.abilities.get(boss).expect("book").slots[slot].cooldown,
        0
    );
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        Some(true)
    );
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        Some(false)
    );
    run(&mut world, 450);
    assert_eq!(
        world.block_cast_target(hero, AbilityId::DISMEMBER, target),
        Some(true)
    );
}

#[test]
fn dismember_refuses_without_spending_when_pit_sight_disappears_before_its_cast_point() {
    use bota_core::{ModifierKind, OBSERVER_WARD};
    use bota_proto::{AbilitySlot, HeroId, Target, Team, Vec2};
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    cfg.picks[0].hero = HeroId::PUDGE;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    let hero = world.seats[0].unit.expect("hero");
    let boss = boss(&world);
    let home = world.pos(boss).expect("boss");
    let in_pit = |at| world.map.boss_zones.iter().any(|zone| zone.contains(at));
    let mut pair = None;
    'search: for x in (-400..=500).step_by(50) {
        for y in (-650..=300).step_by(50) {
            let inside = home + Vec2::from_ints(x, y);
            if !in_pit(inside) || !world.clearance.stands_clear(inside) {
                continue;
            }
            for offset in [(150, 0), (-150, 0), (0, 150), (0, -150)] {
                let outside = inside + Vec2::from_ints(offset.0, offset.1);
                if !in_pit(outside) && world.clearance.stands_clear(outside) {
                    pair = Some((inside, outside));
                    break 'search;
                }
            }
        }
    }
    let (inside, outside) = pair.expect("walkable locations on opposite sides of the pit boundary");
    world.transform.get_mut(boss).expect("boss").pos = inside;
    world.transform.get_mut(hero).expect("hero").pos = outside;
    // An ability's temporary vision source can reveal the pit from within it.
    const VISION: bota_core::UnitDef = bota_core::UnitDef {
        hides: false,
        invulnerable: true,
        ..OBSERVER_WARD
    };
    let vision = world.spawn_unit(&VISION, Team::Radiant, inside);
    set_level(&mut world, hero, 3, 1);
    world.settle();
    assert!(world.hostile(hero, boss));
    world.despawn(vision);
    world.settle();
    assert!(!world.hostile(hero, boss));
    let mana = world.mana.get(hero).expect("mana").mana;
    assert!(!world.begin_ability(
        hero,
        AbilitySlot(3),
        Target::Unit(crate::game::wire_id(boss))
    ));
    assert_eq!(world.mana.get(hero).expect("mana").mana, mana);
    assert_eq!(
        world.abilities.get(hero).expect("book").slots[3].cooldown,
        0
    );
    assert_eq!(
        world.abilities.get(boss).expect("block").slots[roshan_spell_block_slot(&world, boss)]
            .cooldown,
        0
    );
    assert!(!world.wears(boss, ModifierKind::Dismembered { level: 0 }));
}

#[test]
fn roshan_slams_a_crowd_with_a_hero_and_deals_double_damage_to_creeps() {
    use crate::game::UnitOrder;
    use bota_core::{MELEE_CREEP, ability_def};
    use bota_proto::{DamageKind, EventKind, Team, Vec2};
    const CREEP: bota_core::UnitDef = bota_core::UnitDef {
        max_hp: 5000,
        damage: 0,
        hp_regen: bota_proto::Fixed::ZERO,
        move_speed: 300,
        ..MELEE_CREEP
    };
    let mut world = match_with_roshan();
    world.immortal_heroes = true;
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
    world.set_order(hero, UnitOrder::Stand);
    let a = world.spawn_unit(&CREEP, Team::Radiant, at + Vec2::from_ints(0, 160));
    let b = world.spawn_unit(&CREEP, Team::Radiant, at + Vec2::from_ints(0, -160));
    world.set_order(a, UnitOrder::Stand);
    world.set_order(b, UnitOrder::Stand);
    world.settle();
    let mut casts = 0;
    let mut hits = Vec::new();
    for _ in 0..90 {
        for event in world.step() {
            match event.kind {
                EventKind::AbilityCast { ability, .. }
                    if ability_def(ability).is_some_and(|d| d.key == "roshan_slam") =>
                {
                    casts += 1
                }
                EventKind::Damaged {
                    target,
                    amount,
                    kind: DamageKind::Magical,
                    ..
                } if target == crate::game::wire_id(a) || target == crate::game::wire_id(b) => {
                    hits.push(amount)
                }
                _ => {}
            }
        }
    }
    assert_eq!(casts, 1);
    assert_eq!(hits, vec![140, 140]);
    for target in [hero, a, b] {
        let slow = world
            .modifiers
            .get(target)
            .expect("effects")
            .active()
            .find(|m| m.kind.effect_id() == bota_proto::EffectId(83))
            .expect("Slam slows nearby units");
        assert_eq!(
            slow.ticks_total,
            Some(120),
            "creeps and heroes have the same duration in 7.40+"
        );
    }
    assert_eq!(
        world.stats.get(a).expect("creep stats").move_speed,
        bota_proto::Fixed::from_int(150)
    );
    assert_eq!(world.stats.get(a).expect("creep stats").attack_speed, 50);
    run(&mut world, 120);
    for target in [hero, a, b] {
        assert!(
            !world
                .modifiers
                .get(target)
                .expect("effects")
                .active()
                .any(|m| m.kind.effect_id() == bota_proto::EffectId(83))
        );
    }
}

#[test]
fn roshans_bash_is_a_conditional_physical_attack_bonus_blocked_once_and_disabled_by_break() {
    use crate::game::Modifier;
    use bota_core::{HERO, ModifierKind, ability_def, effect_def};
    use bota_proto::{DamageKind, EventKind, Team, Vec2};
    const TARGET: bota_core::UnitDef = bota_core::UnitDef {
        attributes: bota_proto::Attributes::ZERO,
        primary: None,
        max_hp: 10000,
        armor: 0,
        magic_resist_pct: 100,
        damage: 0,
        hp_regen: bota_proto::Fixed::ZERO,
        move_speed: 0,
        ..HERO
    };
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    assert!(
        world
            .abilities
            .get(boss)
            .expect("book")
            .slots
            .iter()
            .any(|s| ability_def(s.id).is_some_and(|d| d.key == "roshan_bash"))
    );
    let target = world.spawn_unit(&TARGET, Team::Radiant, at + Vec2::from_ints(100, 0));
    world.put_modifier(
        target,
        Modifier::new(
            ModifierKind::MeatShield { amount: 10 },
            None,
            None,
            Some(5000),
        ),
    );
    world.settle();
    let mut plain = 0;
    let mut bashes = 0;
    for _ in 0..1200 {
        for event in world.step() {
            if let EventKind::Damaged {
                target: hit,
                amount,
                kind,
                ..
            } = event.kind
                && hit == crate::game::wire_id(target)
            {
                assert_eq!(kind, DamageKind::Physical);
                match amount {
                    65 => plain += 1,
                    115 => {
                        bashes += 1;
                        assert!(world.held(target));
                        let held = world
                            .modifiers
                            .get(target)
                            .expect("effects")
                            .active()
                            .find(|m| {
                                effect_def(m.kind.effect_id())
                                    .is_some_and(|d| d.key == "modifier_bashed")
                            })
                            .expect("Bash disable");
                        assert!(
                            !effect_def(held.kind.effect_id())
                                .expect("effect")
                                .dispellable
                        );
                    }
                    other => panic!("unexpected attack damage {other}"),
                }
            }
        }
    }
    assert!(plain > 0);
    assert!(bashes > 0, "PRD must proc within32 attacks");
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Broken, None, None, Some(2000)),
    );
    for _ in 0..600 {
        for event in world.step() {
            if let EventKind::Damaged {
                target: hit,
                amount,
                ..
            } = event.kind
                && hit == crate::game::wire_id(target)
            {
                assert_eq!(amount, 65);
            }
        }
    }
}

#[test]
fn roshan_roars_while_stunned_and_silenced_then_disarms_and_amplifies_after_windup() {
    use crate::game::{Modifier, UnitOrder};
    use bota_core::{Control, ModifierKind, ability_def, effect_def};
    use bota_proto::{AbilitySlot, DamageKind, EffectId, EventKind, Target, Vec2};
    let mut world = match_with_roshan();
    world.immortal_heroes = true;
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
    world.set_order(hero, UnitOrder::Stand);
    world.settle();
    let slot = world
        .abilities
        .get(boss)
        .expect("book")
        .slots
        .iter()
        .position(|s| ability_def(s.id).is_some_and(|d| d.key == "roshan_revengeroar"))
        .expect("Roar in the imported NPC book");
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Stunned, Some(hero), None, Some(300)),
    );
    world.put_modifier(
        boss,
        Modifier::new(
            ModifierKind::Control {
                control: Control::Silence,
                effect: EffectId(89),
            },
            Some(hero),
            None,
            Some(300),
        ),
    );
    world.put_modifier(
        boss,
        Modifier::new(ModifierKind::Rooted, Some(hero), None, Some(300)),
    );
    assert!(world.begin_ability(boss, AbilitySlot(slot as u8), Target::None));
    assert!(!world.rooted(boss), "strong dispel removes Ensnare's root");
    assert!(
        !world.wears(boss, ModifierKind::Stunned),
        "strong dispel removes the incoming stun"
    );
    assert!(
        !world
            .modifiers
            .get(boss)
            .expect("effects")
            .active()
            .any(|m| m.kind.controls(Control::Silence))
    );
    assert!(world.held(boss), "Roar has its own full-disable windup");
    let mut hits = Vec::new();
    for tick in 1..=16 {
        for event in world.step() {
            if let EventKind::Damaged {
                target,
                amount,
                kind: DamageKind::Magical,
                ..
            } = event.kind
                && target == crate::game::wire_id(hero)
            {
                hits.push((tick, amount));
            }
        }
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 15, "half-second windup");
    assert_eq!(
        hits[0].1, 45,
        "own aura amplifies 50 magic damage before hero resistance"
    );
    assert!(!world.held(boss));
    assert!(world.disarmed(hero));
    let disarm = world
        .modifiers
        .get(hero)
        .expect("effects")
        .active()
        .find(|m| m.kind.effect_id() == EffectId(87))
        .expect("Roar disarm");
    let def = effect_def(disarm.kind.effect_id()).expect("effect");
    assert!(!def.dispellable);
    assert!(!def.strong_dispellable);
    assert_eq!(world.stats.get(hero).expect("stats").damage_taken_bp, 12500);
    assert_eq!(
        world.abilities.get(boss).expect("book").slots[slot].cooldown,
        1800 - 16
    );
}

#[test]
fn roshan_never_procs_bash_on_a_banner_or_other_ward() {
    use bota_core::{BANNERS, NeutralAbility};
    use bota_proto::{EventKind, Team, Vec2};
    let mut world = match_with_roshan();
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let banner = world.spawn_unit(&BANNERS[0], Team::Radiant, at + Vec2::from_ints(100, 0));
    world.settle();
    world.set_target(boss, banner);
    for _ in 0..2000 {
        let hp = world.stats.get(banner).expect("banner").max_hp;
        world.health.get_mut(banner).expect("banner").hp = hp;
        for event in world.step() {
            assert!(
                !matches!(event.kind,EventKind::AbilityCast{ability,..} if ability==NeutralAbility::Bash.id()),
                "wards must not roll Bash"
            );
        }
        assert!(!world.held(banner));
    }
}

#[test]
fn roshan_only_roars_below_85_percent_at_a_real_hero_from_the_previous_killing_side() {
    use crate::game::{Modifier, UnitOrder};
    use bota_core::{Control, ModifierKind, NeutralAbility};
    use bota_proto::{EffectId, EventKind, Fixed, Team, Vec2};
    for (previous, hero_side, hp_pct, expected) in [
        (None, Team::Radiant, 84, false),
        (Some(Team::Dire), Team::Radiant, 84, false),
        (Some(Team::Radiant), Team::Radiant, 85, false),
        (Some(Team::Radiant), Team::Radiant, 84, true),
    ] {
        let mut world = match_with_roshan();
        world.immortal_heroes = true;
        let boss = boss(&world);
        let at = world.pos(boss).expect("boss");
        let hero = world.seats[0].unit.expect("hero");
        world.set_team(hero, hero_side);
        world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
        world.set_order(hero, UnitOrder::Stand);
        world.lairs[0].last_killer = previous;
        world.settle();
        let max = world.stats.get(boss).expect("stats").max_hp;
        world.health.get_mut(boss).expect("boss").hp = Fixed {
            raw: (i64::from(max.raw) * hp_pct / 100) as i32,
        };
        world.set_target(boss, hero);
        world.neutral_ai.get_mut(boss).expect("mind").awake = true;
        world.neutral_ai.get_mut(boss).expect("mind").roused_by = Some(hero);
        world.put_modifier(
            boss,
            Modifier::new(ModifierKind::Disarmed, Some(hero), None, Some(300)),
        );
        world.put_modifier(
            boss,
            Modifier::new(ModifierKind::Stunned, Some(hero), None, Some(300)),
        );
        world.put_modifier(
            boss,
            Modifier::new(
                ModifierKind::Control {
                    control: Control::Silence,
                    effect: EffectId(89),
                },
                Some(hero),
                None,
                Some(300),
            ),
        );
        let mut roar = false;
        let mut message = false;
        for _ in 0..3 {
            for event in world.step() {
                match event.kind {
                    EventKind::AbilityCast { ability, .. }
                        if ability == NeutralAbility::RevengeRoar.id() =>
                    {
                        roar = true
                    }
                    EventKind::BossRoared { team, .. } => {
                        assert_eq!(Some(team), previous);
                        message = true;
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(roar, expected, "previous={previous:?}, hp={hp_pct}");
        assert_eq!(message, expected);
        if expected {
            let shown = world.view(Team::Radiant);
            let cast = shown
                .unit(crate::game::wire_id(boss))
                .expect("visible boss")
                .cast
                .expect("windup projected for the real animation");
            assert_eq!(cast.ability, Some(NeutralAbility::RevengeRoar.id()));
            assert!(cast.goes_off_in > 0);
        }
    }
}

#[test]
fn unblocked_dismember_releases_roshan_after_his_status_resisted_duration() {
    use crate::game::{AppliedModifier, AppliedModifiers, AppliedOrigin};
    use bota_core::{ModifierKind, rules};
    use bota_proto::{AbilitySlot, ModifierSpec, Order, Target, Vec2};
    for change_resistance in [false, true] {
        let mut cfg = config();
        cfg.map = bota_proto::MapId(0);
        cfg.objectives = bota_proto::ObjectiveSet::of(bota_proto::ObjectiveKind::Roshan);
        cfg.picks[0].hero = bota_proto::HeroId::PUDGE;
        let mut world = World::for_match(&cfg, cfg.rng());
        world.creeps_halted = true;
        world.immortal_heroes = true;
        let boss = boss(&world);
        let at = world.pos(boss).expect("boss");
        let hero = world.seats[0].unit.expect("hero");
        world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
        set_level(&mut world, hero, 3, 1);
        world.settle();
        let block = roshan_spell_block_slot(&world, boss);
        world.abilities.get_mut(boss).expect("book").slots[block].cooldown = 1000;
        advance_validated(
            &mut world,
            Some(hero),
            Order::Cast {
                slot: AbilitySlot(3),
                target: Target::Unit(crate::game::wire_id(boss)),
            },
        );
        for _ in 0..60 {
            if world.is_channelling(hero) {
                break;
            }
            world.step();
        }
        assert!(world.is_channelling(hero));
        let duration = rules::DISMEMBER_TICKS * 3 / 4;
        let stun = world
            .modifiers
            .get(boss)
            .expect("effects")
            .active()
            .find(|m| m.kind == ModifierKind::Stunned)
            .expect("stunned");
        assert_eq!(stun.ticks_total, Some(duration));
        let left = stun.ticks_left.expect("timed stun");
        if change_resistance {
            let mut mods = AppliedModifiers::default();
            mods.push(AppliedModifier {
                spec: ModifierSpec {
                    status_resist: 5000,
                    ..ModifierSpec::NOMINAL
                },
                ticks_left: Some(1000),
                origin: AppliedOrigin::Setup,
            });
            world.applied.insert(boss, mods);
        }
        for _ in 0..left - 1 {
            world.step();
            assert!(world.held(boss));
            assert!(
                world.is_channelling(hero),
                "the channel keeps its captured resistance"
            );
        }
        world.step();
        assert!(!world.held(boss));
        assert!(!world.is_channelling(hero));
        let released = world.pos(boss).expect("released");
        world.tick_neutral_spells();
        assert_eq!(world.pos(boss), Some(released));
    }
}

#[test]
fn roshan_does_not_slam_two_foes_or_a_crowd_without_a_hero() {
    use crate::game::UnitOrder;
    use bota_core::{MELEE_CREEP, NeutralAbility};
    use bota_proto::{EventKind, Team, Vec2};
    const CREEP: bota_core::UnitDef = bota_core::UnitDef {
        max_hp: 5000,
        damage: 0,
        move_speed: 0,
        ..MELEE_CREEP
    };
    for heroes in [false, true] {
        let mut world = match_with_roshan();
        world.immortal_heroes = true;
        let boss = boss(&world);
        let at = world.pos(boss).expect("boss");
        if heroes {
            let hero = world.seats[0].unit.expect("hero");
            world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
            world.set_order(hero, UnitOrder::Stand);
        }
        for offset in if heroes {
            &[(0, 160)][..]
        } else {
            &[(0, 100), (100, 0), (-100, 0)][..]
        } {
            let unit = world.spawn_unit(
                &CREEP,
                Team::Radiant,
                at + Vec2::from_ints(offset.0, offset.1),
            );
            world.set_order(unit, UnitOrder::Stand);
        }
        world.settle();
        for _ in 0..100 {
            for event in world.step() {
                assert!(
                    !matches!(event.kind,EventKind::AbilityCast{ability,..} if ability==NeutralAbility::RoshanSlam.id())
                );
            }
        }
    }
}

#[test]
fn slam_does_not_damage_or_slow_spell_immune_heroes_or_couriers() {
    use crate::game::Modifier;
    use bota_core::{COURIER, HERO, ModifierKind, NeutralAbility};
    use bota_proto::{AbilitySlot, Target, Team, Vec2};
    for def in [&COURIER, &HERO] {
        let mut world = match_with_roshan();
        let boss = boss(&world);
        let at = world.pos(boss).expect("boss");
        let target = world.spawn_unit(def, Team::Radiant, at + Vec2::from_ints(100, 0));
        world.put_modifier(
            target,
            Modifier::new(ModifierKind::Shielded, None, None, Some(100)),
        );
        world.settle();
        let hp = world.health.get(target).expect("target").hp;
        let slot = world
            .abilities
            .get(boss)
            .expect("book")
            .slots
            .iter()
            .position(|s| s.id == NeutralAbility::RoshanSlam.id())
            .expect("Slam");
        assert!(world.begin_ability(boss, AbilitySlot(slot as u8), Target::None));
        assert!(
            !world
                .modifiers
                .get(target)
                .expect("effects")
                .active()
                .any(|m| m.kind.effect_id() == bota_proto::EffectId(83))
        );
        world.step();
        assert!(world.health.get(target).expect("target").hp >= hp);
    }
}

#[test]
fn roars_strong_dispel_cancels_a_normal_dismember_without_a_final_pull_or_bite() {
    use bota_core::{ModifierKind, NeutralAbility};
    use bota_proto::{AbilitySlot, Fixed, Order, Target, Vec2};
    let mut cfg = config();
    cfg.map = bota_proto::MapId(0);
    cfg.objectives = bota_proto::ObjectiveSet::of(bota_proto::ObjectiveKind::Roshan);
    cfg.picks[0].hero = bota_proto::HeroId::PUDGE;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    world.immortal_heroes = true;
    let boss = boss(&world);
    let at = world.pos(boss).expect("boss");
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero").pos = at + Vec2::from_ints(100, 0);
    set_level(&mut world, hero, 3, 1);
    world.settle();
    let block = roshan_spell_block_slot(&world, boss);
    world.abilities.get_mut(boss).expect("book").slots[block].cooldown = 1000;
    let target = Target::Unit(crate::game::wire_id(boss));
    advance_validated(
        &mut world,
        Some(hero),
        Order::Cast {
            slot: AbilitySlot(3),
            target,
        },
    );
    for _ in 0..60 {
        if world.is_channelling(hero) {
            break;
        }
        world.step();
    }
    assert!(world.is_channelling(hero));
    assert!(world.wears(boss, ModifierKind::Dismembered { level: 0 }));
    let slot = world
        .abilities
        .get(boss)
        .expect("book")
        .slots
        .iter()
        .position(|s| s.id == NeutralAbility::RevengeRoar.id())
        .expect("Roar");
    assert!(world.begin_ability(boss, AbilitySlot(slot as u8), Target::None));
    let released = world.pos(hero).expect("caster") + Vec2::from_ints(200, 0);
    world.transform.get_mut(boss).expect("boss").pos = released;
    assert!(!world.dismember_holds(hero, target));
    assert_eq!(world.pos(boss), Some(released));
    let hp = world.health.get(boss).expect("boss").hp;
    world.step();
    assert!(!world.is_channelling(hero));
    assert_eq!(world.pos(boss), Some(released));
    assert!(
        world.health.get(boss).expect("boss").hp >= hp - Fixed::ONE,
        "no final damage bite after release"
    );
}

#[test]
fn an_illusion_rune_cleanses_slam_but_preserves_the_heros_beneficial_haste() {
    use crate::game::Modifier;
    use bota_core::{ModifierKind, NeutralAbility};
    let mut world = match_with_roshan();
    let hero = world.seats[0].unit.expect("hero");
    world.put_modifier(
        hero,
        Modifier::new(
            ModifierKind::Spell {
                ability: NeutralAbility::RoshanSlam,
                level: 1,
                amount: 0,
            },
            None,
            None,
            Some(120),
        ),
    );
    world.put_modifier(
        hero,
        Modifier::new(ModifierKind::Hastened { pct: 20 }, None, None, Some(120)),
    );
    world.make_rune_illusions(hero);
    assert!(
        !world
            .modifiers
            .get(hero)
            .expect("effects")
            .active()
            .any(|m| m.kind.effect_id() == bota_proto::EffectId(83))
    );
    assert!(world.wears(hero, ModifierKind::Hastened { pct: 20 }));
}
