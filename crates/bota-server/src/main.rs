//! Command line entry point of the server.

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};

use bota_server::game_loop::{ServerOpts, run};

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
    /// Which map to play: 0 is the Dota map, 1 the small test lane, 2 the
    /// Dota map to a short finish — a tower or two deaths ends it.
    #[arg(long, default_value_t = 0)]
    map: u16,
    /// Seed of the match randomness. A fresh one is drawn when absent.
    #[arg(long)]
    seed: Option<u64>,
    /// Lockstep: tick-lengths to wait for a straggler before moving on.
    #[arg(long, default_value_t = 150)]
    ack_timeout_ticks: u32,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
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
        },
    )
}
