//! What a neutral's passives and the statuses they lay do, played out with
//! real camps against a hero.

use bota_core::{CAMPS, ModifierKind, NeutralAbility, NeutralKind, mitigate, rules};
use bota_proto::{DamageKind, EventKind, Fixed, ItemId, Team, Vec2};

use crate::game::{Entity, Modifier, UnitOrder, World, wire_id};

use super::support::*;

/// A spot on open ground, `dx` east of the fight.
fn at(dx: i32) -> Vec2 {
    Vec2::from_ints(5000 + dx, 5000)
}

/// A world in the first minutes of the day whose heroes do not fall.
fn day() -> World {
    let mut world = World::new();
    world.tick = rules::PREGAME_TICKS;
    world.immortal_heroes = true;
    world
}

/// A neutral of a kind standing at a spot, awake.
fn awake(world: &mut World, kind: NeutralKind, pos: Vec2) -> Entity {
    let beast = world.spawn_neutral(kind, pos, 0, 0);
    if let Some(ai) = world.neutral_ai.get_mut(beast) {
        ai.awake = true;
    }
    beast
}

/// A hero that takes nothing on of its own accord.
fn passive_hero(world: &mut World, pos: Vec2) -> Entity {
    let hero = hero_at(world, pos);
    world.set_order(hero, UnitOrder::Stand);
    hero
}

/// Whether a unit wears the modifier a neutral ability left.
fn wears(world: &World, unit: Entity, ability: NeutralAbility) -> bool {
    world.modifiers.get(unit).is_some_and(|on_it| {
        on_it.active().any(
            |held| matches!(held.kind, ModifierKind::Spell { ability: it, .. } if it == ability),
        )
    })
}

/// How many modifiers a neutral ability left on a unit.
fn worn(world: &World, unit: Entity, ability: NeutralAbility) -> usize {
    world.modifiers.get(unit).map_or(0, |on_it| {
        on_it
            .active()
            .filter(
                |held| matches!(held.kind, ModifierKind::Spell { ability: it, .. } if it == ability),
            )
            .count()
    })
}

/// Puts a plain status on a unit for so many ticks.
fn afflict(world: &mut World, unit: Entity, kind: ModifierKind, ticks: u32) {
    world.put_modifier(unit, Modifier::new(kind, None, Some(ticks)));
}

/// The physical and magical blows one unit dealt another over a run, as
/// `(tick, amount, kind)`.
fn blows(world: &mut World, ticks: u32, from: Entity, on: Entity) -> Vec<(u32, i32, DamageKind)> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        for event in world.step() {
            if let EventKind::Damaged {
                source,
                target,
                amount,
                kind,
            } = event.kind
                && source == Some(wire_id(from))
                && target == wire_id(on)
            {
                seen.push((world.tick, amount, kind));
            }
        }
    }
    seen
}

fn hp(world: &World, unit: Entity) -> Fixed {
    world.health.get(unit).expect("alive").hp
}

fn speed(world: &World, unit: Entity) -> Fixed {
    world.stats.get(unit).expect("derived").move_speed
}

#[test]
fn an_aura_reaches_allies_as_far_as_a_neutral_camp_aura_does_and_never_stacks() {
    let mut world = day();
    let foreman = world.spawn_neutral(NeutralKind::KoboldTaskmaster, at(0), 0, 0);
    let near = world.spawn_neutral(
        NeutralKind::Kobold,
        at(rules::NEUTRAL_AURA_RADIUS - 100),
        0,
        0,
    );
    let far = world.spawn_neutral(
        NeutralKind::Kobold,
        at(rules::NEUTRAL_AURA_RADIUS + 100),
        0,
        0,
    );
    let second = world.spawn_neutral(NeutralKind::KoboldTaskmaster, at(-100), 0, 0);
    world.settle();
    world.step();
    let base = Fixed::from_int(NeutralKind::Kobold.def().move_speed);
    assert!(
        (speed(&world, near).raw - (base * Fixed::from_ratio(112, 100)).raw).abs() < 100,
        "inside the camp cap a speed aura adds its twelve percent, though its own radius is 1200"
    );
    assert_eq!(speed(&world, far), base, "and 900 out it adds nothing");
    assert_eq!(
        worn(&world, near, NeutralAbility::SpeedAura),
        1,
        "two foremen hand out one modifier between them"
    );
    assert_eq!(
        speed(&world, foreman),
        speed(&world, second),
        "and each wears it too"
    );
}

