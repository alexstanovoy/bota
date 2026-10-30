//! What a snapshot or an event batch must satisfy before a session folds it.

use bota_core::rules;
use bota_proto::{
    EntityId, EventKind, HeroId, LobbySlot, Pick, PlayerView, ProjectileModel, StatusFlags,
    StockView, Team, UnitKind, WorldView,
};

use crate::limits::{
    COURIER_ITEM_SLOTS, HERO_ITEM_SLOTS, MAX_ABILITY_SLOTS, MAX_EFFECTS_PER_UNIT,
    MAX_EVENTS_PER_BATCH, MAX_PLANTED_TREES, MAX_PROJECTILES, MAX_RESTORATION_REPORT, MAX_SEATS,
    MAX_SHOP_ITEMS, MAX_SKIPPED_TICKS, MAX_SNAPSHOT_LOOT, MAX_STATIC_TREES, MAX_TRACKED_ENTITIES,
    OWN_ITEM_SLOTS, PROJECTILE_MAP_MARGIN, STASH_ITEM_SLOTS,
};
use crate::match_static::{MatchStatic, RawExtent, check_limit, validate_positions};
use crate::session::SessionError;
use crate::session::tracks::{EntityTrack, Owner, pair_tracked};

/// What a snapshot is checked against.
pub(crate) struct Expect<'a> {
    pub statics: &'a MatchStatic,
    /// The seat the stream is for; absent for a spectator.
    pub owner: Option<Owner>,
    /// The hero the seat must play, when one is required.
    pub hero: Option<HeroId>,
    pub current_tick: Option<u32>,
    pub last_event_tick: Option<u32>,
}

/// What the fold of a snapshot relies on, checked again after an observer
/// reshaped the snapshot: its bounds, its skipped ticks, its units sorted
/// and within their bounds, and the units its aggro names. The fold reads
/// nothing else of the view that it does not check itself, so whatever the
/// observer did, it holds by construction.
pub(crate) fn reshaped(expect: &Expect<'_>, view: &WorldView) -> Result<(), SessionError> {
    snapshot_limits(view)?;
    skipped_ticks(expect, view)?;
    units(
        expect.statics.roster(),
        expect.statics.metadata().cheats,
        view,
    )?;
    aggro(view)
}

pub(crate) fn snapshot(expect: &Expect<'_>, view: &WorldView) -> Result<(), SessionError> {
    if let Some(owner) = expect.owner
        && view.viewer != Some(owner.team)
    {
        return Err(SessionError::WrongViewer(view.viewer, owner.team));
    }
    if let Some(current) = expect.current_tick
        && view.tick <= current
    {
        return Err(SessionError::SnapshotTick {
            incoming: view.tick,
            current,
        });
    }
    snapshot_limits(view)?;
    seen_by(view)?;
    withheld(view)?;
    skipped_ticks(expect, view)?;
    snapshot_positions(expect.statics.metadata().terrain_cells, view)?;
    units(
        expect.statics.roster(),
        expect.statics.metadata().cheats,
        view,
    )?;
    aggro(view)?;
    casts(view)?;
    effects(view)?;
    projectiles(view)?;
    dynamic_handles(view)?;
    players(
        expect.statics.roster(),
        expect.statics.metadata().cheats,
        view,
    )?;
    felled_trees(expect.statics.trees().len(), view)?;
    match expect.owner {
        Some(owner) => own_state(owner, expect.hero, view),
        None => Ok(()),
    }
}

/// Which sides see what is told in a view of the whole map alone: a side's
/// view that says it breaks the contract (it would tell what the other side
/// sees).
fn seen_by(view: &WorldView) -> Result<(), SessionError> {
    let told = view.units.iter().any(|unit| unit.seen_by.is_some())
        || view.projectiles.iter().any(|shot| shot.seen_by.is_some())
        || view.loot.iter().any(|item| item.seen_by.is_some());
    match view.viewer {
        Some(_) if told => Err(SessionError::Stream(
            "a side's view says which sides see what",
        )),
        _ => Ok(()),
    }
}

