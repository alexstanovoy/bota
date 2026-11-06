//! Message reservations remain held until the simulation consumes its events.

use super::*;
use crate::net::{
    Connection, FrameSource, FrameStream, Linger, LiveConnections, NetEvent, ReadEnd, Split, Terms,
};
use std::sync::Arc;

struct WatchedStream(TcpStream, mpsc::Sender<()>);
struct WatchedSource(Box<dyn FrameSource>, mpsc::Sender<()>);

impl FrameSource for WatchedSource {
    fn next_frame(&mut self) -> Result<Vec<u8>, ReadEnd> {
        self.0.next_frame()
    }
}
impl Drop for WatchedSource {
    fn drop(&mut self) {
        let _ = self.1.send(());
    }
}
impl FrameStream for WatchedStream {
    fn split(self: Box<Self>, timeout: Option<Duration>) -> std::io::Result<Split> {
        let Self(stream, ended) = *self;
        let mut split = Box::new(stream).split(timeout)?;
        split.source = Box::new(WatchedSource(split.source, ended));
        Ok(split)
    }
}

fn connection(
    id: u32,
    events: &mpsc::Sender<NetEvent>,
    live: &LiveConnections,
    inbound: &crate::net::InboundBudget,
) -> (Connection, TcpStream, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let peer = TcpStream::connect(listener.local_addr().unwrap()).expect("peer");
    let (stream, _) = listener.accept().expect("accepted");
    let (ended, done) = mpsc::channel();
    let tick = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let terms = Terms {
        events,
        inbound,
        live,
        world_tick: &tick,
        linger: &Linger::SERVED,
        idle_timeout: Some(HANG_GUARD),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
    };
    let connection = Connection::spawn(
        bota_proto::PlayerId(id),
        Box::new(WatchedStream(stream, ended)),
        &terms,
    )
    .expect("connection threads");
    (connection, peer, done)
}

fn send_while_consumer_is_held(peer: &mut TcpStream, done: &mpsc::Receiver<()>, count: usize) {
    let frame = encode_frame_to_vec(&ClientMsg::SetReady(true)).expect("encoded");
    peer.write_all(&frame.repeat(count)).expect("messages sent");
    peer.shutdown(std::net::Shutdown::Write)
        .expect("finished sending");
    done.recv_timeout(HANG_GUARD)
        .expect("the reader finished before the consumer runs");
}

#[test]
fn inbound_queue_bounds_one_peer_without_losing_its_terminal_event() {
    let (send, receive) = mpsc::channel();
    let live = LiveConnections::default();
    let inbound = crate::net::InboundBudget::default();
    let (connection, mut peer, done) = connection(1, &send, &live, &inbound);
    send_while_consumer_is_held(&mut peer, &done, 65);
    let events: Vec<_> = receive.try_iter().collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, NetEvent::Msg(..)))
            .count(),
        64,
        "a peer may queue only its bounded prefix"
    );
    assert!(
        matches!(events.last(), Some(NetEvent::Breached(..))),
        "the overload signal still reaches the consumer"
    );
    if let Some(NetEvent::Breached(_, reason)) = events.last() {
        assert_eq!(
            reason.to_string(),
            "reached the queued client message limit"
        );
    }
    connection.abort();
}

#[test]
fn inbound_queue_bounds_all_peers_and_releases_consumed_reservations() {
    let (send, receive) = mpsc::channel();
    let live = LiveConnections::default();
    let inbound = crate::net::InboundBudget::default();
    let mut connections = Vec::new();
    for id in 0..17 {
        let (connection, mut peer, done) = connection(id, &send, &live, &inbound);
        send_while_consumer_is_held(&mut peer, &done, 65);
        connections.push((connection, peer));
    }
    let events: Vec<_> = receive.try_iter().collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, NetEvent::Msg(..)))
            .count(),
        1024,
        "all peers share a bounded data queue"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, NetEvent::Breached(..)))
            .count(),
        17,
        "data exhaustion cannot suppress terminal events"
    );
    drop(events);
    let (fresh, mut peer, done) = connection(100, &send, &live, &inbound);
    send_while_consumer_is_held(&mut peer, &done, 1);
    assert_eq!(
        receive
            .try_iter()
            .filter(|e| matches!(e, NetEvent::Msg(..)))
            .count(),
        1,
        "consuming events gives the next peer its room back"
    );
    fresh.abort();
    for (connection, _) in connections {
        connection.abort();
    }
}
