//! The chat log a front end overlays, and what this client may say.

use std::collections::VecDeque;

use bota_core::TextFault;
use bota_proto::{Audience, Heard, Ping, SayRefusal, ServerMsg, SlotId, Speaker, Team};

use crate::chat::{ping, say};
use crate::ui::app::{App, Source};

/// Lines the log keeps at most; the oldest give way.
pub const MAX_CHAT_LINES: usize = 100;

/// Seconds a refusal of this client's own line is shown.
const REFUSAL_SECS: f32 = 3.0;

/// One line of the chat log.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub heard: Heard,
    /// Whether the speaker's hero lay dead when the line came: Dota marks
    /// such a line.
    pub dead: bool,
    /// Seconds since it came.
    pub age: f32,
}

/// The chat as this client knows it.
#[derive(Clone, Debug, Default)]
pub struct Chat {
    lines: VecDeque<ChatLine>,
    /// Why this client's last line was not passed on, and the seconds left
    /// to show it.
    pub refused: Option<(SayRefusal, f32)>,
}

impl Chat {
    /// Every line kept, oldest first.
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &ChatLine> {
        self.lines.iter()
    }

    fn push(&mut self, line: ChatLine) {
        if self.lines.len() == MAX_CHAT_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// Ages the lines and the refusal by `dt` seconds.
    pub fn age(&mut self, dt: f32) {
        for line in &mut self.lines {
            line.age += dt;
        }
        if let Some((_, left)) = &mut self.refused {
            *left -= dt;
            if *left <= 0.0 {
                self.refused = None;
            }
        }
    }
}

impl App {
    /// The lines this viewer reads, oldest first. A replay watched through a
    /// seat's eyes shows everyone's lines and that side's team lines only,
    /// as the seat heard them; otherwise every line this client was told.
    pub fn chat_lines(&self) -> impl DoubleEndedIterator<Item = &ChatLine> {
        let side = match self.source {
            Source::Replay(_) => self.pov.and_then(|slot| self.side_of(slot)),
            Source::Live(_) => None,
        };
        self.chat
            .lines()
            .filter(move |line| match (&line.heard.from, line.heard.line.to) {
                (Speaker::Seat(from), Audience::Team) => {
                    side.is_none_or(|side| self.side_of(*from) == Some(side))
                }
                _ => true,
            })
    }

    /// Whether this client may say anything: a live link, welcomed.
    pub fn can_say(&self) -> bool {
        matches!(self.source, Source::Live(_)) && self.lobby.me.is_some()
    }

    /// The audiences this client speaks to: what Enter opens (a seat's team,
    /// a spectator's fellows) and what Shift+Enter opens (everyone; none for
    /// a spectator, whom no seat hears).
    pub fn chat_audiences(&self) -> (Audience, Option<Audience>) {
        match self.lobby.slot {
            Some(_) => (Audience::Team, Some(Audience::All)),
            None => (Audience::Spectators, None),
        }
    }

    /// Says `text` to `to`, trimmed, or tells why it may not be said; the
    /// line shows once the server passes it back.
    pub fn say(&mut self, to: Audience, text: &str) -> Result<(), TextFault> {
        let message = say(to, text)?;
        self.source.send(&message);
        Ok(())
    }

    /// Pings for this client's side; the mark shows once the server passes
    /// it back. A unit is named as this client's view names it.
    pub fn ping(&mut self, at: Ping) {
        self.source.send(&ping(at));
    }

    /// What a speaker is called: a seat by its name, a spectator by its own.
    pub fn speaker_name(&self, from: &Speaker) -> String {
        match from {
            Speaker::Seat(slot) => self.seat_name(*slot),
            Speaker::Spectator(name) => name.clone(),
        }
    }

    /// Takes a chat message the stream passed in.
    pub(crate) fn note_chat(&mut self, msg: &ServerMsg) {
        match msg {
            ServerMsg::Heard(heard) => {
                let dead = match heard.from {
                    Speaker::Seat(slot) => self.player(slot).is_some_and(|p| p.unit.is_none()),
                    Speaker::Spectator(_) => false,
                };
                self.chat.push(ChatLine {
                    heard: heard.clone(),
                    dead,
                    age: 0.0,
                });
            }
            ServerMsg::SayRefused { reason } => self.chat.refused = Some((*reason, REFUSAL_SECS)),
            _ => {}
        }
    }

    /// The side a seat plays for, from the match or the lobby.
    fn side_of(&self, slot: SlotId) -> Option<Team> {
        if let Some(pick) = self.statics().and_then(|statics| statics.pick(slot)) {
            return Some(pick.team);
        }
        let seat = self.lobby.slots.iter().find(|seat| seat.slot == slot)?;
        Some(seat.team)
    }
}

/// How a refusal of one's own line reads.
pub fn say_refusal(reason: SayRefusal) -> String {
    match reason {
        SayRefusal::TooFast { retry_in_ms } => {
            format!(
                "You are sending messages too fast; wait {:.1} s",
                retry_in_ms as f32 / 1000.0
            )
        }
        SayRefusal::NotAllowed => "Nobody here may hear that".to_owned(),
        SayRefusal::Unfit => "That message cannot be sent".to_owned(),
    }
}
