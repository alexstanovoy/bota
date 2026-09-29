//! Applied stat changes: what each is worth and what it ignores.

use bota_proto::{
    AbilitySlot, DamageKind, Fixed, HeroId, ItemId, ModifierSpec, SlotId, Target, Team, UnitKind,
    Vec2,
};

use crate::game::{
    AppliedModifier, AppliedModifiers, AppliedOrigin, Aura, AuraCx, Auras, Entity, ITEMS,
    ItemStack, MAX_APPLIED_MODIFIERS_PER_UNIT, MAX_COMBINED_MODIFIER_SCALE, MELEE_CREEP, Modifier,
    ModifierKind, Reach, StackKind, Stacks, World, ability, ability_cooldown, ability_mana_cost,
    aura_system, cooldown_after, cost_after, rules, wire_id,
};

/// A world with a hero and a creep standing well apart.
fn arena() -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(7000, 7000),
        SlotId(0),
        HeroId(0),
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1000, 1000));
    world.settle();
    (world, hero, creep)
}

/// One nominal spec with `change` applied to it.
fn spec(change: impl FnOnce(&mut ModifierSpec)) -> ModifierSpec {
    let mut spec = ModifierSpec::NOMINAL;
    change(&mut spec);
    spec
}

/// One setup entry in an applied set.
fn one_applied(spec: ModifierSpec, ticks: u32) -> AppliedModifiers {
    let mut applied = AppliedModifiers::default();
    applied.push(AppliedModifier {
        spec,
        ticks_left: Some(ticks),
        origin: AppliedOrigin::Setup,
    });
    applied
}

/// One extreme spec, with every scale at `scale`.
fn extreme_spec(scale: i32, magic_resist: i32, status_resist: i32) -> ModifierSpec {
    ModifierSpec {
        magic_resist,
        status_resist,
        physical_damage: scale,
        magic_damage: scale,
        pure_damage: scale,
        cooldown_rate: scale,
        mana_cost_rate: scale,
        move_speed: scale,
        max_hp: scale,
        max_mana: scale,
        gold_income: scale,
    }
}

/// A full per-unit set, every setup source and the one cheat source.
fn full_applied(spec: ModifierSpec) -> AppliedModifiers {
    let mut applied = AppliedModifiers::default();
    for at in 0..MAX_APPLIED_MODIFIERS_PER_UNIT {
        applied.push(AppliedModifier {
            spec,
            ticks_left: None,
            origin: if at + 1 == MAX_APPLIED_MODIFIERS_PER_UNIT {
                AppliedOrigin::Cheat
            } else {
                AppliedOrigin::Setup
            },
        });
    }
    applied
}

/// Puts a spec on a unit and works its stats out again.
fn apply(world: &mut World, on: Entity, spec: ModifierSpec, ticks: u32) {
    world.applied.insert(on, one_applied(spec, ticks));
    world.settle();
}

/// Damage one blow of a kind takes off a target, through a whole tick.
fn dealt(world: &mut World, from: Entity, to: Entity, amount: i32, kind: DamageKind) -> i32 {
    let before = world.health.get(to).expect("standing").hp;
    world.push_hit(Some(from), to, amount, kind);
    world.step();
    (before - world.health.get(to).expect("standing").hp).to_int()
}

/// A magical bounce in flight, its source carrying double amplification.
fn amplified_bounce(ticks: u32) -> (World, Entity, Entity, Entity) {
    let mut world = World::new();
    let caster = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId(0),
    );
    let target = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5500, 5000));
    world.settle();
    apply(&mut world, caster, spec(|s| s.magic_damage = 20_000), ticks);
    assert!(world.cast_bounce(caster, 0, Target::Unit(wire_id(target))));
    let missile = world
        .entities
        .iter()
        .find(|entity| world.projectile.get(*entity).is_some())
        .expect("the bounce is in flight");
    let shot = world.projectile.get_mut(missile).expect("in flight");
    shot.bounces_left = 0;
    assert_eq!(shot.damage_amp_bp, 20_000);
    (world, caster, target, missile)
}

/// Runs one launched bounce to impact and returns health it took.
fn land_bounce(world: &mut World, target: Entity, missile: Entity) -> i32 {
    let before = world.health.get(target).expect("standing").hp;
    for _ in 0..64 {
        world.step();
        if world.projectile.get(missile).is_none() {
            let after = world.health.get(target).expect("survives").hp;
            return (before - after).to_int();
        }
    }
    panic!("the bounce did not land inside its bounded flight")
}

