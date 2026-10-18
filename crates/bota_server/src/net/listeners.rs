//! Where a server takes its connections from.

use std::net::TcpListener;
use std::sync::mpsc::Sender;
use std::thread;

use crate::net::{NetEvent, accept_loop};

/// The listeners of one server: TCP always, and with the `websocket` feature
/// a WebSocket listener on a port of its own beside it.
pub struct Listeners {
    tcp: TcpListener,
    #[cfg(feature = "websocket")]
    ws: Option<crate::net::WsListener>,
}

impl From<TcpListener> for Listeners {
    fn from(tcp: TcpListener) -> Listeners {
        Listeners {
            tcp,
            #[cfg(feature = "websocket")]
            ws: None,
        }
    }
}

impl Listeners {
    /// The TCP listener with a WebSocket listener beside it.
    #[cfg(feature = "websocket")]
    pub fn with_websocket(tcp: TcpListener, ws: crate::net::WsListener) -> Listeners {
        Listeners { tcp, ws: Some(ws) }
    }

    /// Starts accepting on every listener; each connection is handed to the
    /// server thread through `events`.
    pub fn start(self, events: &Sender<NetEvent>) -> std::io::Result<()> {
        #[cfg(feature = "websocket")]
        if let Some(ws) = self.ws {
            ws.start(events.clone())?;
        }
        let events = events.clone();
        thread::Builder::new().spawn(move || accept_loop(self.tcp, events))?;
        Ok(())
    }
}
