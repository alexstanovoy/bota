//! The real radovid service on a loopback port, launching the real
//! `bota_server`, and bota_radovid clients to talk to it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bota_radovid::proto::*;
use bota_radovid::{Client, Handoff, Incoming, Tls, decode_report, fingerprint};
use radovid::config::Config;
use radovid::service::Service;

pub const PASSWORD: &str = "correct horse battery";

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);
static NEXT_PORTS: AtomicU16 = AtomicU16::new(0);
/// Host ports per service.
const PORTS: u16 = 4;
/// The blocks of `PORTS` ports services may take, below the range the system hands out to
/// outgoing connections.
const BLOCKS: u16 = 12_000 / PORTS;
const BLOCKS_FROM: u16 = 20_000;

/// The first port of a block of `PORTS` consecutive loopback ports nobody listens on. Test
/// processes run at once (debug and release, several worktrees), and the service binds its
/// game ports only when it launches a match: each process starts looking at a block its process
/// id picks, and a service takes the next block it can bind whole, so processes meet rarely and
/// never take a block that is in use.
fn free_ports() -> u16 {
    let start = std::process::id() % u32::from(BLOCKS);
    let taken = u32::from(NEXT_PORTS.fetch_add(1, Ordering::Relaxed));
    for step in 0..u32::from(BLOCKS) {
        let block = (start + taken + step) % u32::from(BLOCKS);
        let first = BLOCKS_FROM + PORTS * u16::try_from(block).expect("a block");
        let held: Result<Vec<_>, _> = (first..first + PORTS)
            .map(|port| std::net::TcpListener::bind(("127.0.0.1", port)))
            .collect();
        if held.is_ok() {
            return first;
        }
    }
    panic!("no block of {PORTS} free ports");
}

/// The game server binary, built once per test run with the WebSocket
/// listener that radovid's handoff URL points to.
fn server_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        if let Some(path) = std::env::var_os("BOTA_SERVER_BIN") {
            return PathBuf::from(path);
        }
        let exe = std::env::current_exe().expect("the test's path");
        // target/<profile>/deps/<test> -> target/<profile>
        let profile_dir = exe.parent().and_then(Path::parent).expect("target dir");
        let release = profile_dir
            .file_name()
            .is_some_and(|name| name == "release");
        let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        build
            .args([
                "build",
                "--quiet",
                "-p",
                "bota_server",
                "--features",
                "websocket",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if release {
            build.arg("--release");
        }
        assert!(
            build.status().expect("cargo runs").success(),
            "bota_server builds"
        );
        profile_dir.join("bota_server")
    })
}

/// A running service, stopped when dropped.
pub struct Env {
    runtime: tokio::runtime::Runtime,
    service: Option<Service>,
    pub url: String,
    pub tls: Tls,
    dir: PathBuf,
}

pub const RULESET: &str = "solo_mid";

pub fn solo() -> RulesetId {
    RulesetId(RULESET.to_owned())
}

/// A service with one rated ruleset of two teams of one, played by the real
/// server in lockstep.
pub fn start() -> Env {
    start_with(false)
}

