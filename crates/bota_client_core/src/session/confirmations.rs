//! Correlating snapshot settlements with this connection's own requests.

use bota_proto::{OrderSettlement, PlayerId};
use std::collections::VecDeque;

/// At least a full intentional-delay window at the wire's per-tick budget.
const MAX_REQUESTS: usize =
    (bota_proto::MAX_ORDER_DELAY_TICKS as usize + 1) * bota_proto::MAX_ORDERS_PER_TICK;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Confirmations {
    connection: Option<PlayerId>,
    sent: VecDeque<(u32, u32)>,
    through: u32,
}

impl Confirmations {
    pub fn bind(&mut self, connection: PlayerId) -> bool {
        if self.connection == Some(connection) {
            return false;
        }
        self.connection = Some(connection);
        self.sent.clear();
        self.through = 0;
        true
    }

    pub fn record(&mut self, sequence: u32, seen: u32) {
        if self.sent.len() == MAX_REQUESTS {
            self.sent.pop_front();
        }
        self.sent.push_back((sequence, seen));
    }

    pub fn snapshot(&mut self, settlement: Option<OrderSettlement>, tick: u32) {
        let Some(settled) = settlement else { return };
        if settled.connection != self.connection || settled.tick > tick {
            return;
        }
        let Some(seen) = settled.seen else { return };
        if !self.sent.contains(&(settled.sequence, seen)) {
            return;
        }
        self.through = self.through.max(settled.sequence);
        while self
            .sent
            .front()
            .is_some_and(|(sequence, _)| *sequence <= self.through)
        {
            self.sent.pop_front();
        }
    }

    pub fn through(&self) -> u32 {
        self.through
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_connections_identical_sequence_and_seen_cannot_settle_a_new_request() {
        let mut confirmations = Confirmations::default();
        confirmations.bind(PlayerId(1));
        confirmations.record(1, 100);
        confirmations.bind(PlayerId(2));
        confirmations.record(1, 100);
        let mut settled = OrderSettlement {
            connection: Some(PlayerId(1)),
            sequence: 1,
            seen: Some(100),
            tick: 101,
        };
        confirmations.snapshot(Some(settled), 101);
        assert_eq!(confirmations.through(), 0);
        settled.connection = Some(PlayerId(2));
        settled.seen = Some(99);
        confirmations.snapshot(Some(settled), 101);
        assert_eq!(
            confirmations.through(),
            0,
            "the exact locally issued pair must match"
        );
        settled.seen = Some(100);
        confirmations.snapshot(Some(settled), 100);
        assert_eq!(
            confirmations.through(),
            0,
            "a future settlement is not in this snapshot"
        );
        confirmations.snapshot(Some(settled), 101);
        assert_eq!(confirmations.through(), 1);
    }
}
