//! The server: lobby phase, then the tick loop in either mode.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, EventKind, MapId, PlayerId, ReplayRecord, Role, ServerMsg, SlotId, SlotOrder, Team,
    TickMode, WorldView, encode_frame_to_vec,
};

use crate::game::{
    Command, DelayLine, Event, EventVisibility, MatchConfig, OrderDelay, RefusedOrder, World,
};
use crate::lobby::Roster;
use crate::net::{Connection, NetEvent, accept_loop};
use crate::replay::ReplayWriter;

/// Everything the command line decides.
#[derive(Clone, Debug)]
pub struct ServerOpts {
    /// How ticks advance.
    pub mode: TickMode,
    /// Ticks per wall-clock second.
    pub tick_rate: u16,
    /// Seats in the match.
    pub players: u8,
    /// Where to write the replay. Absent records nothing.
    pub replay: Option<PathBuf>,
    /// Seed of the match randomness and the match id.
    pub seed: u64,
    /// Which map to play.
    pub map: MapId,
    /// Whether cheat orders are honoured.
    pub cheats: bool,
    /// Whether structures keep at least 1 hp and never fall.
    pub immortal_structures: bool,
    /// Whether heroes keep at least 1 hp and never die.
    pub immortal_heroes: bool,
    /// Whether the last seat is a dummy: a hero that stays put and never acts.
    pub dummy_seat: bool,
    /// Lockstep: how many tick-lengths to wait for a straggler.
    pub ack_timeout_ticks: u32,
    /// How late the seats' orders take effect: nothing for no delay, one for
    /// every seat, or one per seat in slot order.
    pub order_delays: Vec<OrderDelay>,
}

/// Runs one match on an already-bound listener and returns when it ends.
pub fn run(listener: TcpListener, opts: ServerOpts) -> std::io::Result<()> {
    let (tx, rx) = channel();
    let accept_tx = tx.clone();
    thread::spawn(move || accept_loop(listener, accept_tx));
    let roster = Roster::new(opts.players, opts.dummy_seat);
    let mut server = Server {
        opts,
        rx,
        tx,
        conns: Vec::new(),
        spectators: Vec::new(),
        roster,
        next_player: 1,
    };
    let cfg = server.lobby_phase();
    server.match_phase(cfg);
    Ok(())
}

struct Server {
    opts: ServerOpts,
    rx: Receiver<NetEvent>,
    /// Cloned into every connection's reader thread; keeps `rx` open.
    tx: Sender<NetEvent>,
    conns: Vec<Connection>,
    /// Connections that said Hello as spectators, and the seat whose eyes
    /// each one watches through. Absent eyes watch everything.
    spectators: Vec<(PlayerId, Option<SlotId>)>,
    roster: Roster,
    next_player: u32,
}

/// What the seats sent during the match.
struct Inbox {
    /// The valid orders of each seat on their way.
    line: DelayLine,
    /// The latest tick each seat acknowledged, indexed by slot.
    acked: Vec<u32>,
}

/// The wall-clock timing of the tick loop.
struct Pace {
    /// When the first tick began.
    started: Instant,
    /// One tick of wall-clock time.
    period: Duration,
    /// Lockstep: how long to wait for a straggler's acknowledgement.
    ack_timeout: Duration,
}

impl Server {
    fn conn(&self, id: PlayerId) -> Option<&Connection> {
        self.conns.iter().find(|c| c.id == id)
    }

    fn broadcast(&self, msg: &ServerMsg) {
        for conn in &self.conns {
            conn.send(msg);
        }
    }

    fn broadcast_lobby(&self) {
        self.broadcast(&ServerMsg::LobbyState {
            slots: self.roster.lobby_slots(),
        });
    }

    fn drop_conn(&mut self, id: PlayerId) {
        if let Some(conn) = self.conn(id) {
            conn.close();
        }
        self.conns.retain(|c| c.id != id);
        self.spectators.retain(|&(p, _)| p != id);
    }

    /// Tells the operator of a peer that sent bytes no message decodes
    /// from. The peer is then treated as having left, so only its own
    /// connection closes; the protocol has no message to answer it with.
    fn note(&self, event: &NetEvent) {
        let NetEvent::Malformed(id, error) = event else {
            return;
        };
        let seat = self.roster.seat_of(*id).map_or_else(
            || "no seat".to_string(),
            |seat| format!("seat {}", seat.slot.0),
        );
        eprintln!(
            "bota-server: player {} ({seat}) sent an undecodable message ({error}); closing its connection",
            id.0
        );
    }

