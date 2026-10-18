//! Into the game a handoff names, over whatever transport its URL says.

use std::io::Result;

use bota_client_core::{DEFAULT_IO_TIMEOUT, Endpoint, Link, Net, Redial, Seated, greet};
use bota_proto::{HeroId, Role};
use radovid_client::proto::Handoff;

use crate::game::{check_handoff, seat_key, seat_name};
use crate::tls::Tls;

fn endpoint(handoff: &Handoff, tls: &Tls) -> Result<Endpoint> {
    check_handoff(handoff)?;
    Ok(Endpoint::parse(&handoff.game_url)?.with_tls(tls.game.clone()))
}

/// Opens a link to the game server of `handoff` without saying Hello:
/// `tcp://`, `ws://` or `wss://` by the URL's scheme. A break is dialed
/// again, and a rejoin greets the server again with the ticket.
pub fn connect(handoff: &Handoff, tls: &Tls) -> Result<Link> {
    Link::open(endpoint(handoff, tls)?, DEFAULT_IO_TIMEOUT)
}

/// As [`connect`], on a thread of its own, for a frame loop that must not
/// block.
pub fn connect_net(handoff: &Handoff, tls: &Tls) -> Result<Net> {
    Net::open(endpoint(handoff, tls)?, Redial::DEFAULT)
}

/// Takes the seat of `handoff` as a bot, with the ticket as the seat key,
/// picks `hero` and says ready. Taking it again later (after the link or the
/// client went away) is a reconnect: the server gives the seat back to the
/// ticket's holder.
pub fn take_seat(handoff: &Handoff, hero: HeroId, tls: &Tls) -> Result<(Link, Seated)> {
    Link::join_at(
        endpoint(handoff, tls)?,
        &seat_name(handoff)?,
        hero,
        Some(seat_key(handoff)),
        DEFAULT_IO_TIMEOUT,
        Redial::DEFAULT,
    )
}

/// Watches the seat of `handoff` with its ticket: live, with the seat's own
/// orders, and never holding the seat. Read the match from the link as any
/// spectator does.
pub fn spectate(handoff: &Handoff, tls: &Tls) -> Result<Link> {
    let mut link = connect(handoff, tls)?;
    greet(
        &mut link,
        Role::Spectator,
        &seat_name(handoff)?,
        Some(seat_key(handoff)),
    )?;
    Ok(link)
}
