//! Runes at Dota 7.41f on the Dota map: when and where they appear, who takes
//! them, and what each does to the hero that takes it.
//!
//! Every test plays a match from its setup through the world's own orders
//! and ticks, and reads what a side's view tells. The game clock is moved
//! to the minute a test needs with [`step_at`].

use bota_core::{
    AMPLIFY_DAMAGE_PCT, AMPLIFY_SPELL_AMP_BP, ARCANE_COOLDOWN_CUT_BP, ARCANE_MANA_CUT_BP,
    HASTE_RUNE_SPEED, ILLUSION_DEALT_BP, ILLUSION_RANGED_TAKEN_BP, ModifierKind, POWER_ROTATION,
    SHIELD_RUNE_TICKS, WATER_RUNE_HP, WATER_RUNE_MANA, map_of, rules,
};
use bota_proto::{
    AbilityId, DamageKind, EffectId, EventKind, Fixed, HeroId, MapId, ObjectiveSet, Order, Pick,
    RuneKind, RuneSet, SlotId, StatusFlags, Target, Team, Vec2,
};

use crate::game::{Entity, Event, UnitOrder, World, wire_id};

use super::support::*;

/// A match on the Dota map with every rune it plays and no other objective,
/// its seats Shadow Fiends of the sides given, before any tick.
pub(super) fn rune_match(sides: &[Team]) -> World {
    let cfg = rune_config(sides);
    World::for_match(&cfg, cfg.rng())
}

