//! Command line entry point of the bot.

use bota_bot::{Chair, Playbook, Role, play};
use bota_proto::HeroId;
use clap::Parser;

/// bota bot: joins a server, takes a seat, and plays one match.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Where the server listens.
    #[arg(long, default_value = "127.0.0.1:4455")]
    addr: String,
    /// The name the lobby shows.
    #[arg(long, default_value = "bot")]
    name: String,
    /// Which hero to ask for: 0 Sylla, 1 Pudge, 2 Shadow Fiend.
    #[arg(long, default_value_t = 2)]
    hero: u16,
    /// What the seat is there to do, one to five.
    #[arg(long, default_value_t = 2)]
    role: u8,
    /// Leave after this many ticks.
    #[arg(long, value_name = "TICKS")]
    limit: Option<u32>,
    /// Write a line about what the seat is doing every so many ticks.
    #[arg(long, value_name = "TICKS", default_value_t = 0)]
    trace: u32,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let Some(role) = Role::of(args.role) else {
        return Err(std::io::Error::other(format!(
            "there is no role {}: they are numbered one to five",
            args.role
        )));
    };
    let hero = HeroId(args.hero);
    let chair = Chair {
        addr: args.addr,
        name: args.name,
        hero,
        limit: args.limit,
    };
    let mut playbook = Playbook::new(role, hero);
    playbook.trace_every(args.trace);
    let out = play(&mut playbook, &chair)?;
    let mine = out.mine.as_ref();
    println!(
        "played {} ticks as {:?}: {} last hits, {} denies, {} kills, {} deaths, level {}",
        out.ticks,
        out.team,
        mine.map_or(0, |row| row.last_hits),
        mine.map_or(0, |row| row.denies),
        mine.map_or(0, |row| row.kills),
        mine.map_or(0, |row| row.deaths),
        mine.map_or(0, |row| row.level),
    );
    match out.winner {
        Some(winner) => println!("{winner:?} won"),
        None => println!("the match did not end"),
    }
    println!("{} orders sent, {} refused", out.ordered, out.rejected);
    for (reason, many) in &out.refusals {
        println!("  {many} refused: {reason:?}");
    }
    Ok(())
}