#[test]
fn a_broken_bearer_hands_nothing_out() {
    let mut world = day();
    let foreman = world.spawn_neutral(NeutralKind::KoboldTaskmaster, at(0), 0, 0);
    let ally = world.spawn_neutral(NeutralKind::Kobold, at(200), 0, 0);
    afflict(&mut world, foreman, ModifierKind::Broken, 600);
    world.settle();
    world.step();
    assert!(
        !wears(&world, ally, NeutralAbility::SpeedAura),
        "its aura is off while it is broken"
    );
    run(&mut world, 600);
    world.step();
    assert!(
        wears(&world, ally, NeutralAbility::SpeedAura),
        "and back on once the break lifts"
    );
}

#[test]
fn an_aura_of_the_enemy_side_reaches_the_hero_and_dragonhide_stacks() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    world.spawn_neutral(NeutralKind::RockGolem, at(500), 0, 0);
    world.spawn_neutral(NeutralKind::RockGolem, at(600), 0, 0);
    let dragons: Vec<Entity> = (0..2)
        .map(|n| world.spawn_neutral(NeutralKind::BlackDragon, at(1500 + 100 * n), 0, 0))
        .collect();
    world.settle();
    world.fill_pools(hero);
    let bare = world.stats.get(hero).expect("derived").armor;
    world.step();
    assert_eq!(
        world.stats.get(hero).expect("derived").armor,
        bare - Fixed::from_int(3),
        "two weakening auras take one aura's three armor"
    );
    assert_eq!(worn(&world, hero, NeutralAbility::DragonhideAura), 0);
    let plain = world.stats.get(dragons[0]).expect("derived").armor;
    let def = NeutralKind::BlackDragon.def();
    assert_eq!(
        plain,
        Fixed::from_int(def.armor) + Fixed::from_int(6),
        "each dragon's hide counts"
    );
}

#[test]
fn rally_adds_two_damage_for_every_skeleton_standing_together() {
    let mut world = day();
    let skeletons: Vec<Entity> = (0..3)
        .map(|n| world.spawn_neutral(NeutralKind::SkeletonWarrior, at(60 * n), 0, 0))
        .collect();
    world.settle();
    world.step();
    let base = NeutralKind::SkeletonWarrior.def().damage;
    assert_eq!(
        world.stats.get(skeletons[0]).expect("derived").damage,
        base + 6,
        "three skeletons: each wears three ralliers"
    );
    world.despawn(skeletons[2]);
    run(&mut world, 1 + rules::AURA_LINGER_TICKS);
    assert_eq!(
        world.stats.get(skeletons[0]).expect("derived").damage,
        base + 4,
        "one fewer takes its two away once the linger is over"
    );
}

#[test]
fn riverborn_works_only_for_a_bearer_standing_in_the_water() {
    let wet = CAMPS
        .iter()
        .find(|camp| camp.flooded)
        .expect("a flooded camp")
        .pos;
    let dry = CAMPS
        .iter()
        .find(|camp| !camp.flooded)
        .expect("a dry camp")
        .pos;
    let mut world = day();
    let frog = world.spawn_neutral(NeutralKind::Tadpole, wet, 0, 0);
    let friend = world.spawn_neutral(NeutralKind::Kobold, wet + Vec2::from_ints(150, 0), 0, 0);
    world.settle();
    world.step();
    assert!(
        wears(&world, friend, NeutralAbility::RiverbornAura),
        "in the stream"
    );
    world.transform.get_mut(frog).expect("standing").pos = dry;
    world.transform.get_mut(friend).expect("standing").pos = dry + Vec2::from_ints(150, 0);
    run(&mut world, 1 + rules::AURA_LINGER_TICKS);
    assert!(
        !wears(&world, friend, NeutralAbility::RiverbornAura),
        "on dry ground it does nothing"
    );
}