/// The setup of [`rune_match`].
pub(super) fn rune_config(sides: &[Team]) -> crate::game::MatchConfig {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.runes = RuneSet::ALL;
    cfg.objectives = ObjectiveSet::NONE;
    cfg.picks = sides
        .iter()
        .enumerate()
        .map(|(slot, team)| Pick {
            slot: SlotId(slot as u8),
            team: *team,
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    cfg
}

/// A share of a pool: `num` over `den` of it.
pub(super) fn fraction(pool: Fixed, num: i32, den: i32) -> Fixed {
    Fixed {
        raw: (i64::from(pool.raw) * i64::from(num) / i64::from(den)) as i32,
    }
}

/// Steps the one tick whose game clock is `clock`, whatever came before it.
/// [`step_at`] the game clock's minute `minute` of the world's match, `off` ticks from it.
pub(super) fn step_at_minute(world: &mut World, minute: u32, off: i32) -> Vec<Event> {
    let clock = world
        .schedule()
        .at_minute(minute)
        .saturating_add_signed(off);
    step_at(world, clock)
}

pub(super) fn step_at(world: &mut World, clock: u32) -> Vec<Event> {
    world.tick = clock - 1 - world.clock_shift.game;
    world.step()
}

/// What the spawns among some events were: the kind and the spot.
fn spawned(events: &[Event]) -> Vec<(RuneKind, Vec2)> {
    events
        .iter()
        .filter_map(|event| match event.kind() {
            EventKind::RuneSpawned { kind, at } => Some((kind, at)),
            _ => None,
        })
        .collect()
}

/// The runes lying anywhere, as the view of everything tells.
pub(super) fn lying(world: &World) -> Vec<bota_proto::RuneView> {
    world.view_full().runes
}

/// The hero of a seat.
pub(super) fn hero(world: &World, slot: usize) -> Entity {
    world.seats[slot].unit.expect("standing")
}

/// Puts a hero at a spot, standing still.
pub(super) fn place(world: &mut World, unit: Entity, at: Vec2) {
    world.transform.get_mut(unit).expect("standing").pos = at;
    world.set_order(unit, UnitOrder::Stand);
    world.settle();
}

/// The map's rune spots: the two river spots, then the Dire and the Radiant
/// jungle's.
fn spots() -> [Vec2; 4] {
    let spots = map_of(MapId(0)).rune_spots;
    std::array::from_fn(|at| spots[at].at)
}

const RIVER_TOP: usize = 0;
const RIVER_BOT: usize = 1;
const DIRE_JUNGLE: usize = 2;
const RADIANT_JUNGLE: usize = 3;

/// A rune's event must disclose no more than its snapshot, for either side and either schedule.
#[test]
fn rune_spawn_events_follow_the_same_vision_as_rune_snapshots() {
    for team in [Team::Radiant, Team::Dire] {
        for (kind, spot, minute) in [
            (RuneKind::Water, RIVER_TOP, 2),
            (RuneKind::Bounty, RADIANT_JUNGLE, 4),
        ] {
            assert_spawn_audience(team, kind, spot, minute);
        }
    }
}

fn assert_spawn_audience(team: Team, kind: RuneKind, spot: usize, minute: u32) {
    let mut world = rune_match(&[team]);
    let spot = if kind == RuneKind::Bounty && team == Team::Dire {
        DIRE_JUNGLE
    } else {
        spot
    };
    let at = spots()[spot];
    let viewer = hero(&world, 0);
    place(&mut world, viewer, at + Vec2::from_ints(200, 0));
    let other = if team == Team::Radiant {
        Team::Dire
    } else {
        Team::Radiant
    };
    assert!(
        world.can_see_point(team, at),
        "the watcher sees the rune spot"
    );
    assert!(
        !world.can_see_point(other, at),
        "the other side has no eyes there"
    );
    let events = step_at_minute(&mut world, minute, 0);
    let names_rune = |event: &EventKind| matches!(event, EventKind::RuneSpawned { kind: got, at: where_ } if *got == kind && *where_ == at);
    assert!(
        world
            .view(team)
            .runes
            .iter()
            .any(|rune| rune.kind == kind && rune.pos == at)
    );
    assert!(
        !world.view(other).runes.iter().any(|rune| rune.pos == at),
        "snapshot keeps the hidden rune private"
    );
    assert!(
        crate::game_loop::visible_events(&events, Some(team))
            .iter()
            .any(names_rune),
        "the side with vision hears the spawn"
    );
    assert!(
        !crate::game_loop::visible_events(&events, Some(other))
            .iter()
            .any(names_rune),
        "a hidden rune's kind and position must not leak in its spawn event"
    );
    assert!(
        crate::game_loop::visible_events(&events, None)
            .iter()
            .any(names_rune),
        "the omniscient replay keeps the event"
    );
}

#[test]
fn a_watcher_leaving_vision_on_the_spawn_tick_gets_no_hidden_rune_event() {
    let mut world = rune_match(&[Team::Radiant]);
    let viewer = hero(&world, 0);
    let at = spots()[RIVER_TOP];
    world.tick = world.schedule().at_minute(2) - 1;
    // Find the actual terrain's edge of sight rather than assuming an unobstructed circle.
    let mut last_seen = None;
    for distance in (300..2400).step_by(4) {
        let from = at + Vec2::from_ints(distance, 0);
        place(&mut world, viewer, from);
        if world.can_see_point(Team::Radiant, at) {
            last_seen = Some(from);
        } else if last_seen.is_some() {
            break;
        }
    }
    let from = last_seen.expect("an edge of the rune spot's vision");
    place(&mut world, viewer, from);
    assert!(world.can_see_point(Team::Radiant, at));
    let events = advance_validated(
        &mut world,
        None,
        Order::Move {
            target: Target::Pos(from + Vec2::from_ints(500, 0)),
        },
    );
    assert!(
        world.pos(viewer).unwrap().x > from.x,
        "the real move order took a step"
    );
    assert!(
        !world
            .view(Team::Radiant)
            .runes
            .iter()
            .any(|rune| rune.pos == at),
        "the post-movement snapshot hides this spawn"
    );
    let spawned_here =
        |event: &EventKind| matches!(event, EventKind::RuneSpawned { at: spot, .. } if *spot == at);
    assert!(
        crate::game_loop::visible_events(&events, None)
            .iter()
            .any(spawned_here),
        "the rune really spawned on this tick"
    );
    assert!(
        !crate::game_loop::visible_events(&events, Some(Team::Radiant))
            .iter()
            .any(spawned_here),
        "the event must use the snapshot's post-movement sight"
    );
}

#[test]
fn rune_denial_audience_follows_the_spot_and_always_tells_the_attacker() {
    for side in [Team::Radiant, Team::Dire] {
        let other = if side == Team::Radiant {
            Team::Dire
        } else {
            Team::Radiant
        };
        for (spot_seen, attacker_seen) in [(false, false), (true, true), (true, false)] {
            let mut world = rune_match(&[side, other]);
            let attacker = hero(&world, 0);
            let watcher = hero(&world, 1);
            let at = spots()[RIVER_TOP];
            place(&mut world, attacker, at + Vec2::from_ints(-400, 0));
            if spot_seen {
                let distance = if attacker_seen { 400 } else { 600 };
                place(&mut world, watcher, at + Vec2::from_ints(distance, 0));
            }
            if spot_seen && !attacker_seen {
                world.cheats = true;
                advance_validated(
                    &mut world,
                    None,
                    Order::Cheat {
                        cheat: bota_proto::Cheat::TimeOfDay { night: true },
                    },
                );
            }
            world.put_rune(RuneKind::Haste, RIVER_TOP);
            world.settle();
            assert_eq!(!world.view(other).runes.is_empty(), spot_seen);
            assert_eq!(
                world.sees_now(other, attacker),
                attacker_seen,
                "the fixture distinguishes the rune spot from its attacker"
            );
            let id = lying(&world)[0].id;
            let events = advance_validated(
                &mut world,
                None,
                Order::Attack {
                    target: Target::Unit(id),
                },
            );
            let denied = |event: &EventKind| {
                matches!(
                    event,
                    EventKind::RuneDenied {
                        kind: RuneKind::Haste,
                        ..
                    }
                )
            };
            assert!(lying(&world).is_empty(), "the attack destroyed the rune");
            assert!(
                crate::game_loop::visible_events(&events, Some(side))
                    .iter()
                    .any(denied)
            );
            assert_eq!(
                crate::game_loop::visible_events(&events, Some(other))
                    .iter()
                    .any(denied),
                spot_seen && attacker_seen,
                "a side must see both the spot and attacker before learning its identity"
            );
            assert!(
                crate::game_loop::visible_events(&events, None)
                    .iter()
                    .any(denied)
            );
        }
    }
}

/// Sends a hero to take the rune a view shows, as the order to take it does.
pub(super) fn send_to_take(world: &mut World, taker: Entity, rune: &bota_proto::RuneView) {
    assert!(world.take_item(taker, rune.id), "the hero may take it");
}

/// A match whose first seat's hero stands at the river spot with a rune of a
/// kind lying under it and the order to take it given, one tick before it is
/// taken. The hero takes nothing unasked: the order is what takes it.
pub(super) fn rune_under_hero(sides: &[Team], kind: RuneKind) -> (World, Entity) {
    let mut world = rune_match(sides);
    let taker = hero(&world, 0);
    world.put_rune(kind, RIVER_TOP);
    place(&mut world, taker, spots()[RIVER_TOP]);
    let rune = lying(&world)[0];
    send_to_take(&mut world, taker, &rune);
    (world, taker)
}

/// A match whose first seat's hero has taken a rune of a kind.
fn took(sides: &[Team], kind: RuneKind) -> (World, Entity) {
    let (mut world, taker) = rune_under_hero(sides, kind);
    let events = world.step();
    assert!(
        events.iter().any(|event| matches!(
            event.kind(),
            EventKind::RuneActivated { kind: got, bottled: false, .. } if got == kind
        )),
        "the rune took effect on the tick the hero was ordered to take it standing on it"
    );
    assert!(lying(&world).is_empty(), "and is gone");
    (world, taker)
}

fn radiant_and_dire() -> [Team; 2] {
    [Team::Radiant, Team::Dire]
}

#[test]
fn the_horn_brings_a_bounty_rune_to_each_of_the_four_spots() {
    // Owner-confirmed current Dota: runes appear at all 4 spots at 0:00.
    let mut world = rune_match(&radiant_and_dire());
    let horn = world.schedule().at_minute(0);
    assert!(
        spawned(&step_at(&mut world, horn - 1)).is_empty(),
        "nothing before the horn"
    );
    let events = step_at(&mut world, horn);
    let mut got = spawned(&events);
    got.sort();
    let mut want: Vec<(RuneKind, Vec2)> = spots()
        .into_iter()
        .map(|at| (RuneKind::Bounty, at))
        .collect();
    want.sort();
    assert_eq!(got, want, "the river pair and the jungle's");
    let gold: Vec<Option<i32>> = lying(&world).iter().map(|rune| rune.gold).collect();
    assert_eq!(gold, vec![Some(40); 4], "worth forty gold at the horn");
}

#[test]
fn water_comes_at_two_and_four_minutes_to_both_river_spots_and_goes_with_the_next_river_spawn() {
    let mut world = rune_match(&radiant_and_dire());
    let river = [spots()[RIVER_TOP], spots()[RIVER_BOT]];
    for minute in [2, 4] {
        let before = world.schedule().at_minute(minute) - 1;
        assert!(spawned(&step_at(&mut world, before)).is_empty());
        let events = step_at_minute(&mut world, minute, 0);
        let water: Vec<Vec2> = spawned(&events)
            .into_iter()
            .filter(|(kind, _)| *kind == RuneKind::Water)
            .map(|(_, at)| at)
            .collect();
        assert_eq!(water.len(), 2, "two Water Runes at {minute}:00");
        for at in river {
            assert!(water.contains(&at), "one at each river spot");
        }
    }
    assert_eq!(
        lying(&world)
            .iter()
            .filter(|rune| rune.kind == RuneKind::Water)
            .count(),
        2,
        "the Water Runes of 2:00 went with those of 4:00"
    );
    // At 6:00 the river turns over again, to a power rune, and no more Water.
    let events = step_at_minute(&mut world, 6, 0);
    assert!(
        lying(&world)
            .iter()
            .all(|rune| rune.kind != RuneKind::Water)
    );
    assert!(
        spawned(&events)
            .iter()
            .all(|(kind, _)| *kind != RuneKind::Water),
        "Water Runes are no longer created"
    );
}

#[test]
fn the_first_water_spawn_replaces_opening_river_bounties_and_keeps_jungle_bounties() {
    let mut world = rune_match(&radiant_and_dire());
    step_at_minute(&mut world, 0, 0);
    let opening = lying(&world);
    assert_eq!(opening.len(), 4);
    step_at_minute(&mut world, 2, -1);
    assert_eq!(
        lying(&world),
        opening,
        "opening runes remain until the turnover"
    );
    step_at_minute(&mut world, 2, 0);
    let next = lying(&world);
    for rune in opening {
        let river = [spots()[RIVER_TOP], spots()[RIVER_BOT]].contains(&rune.pos);
        assert_eq!(
            next.iter().any(|r| r.id == rune.id),
            !river,
            "river spawners replace their opening bounties; jungle spawners retain them"
        );
    }
    assert_eq!(next.iter().filter(|r| r.kind == RuneKind::Water).count(), 2);
    assert_eq!(
        next.iter().filter(|r| r.kind == RuneKind::Bounty).count(),
        2
    );
}

#[test]
fn each_river_runes_advertised_expiry_is_its_first_absent_tick() {
    let mut world = rune_match(&[Team::Radiant]);
    world.creeps_halted = true;
    step_at_minute(&mut world, 0, 0);
    for (minute, count) in [(2, 2), (4, 2), (6, 2), (8, 1), (10, 1)] {
        let runes: Vec<_> = lying(&world)
            .into_iter()
            .filter(|r| r.expires_at.is_some())
            .collect();
        assert_eq!(
            runes.len(),
            count,
            "all river runes advertise expiry before {minute}:00"
        );
        let expected = world.schedule().at_minute(minute);
        let mut first_absent = std::collections::BTreeMap::new();
        for rune in &runes {
            assert_eq!(rune.expires_at, Some(expected));
        }
        while world.clock() < expected {
            world.step();
            let present = lying(&world);
            for rune in &runes {
                if !present.iter().any(|r| r.id == rune.id) {
                    first_absent.entry(rune.id).or_insert(world.clock());
                }
            }
        }
        for rune in runes {
            assert_eq!(
                first_absent.get(&rune.id).copied(),
                rune.expires_at,
                "the advertised deadline equals the first absence, not a later observation"
            );
        }
    }
}

#[test]
fn bounty_runes_come_to_the_jungles_every_four_minutes_worth_six_more_each_time_and_never_go() {
    let mut world = rune_match(&radiant_and_dire());
    step_at_minute(&mut world, 0, 0);
    let jungle = [spots()[DIRE_JUNGLE], spots()[RADIANT_JUNGLE]];
    for (minute, gold) in [(4, 46), (8, 52), (12, 58)] {
        let before: std::collections::BTreeSet<_> = lying(&world).iter().map(|r| r.id).collect();
        let events = step_at_minute(&mut world, minute, 0);
        let got: Vec<Vec2> = spawned(&events)
            .into_iter()
            .filter(|(kind, _)| *kind == RuneKind::Bounty)
            .map(|(_, at)| at)
            .collect();
        assert_eq!(got.len(), jungle.len());
        for origin in jungle {
            assert_eq!(
                got.iter()
                    .filter(|at| at.within(origin, rules::units(200)))
                    .count(),
                1,
                "a new Bounty Rune beside each jungle spawner at {minute}:00"
            );
        }
        let worth: Vec<i32> = lying(&world)
            .iter()
            .filter(|rune| {
                rune.pos.within(jungle[0], rules::units(200)) && !before.contains(&rune.id)
            })
            .filter_map(|rune| rune.gold)
            .collect();
        assert_eq!(
            worth,
            [gold],
            "the new rune is worth {gold} gold, fixed now"
        );
    }
    let at_jungle = lying(&world)
        .iter()
        .filter(|rune| rune.pos.within(jungle[0], rules::units(200)))
        .count();
    assert_eq!(
        at_jungle, 4,
        "the opening one and the three since, all still there"
    );
    let at_river = lying(&world)
        .iter()
        .filter(|rune| rune.pos == spots()[RIVER_TOP] && rune.kind == RuneKind::Bounty)
        .count();
    assert_eq!(at_river, 0, "the river replaces its opening pair");
}

#[test]
fn a_power_rune_comes_every_two_minutes_from_six_to_a_river_spot_and_never_repeats_in_a_cycle() {
    let mut world = rune_match(&radiant_and_dire());
    let river = [spots()[RIVER_TOP], spots()[RIVER_BOT]];
    assert!(
        spawned(&step_at_minute(&mut world, 6, -1)).is_empty(),
        "none before six minutes"
    );
    let mut kinds = Vec::new();
    for round in 0..18 {
        let events = step_at_minute(&mut world, 6 + 2 * round, 0);
        let power: Vec<(RuneKind, Vec2)> = spawned(&events)
            .into_iter()
            .filter(|(kind, _)| kind.is_power())
            .collect();
        assert_eq!(power.len(), 1, "one power rune at {}:00", 6 + 2 * round);
        assert!(river.contains(&power[0].1), "at a river spot");
        let lone: Vec<_> = lying(&world)
            .into_iter()
            .filter(|rune| rune.kind.is_power())
            .collect();
        assert_eq!(lone.len(), 1, "and the last one went");
        kinds.push(power[0].0);
    }
    let cycle_len = POWER_ROTATION.iter().count();
    for cycle in kinds.chunks(cycle_len) {
        let mut seen = RuneSet::NONE;
        for kind in cycle {
            assert!(!seen.has(*kind), "{kind:?} twice in a cycle: {kinds:?}");
            seen = seen.with(*kind);
        }
        assert_eq!(
            seen.bits, POWER_ROTATION.bits,
            "all {cycle_len} kinds in a cycle"
        );
    }
    assert_ne!(
        kinds[cycle_len - 1],
        kinds[cycle_len],
        "a new cycle never opens with the last"
    );
}

#[test]
fn both_river_spots_get_power_runes_over_a_match() {
    let mut world = rune_match(&radiant_and_dire());
    let mut at_spots = std::collections::HashSet::new();
    for round in 0..14 {
        let events = step_at_minute(&mut world, 6 + 2 * round, 0);
        at_spots.extend(
            spawned(&events)
                .into_iter()
                .filter(|(kind, _)| kind.is_power())
                .map(|(_, at)| at),
        );
    }
    assert_eq!(at_spots.len(), 2, "drawn between the two");
}

#[test]
fn an_illusion_rune_never_spawns_from_the_river_while_every_other_power_kind_does() {
    let mut world = rune_match(&radiant_and_dire());
    assert!(
        world.runes.has(RuneKind::Illusion),
        "the match asks for Illusion Runes"
    );
    let mut power = Vec::new();
    for round in 0..28 {
        let events = step_at_minute(&mut world, 6 + 2 * round, 0);
        power.extend(
            spawned(&events)
                .into_iter()
                .map(|(kind, _)| kind)
                .filter(|kind| kind.is_power()),
        );
    }
    assert_eq!(
        power.len(),
        28,
        "a power rune every two minutes, six to sixty"
    );
    assert!(
        !power.contains(&RuneKind::Illusion),
        "no Illusion Rune spawns: {power:?}"
    );
    for kind in [
        RuneKind::Haste,
        RuneKind::AmplifyDamage,
        RuneKind::Arcane,
        RuneKind::Regeneration,
        RuneKind::Invisibility,
        RuneKind::Shield,
    ] {
        assert!(power.contains(&kind), "{kind:?} still spawns: {power:?}");
    }
}

/// Dota's 1v1 Solo Mid skips the first event of each cycle [SRV, see `bota_core::runes_due`]: the
/// horn's set of Bounty Runes and the 2:00 Water event. What follows is the game's own schedule:
/// bounty at 4:00 and every four minutes, water at 4:00 alone, power runes from 6:00 every two.
#[test]
fn solo_mid_asked_for_runes_skips_the_first_events_and_plays_the_rest_of_the_schedule() {
    let mut cfg = config();
    cfg.map = MapId(2);
    cfg.runes = RuneSet::ALL;
    cfg.objectives = ObjectiveSet::NONE;
    let mut world = World::for_match(&cfg, cfg.rng());
    let at = |world: &mut World, minute| spawned(&step_at_minute(world, minute, 0));
    assert!(
        at(&mut world, 0).is_empty(),
        "\"First rune does not spawn\""
    );
    assert!(
        at(&mut world, 2).is_empty(),
        "nor does the first Water event, the one of 2:00"
    );
    let four = at(&mut world, 4);
    let kinds =
        |spawns: &[(RuneKind, Vec2)], kind| spawns.iter().filter(|(k, _)| *k == kind).count();
    assert_eq!(
        kinds(&four, RuneKind::Water),
        2,
        "water at 4:00, at both river spots"
    );
    assert_eq!(
        kinds(&four, RuneKind::Bounty),
        2,
        "and the jungles' first bounties"
    );
    let six = at(&mut world, 6);
    assert_eq!(six.len(), 1, "a power rune at 6:00");
    assert!(
        lying(&world)
            .iter()
            .all(|rune| rune.kind != RuneKind::Water),
        "the water of 4:00 went with the river's turn at 6:00"
    );
    assert_eq!(
        at(&mut world, 8).len(),
        3,
        "a power rune and the two next bounties at 8:00"
    );
    assert_eq!(map_of(MapId(2)).default_runes, RuneSet::NONE);
    assert!(
        crate::game::check_match_rules(MapId(2), ObjectiveSet::NONE, RuneSet::NONE).is_ok(),
        "solo mid spawns none unless its host asks"
    );
}

/// The full map plays every event: water at 2:00 and 4:00 as well.
#[test]
fn the_full_map_plays_its_first_events_the_horns_bounties_and_the_water_of_two_minutes() {
    let mut world = rune_match(&radiant_and_dire());
    assert_eq!(spawned(&step_at_minute(&mut world, 0, 0)).len(), 4);
    assert_eq!(
        spawned(&step_at_minute(&mut world, 2, 0))
            .iter()
            .filter(|(kind, _)| *kind == RuneKind::Water)
            .count(),
        2
    );
}

#[test]
fn a_hero_standing_on_a_rune_takes_nothing_unasked() {
    let mut world = rune_match(&radiant_and_dire());
    let hero_at = hero(&world, 0);
    world.put_rune(RuneKind::Haste, RIVER_TOP);
    place(&mut world, hero_at, spots()[RIVER_TOP]);
    let events = {
        let mut events = Vec::new();
        for _ in 0..5 * rules::TICKS_PER_SECOND {
            events.extend(world.step());
        }
        events
    };
    assert_eq!(lying(&world).len(), 1, "the rune still lies under the hero");
    assert!(!carries(&world, hero_at, ModifierKind::RuneHaste));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind(), EventKind::RuneActivated { .. })),
        "and nothing was told of a pickup"
    );
}

