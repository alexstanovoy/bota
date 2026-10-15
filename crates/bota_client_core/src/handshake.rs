//! The server's welcome and the match start must agree with each other and
//! with what the client asked for.

use bota_proto::{HeroId, MatchInfo, PlayerId, SlotId, Team, TickMode};

use crate::session::SessionError;

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
