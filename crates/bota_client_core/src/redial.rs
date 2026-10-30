//! Dialing the server again after a connection broke, for the socket
//! transports: when to try, and what a fresh connection must say again.

use std::collections::VecDeque;
use std::thread;
use std::time::Duration;

use crate::socket::FrameSink;
use bota_proto::{ClientMsg, RejectReason, Role, SeatKey, ServerMsg, encode_frame_to_vec};

/// How a socket transport dials the server again after its connection broke
/// (an error, a timeout, a reset; never a clean close by the server).
///
/// A fresh connection says the latest `Hello`, pick, readiness and choice of
/// eyes again, so the server seats it as before, and the session picks the
/// match up again from the next snapshot (see [`Seat`](crate::Seat)). While
/// the server still holds the seat for the old connection (it has not yet
/// found it dead) it refuses the fresh one with
/// [`RejectReason::SeatHeld`]; that counts as a failed attempt, and the
/// attempts of one break are counted until a fresh connection is welcomed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redial {
    /// Attempts after one break before giving up; zero never dials again.
    pub attempts: u32,
    /// The wait before the first attempt. Each failed attempt doubles it.
    pub first_wait: Duration,
    /// The longest wait between two attempts.
    pub longest_wait: Duration,
}

impl Redial {
    /// Never dial again: a broken connection is the caller's error.
    pub const OFF: Redial = Redial {
        attempts: 0,
        first_wait: Duration::ZERO,
        longest_wait: Duration::ZERO,
    };

    /// About two and a half minutes of attempts, at most four seconds apart.
    pub const DEFAULT: Redial = Redial {
        attempts: 40,
        first_wait: Duration::from_millis(100),
        longest_wait: Duration::from_secs(4),
    };

    /// The wait before attempt `attempt`, counted from zero.
    fn wait(self, attempt: u32) -> Duration {
        self.first_wait
            .saturating_mul(1 << attempt.min(16))
            .min(self.longest_wait)
    }
}

impl Default for Redial {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Orders whose sending failed kept for the next connection, at most.
const MAX_UNSENT: usize = 64;

/// What a fresh connection must say again so the server seats it as before:
/// the latest `Hello`, pick, readiness and choice of eyes sent, then the
/// orders whose sending failed, which therefore never arrived and are safe
/// to send again.
///
/// A seat's `Hello` without a key goes out with a key drawn here, so a server
/// without reserved seats can give the seat back to the same client.
pub(crate) struct Rejoin {
    key: Option<SeatKey>,
    hello: Option<ClientMsg>,
    catalogue: Option<ClientMsg>,
    pick: Option<ClientMsg>,
    ready: Option<ClientMsg>,
    eyes: Option<ClientMsg>,
    compress: Option<ClientMsg>,
    unsent: VecDeque<ClientMsg>,
    /// The match is over, the server speaks another wire or refused the
    /// client for good: nothing to come back to.
    finished: bool,
    /// A fresh connection was greeted and not yet welcomed.
    rejoining: bool,
    /// Attempts spent since the connection broke, until a welcome.
    pub spent: u32,
}

impl Rejoin {
    pub fn new() -> Rejoin {
        let mut key = [0u8; 32];
        Rejoin {
            key: getrandom::fill(&mut key).ok().map(|()| SeatKey(key)),
            hello: None,
            catalogue: None,
            pick: None,
            ready: None,
            eyes: None,
            compress: None,
            unsent: VecDeque::new(),
            finished: false,
            rejoining: false,
            spent: 0,
        }
    }

    /// The message as it goes out, noted for a later connection.
    pub fn outgoing(&mut self, msg: &ClientMsg) -> ClientMsg {
        let mut msg = msg.clone();
        if let ClientMsg::Hello { role, key, .. } = &mut msg
            && matches!(role, Role::Player | Role::Bot)
            && key.is_none()
        {
            key.clone_from(&self.key);
        }
        let kept = match msg {
            ClientMsg::Hello { .. } => &mut self.hello,
            ClientMsg::Catalogue(_) => &mut self.catalogue,
            ClientMsg::PickHero { .. } => &mut self.pick,
            ClientMsg::SetReady(_) => &mut self.ready,
            ClientMsg::ViewAs { .. } => &mut self.eyes,
            ClientMsg::Compress(_) => &mut self.compress,
            // A line said into a broken connection is lost, as in Dota.
            ClientMsg::Order { .. }
            | ClientMsg::Ack { .. }
            | ClientMsg::Pong
            | ClientMsg::Probe { .. }
            | ClientMsg::LeaveGame
            | ClientMsg::Say(_) => {
                return msg;
            }
        };
        *kept = Some(msg.clone());
        msg
    }

    /// Keeps an order that failed to go out; the oldest give way.
    pub fn unsent(&mut self, msg: ClientMsg) {
        if matches!(msg, ClientMsg::Order { .. }) {
            if self.unsent.len() == MAX_UNSENT {
                self.unsent.pop_front();
            }
            self.unsent.push_back(msg);
        }
    }

    /// Notes what the server said, and whether it refused a fresh connection
    /// only because the old one still holds the seat: then the caller dials
    /// again. After the end of the match, a refused wire or any other
    /// refusal there is nothing to dial again for.
    pub fn heard(&mut self, msg: &ServerMsg) -> bool {
        match msg {
            ServerMsg::Welcome { .. } => {
                self.rejoining = false;
                self.spent = 0;
            }
            ServerMsg::Refused {
                reason: RejectReason::SeatHeld,
            } if self.rejoining => return true,
            ServerMsg::MatchOver { .. }
            | ServerMsg::WireMismatch { .. }
            | ServerMsg::Refused { .. } => self.finished = true,
            _ => {}
        }
        false
    }

    /// Whether a broken connection is worth dialing again: a `Hello` went
    /// out and the match has not ended.
    pub fn wanted(&self) -> bool {
        self.hello.is_some() && !self.finished
    }

    /// Says everything again on a fresh connection.
    pub fn greet(&mut self, stream: &mut dyn FrameSink) -> std::io::Result<()> {
        let greeting = [
            &self.hello,
            &self.catalogue,
            &self.compress,
            &self.pick,
            &self.ready,
            &self.eyes,
        ];
        let frames = greeting
            .into_iter()
            .flatten()
            .chain(&self.unsent)
            .map(|msg| encode_frame_to_vec(msg).map_err(std::io::Error::other))
            .collect::<std::io::Result<Vec<_>>>()?;
        stream.send_frames(&frames)?;
        self.unsent.clear();
        self.rejoining = true;
        Ok(())
    }
}

/// The server refused a fresh connection because the old one still holds
/// the seat.
pub(crate) fn seat_held() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "the server still holds the seat for the broken connection",
    )
}

/// Dials again by `policy`, `spent` attempts of it already used since the
/// break, handing each fresh stream to `adopt` (which greets the server on
/// it) until one is adopted; the error says why the connection broke and why
/// it could not be made again.
pub(crate) fn redial<S>(
    policy: Redial,
    spent: &mut u32,
    cause: std::io::Error,
    mut dial: impl FnMut() -> std::io::Result<S>,
    mut adopt: impl FnMut(S) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut last = None;
    while *spent < policy.attempts {
        thread::sleep(policy.wait(*spent));
        *spent += 1;
        match dial().and_then(&mut adopt) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    Err(match last {
        None => cause,
        Some(last) => std::io::Error::new(
            cause.kind(),
            format!(
                "{cause}; gave up after {} attempts to reconnect: {last}",
                policy.attempts
            ),
        ),
    })
}