/// A stun of `ticks` ticks.
fn stun(ticks: u32) -> Modifier {
    Modifier {
        kind: ModifierKind::Stunned,
        source: None,
        ticks_left: Some(ticks),
    }
}

/// What a modifier of one kind has left on a unit.
fn left_on(world: &World, on: Entity, kind: ModifierKind) -> Option<u32> {
    world
        .modifiers
        .get(on)?
        .active()
        .find(|held| held.kind == kind)?
        .ticks_left
}

/// Puts an item in the first bag slot of a unit.
fn give(world: &mut World, on: Entity, item: u16) {
    let tick = world.tick;
    let Some(stack) = ItemStack::bought(ItemId(item), SlotId(0), tick) else {
        return;
    };
    if let Some(bag) = world.inventory.get_mut(on) {
        bag.slots[0] = Some(stack);
    }
}

#[test]
fn every_combined_modifier_family_has_an_explicit_upper_bound() {
    let (mut world, hero, _creep) = arena();
    world.applied.insert(
        hero,
        full_applied(extreme_spec(
            ModifierSpec::MAX_SCALE,
            ModifierSpec::MAX_RESIST,
            ModifierSpec::MAX_STATUS_RESIST,
        )),
    );
    world.settle();
    let stats = *world.stats.get(hero).expect("settled");
    assert_eq!(stats.magic_resist_pct, 100);
    assert_eq!(stats.status_resist_bp, rules::NOMINAL_BP - 1);
    assert_eq!(stats.physical_amp_bp, MAX_COMBINED_MODIFIER_SCALE);
    assert_eq!(stats.magic_amp_bp, MAX_COMBINED_MODIFIER_SCALE);
    assert_eq!(stats.pure_amp_bp, MAX_COMBINED_MODIFIER_SCALE);
    assert_eq!(stats.cooldown_rate_bp, MAX_COMBINED_MODIFIER_SCALE);
    assert_eq!(stats.mana_cost_rate_bp, MAX_COMBINED_MODIFIER_SCALE);
    assert_eq!(
        stats.move_speed.to_int(),
        rules::HERO_MOVE_SPEED * ModifierSpec::MAX_SCALE / rules::NOMINAL_BP,
        "world magnitudes saturate at one source's 4x bound"
    );
    assert_eq!(
        stats.max_hp.to_int(),
        rules::HERO_HP * ModifierSpec::MAX_SCALE / rules::NOMINAL_BP + rules::HP_PER_STRENGTH * 20
    );
    assert_eq!(
        stats.max_mana.to_int(),
        rules::HERO_MANA * ModifierSpec::MAX_SCALE / rules::NOMINAL_BP
            + rules::MANA_PER_INTELLIGENCE * 18
    );
    assert_eq!(
        world.bounty_after(Some(hero), 100),
        100 * MAX_COMBINED_MODIFIER_SCALE / rules::NOMINAL_BP,
        "bounty keeps every bounded additive source"
    );
    assert_eq!(
        world.applied.get(hero).map(|held| held.iter().count()),
        Some(MAX_APPLIED_MODIFIERS_PER_UNIT)
    );
}

#[test]
fn every_combined_modifier_family_has_an_explicit_lower_bound() {
    let (mut world, hero, _creep) = arena();
    world.applied.insert(
        hero,
        full_applied(extreme_spec(
            ModifierSpec::MIN_SCALE,
            -ModifierSpec::MAX_RESIST,
            ModifierSpec::MAX_STATUS_RESIST,
        )),
    );
    world.settle();
    let stats = *world.stats.get(hero).expect("settled");
    assert_eq!(stats.magic_resist_pct, 0);
    assert_eq!(stats.status_resist_bp, rules::NOMINAL_BP - 1);
    assert_eq!(stats.physical_amp_bp, 0);
    assert_eq!(stats.magic_amp_bp, 0);
    assert_eq!(stats.pure_amp_bp, 0);
    assert_eq!(stats.cooldown_rate_bp, 1);
    assert_eq!(stats.mana_cost_rate_bp, 1);
    assert_eq!(
        stats.move_speed.to_int(),
        rules::HERO_MOVE_SPEED * ModifierSpec::MIN_SCALE / rules::NOMINAL_BP
    );
    assert_eq!(
        stats.max_hp.to_int(),
        rules::HERO_HP * ModifierSpec::MIN_SCALE / rules::NOMINAL_BP + rules::HP_PER_STRENGTH * 20
    );
    assert_eq!(
        stats.max_mana.to_int(),
        rules::HERO_MANA * ModifierSpec::MIN_SCALE / rules::NOMINAL_BP
            + rules::MANA_PER_INTELLIGENCE * 18
    );
    assert_eq!(world.bounty_after(Some(hero), 100), 0);
}

