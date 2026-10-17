//! How a client talks to a server: the [`Transport`] a session reads from and
//! writes to. The socket implementations live in [`socket`](crate::socket) and are
//! left out when compiling for the browser.

use std::time::Duration;

use bota_proto::{ClientMsg, ServerMsg};

/// How long a socket transport waits on the server before it takes the
/// connection for broken, by default. The server pings every second, so five
/// seconds of silence is a dead link, not a quiet lobby.
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// A framed connection to a match server.
pub trait Transport {
    /// Reads the next server message, or no message after a clean close.
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>>;

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()>;

    /// Time spent waiting on the socket since the last call, for the caller's
    /// own timing; none for a transport that does not measure it.
    fn take_receive_wait(&mut self) -> Option<Duration> {
        None
    }
}
