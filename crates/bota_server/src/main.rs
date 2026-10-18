//! Command line entry point of the server.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, ValueEnum};

use bota_core::rules;
use bota_proto::{HeroId, Role, SeatKey, Team, encode_frame_to_vec};
use bota_server::game::{OrderDelay, check_order_delays};
use bota_server::game_loop::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, check_opts, run,
};
use bota_server::lobby::ReservedSeat;
use bota_server::net::{DEFAULT_MESSAGES_PER_SEC, Listeners};
use bota_server::spectators::Spectators;

/// The exit code of a match nobody played: no result, nothing to rate.
const EXIT_ABANDONED: u8 = 3;

/// Connections held at once unless the command line says otherwise.
const DEFAULT_MAX_CONNECTIONS: usize = 64;

#[cfg(test)]
mod map_cli_tests;

/// How the server advances ticks.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    /// A fixed rate on the wall clock.
    Realtime,
    /// Wait for every participant each tick.
    Lockstep,
}

/// The environment variables that carry keys: an argument would show in every
/// process listing, so keys are taken from nowhere else.
const SEATS_VAR: &str = "BOTA_SEATS";
const REFEREE_VAR: &str = "BOTA_REFEREE_KEY";

/// bota match server: one lobby, one match, then exit.
#[derive(Parser, Debug)]
#[command(
    after_help = "Keys come from the environment only (an argument shows in process lists):
  BOTA_SEATS        reserve the seats, in slot order: key_hex:team:name[:kind] or key_hex:name,
                    comma separated (team 0 Radiant, 1 Dire; key_hex is 64 hex digits; kind bot
                    or human admits only that kind of player to the seat). A Hello must then
                    carry a key, and a key may take its seat again after a disconnect.
  BOTA_REFEREE_KEY  admit a referee with this key: told the lobby, the start, departures and
                    the end, never a tick."
)]
struct Args {
    /// Port to listen on.
    #[arg(long, default_value_t = 4455)]
    port: u16,
    /// Address every listener binds to. Loopback keeps the server to this machine; any other
    /// address needs a certificate on the WebSocket port (--tls-cert, --tls-key), and then the
    /// plain TCP port stays on loopback, or --insecure-plaintext.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,
    /// Let plain text (TCP, ws://) listen on a --bind other machines reach. Keys and orders then
    /// cross the network in clear: only for a network you trust.
    #[arg(long)]
    insecure_plaintext: bool,
    /// Also listen for WebSocket connections on this port: one binary message per frame, and
    /// everything else as on the TCP port. Plain ws:// without --tls-cert and --tls-key, for
    /// local use; wss:// (TLS 1.3) with them.
    #[cfg(feature = "websocket")]
    #[arg(long)]
    ws_port: Option<u16>,
    /// PEM certificate chain of the WebSocket listener, leaf first.
    #[cfg(feature = "websocket")]
    #[arg(long, requires_all = ["ws_port", "tls_key"])]
    tls_cert: Option<PathBuf>,
    /// PEM private key of the WebSocket listener's certificate.
    #[cfg(feature = "websocket")]
    #[arg(long, requires_all = ["ws_port", "tls_cert"])]
    tls_key: Option<PathBuf>,
    /// How ticks advance.
    #[arg(long, value_enum, default_value_t = Mode::Realtime)]
    mode: Mode,
    /// Ticks per wall-clock second.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u16).range(1..))]
    tick_rate: u16,
    /// Seats in the match. Implied by the seat reservations when there are any.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..))]
    players: Option<u8>,
    /// Write a replay to this file.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Which map to play: 0 Dota, 1 hero demo, 2 mid-only Dota (two lives or first tower, twenty game minutes).
    #[arg(long, default_value_t = 0, value_parser = parse_map)]
    map: u16,
    /// Seed of the match randomness. Drawn from the operating system when absent, and then
    /// known to no one.
    #[arg(long)]
    seed: Option<u64>,
    /// Public label of the match. Drawn from the key when absent (the same for the same seed); no
    /// randomness comes from it.
    #[arg(long)]
    match_id: Option<u64>,
    /// Seat reservations, from `BOTA_SEATS` only.
    #[arg(skip)]
    seats: Vec<String>,
    /// The referee's key, from `BOTA_REFEREE_KEY` only.
    #[arg(skip)]
    referee_key: Option<String>,
    /// With reserved seats, end the lobby phase this many seconds after listening at the latest;
    /// a seat without a pick then gets a random hero.
    #[arg(long)]
    start_after_secs: Option<u64>,
    /// When the lobby phase runs out of time (--start-after-secs, or 60 s without it) with no
    /// seat held, exit with code 3 instead of playing the match: nobody came.
    #[arg(long)]
    abandon_unjoined: bool,
    /// Exit with code 3 once every seat has been empty this many seconds during the match;
    /// without it an empty match whose seats may be taken back plays on to its end.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    empty_after_secs: Option<u64>,
    /// Who may watch without a seat's key: off; delayed=SECS, everything that many seconds of
    /// game time late (as Dota's tv_delay); open, everything live (local tools only). A seat's
    /// key always watches its seat live, and only it is told that seat's orders live.
    #[arg(long, default_value_t = Spectators::Off)]
    spectators: Spectators,
    /// The heroes a seat may pick, by id, comma separated; a seat without a pick is given one of
    /// them. Every hero when absent.
    #[arg(long, value_delimiter = ',')]
    heroes: Option<Vec<u16>>,
    /// Seconds the seats have to pick once all are held (Dota's default). After that a seat
    /// without a hero loses 2 starting gold per second until it picks, and is given a random
    /// allowed hero after 30 more seconds.
    #[arg(long, default_value_t = bota_core::rules::PICK_SECS)]
    pick_secs: u64,
    /// Seconds of tactical phase after the picks (Dota's default): the seats plan their start
    /// before the match begins. A match of bots alone has none.
    #[arg(long, default_value_t = bota_core::rules::TACTICAL_SECS)]
    tactical_secs: u64,
    /// Whether two seats may pick the same hero (Dota's 1v1 Solo Mid and Mid Only allow it).
    /// With `false`, a hero some seat holds is refused to the others.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    same_heroes_allowed: bool,
    /// Most connections held at once.
    #[arg(long, default_value_t = DEFAULT_MAX_CONNECTIONS)]
    max_connections: usize,
    /// Print the fingerprint of the wire registries this build speaks and exit. Every client
    /// sends its own in its Hello, and one that differs is refused.
    #[arg(long)]
    wire_fingerprint: bool,
    /// Lockstep: tick-lengths to wait for a straggler before moving on.
    #[arg(long, default_value_t = 150)]
    ack_timeout_ticks: u32,
    /// Seconds a connection may send nothing (the server pings every second and a client
    /// answers), or take nothing of what is sent, before it is dropped as dead and its seat
    /// waits for a reconnect. 0 waits for ever.
    #[arg(long, default_value_t = DEFAULT_IDLE_TIMEOUT.as_secs())]
    idle_timeout_secs: u64,
    /// Messages one connection may send a second, besides four for every tick it is sent and
    /// a burst of two seconds' worth; a peer that sends more is cut off.
    #[arg(long, default_value_t = DEFAULT_MESSAGES_PER_SEC)]
    messages_per_sec: NonZeroU32,
    /// Spectators that may watch through one reserved seat's key at once.
    #[arg(long, default_value_t = DEFAULT_SPECTATORS_PER_SEAT)]
    max_spectators_per_seat: usize,
    /// Honour cheat orders: gold, levels, items and refreshes for the asking.
    #[arg(long)]
    cheats: bool,
    /// Structures take damage but never fall: no tower-loss ending, for practice.
    #[arg(long)]
    immortal_structures: bool,
    /// Heroes take damage but never die: no hero-death ending, for last-hit practice.
    #[arg(long)]
    immortal_heroes: bool,
    /// Seat a dummy in the last seat: its hero stays put and never acts, so one player can practise alone.
    #[arg(long)]
    dummy_seat: bool,
    /// Ticks every order waits past the earliest tick it could take effect on: one value for every seat, or one per seat in slot order.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    order_delay_ticks: Vec<u32>,
    /// Most extra ticks one order waits on top of its delay, drawn per order from a hidden stream of the seed: one value for every seat, or one per seat.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    order_jitter_ticks: Vec<u32>,
}

