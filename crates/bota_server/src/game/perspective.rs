//! The facts each side may learn, shared by served and local matches.

use bota_proto::{EventKind, SlotId, SlotOrder, Team, WorldView};

use crate::game::{DelayLine, Event, Handles, World};

/// What one side sees of the world, in its own names, with the order delays
/// it is told.
pub(crate) fn side_view(
    world: &World,
    line: &DelayLine,
    handles: &mut Handles,
    team: Team,
) -> WorldView {
    let mut view = world.view(team);
    view.order_delays = line.views(Some(team));
    handles.tell_view(world, &mut view);
    view
}

/// The orders of one seat taken on a tick, in the names of the side shown
/// them.
pub(crate) fn seat_orders(
    taken: &[SlotOrder],
    seat: SlotId,
    (handles, team): (&mut Handles, Team),
    tick: u32,
) -> Vec<SlotOrder> {
    let mut told: Vec<SlotOrder> = taken
        .iter()
        .filter(|given| given.slot == seat)
        .copied()
        .collect();
    handles.tell_orders(team, tick, &mut told);
    told
}

/// The events one side is told of on a tick, in its own names.
pub(crate) fn side_events(
    events: &[Event],
    handles: &mut Handles,
    team: Team,
    tick: u32,
) -> Vec<EventKind> {
    let told = visible_events(events, Some(team));
    let (mut told, runs) = disguise_level_ups(told, handles, team);
    handles.tell_events(team, tick, &mut told);
    // In the side's own names, a hero's level up cannot be told from its
    // illusions' by where it stands among them.
    for run in runs {
        told[run].sort_unstable_by_key(|event| match event {
            EventKind::LevelUp { unit, .. } => *unit,
            _ => unreachable!("a run holds level ups alone"),
        });
    }
    told
}

/// The events a side is told with every level up of the other side's hero
/// told once more for each illusion of it, and where each such run of level
/// ups stands.
fn disguise_level_ups(
    events: Vec<EventKind>,
    handles: &Handles,
    team: Team,
) -> (Vec<EventKind>, Vec<core::ops::Range<usize>>) {
    let mut told = Vec::with_capacity(events.len());
    let mut runs = Vec::new();
    for event in events {
        let EventKind::LevelUp { unit, level } = event else {
            told.push(event);
            continue;
        };
        let start = told.len();
        told.push(event);
        let illusions = handles.illusions_of(team, unit);
        told.extend(
            illusions
                .iter()
                .map(|&unit| EventKind::LevelUp { unit, level }),
        );
        if !illusions.is_empty() {
            runs.push(start..told.len());
        }
    }
    (told, runs)
}

/// The kinds of the events a side may see. No side sees all of them; a
/// dealer the side does not see is told as nobody, and a death's gold is told
/// only to the side that earned it unless a hero fell: the other side is
/// told none, so a visible creep kill never leaks the enemy's income.
pub(crate) fn visible_events(events: &[Event], viewing: Option<Team>) -> Vec<EventKind> {
    events
        .iter()
        .filter(|event| {
            viewing.is_none_or(|mine| {
                event.visible_to().tells(mine)
                    || matches!(event,
                        Event::Fact { kind: EventKind::Died { awards: Some(awards), .. }, .. }
                            if awards.iter().any(|award| award.team == mine))
            })
        })
        .map(|e| {
            let mut kind = e.kind();
            if let Some(mine) = viewing {
                told_as(&mut kind, e, mine);
            }
            kind
        })
        .collect()
}

/// Blanks what an event tells that one side may not learn from it.
fn told_as(kind: &mut EventKind, event: &Event, mine: Team) {
    if let Event::ItemUsed { point_told_to, .. } = event
        && !point_told_to.tells(mine)
        && let EventKind::ItemUsed { at, target, .. } = kind
    {
        *at = None;
        *target = None;
    }
    let hide_dealer = !event.source_told_to().tells(mine);
    match kind {
        // What it was dealt with would name what an unseen dealer carries.
        EventKind::Damaged {
            source, inflictor, ..
        }
        | EventKind::Healed {
            source, inflictor, ..
        } if hide_dealer => {
            *source = None;
            *inflictor = None;
        }
        EventKind::Missed { source, .. } if hide_dealer => *source = None,
        EventKind::Landed { caster, .. }
        | EventKind::TreeFelled { by: caster, .. }
        | EventKind::ItemUsed { caster, .. }
            if hide_dealer =>
        {
            *caster = None
        }
        EventKind::Captured { by, .. } | EventKind::ShrineActivated { by, .. } if hide_dealer => {
            *by = None
        }
        EventKind::LotusPlucked { unit, .. } if hide_dealer => *unit = None,
        EventKind::Died {
            killer,
            bounty,
            assists,
            awards,
            ..
        } => {
            if hide_dealer {
                *killer = None;
                *assists = 0;
            }
            if event.gold_hidden_from() == Some(mine) {
                *bounty = None;
            }
            if let Some(disclosed) = awards {
                disclosed.retain(|award| award.team == mine);
                if disclosed.is_empty() {
                    *awards = None;
                }
            }
        }
        _ => {}
    }
}
