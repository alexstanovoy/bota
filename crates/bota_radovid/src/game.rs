//! What makes a handoff, a seat and a Hello bota's.

use bota_proto::{ClientMsg, Role, SeatKey};
use radovid_client::proto::{Handoff, RulesetInfo};

/// The label radovid's rulesets carry for bota.
pub const GAME: &str = "bota";

/// A bit of the first byte of the `info` a bota ruleset carries (radovid hands it to clients untouched
/// and reads none of it): the ruleset is Dota's Demo Hero, one player with cheats on the whole map, which
/// a client opens from its own entry and never lists beside the games to search or sit down in. The
/// service operator sets it with the launcher's `--cheats`; later bits are appended.
pub const INFO_DEMO: u8 = 1;

/// Whether `ruleset` is bota's Demo Hero (see [`INFO_DEMO`]).
pub fn is_demo(ruleset: &RulesetInfo) -> bool {
    ruleset
        .info
        .first()
        .is_some_and(|flags| flags & INFO_DEMO != 0)
}

/// The wire this build speaks, its rules folded in. A ruleset of it, or of
/// an earlier wire this build admits ([`bota_core::ADMITTED_WIRES`]), is
/// played with it; the Hello names the handoff's ([`seat_hello`]).
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
    if !bota_core::admits_wire(handoff.game_fingerprint) {
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
/// sit and Spectator to watch, speaking the server's wire as the handoff names
/// it (this build's, or an earlier one it admits). The client's catalogue
/// follows it, the items it holds on that wire
/// (`bota_client_core::catalogue_after`).
pub fn seat_hello(handoff: &Handoff, role: Role) -> std::io::Result<ClientMsg> {
    check_handoff(handoff)?;
    Ok(ClientMsg::hello_with_version(
        role,
        seat_name(handoff)?,
        Some(seat_key(handoff)),
        handoff.game_fingerprint,
        (role == Role::Bot)
            .then(|| {
                handoff
                    .seats
                    .iter()
                    .find(|seat| seat.slot == handoff.slot)
                    .and_then(|seat| seat.version.clone())
            })
            .flatten(),
    ))
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn version_names_have_the_same_bound_and_validation_on_both_wires() {
        assert_eq!(
            bota_proto::MAX_VERSION_LEN,
            radovid_client::proto::MAX_VERSION_LEN
        );
        let check = |value: &str| {
            assert_eq!(
                bota_proto::valid_version(value),
                radovid_client::proto::valid_version(value),
                "{value:?}"
            )
        };
        for byte in 0..=255u8 {
            check(&format!("a{}b", char::from(byte)));
        }
        for length in 0..=bota_proto::MAX_VERSION_LEN + 1 {
            check(&"x".repeat(length));
        }
        for value in [" leading", "trailing ", "v1, (x:y)", "v1\u{200b}", "v1\n"] {
            check(value);
        }
    }
}
