//! A bot's whole match in one call: [`play`] drives a seat from the lobby to
//! the end and asks the bot for its orders once per tick.

use core::convert::Infallible;
use core::fmt;

use bota_proto::{ClientMsg, ServerMsg, SlotId, Team};

use crate::handshake::Seated;
use crate::observer::Observer;
use crate::orders::IssuedOrder;
use crate::seat::Seat;
use crate::session::{ClientSession, IngestError, Ingested, SessionError, SessionOptions};
use crate::transport::Transport;

/// How a match ended for the seat that played it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub slot: SlotId,
    pub team: Team,
    /// The side that won; [`Team::Neutral`] for a draw.
    pub winner: Team,
    /// The last tick the seat was shown.
    pub ticks: u32,
    /// Orders the server refused.
    pub rejections: u32,
    /// Orders for the hero that were not sent because the hero was dead on
    /// the tick they were decided: the server refuses every one of them.
    /// Courier orders go out whatever the hero's state.
    pub unsent_while_dead: u32,
}

/// Why a match could not be played to its end.
#[derive(Debug)]
pub enum PlayError<E = Infallible> {
    /// The connection failed.
    Io(std::io::Error),
    /// The server's stream broke the rules of a match.
    Session(SessionError),
    /// The observer refused what it was shown.
    Observer(E),
    /// The server closed the connection before the match ended.
    Closed,
}

impl<E: fmt::Display> fmt::Display for PlayError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "connection to the server failed: {error}"),
            Self::Session(error) => write!(
                f,
                "the server's stream broke the protocol ({error}); build the client and the \
                 server from matching commits"
            ),
            Self::Observer(error) => write!(f, "{error}"),
            Self::Closed => f.write_str("the server closed the connection before the match ended"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for PlayError<E> {}

impl<E> From<std::io::Error> for PlayError<E> {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl<E> From<SessionError> for PlayError<E> {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl<E> From<IngestError<E>> for PlayError<E> {
    fn from(error: IngestError<E>) -> Self {
        match error {
            IngestError::Session(error) => Self::Session(error),
            IngestError::Observer(error) => Self::Observer(error),
        }
    }
}

/// Plays a joined seat to the end of its match.
///
/// Every tick, once its events are in, `decide` is shown the session and
/// returns the orders it wants: none, one (an `Option`) or several. An order
/// that repeats the one standing is not sent again, nor is an order for the
/// hero while it is dead (counted in [`Outcome::unsent_while_dead`]); the
/// rest go out numbered, and the tick is acknowledged. The observer sees the
/// stream as [`ClientSession::ingest`] folds it.
///
/// A transport that dials again after a break (a [`Link`](crate::Link) does)
/// hands on the fresh connection's welcome and match start; the seat picks
/// the match up from them and the standing orders go out again. A ping that
/// reaches the loop is answered.
pub fn play<T, O, D, I>(
    transport: &mut T,
    seated: Seated,
    observer: &mut O,
    mut decide: D,
) -> Result<Outcome, PlayError<O::Error>>
where
    T: Transport,
    O: Observer,
    D: FnMut(&ClientSession) -> I,
    I: IntoIterator<Item = IssuedOrder>,
{
    let mut seat = Seat::new(seated, SessionOptions::default());
    let mut rejections = 0u32;
    let mut unsent_while_dead = 0u32;
    // Each message makes progress: the session bounds the ones without a
    // snapshot, and snapshot ticks only rise.
    loop {
        let msg = transport.recv()?.ok_or(PlayError::Closed)?;
        match seat.ingest(msg, observer)? {
            Ingested::TickComplete { .. } => {
                let session = seat.session_mut().expect("a tick comes after the start");
                let unsent = take_turn(transport, session, &mut decide)?;
                unsent_while_dead = unsent_while_dead.saturating_add(unsent);
            }
            Ingested::Rejected { .. } => rejections = rejections.saturating_add(1),
            Ingested::Resumed { resend } => {
                for order in resend {
                    transport.send(&order.message())?;
                }
            }
            Ingested::Passed(msg) if matches!(*msg, ServerMsg::Ping) => {
                transport.send(&ClientMsg::Pong)?;
            }
            Ingested::Over { winner, .. } => {
                let session = seat.session().expect("the end comes after the start");
                return Ok(Outcome {
                    slot: session.slot().unwrap_or(seated.slot),
                    team: session.team(),
                    winner,
                    ticks: session.current().map_or(0, |view| view.tick),
                    rejections,
                    unsent_while_dead,
                });
            }
            _ => {}
        }
    }
}

/// Sends what the bot decided on the tick, then the acknowledgement, and
/// says how many hero orders it held back for a dead hero.
fn take_turn<E, I: IntoIterator<Item = IssuedOrder>>(
    transport: &mut impl Transport,
    session: &mut ClientSession,
    decide: &mut impl FnMut(&ClientSession) -> I,
) -> Result<u32, PlayError<E>> {
    let hero_down = session.own_hero().is_none();
    let mut unsent = 0u32;
    for order in decide(session) {
        if hero_down && order.unit.is_none() {
            unsent = unsent.saturating_add(1);
            continue;
        }
        if session.ledger().repeats(order) {
            continue;
        }
        let outgoing = session.issue(order)?;
        transport.send(&outgoing.message())?;
    }
    if let Some(tick) = session.end_turn() {
        transport.send(&ClientMsg::Ack { tick })?;
    }
    Ok(unsent)
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::socket::Link {
    /// Connects, takes a seat with `hero` and plays the match to its end, as
    /// [`play`] does.
    pub fn play<I: IntoIterator<Item = IssuedOrder>>(
        address: &str,
        name: &str,
        hero: bota_proto::HeroId,
        decide: impl FnMut(&ClientSession) -> I,
    ) -> Result<Outcome, PlayError> {
        let (mut link, seated) = Self::join(address, name, hero, None)?;
        play(&mut link, seated, &mut (), decide)
    }
}