#[test]
#[should_panic(expected = "a unit may carry at most 65 applied modifiers")]
fn a_unit_refuses_one_applied_entry_past_its_bound() {
    let mut applied = full_applied(ModifierSpec::NOMINAL);
    applied.push(AppliedModifier {
        spec: ModifierSpec::NOMINAL,
        ticks_left: None,
        origin: AppliedOrigin::Setup,
    });
}

#[test]
fn neutral_stats_leave_damage_alone() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, ModifierSpec::NOMINAL, 10);
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Pure), 100);
    assert_eq!(dealt(&mut world, hero, creep, 64, DamageKind::Magical), 64);
}

#[test]
fn magic_resistance_delta_reduces_magical_damage_only() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, creep, spec(|s| s.magic_resist = 5_000), 10);
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Magical), 50);
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Pure), 100);
}

#[test]
fn magic_resistance_delta_is_clamped_at_total_immunity() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, creep, spec(|s| s.magic_resist = 10_000), 10);
    assert_eq!(
        world.stats.get(creep).map(|stats| stats.magic_resist_pct),
        Some(100),
        "the delta may not take it past immunity"
    );
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Magical), 0);
}

#[test]
fn magical_amplification_scales_magical_damage() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, spec(|s| s.magic_damage = 15_000), 10);
    assert_eq!(
        dealt(&mut world, hero, creep, 100, DamageKind::Magical),
        150
    );
}

#[test]
fn magical_amplification_applies_before_magic_resistance() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, spec(|s| s.magic_damage = 20_000), 10);
    apply(&mut world, creep, spec(|s| s.magic_resist = 5_000), 10);
    assert_eq!(
        dealt(&mut world, hero, creep, 100, DamageKind::Magical),
        100,
        "double the blow, then half of it kept"
    );
}

#[test]
fn physical_amplification_scales_physical_damage() {
    let (mut world, hero, creep) = arena();
    let neutral = dealt(&mut world, hero, creep, 100, DamageKind::Physical);
    assert!(neutral > 0);
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, spec(|s| s.physical_damage = 20_000), 10);
    assert_eq!(
        dealt(&mut world, hero, creep, 100, DamageKind::Physical),
        2 * neutral
    );
}

#[test]
fn pure_amplification_scales_pure_damage() {
    let (mut world, hero, creep) = arena();
    let neutral = dealt(&mut world, hero, creep, 100, DamageKind::Pure);
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, spec(|s| s.pure_damage = 15_000), 10);
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Pure), 150);
    assert_eq!(neutral, 100);
}

#[test]
fn amplification_of_one_kind_leaves_the_other_kinds_alone() {
    let (mut world, hero, creep) = arena();
    apply(&mut world, hero, spec(|s| s.physical_damage = 20_000), 10);
    assert_eq!(
        dealt(&mut world, hero, creep, 100, DamageKind::Magical),
        100
    );
    assert_eq!(dealt(&mut world, hero, creep, 100, DamageKind::Pure), 100);
}

#[test]
fn a_projectile_keeps_its_launch_amplification_after_the_attacker_falls() {
    let (mut world, caster, target, missile) = amplified_bounce(100);
    world.push_hit(None, caster, 1_000_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(caster), "the caster fell before impact");
    let before = world.health.get(target).expect("standing").hp;
    let taken = land_bounce(&mut world, target, missile);
    assert_eq!(taken, rules::SYLLA_BOUNCE_DAMAGE[0] * 2);
    assert_eq!(
        world.health.get(target).expect("survives").hp,
        before - Fixed::from_int(taken)
    );
}

#[test]
fn a_projectile_keeps_its_launch_amplification_after_the_modifier_lifts() {
    let (mut world, caster, target, missile) = amplified_bounce(1);
    world.step();
    assert!(world.alive(caster));
    assert!(!world.applied.contains(caster), "the modifier has run out");
    assert_eq!(
        land_bounce(&mut world, target, missile),
        rules::SYLLA_BOUNCE_DAMAGE[0] * 2
    );
}

