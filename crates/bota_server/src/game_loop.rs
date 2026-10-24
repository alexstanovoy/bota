//! The server: lobby phase, then the tick loop in either mode.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use bota_core::{WIRE_FINGERPRINT, rules};
use bota_proto::{
    ClientMsg, EventKind, Heard, HeroId, MapId, MatchReport, PlayerId, RejectReason, ReplayRecord,
    Role, SeatKey, ServerMsg, SlotId, SlotOrder, Team, TickMode, WorldView, encode_frame_to_vec,
};
use rand_chacha::rand_core::Rng as _;

use crate::chat::{ChatOpts, Rates};
use crate::game::{
    Command, DelayLine, Event, EventVisibility, Handles, MatchConfig, MatchSecrets, OrderDelay,
    RefusedOrder, World,
};
use crate::lobby::{ReservedSeat, Roster};
use crate::net::{Connection, FrameStream, Listeners, NetEvent};
use crate::pick_clock::PickClock;
use crate::replay::ReplayWriter;
use crate::spectators::{DelayedFeed, HeldTick, MAX_DELAYED_TICKS, Spectators, sight};

mod chat;
mod participants;

/// How long a connection may stay silent before it is dropped as dead, by
/// default: five pings missed. A seat is not handed to a new connection while
/// a live one holds it, so this is also how long a player whose line died
/// silently waits to take the seat back.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a connection has to say Hello. One that has not by then is let
/// go: a stranger answering pings would otherwise hold one of the server's
/// connections for ever.
pub const HELLO_DEADLINE: Duration = Duration::from_secs(10);

/// How long a lobby without a deadline of its own waits for a first seat to
/// be held before an unjoined match is abandoned.
pub const UNJOINED_AFTER: Duration = Duration::from_secs(60);

/// Spectators that may watch through one seat's key at once, by default.
pub const DEFAULT_SPECTATORS_PER_SEAT: usize = 7;

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
    /// Seed of the match randomness. Drawn from the operating system when
    /// absent, and then never known to anyone.
    pub seed: Option<u64>,
    /// The public label of the match. Absent, one drawn from the key (the same
    /// for the same seed); no randomness is ever derived from it.
    pub match_id: Option<u64>,
    /// Seats held back for key holders, in slot order. With none, seats go to
    /// whoever asks first.
    pub reserved: Vec<ReservedSeat>,
    /// What a referee's connection must present. Without it no referee is
    /// admitted.
    pub referee_key: Option<SeatKey>,
    /// With reserved seats, the lobby phase ends this long after the server
    /// starts listening at the latest; a seat still without a pick then gets a
    /// random hero.
    pub start_after: Option<Duration>,
    /// Most connections held at once. Absent for no bound.
    pub max_connections: Option<usize>,
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
    /// How long a connection may send nothing, or take nothing of what is
    /// sent, before it is dropped as dead. Absent waits for ever.
    pub idle_timeout: Option<Duration>,
    /// How late the seats' orders take effect: nothing for no delay, one for
    /// every seat, or one per seat in slot order.
    pub order_delays: Vec<OrderDelay>,
    /// Messages a connection may send a second, on top of a few for every
    /// tick it is sent (see [`Allowance`](crate::net::Allowance)); a peer that
    /// sends more is cut off.
    pub messages_per_sec: NonZeroU32,
    /// Spectators that may watch through one reserved seat's key at once.
    pub max_spectators_per_seat: usize,
    /// Who may watch without a seat's key.
    pub spectators: Spectators,
    /// The heroes a seat may pick, and a seat without a pick is given one
    /// of. Absent allows every hero.
    pub heroes: Option<Vec<HeroId>>,
    /// End the match unplayed when no seat is held as the lobby phase runs
    /// out of time: [`ServerOpts::start_after`], or [`UNJOINED_AFTER`]
    /// without it.
    pub abandon_unjoined: bool,
    /// End the match unplayed once every seat has been empty this long.
    pub empty_after: Option<Duration>,
    /// How long the seats have to pick once all are held; Dota's default is
    /// [`rules::PICK_SECS`](bota_core::rules::PICK_SECS). Past it a seat
    /// without a hero loses starting gold until it picks, and is given a
    /// random one after the penalty phase.
    pub pick_time: Duration,
    /// How long past the pick time a seat without a hero may lose gold before
    /// it is given one: [`rules::PICK_PENALTY_SECS`].
    pub pick_penalty_time: Duration,
    /// How long the tactical phase lasts once every seat has picked, when a
    /// human plays: Dota's [`rules::TACTICAL_SECS`](bota_core::rules::TACTICAL_SECS).
    pub tactical_time: Duration,
    /// Whether two seats may pick the same hero (Dota's 1v1 Solo Mid and Mid
    /// Only allow it). When not, a hero some seat holds is refused to the
    /// others, and a drawn hero is never one that is held.
    pub same_heroes: bool,
    /// Who may talk, and what the replay keeps of it.
    pub chat: ChatOpts,
}

/// The longest pick time that can be asked for.
pub const MAX_PICK_TIME: Duration = Duration::from_secs(24 * 60 * 60);

/// How a match ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchEnd {
    /// It was played to a winner or a draw ([`Team::Neutral`]).
    Over { winner: Team, report: MatchReport },
    /// Nobody played it: no seat was held when the lobby ran out of time, or
    /// every seat was empty for good or for longer than allowed. There is no
    /// result.
    Abandoned,
}

