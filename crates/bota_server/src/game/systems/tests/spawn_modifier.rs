//! Trusted setup modifiers: which spawns they take and how long they hold.

use bota_core::{MELEE_CREEP, RANGED_CREEP, SHADOW_FIEND, rules, tower_def};

use super::support::{a_spec, spec};
use crate::game::{
    AppliedModifier, AppliedModifiers, AppliedOrigin, Command, Def, Entity, MAX_SPAWN_MODIFIERS,
    MAX_SPAWN_TARGETS, MatchConfig, MatchConfigError, ModifierDuration, Place, SpawnCategory,
    SpawnModifier, SpawnModifierError, SpawnSelector, SpawnTarget, World,
};
use bota_proto::{
    Cheat, DamageKind, Fixed, HeroId, MAX_MODIFIER_TICKS, MapId, ModifierSpec, Order, Pick, SlotId,
    Target, Team, TickMode, UnitKind, Vec2,
};

/// A rule taking everything either side, running until the body falls.
fn a_rule(spec: ModifierSpec) -> SpawnModifier {
    SpawnModifier {
        select: SpawnSelector {
            team: None,
            targets: Vec::new(),
        },
        spec,
        duration: ModifierDuration::MatchLong,
    }
}

/// A one-seat match on the full map with the given rules.
fn config(cheats: bool, rules: Vec<SpawnModifier>) -> MatchConfig {
    MatchConfig {
        match_id: 21,
        master_key: [5; 32],
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::SHADOW_FIEND,
        }],
        map: MapId(0),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 30,
        cheats,
        spawn_modifiers: rules,
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// A world with a hero and a creep, the given rules standing on it.
fn arena(rules: Vec<SpawnModifier>) -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(7000, 7000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(2000, 2000));
    world.spawn_modifiers = rules;
    world.apply_spawn_modifiers_to_all();
    world.settle();
    (world, hero, creep)
}

/// A rule scaling maximum health by `scale` basis points on everything.
fn hp_rule(scale: i32) -> SpawnModifier {
    a_rule(spec(|s| s.max_hp = scale))
}

/// A match under `rules`, and the one seat's hero.
fn a_match(cheats: bool, rules: Vec<SpawnModifier>) -> (World, Entity) {
    let cfg = config(cheats, rules);
    let world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    (world, hero)
}

/// A unit's maximum health from its definition and upgrades alone.
fn raised_hp(world: &World, entity: Entity) -> Fixed {
    let Def(def) = world.def.get(entity).expect("a unit");
    let up = world.upgrades.get(entity).map_or(0, |u| u.0 as i32);
    Fixed::from_int(def.max_hp + up * def.per_upgrade.hp)
}

/// Ticks left on the first applied entry of a unit.
fn first_ticks_left(world: &World, entity: Entity) -> Option<u32> {
    world.applied.get(entity)?.iter().next()?.ticks_left
}

/// Every unit of `kind`, each asserted to stand full at double its base
/// health and to carry the rule.
fn doubled(world: &World, kind: UnitKind, case: &str) -> Vec<Entity> {
    let found: Vec<Entity> = world
        .entities
        .iter()
        .filter(|entity| world.kind.get(*entity).copied() == Some(kind))
        .collect();
    assert!(!found.is_empty(), "{case}: some stood up");
    for &entity in &found {
        let expected = Some(raised_hp(world, entity) * Fixed::from_int(2));
        let max_hp = world.stats.get(entity).map(|stats| stats.max_hp);
        assert_eq!(max_hp, expected, "{case}: raised");
        let hp = world.health.get(entity).map(|health| health.hp);
        assert_eq!(hp, expected, "{case}: and stands full");
        assert!(
            world.applied.contains(entity),
            "{case}: and carries the rule"
        );
    }
    found
}

/// Kills the seat's hero and stands it up again at once.
fn kill_and_respawn(world: &mut World, hero: Entity) -> Entity {
    world.push_hit(None, hero, 1_000_000, DamageKind::Pure);
    world.step();
    world.seats[0].respawn_left = 1;
    world.tick_respawns();
    world.seats[0].unit.expect("respawned")
}