/// What a side's view withholds from it is absent there, not a default: the
/// other side's creep score and gold, its Glyph's cooldown, its seats'
/// open talents, its units' unspent points and sale marks, its heroes' draw
/// waits, and a hold on a unit the view does not carry. A view that tells
/// any of them breaks the contract.
fn withheld(view: &WorldView) -> Result<(), SessionError> {
    for entry in &view.aggro {
        if entry.target.is_none() && entry.held_for.is_some() {
            return Err(SessionError::Stream("a hold on nobody in view is told"));
        }
    }
    let Some(team) = view.viewer else {
        return Ok(());
    };
    let told_of_theirs = view
        .players
        .iter()
        .filter(|player| player.team != team)
        .any(|player| player.creep_score.is_some() || player.gold.is_some())
        || view
            .glyphs
            .iter()
            .any(|glyph| glyph.team != team && glyph.ready_after.is_some())
        || view.talents.iter().any(|talents| {
            (talents.may_learn.is_some() || talents.may_learn_attribute_bonus.is_some())
                && view
                    .player(talents.slot)
                    .is_some_and(|player| player.team != team)
        })
        || view
            .units
            .iter()
            .filter(|unit| unit.team != team)
            .any(|unit| {
                unit.abilities
                    .iter()
                    .any(|ability| ability.learnable.is_some())
                    || unit
                        .items
                        .iter()
                        .flatten()
                        .chain(&unit.town_portal)
                        .any(|item| item.selling.is_some())
            })
        || view.aggro.iter().any(|entry| {
            entry.draw_wait.is_some()
                && view
                    .unit(entry.unit)
                    .is_some_and(|unit| unit.team != team && unit.kind == UnitKind::Hero)
        });
    match told_of_theirs {
        true => Err(SessionError::Stream(
            "a side's view tells what the other side keeps to itself",
        )),
        false => Ok(()),
    }
}

/// A lobby as a server tells it: a row per seat at most, each name within
/// what a server shows.
pub(crate) fn lobby(slots: &[LobbySlot]) -> Result<(), SessionError> {
    check_limit("LobbyState.slots", slots.len(), MAX_SEATS)?;
    for slot in slots {
        check_limit(
            "LobbySlot.name",
            slot.name.len(),
            rules::MAX_SHOWN_NAME_BYTES,
        )?;
    }
    Ok(())
}

/// An event batch: bounded, its restoration reports in range and its tick
/// above the last batch's.
pub(crate) fn events(
    tick: u32,
    events: &[EventKind],
    last_event_tick: Option<u32>,
) -> Result<(), SessionError> {
    check_limit("event batch", events.len(), MAX_EVENTS_PER_BATCH)?;
    restoration_reports(events)?;
    if let Some(previous) = last_event_tick
        && tick <= previous
    {
        return Err(SessionError::EventTickNotIncreasing {
            incoming: tick,
            previous,
        });
    }
    Ok(())
}

/// Skipped ticks strictly increasing, after the last snapshot and the last events and before
/// the snapshot; their events bounded. A gap need not be told.
fn skipped_ticks(expect: &Expect<'_>, view: &WorldView) -> Result<(), SessionError> {
    let skipped = &view.skipped_ticks;
    let after = expect.current_tick.max(expect.last_event_tick);
    if !skipped.windows(2).all(|pair| pair[0].tick < pair[1].tick)
        || skipped.last().is_some_and(|last| last.tick >= view.tick)
        || skipped
            .first()
            .is_some_and(|first| after.is_some_and(|after| first.tick <= after))
    {
        return Err(SessionError::SkippedTicks {
            incoming: view.tick,
        });
    }
    for skipped in skipped {
        check_limit(
            "skipped tick events",
            skipped.events.len(),
            MAX_EVENTS_PER_BATCH,
        )?;
        restoration_reports(&skipped.events)?;
    }
    Ok(())
}

