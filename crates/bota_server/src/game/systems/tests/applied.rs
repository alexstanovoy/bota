//! Applied stat changes: what each is worth and what it ignores.

use bota_core::{
    Aura, ITEMS, MELEE_CREEP, ModifierKind, Reach, SHADOW_FIEND, ability_cooldown,
    ability_mana_cost, map_of, rules, tower_def,
};

use crate::game::{
    AppliedModifier, AppliedModifiers, AppliedOrigin, AuraCx, Auras, Entity,
    MAX_APPLIED_MODIFIERS_PER_UNIT, MAX_COMBINED_MODIFIER_SCALE, Modifier, Place, Seat, Spots,
    StackKind, Stacks, World, aura_system, cooldown_after, cost_after, wire_id,
};
use bota_proto::{
    AbilityId, AbilitySlot, DamageKind, Fixed, HeroId, ItemId, MapId, ModifierSpec, SlotId, Target,
    Team, UnitKind, Vec2,
};

use super::support::{hold, spec};

/// A world with a hero and a creep standing well apart.
fn arena() -> (World, Entity, Entity) {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(7000, 7000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1000, 1000));
    world.settle();
    (world, hero, creep)
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

/// A requiem line in flight at a creep, its source carrying double
/// amplification.
fn amplified_requiem(ticks: u32) -> (World, Entity, Entity) {
    let mut world = World::new();
    let caster = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let target = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    apply(&mut world, caster, spec(|s| s.magic_damage = 20_000), ticks);
    let mut kept = world.stacks.get(caster).copied().unwrap_or_default();
    kept.set(StackKind::Souls, 1);
    world.stacks.insert(caster, kept);
    assert!(world.cast_requiem(caster, 0));
    let line = world
        .entities
        .iter()
        .find_map(|entity| world.requiem_line.get(entity))
        .expect("the line is in flight");
    assert_eq!(line.damage_amp_bp, 20_000);
    (world, caster, target)
}

