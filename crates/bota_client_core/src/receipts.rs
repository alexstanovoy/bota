//! How long the server takes to say it received an order, timed where the bytes leave and
//! arrive (see [`PingMeter`](crate::PingMeter)).
//!
//! An order is stamped as sent when the socket takes it, and its
//! [`ServerMsg::OrderReceived`](bota_proto::ServerMsg::OrderReceived) as it arrives. The
//! difference is a round trip plus the server's reading, never a tick: what a player waits for
//! before the game acknowledges a click.

use std::collections::VecDeque;
use std::time::Duration;

use crate::ping::smooth;

/// Orders awaiting their receipt at most; the oldest is given up on past them.
const MAX_PENDING: usize = 64;

/// What the server said of the last order it received.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// From the socket taking the order to the receipt arriving.
    pub latency: Duration,
    /// The earliest tick the order could take effect on.
    pub tick: u32,
}

/// The orders sent and not yet received, and the latency of those that were.
#[derive(Clone, Debug, Default)]
pub struct OrderReceipts {
    pending: VecDeque<(u32, Duration)>,
    last: Option<Receipt>,
    smoothed: Option<Duration>,
}

impl OrderReceipts {
    /// Notes order `seq` as sent at `now`.
    pub fn sent(&mut self, seq: u32, now: Duration) {
        if self.pending.len() == MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back((seq, now));
    }

    /// Takes the receipt of order `seq` as arrived at `now`; one for an order given up on is
    /// ignored.
    pub fn received(&mut self, seq: u32, tick: u32, now: Duration) {
        let Some(at) = self.pending.iter().position(|(sent, _)| *sent == seq) else {
            return;
        };
        let Some((_, sent)) = self.pending.remove(at) else {
            return;
        };
        let latency = now.saturating_sub(sent);
        self.last = Some(Receipt { latency, tick });
        self.smoothed = Some(smooth(self.smoothed, latency));
    }

    /// Orders sent and not yet received.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// The latest receipt.
    pub const fn last(&self) -> Option<Receipt> {
        self.last
    }

    /// The smoothed latency of receipts, once one has come.
    pub const fn smoothed(&self) -> Option<Duration> {
        self.smoothed
    }
}
