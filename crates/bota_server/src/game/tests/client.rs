//! The client library's session folds what a server sends and nothing else.

use core::convert::Infallible;
use std::time::{Duration, Instant};

use bota_client_core::{
    AckPolicy, ClientSession, IngestError, Ingested, IssuedOrder, MAX_TRACKED_ENTITIES,
    MatchStatic, Observer, SessionError, SessionOptions,
};
use bota_core::{ModifierKind, NeutralAbility, map_of, rules};
use bota_proto::{
    AbilitySlot, AggroView, Angle, EntityId, EventKind, Fixed, HeroId, ItemId, ItemSlot, LootView,
    MatchInfo, Order, OutgoingOrder, ProjectileView, ServerMsg, SkippedTick, SlotId, StockView,
    Target, Team, UnitKind, Vec2, WorldView,
};

use super::local::mid_config;
use super::support::scroll_def;
use crate::game::{Modifier, OrderDelay, UnitOrder};
use crate::local::LocalMatch;

/// Ticks played: past the first wave of the longest pregame.
const TICKS: u32 = rules::Schedule::of(rules::DOTA_PREGAME_TICKS).first_wave() + 4000;

/// A two-seat map-2 match with both heroes attack-moving at the other's
/// fountain: every message each seat was sent, in order.
fn play() -> Vec<Vec<ServerMsg>> {
    let cfg = mid_config();
    let map = map_of(cfg.map);
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    for _ in 0..TICKS {
        let tick = local.tick();
        let order = |slot: usize| OutgoingOrder {
            seq: tick,
            unit: None,
            order: Order::Attack {
                target: Target::Pos(map.fountains[1 - slot]),
            },
            seen: tick,
            queue: false,
        };
        let step = local.step(&[&[order(0)], &[order(1)]]).expect("a step");
        let over = step[0]
            .iter()
            .any(|message| matches!(message, ServerMsg::MatchOver { .. }));
        for (stream, messages) in seats.iter_mut().zip(step) {
            stream.extend(messages);
        }
        if over {
            break;
        }
    }
    seats
}

fn session_for(info: &MatchInfo, slot: u8) -> ClientSession {
    let options = SessionOptions {
        ack: AckPolicy::Never,
        ..SessionOptions::default()
    };
    session_with(info, slot, options)
}

fn session_with(info: &MatchInfo, slot: u8, options: SessionOptions) -> ClientSession {
    let statics = MatchStatic::new(info).expect("valid terms");
    ClientSession::seat(statics, SlotId(slot), Some(HeroId::SHADOW_FIEND), options).expect("a seat")
}

fn info_of(stream: &[ServerMsg]) -> &MatchInfo {
    match &stream[0] {
        ServerMsg::MatchStart { info } => info,
        other => panic!("a stream starts with MatchStart, not {other:?}"),
    }
}

#[test]
fn two_sessions_fed_one_stream_hold_equal_state() {
    let seats = play();
    let info = info_of(&seats[0]);
    let (mut first, mut second) = (session_for(info, 0), session_for(info, 0));
    for message in &seats[0][1..] {
        let a = first.ingest(message.clone(), &mut ()).expect("folds");
        let b = second.ingest(message.clone(), &mut ()).expect("folds");
        assert_eq!(a, b);
    }
    assert_eq!(first, second);
    let tracked = first.entities().len();
    assert!(tracked > 10, "a match is seen: {tracked} entities");
}

#[test]
fn a_stream_with_skipped_ticks_folds_every_death_and_damage() {
    const EVERY: u32 = 3;
    let seats = play();
    let info = info_of(&seats[0]);
    let mut every = session_for(info, 0);
    let mut slow = session_for(info, 0);
    let mut skipped: Vec<SkippedTick> = Vec::new();
    let mut told_by_skips = 0;
    let mut snapshot = None;
    for message in &seats[0][1..] {
        every.ingest(message.clone(), &mut ()).expect("folds");
        match message {
            ServerMsg::Snapshot { view } => snapshot = Some(view.clone()),
            ServerMsg::Events { tick, events } if tick % EVERY == 0 => {
                let mut view = snapshot.take().expect("events follow their snapshot");
                told_by_skips += skipped
                    .iter()
                    .flat_map(|tick| &tick.events)
                    .filter(|e| matches!(e, EventKind::Died { .. } | EventKind::Damaged { .. }))
                    .count();
                view.skipped_ticks = std::mem::take(&mut skipped);
                slow.ingest(ServerMsg::Snapshot { view }, &mut ())
                    .expect("folds");
                slow.ingest(message.clone(), &mut ()).expect("folds");
                assert_eq!(slow.recent_events(), every.recent_events(), "tick {tick}");
            }
            ServerMsg::Events { tick, events } => skipped.push(SkippedTick {
                tick: *tick,
                events: events.clone(),
            }),
            _ => {}
        }
    }
    assert!(told_by_skips > 0, "the skipped ticks carried a fight");
}