fn main() -> ExitCode {
    let mut args = Args::parse();
    if args.wire_fingerprint {
        println!(
            "{}",
            bota_proto::fingerprint_to_hex(bota_proto::WIRE_FINGERPRINT)
        );
        return ExitCode::SUCCESS;
    }
    let tick_rate = args.tick_rate;
    let keys = secret_env(SEATS_VAR).and_then(|seats| Ok((seats, secret_env(REFEREE_VAR)?)));
    match keys {
        Ok((seats, referee_key)) => {
            args.seats = seats.map_or_else(Vec::new, |s| s.split(',').map(str::to_owned).collect());
            args.referee_key = referee_key;
        }
        Err(error) => {
            eprintln!("bota_server: {error}");
            return ExitCode::FAILURE;
        }
    }
    match serve(args) {
        Ok(MatchEnd::Over { winner, report }) => {
            println!("{}", result_line(winner, &report, tick_rate));
            ExitCode::SUCCESS
        }
        Ok(MatchEnd::Abandoned) => ExitCode::from(EXIT_ABANDONED),
        Err(error) => {
            eprintln!("bota_server: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Listens, says `READY`, and plays the match.
fn serve(args: Args) -> std::io::Result<MatchEnd> {
    let opts = server_opts(&args)?;
    // Contradicting options fail before READY, never after.
    check_opts(&opts).map_err(std::io::Error::other)?;
    // A replay that cannot be written fails before the port is announced.
    if let Some(path) = &opts.replay {
        bota_server::replay::ReplayWriter::create(path)?;
    }
    let listeners = listen(&args)?;
    println!("READY");
    run(listeners, opts)
}

/// A variable of the environment; absent when unset or empty.
fn secret_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value).filter(|value| !value.is_empty())),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not UTF-8")),
    }
}