/// The service with `[artifacts] save_replays` as given: on, the game server is told to record
/// the match into the file the service keeps for its seats.
pub fn start_with(replays: bool) -> Env {
    let binary = server_binary().to_owned();
    let dir = std::env::temp_dir().join(format!(
        "bota-radovid-test-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("test dir");
    let first = free_ports();
    let config = format!(
        r#"
listen = "127.0.0.1:0"
database = "{db}"
tls_cert = "unused"
tls_key = "unused"
game_ports = "{first}-{last}"
[password]
memory_kib = 1024
iterations = 1
[artifacts]
save_replays = {replays}
[limits]
games_per_source = 200
game_capacity = {PORTS}
lobby_games_max = 1
[[ruleset]]
id = "{RULESET}"
name = "1x1 Mid Only"
game = "bota"
game_fingerprint = "{fingerprint:016x}"
teams = [1, 1]
rated = true
max_duration_secs = 600
[ruleset.launcher]
kind = "local"
binary = "{binary}"
public_url = "ws://127.0.0.1:{{port}}"
version_probe = ["--wire-fingerprint"]
start_timeout_secs = 30
args = ["--map", "2", "--mode", "lockstep", "--ack-timeout-ticks", "900",
        "--match-id", "{{match_id}}", "--bind", "127.0.0.1", "--port", "0",
        "--ws-port", "{{port}}", "--start-after-secs", "30", "--abandon-unjoined",
        "--empty-after-secs", "60"]
env = {{ BOTA_SEATS = "{{seats}}" }}
artifact = "{artifacts}/replay-{{match_id}}.brp"
artifact_args = ["--replay", "{{artifact}}"]
"#,
        db = dir.join("radovid.db").display(),
        artifacts = dir.join("artifacts").display(),
        last = first + PORTS - 1,
        fingerprint = fingerprint(),
        binary = binary.display(),
    );
    let path = dir.join("radovid.toml");
    std::fs::write(&path, config).expect("config");
    let config = Config::load(&path).expect("a valid configuration");
    let identity = radovid::tls::self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
        .expect("certificate");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let service = runtime
        .block_on(Service::start(config, &identity))
        .expect("the service starts");
    Env {
        url: format!("wss://127.0.0.1:{}", service.addr.port()),
        tls: Tls::trust_pem(&identity.cert_pem).expect("trust"),
        runtime,
        service: Some(service),
        dir,
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Some(service) = self.service.take() {
            self.runtime.block_on(service.shutdown());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Env {
    /// A registered, logged-in account of `kind`.
    pub fn account(&self, login: &str, kind: AccountKind) -> Account {
        let mut account = Account {
            client: Client::open(&self.url, &self.tls).expect("connect"),
            log: Vec::new(),
        };
        let bot_version = (kind == AccountKind::Bot).then(|| "test".to_owned());
        let seq = account
            .client
            .register(login, PASSWORD, kind, bot_version)
            .expect("register");
        account.reply(seq).expect("registered");
        account
    }

    pub fn bot(&self, login: &str) -> Account {
        self.account(login, AccountKind::Bot)
    }
}

/// A client and what it has heard.
pub struct Account {
    pub client: Client,
    pub log: Vec<Event>,
}

impl Account {
    /// Polls for up to `wait`, keeping what comes as `log`; the reply to
    /// `seq`, if it comes.
    fn poll_for(&mut self, wait: Duration, seq: Option<u32>) -> Option<Result<Reply, Error>> {
        match self.client.wait(wait).expect("the connection holds") {
            Some(Incoming::Event(event)) => self.log.push(event),
            Some(Incoming::Reply { seq: got, result }) if Some(got) == seq => return Some(result),
            _ => {}
        }
        None
    }

    /// The answer to the request numbered `seq`.
    pub fn reply(&mut self, seq: u32) -> Result<Reply, Error> {
        let deadline = Instant::now() + HANG_GUARD;
        while Instant::now() < deadline {
            if let Some(result) = self.poll_for(Duration::from_millis(50), Some(seq)) {
                return result;
            }
        }
        panic!("no answer to request {seq}");
    }

    pub fn ask(&mut self, msg: ClientMsg) -> Result<Reply, Error> {
        let seq = self.client.request(msg).expect("sent");
        self.reply(seq)
    }

    pub fn queue(&mut self) -> QueueEntry {
        let seq = self.client.queue(&solo()).expect("sent");
        match self.reply(seq) {
            Ok(Reply::Queued { entry }) => entry,
            other => panic!("queue: {other:?}"),
        }
    }

    pub fn proposal(&self) -> Option<Proposal> {
        self.log.iter().rev().find_map(|event| match event {
            Event::MatchProposed { proposal } => Some(proposal.clone()),
            _ => None,
        })
    }

    pub fn handoff(&self) -> Option<Handoff> {
        self.log.iter().find_map(|event| match event {
            Event::MatchFound { handoff } => Some(handoff.clone()),
            _ => None,
        })
    }

    pub fn result(&self) -> Option<MatchResult> {
        self.log.iter().find_map(|event| match event {
            Event::MatchResult { result } => Some(result.clone()),
            _ => None,
        })
    }
}

/// How long a test waits for the service or a match to do what it must. It guards against a
/// hang only: a match is played at the pace of a busy machine, and nothing asserts how long
/// anything took.
const HANG_GUARD: Duration = Duration::from_secs(600);

/// Polls every account until `done` holds for all of them: automatic answers happen while an
/// account is polled, so accounts that wait on each other are polled together.
pub fn settle(accounts: &mut [&mut Account], mut done: impl FnMut(&Account) -> bool) {
    let deadline = Instant::now() + HANG_GUARD;
    while !accounts.iter().all(|account| done(account)) {
        assert!(Instant::now() < deadline, "the accounts did not settle");
        for account in accounts.iter_mut() {
            account.poll_for(Duration::from_millis(5), None);
        }
    }
}

/// Both bots at the game once a match is found: their handoffs.
pub fn found(accounts: &mut [&mut Account]) -> Vec<Handoff> {
    settle(accounts, |account| account.handoff().is_some());
    accounts.iter().filter_map(|a| a.handoff()).collect()
}

/// The heroes of a result's report, by slot.
pub fn heroes(result: &MatchResult) -> Vec<bota_proto::HeroId> {
    decode_report(result)
        .expect("a report")
        .picks
        .iter()
        .map(|pick| pick.hero)
        .collect()
}