#[test]
fn a_refused_order_is_rolled_back_in_the_ledger() {
    let cfg = mid_config();
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session = session_for(info_of(&seats[0]), 0);
    for message in seats[0].drain(1..) {
        session.ingest(message, &mut ()).expect("folds");
    }
    let nowhere = EntityId {
        idx: 9_000,
        generation: 1,
    };
    let issued = IssuedOrder::hero(Order::Attack {
        target: Target::Unit(nowhere),
    });
    let sent = session.issue(issued).expect("a snapshot is held");
    assert_eq!((sent.seq, sent.seen), (1, local.tick()));
    assert!(session.ledger().repeats(issued));
    let step = local.step(&[&[sent], &[]]).expect("a step");
    let mut refused = false;
    for message in step[0].clone() {
        refused |= matches!(
            session.ingest(message, &mut ()).expect("folds"),
            Ingested::Rejected { seq: 1, .. }
        );
    }
    assert!(refused, "the server refused an order at nobody");
    assert!(!session.ledger().repeats(issued), "nothing stands any more");
}

/// Gives seat 0 one order through its session and plays `ticks` ticks on,
/// the session folding every message.
fn give(local: &mut LocalMatch, session: &mut ClientSession, order: Order, ticks: u32) {
    let sent = session
        .issue(IssuedOrder::hero(order))
        .expect("a snapshot is held");
    let mut first = Some(sent);
    for _ in 0..ticks {
        let step = local.step(&[first.take().as_slice(), &[]]).expect("a step");
        for message in step.into_iter().next().expect("seat 0") {
            let folded = session.ingest(message, &mut ()).expect("folds");
            assert!(
                !matches!(folded, Ingested::Rejected { .. }),
                "{order:?} refused"
            );
        }
    }
}

#[test]
fn the_ledger_keeps_a_standing_walk_exactly_while_the_server_does() {
    let cfg = mid_config();
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session = session_for(info_of(&seats[0]), 0);
    for message in seats[0].drain(1..) {
        session.ingest(message, &mut ()).expect("folds");
    }
    let hero = local.world().seats[0].unit.expect("a hero");
    let idle = Order::Move {
        target: Target::None,
    };
    let ready = local.world().schedule().scroll_first_ready();
    give(&mut local, &mut session, idle, ready);
    super::support::hand_item(local.world_mut(), hero, ItemId::HEALING_SALVE, 1);
    local.world_mut().health.get_mut(hero).expect("health").hp = Fixed::from_int(300);
    let home = local.world().transform.get(hero).expect("standing").pos;
    let goal = home + Vec2::from_ints(1_500, 1_500);
    let walk = Order::Move {
        target: Target::Pos(goal),
    };
    let walks_on = |local: &LocalMatch, session: &ClientSession| {
        let server = local.world().orders.get(hero).expect("orders").current;
        let ledger = session
            .ledger()
            .standing(None)
            .map(|(_, issued)| issued.order);
        (
            server == UnitOrder::Move { pos: goal },
            ledger == Some(walk),
        )
    };
    give(&mut local, &mut session, walk, 5);
    assert_eq!(walks_on(&local, &session), (true, true));
    let drink = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(local.handle_of(0, hero).expect("its own hero")),
    };
    give(&mut local, &mut session, drink, 5);
    assert!(local.world().inventory.get(hero).expect("a bag").slots[0].is_none());
    assert_eq!(walks_on(&local, &session), (true, true), "a salve");
    let near = AbilitySlot(0);
    give(&mut local, &mut session, Order::Learn { slot: near }, 1);
    let raze = Order::Cast {
        slot: near,
        target: Target::None,
    };
    // Long enough for the raze to go off: an order given inside its cast
    // point waits for it.
    give(&mut local, &mut session, raze, 20);
    assert_eq!(walks_on(&local, &session), (true, true), "a raze");
    let world = local.world();
    let tower = world
        .entities
        .iter()
        .find(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Tower)
                && world.team.get(unit) == Some(&Team::Radiant)
        })
        .expect("a tower of its own");
    let read = Order::Use {
        slot: ItemSlot::TOWN_PORTAL,
        target: Target::Unit(local.handle_of(0, tower).expect("its own tower")),
    };
    give(&mut local, &mut session, read, scroll_def().duration + 2);
    let server = local.world().orders.get(hero).expect("orders").current;
    assert_eq!(server, UnitOrder::Idle, "a scroll's landing ends the walk");
    assert_eq!(
        session.ledger().standing(None),
        None,
        "and the ledger agrees"
    );
}