#[test]
fn an_ordered_hero_takes_a_rune_from_within_150_units_and_a_courier_never_does() {
    let mut world = rune_match(&radiant_and_dire());
    let taker = hero(&world, 0);
    let courier = the_courier(&world);
    world.put_rune(RuneKind::Haste, RIVER_TOP);
    let spot = spots()[RIVER_TOP];
    // A courier standing on it takes nothing, and may not be told to.
    world.transform.get_mut(courier).expect("standing").pos = spot;
    run(&mut world, 3);
    assert_eq!(lying(&world).len(), 1, "a courier takes no rune");
    let rune = wire_id(
        world
            .entities
            .iter()
            .find(|&entity| world.rune.get(entity).is_some())
            .expect("a rune"),
    );
    assert!(!world.take_item(courier, rune), "a courier cannot be sent");
    // A hero 140 away takes it the tick the order is carried out.
    let reach = Vec2::from_ints(spot.x.to_int() + 140, spot.y.to_int());
    place(&mut world, taker, reach);
    assert!(world.take_item(taker, rune));
    world.step();
    assert!(lying(&world).is_empty(), "taken from 140 units");
    assert!(carries(&world, taker, ModifierKind::RuneHaste));
}

#[test]
fn an_order_to_take_a_rune_walks_the_hero_to_it() {
    let mut world = rune_match(&radiant_and_dire());
    let taker = hero(&world, 0);
    world.put_rune(RuneKind::Arcane, RIVER_TOP);
    let spot = spots()[RIVER_TOP];
    let away = Vec2::from_ints(spot.x.to_int() - 600, spot.y.to_int());
    place(&mut world, taker, away);
    world.step();
    let id = lying(&world)[0].id;
    assert_ne!(
        lying(&world).len(),
        0,
        "the hero sees it from where it stands"
    );
    let order = Order::Take {
        target: Target::Unit(id),
    };
    advance_validated(&mut world, None, order);
    let mut ticks = 1;
    while !lying(&world).is_empty() {
        world.step();
        ticks += 1;
        assert!(ticks < 200, "the hero never reached it");
    }
    assert!(carries(&world, taker, ModifierKind::RuneArcane));
    assert!(ticks > 20, "it took a walk, not a blink: {ticks} ticks");
}