#[test]
fn a_kobold_soldier_disarms_on_the_third_blow_then_waits_nine_seconds() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let soldier = awake(&mut world, NeutralKind::KoboldTunneler, at(100));
    world.settle();
    let mut landed = 0;
    let mut disarmed_at = None;
    for _ in 0..600 {
        landed += blows(&mut world, 1, soldier, hero).len();
        if world.disarmed(hero) {
            disarmed_at = Some(world.tick);
            break;
        }
    }
    let first = disarmed_at.expect("the soldier disarmed the hero");
    assert_eq!(landed, 3, "on the third blow, not before");
    // A disarmed hero cannot begin a swing, though it is told to.
    let health = hp(&world, soldier);
    world.set_order(
        hero,
        UnitOrder::Attack {
            target: soldier,
            last_seen: at(100),
        },
    );
    run(&mut world, 60);
    assert!(world.disarmed(hero));
    assert_eq!(
        hp(&world, soldier),
        health,
        "it does not swing while disarmed"
    );
    // Three more blows inside the wait disarm nobody.
    let mut again = 0;
    while world.tick < first + rules::TICKS_PER_SECOND * 9 - 1 {
        world.step();
        again += usize::from(world.disarmed(hero) && world.tick > first + 100);
    }
    assert_eq!(again, 0, "off its wait the soldier counts and does nothing");
    run(&mut world, 200);
    assert!(
        world.tick - first >= rules::TICKS_PER_SECOND * 9,
        "the second disarm needs the wait to have run out"
    );
}

#[test]
fn a_disarmed_neutral_takes_nothing_on_and_stands() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let beast = awake(&mut world, NeutralKind::Kobold, at(150));
    afflict(&mut world, beast, ModifierKind::Disarmed, 90);
    world.settle();
    run(&mut world, 60);
    assert_eq!(world.target_of(beast), None, "it cannot be aggroed");
    assert_eq!(
        hp(&world, hero),
        world.stats.get(hero).expect("derived").max_hp
    );
    run(&mut world, 60);
    assert_eq!(
        world.target_of(beast),
        Some(hero),
        "and is itself again after"
    );
}

#[test]
fn a_breaking_blow_breaks_for_three_seconds_and_waits_ten() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let troll = awake(&mut world, NeutralKind::ForestTrollBerserker, at(300));
    world.settle();
    let mut broke = None;
    for _ in 0..300 {
        world.step();
        if world.broken(hero) {
            broke = Some(world.tick);
            break;
        }
    }
    let first = broke.expect("the berserker's missile broke the hero");
    let _ = troll;
    run(&mut world, 3 * rules::TICKS_PER_SECOND + 1);
    assert!(!world.broken(hero), "the break lifts after three seconds");
    run(&mut world, 3 * rules::TICKS_PER_SECOND);
    assert!(
        !world.broken(hero),
        "and does not come back inside the wait"
    );
    run(&mut world, 5 * rules::TICKS_PER_SECOND);
    assert!(
        world.broken(hero) || world.tick >= first + 10 * rules::TICKS_PER_SECOND,
        "only after ten seconds can it break again"
    );
}

