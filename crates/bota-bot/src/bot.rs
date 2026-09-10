//! The seam between a match and whatever decides what to do in it.
//!
//! A bot is handed a seat, the terms of the match, and then one tick at a
//! time; it answers each tick with at most one [`Ask`]. [`play`] is the loop
//! that carries that conversation over a socket.

use bota_proto::{
    EventKind, HeroId, MatchInfo, MatchStats, PlayerView, RejectReason, ServerMsg, SlotId, Team,
    TickMode, WorldView,
};

use crate::{Ask, Link, Seated};

/// Anything that can hold a seat in a match.
///
/// The hero is picked when the connection is made rather than asked for here:
/// picking happens in the lobby, before there is a [`MatchInfo`] to decide
/// from.
pub trait Bot {
    /// Told which seat the server gave this connection, before the match
    /// starts. Absent when no seat was free.
    fn seated(&mut self, slot: Option<SlotId>);

    /// Told the terms of the match, once, when it begins.
    fn match_started(&mut self, info: &MatchInfo);

    /// Handed one tick, and answers with at most one order.
    fn on_tick(&mut self, view: &WorldView) -> Option<Ask>;

    /// Told what happened during a tick, after that tick's snapshot.
    fn on_events(&mut self, _tick: u32, _events: &[EventKind]) {}

    /// Told that an order was not accepted.
    fn on_reject(&mut self, _seq: u32, _reason: RejectReason) {}

    /// Told how the match ended.
    fn finished(&mut self, _winner: Team, _stats: &MatchStats) {}
}

/// What to join, as what, and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chair {
    /// Where the server listens.
    pub addr: String,
    /// The name the lobby shows.
    pub name: String,
    /// Which hero to ask for.
    pub hero: HeroId,
    /// Ticks to play before leaving. Absent plays until the match ends.
    pub limit: Option<u32>,
}

/// What one match came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The seat that was held. Absent when none was free.
    pub slot: Option<SlotId>,
    /// The side that seat played for.
    pub team: Option<Team>,
    /// Which side won. Absent when the bot left before the end.
    pub winner: Option<Team>,
    /// The last tick that was seen.
    pub ticks: u32,
    /// The seat's own last row of the scoreboard.
    pub mine: Option<PlayerView>,
    /// The final numbers, when the match ran to its end.
    pub stats: Option<MatchStats>,
    /// Orders the server would not take.
    pub rejected: u32,
    /// The reasons it gave, counted.
    pub refusals: Vec<(RejectReason, u32)>,
    /// Ticks an order was sent on.
    pub ordered: u32,
}

/// Joins a server and plays one match.
pub fn play(bot: &mut dyn Bot, chair: &Chair) -> std::io::Result<Outcome> {
    let (link, seated) = Link::join(&chair.addr, &chair.name, chair.hero)?;
    play_on(bot, link, seated, chair)
}

/// The same, on a connection that has already been given its seat.
pub fn play_on(
    bot: &mut dyn Bot,
    mut link: Link,
    seated: Seated,
    chair: &Chair,
) -> std::io::Result<Outcome> {
    let mut out = Outcome {
        slot: seated.slot,
        ..Outcome::default()
    };
    bot.seated(seated.slot);
    let Some(slot) = seated.slot else {
        return Err(std::io::Error::other("no seat was free"));
    };
    let lockstep = seated.mode == TickMode::Lockstep;
    while let Some(msg) = link.hear()? {
        match msg {
            ServerMsg::MatchStart { info } => {
                out.team = info
                    .picks
                    .iter()
                    .find(|pick| pick.slot == slot)
                    .map(|pick| pick.team);
                bot.match_started(&info);
            }
            ServerMsg::Snapshot { view } => {
                out.ticks = view.tick;
                out.mine = view
                    .players
                    .iter()
                    .find(|player| player.slot == slot)
                    .cloned();
                if let Some(ask) = bot.on_tick(&view) {
                    out.ordered += 1;
                    link.order(ask)?;
                }
                if lockstep {
                    link.done_thinking(view.tick)?;
                }
                if chair.limit.is_some_and(|limit| view.tick >= limit) {
                    return Ok(out);
                }
            }
            ServerMsg::Events { tick, events } => bot.on_events(tick, &events),
            ServerMsg::OrderRejected { seq, reason } => {
                out.rejected += 1;
                match out.refusals.iter_mut().find(|(had, _)| *had == reason) {
                    Some((_, many)) => *many += 1,
                    None => out.refusals.push((reason, 1)),
                }
                bot.on_reject(seq, reason);
            }
            ServerMsg::MatchOver { winner, stats } => {
                out.winner = Some(winner);
                bot.finished(winner, &stats);
                out.stats = Some(stats);
                return Ok(out);
            }
            ServerMsg::Welcome { .. }
            | ServerMsg::LobbyState { .. }
            | ServerMsg::ParticipantLeft { .. }
            | ServerMsg::Orders { .. } => {}
        }
    }
    Ok(out)
}