    /// Accepts a fresh stream and hands out a `PlayerId`.
    fn adopt(&mut self, stream: std::net::TcpStream) -> PlayerId {
        let id = PlayerId(self.next_player);
        self.next_player += 1;
        let conn = Connection::spawn(id, stream, self.tx.clone());
        self.conns.push(conn);
        id
    }

    /// A Hello: seats players and bots, registers spectators.
    ///
    /// Returns whether the peer was accepted. `info` carries the running
    /// match to a spectator joining after the start.
    fn greet(
        &mut self,
        id: PlayerId,
        role: Role,
        name: String,
        info: Option<&MatchConfig>,
    ) -> bool {
        if self.roster.seat_of(id).is_some() || self.spectators.iter().any(|&(p, _)| p == id) {
            return true; // a second Hello changes nothing
        }
        let (slot, accepted) = match role {
            Role::Spectator => {
                self.spectators.push((id, None));
                (None, true)
            }
            Role::Player | Role::Bot => {
                if info.is_some() {
                    (None, false) // seats are not handed out mid-match
                } else {
                    match self.roster.free_seat_mut() {
                        None => (None, false),
                        Some(seat) => {
                            seat.player = Some(id);
                            seat.name = name;
                            seat.role = Some(role);
                            seat.ready = false;
                            (Some(seat.slot), true)
                        }
                    }
                }
            }
        };
        if !accepted {
            self.drop_conn(id);
            return false;
        }
        if let Some(conn) = self.conn(id) {
            conn.send(&ServerMsg::Welcome {
                player_id: id,
                slot,
                tick_rate: self.opts.tick_rate,
                mode: self.opts.mode,
            });
            if let Some(cfg) = info {
                conn.send(&ServerMsg::MatchStart { info: cfg.info() });
            }
        }
        true
    }

    /// Collects seats and picks until everyone is ready.
    fn lobby_phase(&mut self) -> MatchConfig {
        loop {
            let event = self.rx.recv().expect("the server holds a sender");
            self.note(&event);
            match event {
                NetEvent::Connected(stream) => {
                    self.adopt(stream);
                }
                NetEvent::Disconnected(id) | NetEvent::Malformed(id, _) => {
                    self.roster.release(id);
                    self.drop_conn(id);
                    self.broadcast_lobby();
                }
                NetEvent::Msg(id, msg) => match msg {
                    ClientMsg::Hello { role, name } => {
                        if self.greet(id, role, name, None) {
                            self.broadcast_lobby();
                        }
                    }
                    ClientMsg::PickHero { hero } => {
                        // A pick of nobody leaves whatever was picked before.
                        if crate::game::hero_def(hero).is_some()
                            && let Some(seat) = self.roster.seat_of_mut(id)
                        {
                            seat.hero = Some(hero);
                            self.broadcast_lobby();
                        }
                    }
                    ClientMsg::SetReady(ready) => {
                        if let Some(seat) = self.roster.seat_of_mut(id) {
                            seat.ready = ready;
                            self.broadcast_lobby();
                            if self.roster.all_ready() {
                                return self.build_config();
                            }
                        }
                    }
                    ClientMsg::Order { .. } | ClientMsg::Ack { .. } | ClientMsg::ViewAs { .. } => {}
                },
            }
        }
    }

    fn build_config(&self) -> MatchConfig {
        let seed = self.opts.seed.to_le_bytes();
        MatchConfig {
            match_id: self.opts.seed,
            master_key: std::array::from_fn(|i| seed[i % seed.len()]),
            picks: self.roster.picks(),
            map: self.opts.map,
            tick_rate: self.opts.tick_rate,
            mode: self.opts.mode,
            ack_timeout_ticks: self.opts.ack_timeout_ticks,
            cheats: self.opts.cheats,
            immortal_structures: self.opts.immortal_structures,
            immortal_heroes: self.opts.immortal_heroes,
            spawn_modifiers: Vec::new(),
        }
    }

