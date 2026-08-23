//! Playing one match: the loop between a server and a mind.
//!
//! It reads a tick, turns it into what a mind is shown, asks for one deed,
//! turns that back into an order, and acknowledges. Everything it knows about
//! the game is in [`Field`]; everything it knows about deciding is that a mind
//! answers with a number.

use bota_proto::{HeroId, MatchStats, PlayerView, ServerMsg, SlotId, Team, TickMode};

use crate::{Card, Deed, Field, Lesson, Link, Marker, Mind, Moment, Role, Wire, lane_of, shown};

/// What to join, as what, and for how long.
#[derive(Clone, Debug)]
pub struct Chair {
    /// Where the server listens.
    pub addr: String,
    /// The name the lobby shows.
    pub name: String,
    /// Which hero to ask for.
    pub hero: HeroId,
    /// Ticks to play before leaving. Absent plays until the match ends.
    pub limit: Option<u32>,
    /// What the seat is there to do.
    pub role: Role,
    /// What it is being taught, which decides what a tick pays.
    pub lesson: Lesson,
    /// The tick that lesson stops being paid on. Absent pays it for as long as
    /// its rung of the ladder says.
    pub until: Option<u32>,
}

/// What one match came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Which seat it held.
    pub slot: Option<SlotId>,
    /// Which side that seat played for.
    pub team: Option<Team>,
    /// Which side won. Absent when the bot left before the end.
    pub winner: Option<Team>,
    /// The last tick it saw.
    pub ticks: u32,
    /// Its own last row of the scoreboard.
    pub mine: Option<PlayerView>,
    /// The final numbers, when the match ran to its end.
    pub stats: Option<MatchStats>,
    /// Orders the server would not take.
    pub rejected: u32,
    /// The reasons it gave, counted.
    ///
    /// Counted rather than only totalled: a refusal is a place where what the
    /// bot believes it may do and what the server enforces have come apart, and
    /// which reason it was says which of the two is wrong.
    pub refusals: Vec<(bota_proto::RejectReason, u32)>,
    /// Ticks it chose a deed on.
    pub chose: u32,
    /// Ticks it chose a deed that could not be done.
    ///
    /// Should stay at nought: what may be done is handed to the mind, so
    /// anything here is a mind that ignored it, and every one of them is a
    /// tick thrown away.
    pub refused: u32,
    /// What every lesson paid it over the match, side by side.
    ///
    /// Every one of them however few are being taught: a match run to the
    /// longest lesson's clock is a reading of the whole ladder, and reading
    /// them off one match is what makes the numbers comparable.
    pub card: Card,
}

/// Joins, plays, and returns what the match came to.
pub fn play(mind: &mut (dyn Mind + Send), chair: &Chair) -> std::io::Result<Outcome> {
    let (link, seated) = Link::join(&chair.addr, &chair.name, chair.hero)?;
    play_on(mind, link, seated, chair)
}