#[test]
fn only_a_hero_may_be_ordered_to_take_a_rune() {
    let mut world = rune_match(&radiant_and_dire());
    world.put_rune(RuneKind::Arcane, RIVER_TOP);
    let taker = hero(&world, 0);
    place(&mut world, taker, spots()[RIVER_TOP]);
    let courier = the_courier(&world);
    world.step();
    // The hero took it already; lay another beside the courier.
    world.put_rune(RuneKind::Arcane, RIVER_BOT);
    let courier_at = spots()[RIVER_BOT];
    world.transform.get_mut(courier).expect("standing").pos = courier_at;
    run(&mut world, 2);
    let id = lying(&world)[0].id;
    let order = Order::Take {
        target: Target::Unit(id),
    };
    let other = hero(&world, 1);
    place(&mut world, other, courier_at);
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &order),
        Err(bota_proto::RejectReason::WrongTargetKind),
        "a courier cannot"
    );
}

#[test]
fn an_attack_order_destroys_a_rune_for_neither_a_last_hit_nor_a_deny() {
    let mut world = rune_match(&radiant_and_dire());
    let attacker = hero(&world, 0);
    world.put_rune(RuneKind::Haste, RIVER_TOP);
    let spot = spots()[RIVER_TOP];
    let away = Vec2::from_ints(spot.x.to_int() - 500, spot.y.to_int());
    place(&mut world, attacker, away);
    world.step();
    let id = lying(&world)[0].id;
    let order = Order::Attack {
        target: Target::Unit(id),
    };
    let mut told = Vec::new();
    told.extend(advance_validated(&mut world, None, order));
    for _ in 0..200 {
        if lying(&world).is_empty() {
            break;
        }
        told.extend(world.step());
    }
    assert!(lying(&world).is_empty(), "the rune is gone");
    assert!(told.iter().any(|event| matches!(
        event.kind(),
        EventKind::RuneDenied {
            kind: RuneKind::Haste,
            ..
        }
    )));
    assert!(!carries(&world, attacker, ModifierKind::RuneHaste));
    assert_eq!(world.seats[0].last_hits, 0);
    assert_eq!(world.seats[0].denies, 0);
}

#[test]
fn a_fogged_rune_is_not_told_to_the_side_that_cannot_see_it() {
    let mut world = rune_match(&radiant_and_dire());
    world.put_rune(RuneKind::Shield, RIVER_TOP);
    run(&mut world, 3);
    assert!(
        world.view(Team::Radiant).runes.is_empty(),
        "no hero of either side is near"
    );
    assert!(world.view(Team::Dire).runes.is_empty());
    assert_eq!(world.view_full().runes.len(), 1);
    let taker = hero(&world, 1);
    place(&mut world, taker, Vec2::from_ints(7576 + 700, 10328));
    world.step();
    assert_eq!(world.view(Team::Dire).runes.len(), 1, "seen from 700 away");
    assert!(world.view(Team::Radiant).runes.is_empty());
}

#[test]
fn a_water_rune_mends_forty_health_and_eighty_mana_at_once() {
    let (mut world, taker) = rune_under_hero(&radiant_and_dire(), RuneKind::Water);
    let stats = *world.stats.get(taker).expect("stats");
    world.health.get_mut(taker).expect("hp").hp = fraction(stats.max_hp, 1, 2);
    world.mana.get_mut(taker).expect("mana").mana = fraction(stats.max_mana, 1, 2);
    let (hp, mana) = (
        world.health.get(taker).expect("hp").hp,
        world.mana.get(taker).expect("mana").mana,
    );
    world.step();
    let gained = |now: Fixed, then: Fixed| (now - then).to_int();
    let regen_hp = stats.hp_regen.to_int() + 1;
    let regen_mana = stats.mana_regen.to_int() + 1;
    let now_hp = world.health.get(taker).expect("hp").hp;
    let now_mana = world.mana.get(taker).expect("mana").mana;
    assert!(
        (WATER_RUNE_HP..=WATER_RUNE_HP + regen_hp).contains(&gained(now_hp, hp)),
        "forty health: {}",
        gained(now_hp, hp)
    );
    assert!(
        (WATER_RUNE_MANA..=WATER_RUNE_MANA + regen_mana).contains(&gained(now_mana, mana)),
        "eighty mana: {}",
        gained(now_mana, mana)
    );
}