#[test]
fn a_directional_move_replaces_the_walk_standing_in_the_ledger() {
    let (mut local, mut session) = seated();
    let hero = local.world().seats[0].unit.expect("a hero");
    let home = local.world().transform.get(hero).expect("standing").pos;
    let walk = Order::Move {
        target: Target::Pos(home + Vec2::from_ints(1_500, 1_500)),
    };
    let along = Order::MoveDirection {
        toward: home + Vec2::from_ints(0, 700),
    };
    give(&mut local, &mut session, walk, 3);
    give(&mut local, &mut session, along, 3);
    assert!(matches!(
        local.world().orders.get(hero).expect("orders").current,
        UnitOrder::Direction { .. }
    ));
    let standing = session
        .ledger()
        .standing(None)
        .map(|(_, issued)| issued.order);
    assert_eq!(standing, Some(along), "the ledger agrees");
    assert!(
        !session.ledger().repeats(IssuedOrder::hero(walk)),
        "the same walk again is a new order, not a repeat"
    );
}

/// A map-2 match and a session of seat 0 holding everything it was first sent.
pub(super) fn seated() -> (LocalMatch, ClientSession) {
    seated_with(SessionOptions {
        ack: AckPolicy::Never,
        ..SessionOptions::default()
    })
}

fn seated_with(options: SessionOptions) -> (LocalMatch, ClientSession) {
    let cfg = mid_config();
    let (local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session = session_with(info_of(&seats[0]), 0, options);
    for message in seats[0].drain(1..) {
        session.ingest(message, &mut ()).expect("folds");
    }
    (local, session)
}

/// Whether the session's journal holds an event.
fn told(session: &ClientSession, wanted: &EventKind) -> bool {
    session
        .recent_events()
        .iter()
        .any(|event| &event.kind == wanted)
}

/// Seat 0 reads its scroll at its own tower and plays `ticks` ticks on.
fn read_scroll_and_play(local: &mut LocalMatch, session: &mut ClientSession, ticks: u32) {
    local.world_mut().seats[0].item_clocks.clear();
    let world = local.world();
    let tower = world
        .entities
        .iter()
        .find(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Tower)
                && world.team.get(unit) == Some(&Team::Radiant)
        })
        .expect("a tower of its own");
    let read = Order::Use {
        slot: ItemSlot::TOWN_PORTAL,
        target: Target::Unit(local.handle_of(0, tower).expect("its own tower")),
    };
    give(local, session, read, ticks);
}

#[test]
fn a_session_is_told_a_scroll_it_read_landed() {
    let (mut local, mut session) = seated();
    read_scroll_and_play(&mut local, &mut session, scroll_def().duration + 1);
    let landed = EventKind::Teleported {
        slot: SlotId(0),
        landed: true,
    };
    assert!(told(&session, &landed));
}

#[test]
fn a_session_is_told_a_scroll_it_read_was_broken_off() {
    let (mut local, mut session) = seated();
    read_scroll_and_play(&mut local, &mut session, 10);
    let stop = Order::Move {
        target: Target::None,
    };
    give(&mut local, &mut session, stop, 1);
    let broken = EventKind::Teleported {
        slot: SlotId(0),
        landed: false,
    };
    assert!(told(&session, &broken));
}

/// The session's last view, `ticks` later: what a server sends next.
fn later(base: &WorldView, ticks: u32) -> WorldView {
    let mut view = base.clone();
    view.tick += ticks;
    view
}

/// Folds a hostile `view` and the events that complete its tick.
fn fold_view(session: &mut ClientSession, view: WorldView) -> Result<(), SessionError> {
    let tick = view.tick;
    session.ingest(ServerMsg::Snapshot { view }, &mut ())?;
    session.ingest(
        ServerMsg::Events {
            tick,
            events: Vec::new(),
        },
        &mut (),
    )?;
    Ok(())
}

