//! The server's welcome and the match start must agree with each other and
//! with what the client asked for.

use std::time::Duration;

use bota_proto::{
    ClientMsg, HeroId, LobbySlot, MatchInfo, PlayerId, Role, ServerMsg, SlotId, Team, TickMode,
};

use crate::session::SessionError;
use crate::transport::{DEFAULT_IO_TIMEOUT, Link, Transport};

const MAX_MESSAGES_BEFORE_WELCOME: usize = 64;
const MAX_NAME_LEN: usize = 64;

/// A seat assigned by the server's welcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seated {
    /// The connection identity assigned by the server.
    pub player: PlayerId,
    /// The seat assigned.
    pub slot: SlotId,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    pub mode: TickMode,
}

/// Checks the match start against the welcome and the seat's pick, and
/// returns the seat's team.
pub fn check_match(
    seated: &Seated,
    info: &MatchInfo,
    hero: Option<HeroId>,
) -> Result<Team, SessionError> {
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
    if let Some(expected) = hero
        && pick.hero != expected
    {
        return Err(SessionError::WrongSeatHero {
            slot: seated.slot.0,
            actual: pick.hero,
            expected,
        });
    }
    Ok(pick.team)
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
        })
    }
}

/// Takes a seat: says hello, waits for the welcome, picks `hero` if there is
/// one to pick, and says ready.
pub fn join(
    transport: &mut impl Transport,
    name: &str,
    hero: Option<HeroId>,
) -> std::io::Result<Seated> {
    validate_name(name)?;
    transport.send(&ClientMsg::Hello {
        role: Role::Bot,
        name: name.to_owned(),
    })?;
    let seated = wait_for_welcome(transport)?;
    if let Some(hero) = hero {
        transport.send(&ClientMsg::PickHero { hero })?;
    }
    transport.send(&ClientMsg::SetReady(true))?;
    Ok(seated)
}

fn wait_for_welcome(transport: &mut impl Transport) -> std::io::Result<Seated> {
    let mut lobby = Lobby::default();
    for _ in 0..MAX_MESSAGES_BEFORE_WELCOME {
        let message = transport
            .recv()?
            .ok_or_else(|| std::io::Error::other("server closed the connection before Welcome"))?;
        if matches!(message, ServerMsg::Welcome { .. }) {
            lobby.observe(&message);
            return lobby.seated().ok_or_else(|| {
                std::io::Error::other("server accepted the client without assigning a seat")
            });
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
    if name.len() > MAX_NAME_LEN {
        return Err(std::io::Error::other(format!(
            "client name is {} bytes; maximum is {MAX_NAME_LEN}",
            name.len()
        )));
    }
    Ok(())
}

impl Link {
    /// Connects and takes a seat, as [`join`] does.
    pub fn join(
        address: &str,
        name: &str,
        hero: Option<HeroId>,
    ) -> std::io::Result<(Self, Seated)> {
        Self::join_with_timeout(address, name, hero, DEFAULT_IO_TIMEOUT)
    }

    /// Connects with one timeout for establishing and using the connection,
    /// and takes a seat.
    pub fn join_with_timeout(
        address: &str,
        name: &str,
        hero: Option<HeroId>,
        timeout: Duration,
    ) -> std::io::Result<(Self, Seated)> {
        validate_name(name)?;
        let mut link = Self::connect(address, timeout)?;
        let seated = join(&mut link, name, hero)?;
        Ok((link, seated))
    }
}