/// The same, on a connection that has already been given its seat.
pub fn play_on(
    mind: &mut (dyn Mind + Send),
    mut link: impl Wire,
    seated: crate::Seated,
    chair: &Chair,
) -> std::io::Result<Outcome> {
    let Chair {
        limit,
        role,
        lesson,
        ..
    } = *chair;
    let mut out = Outcome {
        slot: seated.slot,
        ..Outcome::default()
    };
    let lockstep = seated.mode == TickMode::Lockstep;
    let Some(slot) = seated.slot else {
        return Err(std::io::Error::other("no seat was free"));
    };
    mind.starting();
    // The tick being gathered: its snapshot and the events that arrived after
    // it. A tick is scored once, when the next snapshot shows what it came to.
    let mut held: Option<bota_proto::WorldView> = None;
    let mut during: Vec<bota_proto::EventKind> = Vec::new();
    // Every lesson is marked at once, whichever one is being taught: one match
    // run to the longest clock is a reading of the whole ladder.
    let mut marker = match chair.until {
        None => Marker::new(),
        Some(ticks) => Marker::paid_until(lesson, ticks),
    };
    while let Some(msg) = link.hear()? {
        match msg {
            ServerMsg::MatchStart { info } => {
                out.team = info
                    .picks
                    .iter()
                    .find(|pick| pick.slot == slot)
                    .map(|pick| pick.team);
            }
            ServerMsg::Snapshot { view } => {
                // The tick before this one is only now finished: its events
                // have all arrived, and this snapshot's scoreboard says what it
                // came to. So it is scored here, once, whole.
                let was = out
                    .mine
                    .as_ref()
                    .map_or((0, 0, 0), |row| (row.last_hits, row.kills, row.deaths));
                let mine = view
                    .players
                    .iter()
                    .find(|player| player.slot == slot)
                    .cloned();
                let now = mine
                    .as_ref()
                    .map_or((0, 0, 0), |row| (row.last_hits, row.kills, row.deaths));
                if let Some(before) = held.as_ref() {
                    let paid = close_a_tick(
                        &mut marker,
                        before,
                        slot,
                        role,
                        &during,
                        (
                            now.0.saturating_sub(was.0),
                            now.1.saturating_sub(was.1),
                            now.2.saturating_sub(was.2),
                        ),
                    );
                    mind.paid(before.tick, paid.of(lesson));
                }
                during.clear();
                out.ticks = view.tick;
                out.mine = mine;
                if let Some(field) = Field::of(&view, slot, role) {
                    let seen = shown(&field);
                    if let Some(at) = mind.choose(&seen) {
                        out.chose += 1;
                        // A mind that names a deed it was told it could not do
                        // gets nothing: the tick is already spent, and sending
                        // the order anyway would only have it refused further
                        // along.
                        match seen.allowed.get(at) {
                            Some(true) => {
                                if let Some(ask) =
                                    Deed::at(at).and_then(|deed| deed.into_ask(&field))
                                {
                                    link.order(ask)?;
                                }
                            }
                            _ => out.refused += 1,
                        }
                    }
                }
                held = Some(view);
                if lockstep {
                    link.done_thinking(held.as_ref().expect("just held").tick)?;
                }
                if limit.is_some_and(|limit| held.as_ref().is_some_and(|view| view.tick >= limit)) {
                    out.card = last_tick(&mut marker, held.as_ref(), slot, role, &during);
                    return Ok(out);
                }
            }
            ServerMsg::Events { events, .. } => {
                // Kept for the tick they belong to rather than scored now: they
                // arrive after its snapshot, and a lesson is one function over
                // a whole tick rather than one for standing and one for blows.
                during.extend(events);
            }
            ServerMsg::OrderRejected { reason, .. } => {
                out.rejected += 1;
                match out.refusals.iter_mut().find(|(had, _)| *had == reason) {
                    Some((_, many)) => *many += 1,
                    None => out.refusals.push((reason, 1)),
                }
            }
            ServerMsg::MatchOver { winner, stats } => {
                out.winner = Some(winner);
                out.stats = Some(stats);
                out.card = last_tick(&mut marker, held.as_ref(), slot, role, &during);
                return Ok(out);
            }
            ServerMsg::Welcome { .. }
            | ServerMsg::LobbyState { .. }
            | ServerMsg::ParticipantLeft { .. } => {}
        }
    }
    out.card = last_tick(&mut marker, held.as_ref(), slot, role, &during);
    Ok(out)
}

/// Scores one finished tick, and says what it paid lesson by lesson.
fn close_a_tick(
    marker: &mut Marker,
    view: &bota_proto::WorldView,
    slot: SlotId,
    role: Role,
    during: &[bota_proto::EventKind],
    scored: (u16, u16, u16),
) -> Card {
    let Some(field) = Field::of(view, slot, role) else {
        return Card::new();
    };
    let lane = lane_of(&field, role);
    marker.tick(&Moment {
        field: &field,
        lane: lane.as_ref(),
        events: during,
        took: scored.0,
        killed: scored.1,
        died: scored.2,
    })
}

/// Scores the tick the match ended on, and hands back the whole card.
///
/// The scoreboard never moves again, so what that tick came to is only what its
/// events say. Left unscored, a match ending on the tick a tower falls would
/// not be paid for the tower.
fn last_tick(
    marker: &mut Marker,
    held: Option<&bota_proto::WorldView>,
    slot: SlotId,
    role: Role,
    during: &[bota_proto::EventKind],
) -> Card {
    if let Some(view) = held {
        close_a_tick(marker, view, slot, role, during, (0, 0, 0));
    }
    marker.card()
}