/// The line a launcher reads the result from: the winning team (0 Radiant,
/// 1 Dire) or a draw, the game time the match lasted, and the
/// [`bota_proto::MatchReport`] as hex postcard bytes.
fn result_line(winner: Team, report: &bota_proto::MatchReport, tick_rate: u16) -> String {
    let winner = match winner {
        Team::Radiant => "0",
        Team::Dire => "1",
        Team::Neutral => "draw",
    };
    let ms = u64::from(report.stats.duration) * 1000 / u64::from(tick_rate);
    let duration_ms = u32::try_from(ms).unwrap_or(u32::MAX);
    let frame = encode_frame_to_vec(report).expect("a report encodes");
    let hex: String = frame[bota_proto::LEN_PREFIX..]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("RESULT winner={winner} duration_ms={duration_ms} report={hex}")
}

/// The server's options from the command line.
fn server_opts(args: &Args) -> std::io::Result<ServerOpts> {
    let reserved = reservations(&args.seats).map_err(std::io::Error::other)?;
    let referee_key = args
        .referee_key
        .as_deref()
        .map(|hex| SeatKey::from_hex(hex).ok_or("a key is 64 hex digits"))
        .transpose()
        .map_err(std::io::Error::other)?;
    let players = match (args.players, reserved.len()) {
        (Some(players), 0) => players,
        (None, 0) => 2,
        (Some(players), seats) if usize::from(players) != seats => {
            return Err(std::io::Error::other(format!(
                "--players {players} contradicts the {seats} reserved seats"
            )));
        }
        (_, seats) if args.dummy_seat || seats > usize::from(u8::MAX) => {
            return Err(std::io::Error::other(
                "reserved seats take no dummy seat and at most 255 seats",
            ));
        }
        (_, seats) => seats as u8,
    };
    Ok(ServerOpts {
        mode: match args.mode {
            Mode::Realtime => bota_proto::TickMode::Realtime,
            Mode::Lockstep => bota_proto::TickMode::Lockstep,
        },
        tick_rate: args.tick_rate,
        players,
        replay: args.replay.clone(),
        seed: args.seed,
        match_id: args.match_id,
        reserved,
        referee_key,
        start_after: args.start_after_secs.map(Duration::from_secs),
        max_connections: Some(args.max_connections),
        map: bota_proto::MapId(args.map),
        ack_timeout_ticks: args.ack_timeout_ticks,
        idle_timeout: (args.idle_timeout_secs > 0)
            .then(|| Duration::from_secs(args.idle_timeout_secs)),
        cheats: args.cheats,
        immortal_structures: args.immortal_structures,
        immortal_heroes: args.immortal_heroes,
        dummy_seat: args.dummy_seat,
        order_delays: order_delays(args, players).map_err(std::io::Error::other)?,
        messages_per_sec: args.messages_per_sec,
        max_spectators_per_seat: args.max_spectators_per_seat,
        spectators: args.spectators,
        heroes: args
            .heroes
            .as_ref()
            .map(|ids| ids.iter().copied().map(HeroId).collect()),
        abandon_unjoined: args.abandon_unjoined,
        empty_after: args.empty_after_secs.map(Duration::from_secs),
        pick_time: Duration::from_secs(args.pick_secs),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::from_secs(args.tactical_secs),
        same_heroes: args.same_heroes_allowed,
    })
}

/// Binds every listener the command line asks for, then announces each, the
/// TCP one first.
fn listen(args: &Args) -> std::io::Result<Listeners> {
    #[cfg(feature = "websocket")]
    let tls = args.tls_cert.is_some();
    #[cfg(not(feature = "websocket"))]
    let tls = false;
    let plain =
        plain_bind(args.bind, tls, args.insecure_plaintext).map_err(std::io::Error::other)?;
    if !plain.is_loopback() {
        eprintln!("bota_server: plain text on {plain}: keys and orders cross the network in clear");
    }
    let tcp = TcpListener::bind((plain, args.port))?;
    #[cfg(feature = "websocket")]
    if let Some(port) = args.ws_port {
        let tls = match (&args.tls_cert, &args.tls_key) {
            (Some(cert), Some(key)) => Some(bota_server::net::Tls::from_pem_files(cert, key)?),
            _ => None,
        };
        let scheme = if tls.is_some() { "wss" } else { "ws" };
        let ws = TcpListener::bind((args.bind, port))?;
        println!("bota_server listening on {}", tcp.local_addr()?);
        println!("bota_server listening on {scheme}://{}", ws.local_addr()?);
        let ws = bota_server::net::WsListener::new(ws, tls);
        return Ok(Listeners::with_websocket(tcp, ws));
    }
    println!("bota_server listening on {}", tcp.local_addr()?);
    Ok(tcp.into())
}

