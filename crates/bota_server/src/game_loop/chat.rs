//! Chat in the lobby and the match: who hears a line, and when.
//!
//! A line is stamped with the tick its speaker was watching, and each hearer
//! hears it once its own feed has reached that tick: a late spectator hears
//! a seat's all-chat with the fight it is about, never before.

use std::time::Instant;

use bota_core::chat_text_fault;
use bota_proto::{
    Audience, Body, Heard, Line, PlayerId, ReplayRecord, SayRefusal, ServerMsg, Speaker, Team,
};

use super::Server;
use crate::chat::Talker;

/// When a connection hears a line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hearing {
    Now,
    /// With the delayed feed's tick the line was said on.
    Late,
    Never,
}

impl Server {
    /// A line `id` said while the match stood on `tick` (none in the lobby):
    /// passed on to its audience, the speaker among it, or refused to the
    /// speaker alone. The text is never logged.
    pub(super) fn say(&mut self, id: PlayerId, line: Line, tick: Option<u32>) {
        if !self.greeted(id) {
            return;
        }
        let talker = self.talker(id);
        let taken = self.take_line(id, talker, &line);
        if let Err(reason) = taken {
            if let Some(conn) = self.conn(id) {
                conn.send(&ServerMsg::SayRefused { reason });
            }
            return;
        }
        let Some((from, late)) = self.speaker(id) else {
            return;
        };
        let tick = match late {
            true => self.delayed.as_ref().and_then(|feed| feed.shown()),
            false => tick,
        };
        let heard = Heard { tick, from, line };
        self.tell(&heard, late);
        self.keep_for_replay(heard);
    }

    /// Takes a line of the speaker's rate, then checks it may be said: the
    /// rate first, so refused lines cost it too.
    fn take_line(&mut self, id: PlayerId, talker: Talker, line: &Line) -> Result<(), SayRefusal> {
        self.rates.take(id, talker, Instant::now())?;
        if !self.opts.chat.allows(talker, line.to) {
            return Err(SayRefusal::NotAllowed);
        }
        match &line.body {
            Body::Text(text) if chat_text_fault(text).is_some() => Err(SayRefusal::Unfit),
            Body::Text(_) => Ok(()),
        }
    }

    /// What kind of speaker a connection is.
    fn talker(&self, id: PlayerId) -> Talker {
        if self.roster.seat_of(id).is_some() {
            Talker::Seat
        } else if self.spectators.iter().any(|w| w.id == id) {
            Talker::Spectator
        } else {
            Talker::Mute
        }
    }

    /// Who a line of `id`'s is from, and whether `id` watches late.
    fn speaker(&self, id: PlayerId) -> Option<(Speaker, bool)> {
        if let Some(seat) = self.roster.seat_of(id) {
            return Some((Speaker::Seat(seat.slot), false));
        }
        let watcher = self.spectators.iter().find(|w| w.id == id)?;
        Some((Speaker::Spectator(watcher.name.clone()), watcher.late))
    }

    /// Sends a line to everyone who hears it now, and holds it back with the
    /// delayed feed's newest tick for the late spectators who hear it; with
    /// no tick held, they hear it now.
    fn tell(&mut self, heard: &Heard, speaker_late: bool) {
        let msg = ServerMsg::Heard(heard.clone());
        let mut held = false;
        for conn in &self.conns {
            match self.hearing(conn.id, heard, speaker_late) {
                Hearing::Now => conn.send_chat(&msg),
                Hearing::Late => held = true,
                Hearing::Never => {}
            }
        }
        let holds = held && self.delayed.as_mut().is_some_and(|feed| feed.hold(heard));
        if held && !holds {
            for watcher in self.spectators.iter().filter(|w| w.late) {
                if let Some(conn) = self.conn(watcher.id) {
                    conn.send_chat(&msg);
                }
            }
        }
    }

    /// Whether and when connection `id` hears a line.
    fn hearing(&self, id: PlayerId, heard: &Heard, speaker_late: bool) -> Hearing {
        let to = heard.line.to;
        if let Some(seat) = self.roster.seat_of(id) {
            return match (&heard.from, to) {
                (Speaker::Seat(_), Audience::All) => Hearing::Now,
                (Speaker::Seat(from), Audience::Team) if self.team_of(*from) == Some(seat.team) => {
                    Hearing::Now
                }
                _ => Hearing::Never,
            };
        }
        let Some(watcher) = self.spectators.iter().find(|w| w.id == id) else {
            return Hearing::Never;
        };
        let in_step = match watcher.late && !speaker_late {
            true => Hearing::Late,
            false => Hearing::Now,
        };
        match (&heard.from, to) {
            (Speaker::Seat(_), Audience::All) | (Speaker::Spectator(_), Audience::Spectators) => {
                in_step
            }
            // A side's lines reach the spectators that hold one of its seats'
            // keys, never one that only borrows a seat's eyes.
            (Speaker::Seat(from), Audience::Team)
                if watcher.locked
                    && self.team_of(*from).is_some()
                    && watcher.eyes.and_then(|eyes| self.team_of(eyes)) == self.team_of(*from) =>
            {
                Hearing::Now
            }
            _ => Hearing::Never,
        }
    }

    fn team_of(&self, slot: bota_proto::SlotId) -> Option<Team> {
        let seat = self.roster.seats.get(usize::from(slot.0))?;
        Some(seat.team)
    }

    /// Keeps a seat's line for the replay: every all-chat line, and the team
    /// lines unless the server was told to leave them out. Spectators' lines
    /// are not the match's.
    fn keep_for_replay(&mut self, heard: Heard) {
        let kept = match (&heard.from, heard.line.to) {
            (Speaker::Seat(_), Audience::All) => true,
            (Speaker::Seat(_), Audience::Team) => self.opts.chat.replay_team,
            _ => false,
        };
        if kept {
            self.heard.push(heard);
        }
    }

    /// Writes the lines kept since the last call into the replay, in the
    /// order they were taken in.
    pub(super) fn record_heard(&mut self, replay: &mut crate::replay::ReplayWriter) {
        for heard in self.heard.drain(..) {
            replay.record(&ReplayRecord::Heard(heard));
        }
    }

    /// Compresses what is sent to `id` from now on, once it said hello.
    pub(super) fn compress(&self, id: PlayerId) {
        if self.greeted(id)
            && let Some(conn) = self.conn(id)
        {
            conn.compress();
        }
    }
}
