//! The transports a test reaches a server over: TCP, and with the
//! `websocket` feature a WebSocket, plain or over TLS with the certificate in
//! `tests/tls` (self-signed for localhost; its key guards nothing).
//!
//! A test's clients speak TCP. For a WebSocket, [`dial`] puts an adapter in
//! front of the server: it takes each client connection and carries every
//! frame as one binary message, a clean close on as a clean close and a
//! broken connection on as a reset, both ways. Whatever stands between the
//! adapter and the server (a bad line) works on the WebSocket's bytes. A test
//! that drives the client library asks for an [`endpoint`] instead and needs
//! no adapter.
#![allow(dead_code)] // each test binary uses a part of this shared module

/// Runs each named test, a function of a [`Wire`] in the calling module,
/// over every wire this build speaks.
macro_rules! over_every_wire {
    ($($test:ident),* $(,)?) => {
        mod over_tcp {
            $(#[test]
            fn $test() {
                super::$test(super::Wire::Tcp);
            })*
        }
        #[cfg(feature = "websocket")]
        mod over_ws {
            $(#[test]
            fn $test() {
                super::$test(super::Wire::Ws);
            })*
        }
        #[cfg(feature = "websocket")]
        mod over_wss {
            $(#[test]
            fn $test() {
                super::$test(super::Wire::Wss);
            })*
        }
    };
}

/// The test certificate and its key.
#[cfg(feature = "websocket")]
pub const CERT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/tls/cert.pem");
#[cfg(feature = "websocket")]
pub const KEY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/tls/key.pem");

/// How a client reaches the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Tcp,
    #[cfg(feature = "websocket")]
    Ws,
    #[cfg(feature = "websocket")]
    Wss,
}

/// Where a client reaches `target`, which speaks `wire`, by the client
/// library's own transport.
pub fn endpoint(wire: Wire, target: &str) -> bota_client_core::Endpoint {
    let endpoint = |text: &str| bota_client_core::Endpoint::parse(text).expect("an endpoint");
    match wire {
        Wire::Tcp => endpoint(target),
        #[cfg(feature = "websocket")]
        Wire::Ws => endpoint(&format!("ws://{target}")),
        #[cfg(feature = "websocket")]
        Wire::Wss => {
            let port = target.rsplit(':').next().expect("a port");
            let pem = std::fs::read_to_string(CERT).expect("the test certificate");
            let tls = bota_client_core::TlsConfig::trust_pem(&pem).expect("a trusted cert");
            endpoint(&format!("wss://localhost:{port}")).with_tls(tls)
        }
    }
}

/// The address a TCP client dials to reach `target`, which speaks `wire`:
/// the server's own port, or a proxy in front of it.
pub fn dial(wire: Wire, target: &str) -> String {
    match wire {
        Wire::Tcp => target.to_owned(),
        #[cfg(feature = "websocket")]
        Wire::Ws => adapter::start(target, None),
        #[cfg(feature = "websocket")]
        Wire::Wss => adapter::start(target, Some(adapter::trusting_test_certificate())),
    }
}