#[test]
fn the_alpha_wolf_crits_one_blow_in_five() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let wolf = awake(&mut world, NeutralKind::AlphaWolf, at(100));
    world.settle();
    let armor = world.stats.get(hero).expect("derived").armor;
    let mut seen = Vec::new();
    for _ in 0..1800 {
        world.fill_pools(hero);
        seen.extend(blows(&mut world, 1, wolf, hero));
    }
    let loud = mitigate(48, DamageKind::Physical, armor, 0);
    let strikes: Vec<bool> = seen
        .iter()
        .filter(|(_, _, kind)| *kind == DamageKind::Physical)
        .map(|(_, amount, _)| *amount > loud)
        .collect();
    assert!(strikes.len() >= 25, "enough swings: {}", strikes.len());
    assert!(strikes.iter().any(|crit| *crit), "some blows are doubled");
    for window in strikes.windows(10) {
        assert!(
            window.iter().any(|crit| *crit),
            "a crit chance held exactly leaves no ten swings without one"
        );
    }
    let crits = strikes.iter().filter(|crit| **crit).count();
    assert!(
        crits * 5 <= strikes.len() + 5 && crits * 5 + 5 >= strikes.len(),
        "about one in five: {crits} of {}",
        strikes.len()
    );
}

#[test]
fn a_dragons_blow_splashes_the_ones_beside_the_struck_and_no_further() {
    let mut world = day();
    let struck = passive_hero(&mut world, at(0));
    let beside = world.spawn_hero(
        Team::Radiant,
        at(120),
        bota_proto::SlotId(1),
        bota_proto::HeroId::SHADOW_FIEND,
    );
    world.set_order(beside, UnitOrder::Stand);
    let away = world.spawn_hero(
        Team::Radiant,
        at(900),
        bota_proto::SlotId(2),
        bota_proto::HeroId::SHADOW_FIEND,
    );
    world.set_order(away, UnitOrder::Stand);
    let dragon = awake(&mut world, NeutralKind::BlackDragon, at(-300));
    world.set_target(dragon, struck);
    world.settle();
    world.fill_pools(struck);
    world.fill_pools(beside);
    world.fill_pools(away);
    let full = hp(&world, struck);
    let swung = blows(&mut world, 240, dragon, struck);
    assert!(!swung.is_empty(), "the dragon landed blows");
    let lost = |unit| full - hp(&world, unit);
    assert!(lost(struck) > Fixed::ZERO);
    assert!(
        lost(beside) > Fixed::ZERO,
        "the hero beside the struck one took a share"
    );
    assert_eq!(lost(away), Fixed::ZERO, "the one 900 off took none");
}

#[test]
fn a_hellbears_death_enrages_its_partner_unless_it_was_broken() {
    for broken in [false, true] {
        let mut world = day();
        let bear = world.spawn_neutral(NeutralKind::PolarFurbolgChampion, at(0), 0, 0);
        let partner = world.spawn_neutral(NeutralKind::PolarFurbolgUrsaWarrior, at(200), 0, 0);
        let outsider = world.spawn_neutral(NeutralKind::Kobold, at(1000), 0, 0);
        world.settle();
        if broken {
            afflict(&mut world, bear, ModifierKind::Broken, 600);
        }
        world.step();
        let quick = world.stats.get(partner).expect("derived").attack_speed;
        world.push_hit(None, bear, 99_999, DamageKind::Pure);
        world.step();
        assert!(!world.alive(bear));
        world.step();
        let now = world.stats.get(partner).expect("derived").attack_speed;
        assert_eq!(
            now - quick,
            if broken { 0 } else { 120 },
            "broken: {broken}: the partner's attack speed"
        );
        assert!(!wears(&world, outsider, NeutralAbility::DeathThroeRush));
    }
}