#[test]
fn a_hero_starts_with_a_match_setup_modifier() {
    let cfg = config(false, vec![hp_rule(20_000)]);
    assert!(cfg.validate().is_ok());
    let (world, hero) = a_match(false, vec![hp_rule(20_000)]);
    let stats = world.stats.get(hero).expect("settled");
    let expected = SHADOW_FIEND.max_hp * 2 + rules::HP_PER_STRENGTH * 19;
    assert_eq!(stats.max_hp.to_int(), expected);
    assert_eq!(
        world.health.get(hero).map(|health| health.hp),
        Some(stats.max_hp),
        "it stands up full"
    );
    let applied = world.applied.get(hero).expect("seeded");
    assert_eq!(applied.iter().count(), 1);
    assert_eq!(
        applied
            .iter()
            .next()
            .map(|held| (held.origin, held.ticks_left)),
        Some((AppliedOrigin::Setup, None))
    );
}

#[test]
fn a_hero_on_the_other_side_is_left_alone() {
    let mut rule = hp_rule(20_000);
    rule.select.team = Some(Team::Dire);
    let (world, hero, creep) = arena(vec![rule]);
    assert!(!world.applied.contains(hero));
    assert!(world.applied.contains(creep));
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_hp),
        Some(Fixed::from_int(
            SHADOW_FIEND.max_hp + rules::HP_PER_STRENGTH * 19
        )),
        "the hero is untouched"
    );
}

#[test]
fn an_exact_kind_selector_takes_only_that_kind() {
    let mut world = World::new();
    let melee = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(2000, 2000));
    let ranged = world.spawn_unit(&RANGED_CREEP, Team::Dire, Vec2::from_ints(2200, 2000));
    world.spawn_modifiers = vec![SpawnModifier {
        select: SpawnSelector {
            targets: vec![SpawnTarget::Kind(UnitKind::CreepRanged)],
            ..SpawnSelector::default()
        },
        spec: spec(|s| s.max_hp = 15_000),
        duration: ModifierDuration::MatchLong,
    }];
    world.apply_spawn_modifiers_to_all();
    world.settle();
    assert!(!world.applied.contains(melee));
    assert!(world.applied.contains(ranged));
}

#[test]
fn lane_creeps_are_seeded_on_every_wave() {
    let (mut world, _hero) = a_match(false, vec![hp_rule(20_000)]);
    for wave in 1..=2u32 {
        world.tick = rules::FIRST_WAVE_TICK + (wave - 1) * rules::WAVE_PERIOD_TICKS;
        world.spawn_waves();
        doubled(&world, UnitKind::CreepMelee, &format!("wave {wave}"));
    }
}

#[test]
fn camp_creeps_are_seeded_on_every_fill() {
    let (mut world, _hero) = a_match(false, vec![hp_rule(20_000)]);
    world.tick = rules::FIRST_NEUTRAL_TICK;
    world.fill_camps();
    let first = doubled(&world, UnitKind::CreepNeutral, "the first fill");
    for entity in &first {
        world.push_hit(None, *entity, 1_000_000, DamageKind::Pure);
    }
    world.step();
    assert!(
        world
            .entities
            .iter()
            .all(|entity| world.kind.get(entity).copied() != Some(UnitKind::CreepNeutral)),
        "the camp is cleared"
    );
    world.tick = rules::FIRST_NEUTRAL_TICK + rules::NEUTRAL_SPAWN_PERIOD_TICKS;
    world.fill_camps();
    let refilled = doubled(&world, UnitKind::CreepNeutral, "the refill");
    assert!(
        refilled.iter().all(|entity| !first.contains(entity)),
        "a refilled camp stands new bodies"
    );
}

#[test]
fn structures_are_seeded_at_the_start_and_when_stood_up() {
    let rule = SpawnModifier {
        select: SpawnSelector {
            targets: vec![SpawnTarget::Category(SpawnCategory::Structure)],
            ..SpawnSelector::default()
        },
        spec: spec(|s| s.max_hp = 15_000),
        duration: ModifierDuration::MatchLong,
    };
    let (world, _hero) = a_match(false, vec![rule.clone()]);
    let tower = world
        .entities
        .iter()
        .find(|entity| world.kind.get(*entity).copied() == Some(UnitKind::Tower))
        .expect("the map stands towers");
    assert!(world.applied.contains(tower), "a standing tower is seeded");
    let expected = world
        .stats
        .get(tower)
        .map(|stats| stats.max_hp)
        .expect("settled");
    assert_eq!(
        expected,
        Fixed {
            raw: (i64::from(raised_hp(&world, tower).raw) * 15_000 / 10_000) as i32
        }
    );
    assert_eq!(
        world.health.get(tower).map(|health| health.hp),
        Some(expected),
        "and it is full to the raised maximum right away"
    );

    let mut later = World::new();
    later.spawn_modifiers = vec![rule];
    let standing = later.spawn_building(
        tower_def(1),
        Team::Dire,
        Vec2::from_ints(5000, 5000),
        Place::Tower { lane: 0, tier: 1 },
    );
    later.settle();
    assert!(later.applied.contains(standing), "a later tower is seeded");
    assert_eq!(
        later.stats.get(standing).map(|stats| stats.max_hp.to_int()),
        Some(tower_def(1).max_hp * 15_000 / 10_000)
    );
}