#[test]
fn a_projectile_keeps_its_launch_amplification_when_the_source_slot_is_reused() {
    let (mut world, caster, target, missile) = amplified_bounce(100);
    world.push_hit(None, caster, 1_000_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(caster));
    let replacement = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(3000, 3000));
    assert_eq!(
        replacement.index(),
        caster.index(),
        "the free slot is reused"
    );
    assert_ne!(replacement.generation(), caster.generation());
    world.settle();
    assert_eq!(
        land_bounce(&mut world, target, missile),
        rules::SYLLA_BOUNCE_DAMAGE[0] * 2
    );
}

#[test]
fn neutral_status_resistance_leaves_durations_alone() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, ModifierSpec::NOMINAL, 10);
    world.put_modifier(hero, stun(10));
    assert_eq!(left_on(&world, hero, ModifierKind::Stunned), Some(10));
}

#[test]
fn status_resistance_shortens_stuns_fears_and_slows() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, spec(|s| s.status_resist = 5_000), 10);
    for kind in [
        ModifierKind::Stunned,
        ModifierKind::Feared,
        ModifierKind::Slowed { pct: 30 },
    ] {
        world.put_modifier(
            hero,
            Modifier {
                kind,
                source: None,
                ticks_left: Some(10),
            },
        );
        assert_eq!(left_on(&world, hero, kind), Some(5), "{kind:?}");
        world.take_modifier(hero, kind, None);
    }
    world.put_modifier(
        hero,
        Modifier {
            kind: ModifierKind::Haste { speed: 10 },
            source: None,
            ticks_left: Some(10),
        },
    );
    assert_eq!(
        left_on(&world, hero, ModifierKind::Haste { speed: 10 }),
        Some(10),
        "a buff is not a disable"
    );
}

#[test]
fn status_resistance_never_shortens_a_disable_below_one_tick() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, spec(|s| s.status_resist = 9_000), 10);
    world.put_modifier(hero, stun(3));
    assert_eq!(left_on(&world, hero, ModifierKind::Stunned), Some(1));
}

#[test]
fn status_resistance_shortens_the_stretch_extended_by_once() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, spec(|s| s.status_resist = 5_000), 10);
    world.put_modifier(hero, stun(10));
    assert_eq!(left_on(&world, hero, ModifierKind::Stunned), Some(5));
    world.extend_modifier(hero, stun(10), 100);
    assert_eq!(
        left_on(&world, hero, ModifierKind::Stunned),
        Some(10),
        "what is already held is not shortened a second time"
    );
}

#[test]
fn neutral_rates_leave_costs_and_cooldowns_alone() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, ModifierSpec::NOMINAL, 10);
    assert_eq!(
        world.ability_mana_cost(hero, ability::FRENZY, 1),
        ability_mana_cost(ability::FRENZY, 1)
    );
    assert_eq!(
        world.ability_cooldown(hero, ability::FRENZY, 1),
        ability_cooldown(ability::FRENZY, 1)
    );
}

#[test]
fn mana_cost_rate_is_charged_and_shown() {
    let (mut world, hero, _creep) = arena();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].level = 1;
    }
    apply(&mut world, hero, spec(|s| s.mana_cost_rate = 5_000), 10);
    let base = ability_mana_cost(ability::FRENZY, 1);
    assert_eq!(base, rules::SYLLA_FRENZY_MANA[0]);
    assert_eq!(world.ability_mana_cost(hero, ability::FRENZY, 1), base / 2);
    let view = world.view_full();
    let unit = view
        .units
        .iter()
        .find(|unit| unit.id == wire_id(hero))
        .expect("the hero is in the view");
    assert_eq!(
        unit.abilities[1].mana_cost,
        base / 2,
        "the projection shows what a cast pays"
    );
    let before = world.mana.get(hero).expect("has mana").mana;
    assert!(world.begin_ability(hero, AbilitySlot(1), Target::None));
    assert_eq!(
        before - world.mana.get(hero).expect("has mana").mana,
        Fixed::from_int(base / 2),
        "the cast charged the scaled cost"
    );
}

#[test]
fn cooldown_rate_shortens_what_a_cast_sets() {
    let (mut world, hero, _creep) = arena();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].level = 1;
    }
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    assert!(world.begin_ability(hero, AbilitySlot(1), Target::None));
    assert_eq!(
        world
            .abilities
            .get(hero)
            .and_then(|book| book.slots.get(1))
            .map(|slot| slot.cooldown),
        Some(ability_cooldown(ability::FRENZY, 1) / 2)
    );
}