#[test]
fn a_bounty_rune_pays_every_hero_of_the_side_that_takes_it_and_nobody_else() {
    let sides = [Team::Radiant, Team::Radiant, Team::Dire];
    let mut world = rune_match(&sides);
    step_at_minute(&mut world, 8, 0);
    // The jungle rune of 8:00 is worth 52; the opening ones are 40.
    let before: Vec<i32> = world.seats.iter().map(|seat| seat.gold).collect();
    let nw: Vec<i32> = world.seats.iter().map(|seat| seat.net_worth).collect();
    let taker = hero(&world, 0);
    let jungle = spots()[RADIANT_JUNGLE];
    let fresh = lying(&world)
        .into_iter()
        .filter(|rune| rune.pos == jungle)
        .map(|rune| rune.gold)
        .max();
    assert_eq!(fresh, Some(Some(52)));
    place(&mut world, taker, jungle);
    // Two runes lie at the hero's feet (8:00's and 0:00's are 12 minutes'
    // worth apart); the order names the fresh one.
    let fresh = lying(&world)
        .into_iter()
        .find(|rune| rune.gold == Some(52) && rune.pos == jungle)
        .expect("the fresh rune");
    send_to_take(&mut world, taker, &fresh);
    world.step();
    let after: Vec<i32> = world.seats.iter().map(|seat| seat.gold).collect();
    let paid: Vec<i32> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
    let passive = paid[2];
    // The Dire seat's gain is the clock's alone; the Radiant seats' add the
    // runes at the jungle spot: the 4:00 one is not there (none was put out
    // before 8:00 in this world), so exactly the 52 of the 8:00 one.
    assert_eq!(paid[0] - passive, 52, "the taker is paid");
    assert_eq!(paid[1] - passive, 52, "so is its ally, wherever it stands");
    let worth: Vec<i32> = world
        .seats
        .iter()
        .zip(&nw)
        .map(|(seat, was)| seat.net_worth - was)
        .collect();
    assert_eq!(worth[0] - worth[2], 52, "and it counts toward net worth");
}

#[test]
fn a_haste_rune_sets_the_speed_to_550_for_22_seconds_and_nothing_slows_below_it() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Haste);
    world.settle();
    assert_eq!(
        world.stats.get(taker).expect("stats").move_speed.to_int(),
        HASTE_RUNE_SPEED
    );
    put_on(&mut world, taker, ModifierKind::Slowed { pct: 60 }, 600);
    world.settle();
    assert_eq!(
        world.stats.get(taker).expect("stats").move_speed.to_int(),
        HASTE_RUNE_SPEED,
        "a slow cannot take it lower"
    );
    run(&mut world, 22 * rules::TICKS_PER_SECOND);
    assert!(!carries(&world, taker, ModifierKind::RuneHaste));
    assert!(world.stats.get(taker).expect("stats").move_speed.to_int() < HASTE_RUNE_SPEED);
}

#[test]
fn an_amplify_damage_rune_adds_eighty_percent_of_base_damage_and_fifteen_percent_spell_amp() {
    let (mut world, taker) = rune_under_hero(&radiant_and_dire(), RuneKind::AmplifyDamage);
    world.settle();
    let before = *world.stats.get(taker).expect("stats");
    world.step();
    world.settle();
    let after = *world.stats.get(taker).expect("stats");
    assert_eq!(
        after.damage,
        before.damage + before.damage * AMPLIFY_DAMAGE_PCT / 100
    );
    assert_eq!(
        after.magic_amp_bp,
        before.magic_amp_bp + AMPLIFY_SPELL_AMP_BP
    );
    assert_eq!(after.pure_amp_bp, before.pure_amp_bp + AMPLIFY_SPELL_AMP_BP);
    run(&mut world, 45 * rules::TICKS_PER_SECOND);
    assert!(!carries(&world, taker, ModifierKind::RuneAmplifyDamage));
    assert_eq!(world.stats.get(taker).expect("stats").damage, before.damage);
}

#[test]
fn an_arcane_rune_cuts_cooldowns_a_quarter_and_mana_costs_thirty_percent() {
    let (mut world, taker) = rune_under_hero(&radiant_and_dire(), RuneKind::Arcane);
    world.settle();
    let base = world.mana_cost_of_raze(taker);
    world.step();
    world.settle();
    let stats = *world.stats.get(taker).expect("stats");
    assert_eq!(
        stats.cooldown_rate_bp,
        rules::NOMINAL_BP - ARCANE_COOLDOWN_CUT_BP
    );
    assert_eq!(
        stats.mana_cost_rate_bp,
        rules::NOMINAL_BP - ARCANE_MANA_CUT_BP
    );
    let cut = world.mana_cost_of_raze(taker);
    assert_eq!(cut, base * 7 / 10, "a raze costs 70 percent of its mana");
    run(&mut world, 50 * rules::TICKS_PER_SECOND);
    assert!(!carries(&world, taker, ModifierKind::RuneArcane));
    assert_eq!(world.mana_cost_of_raze(taker), base);
}

impl World {
    /// What the nearest raze would cost the hero now.
    fn mana_cost_of_raze(&self, hero: Entity) -> i32 {
        self.ability_mana_cost(hero, AbilityId::RAZE_NEAR, 1)
    }
}

#[test]
fn a_regeneration_rune_mends_six_percent_a_second_damped_to_one_by_a_hero_and_ends_when_full() {
    let (mut world, taker) = rune_under_hero(&radiant_and_dire(), RuneKind::Regeneration);
    let enemy = hero(&world, 1);
    let stats = *world.stats.get(taker).expect("stats");
    world.health.get_mut(taker).expect("hp").hp = fraction(stats.max_hp, 1, 4);
    world.mana.get_mut(taker).expect("mana").mana = fraction(stats.max_mana, 1, 4);
    world.step();
    let second = rules::TICKS_PER_SECOND;
    let hp = |world: &World| world.health.get(taker).expect("hp").hp;
    let mana = |world: &World| world.mana.get(taker).expect("mana").mana;
    let (hp0, mana0) = (hp(&world), mana(&world));
    run(&mut world, second);
    let (gained_hp, gained_mana) = ((hp(&world) - hp0).to_int(), (mana(&world) - mana0).to_int());
    let share = |max: Fixed, pct: i32| max.to_int() * pct / 100;
    assert!(
        (share(stats.max_hp, 6)..=share(stats.max_hp, 6) + 4).contains(&gained_hp),
        "six percent of the health a second: {gained_hp} of {}",
        stats.max_hp.to_int()
    );
    assert!(
        (share(stats.max_mana, 6)..=share(stats.max_mana, 6) + 4).contains(&gained_mana),
        "six percent of the mana a second: {gained_mana}"
    );
    // A blow from a hero damps it to one percent for three seconds.
    world.push_hit(Some(enemy), taker, 20, DamageKind::Pure, None);
    world.step();
    let (hp1, mana1) = (hp(&world), mana(&world));
    run(&mut world, second);
    let damped_hp = (hp(&world) - hp1).to_int();
    assert!(
        damped_hp <= share(stats.max_hp, 1) + 4,
        "one percent while damped: {damped_hp}"
    );
    assert!((mana(&world) - mana1).to_int() <= share(stats.max_mana, 1) + 4);
    run(&mut world, 3 * second);
    let (hp2, _) = (hp(&world), mana(&world));
    run(&mut world, second);
    assert!(
        (hp(&world) - hp2).to_int() >= share(stats.max_hp, 5),
        "back at six percent after the three seconds"
    );
    // Full in both pools it is over.
    world.fill_pools(taker);
    run(&mut world, 2);
    assert!(!carries(
        &world,
        taker,
        ModifierKind::RuneRegeneration { damped: 0 }
    ));
}

#[test]
fn a_creep_or_a_tower_does_not_damp_a_regeneration_rune() {
    let (mut world, taker) = rune_under_hero(&radiant_and_dire(), RuneKind::Regeneration);
    let stats = *world.stats.get(taker).expect("stats");
    world.health.get_mut(taker).expect("hp").hp = fraction(stats.max_hp, 1, 4);
    world.step();
    let creep = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(100, 100));
    world.push_hit(Some(creep), taker, 20, DamageKind::Pure, None);
    world.step();
    let held = world
        .modifiers
        .get(taker)
        .and_then(|on_it| {
            on_it.active().find_map(|held| match held.kind {
                ModifierKind::RuneRegeneration { damped } => Some(damped),
                _ => None,
            })
        })
        .expect("still regenerating");
    assert_eq!(held, 0, "damped by players only");
}