/// Projectiles and loot sorted strictly by handle, as the contract says: one
/// pass, which also rules out a handle told twice. Felled trees and the
/// shop's stock keep their promised order the same way.
fn dynamic_handles(view: &WorldView) -> Result<(), SessionError> {
    ascending(
        &view.projectiles,
        |shot| shot.id,
        SessionError::DuplicateProjectile,
        SessionError::UnsortedProjectiles,
    )?;
    ascending(
        &view.loot,
        |item| item.id,
        SessionError::DuplicateLoot,
        SessionError::UnsortedLoot,
    )?;
    if !view.felled_trees.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(SessionError::UnsortedFelledTrees);
    }
    let stock_key = |stock: &StockView| (stock.team, stock.item);
    if !view
        .stock
        .windows(2)
        .all(|pair| stock_key(&pair[0]) < stock_key(&pair[1]))
    {
        return Err(SessionError::UnsortedStock);
    }
    Ok(())
}

/// Handles ascending strictly: `repeated` names the first that repeats its
/// predecessor, `unsorted` stands for a list that falls.
fn ascending<T>(
    items: &[T],
    id: impl Fn(&T) -> EntityId,
    repeated: fn(EntityId) -> SessionError,
    unsorted: SessionError,
) -> Result<(), SessionError> {
    for pair in items.windows(2) {
        let (before, after) = (id(&pair[0]), id(&pair[1]));
        if before == after {
            return Err(repeated(after));
        }
        if before > after {
            return Err(unsorted);
        }
    }
    Ok(())
}

fn snapshot_limits(view: &WorldView) -> Result<(), SessionError> {
    check_limit("WorldView.units", view.units.len(), MAX_TRACKED_ENTITIES)?;
    check_limit(
        "WorldView.projectiles",
        view.projectiles.len(),
        MAX_PROJECTILES,
    )?;
    check_limit("WorldView.players", view.players.len(), MAX_SEATS)?;
    check_limit("WorldView.aggro", view.aggro.len(), view.units.len())?;
    check_limit("WorldView.order_delays", view.order_delays.len(), MAX_SEATS)?;
    check_limit("WorldView.talents", view.talents.len(), MAX_SEATS)?;
    check_limit("WorldView.teleports", view.teleports.len(), MAX_SEATS)?;
    check_limit(
        "WorldView.skipped_ticks",
        view.skipped_ticks.len(),
        MAX_SKIPPED_TICKS,
    )?;
    check_limit("WorldView.loot", view.loot.len(), MAX_SNAPSHOT_LOOT)?;
    // Two sides' shelves for every item of the shop at most.
    check_limit("WorldView.stock", view.stock.len(), 2 * MAX_SHOP_ITEMS)?;
    check_limit(
        "WorldView.felled_trees",
        view.felled_trees.len(),
        MAX_STATIC_TREES,
    )?;
    check_limit(
        "WorldView.planted_trees",
        view.planted_trees.len(),
        MAX_PLANTED_TREES,
    )?;
    Ok(())
}

/// Units sorted strictly by id, bounded, and owned by nobody or a seat of
/// the roster: what a client remembers is kept per seat.
fn units(roster: &[Pick], cheats: bool, view: &WorldView) -> Result<(), SessionError> {
    for (index, unit) in view.units.iter().enumerate() {
        if index > 0 {
            if view.units[index - 1].id == unit.id {
                return Err(SessionError::DuplicateUnit(unit.id));
            }
            if view.units[index - 1].id > unit.id {
                return Err(SessionError::UnsortedUnits);
            }
        }
        check_limit(
            "UnitView.abilities",
            unit.abilities.len(),
            MAX_ABILITY_SLOTS,
        )?;
        check_limit("UnitView.items", unit.items.len(), HERO_ITEM_SLOTS)?;
        check_limit("UnitView.effects", unit.effects.len(), MAX_EFFECTS_PER_UNIT)?;
        check_limit("UnitView.queue", unit.queue.len(), rules::MAX_QUEUED_ORDERS)?;
        // A hero a cheat created has a seat of its own, told among the players.
        let created = |slot| cheats && view.player(slot).is_some();
        if let Some(slot) = unit.owner
            && roster
                .binary_search_by_key(&slot, |pick| pick.slot)
                .is_err()
            && !created(slot)
        {
            return Err(SessionError::UnknownUnitOwner {
                unit: unit.id,
                slot: slot.0,
            });
        }
    }
    Ok(())
}

