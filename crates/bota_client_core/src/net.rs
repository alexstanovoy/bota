//! The live connection to a server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use bota_proto::{ClientMsg, FrameReader, ServerMsg, encode_frame_to_vec};

/// A socket with a reader thread behind it.
///
/// Messages pile up in a channel; [`Net::poll`] hands over whatever has
/// arrived without ever blocking the render loop.
pub struct Net {
    stream: TcpStream,
    inbox: Receiver<ServerMsg>,
    closed: Arc<AtomicBool>,
}

impl Net {
    /// Connects and starts reading.
    pub fn connect(addr: &str) -> std::io::Result<Net> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        let reader_stream = stream.try_clone()?;
        let (tx, inbox) = channel();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_flag = Arc::clone(&closed);
        thread::spawn(move || {
            let mut reader = FrameReader::new();
            let mut stream = reader_stream;
            let mut buf = [0u8; 65536];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                reader.push(&buf[..n]);
                loop {
                    match reader.next_message::<ServerMsg>() {
                        Ok(Some(msg)) => {
                            if tx.send(msg).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            closed_flag.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            }
            closed_flag.store(true, Ordering::Relaxed);
        });
        Ok(Net {
            stream,
            inbox,
            closed,
        })
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        self.inbox.try_iter().collect()
    }

    /// Sends one message. A failed send marks the connection closed.
    pub fn send(&mut self, msg: &ClientMsg) {
        let Ok(frame) = encode_frame_to_vec(msg) else {
            return;
        };
        if self.stream.write_all(&frame).is_err() {
            self.closed.store(true, Ordering::Relaxed);
        }
    }

    /// Whether the server is gone.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}