/// Runs one match on already-bound listeners and returns how it ended.
/// Fails before the lobby opens when the options contradict themselves, the
/// replay file cannot be created or the listeners cannot be started.
pub fn run(listeners: impl Into<Listeners>, opts: ServerOpts) -> std::io::Result<MatchEnd> {
    check_opts(&opts).map_err(std::io::Error::other)?;
    let secrets = draw_secrets(opts.seed, opts.match_id)?;
    let replay = match &opts.replay {
        None => ReplayWriter::disabled(),
        Some(path) => ReplayWriter::create(path)?,
    };
    let (tx, rx) = channel();
    listeners.into().start(&tx)?;
    let roster = match opts.reserved.len() {
        0 => Roster::new(opts.players, opts.dummy_seat),
        _ => Roster::reserved(&opts.reserved),
    };
    let delayed = opts
        .spectators
        .delay_ticks(opts.tick_rate)
        .map(DelayedFeed::new);
    if opts.spectators == Spectators::Open {
        eprintln!(
            "bota_server: --spectators open: anyone may watch the whole map live; for local use only"
        );
    }
    let mut server = Server {
        rates: Rates::new(Instant::now()),
        heard: Vec::new(),
        replay,
        opts,
        secrets,
        rx,
        tx,
        conns: Vec::new(),
        spectators: Vec::new(),
        referees: Vec::new(),
        roster,
        next_player: 1,
        told: participants::Told::default(),
        started: Instant::now(),
        delayed,
        pick_clock: PickClock::Waiting,
    };
    let Some(cfg) = server.lobby_phase() else {
        server.close_all();
        return Ok(MatchEnd::Abandoned);
    };
    Ok(server.match_phase(cfg))
}

/// What the options must agree on beyond what their types hold; [`run`]
/// checks it before anything else.
pub fn check_opts(opts: &ServerOpts) -> Result<(), String> {
    if opts.reserved.len() > usize::from(u8::MAX) {
        return Err("at most 255 reserved seats".to_owned());
    }
    if opts.pick_time > MAX_PICK_TIME
        || opts.pick_penalty_time > MAX_PICK_TIME
        || opts.tactical_time > MAX_PICK_TIME
    {
        return Err(format!(
            "the pick and tactical times are at most {} s",
            MAX_PICK_TIME.as_secs()
        ));
    }
    let seats = match opts.reserved.len() {
        0 => usize::from(opts.players),
        n => n,
    };
    let offered = opts
        .heroes
        .as_ref()
        .map_or(bota_core::HEROES.len(), Vec::len);
    if !opts.same_heroes && offered < seats {
        return Err(format!(
            "{seats} seats cannot each pick another hero out of {offered}"
        ));
    }
    if let Some(heroes) = &opts.heroes {
        if heroes.is_empty() {
            return Err("the allowed heroes name none".to_owned());
        }
        if let Some(hero) = heroes.iter().find(|h| bota_core::hero_def(**h).is_none()) {
            return Err(format!("hero {} does not exist", hero.0));
        }
    }
    match opts.spectators.delay_ticks(opts.tick_rate) {
        Some(ticks) if ticks > MAX_DELAYED_TICKS => Err(format!(
            "spectators {} at {} ticks a second holds back more than {MAX_DELAYED_TICKS} ticks",
            opts.spectators, opts.tick_rate
        )),
        _ => Ok(()),
    }
}

/// The secrets of the match: from the seed when one was given, else from a
/// key drawn from the operating system; the label replaced by the one given.
fn draw_secrets(seed: Option<u64>, match_id: Option<u64>) -> std::io::Result<MatchSecrets> {
    let mut secrets = match seed {
        Some(seed) => MatchSecrets::seeded(seed),
        None => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|error| std::io::Error::other(error.to_string()))?;
            MatchSecrets::of_key(key)
        }
    };
    if let Some(match_id) = match_id {
        secrets.match_id = match_id;
    }
    Ok(secrets)
}

/// A connection that said Hello as a spectator.
struct Watcher {
    id: PlayerId,
    /// The seat whose eyes it watches through. Absent watches everything.
    eyes: Option<SlotId>,
    /// Whether the key it came with fixed its eyes: then it watches live and
    /// is told that seat's orders.
    locked: bool,
    /// Whether it watches from the delayed feed.
    late: bool,
    /// The name it announced, as a seat's is shown.
    name: String,
}

/// How a spectator may watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Watch {
    /// Through the eyes of the seat its key opens, live, with its orders.
    Seat(SlotId),
    /// Live, without orders ([`Spectators::Open`]).
    Live,
    /// From the delayed feed ([`Spectators::Delayed`]).
    Late,
}

/// What a Hello entitles its connection to.
enum Admission {
    /// The first open seat.
    AnySeat,
    /// A seat reserved for the key.
    Seat(SlotId),
    /// A spectator.
    Watch(Watch),
    Referee,
}

struct Server {
    opts: ServerOpts,
    secrets: MatchSecrets,
    started: Instant,
    referees: Vec<PlayerId>,
    rx: Receiver<NetEvent>,
    /// Cloned into every connection's reader thread; keeps `rx` open.
    tx: Sender<NetEvent>,
    conns: Vec<Connection>,
    spectators: Vec<Watcher>,
    roster: Roster,
    next_player: u32,
    /// What each greeted connection is called on the wire.
    told: participants::Told,
    /// The ticks held back for late spectators, when they may watch.
    delayed: Option<DelayedFeed>,
    /// The hero selection clock; runs in the lobby phase only.
    pick_clock: PickClock,
    /// What each connection may still say.
    rates: Rates,
    /// The lines the replay is yet to be given, in the order taken in.
    heard: Vec<Heard>,
    /// Where the lobby's lines go; the match takes it over.
    replay: ReplayWriter,
}

