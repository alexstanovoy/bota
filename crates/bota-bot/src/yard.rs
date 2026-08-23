//! A server of its own for one match.
//!
//! The server plays one match and exits, so a match owns one: started on a
//! port the system picks, both seats joined in order, and killed with the
//! match whatever happened.
//!
//! Lockstep is what makes training possible at all. The server advances as
//! soon as every seat has acknowledged the tick, so a match runs as fast as
//! the two minds can think rather than at the pace a person would watch it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use bota_proto::HeroId;

use crate::{Chair, Link, Mind, play_on};

/// Where matches are played.
#[derive(Clone, Debug)]
pub struct Yard {
    /// The server to run.
    pub server: PathBuf,
    /// Which map.
    pub map: u16,
    /// Ticks a second, which in lockstep only sets the straggler timeout.
    pub tick_rate: u16,
    /// Tick-lengths to wait for a straggler.
    pub ack_timeout_ticks: u32,
    /// Which hero both seats ask for.
    pub hero: HeroId,
    /// Whether matches are played in this process rather than over a socket.
    ///
    /// The same game either way — the same seat loop reading the same messages
    /// — with the postcard, the socket and the second process taken out, which
    /// between them are most of what a match costs.
    pub builtin: bool,
}

impl Default for Yard {
    fn default() -> Yard {
        Yard {
            server: server_beside_us(),
            map: 0,
            tick_rate: 30,
            ack_timeout_ticks: 20,
            hero: HeroId(0),
            builtin: cfg!(feature = "builtin"),
        }
    }
}

/// The server binary built alongside this one.
pub fn server_beside_us() -> PathBuf {
    let named = if cfg!(windows) {
        "bota-server.exe"
    } else {
        "bota-server"
    };
    std::env::current_exe()
        .ok()
        .and_then(|me| me.parent().map(|dir| dir.join(named)))
        .unwrap_or_else(|| PathBuf::from(named))
}

/// A server running one match, killed when this is dropped.
struct Running {
    child: Child,
    addr: String,
    /// What it said went wrong, kept so a match that fails can say why.
    ///
    /// Thrown away, a server that panics reaches the caller as a connection
    /// that closed, which names neither the server nor the panic.
    grumbles: Arc<Mutex<Vec<String>>>,
}

impl Running {
    /// The last few things the server complained about.
    fn last_words(&self) -> String {
        let held = self.grumbles.lock().expect("grumbles lock");
        let from = held.len().saturating_sub(4);
        held[from..].join("; ")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Yard {
    /// Plays one match between two minds.
    ///
    /// Both seats are joined before either plays: seats go out in the order
    /// the server sees connections arrive, and which side a mind plays has to
    /// be the caller's decision rather than the scheduler's.
    pub fn play_a_match(
        &self,
        seed: u64,
        one: &mut (dyn Mind + Send),
        other: &mut (dyn Mind + Send),
        first: &Chair,
        second: &Chair,
    ) -> std::io::Result<(crate::Outcome, crate::Outcome)> {
        #[cfg(feature = "builtin")]
        if self.builtin {
            return self.play_here(seed, one, other, first, second);
        }
        let running = self.start(seed)?;
        let (link, seated) = Link::join(&running.addr, &first.name, first.hero)?;
        let (theirs, their_seat) = Link::join(&running.addr, &second.name, second.hero)?;
        let played = thread::scope(|scope| {
            let ours = scope.spawn(|| play_on(one, link, seated, first));
            let mine = play_on(other, theirs, their_seat, second);
            let yours = ours
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("a seat gave up")));
            (yours, mine)
        });
        let blame = |wrong: std::io::Error| -> std::io::Error {
            let said = running.last_words();
            if said.is_empty() {
                wrong
            } else {
                std::io::Error::other(format!("{wrong}; the server said: {said}"))
            }
        };
        Ok((played.0.map_err(blame)?, played.1.map_err(blame)?))
    }

    /// Plays one match in this process.
    ///
    /// Both seats are taken before either plays, as over the wire, and which
    /// side a mind plays stays the caller's decision.
    #[cfg(feature = "builtin")]
    fn play_here(
        &self,
        seed: u64,
        one: &mut (dyn Mind + Send),
        other: &mut (dyn Mind + Send),
        first: &Chair,
        second: &Chair,
    ) -> std::io::Result<(crate::Outcome, crate::Outcome)> {
        let (mut benches, seated) = crate::sit_down(seed, self.map, self.hero, 2);
        let mine = benches.pop().expect("a bench a seat");
        let ours = benches.pop().expect("a bench a seat");
        let played = thread::scope(|scope| {
            let theirs = scope.spawn(|| play_on(one, ours, seated[0], first));
            let here = play_on(other, mine, seated[1], second);
            let there = theirs
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("a seat gave up")));
            (there, here)
        });
        Ok((played.0?, played.1?))
    }

    /// Starts a server and waits until it says where it listens.
    fn start(&self, seed: u64) -> std::io::Result<Running> {
        if !Path::new(&self.server).exists() {
            return Err(std::io::Error::other(format!(
                "no server at {}",
                self.server.display()
            )));
        }
        let mut child = Command::new(&self.server)
            .arg("--port")
            .arg("0")
            .arg("--mode")
            .arg("lockstep")
            .arg("--players")
            .arg("2")
            .arg("--tick-rate")
            .arg(self.tick_rate.to_string())
            .arg("--map")
            .arg(self.map.to_string())
            .arg("--seed")
            .arg(seed.to_string())
            .arg("--ack-timeout-ticks")
            .arg(self.ack_timeout_ticks.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let grumbles: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        if let Some(moans) = child.stderr.take() {
            let kept = Arc::clone(&grumbles);
            thread::spawn(move || {
                for line in BufReader::new(moans).lines().map_while(Result::ok) {
                    kept.lock().expect("grumbles lock").push(line);
                }
            });
        }
        let Some(out) = child.stdout.take() else {
            return Err(std::io::Error::other("the server said nothing"));
        };
        let mut talking = BufReader::new(out);
        let mut line = String::new();
        talking.read_line(&mut line)?;
        let Some(port) = line.rsplit(':').next().map(str::trim) else {
            return Err(std::io::Error::other("the server named no port"));
        };
        if port.parse::<u16>().is_err() {
            return Err(std::io::Error::other(format!(
                "the server said {line:?}, which names no port"
            )));
        }
        // The rest of what it says is read and dropped rather than left
        // unread: a pipe nobody is reading fills, and the server stops on the
        // write that fills it.
        thread::spawn(move || {
            let mut rest = String::new();
            while let Ok(read) = talking.read_line(&mut rest) {
                if read == 0 {
                    break;
                }
                rest.clear();
            }
        });
        Ok(Running {
            child,
            addr: format!("127.0.0.1:{port}"),
            grumbles,
        })
    }
}
