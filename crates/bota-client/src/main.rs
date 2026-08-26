//! bota client: rendering, input, spectating and replays.
//!
//! The client draws what the server sends and sends back intents. It holds no
//! `World` and cannot: the simulation lives in a crate this one does not
//! depend on.

mod camera;
mod catalog;
mod hud;
mod icons;
mod input;
mod net;
mod render;
mod replay_play;
mod slots;
mod state;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use bota_proto::{ClientMsg, Role};
use clap::Parser;
use macroquad::prelude::{Conf, next_frame};

use crate::net::Net;
use crate::replay_play::ReplayPlayer;
use crate::state::{App, Source};

/// bota client: rendering, input, spectating and replays.
#[derive(Parser, Clone, Debug, PartialEq, Eq)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Where the server listens.
    #[arg(long, default_value = "127.0.0.1:4455")]
    pub addr: String,
    /// What the lobby shows.
    #[arg(long, default_value = "player")]
    pub name: String,
    /// Watch instead of playing.
    #[arg(long)]
    pub spectate: bool,
    /// Play this file instead of connecting anywhere.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["addr", "spectate"])]
    pub replay: Option<PathBuf>,
}

fn conf() -> Conf {
    Conf {
        window_title: "bota".to_string(),
        window_width: 1280,
        window_height: 800,
        ..Default::default()
    }
}

#[macroquad::main(conf)]
async fn main() {
    let args = Args::parse();
    let mut app = match build_app(&args) {
        Ok(app) => app,
        Err(err) => {
            eprintln!("bota-client: {err}");
            return;
        }
    };
    loop {
        let dt = macroquad::prelude::get_frame_time();
        let msgs = app.source.poll(dt);
        for msg in msgs {
            app.handle(msg);
        }
        for (tick, orders) in app.source.take_orders() {
            for given in orders {
                app.note_order(given.slot, given.unit, tick, given.order);
            }
        }
        input::handle(&mut app);
        app.check_connection();
        app.tick_effects(dt);
        render::draw(&app);
        if app.quit {
            return;
        }
        next_frame().await;
    }
}

fn build_app(args: &Args) -> std::io::Result<App> {
    if let Some(path) = &args.replay {
        let player = ReplayPlayer::load(path)?;
        return Ok(App::new(Source::Replay(player)));
    }
    let mut net = Net::connect(&args.addr)?;
    let role = if args.spectate {
        Role::Spectator
    } else {
        Role::Player
    };
    net.send(&ClientMsg::Hello {
        role,
        name: args.name.clone(),
    });
    Ok(App::new(Source::Live(net)))
}