    /// Runs the match to its end.
    fn match_phase(&mut self, cfg: MatchConfig) {
        let mut world = World::for_match(&cfg, cfg.rng());
        let mut replay = match &self.opts.replay {
            None => ReplayWriter::disabled(),
            Some(path) => ReplayWriter::create(path).unwrap_or_else(|_| ReplayWriter::disabled()),
        };
        replay.record(&ReplayRecord::Msg(ServerMsg::MatchStart {
            info: cfg.info(),
        }));
        self.broadcast(&ServerMsg::MatchStart { info: cfg.info() });
        let period = Duration::from_nanos(1_000_000_000 / u64::from(self.opts.tick_rate.max(1)));
        let pace = Pace {
            started: Instant::now(),
            period,
            ack_timeout: period * self.opts.ack_timeout_ticks.max(1),
        };
        let seats = self.roster.seats.len();
        let mut inbox = Inbox {
            line: DelayLine::new(&cfg, &self.opts.order_delays)
                .expect("the command line checked the order delays"),
            acked: vec![0; seats],
        };
        let mut refused = Vec::new();
        loop {
            self.gather_input(&world, &cfg, &mut inbox, &pace);
            if self.roster.seats.iter().all(|s| s.player.is_none()) {
                break; // every seat is empty
            }
            let cmds: Vec<Command> = inbox.line.release(&world, &mut refused);
            self.refuse(&mut refused);
            let taken: Vec<SlotOrder> = cmds
                .iter()
                .map(|c| SlotOrder {
                    slot: c.slot,
                    unit: c.unit,
                    order: c.order,
                })
                .collect();
            replay.record(&ReplayRecord::Orders {
                tick: world.tick + 1,
                orders: taken.clone(),
            });
            let events = world.advance(&cmds);
            self.publish_tick(&world, &inbox.line, &events, &taken, &mut replay);
            if let Some(winner) = world.victor() {
                let over = ServerMsg::MatchOver {
                    winner,
                    stats: world.match_stats(),
                };
                replay.record(&ReplayRecord::Msg(over.clone()));
                self.broadcast(&over);
                break;
            }
        }
        replay.finish();
        for conn in &mut self.conns {
            conn.close_and_wait();
        }
    }

    /// Sends a finished tick to every connection, each through its own eyes,
    /// and records it.
    fn publish_tick(
        &self,
        world: &World,
        line: &DelayLine,
        events: &[Event],
        taken: &[SlotOrder],
        replay: &mut ReplayWriter,
    ) {
        let mut full = world.view_full();
        full.order_delays = line.views(None);
        let full_frame = snapshot_frame(full.clone());
        let side_frame = |team| {
            let mut view = world.view(team);
            view.order_delays = line.views(Some(team));
            snapshot_frame(view)
        };
        let radiant_frame = side_frame(Team::Radiant);
        let dire_frame = side_frame(Team::Dire);
        replay.record(&ReplayRecord::Msg(ServerMsg::Snapshot { view: full }));
        for conn in &self.conns {
            let (eyes, viewing) = self.sight_of(conn.id);
            let frame = match viewing {
                Some(Team::Radiant) => radiant_frame.clone(),
                Some(Team::Dire) => dire_frame.clone(),
                // Seats never sit on the jungle's side.
                Some(Team::Neutral) | None => full_frame.clone(),
            };
            conn.send_tick(frame, world.tick, visible_events(events, viewing));
            // A spectator watching through one seat's eyes is told that
            // seat's orders; nobody else is told any.
            if let Some(eyes) = eyes {
                let told: Vec<SlotOrder> = taken
                    .iter()
                    .filter(|given| given.slot == eyes)
                    .copied()
                    .collect();
                if !told.is_empty() {
                    conn.send(&ServerMsg::Orders {
                        tick: world.tick,
                        orders: told,
                    });
                }
            }
        }
        if !events.is_empty() {
            replay.record(&ReplayRecord::Msg(ServerMsg::Events {
                tick: world.tick,
                events: events.iter().map(|e| e.kind.clone()).collect(),
            }));
        }
    }

    /// The seat a connection borrows eyes from and the side it sees as.
    ///
    /// A seated connection sees as its own side and borrows nothing; a
    /// spectator sees as the side of the seat it chose. No side sees
    /// everything.
    fn sight_of(&self, id: PlayerId) -> (Option<SlotId>, Option<Team>) {
        if let Some(seat) = self.roster.seat_of(id) {
            return (None, Some(seat.team));
        }
        let eyes = self
            .spectators
            .iter()
            .find(|&&(p, _)| p == id)
            .and_then(|&(_, eyes)| eyes);
        let team = eyes.and_then(|slot| {
            self.roster
                .seats
                .iter()
                .find(|s| s.slot == slot)
                .map(|s| s.team)
        });
        (eyes, team)
    }

