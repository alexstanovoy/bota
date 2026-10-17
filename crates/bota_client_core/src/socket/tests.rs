//! How the threaded [`Net`] ends: a clean close is no message, a broken
//! connection is an error.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_proto::{ClientMsg, Role, ServerMsg, WIRE_FINGERPRINT, encode_frame_to_vec};

use crate::{Net, Redial, Transport};

fn hello() -> ClientMsg {
    ClientMsg::Hello {
        role: Role::Bot,
        name: "net".to_string(),
        key: None,
        wire_fingerprint: WIRE_FINGERPRINT,
    }
}

#[test]
fn a_clean_close_is_no_message() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let (mut peer, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 256];
        let _ = peer.read(&mut buf).expect("the hello arrives");
    });
    let mut net = Net::connect_with(&addr, Redial::OFF).expect("connect");
    net.send(&hello()).expect("send");
    server.join().expect("the server side ends");
    assert!(net.recv().expect("no error").is_none());
}

#[test]
fn a_reset_connection_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let (peer, _) = listener.accept().expect("accept");
        // Closing with the hello unread makes the kernel reset the connection.
        thread::sleep(Duration::from_millis(200));
        drop(peer);
    });
    let mut net = Net::connect_with(&addr, Redial::OFF).expect("connect");
    net.send(&hello()).expect("send");
    server.join().expect("the server side ends");
    net.recv().expect_err("a reset is not a clean close");
}

#[test]
fn a_redial_lasts_until_the_server_answers_on_the_fresh_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let (first, _) = listener.accept().expect("accept");
        thread::sleep(Duration::from_millis(200));
        drop(first);
        let (mut fresh, _) = listener.accept().expect("the client dials again");
        let mut buf = [0u8; 256];
        let _ = fresh.read(&mut buf).expect("the hello arrives again");
        // Connected and greeted, but nothing back for a while: a black hole.
        thread::sleep(Duration::from_millis(500));
        fresh
            .write_all(&encode_frame_to_vec(&ServerMsg::Ping).expect("encode"))
            .expect("ping");
        thread::sleep(Duration::from_millis(300));
    });
    let mut net = Net::connect(&addr).expect("connect");
    net.send(&hello()).expect("send");
    thread::sleep(Duration::from_millis(500));
    assert!(net.is_redialing(), "no word from the fresh connection yet");
    thread::sleep(Duration::from_millis(500));
    assert!(!net.is_redialing(), "the server answered");
    server.join().expect("the server side ends");
}