fn entity(idx: u32) -> EntityId {
    EntityId { idx, generation: 1 }
}

/// `view` with a copy of one of its plain units at each handle of `idxs`.
fn crowded(mut view: WorldView, idxs: impl IntoIterator<Item = u32>) -> WorldView {
    let mut template = view
        .units
        .iter()
        .find(|unit| unit.owner.is_none() && unit.cast.is_none())
        .expect("a unit nobody owns")
        .clone();
    template.effects.clear();
    for idx in idxs {
        template.id = entity(idx);
        view.units.push(template.clone());
    }
    view.units.sort_by_key(|unit| unit.id);
    view
}

fn missile(idx: u32) -> ProjectileView {
    ProjectileView {
        id: entity(idx),
        pos: Vec2::from_ints(0, 0),
        facing: Angle::default(),
        team: Team::Radiant,
        ability: None,
        source: None,
        target: None,
        inflictor: None,
        seen_by: None,
        model: None,
    }
}

fn dropped(idx: u32) -> LootView {
    LootView {
        id: entity(idx),
        pos: Vec2::from_ints(0, 0),
        item: ItemId::HEALING_SALVE,
        charges: None,
        seen_by: None,
    }
}

/// Handles the contract promises in ascending order are checked to be, in
/// one pass: a list out of order or repeating is refused, whatever it holds.
#[test]
fn projectiles_and_loot_not_strictly_ascending_by_handle_are_refused() {
    type Fill = fn(&mut WorldView, [u32; 3]);
    let lists: [(&str, Fill); 2] = [
        ("projectiles", |view, ids| {
            view.projectiles = ids.map(missile).to_vec();
        }),
        ("loot", |view, ids| view.loot = ids.map(dropped).to_vec()),
    ];
    for (name, set) in lists {
        for ids in [[1, 2, 3], [1, 3, 2], [3, 2, 1], [1, 2, 2]] {
            let (_, mut session) = seated();
            let mut view = later(session.current().expect("a view"), 1);
            set(&mut view, ids);
            let folded = fold_view(&mut session, view);
            assert_eq!(
                folded.is_ok(),
                ids == [1, 2, 3],
                "{name} {ids:?}: {folded:?}"
            );
        }
    }
}

/// A server that lists felled trees or the shop's stock out of order breaks
/// the contract the same way.
#[test]
fn felled_trees_and_stock_out_of_order_are_refused() {
    let (_, mut session) = seated();
    let mut view = later(session.current().expect("a view"), 1);
    view.felled_trees = vec![3, 1];
    assert!(fold_view(&mut session, view).is_err());
    let (_, mut session) = seated();
    let mut view = later(session.current().expect("a view"), 1);
    let shelf = |team, item| StockView {
        team,
        item,
        left: 1,
        max: 1,
        restock_left: 0,
    };
    view.stock = vec![
        shelf(Team::Dire, ItemId::HEALING_SALVE),
        shelf(Team::Radiant, ItemId::HEALING_SALVE),
    ];
    assert!(fold_view(&mut session, view).is_err());
}

/// Half of the tracked units keep their place and as many new ones appear
/// between them, the most each snapshot may add: folded in a merge, in a time
/// that does not grow with every unit shifting the ones behind it. The same
/// number of new units arriving behind all the tracked ones shifts nothing, so
/// a fold that costs about what that one does is linear.
#[test]
fn a_crowd_appearing_between_tracked_units_folds_in_linear_time() {
    let (_, mut session) = seated();
    let base = session.current().expect("a view").clone();
    let crowd = (MAX_TRACKED_ENTITIES - base.units.len()) as u32 / 2;
    let first = crowded(later(&base, 1), (0..crowd).map(|n| 10_000 + 2 * n));
    fold_view(&mut session, first).expect("a crowd folds");
    let between = crowded(later(&base, 2), (0..2 * crowd).map(|n| 10_000 + n));
    let behind = crowded(later(&base, 2), (0..2 * crowd).map(|n| 100_000 + n));
    let (between_cost, behind_cost) = costs(&session, between.clone(), behind);
    assert_linear(between_cost, behind_cost);
    fold_view(&mut session, between).expect("a crowd folds");
    assert_eq!(
        session.entities().len(),
        base.units.len() + 2 * crowd as usize
    );
}