#[test]
fn a_dying_mud_golem_splits_into_shards_that_walk_home_and_slow_the_hero() {
    let mut world = day();
    let home = at(0);
    let golem = world.spawn_neutral(NeutralKind::MudGolem, home, 3, 0);
    let hero = passive_hero(&mut world, at(700));
    world.transform.get_mut(golem).expect("standing").pos = at(600);
    world.settle();
    world.push_hit(None, golem, 99_999, DamageKind::Pure);
    world.step();
    assert!(!world.alive(golem));
    let shards: Vec<Entity> = world
        .entities
        .iter()
        .filter(|e| world.neutral.get(*e) == Some(&NeutralKind::MudGolemSplit))
        .collect();
    assert_eq!(shards.len(), 2, "two shards at the first level");
    world.step();
    for shard in &shards {
        assert_eq!(
            world.camp_home.get(*shard).map(|c| (c.camp, c.home)),
            Some((3, home))
        );
        assert_eq!(
            world.stats.get(*shard).expect("derived").max_hp,
            Fixed::from_int(250)
        );
        assert!(life_left(&world, *shard).is_some_and(|left| left > 59 * 30));
    }
    assert!(
        wears(&world, hero, NeutralAbility::ShardSplit),
        "the hero was slowed"
    );
    let before = world.pos(shards[0]).expect("standing");
    run(&mut world, 120);
    let after = world.pos(shards[0]).expect("standing");
    assert!(
        after.distance_squared(home) < before.distance_squared(home),
        "the shards walk back to the camp"
    );
}

#[test]
fn a_bubble_soaks_magic_and_bursts_into_a_mend() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let friend = world.spawn_hero(
        Team::Radiant,
        at(300),
        bota_proto::SlotId(1),
        bota_proto::HeroId::SHADOW_FIEND,
    );
    world.set_order(friend, UnitOrder::Stand);
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(friend);
    let bubble = |ability, amount| ModifierKind::Spell {
        ability,
        level: 1,
        amount,
    };
    afflict(
        &mut world,
        hero,
        bubble(NeutralAbility::WaterBubbleLarge, 210),
        300,
    );
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(300);
    world.health.get_mut(friend).expect("alive").hp = Fixed::from_int(300);
    // Pure and physical damage are not soaked; magic is.
    world.push_hit(None, hero, 100, DamageKind::Magical);
    world.step();
    let regen = world.stats.get(hero).expect("derived").hp_regen;
    assert_eq!(
        hp(&world, hero),
        Fixed::from_int(300) + regen,
        "the first hundred is soaked"
    );
    world.push_hit(None, hero, 200, DamageKind::Magical);
    world.step();
    let resist = world.stats.get(hero).expect("derived").magic_resist_bp;
    let first = mitigate(100, DamageKind::Magical, Fixed::ZERO, resist);
    let second = mitigate(200, DamageKind::Magical, Fixed::ZERO, resist);
    let spill = (second - (210 - first)).max(0);
    assert!(spill > 0, "the second hit outlasts the bubble");
    let expected = 300 - spill + 105;
    assert!(
        (hp(&world, hero).to_int() - expected).abs() <= 1,
        "what got through, then half the barrier back: {:?} not {expected}",
        hp(&world, hero)
    );
    assert!(
        !wears(&world, hero, NeutralAbility::WaterBubbleLarge),
        "and it is gone"
    );
    // The large one mends half its barrier, 105, on the unit and its ally.
    assert!(
        hp(&world, friend) > Fixed::from_int(400),
        "the ally mended too"
    );
}

#[test]
fn a_bubble_that_runs_out_mends_the_unit_it_was_on() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    world.settle();
    world.fill_pools(hero);
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(400);
    let kind = ModifierKind::Spell {
        ability: NeutralAbility::WaterBubbleMedium,
        level: 1,
        amount: 150,
    };
    afflict(&mut world, hero, kind, 10);
    let regen = world.stats.get(hero).expect("derived").hp_regen;
    run(&mut world, 12);
    assert!(!wears(&world, hero, NeutralAbility::WaterBubbleMedium));
    assert!(
        hp(&world, hero) >= Fixed::from_int(475),
        "half of 150 came back: {:?} (regen {regen:?})",
        hp(&world, hero)
    );
}

