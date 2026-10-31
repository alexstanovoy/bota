//! Neutral camps casting: a real camp, fought by heroes, lets its spells go
//! when Dota's neutral AI does, and each spell does what it should.

use bota_core::{ModifierKind, NeutralAbility, NeutralKind, Spell, rules, ticks_of_ms};
use bota_proto::{
    DamageKind, EffectId, EventKind, Fixed, HeroId, Inflictor, SlotId, StatusFlags, Team, Vec2,
};

use crate::game::{Entity, MatchRng, UnitOrder, World, wire_id};

use super::support::*;

const CAMP: Vec2 = Vec2::from_ints(5000, 5000);

/// A world in daylight where heroes never die.
fn day() -> World {
    let mut world = World::new();
    world.tick = world.schedule().horn;
    world.immortal_heroes = true;
    world
}

/// A Radiant hero that stays where it stands and strikes at nothing.
fn bystander(world: &mut World, at: Vec2) -> Entity {
    let hero = hero_at(world, at);
    world.set_order(hero, UnitOrder::Stand);
    hero
}

/// Wakes a camp onto a hero, with the blow of a hero that stands by.
fn rouse(world: &mut World, beast: Entity, by: Entity) {
    world.push_hit(Some(by), beast, 1, DamageKind::Physical, None);
    world.step();
}

/// Sets a unit's health to a share of its maximum.
fn set_health_pct(world: &mut World, unit: Entity, pct: i32) {
    let most = world.stats.get(unit).expect("derived").max_hp.to_int();
    world.health.get_mut(unit).expect("alive").hp = Fixed::from_int(most * pct / 100);
}

/// Steps until the unit casts the ability, at most `ticks` ticks. The world
/// is left in the tick of the cast.
fn until_cast(world: &mut World, caster: Entity, ability: NeutralAbility, ticks: u32) -> bool {
    for _ in 0..ticks {
        let events = world.step();
        let cast = events.iter().any(|event| {
            matches!(event.kind, EventKind::AbilityCast { caster: who, ability: id }
                if who == wire_id(caster) && id == ability.id())
        });
        if cast {
            return true;
        }
    }
    false
}

/// How many times anything casts the ability over `ticks` ticks.
fn casts_over(world: &mut World, ability: NeutralAbility, ticks: u32) -> usize {
    let mut cast = 0;
    for _ in 0..ticks {
        cast += world
            .step()
            .iter()
            .filter(|event| {
                matches!(event.kind, EventKind::AbilityCast { ability: id, .. } if id == ability.id())
            })
            .count();
    }
    cast
}

/// The modifier a spell left on a unit, if it wears one.
fn spell_on(world: &World, unit: Entity, wanted: NeutralAbility) -> Option<(u8, u16)> {
    world
        .modifiers
        .get(unit)?
        .active()
        .find_map(|held| match held.kind {
            ModifierKind::Spell {
                ability,
                level,
                amount,
            } if ability == wanted => Some((level, amount)),
            _ => None,
        })
}

fn cooldown_of(world: &World, unit: Entity, wanted: NeutralAbility) -> u32 {
    world
        .abilities
        .get(unit)
        .expect("a book")
        .slots
        .iter()
        .find(|slot| slot.id == wanted.id())
        .expect("it carries the spell")
        .cooldown
}

fn health_of(world: &World, unit: Entity) -> i32 {
    world.health.get(unit).expect("alive").hp.to_int()
}

#[test]
fn the_conqueror_stomps_when_three_enemies_stand_within_250_and_not_for_two() {
    for (standing, stomps) in [(3, true), (2, false)] {
        let mut world = day();
        let khan = world.spawn_neutral(NeutralKind::CentaurKhan, CAMP, 1, 0);
        let heroes: Vec<Entity> = (0..standing)
            .map(|at| bystander(&mut world, CAMP + Vec2::from_ints(120, 60 * at - 60)))
            .collect();
        world.settle();
        rouse(&mut world, khan, heroes[0]);
        let went = until_cast(&mut world, khan, NeutralAbility::WarStomp, 120);
        assert_eq!(went, stomps, "{standing} enemies close to the Conqueror");
        if stomps {
            for hero in heroes {
                assert!(carries(&world, hero, ModifierKind::Stunned), "stunned");
            }
        }
    }
}