/// Room for a crowd made by forgetting, one tick's cohort at a time, the
/// thousands of units seen once each: as dear as the same crowd arriving where
/// nothing needs forgetting, give or take the sort of the cohorts.
#[test]
fn a_crowd_evicting_thousands_of_cohorts_folds_in_time_linear_in_them() {
    const SEEN_ONCE: u32 = 2_000;
    let (_, mut session) = seated_with(SessionOptions {
        ack: AckPolicy::Never,
        forget_after_ticks: u32::MAX,
    });
    let base = session.current().expect("a view").clone();
    let roomy = session.clone();
    for n in 0..SEEN_ONCE {
        let view = crowded(later(&base, 1 + n), [10_000 + n]);
        fold_view(&mut session, view).expect("a unit passes");
    }
    let room = (MAX_TRACKED_ENTITIES - base.units.len()) as u32;
    let crowd = crowded(later(&base, 1 + SEEN_ONCE), 20_000..20_000 + room);
    // The same crowd, folded where the tracks have the room already.
    let (evicting_cost, roomy_cost) = {
        let (mut evicting, mut roomy_cost) = (Duration::MAX, Duration::MAX);
        for _ in 0..FOLD_REPEATS {
            evicting = evicting.min(cost(&session, crowd.clone()));
            roomy_cost = roomy_cost.min(cost(&roomy, crowd.clone()));
        }
        (evicting, roomy_cost)
    };
    assert_linear(evicting_cost, roomy_cost);
    fold_view(&mut session, crowd).expect("a crowd folds");
    assert_eq!(session.entities().len(), MAX_TRACKED_ENTITIES);
}

/// How often a fold is timed: the dearest runs are a preempted thread's, so
/// the cheapest stands for what the fold costs.
const FOLD_REPEATS: usize = 15;

/// How many times a fold may cost what its linear yardstick does. The
/// quadratic folds this guards against cost over ten times as much (shifting
/// the tracks behind each new one, rescanning them for each cohort), a linear one
/// about as much. Both sides of a ratio are timed in turns on the same thread,
/// so the machine's load is on both.
const LINEAR_RATIO: u32 = 5;

/// What folding `view` into a copy of `session` costs.
fn cost(session: &ClientSession, view: WorldView) -> Duration {
    let mut copy = session.clone();
    let started = Instant::now();
    fold_view(&mut copy, view).expect("a crowd folds");
    started.elapsed()
}

/// The cheapest of several folds of each of two views into copies of `session`,
/// taken in turns.
fn costs(session: &ClientSession, one: WorldView, other: WorldView) -> (Duration, Duration) {
    let (mut one_cost, mut other_cost) = (Duration::MAX, Duration::MAX);
    for _ in 0..FOLD_REPEATS {
        one_cost = one_cost.min(cost(session, one.clone()));
        other_cost = other_cost.min(cost(session, other.clone()));
    }
    (one_cost, other_cost)
}

#[track_caller]
fn assert_linear(fold: Duration, yardstick: Duration) {
    assert!(
        fold < yardstick * LINEAR_RATIO,
        "a fold of {fold:?} against {yardstick:?} for the same crowd with no shifting or forgetting"
    );
}

/// What a socket does to a message: framed, taken out of the stream and
/// decoded, as a server sends it and a client reads it.
fn over_the_wire(message: &ServerMsg) -> ServerMsg {
    let mut reader = bota_proto::FrameReader::new();
    reader.push(&bota_proto::encode_frame_to_vec(message).expect("a frame"));
    reader
        .next_message()
        .expect("a frame that decodes")
        .expect("a whole frame")
}

/// Lays the stacking Dragonhide Aura of `sources` other units on seat 0's hero
/// and plays a tick on, folding it: the hero's effects as the client then
/// holds them, and as many as the server holds on it.
fn crowded_hero(sources: usize) -> (Result<(), SessionError>, usize, usize) {
    let (mut local, mut session) = seated();
    let world = local.world_mut();
    let hero = world.seats[0].unit.expect("a hero");
    let carriers: Vec<_> = (0..sources).map(|_| world.spawn()).collect();
    let aura = ModifierKind::Spell {
        ability: NeutralAbility::DragonhideAura,
        level: 1,
        amount: 0,
    };
    assert!(NeutralAbility::DragonhideAura.per_source());
    for carrier in carriers {
        let modifier = Modifier::new(aura, Some(carrier), None, Some(600));
        world
            .modifiers
            .get_mut(hero)
            .expect("a hero wears")
            .put(modifier);
    }
    let step = local.step(&[&[], &[]]).expect("a step");
    let folded = step[0].iter().try_for_each(|message| {
        session
            .ingest(over_the_wire(message), &mut ())
            .map(drop)
            .map_err(SessionError::from)
    });
    let held = local.world().modifiers.get(hero).expect("worn").0.len();
    let told = session.own_hero().map_or(0, |hero| hero.effects.len());
    (folded, held, told)
}