#[test]
fn a_tempest_burns_harder_at_its_centre_and_slows() {
    let burn = |apart: i32| {
        let mut world = day();
        let hero = passive_hero(&mut world, at(apart));
        world.spawn_neutral(NeutralKind::WildkinTornado, at(0), 0, 0);
        world.settle();
        world.fill_pools(hero);
        let slowed_base = speed(&world, hero);
        let full = hp(&world, hero);
        run(&mut world, 60);
        (
            full - hp(&world, hero),
            speed(&world, hero) < slowed_base,
            world.stats.get(hero).expect("derived").hp_regen,
        )
    };
    let (centre, slowed, regen) = burn(50);
    let (rim, rim_slowed, _) = burn(550);
    let (outside, outside_slowed, _) = burn(700);
    assert!(slowed && rim_slowed && !outside_slowed);
    let regen = regen * Fixed::from_int(60);
    assert!(
        centre + regen > (rim + regen) * Fixed::from_int(2),
        "{centre:?} vs {rim:?}"
    );
    assert!(rim > outside, "the edge still burns, past it nothing");
}

#[test]
fn a_prowler_shaman_below_ninety_percent_roots_and_burns_its_target() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let shaman = awake(&mut world, NeutralKind::ProwlerShaman, at(100));
    world.settle();
    world.fill_pools(hero);
    let max = world.stats.get(shaman).expect("derived").max_hp;
    world.health.get_mut(shaman).expect("alive").hp = max * Fixed::from_ratio(50, 100);
    let mut rooted = None;
    for _ in 0..200 {
        world.step();
        if world.rooted(hero) {
            rooted = Some(world.tick);
            break;
        }
    }
    let first = rooted.expect("the shaman petrified the hero");
    // Told to walk away, a rooted hero stays where it is.
    let stood = world.pos(hero).expect("standing");
    world.set_order(hero, UnitOrder::Move { pos: at(-900) });
    let before = hp(&world, hero);
    run(&mut world, 30);
    assert_eq!(world.pos(hero), Some(stood));
    assert!(
        before - hp(&world, hero) >= Fixed::from_int(60),
        "the burn ticks 10 every tenth second"
    );
    run(&mut world, 60);
    assert!(!world.rooted(hero), "and lets go after a second and a bit");
    assert!(world.tick > first);
    assert_ne!(world.pos(hero), Some(stood), "free, it walks");
}

#[test]
fn a_gnoll_poison_ticks_every_second_and_cuts_regeneration() {
    let mut world = day();
    world.tick = rules::tick_at_game_minute(20);
    world.immortal_heroes = true;
    let hero = passive_hero(&mut world, at(0));
    let gnoll = awake(&mut world, NeutralKind::GnollAssassin, at(300));
    world.settle();
    let seen = blows(&mut world, 240, gnoll, hero);
    let poison: Vec<u32> = seen
        .iter()
        .filter(|(_, _, kind)| *kind == DamageKind::Magical)
        .map(|(tick, _, _)| *tick)
        .collect();
    assert!(poison.len() >= 2, "it burns: {poison:?}");
    assert!(
        poison
            .windows(2)
            .all(|pair| pair[1] - pair[0] == rules::TICKS_PER_SECOND)
    );
}

#[test]
fn a_ghost_chills_whoever_it_strikes() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let ghost = awake(&mut world, NeutralKind::Ghost, at(300));
    world.settle();
    let free = speed(&world, hero);
    let _ = blows(&mut world, 120, ghost, hero);
    assert!(wears(&world, hero, NeutralAbility::FrostAttack));
    assert!(speed(&world, hero) < free * Fixed::from_ratio(80, 100));
}