/// Casts shown exactly while their units cast, going off within the cast point of their ability,
/// or of the slowest item for the use of an item.
fn casts(view: &WorldView) -> Result<(), SessionError> {
    for unit in &view.units {
        let casting = unit.statuses.bits & StatusFlags::CASTING != 0;
        if unit.cast.is_some() != casting {
            return Err(SessionError::CastOutOfRange(unit.id));
        }
        let Some(cast) = unit.cast else {
            continue;
        };
        let point_ms = match cast.ability {
            Some(ability) => bota_core::ability_def(ability).map_or(0, |def| def.point),
            None => bota_core::ITEMS
                .iter()
                .map(|def| def.point)
                .max()
                .unwrap_or(0),
        };
        // Widened as for a swing: the cast point is the rules' word in milliseconds.
        let point = (u64::from(point_ms) * u64::from(rules::TICKS_PER_SECOND)).div_ceil(1000);
        if !(1..=point.max(1)).contains(&u64::from(cast.goes_off_in)) {
            return Err(SessionError::CastOutOfRange(unit.id));
        }
    }
    Ok(())
}

/// Effects naming visible units only as their casters, each running down from its full length.
fn effects(view: &WorldView) -> Result<(), SessionError> {
    for unit in &view.units {
        for effect in &unit.effects {
            if let Some(caster) = effect.caster {
                view.unit(caster)
                    .ok_or(SessionError::UnknownEffectCaster(caster))?;
            }
            let runs_down = match (effect.ticks_left, effect.ticks_total) {
                (None, None) => true,
                (Some(left), Some(total)) => left <= total,
                _ => false,
            };
            if !runs_down {
                return Err(SessionError::EffectOutOfRange(unit.id));
            }
        }
    }
    Ok(())
}

/// Projectiles naming visible units only, as their sources and their targets,
/// each looking as no more than it shows: an attack's missile as its
/// thrower's kind of unit throws, of its side, and a spell's as its
/// inflictor alone.
fn projectiles(view: &WorldView) -> Result<(), SessionError> {
    for shot in &view.projectiles {
        for id in [shot.source, shot.target].into_iter().flatten() {
            view.unit(id)
                .ok_or(SessionError::UnknownProjectileUnit(id))?;
        }
        let Some(model) = shot.model else {
            continue;
        };
        let of_neutrals = matches!(model, ProjectileModel::Neutral(_));
        let source = shot.source.and_then(|id| view.unit(id));
        if shot.inflictor.is_some()
            || of_neutrals != (shot.team == Team::Neutral)
            || source.is_some_and(|unit| thrown_by(unit) != Some(model))
        {
            return Err(SessionError::Stream(
                "a missile looks as something other than what let it go",
            ));
        }
    }
    Ok(())
}

/// How a unit's attack missile looks: Dota's `ProjectileModel` of its kind,
/// an illusion's as its hero's; none for a unit that throws none.
fn thrown_by(unit: &bota_proto::UnitView) -> Option<ProjectileModel> {
    Some(match unit.kind {
        UnitKind::CreepRanged => ProjectileModel::RangedCreep,
        UnitKind::CreepSiege => ProjectileModel::SiegeCreep,
        UnitKind::Tower => ProjectileModel::Tower,
        UnitKind::Fountain => ProjectileModel::Fountain,
        UnitKind::Hero => ProjectileModel::Hero(unit.hero?),
        UnitKind::CreepNeutral => ProjectileModel::Neutral(unit.neutral?),
        _ => return None,
    })
}

/// Aggro entries sorted strictly by unit, naming visible units only, their timers within the
/// rules' bounds and a swing striking within its attacker's attack point.
fn aggro(view: &WorldView) -> Result<(), SessionError> {
    let unit_of = |id: EntityId| view.unit(id).ok_or(SessionError::UnknownAggroUnit(id));
    for (index, entry) in view.aggro.iter().enumerate() {
        if index > 0 && view.aggro[index - 1].unit >= entry.unit {
            return Err(SessionError::UnsortedAggro);
        }
        let unit = unit_of(entry.unit)?;
        if let Some(target) = entry.target {
            unit_of(target)?;
        }
        // Widened: the attack point is the view's word, and `ms_to_ticks_ceil` is u32.
        let point = u64::from(unit.attack_point) * u64::from(rules::TICKS_PER_SECOND);
        let strike_bound = point.div_ceil(1000) + 1;
        let strikes_in_bounds = entry
            .swing
            .is_none_or(|swing| (1..=strike_bound).contains(&u64::from(swing.strike_in)));
        if let Some(swing) = entry.swing {
            unit_of(swing.target)?;
        }
        if !strikes_in_bounds
            || entry.held_for > Some(rules::ORDER_AGGRO_HOLD_TICKS)
            || entry.draw_wait > Some(rules::ORDER_AGGRO_COOLDOWN_TICKS)
            || entry.chased_for.is_some()
        {
            return Err(SessionError::AggroOutOfRange(entry.unit));
        }
    }
    Ok(())
}

