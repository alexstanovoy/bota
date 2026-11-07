use crate::{BotSeat, Keys, Limits, LuaTeam, ModuleBank, TeamTurn, state::Snapshot};
use bota_client_core::ClientSession;
use bota_proto::Team;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct NativeConfig {
    pub team: Team,
    pub seats: Vec<BotSeat>,
    pub limits: Limits,
    pub keys: Keys,
    pub think_timeout: Duration,
    pub startup_timeout: Duration,
}
impl NativeConfig {
    pub fn new(team: Team, seats: Vec<BotSeat>, keys: Keys) -> Self {
        Self {
            team,
            seats,
            keys,
            limits: Limits::DEFAULT,
            think_timeout: Duration::from_millis(33),
            startup_timeout: Duration::from_secs(30),
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

/// One request at a time on a dedicated stack. Late outputs are discarded and the worker is retired.
/// Cleanup never waits on the caller. Retired workers finish bounded fuel asynchronously; a process-wide
/// permit bounds active and retiring workers together to 32. Native limits cannot exceed interpreter defaults.
pub struct NativeTeam {
    requests: Option<SyncSender<Snapshot>>,
    replies: Option<Receiver<Reply>>,
    worker: Option<JoinHandle<()>>,
    team: Team,
    seats: Vec<BotSeat>,
    timeout: Duration,
    last_tick: Option<u32>,
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
                "native limits may only reduce interpreter defaults".to_owned(),
            ));
        }
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
                    build(config, modules, &input)
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
                last_tick: None,
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
        if self.timeout.is_zero() {
            self.retire();
            return Err(NativeError::Deadline);
        }
        let began = Instant::now();
        if session.team() != self.team {
            return Err(NativeError::Input(
                "worker needs its team's filtered session".to_owned(),
            ));
        }
        let snapshot =
            Snapshot::capture(session).map_err(|error| NativeError::Input(error.to_string()))?;
        if self.last_tick.is_some_and(|last| snapshot.tick <= last) {
            return Err(NativeError::Input("worker tick must increase".to_owned()));
        }
        self.last_tick = Some(snapshot.tick);
        let Some(remaining) = self
            .timeout
            .checked_sub(began.elapsed())
            .filter(|left| !left.is_zero())
        else {
            self.retire();
            return Err(NativeError::Deadline);
        };
        if self
            .requests
            .as_ref()
            .expect("active worker")
            .try_send(snapshot)
            .is_err()
        {
            self.retire();
            return Err(NativeError::Closed);
        }
        let reply = self
            .replies
            .as_ref()
            .expect("active worker")
            .recv_timeout(remaining);
        // A ready value may be late if the caller was descheduled before receiving it.
        if began.elapsed() > self.timeout {
            self.retire();
            return Err(NativeError::Deadline);
        }
        match reply {
            Ok(Ok(turn)) => Ok(turn),
            Ok(Err(error)) => {
                self.retire();
                Err(error)
            }
            Err(_) => {
                self.retire();
                Err(NativeError::Deadline)
            }
        }
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
fn build(
    config: NativeConfig,
    modules: ModuleBank,
    input: &Receiver<Snapshot>,
) -> Result<(LuaTeam, Vec<BotSeat>), NativeError> {
    let mut lua =
        LuaTeam::new(config.team, &config.seats, config.limits, config.keys).map_err(setup)?;
    lua.set_modules(modules).map_err(setup)?;
    lua.prepare_selection().map_err(setup)?;
    for tick in 0..900 {
        if matches!(input.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
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
    let seats = lua.finish_selection();
    lua.load_pack().map_err(setup)?;
    Ok((lua, seats))
}
fn run(lua: &mut LuaTeam, input: Receiver<Snapshot>, output: SyncSender<Reply>) {
    while let Ok(snapshot) = input.recv() {
        let (reply, panicked) =
            match catch_unwind(AssertUnwindSafe(|| lua.think_snapshot(snapshot))) {
                Ok(result) => (
                    result.map_err(|error| NativeError::Input(error.to_string())),
                    false,
                ),
                Err(_) => (Err(NativeError::Panicked), true),
            };
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
    let cap = Limits::DEFAULT;
    limits.fuel_per_call <= cap.fuel_per_call
        && limits.fuel_per_tick <= cap.fuel_per_tick
        && limits.memory <= cap.memory
        && limits.string_length <= cap.string_length
        && limits.call_depth <= cap.call_depth
        && limits.syntax_depth <= cap.syntax_depth
        && limits.pattern_depth <= cap.pattern_depth
        && limits.module_size <= cap.module_size
        && limits.scripts_size <= cap.scripts_size
        && limits.module_count <= cap.module_count
}

#[cfg(test)]
mod tests;