/// Runs one released line to the creep and returns the health it took.
fn land_requiem(world: &mut World, target: Entity) -> i32 {
    let before = world.health.get(target).expect("standing").hp;
    for _ in 0..64 {
        world.step();
        let after = world.health.get(target).expect("survives").hp;
        if after < before {
            return (before - after).to_int();
        }
    }
    panic!("the line did not reach the creep inside its bounded flight")
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

#[test]
fn every_combined_modifier_family_has_an_explicit_bound() {
    let top = MAX_COMBINED_MODIFIER_SCALE;
    let resist = ModifierSpec::MAX_RESIST;
    for (scale, resist, pct, amp, rate, bounty) in [
        (
            ModifierSpec::MAX_SCALE,
            resist,
            rules::NOMINAL_BP,
            top,
            top,
            100 * top / rules::NOMINAL_BP,
        ),
        (ModifierSpec::MIN_SCALE, -resist, 0, 0, 1, 0),
    ] {
        let (mut world, hero, _creep) = arena();
        let extreme = extreme_spec(scale, resist, ModifierSpec::MAX_STATUS_RESIST);
        world.applied.insert(hero, full_applied(extreme));
        world.settle();
        let stats = *world.stats.get(hero).expect("settled");
        assert_eq!(stats.magic_resist_bp, pct, "scale {scale}");
        assert_eq!(
            stats.status_resist_bp,
            rules::NOMINAL_BP - 1,
            "scale {scale}"
        );
        assert_eq!(
            [stats.physical_amp_bp, stats.magic_amp_bp, stats.pure_amp_bp],
            [amp; 3],
            "scale {scale}"
        );
        assert_eq!(
            [stats.cooldown_rate_bp, stats.mana_cost_rate_bp],
            [rate; 2],
            "scale {scale}"
        );
        assert_eq!(
            stats.move_speed.to_int(),
            SHADOW_FIEND.move_speed * scale / rules::NOMINAL_BP,
            "world magnitudes saturate at one source's bound"
        );
        assert_eq!(
            stats.max_hp.to_int(),
            SHADOW_FIEND.max_hp * scale / rules::NOMINAL_BP + rules::HP_PER_STRENGTH * 19
        );
        assert_eq!(
            stats.max_mana.to_int(),
            SHADOW_FIEND.max_mana * scale / rules::NOMINAL_BP + rules::MANA_PER_INTELLIGENCE * 16
        );
        assert_eq!(
            world.bounty_after(Some(hero), 100),
            bounty,
            "bounty keeps every bounded additive source"
        );
        assert_eq!(
            world.applied.get(hero).map(|held| held.iter().count()),
            Some(MAX_APPLIED_MODIFIERS_PER_UNIT)
        );
    }
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
fn damage_modifiers_scale_and_resist_only_their_own_kind() {
    use DamageKind::{Magical, Physical, Pure};
    let (mut world, hero, creep) = arena();
    let physical = dealt(&mut world, hero, creep, 100, Physical);
    assert!(physical > 0, "armor leaves something of a physical blow");
    let nominal = ModifierSpec::NOMINAL;
    let half_resist = spec(|s| s.magic_resist = 5_000);
    let immune = spec(|s| s.magic_resist = 10_000);
    let magic_x1_5 = spec(|s| s.magic_damage = 15_000);
    let magic_x2 = spec(|s| s.magic_damage = 20_000);
    let physical_x2 = spec(|s| s.physical_damage = 20_000);
    let pure_x1_5 = spec(|s| s.pure_damage = 15_000);
    for (attacker, defender, kind, expected, case) in [
        (nominal, nominal, Pure, 100, "nominal pure"),
        (nominal, nominal, Magical, 100, "nominal magical"),
        (nominal, half_resist, Magical, 50, "resisted magical"),
        (nominal, half_resist, Pure, 100, "resistance spares pure"),
        (nominal, immune, Magical, 0, "total immunity"),
        (magic_x1_5, nominal, Magical, 150, "amplified magical"),
        (
            magic_x2,
            half_resist,
            Magical,
            100,
            "doubled first, then half kept",
        ),
        (
            physical_x2,
            nominal,
            Physical,
            2 * physical,
            "amplified physical",
        ),
        (
            physical_x2,
            nominal,
            Magical,
            100,
            "physical amp spares magical",
        ),
        (physical_x2, nominal, Pure, 100, "physical amp spares pure"),
        (pure_x1_5, nominal, Pure, 150, "amplified pure"),
    ] {
        let (mut world, hero, creep) = arena();
        apply(&mut world, hero, attacker, 10);
        apply(&mut world, creep, defender, 10);
        assert_eq!(
            dealt(&mut world, hero, creep, 100, kind),
            expected,
            "{case}"
        );
    }
}

#[test]
fn a_requiem_line_keeps_its_launch_amplification_after_the_modifier_lifts() {
    let (mut world, caster, target) = amplified_requiem(1);
    world.step();
    assert!(world.alive(caster));
    assert!(!world.applied.contains(caster), "the modifier has run out");
    assert_eq!(
        land_requiem(&mut world, target),
        rules::REQUIEM_LINE_DAMAGE[0] * 2
    );
}

#[test]
fn a_requiem_line_keeps_its_launch_amplification_when_the_source_slot_is_reused() {
    let (mut world, caster, target) = amplified_requiem(100);
    world.push_hit(None, caster, 1_000_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(caster));
    // Near enough for its side to see the creep the line is flying at.
    let replacement = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5000, 4700));
    assert_eq!(
        replacement.index(),
        caster.index(),
        "the free slot is reused"
    );
    assert_ne!(replacement.generation(), caster.generation());
    world.settle();
    assert_eq!(
        land_requiem(&mut world, target),
        rules::REQUIEM_LINE_DAMAGE[0] * 2
    );
}

#[test]
fn status_resistance_shortens_a_stun_but_never_below_one_tick() {
    for (resist, ticks, left) in [(0, 10, 10), (5_000, 10, 5), (9_000, 3, 1)] {
        let (mut world, hero, _creep) = arena();
        apply(&mut world, hero, spec(|s| s.status_resist = resist), 10);
        world.put_modifier(hero, stun(ticks));
        assert_eq!(
            left_on(&world, hero, ModifierKind::Stunned),
            Some(left),
            "{resist} bp on {ticks} ticks"
        );
    }
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
            kind: ModifierKind::Hastened { pct: 10 },
            source: None,
            ticks_left: Some(10),
        },
    );
    assert_eq!(
        left_on(&world, hero, ModifierKind::Hastened { pct: 10 }),
        Some(10),
        "a buff is not a disable"
    );
}