fn players(roster: &[Pick], cheats: bool, view: &WorldView) -> Result<(), SessionError> {
    for (index, player) in view.players.iter().enumerate() {
        if index > 0 {
            if view.players[index - 1].slot == player.slot {
                return Err(SessionError::DuplicatePlayer(player.slot.0));
            }
            if view.players[index - 1].slot > player.slot {
                return Err(SessionError::UnsortedPlayers);
            }
        }
        if let Some(stash) = &player.stash {
            check_limit("PlayerView.stash", stash.len(), STASH_ITEM_SLOTS)?;
        }
        if let Some(kit) = &player.kit {
            check_limit("Kit.abilities", kit.abilities.len(), MAX_ABILITY_SLOTS)?;
            check_limit("Kit.items", kit.items.len(), HERO_ITEM_SLOTS)?;
        }
    }
    let known = |slot| view.player(slot).is_some();
    let sorted = view
        .talents
        .windows(2)
        .all(|pair| pair[0].slot < pair[1].slot);
    if !sorted || !view.talents.iter().all(|told| known(told.slot)) {
        return Err(SessionError::UnsortedTalents);
    }
    // A seat channels one scroll at a time.
    let one_each = view.teleports.iter().enumerate().all(|(at, told)| {
        known(told.slot) && view.teleports[..at].iter().all(|o| o.slot != told.slot)
    });
    if !one_each {
        return Err(SessionError::InvalidTeleports);
    }
    roster_rows(roster, cheats, &view.players)
}

/// Every pick of the roster has its row, as picked; a row of a seat the
/// roster lacks only in a match with cheats on, where `-createhero` gives a
/// created hero a seat of its own.
fn roster_rows(roster: &[Pick], cheats: bool, players: &[PlayerView]) -> Result<(), SessionError> {
    for player in players {
        let index = match roster.binary_search_by_key(&player.slot, |pick| pick.slot) {
            Ok(index) => index,
            Err(_) if cheats => continue,
            Err(_) => return Err(SessionError::UnknownRosterPlayer(player.slot.0)),
        };
        let pick = &roster[index];
        if player.team != pick.team {
            return Err(SessionError::WrongRosterTeam {
                slot: player.slot.0,
                actual: player.team,
                expected: pick.team,
            });
        }
        if player.hero != pick.hero {
            return Err(SessionError::WrongRosterHero {
                slot: player.slot.0,
                actual: player.hero,
                expected: pick.hero,
            });
        }
    }
    for pick in roster {
        if players
            .binary_search_by_key(&pick.slot, |player| player.slot)
            .is_err()
        {
            return Err(SessionError::MissingRosterPlayer(pick.slot.0));
        }
    }
    Ok(())
}

fn felled_trees(static_tree_count: usize, view: &WorldView) -> Result<(), SessionError> {
    for index in &view.felled_trees {
        if usize::try_from(*index).map_or(true, |index| index >= static_tree_count) {
            return Err(SessionError::InvalidFelledTree(*index));
        }
    }
    Ok(())
}

