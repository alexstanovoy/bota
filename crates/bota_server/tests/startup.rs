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
