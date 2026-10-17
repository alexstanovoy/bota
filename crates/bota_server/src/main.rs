//! Command line entry point of the server.

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};

use bota_proto::{MAX_CLIENT_NAME_LEN, SeatKey};
use bota_server::game::{OrderDelay, check_order_delays};
use bota_server::game_loop::{DEFAULT_IDLE_TIMEOUT, ServerOpts, run};
use bota_server::lobby::ReservedSeat;

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

/// bota match server: one lobby, one match, then exit.
#[derive(Parser, Debug)]
struct Args {
    /// Port to listen on.
    #[arg(long, default_value_t = 4455)]
    port: u16,
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
    /// Seats held for key holders, one `key_hex:name` per seat in slot order, comma separated.
    /// A Hello must then carry a key. Prefer the environment: arguments show in process lists.
    /// Keys travel in clear text over plain TCP: across a network you do not trust, terminate
    /// TLS in front of the server (a proxy or a bridge) and let only it reach the port.
    #[arg(
        long = "seat",
        env = "BOTA_SEATS",
        value_delimiter = ',',
        hide_env_values = true
    )]
    seats: Vec<String>,
    /// The key a referee's connection presents (64 hex digits): told the lobby, the start,
    /// departures and the end, and no ticks. It travels in clear text, as the seat keys do.
    #[arg(long, env = "BOTA_REFEREE_KEY", hide_env_values = true)]
    referee_key: Option<String>,
    /// With reserved seats, end the lobby phase this many seconds after listening at the latest;
    /// a seat without a pick then gets a random hero.
    #[arg(long)]
    start_after_secs: Option<u64>,
    /// Most connections held at once.
    #[arg(long)]
    max_connections: Option<usize>,
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

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    if args.wire_fingerprint {
        println!(
            "{}",
            bota_proto::fingerprint_to_hex(bota_proto::WIRE_FINGERPRINT)
        );
        return Ok(());
    }
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
    let order_delays = order_delays(&args, players).map_err(std::io::Error::other)?;
    // A replay that cannot be written fails before the port is announced.
    if let Some(path) = &args.replay {
        bota_server::replay::ReplayWriter::create(path)?;
    }
    let listener = TcpListener::bind(("0.0.0.0", args.port))?;
    println!("bota_server listening on {}", listener.local_addr()?);
    run(
        listener,
        ServerOpts {
            mode: match args.mode {
                Mode::Realtime => bota_proto::TickMode::Realtime,
                Mode::Lockstep => bota_proto::TickMode::Lockstep,
            },
            tick_rate: args.tick_rate,
            players,
            replay: args.replay,
            seed: args.seed,
            match_id: args.match_id,
            reserved,
            referee_key,
            start_after: args.start_after_secs.map(Duration::from_secs),
            max_connections: args.max_connections,
            map: bota_proto::MapId(args.map),
            ack_timeout_ticks: args.ack_timeout_ticks,
            idle_timeout: (args.idle_timeout_secs > 0)
                .then(|| Duration::from_secs(args.idle_timeout_secs)),
            cheats: args.cheats,
            immortal_structures: args.immortal_structures,
            immortal_heroes: args.immortal_heroes,
            dummy_seat: args.dummy_seat,
            order_delays,
        },
    )
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

/// Seats from `key_hex:name` entries.
fn reservations(entries: &[String]) -> Result<Vec<ReservedSeat>, String> {
    entries
        .iter()
        .map(|entry| {
            let (key, name) = entry
                .split_once(':')
                .ok_or("a seat reservation reads key_hex:name")?;
            if name.is_empty() || name.len() > MAX_CLIENT_NAME_LEN {
                return Err(format!(
                    "a seat name takes 1 to {MAX_CLIENT_NAME_LEN} bytes"
                ));
            }
            Ok(ReservedSeat {
                key: SeatKey::from_hex(key).ok_or("a key is 64 hex digits")?,
                name: name.to_owned(),
            })
        })
        .collect()
}
