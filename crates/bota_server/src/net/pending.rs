//! Reservations follow accepted sockets through handshakes and the simulation's inbox.

use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use super::{FrameStream, Place, Places, Split};

/// Accepted sockets awaiting adoption, including unfinished WebSocket handshakes.
pub const MAX_PENDING_CONNECTIONS: usize = 64;

#[derive(Clone, Default)]
pub(super) struct PendingAdmissions {
    held: Arc<AtomicUsize>,
    sources: Places,
}

impl PendingAdmissions {
    pub(super) fn take(&self, peer: Option<IpAddr>) -> Option<PendingAdmission> {
        let source = self
            .sources
            .take(peer.unwrap_or(Ipv4Addr::UNSPECIFIED.into()))?;
        self.held
            .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < MAX_PENDING_CONNECTIONS).then_some(held + 1)
            })
            .ok()?;
        Some(PendingAdmission {
            held: Arc::clone(&self.held),
            _source: source,
        })
    }
}

pub(super) struct PendingAdmission {
    held: Arc<AtomicUsize>,
    _source: Place,
}

impl PendingAdmission {
    pub(super) fn stream(self, stream: Box<dyn FrameStream>) -> Box<dyn FrameStream> {
        Box::new(PendingStream {
            stream,
            _admission: self,
        })
    }
}

impl Drop for PendingAdmission {
    fn drop(&mut self) {
        self.held.fetch_sub(1, Ordering::AcqRel);
    }
}

struct PendingStream {
    stream: Box<dyn FrameStream>,
    _admission: PendingAdmission,
}

impl FrameStream for PendingStream {
    fn peer(&self) -> Option<IpAddr> {
        self.stream.peer()
    }

    fn split(self: Box<Self>, timeout: Option<Duration>) -> io::Result<Split> {
        let Self { stream, _admission } = *self;
        stream.split(timeout)
    }
}
