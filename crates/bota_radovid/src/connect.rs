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
    connect_within(handoff, tls, DEFAULT_IO_TIMEOUT)
}

/// As [`connect`], establishing the connection and bounding each of its reads and writes by
/// `timeout`: a caller with a deadline of its own gives what is left of it.
pub fn connect_within(handoff: &Handoff, tls: &Tls, timeout: std::time::Duration) -> Result<Link> {
    Link::open(endpoint(handoff, tls)?, timeout)
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

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::Duration;

    use radovid_client::proto::{MatchId, RulesetId, Ticket};

    use super::*;

    fn handoff(game_url: String) -> Handoff {
        Handoff {
            match_id: MatchId(1),
            ruleset: RulesetId("solo_mid".to_owned()),
            rated: false,
            game: crate::GAME.to_owned(),
            game_url,
            ticket: Ticket([3; 32]),
            slot: 0,
            team: 0,
            seats: Vec::new(),
            game_fingerprint: crate::fingerprint(),
        }
    }

    /// The time given bounds the connection: a link opened with it reaches the game, and none is
    /// opened without any time left.
    #[test]
    fn a_connection_within_a_time_takes_that_time_and_no_other() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("bound");
        let handoff = handoff(format!("tcp://{address}"));
        let tls = Tls::webpki();
        let refused = connect_within(&handoff, &tls, Duration::ZERO)
            .err()
            .expect("no time left");
        assert!(
            refused.to_string().contains("timeout must be positive"),
            "{refused}"
        );
        let link = connect_within(&handoff, &tls, Duration::from_secs(1));
        assert!(link.is_ok(), "the game is reached within the time given");
        listener.accept().expect("the connection came");
    }
}
