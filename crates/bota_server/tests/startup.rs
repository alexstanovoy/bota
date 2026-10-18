//! What the server binary does before it announces its port.

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn a_replay_that_cannot_be_written_fails_before_the_port_is_announced() {
    // A path through a file cannot be created on any platform.
    let replay = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml/match.brp");
    let mut child = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args(["--port", "0", "--replay", replay])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().expect("the server is waited on").is_none() {
        if Instant::now() > deadline {
            child.kill().expect("the server is stopped");
            panic!("the server kept running with an unwritable replay");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().expect("the output is read");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(!stdout.contains("listening on"), "stdout: {stdout}");
    assert!(stderr.contains(replay), "stderr: {stderr}");
}

/// What the binary printed before it said `READY` (then it is stopped) or
/// exited, and whether it got that far.
struct Start {
    ready: bool,
    stdout: Vec<String>,
    stderr: String,
}

fn start(args: &[&str], env: &[(&str, &str)]) -> Start {
    use std::io::{BufRead, BufReader, Read};
    let mut child = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args(["--port", "0"])
        .args(args)
        .env_remove("BOTA_SEATS")
        .env_remove("BOTA_REFEREE_KEY")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server starts");
    let (mut ready, mut stdout) = (false, Vec::new());
    for line in BufReader::new(child.stdout.take().expect("piped")).lines() {
        let line = line.expect("stdout is text");
        if line == "READY" {
            ready = true;
            child.kill().expect("the server is stopped");
            break;
        }
        stdout.push(line);
    }
    child.wait().expect("the server is waited on");
    let mut stderr = String::new();
    let mut pipe = child.stderr.take().expect("piped");
    pipe.read_to_string(&mut stderr).expect("stderr is text");
    Start {
        ready,
        stdout,
        stderr,
    }
}

#[test]
fn without_a_bind_address_the_server_listens_on_loopback_only() {
    let start = start(&[], &[]);
    assert!(start.ready, "{}", start.stderr);
    assert!(
        start.stdout[0].starts_with("bota_server listening on 127.0.0.1:"),
        "{:?}",
        start.stdout
    );
}

#[test]
fn plain_text_where_other_machines_reach_is_refused_unless_asked_for() {
    let refused = start(&["--bind", "0.0.0.0"], &[]);
    assert!(!refused.ready, "{:?}", refused.stdout);
    assert!(refused.stdout.is_empty(), "{:?}", refused.stdout);
    assert!(
        refused.stderr.contains("--insecure-plaintext"),
        "{}",
        refused.stderr
    );

    let allowed = start(&["--bind", "0.0.0.0", "--insecure-plaintext"], &[]);
    assert!(allowed.ready, "{}", allowed.stderr);
    assert!(
        allowed.stdout[0].starts_with("bota_server listening on 0.0.0.0:"),
        "{:?}",
        allowed.stdout
    );
    assert!(allowed.stderr.contains("in clear"), "{}", allowed.stderr);
}

#[test]
fn keys_are_taken_from_the_environment_never_from_arguments() {
    let key = "11".repeat(32);
    let seat = format!("{key}:0:one,{key}:1:two");
    let refused = start(&["--seat", &seat], &[]);
    assert!(!refused.ready, "{:?}", refused.stdout);
    assert!(refused.stdout.is_empty(), "{:?}", refused.stdout);

    let reserved = start(&[], &[("BOTA_SEATS", &seat), ("BOTA_REFEREE_KEY", &key)]);
    assert!(reserved.ready, "{}", reserved.stderr);
    assert!(!reserved.stderr.contains(&key), "{}", reserved.stderr);
}

#[cfg(feature = "websocket")]
#[test]
fn with_a_certificate_only_tls_leaves_the_machine() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/tls");
    let (cert, key) = (format!("{dir}/cert.pem"), format!("{dir}/key.pem"));
    let start = start(
        &[
            "--bind",
            "0.0.0.0",
            "--ws-port",
            "0",
            "--tls-cert",
            &cert,
            "--tls-key",
            &key,
        ],
        &[],
    );
    assert!(start.ready, "{}", start.stderr);
    assert!(
        start.stdout[0].starts_with("bota_server listening on 127.0.0.1:"),
        "{:?}",
        start.stdout
    );
    assert!(
        start.stdout[1].starts_with("bota_server listening on wss://0.0.0.0:"),
        "{:?}",
        start.stdout
    );
}
