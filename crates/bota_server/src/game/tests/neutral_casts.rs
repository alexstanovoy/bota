//! Neutral camps casting: a real camp, fought by heroes, lets its spells go
//! when Dota's neutral AI does, and each spell does what it should.

use bota_core::{
    ModifierKind, NeutralAbility, NeutralKind, Spell, point_ahead, rules, ticks_of_ms,
};
use bota_proto::{
    DamageKind, EffectId, EventKind, Fixed, HeroId, Inflictor, SlotId, StatusFlags, Target, Team,
    Vec2,
};

use crate::game::{Entity, MatchRng, Purpose, UnitOrder, World, wire_id};

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
            matches!(event.kind(), EventKind::AbilityCast { caster: who, ability: id }
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
                matches!(event.kind(), EventKind::AbilityCast { ability: id, .. } if id == ability.id())
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
fn a_shard_golem_hurls_its_boulder_at_a_unit_under_300_health_and_the_other_waits() {
    // Dota 7.42 gave the Mud Golem the Hurl Big Boulder; the Shard Golem keeps Hurl Boulder.
    let mut world = day();
    let golems = [
        world.spawn_neutral(NeutralKind::MudGolemSplit, CAMP, 1, 0),
        world.spawn_neutral(
            NeutralKind::MudGolemSplit,
            CAMP + Vec2::from_ints(0, 90),
            1,
            0,
        ),
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
            .filter(|event| matches!(event.kind(), EventKind::AbilityCast { ability, .. } if ability == NeutralAbility::HurlBoulder.id()))
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
    let golem = world.spawn_neutral(NeutralKind::MudGolemSplit, CAMP, 1, 0);
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
        let blow = events.iter().find_map(|event| match event.kind() {
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
            assert_eq!(
                world.camp_home.get(skeleton).map(|home| home.camp),
                Some(Some(1))
            );
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
    assert_eq!(slow, 80, "Dota 7.42: the Banisher's base slow is 80%");
    world.settle();
    assert!(world.stats.get(hero).expect("derived").move_speed < plain);
    run(&mut world, rules::TICKS_PER_SECOND * 2 + 3);
    let (_, later) = spell_on(&world, hero, NeutralAbility::Purge).expect("still slowed");
    assert_eq!(
        later, 48,
        "a fifth of the 80% off each second (purge_rate 5)"
    );
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
    assert_eq!(shown.attack_speed, plain.to_int(), "and so is its haste");
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

/// A neutral's Chain Lightning leaps on after its caster falls (its chain outlives its source, as
/// it always did; an item's stops with its wielder): the stormcrafter is killed (a lethal blow
/// laid on the queue) after its cast, another of the camp keeping the leap in sight, and the
/// bystander is still struck.
#[test]
fn the_stormcrafters_lightning_leaps_on_after_it_falls() {
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
    rouse(&mut world, harpy, first);
    assert!(until_cast(
        &mut world,
        harpy,
        NeutralAbility::ChainLightning,
        120
    ));
    let watcher = world.spawn_neutral(
        NeutralKind::HarpyStorm,
        CAMP + Vec2::from_ints(500, 300),
        1,
        0,
    );
    world.set_order(watcher, UnitOrder::Stand);
    let other = health_of(&world, second);
    world.push_hit(None, harpy, 1_000_000, DamageKind::Pure, None);
    world.step();
    assert!(!world.alive(harpy));
    run(&mut world, 12);
    assert!(health_of(&world, second) < other, "the lightning leapt on");
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
    // Dota 7.42: it tries at a 25% chance and waits eight seconds after a miss, so one attempt
    // falls every eight seconds. The match is seeded, so this is deterministic; the bound is the
    // rule's: sixteen attempts all missing would be a 1% chance.
    let attempts = 16;
    assert!(until_cast(
        &mut world,
        ogre,
        NeutralAbility::OgreSmash,
        attempts * ticks_of_ms(8000)
    ));
    assert!(carries(&world, hero, ModifierKind::Stunned));
    assert!(
        before - health_of(&world, hero) >= 100,
        "magic damage, a share of the health"
    );
}

#[test]
fn a_player_controlled_ogre_releases_smash_after_its_shorter_cast_point() {
    use bota_proto::{AbilitySlot, Order, Target};

    let (mut world, _) = match_hero();
    let ogre = world.spawn_neutral(NeutralKind::OgreMauler, CAMP, 1, 0);
    world.set_team(ogre, Team::Radiant);
    world.owner.insert(ogre, SlotId(0));
    world.neutral_ai.remove(ogre);
    let ability = NeutralAbility::OgreSmash;
    let slot = world
        .abilities
        .get(ogre)
        .unwrap()
        .slots
        .iter()
        .position(|state| state.id == ability.id())
        .unwrap();
    world.settle();
    let target = Target::Pos(CAMP);
    advance_validated(
        &mut world,
        Some(ogre),
        Order::Cast {
            slot: AbilitySlot(slot as u8),
            target,
        },
    );
    assert!(
        matches!(
            world.action.get(ogre).map(|action| action.state),
            Some(crate::game::ActionState::CastAbility {
                phase: crate::game::ActionPhase::Before { progress: 0 },
                ..
            })
        ),
        "the player starts the Ogre's cast"
    );

    let cast_point = ticks_of_ms(1400);
    let view = world.view(Team::Radiant);
    let cast = view
        .unit(crate::game::wire_id(ogre))
        .expect("visible Ogre")
        .cast
        .expect("projected Smash windup");
    assert_eq!(cast.ability, Some(ability.id()));
    assert_eq!(cast.goes_off_in, cast_point);

    assert!(!until_cast(&mut world, ogre, ability, cast_point - 1));
    assert!(
        until_cast(&mut world, ogre, ability, 1),
        "the cast releases at 1.4 seconds"
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
            if let EventKind::AbilityCast { ability, .. } = event.kind() {
                let which = NeutralAbility::of(ability).expect("only neutrals cast here");
                assert!(
                    !matches!(which.spell(), Spell::Uncast),
                    "{which:?} is never cast"
                );
            }
        }
    }
}

#[test]
fn enemy_purge_ignores_slam_as_a_reason_to_cast_and_preserves_it_when_removing_haste() {
    let mut world = day();
    let satyr = world.spawn_neutral(NeutralKind::SatyrTrickster, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(120, 0));
    world.settle();
    put_on(
        &mut world,
        hero,
        ModifierKind::Spell {
            ability: NeutralAbility::RoshanSlam,
            level: 1,
            amount: 0,
        },
        600,
    );
    rouse(&mut world, satyr, hero);
    assert!(
        !until_cast(&mut world, satyr, NeutralAbility::Purge, 60),
        "an enemy slow is not a buff to purge"
    );
    put_on(&mut world, hero, ModifierKind::Hastened { pct: 20 }, 600);
    assert!(until_cast(&mut world, satyr, NeutralAbility::Purge, 120));
    assert!(!carries(&world, hero, ModifierKind::Hastened { pct: 0 }));
    assert!(
        spell_on(&world, hero, NeutralAbility::RoshanSlam).is_some(),
        "enemy Purge leaves harmful effects alone"
    );
}

/// Dota 7.42, Kobold Soldier: "Steal Weapon: Ability Reworked. After a 1s cast time, pierces all
/// enemies in a line, dealing 50/60/70/80 magical damage and applying Break for 3/3.5/4/4.5s to
/// all affected units. Cast Range 500. No Mana Cost. Cooldown: 9s". The game reads the Break from
/// the ability's own `break_duration` at its level (5 s at level four; the patch note says 4.5 s, the script 5 s).
#[test]
fn a_kobold_soldier_casts_steal_weapon_down_a_line_breaking_and_hurting_everyone_on_it() {
    let mut world = day();
    let soldier = world.spawn_neutral(NeutralKind::KoboldTunneler, CAMP, 1, 0);
    let front = bystander(&mut world, CAMP + Vec2::from_ints(200, 0));
    let behind = bystander(&mut world, CAMP + Vec2::from_ints(400, 0));
    let aside = bystander(&mut world, CAMP + Vec2::from_ints(300, 400));
    world.settle();
    let before = [front, behind, aside].map(|hero| health_of(&world, hero));
    rouse(&mut world, soldier, front);
    assert!(
        until_cast(
            &mut world,
            soldier,
            NeutralAbility::StealWeaponCast,
            60 * rules::TICKS_PER_SECOND
        ),
        "it casts at the hero it fights"
    );
    run(&mut world, 3);
    let break_left = |world: &World, unit| {
        world
            .modifiers
            .get(unit)
            .and_then(|on_it| {
                on_it
                    .active()
                    .find(|held| held.kind == ModifierKind::Broken)
                    .and_then(|held| held.ticks_left)
            })
            .expect("a Break with a time")
    };
    let left = break_left(&world, front);
    assert!(
        left > ticks_of_ms(2800) && left <= ticks_of_ms(3000),
        "level one breaks for the script's 3 s, {left} ticks are left"
    );
    assert!(
        carries(&world, front, ModifierKind::Broken),
        "the nearest is broken"
    );
    assert!(
        carries(&world, behind, ModifierKind::Broken),
        "so is the one behind it: it pierces"
    );
    assert!(
        !carries(&world, aside, ModifierKind::Broken),
        "one off the line is not"
    );
    assert!(health_of(&world, front) < before[0] - 30, "the line hurts");
    assert!(health_of(&world, behind) < before[1] - 30, "all along it");
    assert_eq!(health_of(&world, aside), before[2], "and only it");
}

/// Dota 7.42, Mud Golem: "Hurl Boulder ability replaced with Hurl Big Boulder. After a 1s windup,
/// throws a big slow boulder that deals 75/85/95/105 physical damage and stuns all enemies for 2s
/// in a 175 area. Cast Range: 800. No Mana Cost. Cooldown: 15s. Cast Point: 1s".
#[test]
fn a_mud_golem_hurls_a_big_boulder_that_stuns_everyone_within_175_of_where_it_lands() {
    let mut world = day();
    let golem = world.spawn_neutral(NeutralKind::MudGolem, CAMP, 1, 0);
    let hit = bystander(&mut world, CAMP + Vec2::from_ints(300, 0));
    let near = bystander(&mut world, CAMP + Vec2::from_ints(300, 150));
    let far = bystander(&mut world, CAMP + Vec2::from_ints(300, 500));
    world.settle();
    let before = health_of(&world, hit);
    rouse(&mut world, golem, hit);
    assert!(
        until_cast(
            &mut world,
            golem,
            NeutralAbility::HurlBigBoulder,
            60 * rules::TICKS_PER_SECOND
        ),
        "it throws at the hero it fights"
    );
    run(&mut world, 3);
    assert!(
        carries(&world, hit, ModifierKind::Stunned),
        "the hero it aimed at is stunned"
    );
    assert!(
        carries(&world, near, ModifierKind::Stunned),
        "and the one within 175 of it"
    );
    assert!(
        !carries(&world, far, ModifierKind::Stunned),
        "not the one 500 away"
    );
    assert!(health_of(&world, hit) < before - 50, "it hurts");
    let (_, remaining) = world
        .modifiers
        .get(hit)
        .and_then(|on_it| {
            on_it
                .active()
                .find(|held| held.kind == ModifierKind::Stunned)
        })
        .map(|held| (held.kind, held.ticks_left))
        .expect("the stun");
    assert!(
        remaining.is_some_and(|ticks| ticks > ticks_of_ms(1800)),
        "for two seconds"
    );
}

/// Put the next roll in the requested range before letting the real AI take it.
fn stage_neutral_cast_roll(world: &mut World, range: std::ops::Range<u32>) {
    let stream = world.rng.global(Purpose::NeutralCast);
    for _ in 0..512 {
        if range.contains(&stream.clone().below(100)) {
            return;
        }
        stream.below(100);
    }
    panic!("required neutral cast roll was absent");
}

#[test]
fn a_neutral_that_lets_its_chance_pass_tries_again_exactly_eight_seconds_later() {
    let mut world = day();
    let ogre = world.spawn_neutral(NeutralKind::OgreMauler, CAMP, 1, 0);
    let hero = bystander(&mut world, CAMP + Vec2::from_ints(150, 0));
    world.settle();
    set_health_pct(&mut world, ogre, 60);
    stage_neutral_cast_roll(&mut world, 25..100);
    rouse(&mut world, ogre, hero);

    let deadline = (0..30)
        .find_map(|_| {
            world.step();
            world.neutral_cast_wait.get(ogre).copied()
        })
        .expect("the staged miss starts a wait");
    assert_eq!(deadline, world.tick + ticks_of_ms(8000));
    stage_neutral_cast_roll(&mut world, 0..25);
    while world.tick + 1 < deadline {
        world.step();
        assert_eq!(world.neutral_cast_wait.get(ogre), Some(&deadline));
        assert!(world.pending_cast(ogre).is_none());
    }
    world.step();
    assert_eq!(world.tick, deadline);
    assert!(!world.neutral_cast_wait.contains(ogre));
    assert!(
        world.pending_cast(ogre).is_some()
            || matches!(
                world.action.get(ogre).map(|action| action.state),
                Some(crate::game::ActionState::CastAbility { .. })
            )
    );
}

/// The damage `source`'s spell `ability` deals `target` on each tick of the next `ticks` ticks:
/// `(ticks since the call, amount)` for every blow.
fn blows_over(
    world: &mut World,
    source: Entity,
    target: Entity,
    ability: NeutralAbility,
    ticks: u32,
) -> Vec<(u32, i32)> {
    let mut blows = Vec::new();
    for tick in 0..ticks {
        for event in world.step() {
            if let EventKind::Damaged {
                source: Some(by),
                target: on,
                amount,
                inflictor: Some(Inflictor::Ability(id)),
                ..
            } = event.kind()
                && by == wire_id(source)
                && on == wire_id(target)
                && id == ability.id()
            {
                blows.push((tick, amount));
            }
        }
    }
    blows
}

#[test]
fn the_foremans_slam_strikes_three_pulses_300_ahead_each_for_the_whole_damage() {
    let mut world = day();
    let foreman = world.spawn_neutral(NeutralKind::EimermoleForeman, CAMP, 1, 0);
    let facing = world.transform.get(foreman).expect("placed").facing;
    let spot = point_ahead(CAMP, facing, rules::units(300));
    let inside = bystander(&mut world, spot + Vec2::from_ints(0, 150));
    let edge = bystander(&mut world, point_ahead(spot, facing, rules::units(290)));
    let outside = bystander(&mut world, point_ahead(spot, facing, rules::units(340)));
    // The mirror of the spot across the Foreman: 300 behind it.
    let behind = bystander(
        &mut world,
        Vec2 {
            x: CAMP.x + CAMP.x - spot.x,
            y: CAMP.y + CAMP.y - spot.y,
        },
    );
    world.settle();
    assert!(world.cast_spell(foreman, NeutralAbility::MinersSlam, 1, Target::Pos(spot)));
    let pulses = blows_over(&mut world, foreman, inside, NeutralAbility::MinersSlam, 60);
    assert_eq!(pulses.len(), 3, "three pulses: {pulses:?}");
    let ticks: Vec<u32> = pulses.iter().map(|(tick, _)| *tick).collect();
    // The cast goes off just before the first step, whose tick takes the first pulse's blow;
    // the others strike 0.3 s and 0.6 s after the cast, on the ticks they are due.
    let beat = ticks_of_ms(300);
    assert_eq!(ticks, [0, beat - 1, 2 * beat - 1], "0.3 s apart");
    let resistance = world.stats.get(inside).expect("derived").magic_resist_bp;
    let expected = 50 * (rules::NOMINAL_BP - resistance) / rules::NOMINAL_BP;
    assert_eq!(
        pulses.iter().map(|&(_, amount)| amount).collect::<Vec<_>>(),
        [expected; 3]
    );
    let edged = blows_over(&mut world, foreman, edge, NeutralAbility::MinersSlam, 1);
    assert!(edged.is_empty(), "the pulses are over: {edged:?}");
    assert_eq!(health_gap(&world, outside), 0, "340 from the spot is out");
    assert_eq!(
        health_gap(&world, behind),
        0,
        "the spot is ahead, not round the caster"
    );
}

#[test]
fn the_slam_slows_to_a_standstill_for_two_tenths_of_a_second_after_each_pulse() {
    let mut world = day();
    let foreman = world.spawn_neutral(NeutralKind::EimermoleForeman, CAMP, 1, 0);
    let facing = world.transform.get(foreman).expect("placed").facing;
    let spot = point_ahead(CAMP, facing, rules::units(300));
    let hero = bystander(&mut world, spot);
    world.settle();
    let before = world.stats.get(hero).expect("derived").move_speed;
    assert!(world.cast_spell(foreman, NeutralAbility::MinersSlam, 1, Target::Pos(spot)));
    world.step();
    assert!(spell_on(&world, hero, NeutralAbility::MinersSlam).is_some());
    let slowed = world.stats.get(hero).expect("derived").move_speed;
    assert!(slowed < before, "slowed: {slowed:?} against {before:?}");
    let last_pulse = 2 * ticks_of_ms(300);
    for _ in 0..last_pulse + ticks_of_ms(200) + 2 {
        world.step();
    }
    assert!(spell_on(&world, hero, NeutralAbility::MinersSlam).is_none());
}

/// The health a unit has lost of its maximum.
fn health_gap(world: &World, unit: Entity) -> i32 {
    world.stats.get(unit).expect("derived").max_hp.to_int() - health_of(world, unit)
}

#[test]
fn the_seals_flop_leaps_400_twice_landing_each_time_for_250_and_a_standstill_for_1_5_seconds() {
    let mut world = day();
    let seal = world.spawn_neutral(NeutralKind::OgreSeal, CAMP, 1, 0);
    let facing = world.transform.get(seal).expect("placed").facing;
    let aside = Vec2::from_ints(0, 150);
    let first = bystander(
        &mut world,
        point_ahead(CAMP + aside, facing, rules::units(400)),
    );
    let second = bystander(
        &mut world,
        point_ahead(CAMP + aside, facing, rules::units(800)),
    );
    world.settle();
    assert!(world.cast_spell(seal, NeutralAbility::SealFlop, 1, Target::None));
    // 400 at 800 a second speeding up by 1250 a second takes 0.384 s.
    let leap = ticks_of_ms(384);
    let mut landed = Vec::new();
    for tick in 1..=3 * leap {
        for event in world.step() {
            if let EventKind::Damaged {
                source: Some(by),
                target,
                amount,
                inflictor: Some(Inflictor::Ability(id)),
                ..
            } = event.kind()
                && by == wire_id(seal)
                && id == NeutralAbility::SealFlop.id()
            {
                landed.push((tick, target, amount));
            }
        }
    }
    assert_eq!(
        landed.len(),
        2,
        "one landing for each, two leaps in all: {landed:?}"
    );
    assert_eq!(landed[0].1, wire_id(first));
    assert_eq!(landed[1].1, wire_id(second));
    assert!(
        landed[0].0.abs_diff(leap) <= 1 && landed[1].0.abs_diff(2 * leap) <= 1,
        "a leap each {leap} ticks: {landed:?}"
    );
    for (&(_, _, amount), hero) in landed.iter().zip([first, second]) {
        let resistance = world.stats.get(hero).expect("derived").magic_resist_bp;
        let expected = 250 * (rules::NOMINAL_BP - resistance) / rules::NOMINAL_BP;
        assert_eq!(amount, expected);
    }
    let at = world.pos(seal).expect("placed");
    let end = point_ahead(CAMP, facing, rules::units(800));
    assert!(
        at.distance_squared(end) < 50 * 50 * Fixed::ONE.raw as i64 * Fixed::ONE.raw as i64,
        "the seal stands 800 on: {at:?} against {end:?}"
    );
    assert!(!world.push.contains(seal));
}

#[test]
fn the_flops_landing_slows_what_it_catches_for_1_5_seconds_and_no_more() {
    let mut world = day();
    let seal = world.spawn_neutral(NeutralKind::OgreSeal, CAMP, 1, 0);
    let facing = world.transform.get(seal).expect("placed").facing;
    let hero = bystander(
        &mut world,
        point_ahead(CAMP + Vec2::from_ints(0, 150), facing, rules::units(400)),
    );
    world.settle();
    let before = world.stats.get(hero).expect("derived").move_speed;
    assert!(world.cast_spell(seal, NeutralAbility::SealFlop, 1, Target::None));
    for _ in 0..ticks_of_ms(384) + 2 {
        world.step();
    }
    let slowed = world.stats.get(hero).expect("derived").move_speed;
    assert!(slowed < before, "slowed: {slowed:?} against {before:?}");
    for _ in 0..ticks_of_ms(1500) + 2 {
        world.step();
    }
    assert!(spell_on(&world, hero, NeutralAbility::SealFlop).is_none());
}

/// The hits of the Snowball Shot over the next `ticks` ticks, `(tick, who)` in order: a foe's
/// damage names it; an ally's heal shows as health gained.
fn shot_hits(
    world: &mut World,
    caster: Entity,
    ally: Option<Entity>,
    kept_whole: &[Entity],
    ticks: u32,
) -> Vec<(u32, bota_proto::EntityId)> {
    let mut hits = Vec::new();
    let mut ally_health = ally.map(|ally| health_of(world, ally));
    for tick in 0..ticks {
        // A hero that cannot die still stops losing health at one point, and then no blow is
        // told.
        for unit in kept_whole {
            let most = world.stats.get(*unit).expect("derived").max_hp;
            world.health.get_mut(*unit).expect("alive").hp = most;
        }
        for event in world.step() {
            if let EventKind::Damaged {
                source: Some(by),
                target,
                inflictor: Some(Inflictor::Ability(id)),
                ..
            } = event.kind()
                && by == wire_id(caster)
                && id == NeutralAbility::SnowballShot.id()
            {
                hits.push((tick, target));
            }
        }
        if let (Some(ally), Some(before)) = (ally, ally_health) {
            let now = health_of(world, ally);
            if now > before {
                hits.push((tick, wire_id(ally)));
            }
            ally_health = Some(now);
        }
    }
    hits
}

#[test]
fn a_snowball_shot_bounces_four_times_300_milliseconds_apart_never_straight_back_onto_the_unit_it_hit()
 {
    let mut world = day();
    let mage = world.spawn_neutral(NeutralKind::WarpineRaider, CAMP, 1, 0);
    let facing = world.transform.get(mage).expect("placed").facing;
    let near = bystander(&mut world, point_ahead(CAMP, facing, rules::units(570)));
    // 300 on past the first, which puts the mage 870 from it: out of the bounce range of both.
    let far = bystander(&mut world, point_ahead(CAMP, facing, rules::units(870)));
    world.settle();
    let (near_id, far_id) = (wire_id(near), wire_id(far));
    assert!(world.cast_spell(mage, NeutralAbility::SnowballShot, 1, Target::Unit(near_id)));
    let hits = shot_hits(&mut world, mage, None, &[near, far], 60);
    let who: Vec<_> = hits.iter().map(|(_, who)| *who).collect();
    assert_eq!(
        who,
        [near_id, far_id, near_id, far_id, near_id],
        "the first hit and four bounces: {hits:?}"
    );
    // The cast goes off just before the first step, whose tick takes the first hit.
    let beat = ticks_of_ms(300);
    let gaps: Vec<u32> = hits.windows(2).map(|pair| pair[1].0 - pair[0].0).collect();
    assert_eq!(gaps, [beat - 1, beat, beat, beat], "0.3 s between the hits");
}

#[test]
fn a_snowball_shot_heals_an_ally_it_bounces_to_by_its_damage_and_slows_a_foe_for_one_second() {
    let mut world = day();
    let mage = world.spawn_neutral(NeutralKind::WarpineRaider, CAMP, 1, 0);
    let facing = world.transform.get(mage).expect("placed").facing;
    let foe = bystander(&mut world, point_ahead(CAMP, facing, rules::units(570)));
    let ally = world.spawn_neutral(
        NeutralKind::OgreMauler,
        point_ahead(CAMP, facing, rules::units(870)),
        1,
        0,
    );
    world.settle();
    set_health_pct(&mut world, ally, 50);
    let before = health_of(&world, ally);
    let slowed_before = world.stats.get(foe).expect("derived").move_speed;
    assert!(world.cast_spell(
        mage,
        NeutralAbility::SnowballShot,
        1,
        Target::Unit(wire_id(foe))
    ));
    world.step();
    let slowed = world.stats.get(foe).expect("derived").move_speed;
    assert!(
        slowed < slowed_before,
        "{slowed:?} against {slowed_before:?}"
    );
    for _ in 0..ticks_of_ms(300) {
        world.step();
    }
    assert_eq!(
        health_of(&world, ally) - before,
        70,
        "an ally is healed by the shot's damage (the first level's 70)"
    );
    for _ in 0..ticks_of_ms(1000) + 4 * ticks_of_ms(300) {
        world.step();
    }
    assert!(spell_on(&world, foe, NeutralAbility::SnowballShot).is_none());
}

#[test]
fn a_snowball_shot_picks_among_the_units_in_range_at_random_and_the_same_match_picks_the_same() {
    let run = || {
        let mut world = day();
        let mage = world.spawn_neutral(NeutralKind::WarpineRaider, CAMP, 1, 0);
        let facing = world.transform.get(mage).expect("placed").facing;
        let first = bystander(&mut world, point_ahead(CAMP, facing, rules::units(570)));
        let aside = [Vec2::from_ints(0, 200), Vec2::from_ints(0, -200)];
        let others = aside.map(|aside| {
            bystander(
                &mut world,
                point_ahead(CAMP, facing, rules::units(570)) + aside,
            )
        });
        world.settle();
        // The twelve bounces of the fourth level.
        assert!(world.cast_spell(
            mage,
            NeutralAbility::SnowballShot,
            4,
            Target::Unit(wire_id(first))
        ));
        let hits = shot_hits(
            &mut world,
            mage,
            None,
            &[first, others[0], others[1]],
            12 * ticks_of_ms(300) + 20,
        );
        (
            hits,
            [wire_id(first), wire_id(others[0]), wire_id(others[1])],
        )
    };
    let (hits, heroes) = run();
    assert_eq!(hits.len(), 13);
    assert!(
        hits.windows(2).all(|pair| pair[0].1 != pair[1].1),
        "never twice running on one unit"
    );
    for hero in heroes {
        assert!(hits.iter().any(|(_, who)| *who == hero), "{hero:?} is hit");
    }
    assert_eq!(run().0, hits, "the same match draws the same bounces");
}

#[test]
fn a_roused_frostmage_throws_its_snowball_at_a_hero_in_reach() {
    let mut world = day();
    let mage = world.spawn_neutral(NeutralKind::OdobenusFrostmage, CAMP, 1, 0);
    let facing = world.transform.get(mage).expect("placed").facing;
    let hero = bystander(&mut world, point_ahead(CAMP, facing, rules::units(400)));
    world.settle();
    stage_neutral_cast_roll(&mut world, 0..25);
    rouse(&mut world, mage, hero);
    assert!(until_cast(
        &mut world,
        mage,
        NeutralAbility::SnowballShot,
        30
    ));
}
