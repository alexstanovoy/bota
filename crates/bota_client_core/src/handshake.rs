//! The server's welcome and the match start must agree with each other and
//! with what the client asked for.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use bota_core::rules;

use bota_proto::{
    ClientMsg, HeroId, LobbySlot, MatchInfo, PlayerId, Role, SeatKey, ServerMsg, SlotId, TickMode,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::redial::Redial;
use crate::session::SessionError;
#[cfg(not(target_arch = "wasm32"))]
use crate::socket::Link;
#[cfg(not(target_arch = "wasm32"))]
use crate::transport::DEFAULT_IO_TIMEOUT;
use crate::transport::Transport;

const MAX_MESSAGES_BEFORE_WELCOME: usize = 64;

/// A seat assigned by the server's welcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seated {
    /// The connection identity assigned by the server.
    pub player: PlayerId,
    pub slot: SlotId,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    pub mode: TickMode,
    /// The hero the seat picked, when it picked on joining; the match start
    /// must then give the seat that hero.
    pub hero: Option<HeroId>,
}

/// Checks the match start against the welcome and the seat's pick.
pub(crate) fn check_match(seated: &Seated, info: &MatchInfo) -> Result<(), SessionError> {
    if info.tick_rate != seated.tick_rate {
        return Err(SessionError::TickRateMismatch {
            start: info.tick_rate,
            welcome: seated.tick_rate,
        });
    }
    if info.mode != seated.mode {
        return Err(SessionError::ModeMismatch {
            start: info.mode,
            welcome: seated.mode,
        });
    }
    let pick = info
        .picks
        .iter()
        .find(|pick| pick.slot == seated.slot)
        .ok_or(SessionError::NoPickForSeat(seated.slot.0))?;
    if let Some(expected) = seated.hero
        && pick.hero != expected
    {
        return Err(SessionError::WrongSeatHero {
            slot: seated.slot.0,
            actual: pick.hero,
            expected,
        });
    }
    Ok(())
}

/// What the lobby phase has told: who the connection is, its seat, the
/// match's pace and who sits where.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lobby {
    pub me: Option<PlayerId>,
    /// The seat assigned, if the server assigned one.
    pub slot: Option<SlotId>,
    pub tick_rate: u16,
    pub mode: Option<TickMode>,
    pub slots: Vec<LobbySlot>,
}

impl Lobby {
    /// Takes in a lobby-phase message; others are not its business.
    pub fn observe(&mut self, msg: &ServerMsg) {
        match msg {
            ServerMsg::Welcome {
                player_id,
                slot,
                tick_rate,
                mode,
            } => {
                self.me = Some(*player_id);
                self.slot = *slot;
                self.tick_rate = *tick_rate;
                self.mode = Some(*mode);
            }
            ServerMsg::LobbyState { slots } => self.slots.clone_from(slots),
            _ => {}
        }
    }

    /// The seat the welcome assigned.
    pub fn seated(&self) -> Option<Seated> {
        Some(Seated {
            player: self.me?,
            slot: self.slot?,
            tick_rate: self.tick_rate,
            mode: self.mode?,
            hero: None,
        })
    }
}

/// Takes a seat: says hello (with the key a server with reserved seats asks
/// for), waits for the welcome, picks `hero` and says ready.
pub fn join(
    transport: &mut impl Transport,
    name: &str,
    hero: HeroId,
    key: Option<SeatKey>,
) -> std::io::Result<Seated> {
    let lobby = greet(transport, Role::Bot, name, key)?;
    let seated = lobby.seated().ok_or_else(|| {
        std::io::Error::other("server accepted the client without assigning a seat")
    })?;
    transport.send(&ClientMsg::PickHero { hero })?;
    transport.send(&ClientMsg::SetReady(true))?;
    Ok(Seated {
        hero: Some(hero),
        ..seated
    })
}

/// Says hello and waits for the welcome, answering pings meanwhile; what the
/// lobby phase has told so far comes back. A spectator takes no seat, so its
/// lobby has none.
pub fn greet(
    transport: &mut impl Transport,
    role: Role,
    name: &str,
    key: Option<SeatKey>,
) -> std::io::Result<Lobby> {
    validate_name(name)?;
    transport.send(&ClientMsg::hello(role, name, key))?;
    let mut lobby = Lobby::default();
    for _ in 0..MAX_MESSAGES_BEFORE_WELCOME {
        let message = transport
            .recv()?
            .ok_or_else(|| std::io::Error::other("server closed the connection before Welcome"))?;
        match message {
            ServerMsg::Welcome { .. } => {
                lobby.observe(&message);
                return Ok(lobby);
            }
            ServerMsg::WireMismatch { server_fingerprint } => {
                return Err(std::io::Error::other(SessionError::WireMismatch {
                    server: server_fingerprint,
                }));
            }
            ServerMsg::Refused { reason } => {
                return Err(std::io::Error::other(SessionError::Refused(reason)));
            }
            ServerMsg::Ping => transport.send(&ClientMsg::Pong)?,
            _ => {}
        }
    }
    Err(std::io::Error::other(format!(
        "server sent {MAX_MESSAGES_BEFORE_WELCOME} messages without Welcome"
    )))
}

fn validate_name(name: &str) -> std::io::Result<()> {
    if name.is_empty() {
        return Err(std::io::Error::other("client name must not be empty"));
    }
    if name.len() > rules::MAX_ANNOUNCED_NAME_BYTES {
        return Err(std::io::Error::other(format!(
            "client name is {} bytes; maximum is {}",
            name.len(),
            rules::MAX_ANNOUNCED_NAME_BYTES
        )));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
impl Link {
    /// Connects and takes a seat, as [`join`] does.
    pub fn join(
        address: &str,
        name: &str,
        hero: HeroId,
        key: Option<SeatKey>,
    ) -> std::io::Result<(Self, Seated)> {
        Self::join_with_timeout(address, name, hero, key, DEFAULT_IO_TIMEOUT)
    }

    /// Connects with one timeout for establishing and using the connection,
    /// and takes a seat.
    pub fn join_with_timeout(
        address: &str,
        name: &str,
        hero: HeroId,
        key: Option<SeatKey>,
        timeout: Duration,
    ) -> std::io::Result<(Self, Seated)> {
        Self::join_with_redial(address, name, hero, key, timeout, Redial::DEFAULT)
    }

    /// Connects with one timeout for establishing and using the connection,
    /// dialing again after a break by `redial`, and takes a seat.
    pub fn join_with_redial(
        address: &str,
        name: &str,
        hero: HeroId,
        key: Option<SeatKey>,
        timeout: Duration,
        redial: Redial,
    ) -> std::io::Result<(Self, Seated)> {
        validate_name(name)?;
        let mut link = Self::connect(address, timeout)?.with_redial(redial);
        let seated = join(&mut link, name, hero, key)?;
        Ok((link, seated))
    }
}