#[test]
fn a_prowler_aura_mends_its_bearer_by_a_share_of_what_it_deals() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let acolyte = awake(&mut world, NeutralKind::ProwlerAcolyte, at(100));
    world.settle();
    let max = world.stats.get(acolyte).expect("derived").max_hp;
    world.health.get_mut(acolyte).expect("alive").hp = max * Fixed::from_ratio(50, 100);
    for _ in 0..200 {
        let before = hp(&world, acolyte);
        let blow = blows(&mut world, 1, acolyte, hero);
        if let Some((_, amount, _)) = blow.first() {
            let regen = world.stats.get(acolyte).expect("derived").hp_regen;
            assert_eq!(
                hp(&world, acolyte),
                before + regen + Fixed::from_int(amount * 9 / 100),
                "nine percent of {amount} came back"
            );
            return;
        }
    }
    panic!("the acolyte never struck");
}

#[test]
fn a_black_drake_makes_spells_hurt_more() {
    let hurt = |with_drake: bool| {
        let mut world = day();
        let hero = passive_hero(&mut world, at(0));
        if with_drake {
            world.spawn_neutral(NeutralKind::BlackDrake, at(400), 0, 0);
        }
        world.settle();
        world.fill_pools(hero);
        world.step();
        let before = hp(&world, hero);
        world.push_hit(None, hero, 200, DamageKind::Magical);
        world.step();
        let regen = world.stats.get(hero).expect("derived").hp_regen;
        before + regen - hp(&world, hero)
    };
    let plain = hurt(false);
    let amplified = hurt(true);
    assert_eq!(amplified.to_int(), plain.to_int() * 105 / 100);
}

#[test]
fn piercing_neutrals_hit_creeps_hard_and_heroes_soft() {
    let mut world = day();
    let hero = passive_hero(&mut world, at(0));
    let creep = world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, at(60));
    let harpy = awake(&mut world, NeutralKind::HarpyStorm, at(-100));
    world.settle();
    world.fill_pools(creep);
    let def = NeutralKind::HarpyStorm.def();
    let (creep_armor, hero_armor) = (
        world.stats.get(creep).expect("derived").armor,
        world.stats.get(hero).expect("derived").armor,
    );
    let (least, most) = (def.damage, def.damage + def.damage_spread);
    let to_creep = one_swing_takes(&mut world, harpy, creep);
    let ok = |taken, pct, target_armor| {
        (mitigate(least * pct / 100, DamageKind::Physical, target_armor, 0)
            ..=mitigate(most * pct / 100, DamageKind::Physical, target_armor, 0))
            .contains(&taken)
    };
    assert!(ok(to_creep, 150, creep_armor), "{to_creep} to a creep");
    let to_hero = one_swing_takes(&mut world, harpy, hero);
    assert!(ok(to_hero, 50, hero_armor), "{to_hero} to a hero");
}

#[test]
fn accuracy_lets_blows_through_evasion() {
    let misses = |lizard: NeutralKind| {
        let mut world = day();
        world.tick = rules::PREGAME_TICKS;
        let hero = passive_hero(&mut world, at(0));
        hand_item(&mut world, hero, ItemId::BUTTERFLY, 1);
        let beast = awake(&mut world, lizard, at(300));
        world.settle();
        let (mut hit, mut missed) = (0, 0);
        while hit + missed < 120 && world.tick < rules::PREGAME_TICKS + 20_000 {
            for event in world.step() {
                match event.kind {
                    EventKind::Damaged {
                        source,
                        target,
                        kind,
                        ..
                    } if source == Some(wire_id(beast))
                        && target == wire_id(hero)
                        && kind == DamageKind::Physical =>
                    {
                        hit += 1
                    }
                    EventKind::Missed { source, .. } if source == Some(wire_id(beast)) => {
                        missed += 1
                    }
                    _ => {}
                }
            }
        }
        (missed, hit + missed)
    };
    let (plain, swings) = misses(NeutralKind::FelBeast);
    let (accurate, _) = misses(NeutralKind::SmallThunderLizard);
    assert!(swings >= 100);
    assert!(
        accurate * 10 < plain * 8,
        "{accurate} misses with drums, {plain} without"
    );
}