#[test]
fn cooldown_rate_shortens_what_an_item_use_sets() {
    let (mut world, hero, _creep) = arena();
    give(&mut world, hero, crate::game::ITEM_MAGIC_STICK);
    if let Some(stack) = world
        .inventory
        .get_mut(hero)
        .and_then(|bag| bag.slots[0].as_mut())
    {
        stack.charges = 1;
    }
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    assert!(world.begin_item(hero, 0, Target::None));
    let full = ITEMS[usize::from(crate::game::ITEM_MAGIC_STICK)].cooldown;
    assert_eq!(
        world
            .inventory
            .get(hero)
            .and_then(|bag| bag.slots[0])
            .map(|stack| stack.cooldown),
        Some(full / 2)
    );
}

#[test]
fn a_shared_item_wait_is_scaled_with_the_cooldown_rate() {
    let (mut world, hero, _creep) = arena();
    world.spawn_building(
        crate::game::tower_def(1),
        Team::Radiant,
        Vec2::from_ints(7300, 7000),
        crate::game::Place::Tower { lane: 0, tier: 1 },
    );
    let mut seat = crate::game::Seat::new(
        SlotId(0),
        Team::Radiant,
        HeroId(0),
        rules::STARTING_GOLD,
        rules::STASH_SLOTS,
    );
    seat.unit = Some(hero);
    world.seats.push(seat);
    give(&mut world, hero, crate::game::ITEM_TOWN_PORTAL_SCROLL);
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    let at = world.transform.get(hero).expect("standing").pos;
    assert!(
        world.begin_item(hero, 0, Target::Pos(at)),
        "the scroll reads beside the tower"
    );
    assert_eq!(
        world.seats[0].item_clocks,
        vec![(
            ItemId(crate::game::ITEM_TOWN_PORTAL_SCROLL),
            rules::SCROLL_WAIT_TICKS / 2
        )],
        "the wait on the kind of item is halved"
    );
    assert_eq!(
        world
            .inventory
            .get(hero)
            .and_then(|bag| bag.slots[0])
            .map(|stack| stack.cooldown),
        Some(0),
        "a shared wait lives on the seat, not on the stack"
    );
}

#[test]
fn a_blow_sets_a_muted_item_back_by_the_cooldown_rate() {
    let (mut world, hero, _creep) = arena();
    give(&mut world, hero, crate::game::ITEM_BLINK_DAGGER);
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    let hitter = world.spawn();
    world.kind.insert(hitter, UnitKind::Hero);
    world.push_hit(Some(hitter), hero, 10, DamageKind::Physical);
    world.step();
    let full = ITEMS[usize::from(crate::game::ITEM_BLINK_DAGGER)].breaks_on_damage;
    assert_eq!(
        world
            .inventory
            .get(hero)
            .and_then(|bag| bag.slots[0])
            .map(|stack| stack.cooldown),
        Some(full / 2)
    );
}

#[test]
fn a_cheat_modifier_is_not_an_effect_on_the_wire() {
    let (mut world, hero, _creep) = arena();
    let effects = |world: &World| {
        world
            .view_full()
            .units
            .into_iter()
            .find(|unit| unit.id == wire_id(hero))
            .expect("the hero is in the view")
            .effects
    };
    let before = effects(&world);
    apply(&mut world, hero, spec(|s| s.status_resist = 1_000), 10);
    assert_eq!(effects(&world), before, "nothing new shows on the unit");
}

#[test]
fn nothing_a_normal_modifier_does_reaches_a_cheat_modifier() {
    let (mut world, hero, _creep) = arena();
    apply(&mut world, hero, spec(|s| s.magic_resist = 1_000), 100);
    world.put_modifier(hero, stun(10));
    world.take_modifier(hero, ModifierKind::Stunned, None);
    if let Some(on_it) = world.modifiers.get_mut(hero) {
        // A purge of everything the modifier system holds.
        on_it.0.clear();
    }
    for _ in 0..10 {
        world.tick_gear();
    }
    assert!(
        world.applied.contains(hero),
        "only the countdown may take it away"
    );
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.magic_resist_pct),
        Some(35),
        "and its stat still applies"
    );
}

