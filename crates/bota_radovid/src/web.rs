//! The browser's way into a game.

use bota_client_core::WebSocketLink;
use radovid_client::proto::Handoff;

use crate::game::check_handoff;

/// Starts connecting to the game server of `handoff` (`ws://` or `wss://`).
/// The browser's link does not block, so the caller sends the
/// [`seat_hello`](crate::seat_hello) and the rest of the lobby itself.
pub fn web_link(handoff: &Handoff) -> std::io::Result<WebSocketLink> {
    check_handoff(handoff)?;
    WebSocketLink::connect(&handoff.game_url)
}
