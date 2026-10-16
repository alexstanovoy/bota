//! Hooks a client hangs on a session's fold, at fixed points inside it.

use core::convert::Infallible;

use bota_proto::{EventKind, WorldView};

use crate::session::ClientSession;

/// Where an event batch came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    /// The batch of a tick the client was sent in full.
    Live,
    /// The batch of a tick the server skipped for a slow client and told
    /// with the next snapshot.
    Skipped,
}

/// A client's own state, folded in step with the session's.
///
/// Every hook defaults to nothing. The session passes itself by reference
/// mid-fold, so a hook sees the state as it stands at its point: before the
/// snapshot's fold the session's current view is still the previous
/// snapshot.
pub trait Observer {
    type Error;

    /// The validated snapshot, before anything of it is used: the client may
    /// reshape it.
    fn prepare(&mut self, _view: &mut WorldView) -> Result<(), Self::Error> {
        Ok(())
    }

    /// The snapshot about to be folded; the session still holds the one before.
    fn before_snapshot(
        &mut self,
        _session: &ClientSession,
        _view: &WorldView,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// One tick's events, before they are folded.
    fn on_events(
        &mut self,
        _session: &ClientSession,
        _tick: u32,
        _events: &[EventKind],
        _source: EventSource,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// The snapshot is folded: the session's current view is the new one.
    fn after_snapshot(&mut self, _session: &ClientSession) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Observer for () {
    type Error = Infallible;
}