/// Where the plain-text listeners bind: `--bind` when it is loopback or plain
/// text was allowed there; loopback when the WebSocket port has a certificate,
/// so that only TLS leaves the machine; refused otherwise.
fn plain_bind(bind: IpAddr, tls: bool, insecure_plaintext: bool) -> Result<IpAddr, String> {
    if bind.is_loopback() || insecure_plaintext {
        return Ok(bind);
    }
    if tls {
        return Ok(match bind {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        });
    }
    Err(format!(
        "--bind {bind} lets other machines in, and without a certificate keys and orders would \
cross the network in clear: give the WebSocket port --tls-cert and --tls-key, or pass \
--insecure-plaintext for a network you trust"
    ))
}

fn parse_map(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|id| bota_core::MAPS.iter().any(|map| map.id.0 == *id))
        .ok_or_else(|| {
            format!("unsupported map '{value}'; expected 0 (Dota), 1 (demo), or 2 (mid-only Dota)")
        })
}

/// One order delay per seat from the two lists, each holding one value for
/// every seat or one per seat.
fn order_delays(args: &Args, players: u8) -> Result<Vec<OrderDelay>, String> {
    let seats = usize::from(players);
    let per_seat = |values: &[u32], flag: &str| match values.len() {
        1 => Ok(vec![values[0]; seats]),
        n if n == seats => Ok(values.to_vec()),
        n => Err(format!(
            "--{flag} has {n} values for {seats} seats; give one or one per seat"
        )),
    };
    let bases = per_seat(&args.order_delay_ticks, "order-delay-ticks")?;
    let jitters = per_seat(&args.order_jitter_ticks, "order-jitter-ticks")?;
    let delays: Vec<OrderDelay> = bases
        .into_iter()
        .zip(jitters)
        .map(|(base, jitter)| OrderDelay { base, jitter })
        .collect();
    check_order_delays(&delays, seats).map_err(|error| error.to_string())?;
    Ok(delays)
}

/// Seats from `key_hex:team:name[:kind]` or `key_hex:name` entries. A middle
/// field that is a team index (`0` or `1`) makes the team form, so a two-field
/// name may not start with one followed by a colon; in the team form a last
/// field `bot` or `human` states the kind of player the seat is for.
fn reservations(entries: &[String]) -> Result<Vec<ReservedSeat>, String> {
    let seats: Vec<ReservedSeat> = entries
        .iter()
        .enumerate()
        .map(|(slot, entry)| reservation(slot, entry))
        .collect::<Result<_, _>>()?;
    let sides = [Team::Radiant, Team::Dire].map(|team| seats.iter().any(|s| s.team == team));
    if seats.len() > 1 && sides.contains(&false) {
        return Err("reserved seats leave a side empty".to_owned());
    }
    Ok(seats)
}

fn reservation(slot: usize, entry: &str) -> Result<ReservedSeat, String> {
    let (key, rest) = entry
        .split_once(':')
        .ok_or("a seat reservation reads key_hex:team:name or key_hex:name")?;
    let (team, name) = match rest.split_once(':') {
        Some(("0", name)) => (Some(Team::Radiant), name),
        Some(("1", name)) => (Some(Team::Dire), name),
        _ => (None, rest),
    };
    let (kind, name) = match name.rsplit_once(':') {
        Some((name, "bot")) if team.is_some() => (Some(Role::Bot), name),
        Some((name, "human")) if team.is_some() => (Some(Role::Player), name),
        _ => (None, name),
    };
    let team = team.unwrap_or(if slot.is_multiple_of(2) {
        Team::Radiant
    } else {
        Team::Dire
    });
    if name.is_empty() || name.len() > rules::MAX_ANNOUNCED_NAME_BYTES {
        return Err(format!(
            "a seat name takes 1 to {} bytes",
            rules::MAX_ANNOUNCED_NAME_BYTES
        ));
    }
    Ok(ReservedSeat {
        key: SeatKey::from_hex(key).ok_or("a key is 64 hex digits")?,
        name: name.to_owned(),
        team,
        kind,
    })
}
