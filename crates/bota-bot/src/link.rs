//! The wire between a bot and a server.

use std::io::{Read, Write};
use std::net::TcpStream;

use bota_proto::{
    ClientMsg, FrameReader, HeroId, PlayerId, Role, ServerMsg, SlotId, TickMode,
    encode_frame_to_vec,
};

use crate::Ask;

/// What the server said when it took a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seated {
    /// The handle this connection was given.
    pub player: PlayerId,
    /// The seat it was given. Absent when no seat was free.
    pub slot: Option<SlotId>,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    /// How the server advances ticks.
    pub mode: TickMode,
}

/// One framed connection to a server.
pub struct Link {
    stream: TcpStream,
    reader: FrameReader,
    seq: u32,
}

impl Link {
    /// Joins a server, waits for a seat, and asks for a hero.
    pub fn join(addr: &str, name: &str, hero: HeroId) -> std::io::Result<(Link, Seated)> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        let mut link = Link {
            stream,
            reader: FrameReader::new(),
            seq: 0,
        };
        link.send(&ClientMsg::Hello {
            role: Role::Bot,
            name: name.to_string(),
        })?;
        let seated = loop {
            match link.hear()? {
                None => return Err(std::io::Error::other("the server took nobody")),
                Some(ServerMsg::Welcome {
                    player_id,
                    slot,
                    tick_rate,
                    mode,
                }) => {
                    break Seated {
                        player: player_id,
                        slot,
                        tick_rate,
                        mode,
                    };
                }
                Some(_) => {}
            }
        };
        link.send(&ClientMsg::PickHero { hero })?;
        link.send(&ClientMsg::SetReady(true))?;
        Ok((link, seated))
    }

    /// Puts one message on the wire.
    pub fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        let frame = encode_frame_to_vec(msg)
            .map_err(|_| std::io::Error::other("a message would not encode"))?;
        self.stream.write_all(&frame)
    }

    /// Sends one ask under the next sequence number.
    pub fn order(&mut self, ask: Ask) -> std::io::Result<()> {
        self.seq += 1;
        let seq = self.seq;
        self.send(&ClientMsg::Order {
            seq,
            unit: ask.unit,
            order: ask.order,
        })
    }

    /// Says this seat has finished thinking about a tick.
    pub fn done_thinking(&mut self, tick: u32) -> std::io::Result<()> {
        self.send(&ClientMsg::Ack { tick })
    }

    /// The next message from the server, waiting for one if it must.
    ///
    /// `None` means the server hung up.
    pub fn hear(&mut self) -> std::io::Result<Option<ServerMsg>> {
        loop {
            match self.reader.next_message::<ServerMsg>() {
                Ok(Some(msg)) => return Ok(Some(msg)),
                Ok(None) => {}
                Err(_) => return Err(std::io::Error::other("a message would not decode")),
            }
            let mut buf = [0u8; 65536];
            let read = self.stream.read(&mut buf)?;
            if read == 0 {
                return Ok(None);
            }
            self.reader.push(&buf[..read]);
        }
    }
}
