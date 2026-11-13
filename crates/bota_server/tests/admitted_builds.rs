//! This build and the builds of the wires it admits (`bota_core::ADMITTED_WIRES`) play each
//! other, both ways, as real separate programs: this build's client takes a seat at the earlier
//! build's server and buys an item there, and the earlier build's client (`examples/admitted_peer.rs`
//! as that build compiled it) does the same at this build's server. Both matches play on the
//! earlier build's items.
//!
//! The earlier builds come from `tools/admitted.sh build`, under `ADMITTED_BUILDS` (the
//! workspace's `target/admitted` when unset); a build missing fails the test, naming the command.

#[path = "admitted/peer.rs"]
mod peer;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use bota_core::{ADMITTED_WIRES, AdmittedWire};
use bota_proto::{ItemId, MapId, TickMode, fingerprint_to_hex};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

/// What each client buys: an item of the first catalogue ever admitted, sold at the fountain.
const BOUGHT: ItemId = ItemId::CIRCLET;

/// Where the builds of the admitted wires are.
fn builds() -> PathBuf {
    std::env::var_os("ADMITTED_BUILDS").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/admitted"),
        PathBuf::from,
    )
}

/// A program of the build of `admitted`, which must be there.
fn program(admitted: AdmittedWire, name: &str) -> PathBuf {
    let path = builds()
        .join(fingerprint_to_hex(admitted.wire))
        .join("target/release")
        .join(name);
    assert!(
        path.is_file(),
        "{} is missing: build the admitted wires with `tools/admitted.sh build`",
        path.display()
    );
    path
}

/// The server of the build of `admitted`, started on a free port of loopback for one seat on the
/// demo map with cheats, and its address.
struct EarlierServer {
    child: Child,
    address: String,
}

impl EarlierServer {
    fn start(admitted: AdmittedWire) -> EarlierServer {
        let server = program(admitted, "bota_server");
        let wire = Command::new(&server)
            .arg("--wire-fingerprint")
            .output()
            .expect("the earlier server says its wire");
        assert_eq!(
            String::from_utf8_lossy(&wire.stdout).trim(),
            fingerprint_to_hex(admitted.wire),
            "{} is of another wire",
            server.display()
        );
        let mut child = Command::new(&server)
            .args(["--port", "0", "--players", "1", "--map", "1", "--cheats"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the earlier server starts");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        let mut address = None;
        for line in stdout.lines() {
            let line = line.expect("stdout is text");
            if let Some(at) = line.strip_prefix("bota_server listening on ") {
                address = at.split_whitespace().next().map(str::to_owned);
            }
            if line == "READY" {
                break;
            }
        }
        let address = address.expect("the earlier server announced its port");
        EarlierServer { child, address }
    }
}

impl Drop for EarlierServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// This build's server, in this process, for one seat on the demo map with cheats.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let opts = ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Realtime,
        tick_rate: 30,
        players: 1,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(11),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(1),
        cheats: true,
        lobby_cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
        ack_timeout_ticks: 150,
        idle_timeout: None,
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(1)),
    };
    thread::spawn(move || {
        let _ = run(listener, opts);
    });
    address
}

/// What a client buying [`BOUGHT`] in a match on the items of `admitted` says.
fn bought_on(admitted: AdmittedWire, said: &str) {
    let expected = format!("catalogue {} tick ", admitted.catalogue.items);
    assert!(
        said.starts_with(&expected) && said.ends_with(&format!(" bought {}", BOUGHT.0)),
        "{said:?}"
    );
}

/// This build's client at the server of every admitted wire: it is refused its own wire, dials
/// again speaking the server's, takes the seat and buys in a match on the earlier build's items.
#[test]
fn this_builds_client_plays_on_the_server_of_every_admitted_wire() {
    for &admitted in ADMITTED_WIRES {
        let server = EarlierServer::start(admitted);
        let said = peer::play(&server.address, BOUGHT).unwrap_or_else(|error| {
            panic!(
                "at the server of wire {}: {error}",
                fingerprint_to_hex(admitted.wire)
            )
        });
        bought_on(admitted, &said);
    }
}

/// The client of every admitted wire, its own program, at this build's server: seated, and buying
/// in a match on its items.
#[test]
fn the_client_of_every_admitted_wire_plays_on_this_builds_server() {
    for &admitted in ADMITTED_WIRES {
        let client = program(admitted, "examples/admitted_peer");
        let address = serve();
        let output = Command::new(&client)
            .args([address.as_str(), &BOUGHT.0.to_string()])
            .stdin(Stdio::null())
            .output()
            .expect("the earlier client runs");
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "the client of wire {}: {said} {}",
            fingerprint_to_hex(admitted.wire),
            String::from_utf8_lossy(&output.stderr)
        );
        bought_on(admitted, said.trim());
    }
}
