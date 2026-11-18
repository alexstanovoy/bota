//! Checking a service before signing in to it, as a bota client: the `Hello` names bota's wire.

use std::time::Duration;

use radovid_client::{ServiceFault, ServiceFound};

use crate::game::fingerprint;
use crate::tls::Tls;

type Backend = radovid_client::ServiceCheck;

/// A check of a service under way: the protocol's first exchange (`Hello`, `Welcome`) over one
/// connection, on a thread natively and over the browser's WebSocket on wasm32, never blocking
/// its caller. Dropping it cancels the check.
pub struct ServiceCheck(Backend);

impl ServiceCheck {
    /// Starts checking `url` (`wss://host[:port][/path]`), giving up after `patience`.
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    pub fn start(url: &str, tls: &Tls, patience: Duration) -> ServiceCheck {
        ServiceCheck(Backend::start(url, &tls.control, fingerprint(), patience))
    }

    /// As on the desktop, over the browser's WebSocket, which brings its own trust.
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    pub fn start(url: &str, _tls: &Tls, patience: Duration) -> ServiceCheck {
        ServiceCheck(Backend::start(url, fingerprint(), patience))
    }

    /// The outcome, once, when it is there; nothing while the check runs.
    pub fn poll(&mut self) -> Option<Result<ServiceFound, ServiceFault>> {
        self.0.poll()
    }
}
