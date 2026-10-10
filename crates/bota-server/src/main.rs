//! Command line entry point of the server.

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};

use bota_server::game::{OrderDelay, check_order_delays};
use bota_server::game_loop::{ServerOpts, run};

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
    #[arg(long, default_value_t = 30)]
    tick_rate: u16,
    /// Seats in the match.
    #[arg(long, default_value_t = 2)]
    players: u8,
    /// Write a replay to this file.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Which map to play: 0 Dota, 1 hero demo, 2 mid-only Dota (two lives or first tower, fifteen game minutes).
    #[arg(long, default_value_t = 0, value_parser = parse_map)]
    map: u16,
    /// Seed of the match randomness. A fresh one is drawn when absent.
    #[arg(long)]
    seed: Option<u64>,
    /// Lockstep: tick-lengths to wait for a straggler before moving on.
    #[arg(long, default_value_t = 150)]
    ack_timeout_ticks: u32,
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
    let order_delays = order_delays(&args).map_err(std::io::Error::other)?;
    let listener = TcpListener::bind(("0.0.0.0", args.port))?;
    println!("bota-server listening on {}", listener.local_addr()?);
    let seed = args.seed.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock sits after 1970")
            .as_nanos() as u64
    });
    run(
        listener,
        ServerOpts {
            mode: match args.mode {
                Mode::Realtime => bota_proto::TickMode::Realtime,
                Mode::Lockstep => bota_proto::TickMode::Lockstep,
            },
            tick_rate: args.tick_rate,
            players: args.players,
            replay: args.replay,
            seed,
            map: bota_proto::MapId(args.map),
            ack_timeout_ticks: args.ack_timeout_ticks,
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
        .filter(|id| bota_server::game::MAPS.iter().any(|map| map.id.0 == *id))
        .ok_or_else(|| {
            format!("unsupported map '{value}'; expected 0 (Dota), 1 (demo), or 2 (mid-only Dota)")
        })
}

/// One order delay per seat from the two lists, each holding one value for
/// every seat or one per seat.
fn order_delays(args: &Args) -> Result<Vec<OrderDelay>, String> {
    let seats = usize::from(args.players);
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
