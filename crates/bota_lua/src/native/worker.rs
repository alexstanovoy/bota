use crate::{BotSeat, Keys, Limits, LuaTeam, ModuleBank, SCRIPT_LIMITS, TeamTurn, state::Snapshot};
use bota_client_core::ClientSession;
use bota_proto::{SlotId, Team};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_CONSECUTIVE_LATE_TICKS: u8 = 3;
// At the measured ~13ns/fuel, 600k leaves over four times the execution margin
// inside 33ms, including snapshot copying, collection, and scheduling jitter.
pub(super) const NATIVE_LIMITS: Limits = SCRIPT_LIMITS
    .with_fuel_per_call(125_000)
    .with_fuel_per_tick(600_000);

#[derive(Clone, Debug)]
pub struct NativeConfig {
    pub team: Team,
    pub seats: Vec<BotSeat>,
    pub limits: Limits,
    pub keys: Keys,
    pub think_timeout: Duration,
    pub startup_timeout: Duration,
    pub hero_selection: HeroSelection,
    /// Trusted wire-seat declarations. Undeclared foreign seats are treated as human.
    pub declared_bot_slots: Vec<SlotId>,
    pub statistics: bool,
    /// Use bounded fuel without a wall deadline for reproducible local runs.
    pub deterministic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroSelection {
    /// Run the pack's selection hook, retaining configured heroes as fallbacks.
    Pack,
    /// Use the configured seats without loading or executing selection hooks.
    Fixed,
}
impl NativeConfig {
    pub fn new(team: Team, seats: Vec<BotSeat>, keys: Keys) -> Self {
        Self {
            team,
            seats,
            keys,
            limits: NATIVE_LIMITS,
            think_timeout: Duration::from_millis(33),
            startup_timeout: Duration::from_secs(30),
            hero_selection: HeroSelection::Pack,
            declared_bot_slots: Vec::new(),
            statistics: false,
            deterministic: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum NativeError {
    Setup(String),
    Input(String),
    Deadline,
    Panicked,
    Retired,
    Closed,
}
impl std::fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Setup(message) | Self::Input(message) => message,
            Self::Deadline => "Lua team exceeded its wall-time budget",
            Self::Panicked => "Lua team retired after a host panic",
            Self::Retired => "Lua team is retired",
            Self::Closed => "Lua team worker closed",
        })
    }
}
impl std::error::Error for NativeError {}
type Reply = Result<TeamTurn, NativeError>;
pub(super) struct Request {
    capture_ns: u64,
    snapshot: Snapshot,
    previous_discarded: bool,
}