/// A Pudge with Flesh Heap learned and souls in the heap.
fn pudge_with_a_heap() -> (World, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(7000, 7000),
        SlotId(0),
        HeroId(1),
    );
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    let mut stacks = Stacks::default();
    stacks.set(StackKind::FleshHeap, 10);
    world.stacks.insert(pudge, stacks);
    world.settle();
    (world, pudge)
}

#[test]
fn magic_resistance_adds_before_the_flesh_heap_multiplies() {
    let (mut world, pudge) = pudge_with_a_heap();
    let base = world.stats.get(pudge).expect("settled").magic_resist_pct;
    apply(&mut world, pudge, spec(|s| s.magic_resist = 5_000), 10);
    let before_heap = (rules::HERO_MAGIC_RESIST_PCT + 50).clamp(0, 100);
    let kept = (100 - before_heap) * (100 - rules::FLESH_HEAP_MAGIC_RESIST_PCT[0]) / 100;
    let folded_first = 100 - kept;
    assert_eq!(
        world.stats.get(pudge).map(|stats| stats.magic_resist_pct),
        Some(folded_first),
        "the heap multiplies what the modifier added"
    );
    assert!(
        folded_first < base + 50,
        "adding after the heap would give {}, which would let the modifier dodge it",
        base + 50
    );
}

#[test]
fn magic_resistance_is_clamped_before_the_flesh_heap_multiplies() {
    let (mut world, pudge) = pudge_with_a_heap();
    apply(
        &mut world,
        pudge,
        spec(|s| s.magic_resist = ModifierSpec::MAX_RESIST),
        10,
    );
    assert_eq!(
        world.stats.get(pudge).map(|stats| stats.magic_resist_pct),
        Some(100),
        "the added delta is clamped at immunity and stays there"
    );
}

#[test]
fn movement_speed_adds_before_items_and_slows() {
    let (mut world, hero, _creep) = arena();
    give(&mut world, hero, crate::game::ITEM_BOOTS);
    apply(&mut world, hero, spec(|s| s.move_speed = 15_000), 10);
    let carried = rules::HERO_MOVE_SPEED * 3 / 2 + 45;
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.move_speed.to_int()),
        Some(carried),
        "half again the base, then the boots"
    );
    world.put_modifier(
        hero,
        Modifier {
            kind: ModifierKind::Slowed { pct: 50 },
            source: None,
            ticks_left: Some(10),
        },
    );
    world.settle();
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.move_speed.to_int()),
        Some(carried / 2),
        "the slow multiplies what stands after both"
    );
}

#[test]
fn max_health_scales_the_raised_base_before_items() {
    let (mut world, hero, _creep) = arena();
    give(&mut world, hero, crate::game::ITEM_BRACER);
    apply(&mut world, hero, spec(|s| s.max_hp = 20_000), 10);
    let expected = rules::HERO_HP * 2 + rules::HP_PER_STRENGTH * 26;
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_hp.to_int()),
        Some(expected),
        "the base is doubled first, then the bracer's strength pays"
    );
}

#[test]
fn max_mana_scales_the_raised_base_before_items() {
    let (mut world, hero, _creep) = arena();
    give(&mut world, hero, crate::game::ITEM_NULL_TALISMAN);
    apply(&mut world, hero, spec(|s| s.max_mana = 20_000), 10);
    let expected = rules::HERO_MANA * 2 + rules::MANA_PER_INTELLIGENCE * 24;
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_mana.to_int()),
        Some(expected),
        "the base is doubled first, then the talisman's intelligence pays"
    );
}

#[test]
fn a_nominal_spec_leaves_the_new_stats_alone() {
    let (plain, hero, _creep) = arena();
    let (mut world, twin, _creep) = arena();
    apply(&mut world, twin, ModifierSpec::NOMINAL, 10);
    let plain = *plain.stats.get(hero).expect("settled");
    let scaled = *world.stats.get(twin).expect("settled");
    assert_eq!(plain.max_hp, scaled.max_hp);
    assert_eq!(plain.max_mana, scaled.max_mana);
    assert_eq!(plain.move_speed, scaled.move_speed);
}

#[test]
fn damage_amplification_multiplies_the_whole_damage_before_mitigation() {
    let (mut world, hero, creep) = arena();
    give(&mut world, hero, crate::game::ITEM_PHASE_BOOTS);
    apply(&mut world, hero, spec(|s| s.pure_damage = 15_000), 10);
    let damage = world.stats.get(hero).expect("settled").damage;
    assert!(
        damage > rules::HERO_ATTACK_DAMAGE,
        "the item is in the blow"
    );
    let expected = (i64::from(damage) * 15_000 / 10_000) as i32;
    assert_eq!(
        dealt(&mut world, hero, creep, damage, DamageKind::Pure),
        expected,
        "the whole blow, item damage included, is scaled before anything else"
    );
}

