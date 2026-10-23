//! What the server lets be said, and how fast. Chat is beside the match: it
//! reads no world state, never touches the order line or the
//! acknowledgements, and a line is never part of a tick's hash.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use bota_proto::{Audience, PlayerId, SayRefusal};

/// What the seats may say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatChat {
    /// Nothing: a ladder of bots, where chat would be a channel between
    /// opponents.
    Off,
    /// To their own side only.
    Team,
    /// To their side and to everyone.
    All,
}

/// Who may talk, and whether the replay keeps the sides' team lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatOpts {
    pub seats: SeatChat,
    /// Whether spectators may talk among themselves.
    pub spectators: bool,
    /// Whether the replay records team lines. Whoever holds the replay of
    /// the whole map reads them then, as it sees everything else.
    pub replay_team: bool,
}

impl ChatOpts {
    /// Everyone talks, and the replay keeps every line a seat said.
    pub const DEFAULT: ChatOpts = ChatOpts {
        seats: SeatChat::All,
        spectators: true,
        replay_team: true,
    };
}

impl FromStr for SeatChat {
    type Err = String;

    fn from_str(value: &str) -> Result<SeatChat, String> {
        match value {
            "off" => Ok(SeatChat::Off),
            "team" => Ok(SeatChat::Team),
            "all" => Ok(SeatChat::All),
            _ => Err(format!("'{value}' is none of off, team, all")),
        }
    }
}

impl fmt::Display for SeatChat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SeatChat::Off => "off",
            SeatChat::Team => "team",
            SeatChat::All => "all",
        })
    }
}

/// Who is speaking, as far as the rules of chat care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Talker {
    Seat,
    Spectator,
    /// A referee, or a connection that has not said hello.
    Mute,
}

impl ChatOpts {
    /// Whether `talker` may say a line to `to`.
    pub fn allows(self, talker: Talker, to: Audience) -> bool {
        match (talker, to) {
            (Talker::Seat, Audience::All) => self.seats == SeatChat::All,
            (Talker::Seat, Audience::Team) => self.seats != SeatChat::Off,
            (Talker::Spectator, Audience::Spectators) => self.spectators,
            _ => false,
        }
    }
}

/// Lines one connection may say in a burst.
pub const LINE_BURST: u32 = 5;
/// Once its burst is spent, a connection may say one line this often.
pub const LINE_SPACING: Duration = Duration::from_secs(1);
/// Spectator lines the whole match may carry in a burst: however many
/// spectators come, they cannot drown each other out faster than this.
pub const SPECTATOR_BURST: u32 = 10;
/// Once that burst is spent, one spectator line this often.
pub const SPECTATOR_SPACING: Duration = Duration::from_millis(500);

/// A burst of lines, then one line every `spacing`. Kept as the time every
/// line taken so far is paid for, as the connection allowance is.
#[derive(Clone, Copy, Debug)]
pub struct LineRate {
    spacing: Duration,
    burst: Duration,
    paid_until: Instant,
}

impl LineRate {
    pub fn new(burst: u32, spacing: Duration, now: Instant) -> LineRate {
        LineRate {
            spacing,
            burst: spacing.saturating_mul(burst.saturating_sub(1)),
            paid_until: now,
        }
    }

    /// Takes one line, or says when one would be taken.
    pub fn take(&mut self, now: Instant) -> Result<(), SayRefusal> {
        let paid = self.paid_until.max(now);
        if paid > now + self.burst {
            let wait = paid - (now + self.burst);
            let retry_in_ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).max(1);
            return Err(SayRefusal::TooFast { retry_in_ms });
        }
        self.paid_until = paid + self.spacing;
        Ok(())
    }
}

/// The line allowances of every connection that spoke, and the one all
/// spectators share.
pub struct Rates {
    each: Vec<(PlayerId, LineRate)>,
    spectators: LineRate,
}

impl Rates {
    pub fn new(now: Instant) -> Rates {
        Rates {
            each: Vec::new(),
            spectators: LineRate::new(SPECTATOR_BURST, SPECTATOR_SPACING, now),
        }
    }

    /// Takes one line of `id`'s, and of the spectators' when it is one.
    pub fn take(&mut self, id: PlayerId, talker: Talker, now: Instant) -> Result<(), SayRefusal> {
        let at = match self.each.iter().position(|(owner, _)| *owner == id) {
            Some(at) => at,
            None => {
                self.each
                    .push((id, LineRate::new(LINE_BURST, LINE_SPACING, now)));
                self.each.len() - 1
            }
        };
        let mut own = self.each[at].1;
        own.take(now)?;
        if talker == Talker::Spectator {
            self.spectators.take(now)?;
        }
        self.each[at].1 = own;
        Ok(())
    }

    /// Lets go of a connection that ended.
    pub fn forget(&mut self, id: PlayerId) {
        self.each.retain(|(owner, _)| *owner != id);
    }
}