    /// Waits out the tick according to the mode, handling messages meanwhile.
    fn gather_input(&mut self, world: &World, cfg: &MatchConfig, inbox: &mut Inbox, pace: &Pace) {
        let deadline = match self.opts.mode {
            TickMode::Realtime => pace.started + pace.period * (world.tick + 1),
            TickMode::Lockstep => Instant::now() + pace.ack_timeout,
        };
        loop {
            if self.opts.mode == TickMode::Lockstep && self.all_acked(world.tick, &inbox.acked) {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let event = self.rx.recv_timeout(deadline - now);
            if let Ok(event) = &event {
                self.note(event);
            }
            match event {
                Err(_) => break,
                Ok(NetEvent::Connected(stream)) => {
                    self.adopt(stream);
                }
                Ok(NetEvent::Disconnected(id) | NetEvent::Malformed(id, _)) => {
                    let slot = self.roster.seat_of(id).map(|s| s.slot);
                    self.roster.release(id);
                    self.drop_conn(id);
                    self.broadcast(&ServerMsg::ParticipantLeft {
                        player_id: id,
                        slot,
                    });
                }
                Ok(NetEvent::Msg(id, msg)) => self.receive_in_match(id, msg, world, cfg, inbox),
            }
        }
    }

    /// One message that arrives while the match runs.
    fn receive_in_match(
        &mut self,
        id: PlayerId,
        msg: ClientMsg,
        world: &World,
        cfg: &MatchConfig,
        inbox: &mut Inbox,
    ) {
        match msg {
            ClientMsg::Hello { role, name } => {
                self.greet(id, role, name, Some(cfg));
            }
            ClientMsg::Order {
                seq,
                unit,
                order,
                seen,
            } => {
                let Some(slot) = self.roster.seat_of(id).map(|s| s.slot) else {
                    return;
                };
                if let Err(reason) = inbox.line.give(world, slot, seq, unit, order, seen)
                    && let Some(conn) = self.conn(id)
                {
                    conn.send(&ServerMsg::OrderRejected { seq, reason });
                }
            }
            ClientMsg::Ack { tick } => {
                if let Some(seat) = self.roster.seat_of(id) {
                    let i = usize::from(seat.slot.0);
                    inbox.acked[i] = inbox.acked[i].max(tick);
                }
            }
            ClientMsg::ViewAs { seat } => {
                // Only a seat that is in the match has eyes to lend.
                let eyes = seat.filter(|seat| self.roster.seats.iter().any(|s| s.slot == *seat));
                if let Some(spectator) = self.spectators.iter_mut().find(|(p, _)| *p == id) {
                    spectator.1 = eyes;
                }
            }
            ClientMsg::PickHero { .. } | ClientMsg::SetReady(_) => {}
        }
    }

    /// Tells each seat of its orders that no longer held when their tick came.
    fn refuse(&self, refused: &mut Vec<RefusedOrder>) {
        for refusal in refused.drain(..) {
            let holder = self
                .roster
                .seats
                .iter()
                .find(|seat| seat.slot == refusal.slot)
                .and_then(|seat| seat.player);
            if let Some(conn) = holder.and_then(|id| self.conn(id)) {
                conn.send(&ServerMsg::OrderRejected {
                    seq: refusal.seq,
                    reason: refusal.reason,
                });
            }
        }
    }

    /// Whether every connected seat has acknowledged this tick.
    fn all_acked(&self, tick: u32, acked: &[u32]) -> bool {
        self.roster
            .seats
            .iter()
            .filter(|s| s.player.is_some())
            .all(|s| acked[usize::from(s.slot.0)] >= tick)
    }
}

/// A snapshot as a frame ready to queue.
fn snapshot_frame(view: WorldView) -> Vec<u8> {
    encode_frame_to_vec(&ServerMsg::Snapshot { view }).expect("a view always encodes")
}

/// The kinds of the events a side may see. No side sees all of them, and a
/// death's gold is told only to the side that earned it unless a hero fell:
/// the other side sees zero, so a visible creep kill never leaks the
/// enemy's income.
pub(crate) fn visible_events(events: &[Event], viewing: Option<Team>) -> Vec<EventKind> {
    events
        .iter()
        .filter(|e| match (e.visible_to, viewing) {
            (_, None) | (EventVisibility::Everyone, _) => true,
            (EventVisibility::OneTeam(team), Some(mine)) => team == mine,
        })
        .map(|e| match (&e.kind, viewing) {
            (EventKind::Died { gold, .. }, Some(mine))
                if *gold != 0 && e.gold_hidden_from == Some(mine) =>
            {
                let mut kind = e.kind.clone();
                if let EventKind::Died { gold, .. } = &mut kind {
                    *gold = 0;
                }
                kind
            }
            _ => e.kind.clone(),
        })
        .collect()
}