#[test]
fn an_invisibility_rune_fades_the_hero_from_the_other_side_after_two_seconds_until_it_attacks() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Invisibility);
    let seen_by_dire = |world: &World| {
        world
            .view(Team::Dire)
            .units
            .iter()
            .any(|unit| unit.id == wire_id(taker))
    };
    // The Dire hero stands beside, so that it could see the Radiant one.
    let watcher = hero(&world, 1);
    let at = world.pos(taker).expect("standing");
    place(
        &mut world,
        watcher,
        Vec2::from_ints(at.x.to_int() + 300, at.y.to_int()),
    );
    run(&mut world, rules::TICKS_PER_SECOND);
    assert!(seen_by_dire(&world), "still seen while it fades");
    run(&mut world, 2 * rules::TICKS_PER_SECOND);
    assert!(!seen_by_dire(&world), "gone from the other side's view");
    let own = world.view(Team::Radiant);
    let unit = own
        .units
        .iter()
        .find(|unit| unit.id == wire_id(taker))
        .expect("its own side sees it");
    assert!(unit.statuses.bits & StatusFlags::INVISIBLE != 0);
    // An attack breaks it, at the attack point.
    let creep = a_creep_at(
        &mut world,
        Team::Dire,
        Vec2::from_ints(at.x.to_int() + 150, at.y.to_int()),
    );
    attack_click(&mut world, creep);
    let mut ticks = 0;
    while carries(&world, taker, ModifierKind::RuneInvisibility) {
        world.step();
        ticks += 1;
        assert!(ticks < 120, "the attack never came");
    }
    world.step();
    assert!(seen_by_dire(&world), "seen again once it struck");
}

#[test]
fn a_cast_breaks_an_invisibility_rune() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Invisibility);
    world
        .level
        .insert(taker, crate::game::Level(rules::HERO_MAX_LEVEL));
    world.settle();
    world.fill_pools(taker);
    assert!(world.learn(taker, 0));
    run(&mut world, 3 * rules::TICKS_PER_SECOND);
    assert!(carries(&world, taker, ModifierKind::RuneInvisibility));
    cast(&mut world, 0, Target::None);
    run(&mut world, 20);
    assert!(
        !carries(&world, taker, ModifierKind::RuneInvisibility),
        "casting an ability lets it go"
    );
}

#[test]
fn a_shield_rune_takes_half_the_maximum_health_of_every_kind_of_damage_before_health() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Shield);
    let max = world.stats.get(taker).expect("stats").max_hp.to_int();
    let barrier = max * 50 / 100;
    let health = |world: &World| world.health.get(taker).expect("hp").hp.to_int();
    let before = health(&world);
    world.push_hit(None, taker, barrier / 2, DamageKind::Pure, None);
    world.step();
    assert_eq!(health(&world), before, "the barrier took it");
    let left = world
        .view_full()
        .units
        .iter()
        .find(|unit| unit.id == wire_id(taker))
        .and_then(|unit| {
            unit.effects
                .iter()
                .find(|effect| effect.id == EffectId::RUNE_SHIELD)
                .and_then(|effect| effect.stacks)
        })
        .expect("told with what is left");
    assert_eq!(left as i32, barrier - barrier / 2);
    world.push_hit(None, taker, barrier, DamageKind::Physical, None);
    world.step();
    assert!(
        health(&world) < before,
        "what the barrier could not take hurt"
    );
    assert!(
        !carries(&world, taker, ModifierKind::RuneShield { left: 0 }),
        "and it is gone"
    );
}

#[test]
fn a_shield_rune_runs_out_after_seventy_five_seconds() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Shield);
    run(&mut world, SHIELD_RUNE_TICKS - 5);
    assert!(carries(&world, taker, ModifierKind::RuneShield { left: 0 }));
    run(&mut world, 10);
    assert!(!carries(
        &world,
        taker,
        ModifierKind::RuneShield { left: 0 }
    ));
}

#[test]
fn an_illusion_rune_makes_two_illusions_that_deal_35_percent_and_take_300_percent() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Illusion);
    let made: Vec<Entity> = world
        .entities
        .iter()
        .filter(|&entity| {
            world
                .illusion
                .get(entity)
                .is_some_and(|made| made.of == taker)
        })
        .collect();
    assert_eq!(made.len(), 2, "two illusions");
    for &illusion in &made {
        assert_eq!(world.owner.get(illusion), Some(&SlotId(0)));
        assert_eq!(world.team.get(illusion), Some(&Team::Radiant));
        assert!(world.wears_illusion(illusion));
        let stats = world.stats.get(illusion).expect("stats");
        assert_eq!(
            stats.physical_amp_bp,
            rules::NOMINAL_BP * ILLUSION_DEALT_BP / rules::NOMINAL_BP
        );
        assert_eq!(stats.damage_taken_bp, ILLUSION_RANGED_TAKEN_BP);
        assert_ne!(
            world.pos(illusion),
            world.pos(taker),
            "beside the hero, not in it"
        );
    }
    // Three hundred percent of a hundred points of pure damage.
    let health = |world: &World, who: Entity| world.health.get(who).expect("hp").hp.to_int();
    let before = health(&world, made[0]);
    world.push_hit(None, made[0], 100, DamageKind::Pure, None);
    world.step();
    assert_eq!(before - health(&world, made[0]), 300);
    // An illusion casts nothing and uses no item.
    assert_eq!(
        world.validate_order(
            SlotId(0),
            Some(wire_id(made[1])),
            &Order::Cast {
                slot: bota_proto::AbilitySlot(0),
                target: Target::None
            }
        ),
        Err(bota_proto::RejectReason::NotCastable)
    );
    // The other side cannot tell them from the hero by the scoreboard.
    let seen = world.view(Team::Dire);
    assert!(
        seen.players[0].unit.is_none(),
        "which body is the hero is kept"
    );
    run(&mut world, 75 * rules::TICKS_PER_SECOND);
    assert!(
        world.entities.iter().all(|entity| world
            .illusion
            .get(entity)
            .is_none_or(|made| made.of != taker)
            || !world.alive(entity)),
        "they stand for seventy-five seconds"
    );
}

#[test]
fn a_second_illusion_rune_replaces_the_first_pair() {
    let (mut world, taker) = took(&radiant_and_dire(), RuneKind::Illusion);
    world.put_rune(RuneKind::Illusion, RIVER_TOP);
    run(&mut world, 2);
    let count = world
        .entities
        .iter()
        .filter(|&entity| {
            world
                .illusion
                .get(entity)
                .is_some_and(|made| made.of == taker)
        })
        .count();
    assert_eq!(count, 2, "two again, not four");
}

#[test]
fn a_rune_does_not_spawn_when_the_match_does_not_ask_for_its_kind() {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::NONE;
    cfg.runes = RuneSet::of(RuneKind::Water);
    let mut world = World::for_match(&cfg, cfg.rng());
    assert!(spawned(&step_at_minute(&mut world, 0, 0)).is_empty());
    assert_eq!(spawned(&step_at_minute(&mut world, 2, 0)).len(), 2);
    assert!(spawned(&step_at_minute(&mut world, 6, 0)).is_empty());
}