#[test]
fn a_full_pool_stays_full_when_a_modifier_raises_its_maximum() {
    let (mut world, _hero, creep) = arena();
    let base = world.stats.get(creep).expect("settled").max_hp;
    assert_eq!(
        world.health.get(creep).expect("standing").hp,
        base,
        "it starts full"
    );
    apply(&mut world, creep, spec(|s| s.max_hp = 20_000), 10);
    let scaled = base + base;
    assert_eq!(
        world.stats.get(creep).map(|stats| stats.max_hp),
        Some(scaled)
    );
    assert_eq!(
        world.health.get(creep).map(|health| health.hp),
        Some(scaled),
        "a full pool follows the maximum to full"
    );
}

#[test]
fn a_pool_keeps_its_fraction_through_raises_and_lifts() {
    let (mut world, _hero, creep) = arena();
    let base = world.stats.get(creep).expect("settled").max_hp;
    world.health.get_mut(creep).expect("standing").hp =
        base * Fixed::from_int(3) / Fixed::from_int(5);
    apply(&mut world, creep, spec(|s| s.max_hp = 20_000), 10);
    assert_eq!(
        world.stats.get(creep).map(|stats| stats.max_hp.to_int()),
        Some(rules::MELEE_CREEP_HP * 2),
        "the maximum doubles"
    );
    assert_eq!(
        world.health.get(creep).map(|health| health.hp.to_int()),
        Some(rules::MELEE_CREEP_HP * 2 * 3 / 5),
        "three fifths of it stays three fifths"
    );
    apply(&mut world, creep, spec(|s| s.max_hp = 6_000), 10);
    assert_eq!(
        world.stats.get(creep).map(|stats| stats.max_hp.to_int()),
        Some(rules::MELEE_CREEP_HP * 3 / 5),
        "a forty percent cut leaves three fifths of the base"
    );
    assert_eq!(
        world.health.get(creep).map(|health| health.hp.to_int()),
        Some(rules::MELEE_CREEP_HP * 3 / 5 * 3 / 5),
        "and the pool stays at its fraction of the new maximum"
    );
}

#[test]
fn a_pool_that_held_anything_stays_alive_when_the_maximum_shrinks() {
    let (mut world, _hero, creep) = arena();
    world.health.get_mut(creep).expect("standing").hp = Fixed::ONE;
    apply(&mut world, creep, spec(|s| s.max_hp = 2_500), 10);
    let held = world.health.get(creep).expect("standing").hp;
    assert!(
        held > Fixed::ZERO,
        "a point of health stays a point of health, not nothing"
    );
    assert!(
        held <= world.stats.get(creep).expect("settled").max_hp,
        "and never past the maximum"
    );
}

#[test]
fn a_mana_pool_keeps_its_fraction_too() {
    let (mut world, hero, _creep) = arena();
    let base = world.stats.get(hero).expect("settled").max_mana;
    world.mana.get_mut(hero).expect("has mana").mana = base / Fixed::from_int(2);
    apply(&mut world, hero, spec(|s| s.max_mana = 20_000), 10);
    let doubled = Fixed::from_int(rules::HERO_MANA * 2 + rules::MANA_PER_INTELLIGENCE * 18);
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_mana),
        Some(doubled),
        "the raised base doubles, intelligence pays on top"
    );
    assert_eq!(
        world.mana.get(hero).map(|mana| mana.mana),
        Some(doubled / Fixed::from_int(2)),
        "and the half-filled pool keeps its half"
    );
}

#[test]
fn a_creep_spawned_with_a_modifier_is_raised_with_it() {
    let mut world = World::new();
    let creep = world.spawn_creep(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1000, 1000), 0, 0);
    world
        .applied
        .insert(creep, one_applied(spec(|s| s.max_hp = 12_500), 100));
    world.settle();
    let max = rules::MELEE_CREEP_HP * 12_500 / 10_000;
    assert_eq!(
        world.stats.get(creep).map(|stats| stats.max_hp.to_int()),
        Some(max),
        "the first derive already carries it"
    );
    assert_eq!(
        world.health.get(creep).map(|health| health.hp.to_int()),
        Some(max),
        "it stands up full at its raised maximum"
    );
}