#[test]
fn a_nominal_spec_changes_no_stat_cost_or_cooldown() {
    let (plain, hero, _creep) = arena();
    let (mut world, twin, _creep) = arena();
    apply(&mut world, twin, ModifierSpec::NOMINAL, 10);
    assert_eq!(plain.stats.get(hero), world.stats.get(twin));
    assert_eq!(
        world.ability_mana_cost(twin, AbilityId::RAZE_NEAR, 1),
        ability_mana_cost(AbilityId::RAZE_NEAR, 1)
    );
    assert_eq!(
        world.ability_cooldown(twin, AbilityId::RAZE_NEAR, 1),
        ability_cooldown(AbilityId::RAZE_NEAR, 1)
    );
}

#[test]
fn cost_and_cooldown_rates_scale_what_a_cast_charges_shows_and_sets() {
    let (mut world, hero, _creep) = arena();
    world.abilities.get_mut(hero).expect("has a book").slots[0].level = 1;
    let rates = spec(|s| {
        s.mana_cost_rate = 5_000;
        s.cooldown_rate = 6_000;
    });
    apply(&mut world, hero, rates, 10);
    let base = ability_mana_cost(AbilityId::RAZE_NEAR, 1);
    assert_eq!(base, rules::RAZE_MANA[0]);
    assert_eq!(
        world.ability_mana_cost(hero, AbilityId::RAZE_NEAR, 1),
        base / 2
    );
    let view = world.view_full();
    let unit = view
        .units
        .iter()
        .find(|unit| unit.id == wire_id(hero))
        .expect("the hero is in the view");
    assert_eq!(
        unit.abilities[0].mana_cost,
        base / 2,
        "the projection shows what a cast pays"
    );
    let before = world.mana.get(hero).expect("has mana").mana;
    assert!(world.begin_ability(hero, AbilitySlot(0), Target::None));
    assert_eq!(
        before - world.mana.get(hero).expect("has mana").mana,
        Fixed::from_int(base / 2),
        "the cast charged the scaled cost"
    );
    assert_eq!(
        world.abilities.get(hero).map(|book| book.slots[0].cooldown),
        Some(ability_cooldown(AbilityId::RAZE_NEAR, 1) * 3 / 5),
        "and set the scaled cooldown"
    );
}