#[test]
fn a_sleeping_camp_casts_nothing_and_a_roused_one_only_as_often_as_its_cooldown_lets_it() {
    let mut world = day();
    world.tick = world.schedule().horn + rules::HALF_DAY_TICKS + 30 * rules::TICKS_PER_SECOND;
    let khan = world.spawn_neutral(NeutralKind::CentaurKhan, CAMP, 1, 0);
    let heroes: Vec<Entity> = (0..3)
        .map(|at| bystander(&mut world, CAMP + Vec2::from_ints(120, 60 * at - 60)))
        .collect();
    world.settle();
    let asleep = casts_over(
        &mut world,
        NeutralAbility::WarStomp,
        20 * rules::TICKS_PER_SECOND,
    );
    assert_eq!(asleep, 0, "three heroes at its feet, but it sleeps");
    rouse(&mut world, khan, heroes[0]);
    let roused = casts_over(
        &mut world,
        NeutralAbility::WarStomp,
        30 * rules::TICKS_PER_SECOND,
    );
    assert_eq!(roused, 3, "a stomp every 12 s, at 0, 12 and 24 s of 30");
}

#[test]
fn the_hellbear_smasher_slows_what_its_clap_catches() {
    let mut world = day();
    let bear = world.spawn_neutral(NeutralKind::PolarFurbolgUrsaWarrior, CAMP, 1, 0);
    let heroes = [
        bystander(&mut world, CAMP + Vec2::from_ints(120, 40)),
        bystander(&mut world, CAMP + Vec2::from_ints(120, -40)),
    ];
    world.settle();
    let before = world.stats.get(heroes[0]).expect("derived").move_speed;
    rouse(&mut world, bear, heroes[0]);
    assert!(until_cast(
        &mut world,
        bear,
        NeutralAbility::ThunderClap,
        120
    ));
    world.settle();
    let after = world.stats.get(heroes[0]).expect("derived").move_speed;
    assert!(
        after < before,
        "the clap slows: {after:?} against {before:?}"
    );
    assert!(spell_on(&world, heroes[1], NeutralAbility::ThunderClap).is_some());
    for hero in heroes {
        assert_ne!(
            shown(&world, Some(Team::Radiant), hero).statuses.bits & StatusFlags::SLOWED,
            0,
            "a view tells the clap's slow"
        );
    }
}

#[test]
fn a_golem_hurls_its_boulder_at_a_unit_under_300_health_and_the_other_waits() {
    let mut world = day();
    let golems = [
        world.spawn_neutral(NeutralKind::MudGolem, CAMP, 1, 0),
        world.spawn_neutral(NeutralKind::MudGolem, CAMP + Vec2::from_ints(0, 90), 1, 0),
    ];
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
    world.settle();
    set_health_pct(&mut world, hero, 30);
    let hurt = health_of(&world, hero);
    assert!(hurt < 300);
    rouse(&mut world, golems[0], hero);
    let mut casts = 0;
    let mut stunned = false;
    for _ in 0..ticks_of_ms(4500) {
        casts += world
            .step()
            .iter()
            .filter(|event| matches!(event.kind, EventKind::AbilityCast { ability, .. } if ability == NeutralAbility::HurlBoulder.id()))
            .count();
        stunned |= carries(&world, hero, ModifierKind::Stunned);
    }
    assert_eq!(casts, 1, "one boulder, and the other golem waits");
    assert!(stunned, "the boulder stuns on arriving");
    assert!(health_of(&world, hero) < hurt, "and hurts");
}

