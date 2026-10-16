//! One connection's way into a match: the lobby, the start, then the session.

use bota_proto::ServerMsg;

use crate::handshake::Seated;
use crate::match_static::MatchStatic;
use crate::observer::Observer;
use crate::session::{ClientSession, IngestError, Ingested, SessionError, SessionOptions};

/// Messages a server may send before the match starts. The lobby tells little
/// (the welcome, seats, picks, departures); a stream that never starts is
/// broken.
pub const MAX_MESSAGES_BEFORE_MATCH_START: usize = 4096;

/// The stream of one connection from the lobby to the end of the match.
///
/// Before `MatchStart` it passes lobby messages through and refuses anything
/// that belongs to a match; `MatchStart` builds the session (checked against
/// the welcome and the pick for a seat); after it every message goes to
/// [`ClientSession::ingest`].
#[derive(Clone, Debug)]
pub struct Seat {
    seated: Option<Seated>,
    options: SessionOptions,
    session: Option<ClientSession>,
    before_start: usize,
}

impl Seat {
    /// The stream of the seat a welcome assigned.
    pub const fn new(seated: Seated, options: SessionOptions) -> Seat {
        Seat {
            seated: Some(seated),
            options,
            session: None,
            before_start: 0,
        }
    }

    /// The stream of a connection with no seat: a spectator's, or a replay's.
    pub const fn watching(options: SessionOptions) -> Seat {
        Seat {
            seated: None,
            options,
            session: None,
            before_start: 0,
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
        if let Some(session) = &mut self.session {
            return session.ingest(msg, obs);
        }
        self.before_start += 1;
        if self.before_start > MAX_MESSAGES_BEFORE_MATCH_START {
            return Err(SessionError::Stream("server sent too many messages before MatchStart").into());
        }
        let refused = match msg {
            ServerMsg::MatchStart { info } => return Ok(self.start(&info)?),
            ServerMsg::Snapshot { .. } => "server sent Snapshot before MatchStart",
            ServerMsg::Events { .. } => "server sent Events before MatchStart",
            ServerMsg::OrderRejected { .. } => "server rejected an order before MatchStart",
            ServerMsg::MatchOver { .. } => "server sent MatchOver before MatchStart",
            other @ (ServerMsg::Welcome { .. }
            | ServerMsg::LobbyState { .. }
            | ServerMsg::Orders { .. }
            | ServerMsg::ParticipantLeft { .. }) => return Ok(Ingested::Passed(Box::new(other))),
        };
        Err(SessionError::Stream(refused).into())
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

    /// The session, for a client that keeps it in a state of its own from
    /// the start on.
    pub fn into_session(self) -> Option<ClientSession> {
        self.session
    }
}