/// What the seats sent during the match.
struct Inbox {
    /// The valid orders of each seat on their way.
    line: DelayLine,
    /// The latest tick each seat acknowledged, indexed by slot.
    acked: Vec<u32>,
    /// What each side calls the entities it is told of.
    handles: Handles,
}

/// What a finished tick tells beyond the world itself.
struct Told<'a> {
    line: &'a DelayLine,
    handles: &'a mut Handles,
    events: &'a [Event],
    taken: &'a [SlotOrder],
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

    /// Tells every connection that has said Hello.
    fn broadcast(&self, msg: &ServerMsg) {
        for conn in self.conns.iter().filter(|c| self.greeted(c.id)) {
            conn.send(msg);
        }
    }

    fn broadcast_lobby(&self) {
        self.broadcast(&self.lobby_state());
    }

    fn lobby_state(&self) -> ServerMsg {
        ServerMsg::LobbyState {
            slots: self.roster.lobby_slots(),
            same_heroes: self.opts.same_heroes,
            pick: self.pick_clock.phase(Instant::now()),
        }
    }

    /// Ends a connection the server is done with: what is queued still goes
    /// out before it hangs up.
    fn drop_conn(&mut self, id: PlayerId) {
        if let Some(conn) = self.forget(id) {
            conn.close();
        }
    }

    /// Ends the connection of a peer that is gone or hopeless.
    fn lose_conn(&mut self, id: PlayerId) {
        if let Some(conn) = self.forget(id) {
            conn.abort();
        }
    }

    /// Takes a connection out of every list.
    fn forget(&mut self, id: PlayerId) -> Option<Connection> {
        self.rates.forget(id);
        self.told.take(id);
        self.spectators.retain(|w| w.id != id);
        self.referees.retain(|&p| p != id);
        let at = self.conns.iter().position(|c| c.id == id)?;
        Some(self.conns.remove(at))
    }

    /// Whether a Hello was accepted from this connection.
    fn greeted(&self, id: PlayerId) -> bool {
        self.roster.seat_of(id).is_some()
            || self.spectators.iter().any(|w| w.id == id)
            || self.referees.contains(&id)
    }

    /// Takes in what the network says: adopts a new stream, lets a departed
    /// peer go and tells the others (the lobby as it now stands, or in a
    /// match who left), and hands a message back. A peer that broke the
    /// protocol is told of to the operator and closed: the protocol has no
    /// message to answer it with, and dialing again would not mend it. A peer gone otherwise is reset. What a connection already
    /// dropped still said is not heard.
    fn hear(&mut self, event: NetEvent, in_match: bool) -> Option<(PlayerId, ClientMsg)> {
        let (id, broken) = match event {
            NetEvent::Msg(id, msg) => return self.conn(id).is_some().then_some((id, msg)),
            NetEvent::Connected(stream) => {
                self.adopt(stream);
                return None;
            }
            NetEvent::Disconnected(id) => (id, true),
            NetEvent::Breached(id, breach) => {
                let seat = self.roster.seat_of(id).map_or_else(
                    || "no seat".to_string(),
                    |seat| format!("seat {}", seat.slot.0),
                );
                eprintln!(
                    "bota_server: player {} ({seat}) {breach}; closing its connection",
                    id.0
                );
                (id, false)
            }
        };
        // A connection already refused or dropped was told of then; one that
        // never said hello is nobody's business.
        if self.conn(id).is_some() {
            let slot = self.roster.seat_of(id).map(|s| s.slot);
            let told = self.told.take(id);
            self.roster.release(id);
            if broken {
                self.lose_conn(id);
            } else {
                self.drop_conn(id);
            }
            if let Some(told) = told {
                self.tell_departure(told, slot, in_match);
            }
        }
        None
    }

    /// Lets go of every connection that has not said Hello within
    /// [`HELLO_DEADLINE`].
    fn drop_strangers(&mut self) {
        let late: Vec<PlayerId> = self
            .conns
            .iter()
            .filter(|c| c.opened.elapsed() >= HELLO_DEADLINE && !self.greeted(c.id))
            .map(|c| c.id)
            .collect();
        for id in late {
            self.drop_conn(id);
        }
    }

    /// When the next connection still without a Hello runs out of time.
    fn next_stranger_deadline(&self) -> Option<Instant> {
        self.conns
            .iter()
            .filter(|c| !self.greeted(c.id))
            .map(|c| c.opened + HELLO_DEADLINE)
            .min()
    }

    /// Accepts a fresh stream and hands out a `PlayerId`, unless the server
    /// holds as many connections as it may.
    fn adopt(&mut self, stream: Box<dyn FrameStream>) {
        if self
            .opts
            .max_connections
            .is_some_and(|most| self.conns.len() >= most)
        {
            return;
        }
        let id = PlayerId(self.next_player);
        self.next_player += 1;
        let spawned = Connection::spawn(
            id,
            stream,
            self.tx.clone(),
            self.opts.idle_timeout,
            self.opts.messages_per_sec,
        );
        match spawned {
            Ok(conn) => self.conns.push(conn),
            Err(error) => eprintln!("bota_server: could not take a connection ({error})"),
        }
    }

    /// What a Hello may have: with reserved seats no seat without the key for
    /// it, and a referee only ever with the referee key.
    fn admit(&self, role: Role, key: Option<&SeatKey>) -> Option<Admission> {
        match role {
            Role::Referee => {
                let wanted = self.opts.referee_key.as_ref()?;
                (key? == wanted).then_some(Admission::Referee)
            }
            Role::Spectator => self.admit_spectator(key).map(Admission::Watch),
            Role::Player | Role::Bot if self.opts.reserved.is_empty() => Some(Admission::AnySeat),
            Role::Player | Role::Bot => {
                let slot = self.roster.seat_for_key(&self.opts.reserved, key?)?;
                let kind = self.opts.reserved[usize::from(slot.0)].kind;
                // The reservation knows what the account is; a client that says
                // otherwise would bend the tactical phase rule, so it is refused.
                kind.is_none_or(|kind| kind == role)
                    .then_some(Admission::Seat(slot))
            }
        }
    }

    /// How a spectator may watch: through the seat its key opens (reserved,
    /// or claimed on a server without reservations), else as
    /// [`ServerOpts::spectators`] lets anyone. With reserved seats a key that
    /// opens none admits nothing.
    fn admit_spectator(&self, key: Option<&SeatKey>) -> Option<Watch> {
        let reserved = !self.opts.reserved.is_empty();
        let seat = key.and_then(|key| match reserved {
            true => self.roster.seat_for_key(&self.opts.reserved, key),
            false => self.roster.seat_for_claim(key),
        });
        if let Some(slot) = seat {
            return Some(Watch::Seat(slot));
        }
        if reserved && key.is_some() {
            return None;
        }
        match self.opts.spectators {
            Spectators::Off => None,
            Spectators::Open => Some(Watch::Live),
            Spectators::Delayed { .. } => Some(Watch::Late),
        }
    }

    /// Tells a peer built with another wire so, and the operator too, and
    /// closes its connection.
    fn refuse_wire(&mut self, id: PlayerId, theirs: u64) {
        eprintln!(
            "bota_server: player {} speaks wire {theirs:016x}, this server {WIRE_FINGERPRINT:016x}; closing its connection",
            id.0
        );
        if let Some(conn) = self.conn(id) {
            conn.send(&ServerMsg::WireMismatch {
                server_fingerprint: WIRE_FINGERPRINT,
            });
        }
        self.drop_conn(id);
    }

    /// A Hello: seats players and bots, registers spectators and referees.
    ///
    /// Returns whether the peer was accepted; one that was not is told why
    /// ([`ServerMsg::Refused`]) and closed. `info` carries the running match
    /// to a connection joining after the start.
    fn greet(
        &mut self,
        id: PlayerId,
        role: Role,
        name: String,
        key: Option<SeatKey>,
        wire_fingerprint: u64,
        info: Option<&MatchConfig>,
    ) -> bool {
        if self.greeted(id) {
            return true; // a second Hello changes nothing
        }
        if wire_fingerprint != WIRE_FINGERPRINT {
            self.refuse_wire(id, wire_fingerprint);
            return false;
        }
        let admitted = match self.admit(role, key.as_ref()) {
            None => Err(RejectReason::NotAdmitted),
            Some(Admission::AnySeat) => {
                self.any_seat(id, role, name, key, info.is_some()).map(Some)
            }
            Some(Admission::Seat(slot)) => {
                let name = self.opts.reserved[usize::from(slot.0)].name.clone();
                self.take_seat(id, role, slot, name).map(|()| Some(slot))
            }
            Some(Admission::Watch(watch)) => self.watch(id, watch, &name).map(|()| None),
            Some(Admission::Referee) => {
                self.referees.push(id);
                Ok(None)
            }
        };
        let slot = match admitted {
            Ok(slot) => slot,
            Err(reason) => {
                if let Some(conn) = self.conn(id) {
                    conn.send(&ServerMsg::Refused { reason });
                }
                self.drop_conn(id);
                return false;
            }
        };
        let told = self.told.give(id, slot.is_some());
        if let Some(conn) = self.conn(id) {
            conn.send(&ServerMsg::Welcome {
                player_id: told,
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

    /// An unreserved seat: the one the key claimed before, else in the lobby
    /// the first open one, which the key, if any, then claims. Seats are not
    /// handed out mid-match to whoever asks.
    fn any_seat(
        &mut self,
        id: PlayerId,
        role: Role,
        name: String,
        key: Option<SeatKey>,
        started: bool,
    ) -> Result<SlotId, RejectReason> {
        if let Some(slot) = key.as_ref().and_then(|key| self.roster.seat_for_claim(key)) {
            return self.take_seat(id, role, slot, name).map(|()| slot);
        }
        if started {
            return Err(RejectReason::NotAdmitted);
        }
        let seat = self
            .roster
            .free_seat_mut()
            .ok_or(RejectReason::NotAdmitted)?;
        seat.hold(id, name, role);
        seat.claim = key;
        Ok(seat.slot)
    }

    /// Registers a spectator, unless the seat its key opens already admitted
    /// as many as it may.
    fn watch(&mut self, id: PlayerId, watch: Watch, name: &str) -> Result<(), RejectReason> {
        let eyes = match watch {
            Watch::Seat(slot) => Some(slot),
            Watch::Live | Watch::Late => None,
        };
        if let Some(slot) = eyes {
            let watching = self
                .spectators
                .iter()
                .filter(|w| w.locked && w.eyes == Some(slot))
                .count();
            if watching >= self.opts.max_spectators_per_seat {
                return Err(RejectReason::NotAdmitted);
            }
        }
        self.spectators.push(Watcher {
            id,
            eyes,
            locked: eyes.is_some(),
            late: watch == Watch::Late,
            name: bota_core::clean_name(name),
        });
        Ok(())
    }

    /// Seats a connection in a seat its key entitles it to, unless a live
    /// connection holds it: one that ended, or was found dead, let it go.
    fn take_seat(
        &mut self,
        id: PlayerId,
        role: Role,
        slot: SlotId,
        name: String,
    ) -> Result<(), RejectReason> {
        let seat = &mut self.roster.seats[usize::from(slot.0)];
        if seat.player.is_some() {
            return Err(RejectReason::SeatHeld);
        }
        seat.hold(id, name, role);
        Ok(())
    }

    /// Collects seats and picks until everyone is ready, or until the time
    /// set for it runs out. Nothing when the time ran out with no seat held
    /// and such a match is to be abandoned.
    fn lobby_phase(&mut self) -> Option<MatchConfig> {
        // A wait too long to reach is no deadline at all.
        let started = self.started;
        let start_by = self
            .opts
            .start_after
            .and_then(|after| started.checked_add(after));
        let mut abandon_by = (self.opts.abandon_unjoined)
            .then(|| self.opts.start_after.unwrap_or(UNJOINED_AFTER))
            .and_then(|after| started.checked_add(after));
        loop {
            if self.run_pick_clock(Instant::now()) {
                return Some(self.build_config());
            }
            let deadline = start_by
                .into_iter()
                .chain(abandon_by)
                .chain(self.pick_clock.due())
                .min();
            let Some(event) = self.lobby_event(deadline) else {
                let now = Instant::now();
                if abandon_by.is_some_and(|by| now >= by) {
                    if !self.roster.anyone_seated() {
                        eprintln!("bota_server: no seat was held when the lobby ran out of time");
                        return None;
                    }
                    abandon_by = None;
                }
                if start_by.is_some_and(|by| now >= by) {
                    self.draw_missing_picks(self.pick_clock.late_secs(now));
                    return Some(self.build_config());
                }
                continue;
            };
            if let Some((id, msg)) = self.hear(event, false) {
                self.lobby_message(id, msg);
                let mut replay = std::mem::replace(&mut self.replay, ReplayWriter::disabled());
                self.record_heard(&mut replay);
                self.replay = replay;
            }
        }
    }

    /// One message in the lobby.
    fn lobby_message(&mut self, id: PlayerId, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello {
                role,
                name,
                key,
                wire_fingerprint,
            } => {
                if self.greet(id, role, name, key, wire_fingerprint, None) {
                    self.lobby_after_hello(id);
                }
            }
            ClientMsg::PickHero { hero } => {
                // A pick of nobody, of a hero not allowed or after the pick
                // was locked leaves whatever was picked before.
                let allowed = self.allows(hero)
                    && (self.opts.same_heroes || !self.roster.taken_by_other(id, hero));
                let now = Instant::now();
                let clock = self.pick_clock;
                if let Some(seat) = self.roster.seat_of_mut(id)
                    && allowed
                    && !seat.pick_locked
                {
                    if seat.hero.is_none() {
                        seat.late_secs = clock.late_secs(now);
                    }
                    seat.hero = Some(hero);
                    seat.pick_locked = seat.ready || clock.is_past_selection();
                    self.broadcast_lobby();
                }
            }
            ClientMsg::SetReady(ready) => {
                if let Some(seat) = self.roster.seat_of_mut(id) {
                    seat.ready = ready;
                    seat.pick_locked |= ready && seat.hero.is_some();
                    self.broadcast_lobby();
                }
            }
            ClientMsg::Say(line) => self.say(id, line, None),
            ClientMsg::Compress(_) => self.compress(id),
            ClientMsg::Order { .. }
            | ClientMsg::Ack { .. }
            | ClientMsg::ViewAs { .. }
            | ClientMsg::Pong => {}
        }
    }

    /// Whether a seat may pick this hero.
    fn allows(&self, hero: HeroId) -> bool {
        bota_core::hero_def(hero).is_some()
            && self
                .opts
                .heroes
                .as_ref()
                .is_none_or(|heroes| heroes.contains(&hero))
    }

    /// The next thing the network says in the lobby, or nothing once the
    /// lobby's time is up. Connections that never said Hello are let go
    /// meanwhile.
    fn lobby_event(&mut self, deadline: Option<Instant>) -> Option<NetEvent> {
        loop {
            self.drop_strangers();
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return None;
            }
            let wake = deadline
                .into_iter()
                .chain(self.next_stranger_deadline())
                .min();
            let Some(wake) = wake else {
                return self.rx.recv().ok();
            };
            match self
                .rx
                .recv_timeout(wake.saturating_duration_since(Instant::now()))
            {
                Ok(event) => return Some(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Starts, advances and ends the clock before the match as the time
    /// passes and the seats lock in; true once the match is to begin.
    fn run_pick_clock(&mut self, now: Instant) -> bool {
        let before = self.pick_clock;
        let start = loop {
            match self.pick_clock {
                PickClock::Waiting if self.roster.all_held() => {
                    self.pick_clock = PickClock::start(now, self.opts.pick_time);
                }
                PickClock::Selecting { .. } | PickClock::Late { .. }
                    if self.roster.all_locked_in() =>
                {
                    self.begin_tactical(now);
                }
                PickClock::Selecting { ends } if now >= ends => {
                    for seat in &mut self.roster.seats {
                        seat.pick_locked |= seat.hero.is_some();
                    }
                    if self.roster.all_picked() {
                        self.begin_tactical(now);
                    } else {
                        self.pick_clock = PickClock::run_out(ends, self.opts.pick_penalty_time);
                    }
                }
                PickClock::Late { ends, .. } if now >= ends => {
                    let late = self
                        .pick_clock
                        .late_secs(self.pick_clock.due().unwrap_or(now));
                    self.draw_missing_picks(late);
                    self.begin_tactical(now);
                }
                PickClock::Tactical { ends } if now >= ends || self.roster.only_bots() => {
                    break true;
                }
                _ => break false,
            }
        };
        if self.pick_clock != before && !start {
            self.broadcast_lobby();
        }
        start
    }

    /// Every pick is final: the tactical phase begins.
    fn begin_tactical(&mut self, now: Instant) {
        for seat in &mut self.roster.seats {
            seat.pick_locked = true;
        }
        self.pick_clock = PickClock::Tactical {
            ends: now + self.opts.tactical_time,
        };
    }

    /// Gives every seat still without a pick a random one of the heroes it
    /// may pick, as Dota does for a player who picks nothing, locked, and
    /// ready when somebody holds the seat. Each seat reads one draw whether
    /// it needs it or not, so a seat's hero does not depend on the others'
    /// picks.
    fn draw_missing_picks(&mut self, late_secs: u32) {
        let mut rng = self.secrets.hero_stream();
        for at in 0..self.roster.seats.len() {
            let draw = rng.next_u32() as usize;
            if self.roster.seats[at].hero.is_some() {
                continue;
            }
            let free: Vec<HeroId> = self
                .allowed_heroes()
                .into_iter()
                .filter(|h| self.opts.same_heroes || !self.roster.holds(*h))
                .collect();
            let seat = &mut self.roster.seats[at];
            seat.hero = Some(free[draw % free.len()]);
            seat.pick_locked = true;
            seat.late_secs = late_secs;
            seat.ready = seat.player.is_some();
        }
    }

    /// The heroes a seat may pick.
    fn allowed_heroes(&self) -> Vec<HeroId> {
        match &self.opts.heroes {
            None => (0..bota_core::HEROES.len() as u16).map(HeroId).collect(),
            Some(heroes) => heroes.clone(),
        }
    }

    fn build_config(&self) -> MatchConfig {
        MatchConfig {
            match_id: self.secrets.match_id,
            master_key: self.secrets.master_key,
            stream: self.secrets.stream,
            picks: self.roster.picks(),
            map: self.opts.map,
            tick_rate: self.opts.tick_rate,
            mode: self.opts.mode,
            ack_timeout_ticks: self.opts.ack_timeout_ticks,
            cheats: self.opts.cheats,
            immortal_structures: self.opts.immortal_structures,
            immortal_heroes: self.opts.immortal_heroes,
            spawn_modifiers: Vec::new(),
            starting_gold_lost: self
                .roster
                .seats
                .iter()
                .filter(|s| s.late_secs > 0)
                .map(|s| {
                    (
                        s.slot,
                        s.late_secs as i32 * rules::PICK_PENALTY_GOLD_PER_SEC,
                    )
                })
                .collect(),
        }
    }

    /// Runs the match to its end.
    fn match_phase(&mut self, cfg: MatchConfig) -> MatchEnd {
        let mut replay = std::mem::replace(&mut self.replay, ReplayWriter::disabled());
        let mut world = World::for_match(&cfg, cfg.rng());
        let start = ServerMsg::MatchStart { info: cfg.info() };
        self.broadcast(&start);
        replay.record(&ReplayRecord::Msg(start));
        let period = Duration::from_nanos(1_000_000_000 / u64::from(self.opts.tick_rate.max(1)));
        let pace = Pace {
            started: Instant::now(),
            period,
            ack_timeout: period * self.opts.ack_timeout_ticks.max(1),
        };
        let mut inbox = Inbox {
            line: DelayLine::new(&cfg, &self.opts.order_delays)
                .expect("the command line checked the order delays"),
            acked: vec![0; self.roster.seats.len()],
            handles: Handles::new(&cfg, &world),
        };
        let mut refused = Vec::new();
        let mut empty_since = None;
        let end = loop {
            self.gather_input(&world, &cfg, &mut inbox, &pace);
            self.record_heard(&mut replay);
            if self.deserted(&mut empty_since) {
                eprintln!("bota_server: every seat is empty; the match is abandoned");
                break MatchEnd::Abandoned;
            }
            let commands = inbox.line.release(&world, &mut refused);
            self.refuse(&mut refused);
            let taken: Vec<SlotOrder> = commands.iter().map(Command::told).collect();
            replay.record(&ReplayRecord::Orders {
                tick: world.tick + 1,
                orders: taken.clone(),
            });
            let events = world.advance(&commands);
            refused.append(&mut world.refused);
            self.refuse(&mut refused);
            let told = Told {
                line: &inbox.line,
                handles: &mut inbox.handles,
                events: &events,
                taken: &taken,
            };
            if let Some(held) = self.publish_tick(&world, told, &mut replay) {
                self.feed_late(held);
            }
            if let Some(winner) = world.victor() {
                let stats = world.match_stats();
                let over = ServerMsg::MatchOver {
                    winner,
                    stats: stats.clone(),
                };
                replay.record(&ReplayRecord::Msg(over.clone()));
                self.show_late_the_end();
                self.broadcast(&over);
                let picks = cfg.picks.clone();
                break MatchEnd::Over {
                    winner,
                    report: MatchReport { picks, stats },
                };
            }
        };
        replay.finish();
        self.close_all();
        end
    }

    /// Ends every connection once what is queued for it has gone out.
    fn close_all(&mut self) {
        for conn in &mut self.conns {
            conn.close_and_wait();
        }
    }

    /// Whether the match is over for want of players: every seat is empty
    /// and none may be taken back, or every seat has been empty for longer
    /// than [`ServerOpts::empty_after`]. Otherwise a player who may come back
    /// keeps an empty match going to its end.
    fn deserted(&self, empty_since: &mut Option<Instant>) -> bool {
        if self.roster.anyone_seated() {
            *empty_since = None;
            return false;
        }
        if !self.returnable() {
            return true;
        }
        let since = *empty_since.get_or_insert_with(Instant::now);
        self.opts
            .empty_after
            .is_some_and(|after| since.elapsed() >= after)
    }

    /// Holds a tick back for the late spectators and shows them the ticks
    /// now due.
    fn feed_late(&mut self, held: HeldTick) {
        let Some(feed) = &mut self.delayed else {
            return;
        };
        for due in feed.push(held) {
            self.show_late(&due);
        }
    }

    /// Shows the late spectators the last tick at once: the match is over and
    /// nothing is secret any more.
    fn show_late_the_end(&mut self) {
        if let Some(last) = self.delayed.as_mut().and_then(DelayedFeed::take_last) {
            self.show_late(&last);
        }
    }

    /// Shows one held tick to every late spectator through the eyes it
    /// chose; one watching the whole map is told both seats' orders too.
    fn show_late(&self, held: &HeldTick) {
        for watcher in self.spectators.iter().filter(|w| w.late) {
            let Some(conn) = self.conn(watcher.id) else {
                continue;
            };
            let at = sight(self.sight_of(watcher.id).1);
            conn.send_tick(held.frames[at].clone(), held.tick, held.events[at].clone());
            if at == 0 && !held.orders.is_empty() {
                conn.send(&ServerMsg::Orders {
                    tick: held.tick,
                    orders: held.orders.clone(),
                });
            }
            for heard in &held.heard {
                conn.send_chat(&ServerMsg::Heard(heard.clone()));
            }
        }
    }

    /// Whether a seat nobody holds may still be taken back: it is reserved, or
    /// its last holder came with a key.
    fn returnable(&self) -> bool {
        !self.opts.reserved.is_empty() || self.roster.seats.iter().any(|s| s.claim.is_some())
    }

    /// Sends a finished tick to every connection watching live, each through
    /// its own eyes and in its side's names, and records it. Returns the tick
    /// for the late spectators when they may watch.
    fn publish_tick(
        &self,
        world: &World,
        told: Told,
        replay: &mut ReplayWriter,
    ) -> Option<HeldTick> {
        let Told {
            line,
            handles,
            events,
            taken,
        } = told;
        let mut full = world.view_full();
        full.order_delays = line.views(None);
        let full_frame = snapshot_frame(full.clone());
        let full_events = visible_events(events, None);
        let sides = [Team::Radiant, Team::Dire].map(|team| {
            let frame = snapshot_frame(side_view(world, line, handles, team));
            (team, frame, side_events(events, handles, team, world.tick))
        });
        replay.record(&ReplayRecord::Msg(ServerMsg::Snapshot { view: full }));
        // A connection that has not said Hello is told nothing, a referee no
        // tick, and a late spectator only what the delayed feed shows it.
        let watching = |id: PlayerId| {
            self.roster.seat_of(id).is_some()
                || self.spectators.iter().any(|w| w.id == id && !w.late)
        };
        // Only a spectator whose key fixed its eyes on a seat is told that
        // seat's orders live; borrowed eyes lend fog, never orders.
        let locked = |id: PlayerId| self.spectators.iter().any(|w| w.id == id && w.locked);
        for conn in self.conns.iter().filter(|c| watching(c.id)) {
            let (eyes, viewing) = self.sight_of(conn.id);
            // Seats never sit on the jungle's side.
            let side = sides.iter().find(|(team, ..)| Some(*team) == viewing);
            match side {
                Some((_, frame, told)) => conn.send_tick(frame.clone(), world.tick, told.clone()),
                None => conn.send_tick(full_frame.clone(), world.tick, full_events.clone()),
            }
            if let (Some(eyes), Some(team)) = (eyes.filter(|_| locked(conn.id)), viewing) {
                let told = seat_orders(taken, eyes, (handles, team), world.tick);
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
        self.delayed.as_ref()?;
        // Late spectators watching the whole map are told the simulation's
        // handles, those with a side's fog that side's names, as live.
        let [
            (_, radiant_frame, radiant_events),
            (_, dire_frame, dire_events),
        ] = sides;
        Some(HeldTick {
            tick: world.tick,
            frames: [full_frame, radiant_frame, dire_frame],
            events: [full_events, radiant_events, dire_events],
            orders: taken.to_vec(),
            heard: Vec::new(),
        })
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
            .find(|w| w.id == id)
            .and_then(|w| w.eyes);
        // Seats stand in slot order.
        let team = eyes.and_then(|slot| self.roster.seats.get(usize::from(slot.0)));
        let team = team.map(|seat| seat.team);
        (eyes, team)
    }

    /// Waits out the tick according to the mode, handling messages meanwhile.
    fn gather_input(&mut self, world: &World, cfg: &MatchConfig, inbox: &mut Inbox, pace: &Pace) {
        self.drop_strangers();
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
            let Ok(event) = self.rx.recv_timeout(deadline - now) else {
                break;
            };
            if let Some((id, msg)) = self.hear(event, true) {
                self.receive_in_match(id, msg, world, cfg, inbox);
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
            ClientMsg::Hello {
                role,
                name,
                key,
                wire_fingerprint,
            } => {
                // A seat taken mid-match was never shown the tick the match
                // stands on: lockstep waits for it from the next one.
                if self.greet(id, role, name, key, wire_fingerprint, Some(cfg))
                    && let Some(seat) = self.roster.seat_of(id)
                {
                    let at = usize::from(seat.slot.0);
                    inbox.acked[at] = inbox.acked[at].max(world.tick);
                }
            }
            ClientMsg::Order {
                seq,
                unit,
                order,
                seen,
                queue,
            } => {
                let Some((slot, team)) = self.roster.seat_of(id).map(|s| (s.slot, s.team)) else {
                    return;
                };
                let (unit, order) = inbox.handles.heard(team, unit, order);
                let cmd = Command {
                    slot,
                    unit,
                    order,
                    queue,
                    seq,
                };
                if let Err(reason) = inbox.line.give(world, cmd, seen)
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
                if let Some(spectator) =
                    self.spectators.iter_mut().find(|w| w.id == id && !w.locked)
                {
                    spectator.eyes = eyes;
                }
            }
            ClientMsg::Say(line) => self.say(id, line, Some((world, &inbox.handles))),
            ClientMsg::Compress(_) => self.compress(id),
            ClientMsg::PickHero { .. } | ClientMsg::SetReady(_) | ClientMsg::Pong => {}
        }
    }

    /// Tells each seat of its orders that no longer held when their tick came.
    fn refuse(&self, refused: &mut Vec<RefusedOrder>) {
        for refusal in refused.drain(..) {
            let seat = self.roster.seats.get(usize::from(refusal.slot.0));
            let holder = seat.and_then(|seat| seat.player);
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

/// What one side sees of the world, in its own names, with the order delays
/// it is told.
pub(crate) fn side_view(
    world: &World,
    line: &DelayLine,
    handles: &mut Handles,
    team: Team,
) -> WorldView {
    let mut view = world.view(team);
    view.order_delays = line.views(Some(team));
    handles.tell_view(world, &mut view);
    view
}

/// The orders of one seat taken on a tick, in the names of the side shown
/// them.
fn seat_orders(
    taken: &[SlotOrder],
    seat: SlotId,
    (handles, team): (&mut Handles, Team),
    tick: u32,
) -> Vec<SlotOrder> {
    let mut told: Vec<SlotOrder> = taken
        .iter()
        .filter(|given| given.slot == seat)
        .copied()
        .collect();
    handles.tell_orders(team, tick, &mut told);
    told
}

/// The events one side is told of on a tick, in its own names.
pub(crate) fn side_events(
    events: &[Event],
    handles: &mut Handles,
    team: Team,
    tick: u32,
) -> Vec<EventKind> {
    let told = visible_events(events, Some(team));
    let (mut told, runs) = disguise_level_ups(told, handles, team);
    handles.tell_events(team, tick, &mut told);
    // In the side's own names, a hero's level up cannot be told from its
    // illusions' by where it stands among them.
    for run in runs {
        told[run].sort_unstable_by_key(|event| match event {
            EventKind::LevelUp { unit, .. } => *unit,
            _ => unreachable!("a run holds level ups alone"),
        });
    }
    told
}

/// The events a side is told with every level up of the other side's hero
/// told once more for each illusion of it, and where each such run of level
/// ups stands.
fn disguise_level_ups(
    events: Vec<EventKind>,
    handles: &Handles,
    team: Team,
) -> (Vec<EventKind>, Vec<core::ops::Range<usize>>) {
    let mut told = Vec::with_capacity(events.len());
    let mut runs = Vec::new();
    for event in events {
        let EventKind::LevelUp { unit, level } = event else {
            told.push(event);
            continue;
        };
        let start = told.len();
        told.push(event);
        let illusions = handles.illusions_of(team, unit);
        told.extend(
            illusions
                .iter()
                .map(|&unit| EventKind::LevelUp { unit, level }),
        );
        if !illusions.is_empty() {
            runs.push(start..told.len());
        }
    }
    (told, runs)
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

#[cfg(test)]
mod tests {
    use super::draw_secrets;

    #[test]
    fn an_unseeded_match_draws_a_fresh_key_and_keeps_the_label_given() {
        let a = draw_secrets(None, Some(42)).expect("os randomness");
        let b = draw_secrets(None, Some(42)).expect("os randomness");
        assert_eq!((a.match_id, b.match_id), (42, 42));
        assert_ne!(a.master_key, b.master_key);
    }
}