#[cfg(feature = "websocket")]
mod adapter {
    use std::net::TcpStream as StdTcpStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use bota_proto::FrameReader;
    use futures_util::stream::{SplitSink, SplitStream};
    use futures_util::{SinkExt, StreamExt};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, ServerName};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsConnector;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::Message;

    /// A TLS client that trusts the test certificate alone, TLS 1.3 only.
    pub fn trusting_test_certificate() -> TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        let cert = CertificateDer::from_pem_file(super::CERT).expect("the test certificate");
        roots.add(cert).expect("a usable root");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3")
            .with_root_certificates(roots)
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    }

    /// Listens on a free local port and carries every connection to
    /// `target` over a WebSocket, through TLS when given it.
    pub fn start(target: &str, tls: Option<TlsConnector>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let addr = listener.local_addr().expect("bound").to_string();
        listener.set_nonblocking(true).expect("nonblocking");
        let target = target.to_owned();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            runtime.block_on(async move {
                let listener = TcpListener::from_std(listener).expect("a listener");
                while let Ok((client, _)) = listener.accept().await {
                    tokio::spawn(carry(client, target.clone(), tls.clone()));
                }
            });
        });
        addr
    }

    /// Both connections of one carried client.
    struct Ends {
        client: StdTcpStream,
        upstream: StdTcpStream,
        aborted: AtomicBool,
    }

    impl Ends {
        /// Resets both connections: nothing more is read, no FIN goes out,
        /// and the sockets reset their peers once let go of.
        fn abort(&self) {
            self.aborted.store(true, Ordering::SeqCst);
            for socket in [&self.client, &self.upstream] {
                let _ = socket2::SockRef::from(socket).set_linger(Some(Duration::ZERO));
                let _ = socket.shutdown(std::net::Shutdown::Read);
            }
        }

        fn aborted(&self) -> bool {
            self.aborted.load(Ordering::SeqCst)
        }
    }

    fn dup(stream: &TcpStream) -> StdTcpStream {
        StdTcpStream::from(socket2::SockRef::from(stream).try_clone().expect("a dup"))
    }

    async fn carry(client: TcpStream, target: String, tls: Option<TlsConnector>) {
        let Ok(upstream) = TcpStream::connect(&target).await else {
            let _ = socket2::SockRef::from(&client).set_linger(Some(Duration::ZERO));
            return;
        };
        for stream in [&client, &upstream] {
            let _ = stream.set_nodelay(true);
        }
        let ends = Arc::new(Ends {
            client: dup(&client),
            upstream: dup(&upstream),
            aborted: AtomicBool::new(false),
        });
        let url = "ws://localhost/";
        match tls {
            None => match tokio_tungstenite::client_async(url, upstream).await {
                Ok((ws, _)) => pump(client, ws, ends).await,
                Err(_) => ends.abort(),
            },
            Some(tls) => {
                let name = ServerName::try_from("localhost").expect("a name");
                let Ok(upstream) = tls.connect(name, upstream).await else {
                    return ends.abort();
                };
                match tokio_tungstenite::client_async(url, upstream).await {
                    Ok((ws, _)) => pump(client, ws, ends).await,
                    Err(_) => ends.abort(),
                }
            }
        }
    }

    async fn pump<S>(client: TcpStream, ws: WebSocketStream<S>, ends: Arc<Ends>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (from_client, to_client) = client.into_split();
        let (to_server, from_server) = ws.split();
        let up = tokio::spawn(up(from_client, to_server, Arc::clone(&ends)));
        down(from_server, to_client, &ends).await;
        let _ = up.await;
    }

    /// Client to server: every frame as one binary message.
    async fn up<S>(
        mut from: OwnedReadHalf,
        mut to: SplitSink<WebSocketStream<S>, Message>,
        ends: Arc<Ends>,
    ) where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut frames = FrameReader::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match from.read(&mut buf).await {
                Ok(0) if !ends.aborted() => {
                    let _ = to.close().await;
                    return;
                }
                Ok(n) if n > 0 => n,
                _ => return ends.abort(),
            };
            frames.push(&buf[..n]);
            loop {
                let frame = match frames.next_frame() {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(_) => return ends.abort(),
                };
                if to.send(Message::Binary(frame.into())).await.is_err() {
                    return ends.abort();
                }
            }
        }
    }

    /// Server to client: every binary message's bytes.
    async fn down<S>(from: SplitStream<WebSocketStream<S>>, mut to: OwnedWriteHalf, ends: &Ends)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        if pass_down(from, &mut to).await && !ends.aborted() {
            let _ = to.shutdown().await;
        } else {
            ends.abort();
            // Dropped, the half would send a FIN ahead of the reset.
            to.forget();
        }
    }

    /// Passes messages down until the server closes (true) or the line
    /// breaks (false).
    async fn pass_down<S>(
        mut from: SplitStream<WebSocketStream<S>>,
        to: &mut OwnedWriteHalf,
    ) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        loop {
            match from.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    if to.write_all(&bytes).await.is_err() {
                        return false;
                    }
                }
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return false,
            }
        }
    }
}