/// One request at a time on a dedicated stack. Late outputs are discarded; three consecutive
/// missed ticks retire the worker. A successful timely reply resets that streak.
/// Cleanup never waits on the caller. Retired workers finish bounded fuel asynchronously; a process-wide
/// permit bounds active and retiring workers together to 32. Native limits cannot exceed the script host's.
pub struct NativeTeam {
    requests: Option<SyncSender<Request>>,
    replies: Option<Receiver<Reply>>,
    worker: Option<JoinHandle<()>>,
    team: Team,
    seats: Vec<BotSeat>,
    timeout: Duration,
    deterministic: bool,
    last_tick: Option<u32>,
    in_flight: bool,
    late_ticks: u8,
    deadline_ticks: u64,
    discard_previous: bool,
    statistics: Option<crate::statistics::RunStatistics>,
}
impl NativeTeam {
    pub fn new(config: NativeConfig, modules: ModuleBank) -> Result<Self, NativeError> {
        Self::start(config, modules, || {})
    }
    fn start(
        config: NativeConfig,
        modules: ModuleBank,
        before_start: impl FnOnce() + Send + 'static,
    ) -> Result<Self, NativeError> {
        crate::api::players::validate_team_size(config.seats.len()).map_err(setup)?;
        crate::api::players::bot_declarations(&config.declared_bot_slots).map_err(setup)?;
        if config.startup_timeout.is_zero()
            || config.startup_timeout > Duration::from_secs(120)
            || config.think_timeout > Duration::from_secs(10)
        {
            return Err(NativeError::Setup(
                "native time budgets are outside their bounds".to_owned(),
            ));
        }
        if !bounded_limits(config.limits) {
            return Err(NativeError::Setup(
                "native limits may only reduce the script host limits".to_owned(),
            ));
        }
        let measured = config.statistics;
        let deterministic = config.deterministic;
        let permit = WorkerPermit::acquire()?;
        let (requests, input) = mpsc::sync_channel(1);
        let (output, replies) = mpsc::sync_channel(1);
        let (ready, startup) = mpsc::sync_channel(1);
        let (team, timeout, startup_timeout) =
            (config.team, config.think_timeout, config.startup_timeout);
        let worker = thread::Builder::new()
            .name("lua_team".to_owned())
            .stack_size(16 << 20)
            .spawn(move || {
                let _permit = permit;
                match catch_unwind(AssertUnwindSafe(|| {
                    before_start();
                    build(config, modules, Some(&input))
                })) {
                    Ok(Ok((mut lua, seats))) => {
                        if ready.send(Ok(seats)).is_ok() {
                            run(&mut lua, input, output);
                        }
                    }
                    Ok(Err(error)) => {
                        let _ = ready.send(Err(error));
                    }
                    Err(_) => {
                        let _ = ready.send(Err(NativeError::Panicked));
                    }
                }
            })
            .map_err(|_| NativeError::Setup("could not create Lua team worker".to_owned()))?;
        match startup.recv_timeout(startup_timeout) {
            Ok(Ok(seats)) => Ok(Self {
                requests: Some(requests),
                replies: Some(replies),
                worker: Some(worker),
                team,
                seats,
                timeout,
                deterministic,
                last_tick: None,
                in_flight: false,
                late_ticks: 0,
                deadline_ticks: 0,
                discard_previous: false,
                statistics: measured.then(Default::default),
            }),
            result => {
                drop(requests);
                drop(replies);
                drop(worker);
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => NativeError::Deadline,
                })
            }
        }
    }
    pub fn seats(&self) -> &[BotSeat] {
        &self.seats
    }
    pub fn retired(&self) -> bool {
        self.requests.is_none()
    }
    pub fn think(&mut self, session: &ClientSession) -> Result<TeamTurn, NativeError> {
        if self.retired() {
            return Err(NativeError::Retired);
        }
        if self.deterministic {
            return self.think_deterministic(session);
        }
        let began = Instant::now();
        if session.team() != self.team {
            return Err(NativeError::Input(
                "worker needs its team's filtered session".to_owned(),
            ));
        }
        let capture_start = self
            .statistics
            .as_ref()
            .map(|_| crate::statistics::cpu_now());
        let snapshot =
            Snapshot::capture(session).map_err(|error| NativeError::Input(error.to_string()))?;
        let capture_ns = capture_start.map_or(0, |start| {
            crate::statistics::cpu_now().saturating_sub(start)
        });
        if self.last_tick.is_some_and(|last| snapshot.tick <= last) {
            return Err(NativeError::Input("worker tick must increase".to_owned()));
        }
        self.last_tick = Some(snapshot.tick);
        if self.in_flight {
            match self.receive(began) {
                Ok(_) => {} // This reply belongs to the previously missed tick, never the new one.
                Err(NativeError::Deadline) => return self.late(),
                Err(error) => return Err(error),
            }
        }
        if self.remaining(began).is_none() {
            return self.late();
        }
        let request = Request {
            capture_ns,
            snapshot,
            previous_discarded: self.discard_previous,
        };
        if self
            .requests
            .as_ref()
            .expect("active worker")
            .try_send(request)
            .is_err()
        {
            self.retire();
            return Err(NativeError::Closed);
        }
        self.discard_previous = false;
        self.in_flight = true;
        match self.receive(began) {
            Ok(turn) if began.elapsed() <= self.timeout => {
                self.late_ticks = 0;
                Ok(turn)
            }
            Ok(_) | Err(NativeError::Deadline) => {
                self.discard_previous = true;
                self.late()
            }
            Err(error) => Err(error),
        }
    }

    fn think_deterministic(&mut self, session: &ClientSession) -> Reply {
        if session.team() != self.team {
            return Err(NativeError::Input(
                "worker needs its team's filtered session".to_owned(),
            ));
        }
        let start = self
            .statistics
            .as_ref()
            .map(|_| crate::statistics::cpu_now());
        let snapshot =
            Snapshot::capture(session).map_err(|error| NativeError::Input(error.to_string()))?;
        if self.last_tick.is_some_and(|tick| snapshot.tick <= tick) {
            return Err(NativeError::Input("worker tick must increase".to_owned()));
        }
        self.last_tick = Some(snapshot.tick);
        let capture_ns = start.map_or(0, |start| {
            crate::statistics::cpu_now().saturating_sub(start)
        });
        self.requests
            .as_ref()
            .expect("active worker")
            .send(Request {
                capture_ns,
                snapshot,
                previous_discarded: false,
            })
            .map_err(|_| NativeError::Closed)?;
        let reply = self
            .replies
            .as_ref()
            .expect("active worker")
            .recv()
            .map_err(|_| NativeError::Closed)?;
        if let (Some(summary), Ok(turn)) = (&mut self.statistics, &reply) {
            record_turn(summary, turn);
        }
        if reply.is_err() {
            self.retire();
        }
        reply
    }

    pub fn statistics(&self) -> Option<&crate::statistics::RunStatistics> {
        self.statistics.as_ref()
    }

    fn remaining(&self, began: Instant) -> Option<Duration> {
        self.timeout
            .checked_sub(began.elapsed())
            .filter(|left| !left.is_zero())
    }

    fn receive(&mut self, began: Instant) -> Reply {
        let Some(remaining) = self.remaining(began) else {
            return Err(NativeError::Deadline);
        };
        match self
            .replies
            .as_ref()
            .expect("active worker")
            .recv_timeout(remaining)
        {
            Ok(result) => {
                self.in_flight = false;
                if let (Some(summary), Ok(turn)) = (&mut self.statistics, &result) {
                    record_turn(summary, turn);
                }
                if result.is_err() {
                    self.retire();
                }
                result
            }
            Err(RecvTimeoutError::Timeout) => Err(NativeError::Deadline),
            Err(RecvTimeoutError::Disconnected) => {
                self.retire();
                Err(NativeError::Closed)
            }
        }
    }

    /// Total turns that missed their wall deadline, including recoverable misses.
    pub fn deadline_ticks(&self) -> u64 {
        self.deadline_ticks
    }

    fn late(&mut self) -> Reply {
        if let (Some(summary), Some(tick)) = (&mut self.statistics, self.last_tick) {
            summary.late.record(tick);
            summary.discarded.record(tick);
        }
        self.deadline_ticks = self.deadline_ticks.saturating_add(1);
        self.late_ticks = self.late_ticks.saturating_add(1);
        if self.late_ticks >= MAX_CONSECUTIVE_LATE_TICKS {
            self.retire();
        }
        Err(NativeError::Deadline)
    }
    fn retire(&mut self) {
        self.requests.take();
        self.replies.take();
    }
}
impl Drop for NativeTeam {
    fn drop(&mut self) {
        self.retire();
        self.worker.take();
    }
}
pub(super) fn build(
    config: NativeConfig,
    modules: ModuleBank,
    input: Option<&Receiver<Request>>,
) -> Result<(LuaTeam, Vec<BotSeat>), NativeError> {
    let mut lua = LuaTeam::new_measured(
        config.team,
        &config.seats,
        config.limits,
        config.keys,
        config.statistics,
    )
    .map_err(setup)?;
    lua.set_bot_declarations(&config.declared_bot_slots)
        .map_err(setup)?;
    lua.set_modules(modules).map_err(setup)?;
    if config.hero_selection == HeroSelection::Pack {
        lua.prepare_selection().map_err(setup)?;
        for tick in 0..900 {
            if input.is_some_and(|input| {
                matches!(input.try_recv(), Err(mpsc::TryRecvError::Disconnected))
            }) {
                return Err(NativeError::Closed);
            }
            let turn = lua.select_heroes(tick).map_err(setup)?;
            if let Some(failure) = turn.failures.first() {
                return Err(NativeError::Setup(failure.error.to_string()));
            }
            if turn.complete {
                break;
            }
        }
    }
    let seats = lua.finish_selection();
    lua.load_pack().map_err(setup)?;
    Ok((lua, seats))
}
fn run(lua: &mut LuaTeam, input: Receiver<Request>, output: SyncSender<Reply>) {
    run_with_reply_hook(lua, input, output, |_| {});
}
fn run_with_reply_hook(
    lua: &mut LuaTeam,
    input: Receiver<Request>,
    output: SyncSender<Reply>,
    mut before_reply: impl FnMut(&Reply),
) {
    let mut previous = None;
    while let Ok(request) = input.recv() {
        if let Some(checkpoint) = previous.take()
            && request.previous_discarded
        {
            lua.restore_host(checkpoint);
        }
        let checkpoint = lua.host_checkpoint();
        let (mut reply, panicked) =
            match catch_unwind(AssertUnwindSafe(|| lua.think_snapshot(request.snapshot))) {
                Ok(result) => (
                    result.map_err(|error| NativeError::Input(error.to_string())),
                    false,
                ),
                Err(_) => (Err(NativeError::Panicked), true),
            };
        if let Ok(turn) = &mut reply
            && let Some(data) = &mut turn.statistics
        {
            data.capture_ns = request.capture_ns;
        }
        previous = Some(checkpoint);
        before_reply(&reply);
        if output.send(reply).is_err() || panicked {
            break;
        }
    }
}
fn setup(error: crate::BindingError) -> NativeError {
    let mut message = error.to_string();
    if message.len() > 4096 {
        let mut end = 4096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    NativeError::Setup(message)
}

static LIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> Result<Self, NativeError> {
        LIVE_WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                if live < 32 { Some(live + 1) } else { None }
            })
            .map(|_| Self)
            .map_err(|_| NativeError::Setup("native worker capacity is exhausted".to_owned()))
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        let previous = LIVE_WORKERS.fetch_sub(1, Ordering::Release);
        assert!(previous > 0, "a worker owns its permit");
    }
}
fn bounded_limits(limits: Limits) -> bool {
    let cap = SCRIPT_LIMITS;
    limits.fuel_per_call <= cap.fuel_per_call
        && limits.fuel_per_tick <= cap.fuel_per_tick
        && limits.memory <= cap.memory
        && limits.string_length <= cap.string_length
        && limits.call_depth <= cap.call_depth
        && limits.syntax_depth <= cap.syntax_depth
        && limits.pattern_depth <= cap.pattern_depth
        && limits.function_upvalues <= cap.function_upvalues
        && limits.module_size <= cap.module_size
        && limits.scripts_size <= cap.scripts_size
        && limits.module_count <= cap.module_count
}

#[cfg(test)]
mod tests;

pub(super) fn record_turn(summary: &mut crate::statistics::RunStatistics, turn: &TeamTurn) {
    if let Some(data) = &turn.statistics {
        summary.record(data);
        summary.script_failures = summary
            .script_failures
            .saturating_add(turn.hook_activity.suppressed_team_errors);
        for failure in &turn.failures {
            summary.failure(&failure.error.to_string());
            if matches!(
                failure.error,
                luagee::Error::Limit(luagee::Limit::FuelPerCall | luagee::Limit::FuelPerTick)
            ) {
                summary.fuel_exhaustions.record(data.tick);
            }
        }
    }
}