#[test]
fn cooldown_rate_shortens_what_an_item_use_sets() {
    let (mut world, hero, _creep) = arena();
    hold(&mut world, hero, 0, ItemId::MAGIC_STICK);
    if let Some(stack) = world
        .inventory
        .get_mut(hero)
        .and_then(|bag| bag.slots[0].as_mut())
    {
        stack.charges = 1;
    }
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    assert!(world.begin_item(hero, 0, Target::None));
    let full = ITEMS[usize::from(ItemId::MAGIC_STICK.0)].cooldown;
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
        tower_def(1),
        Team::Radiant,
        Vec2::from_ints(7300, 7000),
        Place::Tower { lane: 0, tier: 1 },
    );
    let mut seat = Seat::new(
        SlotId(0),
        Team::Radiant,
        HeroId::SHADOW_FIEND,
        rules::STARTING_GOLD,
        rules::STASH_SLOTS,
    );
    seat.unit = Some(hero);
    world.seats.push(seat);
    hold(&mut world, hero, 0, ItemId::TOWN_PORTAL_SCROLL);
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    let at = world.transform.get(hero).expect("standing").pos;
    assert!(
        world.begin_item(hero, 0, Target::Pos(at)),
        "the scroll reads beside the tower"
    );
    assert_eq!(
        world.seats[0].item_clocks,
        vec![(ItemId::TOWN_PORTAL_SCROLL, rules::SCROLL_WAIT_TICKS / 2)],
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
    hold(&mut world, hero, 0, ItemId::BLINK_DAGGER);
    apply(&mut world, hero, spec(|s| s.cooldown_rate = 5_000), 10);
    let hitter = world.spawn();
    world.kind.insert(hitter, UnitKind::Hero);
    world.push_hit(Some(hitter), hero, 10, DamageKind::Physical);
    world.step();
    let full = ITEMS[usize::from(ItemId::BLINK_DAGGER.0)].breaks_on_damage;
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
fn an_applied_modifier_is_not_an_effect_on_the_wire() {
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
fn nothing_a_normal_modifier_does_reaches_an_applied_modifier() {
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
        world.stats.get(hero).map(|stats| stats.magic_resist_bp),
        Some(3_500),
        "and its stat still applies"
    );
}

/// A Pudge with Flesh Heap learned and stacks in the heap.
fn pudge_with_a_heap() -> (World, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(7000, 7000),
        SlotId(0),
        HeroId::PUDGE,
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
    let base = world.stats.get(pudge).expect("settled").magic_resist_bp;
    apply(&mut world, pudge, spec(|s| s.magic_resist = 5_000), 10);
    let before_heap = (rules::HERO_MAGIC_RESIST_PCT * 100 + 5_000).clamp(0, rules::NOMINAL_BP);
    let kept =
        (rules::NOMINAL_BP - before_heap) * (100 - rules::FLESH_HEAP_MAGIC_RESIST_PCT[0]) / 100;
    let folded_first = rules::NOMINAL_BP - kept;
    assert_eq!(
        world.stats.get(pudge).map(|stats| stats.magic_resist_bp),
        Some(folded_first),
        "the heap multiplies what the modifier added"
    );
    assert!(
        folded_first < base + 5_000,
        "adding after the heap would give {}, which would let the modifier dodge it",
        base + 5_000
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
        world.stats.get(pudge).map(|stats| stats.magic_resist_bp),
        Some(rules::NOMINAL_BP),
        "the added delta is clamped at immunity and stays there"
    );
}

#[test]
fn movement_speed_adds_before_items_and_slows() {
    let (mut world, hero, _creep) = arena();
    hold(&mut world, hero, 0, ItemId::BOOTS);
    apply(&mut world, hero, spec(|s| s.move_speed = 15_000), 10);
    let carried = SHADOW_FIEND.move_speed * 3 / 2 + 45;
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
    hold(&mut world, hero, 0, ItemId::BRACER);
    apply(&mut world, hero, spec(|s| s.max_hp = 20_000), 10);
    let expected = SHADOW_FIEND.max_hp * 2 + rules::HP_PER_STRENGTH * 24 + 50;
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_hp.to_int()),
        Some(expected),
        "the base is doubled first, then the bracer's strength and health pay"
    );
}

#[test]
fn max_mana_scales_the_raised_base_before_items() {
    let (mut world, hero, _creep) = arena();
    hold(&mut world, hero, 0, ItemId::NULL_TALISMAN);
    apply(&mut world, hero, spec(|s| s.max_mana = 20_000), 10);
    let expected = SHADOW_FIEND.max_mana * 2 + rules::MANA_PER_INTELLIGENCE * 22;
    assert_eq!(
        world.stats.get(hero).map(|stats| stats.max_mana.to_int()),
        Some(expected),
        "the base is doubled first, then the talisman's intelligence pays"
    );
}

#[test]
fn damage_amplification_multiplies_the_whole_damage_before_mitigation() {
    let (mut world, hero, creep) = arena();
    hold(&mut world, hero, 0, ItemId::PHASE_BOOTS);
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
    let doubled = Fixed::from_int(SHADOW_FIEND.max_mana * 2 + rules::MANA_PER_INTELLIGENCE * 16);
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
    let mut world = World::on_map(map_of(MapId(0)));
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
    let def = tower_def(1);
    let tower = world.spawn_building(
        def,
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        Place::Tower { lane: 0, tier: 1 },
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
        upgrades: &world.upgrades,
        stats: &world.stats,
        modifiers: &mut world.modifiers,
        spots: &mut Spots::default(),
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