#[test]
fn several_rules_add_their_scales_once() {
    let (world, hero) = a_match(
        false,
        vec![
            a_rule(spec(|s| s.move_speed = 11_000)),
            a_rule(spec(|s| s.move_speed = 12_000)),
            hp_rule(12_500),
            hp_rule(12_500),
        ],
    );
    let stats = world.stats.get(hero).expect("settled");
    assert_eq!(
        stats.move_speed.to_int(),
        SHADOW_FIEND.move_speed * 13 / 10,
        "deltas add: 10% and 20% make 30%, not 32%"
    );
    assert_eq!(
        stats.max_hp.to_int(),
        SHADOW_FIEND.max_hp * 15 / 10 + rules::HP_PER_STRENGTH * 19,
        "the two quarter shares make one half"
    );
}

#[test]
fn a_ticks_rule_lifts_after_its_ticks() {
    let rule = SpawnModifier {
        duration: ModifierDuration::Ticks(2),
        ..hp_rule(20_000)
    };
    let (mut world, hero) = a_match(false, vec![rule]);
    assert_eq!(first_ticks_left(&world, hero), Some(2));
    world.step();
    assert!(world.applied.contains(hero), "one tick left");
    world.step();
    assert!(!world.applied.contains(hero), "it has run out");
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_hp.to_int()),
        Some(SHADOW_FIEND.max_hp + rules::HP_PER_STRENGTH * 19),
        "and the maximum is back"
    );
}

#[test]
fn a_respawned_hero_is_seeded_afresh_and_stands_full_at_its_maximum() {
    for scale in [20_000, 5_000] {
        let (mut world, hero) = a_match(false, vec![hp_rule(scale)]);
        let full = SHADOW_FIEND.max_hp * scale / 10_000 + rules::HP_PER_STRENGTH * 19;
        let max_hp = |world: &World, hero| world.stats.get(hero).map(|stats| stats.max_hp);
        assert_eq!(max_hp(&world, hero), Some(Fixed::from_int(full)), "{scale}");
        world.push_hit(None, hero, 1_000_000, DamageKind::Pure);
        world.step();
        assert!(!world.alive(hero), "the blow was fatal");
        let wait = world.seats[0].respawn_left;
        assert!(wait > 0);
        for _ in 0..=wait {
            world.step();
        }
        let reborn = world.seats[0].unit.expect("the hero comes back");
        assert_eq!(
            max_hp(&world, reborn),
            Some(Fixed::from_int(full)),
            "{scale}: the rule is on the new body"
        );
        assert_eq!(
            world.health.get(reborn).map(|health| health.hp),
            max_hp(&world, reborn),
            "{scale}: and it stands full at that maximum and no further"
        );
        assert_eq!(
            world
                .applied
                .get(reborn)
                .and_then(|applied| applied.iter().next()),
            Some(&AppliedModifier {
                spec: spec(|s| s.max_hp = scale),
                ticks_left: None,
                origin: AppliedOrigin::Setup,
            })
        );
    }
}