#[test]
fn a_boulder_names_its_spell_and_its_golem_in_flight_in_its_blow_and_on_its_stun() {
    // Dota's tracking missile, its blow in the combat log and the stun's
    // buff icon all name the Hurl Boulder, and the stun names the golem.
    let mut world = day();
    let golem = world.spawn_neutral(NeutralKind::MudGolem, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
    world.settle();
    set_health_pct(&mut world, hero, 30);
    rouse(&mut world, golem, hero);
    let boulder = Some(Inflictor::Ability(NeutralAbility::HurlBoulder.id()));
    let (mut flew, mut struck) = (false, None);
    for _ in 0..ticks_of_ms(4500) {
        let events = world.step();
        let view = world.view(Team::Radiant);
        flew |= view.projectiles.iter().any(|shot| {
            shot.inflictor == boulder
                && shot.source == Some(wire_id(golem))
                && shot.target == Some(wire_id(hero))
                && shot.ability.is_none()
        });
        let blow = events.iter().find_map(|event| match event.kind {
            EventKind::Damaged {
                source,
                target,
                inflictor,
                ..
            } if source == Some(wire_id(golem)) && target == wire_id(hero) => Some(inflictor),
            _ => None,
        });
        if let Some(inflictor) = blow.filter(|inflictor| inflictor.is_some()) {
            let stun = shown(&world, Some(Team::Radiant), hero)
                .effects
                .into_iter()
                .find(|effect| effect.id == EffectId::STUNNED)
                .expect("the boulder stuns as it hits");
            struck = Some((inflictor, stun));
            break;
        }
    }
    assert!(
        flew,
        "the boulder in flight names its spell, its golem and the hero"
    );
    let (inflictor, stun) = struck.expect("the boulder lands");
    assert_eq!(inflictor, boulder, "its blow names the spell");
    assert_eq!(stun.inflictor, boulder, "so does the stun");
    assert_eq!(
        stun.caster,
        Some(wire_id(golem)),
        "and the stun names the golem"
    );
    assert!(stun.ticks_total.is_some_and(|total| total > 0));
    assert_eq!(
        stun.ticks_left, stun.ticks_total,
        "a stun just put on is whole"
    );
}

#[test]
fn a_golem_leaves_a_healthy_unit_alone() {
    let mut world = day();
    let golem = world.spawn_neutral(NeutralKind::MudGolem, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
    world.settle();
    rouse(&mut world, golem, hero);
    assert_eq!(casts_over(&mut world, NeutralAbility::HurlBoulder, 120), 0);
}

#[test]
fn the_summoner_raises_skeletons_below_half_its_health() {
    for (pct, raises) in [(40, true), (70, false)] {
        let mut world = day();
        let troll = world.spawn_neutral(NeutralKind::DarkTrollWarlord, CAMP, 1, 0);
        let hero = bystander(&mut world, CAMP + Vec2::from_ints(200, 0));
        world.settle();
        set_health_pct(&mut world, troll, pct);
        rouse(&mut world, troll, hero);
        let went = until_cast(&mut world, troll, NeutralAbility::RaiseDead, 120);
        assert_eq!(went, raises, "at {pct}% health");
        let skeletons: Vec<Entity> = world
            .entities
            .iter()
            .filter(|unit| world.neutral.get(*unit) == Some(&NeutralKind::SkeletonWarrior))
            .collect();
        assert_eq!(skeletons.len(), if raises { 3 } else { 0 });
        world.step();
        for skeleton in skeletons {
            let stats = world.stats.get(skeleton).expect("derived");
            // The level's own, and the three skeletons' Rally, two each.
            assert_eq!(
                (stats.max_hp.to_int(), stats.damage),
                (250, 12 + 3 * 2),
                "the level's own, rallied"
            );
            let left = life_left(&world, skeleton).expect("they live a while");
            assert!(left > ticks_of_ms(34_000) && left <= ticks_of_ms(35_000));
            assert_eq!(world.camp_home.get(skeleton).map(|home| home.camp), Some(1));
        }
    }
}

#[test]
fn the_frogs_spikes_stun_and_the_others_hold_off_for_five_seconds() {
    let mut world = day();
    let frogs = [
        world.spawn_neutral(NeutralKind::Froglet, CAMP, 1, 0),
        world.spawn_neutral(NeutralKind::Froglet, CAMP + Vec2::from_ints(0, 70), 1, 0),
    ];
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(200, 30));
    world.settle();
    rouse(&mut world, frogs[0], hero);
    assert!(
        until_cast(&mut world, frogs[0], NeutralAbility::ArmOfTheDeep, 120)
            || until_cast(&mut world, frogs[1], NeutralAbility::ArmOfTheDeep, 1)
    );
    let hold = ticks_of_ms(5000) - 1;
    let waiting = frogs
        .iter()
        .filter(|frog| cooldown_of(&world, **frog, NeutralAbility::ArmOfTheDeep) >= hold)
        .count();
    assert_eq!(waiting, 2, "the one that cast and the one that must wait");
    assert!(
        carries(&world, hero, ModifierKind::Stunned),
        "the tentacles stun"
    );
    assert_eq!(
        casts_over(&mut world, NeutralAbility::ArmOfTheDeep, ticks_of_ms(4000)),
        0
    );
}

#[test]
fn the_marshmage_puts_a_bubble_on_an_ally_below_half_health() {
    let mut world = day();
    let mage = world.spawn_neutral(NeutralKind::GrownFrogMage, CAMP, 1, 0);
    let kobold = world.spawn_neutral(NeutralKind::Kobold, CAMP + Vec2::from_ints(60, 60), 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
    world.settle();
    rouse(&mut world, mage, hero);
    assert_eq!(
        casts_over(&mut world, NeutralAbility::WaterBubbleMedium, 90),
        0,
        "all well"
    );
    set_health_pct(&mut world, kobold, 40);
    assert!(until_cast(
        &mut world,
        mage,
        NeutralAbility::WaterBubbleMedium,
        60
    ));
    let (level, soak) = spell_on(&world, kobold, NeutralAbility::WaterBubbleMedium)
        .expect("the hurt ally wears it");
    assert_eq!((level, soak), (1, 150));
    assert!(spell_on(&world, mage, NeutralAbility::WaterBubbleMedium).is_none());
}

#[test]
fn the_thunderhide_frenzies_itself_when_fighting_and_slams_a_crowd() {
    let mut world = day();
    let lizard = world.spawn_neutral(NeutralKind::BigThunderLizard, CAMP, 1, 0);
    let heroes: Vec<Entity> = (0..3)
        .map(|at| hero_at(&mut world, CAMP + Vec2::from_ints(110, 50 * at - 50)))
        .collect();
    for hero in &heroes {
        world.set_order(
            *hero,
            UnitOrder::Attack {
                target: lizard,
                last_seen: CAMP,
            },
        );
    }
    world.settle();
    let plain = world.stats.get(lizard).expect("derived").attack_speed;
    rouse(&mut world, lizard, heroes[0]);
    assert!(until_cast(&mut world, lizard, NeutralAbility::Frenzy, 90));
    assert!(spell_on(&world, lizard, NeutralAbility::Frenzy).is_some());
    world.settle();
    assert!(world.stats.get(lizard).expect("derived").attack_speed > plain);
    assert!(
        until_cast(&mut world, lizard, NeutralAbility::Slam, 240),
        "three beset it"
    );
    assert!(
        heroes
            .iter()
            .all(|hero| spell_on(&world, *hero, NeutralAbility::Slam).is_some())
    );
}

#[test]
fn a_ripper_pushes_a_hero_chasing_another_away() {
    let mut world = day();
    let ripper = world.spawn_neutral(NeutralKind::EnragedWildkin, CAMP, 1, 0);
    let chaser = hero_at(&mut world, CAMP + Vec2::from_ints(200, 0));
    let chased = world.spawn_hero(
        Team::Dire,
        CAMP + Vec2::from_ints(500, 0),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.set_order(chased, UnitOrder::Stand);
    world.set_order(
        chaser,
        UnitOrder::Attack {
            target: chased,
            last_seen: CAMP,
        },
    );
    world.settle();
    let start = world.pos(chaser).expect("placed");
    rouse(&mut world, ripper, chaser);
    assert!(until_cast(
        &mut world,
        ripper,
        NeutralAbility::Hurricane,
        120
    ));
    let from = world.pos(ripper).expect("placed");
    run(&mut world, 30);
    let end = world.pos(chaser).expect("placed");
    let gap = |one: Vec2, other: Vec2| bota_core::isqrt64(one.distance_squared(other));
    assert!(
        gap(end, from) > gap(start, from) + 300,
        "carried away from the Ripper"
    );
}

#[test]
fn the_banisher_purges_a_hero_wearing_a_buff_and_the_slow_falls() {
    let mut world = day();
    let satyr = world.spawn_neutral(NeutralKind::SatyrTrickster, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(120, 0));
    world.settle();
    let plain = world.stats.get(hero).expect("derived").move_speed;
    put_on(&mut world, hero, ModifierKind::Hastened { pct: 20 }, 600);
    rouse(&mut world, satyr, hero);
    assert!(until_cast(&mut world, satyr, NeutralAbility::Purge, 120));
    assert!(
        !carries(&world, hero, ModifierKind::Hastened { pct: 0 }),
        "the buff is gone"
    );
    let (_, slow) = spell_on(&world, hero, NeutralAbility::Purge).expect("slowed");
    assert_eq!(slow, 50);
    world.settle();
    assert!(world.stats.get(hero).expect("derived").move_speed < plain);
    run(&mut world, rules::TICKS_PER_SECOND * 2 + 3);
    let (_, later) = spell_on(&world, hero, NeutralAbility::Purge).expect("still slowed");
    assert_eq!(later, 30, "10% off each second");
}

#[test]
fn the_banisher_purges_a_feasting_fiend_and_his_feast_ends() {
    // Dota's Feast of Souls is basic-dispellable (SPELL_DISPELLABLE_YES): a
    // feasting hero is one a purge goes for, and the purge ends the feast.
    let mut world = day();
    let satyr = world.spawn_neutral(NeutralKind::SatyrTrickster, CAMP, 1, 0);
    let fiend = bystander(&mut world, CAMP + Vec2::from_ints(120, 0));
    world.settle();
    let plain = world.stats.get(fiend).expect("derived").attack_speed;
    assert!(world.cast_feast(fiend, 0));
    rouse(&mut world, satyr, fiend);
    assert!(
        until_cast(&mut world, satyr, NeutralAbility::Purge, 120),
        "the satyr purges the feasting fiend"
    );
    world.step();
    let shown = shown(&world, None, fiend);
    assert!(
        shown
            .effects
            .iter()
            .all(|effect| effect.id != EffectId::FEASTING),
        "the feast is off him"
    );
    assert_eq!(shown.attack_speed, plain, "and so is its haste");
}

#[test]
fn the_stormcrafter_chains_lightning_off_a_weakened_attacker() {
    let mut world = day();
    let harpy = world.spawn_neutral(NeutralKind::HarpyStorm, CAMP, 1, 0);
    let first = hero_at(&mut world, CAMP + Vec2::from_ints(250, 0));
    let second = bystander(&mut world, CAMP + Vec2::from_ints(700, 0));
    world.set_order(
        first,
        UnitOrder::Attack {
            target: harpy,
            last_seen: CAMP,
        },
    );
    world.settle();
    set_health_pct(&mut world, first, 20);
    let (hp, other) = (health_of(&world, first), health_of(&world, second));
    assert!(hp < 330);
    rouse(&mut world, harpy, first);
    assert!(until_cast(
        &mut world,
        harpy,
        NeutralAbility::ChainLightning,
        120
    ));
    assert_eq!(
        health_of(&world, second),
        other,
        "it has not reached it yet"
    );
    run(&mut world, 12);
    assert!(health_of(&world, second) < other, "and then it leaps");
}

#[test]
fn the_troll_ensnares_a_hero_when_heroes_of_both_sides_are_near() {
    let mut world = day();
    let troll = world.spawn_neutral(NeutralKind::DarkTroll, CAMP, 1, 0);
    let target = bystander(&mut world, CAMP + Vec2::from_ints(250, 0));
    let rival = world.spawn_hero(
        Team::Dire,
        CAMP + Vec2::from_ints(-300, 0),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.set_order(rival, UnitOrder::Stand);
    world.settle();
    rouse(&mut world, troll, target);
    assert!(until_cast(&mut world, troll, NeutralAbility::Ensnare, 120));
    run(&mut world, 20);
    assert!(
        carries(&world, target, ModifierKind::Rooted),
        "the net holds"
    );
}

#[test]
fn the_bruiser_smashes_ahead_of_itself_when_hurt() {
    let mut world = day();
    let ogre = world.spawn_neutral(NeutralKind::OgreMauler, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(150, 0));
    world.settle();
    set_health_pct(&mut world, ogre, 60);
    let before = health_of(&world, hero);
    rouse(&mut world, ogre, hero);
    assert!(until_cast(&mut world, ogre, NeutralAbility::OgreSmash, 120));
    assert!(carries(&world, hero, ModifierKind::Stunned));
    assert!(
        before - health_of(&world, hero) >= 100,
        "magic damage, a share of the health"
    );
}

#[test]
fn a_ripper_dying_in_a_camp_sometimes_has_the_wildwing_raise_a_tornado() {
    let mut raised = 0;
    for stream in 0..60 {
        let mut world = day();
        world.rng = MatchRng::new(&[0; 32], stream);
        let wing = world.spawn_neutral(NeutralKind::Wildkin, CAMP, 1, 0);
        let ripper = world.spawn_neutral(
            NeutralKind::EnragedWildkin,
            CAMP + Vec2::from_ints(0, 90),
            1,
            0,
        );
        let hero = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
        world.settle();
        rouse(&mut world, wing, hero);
        world.push_hit(Some(hero), ripper, 100_000, DamageKind::Pure, None);
        world.step();
        run(&mut world, 30);
        let tornado = world
            .entities
            .iter()
            .find(|unit| world.neutral.get(*unit) == Some(&NeutralKind::WildkinTornado));
        if let Some(tornado) = tornado {
            raised += 1;
            let left = life_left(&world, tornado).expect("it stands a while");
            assert!(left > ticks_of_ms(14_000));
            assert!(
                world
                    .pos(tornado)
                    .expect("placed")
                    .within(world.pos(hero).expect("placed"), Fixed::from_int(600))
            );
        }
    }
    assert!(
        (8..=32).contains(&raised),
        "a third of the time: {raised} of 60"
    );
}

#[test]
fn the_spells_that_are_never_cast_stay_uncast() {
    let mut world = day();
    let hero = bystander(&mut world, CAMP);
    let mut beasts = Vec::new();
    for (at, def) in bota_core::NEUTRALS.iter().enumerate() {
        let carries_one = def
            .abilities
            .iter()
            .any(|a| matches!(a.spell(), Spell::Uncast));
        if carries_one {
            let spot =
                CAMP + Vec2::from_ints(60 * (at as i32 % 8) - 200, 60 * (at as i32 / 8) + 100);
            beasts.push(world.spawn_neutral(def.kind, spot, 1, 0));
        }
    }
    world.settle();
    assert!(beasts.len() > 5);
    rouse(&mut world, beasts[0], hero);
    for _ in 0..600 {
        for event in world.step() {
            if let EventKind::AbilityCast { ability, .. } = event.kind {
                let which = NeutralAbility::of(ability).expect("only neutrals cast here");
                assert!(
                    !matches!(which.spell(), Spell::Uncast),
                    "{which:?} is never cast"
                );
            }
        }
    }
}