/// More than the 32 effects a client once refused, which a fight with auras
/// puts on a hero: a legal view the client folds.
#[test]
fn a_hero_wearing_forty_effects_is_a_view_the_client_folds() {
    let (folded, held, told) = crowded_hero(40);
    assert_eq!(folded, Ok(()));
    assert!(held >= 40 && told >= 40, "{held} held, {told} told");
}

/// A server that lists one effect more than the contract allows on a unit
/// breaks it: the session refuses the view, whichever bound it holds.
#[test]
fn a_view_listing_more_effects_than_the_bound_is_refused() {
    let (_, mut session) = seated();
    let mut view = later(session.current().expect("a view"), 1);
    let effect = bota_proto::EffectView {
        id: bota_proto::EffectId(0),
        ticks_left: None,
        stacks: None,
        caster: None,
        inflictor: None,
        ticks_total: None,
    };
    let hero = view
        .units
        .iter_mut()
        .find(|unit| unit.kind == UnitKind::Hero)
        .expect("a hero");
    hero.effects = vec![effect; rules::MAX_EFFECTS_PER_UNIT];
    assert_eq!(fold_view(&mut session, view.clone()), Ok(()));
    let mut view = later(&view, 1);
    view.units
        .iter_mut()
        .find(|unit| unit.kind == UnitKind::Hero)
        .expect("a hero")
        .effects
        .push(effect);
    assert!(matches!(
        fold_view(&mut session, view),
        Err(SessionError::LimitExceeded { .. })
    ));
}

/// A unit never wears more than the bound the contract states, however many
/// units lay an aura on it, and the client takes what is told.
#[test]
fn a_unit_wears_no_more_effects_than_the_contract_bounds_and_the_client_folds_them() {
    let (folded, held, told) = crowded_hero(rules::MAX_EFFECTS_PER_UNIT + 40);
    assert_eq!(folded, Ok(()));
    assert_eq!(held, rules::MAX_EFFECTS_PER_UNIT);
    assert_eq!(told, rules::MAX_EFFECTS_PER_UNIT);
}

/// An observer whose `prepare` reshapes the snapshot the way a client may.
struct Reshape(fn(&mut WorldView));

impl Observer for Reshape {
    type Error = Infallible;

    fn prepare(&mut self, view: &mut WorldView) -> Result<(), Infallible> {
        (self.0)(view);
        Ok(())
    }
}

/// A view in which two units never seen before fight.
fn fighting(base: &WorldView) -> WorldView {
    let mut view = crowded(later(base, 1), [10_000, 10_001]);
    view.aggro = vec![AggroView {
        unit: entity(10_000),
        target: Some(entity(10_001)),
        swing: None,
        hold_left: 0,
        chase_left: 0,
        draw_cooldown: 0,
        returning: false,
        held_for: Some(0),
        chased_for: None,
        draw_wait: None,
    }];
    view
}

/// Whatever an observer makes of a validated snapshot, the session answers
/// with an error, never a panic: what the fold relies on is checked after
/// the observer's edit.
#[test]
fn an_observer_cannot_make_a_validated_snapshot_panic_the_fold() {
    type Edit = fn(&mut WorldView);
    let edits: [(&str, Edit); 4] = [
        ("drops the unit an aggro entry names", |view| {
            view.units.retain(|unit| unit.id != entity(10_001));
        }),
        ("adds units past the bound", |view| {
            let copy = view.units.clone();
            view.units
                .extend(copy.into_iter().cycle().take(MAX_TRACKED_ENTITIES));
        }),
        ("reverses the units", |view| view.units.reverse()),
        ("moves the tick back", |view| view.tick = 0),
    ];
    for (name, edit) in edits {
        let (_, mut session) = seated();
        let base = session.current().expect("a view").clone();
        let view = fighting(&base);
        session
            .ingest(ServerMsg::Snapshot { view: view.clone() }, &mut ())
            .expect("the unedited view folds");
        let (_, mut session) = seated();
        let outcome = session.ingest(ServerMsg::Snapshot { view }, &mut Reshape(edit));
        assert!(
            matches!(outcome, Err(IngestError::Session(_))),
            "an observer that {name}: {outcome:?}"
        );
    }
}