fn own_state(owner: Owner, hero: Option<HeroId>, view: &WorldView) -> Result<(), SessionError> {
    let player = view
        .player(owner.slot)
        .ok_or(SessionError::MissingOwnPlayer(owner.slot.0))?;
    if player.gold.is_none() {
        return Err(SessionError::MissingOwnGold);
    }
    let stash_items = player
        .stash
        .as_ref()
        .ok_or(SessionError::MissingOwnStash)?
        .len();
    let body_items = if let Some(id) = player.unit {
        let unit = view.unit(id).ok_or(SessionError::MissingOwnHero(id))?;
        if unit.kind != UnitKind::Hero
            || unit.team != owner.team
            || unit.owner != Some(owner.slot)
            || hero.is_some_and(|hero| unit.hero != Some(hero))
        {
            return Err(SessionError::InvalidOwnHero(id));
        }
        unit.items.len()
    } else {
        player.kit.as_ref().map_or(0, |kit| kit.items.len())
    };
    own_hero_claims(owner, view, player.unit)?;
    own_courier(owner, view)?;
    own_item_slots(owner, view, body_items, stash_items)
}

fn own_hero_claims(
    owner: Owner,
    view: &WorldView,
    scoreboard: Option<EntityId>,
) -> Result<(), SessionError> {
    for unit in &view.units {
        if unit.kind == UnitKind::Hero
            && unit.owner == Some(owner.slot)
            && Some(unit.id) != scoreboard
        {
            return Err(SessionError::AmbiguousOwnHero {
                scoreboard,
                visible: unit.id,
            });
        }
    }
    Ok(())
}

fn own_courier(owner: Owner, view: &WorldView) -> Result<(), SessionError> {
    let mut courier = None;
    for unit in &view.units {
        if !owner.owns_courier(unit) {
            continue;
        }
        if let Some(first) = courier {
            return Err(SessionError::AmbiguousOwnCourier {
                first,
                second: unit.id,
            });
        }
        courier = Some(unit.id);
    }
    Ok(())
}

fn own_item_slots(
    owner: Owner,
    view: &WorldView,
    body_items: usize,
    stash_items: usize,
) -> Result<(), SessionError> {
    let mut courier_items = 0usize;
    for unit in &view.units {
        if owner.owns_courier(unit) {
            check_limit("own courier items", unit.items.len(), COURIER_ITEM_SLOTS)?;
            courier_items = courier_items.saturating_add(unit.items.len());
        }
    }
    check_limit(
        "own hero, stash, and courier item slots",
        body_items
            .saturating_add(stash_items)
            .saturating_add(courier_items),
        OWN_ITEM_SLOTS,
    )
}

/// Every position difference to a tracked unit must fit the wire vector type.
pub(crate) fn position_deltas(
    entities: &[EntityTrack],
    view: &WorldView,
) -> Result<(), SessionError> {
    for (_, unit, tracked) in pair_tracked(entities, &view.units) {
        if let Some(entity) = tracked {
            let previous = entity.unit.pos;
            if unit.pos.x.checked_sub(previous.x).is_none()
                || unit.pos.y.checked_sub(previous.y).is_none()
            {
                return Err(SessionError::PositionDeltaOverflow(unit.id));
            }
        }
    }
    Ok(())
}

fn snapshot_positions(cells: u32, view: &WorldView) -> Result<(), SessionError> {
    let map = RawExtent::map(cells);
    let flight = map.widened(PROJECTILE_MAP_MARGIN);
    validate_positions("WorldView.units", view.units.iter().map(|u| u.pos), map)?;
    validate_positions(
        "WorldView.projectiles",
        view.projectiles.iter().map(|p| p.pos),
        flight,
    )?;
    validate_positions(
        "WorldView.planted_trees",
        view.planted_trees.iter().copied(),
        map,
    )?;
    validate_positions("WorldView.loot", view.loot.iter().map(|l| l.pos), map)?;
    validate_positions(
        "WorldView.teleports",
        view.teleports.iter().map(|t| t.destination),
        map,
    )
}

fn restoration_reports(events: &[EventKind]) -> Result<(), SessionError> {
    assert!(events.len() <= MAX_EVENTS_PER_BATCH);
    for event in events {
        if let EventKind::Healed { amount, mana, .. } = *event {
            for (channel, amount) in [("health", amount), ("mana", mana)] {
                if !(0..=MAX_RESTORATION_REPORT).contains(&amount) {
                    return Err(SessionError::RestorationReport { channel, amount });
                }
            }
        }
    }
    Ok(())
}
