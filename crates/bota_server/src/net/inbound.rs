//! Only data messages consume this budget. Terminal events always retain their control path.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bota_proto::ClientMsg;

/// Reserve the worst-case wire payload per message; decoded object overhead is count-bounded too.
pub const MAX_QUEUED_CLIENT_MESSAGES: usize = (1 << 20) / bota_proto::MAX_CLIENT_PAYLOAD_LEN;
pub const MAX_QUEUED_MESSAGES_PER_PEER: usize = 64;
const _: () = assert!(MAX_QUEUED_CLIENT_MESSAGES >= MAX_QUEUED_MESSAGES_PER_PEER);

#[derive(Clone, Default)]
pub struct InboundBudget(Arc<AtomicUsize>);

impl InboundBudget {
    pub(super) fn peer(&self) -> Arc<PeerBudget> {
        Arc::new(PeerBudget {
            global: Arc::clone(&self.0),
            queued: AtomicUsize::new(0),
        })
    }
}

pub(super) struct PeerBudget {
    global: Arc<AtomicUsize>,
    queued: AtomicUsize,
}

impl PeerBudget {
    pub(super) fn queue(self: &Arc<Self>, message: ClientMsg) -> Option<QueuedClientMessage> {
        if !reserve(&self.global, MAX_QUEUED_CLIENT_MESSAGES) {
            return None;
        }
        if !reserve(&self.queued, MAX_QUEUED_MESSAGES_PER_PEER) {
            self.global.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(QueuedClientMessage {
            message,
            _reservation: Reservation(Arc::clone(self)),
        })
    }
}

fn reserve(count: &AtomicUsize, limit: usize) -> bool {
    count
        .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
            (held < limit).then_some(held + 1)
        })
        .is_ok()
}

/// Its reservation follows the event until the simulation consumes or drops it.
pub struct QueuedClientMessage {
    message: ClientMsg,
    _reservation: Reservation,
}

impl QueuedClientMessage {
    #[must_use]
    pub fn into_message(self) -> ClientMsg {
        self.message
    }
}

struct Reservation(Arc<PeerBudget>);

impl Drop for Reservation {
    fn drop(&mut self) {
        assert!(self.0.queued.fetch_sub(1, Ordering::AcqRel) > 0);
        assert!(self.0.global.fetch_sub(1, Ordering::AcqRel) > 0);
    }
}
