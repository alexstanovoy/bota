//! What makes a handoff, a seat and a Hello bota's.

use bota_proto::{ClientMsg, Role, SeatKey};
use radovid_client::proto::Handoff;

/// The label radovid's rulesets carry for bota.
pub const GAME: &str = "bota";

/// The wire this build speaks, its rules folded in: what its Hello names,
/// and what a ruleset must name to be played with it.
pub fn fingerprint() -> u64 {
    bota_core::WIRE_FINGERPRINT
}

/// Whether `handoff` is a match of bota's wire that this build can play.
pub fn check_handoff(handoff: &Handoff) -> std::io::Result<()> {
    if handoff.game != GAME {
        return Err(std::io::Error::other(format!(
            "the match is of the game {:?}, not {GAME:?}",
            handoff.game
        )));
    }
    if handoff.game_fingerprint != fingerprint() {
        return Err(std::io::Error::other(format!(
            "the match speaks wire {:016x} and this client {:016x}",
            handoff.game_fingerprint,
            fingerprint()
        )));
    }
    Ok(())
}

/// The seat key of the handoff: its ticket, which the game server holds too.
pub fn seat_key(handoff: &Handoff) -> SeatKey {
    SeatKey(handoff.ticket.0)
}

/// The name to announce in the game: the account's nickname in the handoff's
/// seats.
pub fn seat_name(handoff: &Handoff) -> std::io::Result<String> {
    let seat = handoff
        .seats
        .iter()
        .find(|seat| seat.slot == handoff.slot)
        .ok_or_else(|| std::io::Error::other("the handoff lists no seat of the caller"))?;
    Ok(seat.nickname.clone())
}

/// The first message to the game server, for a client that sends its own
/// (the browser's): the handoff's ticket as the key, `role` Player or Bot to
/// sit and Spectator to watch.
pub fn seat_hello(handoff: &Handoff, role: Role) -> std::io::Result<ClientMsg> {
    check_handoff(handoff)?;
    Ok(ClientMsg::hello(
        role,
        seat_name(handoff)?,
        Some(seat_key(handoff)),
        fingerprint(),
    ))
}