#[test]
fn a_cheat_puts_a_rune_at_the_spot_nearest_the_cursor_as_the_schedule_does() {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.cheats = true;
    cfg.objectives = ObjectiveSet::NONE;
    cfg.runes = RuneSet::of(RuneKind::Haste).with(RuneKind::Bounty);
    let mut world = World::for_match(&cfg, cfg.rng());
    let jungle = spots()[RADIANT_JUNGLE];
    let near = Vec2::from_ints(jungle.x.to_int() + 900, jungle.y.to_int() - 300);
    let cheat = |kind| Order::Cheat {
        cheat: bota_proto::Cheat::SpawnRune { kind, at: near },
    };
    let events = advance_validated(&mut world, None, cheat(RuneKind::Haste));
    assert_eq!(
        spawned(&events),
        vec![(RuneKind::Haste, jungle)],
        "told to both sides at the spot nearest the cursor"
    );
    assert_eq!(lying(&world).len(), 1);
    advance_validated(&mut world, None, cheat(RuneKind::Bounty));
    let worth: Vec<Option<i32>> = lying(&world)
        .iter()
        .filter(|rune| rune.kind == RuneKind::Bounty)
        .map(|rune| rune.gold)
        .collect();
    assert_eq!(worth, vec![Some(40)], "its gold is fixed by the clock");
    // A kind the match leaves out is refused; so is any cheat without cheats on.
    assert_eq!(
        world.validate_order(SlotId(0), None, &cheat(RuneKind::Shield)),
        Err(bota_proto::RejectReason::BadCheat)
    );
    cfg.cheats = false;
    let plain = World::for_match(&cfg, cfg.rng());
    assert_eq!(
        plain.validate_order(SlotId(0), None, &cheat(RuneKind::Haste)),
        Err(bota_proto::RejectReason::NoCheats)
    );
}

#[test]
fn a_cheat_rune_is_refused_on_a_map_without_rune_spots() {
    let mut cfg = config();
    cfg.cheats = true;
    let world = World::for_match(&cfg, cfg.rng());
    let order = Order::Cheat {
        cheat: bota_proto::Cheat::SpawnRune {
            kind: RuneKind::Haste,
            at: Vec2::from_ints(9000, 9000),
        },
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(bota_proto::RejectReason::BadCheat)
    );
}

/// A match played as a server plays it: each seat is told its own names, and
/// its orders name what it was told. Seat 0's hero stands 500 west of the
/// river's top spot with a Haste Rune lying there, one tick after the rune is
/// in its view; answers the match and the name seat 0's view gives the rune.
fn seat_sees_a_rune() -> (crate::local::LocalMatch, bota_proto::EntityId) {
    let cfg = rune_config(&radiant_and_dire());
    let (mut local, _) =
        crate::local::LocalMatch::new(&cfg, &[crate::game::OrderDelay::NONE; 2]).expect("a match");
    let spot = spots()[RIVER_TOP];
    let world = local.world_mut();
    world.put_rune(RuneKind::Haste, RIVER_TOP);
    let taker = hero(world, 0);
    place(
        world,
        taker,
        Vec2::from_ints(spot.x.to_int() - 500, spot.y.to_int()),
    );
    let step = local.step(&[&[], &[]]).expect("a step");
    let view = step[0]
        .iter()
        .find_map(|msg| match msg {
            bota_proto::ServerMsg::Snapshot { view } => Some(view.clone()),
            _ => None,
        })
        .expect("a snapshot");
    assert_eq!(view.runes.len(), 1, "the seat sees the rune");
    (local, view.runes[0].id)
}

/// Seat 0 gives its hero an order and the match plays on until `done`.
fn seat_orders(
    local: &mut crate::local::LocalMatch,
    order: Order,
    done: impl Fn(&World) -> bool,
) -> u32 {
    let given = bota_proto::OutgoingOrder {
        seq: 1,
        unit: None,
        order,
        seen: local.tick(),
        queue: false,
    };
    local.step(&[&[given], &[]]).expect("a step");
    for ticks in 1..300 {
        if done(local.world()) {
            return ticks;
        }
        local.step(&[&[], &[]]).expect("a step");
    }
    panic!("the order was never carried out");
}

#[test]
fn a_seat_that_names_a_rune_as_its_view_does_takes_it() {
    let (mut local, name) = seat_sees_a_rune();
    assert_ne!(
        name,
        wire_id(
            local
                .world()
                .entities
                .iter()
                .find(|&e| local.world().rune.get(e).is_some())
                .expect("a rune")
        ),
        "the seat's name for it is not the simulation's own handle"
    );
    let order = Order::Take {
        target: Target::Unit(name),
    };
    let ticks = seat_orders(&mut local, order, |world| {
        carries(world, hero(world, 0), ModifierKind::RuneHaste)
    });
    assert!(ticks > 20, "the hero walked to it: {ticks} ticks");
}

#[test]
fn a_seat_that_walks_over_a_rune_does_not_take_it() {
    let (mut local, _) = seat_sees_a_rune();
    let spot = spots()[RIVER_TOP];
    let across = Vec2::from_ints(spot.x.to_int() + 500, spot.y.to_int());
    let order = Order::Move {
        target: Target::Pos(across),
    };
    seat_orders(&mut local, order, |world| {
        world
            .pos(hero(world, 0))
            .is_some_and(|at| at.within(across, Fixed::from_int(20)))
    });
    let world = local.world();
    assert_eq!(
        lying(world).len(),
        1,
        "the hero walked over it, and it lies"
    );
    assert!(!carries(world, hero(world, 0), ModifierKind::RuneHaste));
}

#[test]
fn a_seat_that_names_a_rune_as_its_view_does_destroys_it_by_an_attack_order() {
    let (mut local, name) = seat_sees_a_rune();
    let order = Order::Attack {
        target: Target::Unit(name),
    };
    seat_orders(&mut local, order, |world| lying(world).is_empty());
    let world = local.world();
    assert!(!carries(world, hero(world, 0), ModifierKind::RuneHaste));
}

/// The runes lying at one spawner, in the order they appeared (where no entity slot was reused).
fn lying_at_spot(world: &World, spot: usize) -> Vec<(bota_proto::RuneView, Vec2)> {
    let mut runes = lying(world);
    runes.sort_by_key(|rune| (rune.id.idx, rune.id.generation));
    runes
        .into_iter()
        .filter(|rune| {
            world
                .entities
                .iter()
                .find(|&entity| wire_id(entity) == rune.id)
                .and_then(|entity| world.rune.get(entity))
                .is_some_and(|held| usize::from(held.spot) == spot)
        })
        .map(|rune| (rune, rune.pos))
        .collect()
}

/// Whether `at` lies `units` from `origin`, give or take one unit of rounding.
fn about(origin: Vec2, at: Vec2, units: i32) -> bool {
    let d2 = origin.distance_squared(at);
    rules::units(units - 1).squared_raw() <= d2 && d2 <= rules::units(units + 1).squared_raw()
}

/// Whether every two of some positions lie at least the spawner's spacing apart.
fn spaced(positions: &[Vec2]) -> bool {
    let least = rules::units(bota_core::RUNE_SPACING).squared_raw();
    positions.iter().enumerate().all(|(i, a)| {
        positions[i + 1..]
            .iter()
            .all(|b| a.distance_squared(*b) >= least)
    })
}

/// Untaken jungle bounties remain alongside each newer spawn (Valve 7.29), not overlapped: the
/// first at the spawner's origin, every later one on the ring 90 units round it [SRV, see
/// `bota_core::rune_spot`], at least 70 from every other.
#[test]
fn unpicked_bounty_runes_stack_on_the_ring_ninety_units_round_the_first() {
    let mut world = rune_match(&radiant_and_dire());
    for minute in [0, 4, 8, 12] {
        step_at_minute(&mut world, minute, 0);
    }
    for spot in [DIRE_JUNGLE, RADIANT_JUNGLE] {
        let origin = spots()[spot];
        // Told apart by the gold each was worth when it appeared: 40, 46, 52, 58.
        let mut stacked = lying_at_spot(&world, spot);
        stacked.sort_by_key(|(rune, _)| rune.gold);
        let golds: Vec<_> = stacked.iter().map(|(rune, _)| rune.gold).collect();
        assert_eq!(
            golds,
            [Some(40), Some(46), Some(52), Some(58)],
            "opening, four-, eight- and twelve-minute bounties persist, each with its own gold"
        );
        let positions: Vec<Vec2> = stacked.iter().map(|(_, at)| *at).collect();
        assert_eq!(positions[0], origin, "the first lies on the spawner");
        for later in &positions[1..] {
            assert!(
                about(origin, *later, 90),
                "spot {spot}: a later bounty lies 90 units from the spawner: {later:?} of {origin:?}"
            );
        }
        assert!(spaced(&positions), "spot {spot}: {positions:?}");
    }
}

