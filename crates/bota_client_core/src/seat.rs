//! One connection's way into a match: the lobby, the start, then the session.

use bota_proto::{ServerMsg, SlotId, Team};

use crate::chat::{Hearer, heard_fits};
use crate::handshake::Seated;
use crate::match_static::MatchStatic;
use crate::observer::Observer;
use crate::session::{ClientSession, IngestError, Ingested, SessionError, SessionOptions};

/// Messages a server may send before the match starts. The lobby tells little
/// (the welcome, seats, picks, departures); a stream that never starts is
/// broken.
pub const MAX_MESSAGES_BEFORE_MATCH_START: usize = 4096;

/// The stream of a seat (or a spectator) from the lobby to the end of the
/// match, over one connection or several.
///
/// Before `MatchStart` it passes lobby messages through and refuses anything
/// that belongs to a match; `MatchStart` builds the session (checked against
/// the welcome and the pick for a seat); after it every message goes to
/// [`ClientSession::ingest`].
///
/// A `Welcome` after the start means the client connected again: it must
/// name the same seat, the next message must be the same match's start, and
/// that start resumes the session ([`Ingested::Resumed`]).
#[derive(Clone, Debug)]
pub struct Seat {
    seated: Option<Seated>,
    options: SessionOptions,
    session: Option<ClientSession>,
    before_start: usize,
    /// A fresh connection was welcomed and its match start is due.
    resuming: bool,
    /// The lobby's seats and their sides, as last told, for checking chat.
    lobby_seats: Vec<(SlotId, Team)>,
}

impl Seat {
    /// The stream of the seat a welcome assigned.
    pub const fn new(seated: Seated, options: SessionOptions) -> Seat {
        Seat {
            seated: Some(seated),
            options,
            session: None,
            before_start: 0,
            resuming: false,
            lobby_seats: Vec::new(),
        }
    }

    /// The stream of a connection with no seat: a spectator's, or a replay's.
    pub const fn watching(options: SessionOptions) -> Seat {
        Seat {
            seated: None,
            options,
            session: None,
            before_start: 0,
            resuming: false,
            lobby_seats: Vec::new(),
        }
    }

    /// Folds one server message in: [`Ingested::Started`] for the match
    /// start, [`Ingested::Passed`] for the lobby before it, and whatever the
    /// session makes of it after.
    pub fn ingest<O: Observer>(
        &mut self,
        msg: ServerMsg,
        obs: &mut O,
    ) -> Result<Ingested, IngestError<O::Error>> {
        if self.session.is_some() {
            return self.ingest_started(msg, obs);
        }
        match msg {
            // Chat flows however long the lobby lasts.
            ServerMsg::Ping | ServerMsg::SayRefused { .. } => {
                return Ok(Ingested::Passed(Box::new(msg)));
            }
            ServerMsg::Heard(heard) => return Ok(self.lobby_heard(heard)),
            _ => {}
        }
        self.before_start += 1;
        if self.before_start > MAX_MESSAGES_BEFORE_MATCH_START {
            return Err(
                SessionError::Stream("server sent too many messages before MatchStart").into(),
            );
        }
        let refused = match msg {
            ServerMsg::MatchStart { info } => return Ok(self.start(&info)?),
            ServerMsg::Snapshot { .. } => "server sent Snapshot before MatchStart",
            ServerMsg::Events { .. } => "server sent Events before MatchStart",
            ServerMsg::OrderRejected { .. } => "server rejected an order before MatchStart",
            ServerMsg::MatchOver { .. } => "server sent MatchOver before MatchStart",
            ServerMsg::WireMismatch { server_fingerprint } => {
                return Err(SessionError::WireMismatch {
                    server: server_fingerprint,
                }
                .into());
            }
            ServerMsg::Refused { reason } => return Err(SessionError::Refused(reason).into()),
            ServerMsg::Welcome {
                player_id, slot, ..
            } => {
                // A welcome on a fresh connection before the start: the seat
                // it names is the one the match will be checked against.
                if let (Some(seated), Some(slot)) = (&mut self.seated, slot) {
                    seated.player = player_id;
                    seated.slot = slot;
                }
                return Ok(Ingested::Passed(Box::new(msg)));
            }
            ServerMsg::LobbyState { ref slots, .. } => {
                crate::session::validate_lobby(slots)?;
                self.lobby_seats = slots.iter().map(|seat| (seat.slot, seat.team)).collect();
                return Ok(Ingested::Passed(Box::new(msg)));
            }
            ServerMsg::Compressed(_) => {
                "a compressed part reached the seat: the transport did not unpack it"
            }
            ServerMsg::Heard(_) | ServerMsg::SayRefused { .. } => {
                unreachable!("chat was passed on above")
            }
            other @ (ServerMsg::Orders { .. }
            | ServerMsg::ParticipantLeft { .. }
            | ServerMsg::Ping) => return Ok(Ingested::Passed(Box::new(other))),
        };
        Err(SessionError::Stream(refused).into())
    }

    /// A message after the start: a fresh connection's welcome and match
    /// start resume the session; anything else is the session's.
    fn ingest_started<O: Observer>(
        &mut self,
        msg: ServerMsg,
        obs: &mut O,
    ) -> Result<Ingested, IngestError<O::Error>> {
        let session = self.session.as_mut().expect("the match has started");
        match msg {
            ServerMsg::Welcome { slot, .. } => {
                if slot != session.slot() {
                    return Err(SessionError::Stream(
                        "server welcomed a reconnect to another seat",
                    )
                    .into());
                }
                self.resuming = true;
                Ok(Ingested::Passed(Box::new(msg)))
            }
            ServerMsg::MatchStart { info } if self.resuming => {
                if *MatchStatic::new(&info)? != **session.statics() {
                    return Err(SessionError::Stream("server resumed another match").into());
                }
                self.resuming = false;
                let resend = session.resume()?;
                Ok(Ingested::Resumed { resend })
            }
            other => session.ingest(other, obs),
        }
    }

    /// Passes a lobby line on when this connection could have been told it.
    fn lobby_heard(&self, heard: bota_proto::Heard) -> Ingested {
        let mine = self.seated.as_ref().map(|seated| seated.slot);
        let side = self
            .lobby_seats
            .iter()
            .find(|(slot, _)| Some(*slot) == mine)
            .map(|(_, team)| *team);
        let hearer = Hearer {
            seated: mine.is_some(),
            side,
            seats: &self.lobby_seats,
            tick: None,
        };
        match heard_fits(&heard, hearer) {
            true => Ingested::Passed(Box::new(ServerMsg::Heard(heard))),
            false => Ingested::Unheard,
        }
    }

    fn start(&mut self, info: &bota_proto::MatchInfo) -> Result<Ingested, SessionError> {
        let session = match &self.seated {
            Some(seated) => ClientSession::start(seated, info, self.options)?,
            None => ClientSession::spectator(MatchStatic::new(info)?, self.options),
        };
        let started = Ingested::Started {
            slot: session.slot(),
            team: session.team(),
        };
        self.session = Some(session);
        Ok(started)
    }

    /// The seat the welcome assigned; none for a spectator or a replay.
    pub const fn seated(&self) -> Option<&Seated> {
        self.seated.as_ref()
    }

    /// The session, once the match has started.
    pub const fn session(&self) -> Option<&ClientSession> {
        self.session.as_ref()
    }

    /// The session, once the match has started, for issuing orders.
    pub const fn session_mut(&mut self) -> Option<&mut ClientSession> {
        self.session.as_mut()
    }
}
