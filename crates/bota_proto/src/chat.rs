//! What players and spectators say to each other: never part of the match
//! (see the [`contract`](crate::contract)).

use crate::SlotId;
use serde::{Deserialize, Serialize};

/// Who a line is for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Audience {
    /// Every seat and every spectator.
    All,
    /// The speaker's side: its seats, and the spectators watching through
    /// one of them with its key.
    Team,
    /// The spectators. A seat never hears it.
    Spectators,
}

/// What a line says.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    /// Plain text: one line of Unicode within
    /// `bota_core::rules::MAX_CHAT_BYTES` and
    /// `bota_core::rules::MAX_CHAT_GRAPHEMES`, without control, bidi or
    /// invisible characters (`bota_core::chat_text_fault`).
    Text(String),
}

/// One thing said: to whom, and what.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Line {
    pub to: Audience,
    pub body: Body,
}

/// Who said a line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    /// The player or bot holding this seat, alive or dead.
    Seat(SlotId),
    /// A spectator, by the name it announced, cleaned and cut as a seat's
    /// name is.
    Spectator(String),
}

/// A line as its hearers are told it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Heard {
    /// The game tick the speaker was watching when it spoke: the live tick
    /// for a seat, the tick its feed stood on for a late spectator. Absent
    /// before the match.
    pub tick: Option<u32>,
    pub from: Speaker,
    pub line: Line,
}

/// Why a [`ClientMsg::Say`](crate::ClientMsg::Say) was not passed on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SayRefusal {
    /// Over the speaker's rate; the line is dropped. A line sent once this
    /// many milliseconds have passed would be taken.
    TooFast { retry_in_ms: u32 },
    /// This speaker may not talk to this audience on this server, or the
    /// server's chat is off.
    NotAllowed,
    /// The text breaks the text rules: empty, too long, or holding a
    /// character no line may hold.
    Unfit,
}