#[test]
fn at_every_spawner_of_the_map_a_second_rune_lies_ninety_units_from_the_first() {
    for spot in 0..spots().len() {
        let mut world = rune_match(&radiant_and_dire());
        world.put_rune(RuneKind::Bounty, spot);
        world.put_rune(RuneKind::Bounty, spot);
        let stacked = lying_at_spot(&world, spot);
        assert_eq!(stacked.len(), 2, "spawner {spot}");
        assert_eq!(stacked[0].1, spots()[spot], "the first on the spawner");
        assert!(
            about(spots()[spot], stacked[1].1, 90),
            "spawner {spot}: {:?} of {:?}",
            stacked[1].1,
            spots()[spot]
        );
    }
}

#[test]
fn a_third_and_a_fourth_stack_keep_the_ring_and_the_spacing() {
    let mut world = rune_match(&radiant_and_dire());
    for _ in 0..4 {
        world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
        world.tick += 1;
    }
    let origin = spots()[RADIANT_JUNGLE];
    let positions: Vec<Vec2> = lying_at_spot(&world, RADIANT_JUNGLE)
        .iter()
        .map(|(_, at)| *at)
        .collect();
    assert_eq!(positions.len(), 4);
    assert!(positions[1..].iter().all(|at| about(origin, *at, 90)));
    assert!(spaced(&positions), "{positions:?}");
}

/// The ring is turned by a phase drawn for each placement from the match's own randomness: the
/// same match gives the same places, and the direction is not always the same one.
#[test]
fn the_ring_is_turned_by_a_draw_that_the_match_repeats_and_that_differs_between_moments() {
    let second = |ticks: u32| {
        let mut world = rune_match(&radiant_and_dire());
        world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
        world.tick += ticks;
        world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
        lying_at_spot(&world, RADIANT_JUNGLE)[1].1
    };
    assert_eq!(second(7), second(7), "same match, same moment, same place");
    let places: std::collections::BTreeSet<_> = (1..=12).map(second).collect();
    assert!(
        places.len() >= 6,
        "the direction varies with the moment: {places:?}"
    );
}

#[test]
fn a_rune_taken_from_the_spawner_frees_it_for_the_next_spawn() {
    let mut world = rune_match(&[Team::Radiant]);
    let origin = spots()[RADIANT_JUNGLE];
    world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
    world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
    let first = lying_at_spot(&world, RADIANT_JUNGLE)[0].0;
    let taker = hero(&world, 0);
    place(&mut world, taker, origin);
    send_to_take(&mut world, taker, &first);
    world.step();
    assert_eq!(lying_at_spot(&world, RADIANT_JUNGLE).len(), 1, "taken");
    world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
    let at: Vec<Vec2> = lying_at_spot(&world, RADIANT_JUNGLE)
        .iter()
        .map(|(_, at)| *at)
        .collect();
    assert_eq!(at.len(), 2);
    assert!(
        at.contains(&origin),
        "the new rune stands on the spawner again: {at:?}"
    );
}

#[test]
fn accumulated_bounties_keep_to_the_spawner_cap_stand_clear_and_can_be_reached() {
    let mut world = rune_match(&radiant_and_dire());
    for spot in 0..spots().len() {
        for _ in 0..=bota_proto::MAX_RUNES_AT_SPOT {
            world.put_rune(RuneKind::Bounty, spot);
            world.tick += 1;
        }
        let positions: Vec<Vec2> = lying_at_spot(&world, spot)
            .iter()
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(
            positions.len(),
            bota_proto::MAX_RUNES_AT_SPOT,
            "spawner {spot}: the cap is the wire's bound"
        );
        assert!(spaced(&positions), "spawner {spot}: {positions:?}");
        for at in positions {
            assert!(world.clearance.stands_clear(at), "spawner {spot}: {at:?}");
            let path = crate::game::find_path(
                &world.clearance,
                spots()[spot] + Vec2::from_ints(400, 0),
                at,
                rules::units(rules::HERO_COLLISION),
            );
            assert!(
                path.last()
                    .is_some_and(|end| end.within(at, rules::units(bota_core::RUNE_PICKUP_RANGE))),
                "a hero can approach each bounty within pickup range: {at:?}"
            );
        }
    }
}

/// Not the reference: where nothing can stand, no rune is put (a rune nobody can come within
/// reach of would be lost).
#[test]
fn a_bounty_is_not_put_on_ground_nothing_can_stand_on() {
    let mut world = rune_match(&radiant_and_dire());
    let origin = spots()[RADIANT_JUNGLE];
    let mut circles = world.clearance.circles().to_vec();
    // Everything within 130 of the spawner, the first ring (90) and not the second (180).
    circles.push((origin, rules::units(130)));
    world.clearance.set_circles(circles);
    world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
    world.put_rune(RuneKind::Bounty, RADIANT_JUNGLE);
    let stacked = lying_at_spot(&world, RADIANT_JUNGLE);
    assert_eq!(stacked.len(), 2);
    assert_eq!(
        stacked[0].1, origin,
        "the spawner's own spot is taken as it is"
    );
    assert!(
        about(origin, stacked[1].1, 180),
        "the second ring: {:?}",
        stacked[1].1
    );
    assert!(world.clearance.stands_clear(stacked[1].1));
}

/// Seat 0 orders its hero to take the rune `rune` and the match plays one tick.
fn ordered_to_take(local: &mut crate::local::LocalMatch, rune: bota_proto::EntityId) {
    let given = bota_proto::OutgoingOrder {
        seq: 1,
        unit: None,
        order: Order::Take {
            target: Target::Unit(rune),
        },
        seen: local.tick(),
        queue: false,
    };
    local.step(&[&[given], &[]]).expect("a step");
}

/// A hero that cannot act takes no rune, in reach or not, like it does nothing else it is
/// ordered; the order waits and is carried out when the stun lifts.
#[test]
fn a_stunned_hero_in_reach_takes_the_rune_only_when_the_stun_lifts() {
    let (mut local, name) = seat_sees_a_rune();
    let world = local.world_mut();
    let taker = hero(world, 0);
    place(world, taker, spots()[RIVER_TOP]);
    put_on(world, taker, ModifierKind::Stunned, 45);
    world.settle();
    ordered_to_take(&mut local, name);
    assert_eq!(
        lying(local.world()).len(),
        1,
        "the stunned hero does not take it"
    );
    for _ in 0..30 {
        local.step(&[&[], &[]]).expect("a step");
    }
    assert_eq!(lying(local.world()).len(), 1, "still stunned");
    for _ in 0..30 {
        local.step(&[&[], &[]]).expect("a step");
    }
    assert!(
        lying(local.world()).is_empty(),
        "the stun lifted and the order was carried out"
    );
}

/// Two spawners that place a rune on one tick do not draw one phase between them.
#[test]
fn spawners_placing_runes_on_one_tick_draw_different_phases() {
    let mut world = rune_match(&radiant_and_dire());
    world.clearance = crate::game::Clearance::open();
    let mut offsets = Vec::new();
    for spot in [RIVER_TOP, RIVER_BOT] {
        world.put_rune(RuneKind::Bounty, spot);
        world.put_rune(RuneKind::Bounty, spot);
        let positions = lying_at_spot(&world, spot);
        assert_eq!(positions.len(), 2);
        offsets.push(positions[1].1 - positions[0].1);
    }
    assert_ne!(
        offsets[0], offsets[1],
        "the spawner is part of the key of the placement's draw"
    );
}
