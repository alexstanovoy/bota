//! Chat as a client says it and hears it: the text checked before it goes
//! out, and every line checked again when it comes in. A line that fails is
//! dropped, never fatal: chat is not the match.

use bota_core::{TextFault, chat_text_fault, clean_name, ping_fault};
use bota_proto::{Audience, Body, ClientMsg, Heard, Line, Ping, SlotId, Speaker, Team};

/// The message that says `text` to `to`, trimmed, or why it may not be
/// said: the server would refuse it as
/// [`SayRefusal::Unfit`](bota_proto::SayRefusal::Unfit).
pub fn say(to: Audience, text: &str) -> Result<ClientMsg, TextFault> {
    let text = text.trim();
    if let Some(fault) = chat_text_fault(text) {
        return Err(fault);
    }
    Ok(ClientMsg::Say(Line {
        to,
        body: Body::Text(text.to_owned()),
    }))
}

/// The message that pings for one's own side: pings go to the team alone.
pub fn ping(ping: Ping) -> ClientMsg {
    ClientMsg::Say(Line {
        to: Audience::Team,
        body: Body::Ping(ping),
    })
}

/// Who is hearing: what a line must agree with to be believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hearer<'a> {
    /// Whether it holds a seat. A seat never hears spectators.
    pub seated: bool,
    /// The side whose team lines it may hear: a seat's own, the side a
    /// spectator's eyes belong to. Absent hears both sides' (a replay of the
    /// whole map).
    pub side: Option<Team>,
    /// Every seat of the match or the lobby, with its side.
    pub seats: &'a [(SlotId, Team)],
    /// The newest tick heard of; a line is never from a later one.
    pub tick: Option<u32>,
}

/// Whether `heard` is a line this hearer could have been told: its text
/// keeps the rules, its speaker is a seat of the match or a spectator under
/// a clean name, and its audience includes the hearer.
pub fn heard_fits(heard: &Heard, hearer: Hearer<'_>) -> bool {
    let fits = match &heard.line.body {
        Body::Text(text) => chat_text_fault(text).is_none(),
        Body::Ping(ping) => heard.line.to == Audience::Team && !ping_fault(*ping),
    };
    if !fits {
        return false;
    }
    if heard
        .tick
        .is_some_and(|tick| hearer.tick.is_none_or(|newest| tick > newest))
    {
        return false;
    }
    let speaker_side = match &heard.from {
        Speaker::Seat(slot) => match hearer.seats.iter().find(|(seat, _)| seat == slot) {
            Some((_, side)) => Some(*side),
            None => return false,
        },
        Speaker::Spectator(name) if clean_name(name) == *name => None,
        Speaker::Spectator(_) => return false,
    };
    match (heard.line.to, speaker_side) {
        (Audience::All, Some(_)) => true,
        (Audience::Team, Some(side)) => hearer.side.is_none_or(|own| own == side),
        (Audience::Spectators, None) => !hearer.seated,
        _ => false,
    }
}