#[test]
fn a_ticks_rule_restarts_its_full_duration_on_every_respawn() {
    let rule = SpawnModifier {
        select: SpawnSelector {
            team: None,
            targets: vec![SpawnTarget::Category(SpawnCategory::Hero)],
        },
        spec: spec(|s| s.max_hp = 12_500),
        duration: ModifierDuration::Ticks(3),
    };
    let (mut world, first) = a_match(false, vec![rule]);
    assert_eq!(first_ticks_left(&world, first), Some(3));
    let second = kill_and_respawn(&mut world, first);
    assert_eq!(
        first_ticks_left(&world, second),
        Some(3),
        "a new body starts the rule over"
    );
    for expected in [2, 1] {
        world.step();
        assert_eq!(first_ticks_left(&world, second), Some(expected));
    }
    world.step();
    assert!(!world.applied.contains(second), "three ticks have run");
    let third = kill_and_respawn(&mut world, second);
    assert_eq!(
        first_ticks_left(&world, third),
        Some(3),
        "every respawn starts it over"
    );
}

#[test]
fn an_empty_rule_list_leaves_the_world_alone() {
    let (world, _hero) = a_match(false, Vec::new());
    assert!(world.spawn_modifiers.is_empty());
    assert!(world.applied.is_empty(), "nothing was seeded");
    for entity in world.entities.iter() {
        assert!(!world.applied.contains(entity));
    }
}

/// The hash of a world holding one applied spec.
fn hash_with_spec(spec: ModifierSpec) -> u64 {
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(2000, 2000));
    let mut applied = AppliedModifiers::default();
    applied.push(AppliedModifier {
        spec,
        ticks_left: Some(10),
        origin: AppliedOrigin::Setup,
    });
    world.applied.insert(creep, applied);
    world.settle();
    world.hash()
}

/// One named change to a spec field.
type FieldChange = (&'static str, fn(&mut ModifierSpec));

#[test]
fn every_modifier_field_changes_the_hash() {
    let base = a_spec();
    let baseline = hash_with_spec(base);
    let changes: [FieldChange; 11] = [
        ("magic_resist", |s| s.magic_resist += 1),
        ("status_resist", |s| s.status_resist += 1),
        ("physical_damage", |s| s.physical_damage += 1),
        ("magic_damage", |s| s.magic_damage += 1),
        ("pure_damage", |s| s.pure_damage += 1),
        ("cooldown_rate", |s| s.cooldown_rate += 1),
        ("mana_cost_rate", |s| s.mana_cost_rate += 1),
        ("move_speed", |s| s.move_speed += 1),
        ("max_hp", |s| s.max_hp += 1),
        ("max_mana", |s| s.max_mana += 1),
        ("gold_income", |s| s.gold_income += 1),
    ];
    for (name, change) in changes {
        let mut changed = base;
        change(&mut changed);
        assert_ne!(
            hash_with_spec(changed),
            baseline,
            "{name} must show in the hash"
        );
    }
}

/// The hash of a world carrying one trusted spawn rule.
fn hash_with_rule(rule: SpawnModifier) -> u64 {
    let mut world = World::new();
    world.spawn_modifiers = vec![rule];
    world.hash()
}

/// One named change to a spawn rule.
type RuleChange = (&'static str, fn(&mut SpawnModifier));

#[test]
fn every_spawn_rule_field_changes_the_hash() {
    let base = hp_rule(12_500);
    let baseline = hash_with_rule(base.clone());
    let changes: [RuleChange; 5] = [
        ("team", |r| r.select.team = Some(Team::Dire)),
        ("category", |r| {
            r.select.targets = vec![SpawnTarget::Category(SpawnCategory::Hero)];
        }),
        ("kind", |r| {
            r.select.targets = vec![SpawnTarget::Kind(UnitKind::Tower)];
        }),
        ("duration", |r| r.duration = ModifierDuration::Ticks(5)),
        ("spec", |r| r.spec.max_hp = 12_501),
    ];
    for (name, change) in changes {
        let mut changed = base.clone();
        change(&mut changed);
        assert_ne!(
            hash_with_rule(changed),
            baseline,
            "{name} must show in the hash"
        );
    }
    let mut two = World::new();
    two.spawn_modifiers = vec![base.clone(), base];
    assert_ne!(two.hash(), baseline, "the number of rules must show");
}

#[test]
fn invalid_rules_are_refused_at_setup() {
    let refused = |error| Err(MatchConfigError::SpawnModifier { at: 0, error });
    let mut cfg = config(false, vec![a_rule(ModifierSpec::NOMINAL)]);
    cfg.spawn_modifiers[0].spec.status_resist = ModifierSpec::MAX_STATUS_RESIST + 1;
    assert_eq!(cfg.validate(), refused(SpawnModifierError::UnboundedSpec));
    cfg.spawn_modifiers[0].spec = ModifierSpec::NOMINAL;
    for ticks in [0, MAX_MODIFIER_TICKS + 1] {
        cfg.spawn_modifiers[0].duration = ModifierDuration::Ticks(ticks);
        assert_eq!(
            cfg.validate(),
            refused(SpawnModifierError::BadDuration),
            "{ticks} ticks"
        );
    }
    cfg.spawn_modifiers[0].duration = ModifierDuration::MatchLong;
    let selector = &mut cfg.spawn_modifiers[0].select.targets;
    *selector = vec![SpawnTarget::Kind(UnitKind::Hero); MAX_SPAWN_TARGETS];
    assert_eq!(cfg.validate(), Ok(()), "the selector bound is allowed");
    cfg.spawn_modifiers[0]
        .select
        .targets
        .push(SpawnTarget::Kind(UnitKind::Hero));
    assert_eq!(
        cfg.validate(),
        refused(SpawnModifierError::TooManyTargets {
            count: MAX_SPAWN_TARGETS + 1,
        })
    );
    cfg.spawn_modifiers = vec![a_rule(ModifierSpec::NOMINAL); MAX_SPAWN_MODIFIERS + 1];
    assert_eq!(
        cfg.validate(),
        Err(MatchConfigError::TooManySpawnModifiers {
            count: MAX_SPAWN_MODIFIERS + 1,
        })
    );
    cfg.spawn_modifiers = vec![a_rule(ModifierSpec::NOMINAL); MAX_SPAWN_MODIFIERS];
    assert_eq!(cfg.validate(), Ok(()), "the bound itself is allowed");
}

#[test]
#[should_panic(
    expected = "the match setup was refused: spawn modifier 0 is refused: 17 selector targets exceed the 16 a rule may carry"
)]
fn for_match_refuses_an_oversized_selector_loudly() {
    let mut rule = a_rule(ModifierSpec::NOMINAL);
    rule.select.targets = vec![SpawnTarget::Kind(UnitKind::Hero); MAX_SPAWN_TARGETS + 1];
    let cfg = config(false, vec![rule]);
    let _ = World::for_match(&cfg, cfg.rng());
}