#[test]
fn a_tower_takes_the_same_maximum_health_modifier() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let tower = world
        .entities
        .iter()
        .find(|entity| world.kind.get(*entity) == Some(&UnitKind::Tower))
        .expect("the map stands towers");
    let base = world.stats.get(tower).expect("settled").max_hp;
    assert_eq!(
        world.health.get(tower).expect("standing").hp,
        base,
        "it stands full"
    );
    apply(&mut world, tower, spec(|s| s.max_hp = 15_000), 10);
    let expected = Fixed {
        raw: (i64::from(base.raw) * 15_000 / 10_000) as i32,
    };
    assert_eq!(
        world.stats.get(tower).map(|stats| stats.max_hp),
        Some(expected)
    );
    assert_eq!(
        world.health.get(tower).map(|health| health.hp),
        Some(expected),
        "a full tower follows the maximum to full"
    );
    let view = world.view_full();
    assert_eq!(
        view.units
            .iter()
            .find(|unit| unit.id == wire_id(tower))
            .map(|unit| unit.max_hp),
        Some(expected.to_int()),
        "and the projection shows it"
    );
}

#[test]
fn a_tower_spawned_with_a_modifier_is_raised_with_it() {
    let mut world = World::new();
    let def = crate::game::tower_def(1);
    let tower = world.spawn_building(
        def,
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        crate::game::Place::Tower { lane: 0, tier: 1 },
    );
    world
        .applied
        .insert(tower, one_applied(spec(|s| s.max_hp = 15_000), 100));
    world.settle();
    assert_eq!(
        world.stats.get(tower).map(|stats| stats.max_hp.to_int()),
        Some(def.max_hp * 15_000 / 10_000),
        "the first derive already carries it"
    );
}

#[test]
fn an_amplified_creep_deals_more_damage() {
    let blow = |modifier: ModifierSpec| {
        let mut world = World::new();
        let attacker = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(1000, 1000));
        let target = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(2000, 2000));
        world.settle();
        if !modifier.is_nominal() {
            apply(&mut world, attacker, modifier, 10);
        }
        dealt(&mut world, attacker, target, 100, DamageKind::Physical)
    };
    let neutral = blow(ModifierSpec::NOMINAL);
    let amplified = blow(spec(|s| s.physical_damage = 20_000));
    assert!(neutral > 0);
    assert_eq!(
        amplified,
        2 * neutral,
        "the attacking creep's own modifier scales its blow"
    );
}

/// A slow handed out by standing near something.
static SLOWING_AURA: [Aura; 1] = [Aura {
    kind: ModifierKind::Slowed { pct: 30 },
    radius: 400,
    reaches: Reach::All,
    ticks: 10,
}];

#[test]
fn an_aura_hands_out_a_slow_through_the_same_resistance() {
    let (mut world, hero, _creep) = arena();
    let source = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(7100, 7000));
    let plain = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(7200, 7000));
    world.auras.insert(source, Auras(&SLOWING_AURA));
    apply(&mut world, hero, spec(|s| s.status_resist = 5_000), 10);
    aura_system(AuraCx {
        entities: &world.entities,
        transform: &world.transform,
        team: &world.team,
        kind: &world.kind,
        auras: &world.auras,
        stats: &world.stats,
        modifiers: &mut world.modifiers,
    });
    assert_eq!(
        left_on(&world, hero, ModifierKind::Slowed { pct: 30 }),
        Some(5),
        "the resisting unit keeps half of it"
    );
    assert_eq!(
        left_on(&world, plain, ModifierKind::Slowed { pct: 30 }),
        Some(10),
        "and the plain one keeps all of it"
    );
}

#[test]
fn rate_scales_floor_only_after_the_rate_is_applied() {
    assert_eq!(
        cooldown_after(1, ModifierSpec::MIN_SCALE),
        1,
        "a cooldown that was set keeps a tick"
    );
    assert_eq!(
        cooldown_after(0, ModifierSpec::MAX_SCALE),
        0,
        "no cooldown stays none"
    );
    assert_eq!(cooldown_after(100, ModifierSpec::MIN_SCALE), 25);
    assert_eq!(
        cost_after(1, ModifierSpec::MIN_SCALE),
        0,
        "a cost may reach nothing"
    );
    assert_eq!(cost_after(100, 0), 0, "a zero rate is nothing");
}
