//! What a snapshot or an event batch must satisfy before a session folds it.

use bota_core::rules;
use bota_proto::{
    EntityId, EventKind, HeroId, LobbySlot, Pick, PlayerView, StatusFlags, UnitKind, WorldView,
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
    skipped_ticks(expect, view)?;
    snapshot_positions(expect.statics.metadata().terrain_cells, view)?;
    units(expect.statics.roster(), view)?;
    aggro(view)?;
    casts(view)?;
    projectiles(view)?;
    unique_dynamic_handles(view)?;
    players(expect.statics.roster(), view)?;
    felled_trees(expect.statics.trees().len(), view)?;
    match expect.owner {
        Some(owner) => own_state(owner, expect.hero, view),
        None => Ok(()),
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

fn unique_dynamic_handles(view: &WorldView) -> Result<(), SessionError> {
    if let Some(id) = first_repeat(&view.projectiles, |p| p.id) {
        return Err(SessionError::DuplicateProjectile(id));
    }
    first_repeat(&view.loot, |l| l.id).map_or(Ok(()), |id| Err(SessionError::DuplicateLoot(id)))
}

/// The first id that repeats one listed before it.
fn first_repeat<T>(items: &[T], id: impl Fn(&T) -> EntityId) -> Option<EntityId> {
    let mut seen = items.iter().enumerate();
    seen.find_map(|(at, item)| {
        items[..at]
            .iter()
            .any(|o| id(o) == id(item))
            .then(|| id(item))
    })
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
fn units(roster: &[Pick], view: &WorldView) -> Result<(), SessionError> {
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
        if let Some(slot) = unit.owner
            && roster
                .binary_search_by_key(&slot, |pick| pick.slot)
                .is_err()
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

/// Projectiles naming visible units only, as their sources and their targets.
fn projectiles(view: &WorldView) -> Result<(), SessionError> {
    for shot in &view.projectiles {
        for id in [shot.source, shot.target].into_iter().flatten() {
            view.unit(id)
                .ok_or(SessionError::UnknownProjectileUnit(id))?;
        }
    }
    Ok(())
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
            || entry.hold_left > rules::ORDER_AGGRO_HOLD_TICKS
            || entry.draw_cooldown > rules::ORDER_AGGRO_COOLDOWN_TICKS
            || entry.chase_left > rules::CREEP_CHASE_TICKS
        {
            return Err(SessionError::AggroOutOfRange(entry.unit));
        }
    }
    Ok(())
}

fn players(roster: &[Pick], view: &WorldView) -> Result<(), SessionError> {
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
    roster_rows(roster, &view.players)
}

fn roster_rows(roster: &[Pick], players: &[PlayerView]) -> Result<(), SessionError> {
    for player in players {
        let index = roster
            .binary_search_by_key(&player.slot, |pick| pick.slot)
            .map_err(|_| SessionError::UnknownRosterPlayer(player.slot.0))?;
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