#[test]
#[should_panic(
    expected = "the match setup was refused: 65 spawn modifiers exceed the 64 a match may carry"
)]
fn for_match_refuses_an_oversized_rule_list_loudly() {
    let cfg = config(
        false,
        vec![a_rule(ModifierSpec::NOMINAL); MAX_SPAWN_MODIFIERS + 1],
    );
    let _ = World::for_match(&cfg, cfg.rng());
}

#[test]
#[should_panic(
    expected = "spawn modifier 1 was applied without being checked: 17 selector targets exceed the 16 a rule may carry"
)]
fn a_spawn_refuses_an_unchecked_rule_with_its_index_and_reason() {
    let mut invalid = a_rule(ModifierSpec::NOMINAL);
    invalid.select.targets = vec![SpawnTarget::Kind(UnitKind::CreepMelee); MAX_SPAWN_TARGETS + 1];
    let mut world = World::new();
    world.spawn_modifiers = vec![a_rule(ModifierSpec::NOMINAL), invalid];
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(2000, 2000));
}

#[test]
fn a_cheat_does_not_take_trusted_setup_off_a_unit() {
    let (mut world, hero) = a_match(true, vec![hp_rule(20_000)]);
    let cheat = |cheat: Cheat| Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cheat { cheat },
    };
    world.advance(&[cheat(Cheat::ApplyModifier {
        target: Target::None,
        spec: spec(|s| s.status_resist = 2_000),
        ticks: 50,
    })]);
    let applied = world.applied.get(hero).expect("both are on it");
    assert_eq!(applied.iter().count(), 2);
    assert!(
        applied
            .iter()
            .any(|held| held.origin == AppliedOrigin::Setup)
    );
    assert!(
        applied
            .iter()
            .any(|held| held.origin == AppliedOrigin::Cheat)
    );
    world.advance(&[cheat(Cheat::ClearModifiers {
        target: Target::None,
    })]);
    let applied = world.applied.get(hero).expect("setup stays");
    assert_eq!(applied.iter().count(), 1);
    assert_eq!(
        applied.iter().next().map(|held| held.origin),
        Some(AppliedOrigin::Setup)
    );
}
