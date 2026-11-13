//! The client built before names moved out is refused by the normal content-hash handshake.
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_previous_build_is_refused_with_the_exact_wire_mismatch_text() {
    let executable = std::env::current_exe().expect("test executable");
    let target = executable
        .ancestors()
        .nth(3)
        .expect("Cargo target directory");
    let peer = target.join("replaced_client/peer");
    assert!(
        peer.is_file(),
        "build the previous client with tools/replaced_client.sh"
    );
    let fingerprint = Command::new(&peer)
        .arg("--wire-fingerprint")
        .output()
        .expect("previous fingerprint");
    assert!(fingerprint.status.success());
    let previous = String::from_utf8(fingerprint.stdout).expect("hex fingerprint");
    let previous = u64::from_str_radix(previous.trim(), 16).expect("hex fingerprint");
    assert_ne!(previous, bota_core::WIRE_FINGERPRINT);
    assert!(
        !bota_core::ADMITTED_WIRES
            .iter()
            .any(|wire| wire.wire == previous)
    );
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_bota_server"))
            .args(["--port", "0", "--players", "1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("server starts"),
    );
    let pipe = server.0.stdout.take().expect("piped stdout");
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut address = None;
        for line in BufReader::new(pipe).lines().take(16) {
            let line = line.expect("server output");
            if let Some(value) = line.strip_prefix("bota_server listening on ") {
                address = value.split_whitespace().next().map(str::to_owned);
            }
            if line == "READY" {
                let _ = send.send(address.expect("address before readiness"));
                break;
            }
        }
    });
    let address = receive
        .recv_timeout(Duration::from_secs(60))
        .expect("server listening");
    reader.join().expect("reader ends");
    let refused = Command::new(peer)
        .arg(address)
        .output()
        .expect("previous client runs");
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(refused.stderr).expect("error text"),
        format!(
            "the server speaks wire {:016x} and this client {previous:016x}; build both from matching commits\n",
            bota_core::WIRE_FINGERPRINT
        )
    );
    assert!(
        refused.stdout.is_empty(),
        "a refused client never enters the match"
    );
}
