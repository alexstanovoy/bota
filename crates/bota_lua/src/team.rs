use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_proto::{HeroId, SlotId, Team};
use luagee::{Args, Error, Keys, Limit, Limits, Lua, LuaError, TableId, Value, Vm};

use crate::{
    BindingError, api,
    state::{Snapshot, State},
    vector,
};

const MAX_SCRIPTS: usize = 128;
const MAX_DIAGNOSTICS: usize = 64;
const MAX_DIAGNOSTIC_BYTES: usize = 2048;
const MODE_INTERVAL: u32 = 9;
const WARNING_INTERVAL: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotSeat {
    pub slot: SlotId,
    pub hero: HeroId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    Takeover,
    ItemPurchase,
    AbilityItemUsage,
    Mode(u8),
    HeroSelection,
    TeamDesires,
    Minion,
}

struct Script {
    slot: SlotId,
    kind: ScriptKind,
    function: Value,
    environment: TableId,
    name: Value,
    initialized: bool,
}

#[derive(Debug)]
pub struct BotTurn {
    pub slot: SlotId,
    pub orders: Vec<IssuedOrder>,
}

#[derive(Debug)]
pub struct ScriptFailure {
    pub slot: SlotId,
    pub error: Error,
    pub fuel_exhaustions: u64,
    /// Emit this diagnostic now; repeated fuel failures are reported once per thirty ticks.
    pub report: bool,
}

/// Source initializations, attempted hooks and additional team-hook errors; missing functions do not count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookActivity {
    pub initializations: u64,
    pub scripted: u64,
    pub native: u64,
    /// Additional errors after the first reported team-hook error in each turn.
    pub suppressed_team_errors: u64,
}
impl std::ops::AddAssign for HookActivity {
    fn add_assign(&mut self, other: Self) {
        self.initializations = self.initializations.saturating_add(other.initializations);
        self.scripted = self.scripted.saturating_add(other.scripted);
        self.native = self.native.saturating_add(other.native);
        self.suppressed_team_errors = self
            .suppressed_team_errors
            .saturating_add(other.suppressed_team_errors);
    }
}
impl HookActivity {
    fn since(self, previous: Self) -> Self {
        Self {
            initializations: self
                .initializations
                .saturating_sub(previous.initializations),
            scripted: self.scripted.saturating_sub(previous.scripted),
            native: self.native.saturating_sub(previous.native),
            suppressed_team_errors: self
                .suppressed_team_errors
                .saturating_sub(previous.suppressed_team_errors),
        }
    }
}

#[derive(Debug, Default)]
pub struct TeamTurn {
    pub hook_activity: HookActivity,
    pub bots: Vec<BotTurn>,
    pub failures: Vec<ScriptFailure>,
}

#[derive(Debug)]
pub struct SelectionTurn {
    pub seats: Vec<BotSeat>,
    pub complete: bool,
    pub failures: Vec<ScriptFailure>,
}

#[derive(Default)]
struct Schedule {
    modes_loaded: bool,
    active: Option<ActiveMode>,
    evaluated: Option<u32>,
    fuel_exhaustions: u64,
    last_warning: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ActiveMode {
    mode: u8,
    script: Option<usize>,
}

/// One team's interpreter. It consumes filtered sessions and returns ordinary orders, without I/O or threads.
/// Call it once per increasing simulation tick; the native worker supplies wall-time enforcement.
pub struct LuaTeam {
    lua: Lua,
    state: Rc<RefCell<State>>,
    scripts: Vec<Script>,
    schedules: Vec<Schedule>,
    diagnostics: Vec<String>,
    limits: Limits,
    source_bytes: usize,
    last_tick: Option<u32>,
    last_selection_tick: Option<u32>,
    set_environment: Value,
    get_environment: Value,
    modules: Option<Arc<crate::modules::Sources>>,
    module_root: Option<Value>,
    default_mode: Value,
    default_auxiliary: Value,
    lane_desire_reader: Value,
    hook_activity: HookActivity,
}

impl LuaTeam {
    /// Declares bot-controlled wire seats supplied by the trusted host, before execution.
    /// Scripted seats are always bots; every other undeclared seat is treated as human.
    pub fn set_bot_declarations(&mut self, slots: &[SlotId]) -> Result<(), BindingError> {
        if self.last_tick.is_some() || self.last_selection_tick.is_some() {
            return Err(BindingError::Configuration(
                "bot declarations must be set before execution",
            ));
        }
        self.state.borrow_mut().declared_bots = crate::api::players::bot_declarations(slots)?;
        Ok(())
    }

    /// Lists the actual installed API in an isolated interpreter, without loading a pack.
    /// Each line is a tab-separated namespace and name; this is a presence inventory, not a signature check.
    pub fn api_catalogue() -> Result<String, BindingError> {
        let mut team = Self::new(
            Team::Radiant,
            &[BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )?;
        let state = team.state.borrow();
        let arguments = [
            Value::Table(team.lua.globals()),
            state.unit_methods.map_or(Value::Nil, Value::Table),
            state.ability_methods.map_or(Value::Nil, Value::Table),
        ];
        drop(state);
        let function = team
            .lua
            .load("=api_catalogue", include_bytes!("api_catalogue.lua"))?;
        let values = team.lua.call(function, &arguments)?;
        let bytes = values
            .first()
            .and_then(|&value| team.lua.bytes(value))
            .ok_or(BindingError::Configuration(
                "API catalogue did not return text",
            ))?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| BindingError::Configuration("API catalogue is not UTF-8"))
    }

    pub fn new(
        team: Team,
        seats: &[BotSeat],
        limits: Limits,
        keys: Keys,
    ) -> Result<Self, BindingError> {
        validate_seats(team, seats)?;
        let mut lua = Lua::new(limits, keys)?;
        let state = Rc::new(RefCell::new(State::new(team, seats)));
        let storage = lua
            .vm()
            .userdata(state.clone(), None, crate::state::HOST_STORAGE_BYTES)?;
        lua.pin(storage)?;
        let vectors = vector::install(&mut lua)?;
        for (index, name) in BOT_MODES.iter().enumerate() {
            lua.set_global(
                &format!("BOT_MODE_{}", name.to_ascii_uppercase()),
                Value::Number((index + 1) as f64),
            )?;
        }
        api::install(&mut lua, state.clone(), vectors)?;
        let default_mode = crate::default_bot::install(&mut lua, state.clone())?;
        lua.pin(default_mode)?;
        let default_auxiliary = crate::default_bot::install_auxiliary(&mut lua, state.clone())?;
        lua.pin(default_auxiliary)?;
        let raw_next = lua.global("next")?;
        lua.pin(raw_next)?;
        let lane_desire_reader = lua.function(
            "team_lane_desires",
            Rc::new(move |vm, args| lane_desire_values(vm, args, raw_next)),
        )?;
        lua.pin(lane_desire_reader)?;
        let set_environment = lua.global("setfenv")?;
        let get_environment = lua.global("getfenv")?;
        lua.pin(set_environment)?;
        lua.pin(get_environment)?;
        Ok(Self {
            lua,
            state,
            scripts: Vec::new(),
            schedules: (0..seats.len()).map(|_| Schedule::default()).collect(),
            diagnostics: Vec::new(),
            limits,
            source_bytes: 0,
            last_tick: None,
            last_selection_tick: None,
            set_environment,
            get_environment,
            modules: None,
            module_root: None,
            default_mode,
            default_auxiliary,
            lane_desire_reader,
            hook_activity: HookActivity::default(),
        })
    }

    /// A trusted, bounded loader. Native packs are preloaded so loading cannot block a think.
    pub fn set_loader(&mut self, loader: impl luagee::Loader + 'static) {
        self.lua.set_loader(loader);
    }

    /// Installs immutable sources and charges retained storage, including bounded nested loader copies.
    pub fn set_modules(&mut self, modules: crate::ModuleBank) -> Result<(), BindingError> {
        if self.module_root.is_some() || self.last_tick.is_some() {
            return Err(BindingError::Configuration(
                "modules must be installed once before thinking",
            ));
        }
        let modules = modules.freeze(self.limits)?;
        let root = self
            .lua
            .vm()
            .userdata(modules.clone(), None, modules.retained_bytes)?;
        self.lua.pin(root)?;
        self.lua
            .set_loader(crate::modules::MemoryLoader(modules.clone()));
        self.modules = Some(modules);
        self.module_root = Some(root);
        Ok(())
    }

    /// Exercises binding roots under the interpreter's collection stress modes.
    pub fn set_gc_stress(&mut self, stress: luagee::GcStress) {
        self.lua.set_gc_stress(stress);
    }

    /// Compiles one override. Its chunk first runs with a current snapshot, before its first hook.
    pub fn add_script(
        &mut self,
        slot: SlotId,
        kind: ScriptKind,
        name: &str,
        source: &[u8],
    ) -> Result<(), BindingError> {
        if !self
            .state
            .borrow()
            .seats
            .iter()
            .any(|seat| seat.slot == slot)
            || name.len() > 256
        {
            return Err(BindingError::Configuration(
                "script slot or name is invalid",
            ));
        }
        let shared = matches!(kind, ScriptKind::TeamDesires | ScriptKind::HeroSelection);
        if self
            .scripts
            .iter()
            .any(|script| script.kind == kind && (shared || script.slot == slot))
        {
            return Err(BindingError::Configuration("duplicate script override"));
        }
        if self.scripts.len() >= MAX_SCRIPTS
            || source.len() > self.limits.module_size
            || source.len() > self.limits.scripts_size.saturating_sub(self.source_bytes)
        {
            return Err(BindingError::Configuration("script source limit exceeded"));
        }
        let script = self.prepare_script(slot, kind, name, source)?;
        self.scripts.push(script);
        self.source_bytes += source.len();
        Ok(())
    }

    fn prepare_script(
        &mut self,
        slot: SlotId,
        kind: ScriptKind,
        name: &str,
        source: &[u8],
    ) -> Result<Script, BindingError> {
        let mut roots = Vec::with_capacity(3);
        let result = (|| {
            let function = self.lua.load(name, source)?;
            self.lua.pin(function)?;
            roots.push(function);
            let environment = self.lua.vm().new_table()?;
            self.lua.pin(Value::Table(environment))?;
            roots.push(Value::Table(environment));
            let metatable = self.lua.vm().new_table()?;
            let key = self.lua.string(b"__index")?;
            let globals = self.lua.globals();
            self.lua
                .vm()
                .raw_set(metatable, key, Value::Table(globals))?;
            self.lua.vm().set_metatable(environment, Some(metatable))?;
            let name = self.lua.string(name.as_bytes())?;
            self.lua.pin(name)?;
            roots.push(name);
            if self.last_tick.is_none() {
                self.lua.begin_tick();
            }
            self.lua
                .call(self.set_environment, &[function, Value::Table(environment)])?;
            Ok(Script {
                slot,
                kind,
                function,
                environment,
                name,
                initialized: false,
            })
        })();
        if result.is_err() {
            for root in roots {
                let _ = self.lua.unpin(root);
            }
        }
        result
    }

    pub fn think(&mut self, session: &ClientSession) -> Result<TeamTurn, BindingError> {
        if session.team() != self.state.borrow().team {
            return Err(BindingError::Configuration(
                "think needs this team's filtered session",
            ));
        }
        self.think_snapshot(Snapshot::capture(session)?)
    }

    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub(crate) fn host_checkpoint(&self) -> Vec<crate::state::HostCheckpoint> {
        self.state
            .borrow()
            .bots
            .iter()
            .map(|bot| bot.checkpoint())
            .collect()
    }

    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub(crate) fn restore_host(&mut self, checkpoint: Vec<crate::state::HostCheckpoint>) {
        let mut state = self.state.borrow_mut();
        assert_eq!(
            checkpoint.len(),
            state.bots.len(),
            "native acknowledgements belong to this team"
        );
        for (bot, checkpoint) in state.bots.iter_mut().zip(checkpoint) {
            bot.restore(checkpoint);
        }
    }

    pub(crate) fn think_snapshot(&mut self, snapshot: Snapshot) -> Result<TeamTurn, BindingError> {
        if snapshot.team != self.state.borrow().team
            || self.last_tick.is_some_and(|last| snapshot.tick <= last)
        {
            return Err(BindingError::Configuration(
                "think requires an increasing tick for this team",
            ));
        }
        let tick = snapshot.tick;
        let activity_before = self.hook_activity;
        self.last_tick = Some(tick);
        self.state.borrow_mut().snapshot = Some(snapshot);
        let retired = self.state.borrow_mut().retire_handles();
        for table in retired {
            self.lua.unpin(Value::Table(table))?;
        }
        self.state.borrow_mut().purchases = None;
        self.lua.begin_tick();
        let seats = self.state.borrow().seats.clone();
        self.state.borrow_mut().current_slot = seats[0].slot;
        self.state.borrow_mut().orders.clear();
        let _ = self.complete_once(0);
        let first_host = self.state.borrow().bots[0].checkpoint();
        let first_purchases = self.state.borrow().purchases.clone();
        let mut team_error = self.run_team().err();
        let mut result = TeamTurn::default();
        for (index, seat) in seats.iter().enumerate() {
            self.state.borrow_mut().current_slot = seat.slot;
            if index != 0 {
                self.state.borrow_mut().orders.clear();
            }
            let host_before = if index == 0 {
                first_host
            } else {
                let _ = self.complete_once(index);
                self.state.borrow().bots[index].checkpoint()
            };
            let purchases_before = if index == 0 {
                first_purchases.clone()
            } else {
                self.state.borrow().purchases.clone()
            };
            let ended = if index == 0 {
                team_error
                    .take()
                    .map_or_else(|| self.run_seat(index, seat.slot, tick), Err)
            } else {
                self.run_seat(index, seat.slot, tick)
            };
            let orders = if let Err(error) = ended {
                self.state.borrow_mut().orders.clear();
                self.state.borrow_mut().bots[index].restore(host_before);
                self.state.borrow_mut().purchases = purchases_before;
                result
                    .failures
                    .push(self.failure(index, seat.slot, tick, error));
                Vec::new()
            } else {
                let mut state = self.state.borrow_mut();
                if state.bots[index]
                    .pending_once
                    .is_some_and(|pending| pending.launched)
                {
                    state.bots[index].pending_once = None;
                }
                std::mem::take(&mut state.orders)
            };
            result.bots.push(BotTurn {
                slot: seat.slot,
                orders,
            });
        }
        result.hook_activity = self.hook_activity.since(activity_before);
        Ok(result)
    }

    fn complete_once(&mut self, index: usize) -> Option<crate::state::PendingOnce> {
        let mut state = self.state.borrow_mut();
        let mut pending = state.bots[index].pending_once?;
        if state
            .player(pending.source)
            .is_none_or(|player| player.respawn_left > 0)
        {
            state.bots[index].pending_once = None;
            return None;
        }
        let snapshot = state.snapshot.as_ref()?;
        pending.launched |= snapshot.attacks.iter().any(|fact| {
            fact.tick > pending.tick
                && fact.source == pending.source
                && fact.target.is_none_or(|target| target == pending.target)
        });
        if !pending.launched
            && pending
                .expires_at
                .is_some_and(|deadline| snapshot.tick >= deadline)
        {
            state.bots[index].pending_once = None;
            return None;
        }
        state.bots[index].pending_once = Some(pending);
        if pending.launched {
            assert!(
                state.orders.is_empty(),
                "the completion pass precedes this bot's hooks"
            );
            state
                .orders
                .push(IssuedOrder::hero(bota_proto::Order::Move {
                    target: bota_proto::Target::None,
                }));
        }
        Some(pending)
    }

    /// Runs one selection frame; unknown heroes use the configured fallback and foreign seats are ignored.
    pub fn select_heroes(&mut self, tick: u32) -> Result<SelectionTurn, BindingError> {
        if self.last_tick.is_some() || self.last_selection_tick.is_some_and(|last| tick <= last) {
            return Err(BindingError::Configuration(
                "an increasing selection tick before the match is required",
            ));
        }
        self.last_selection_tick = Some(tick);
        self.lua.begin_tick();
        let index = self
            .scripts
            .iter()
            .position(|script| script.kind == ScriptKind::HeroSelection);
        self.state.borrow_mut().selecting = true;
        let ended = index.map_or(Ok(Vec::new()), |index| self.call_hook(index, "Think", &[]));
        self.state.borrow_mut().selecting = false;
        let mut failures = Vec::new();
        if let Err(error) = ended {
            let slot = self.state.borrow().seats[0].slot;
            failures.push(self.failure(0, slot, tick, error));
        }
        let state = self.state.borrow();
        let complete = index.is_none() || state.picks.iter().all(Option::is_some);
        let seats = state
            .seats
            .iter()
            .enumerate()
            .map(|(index, seat)| BotSeat {
                slot: seat.slot,
                hero: state.picks[index].unwrap_or(seat.hero),
            })
            .collect();
        Ok(SelectionTurn {
            seats,
            complete,
            failures,
        })
    }

    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub(crate) fn finish_selection(&mut self) -> Vec<BotSeat> {
        let mut state = self.state.borrow_mut();
        for index in 0..state.seats.len() {
            state.seats[index].hero = state.picks[index].unwrap_or(state.seats[index].hero);
        }
        for schedule in &mut self.schedules {
            schedule.last_warning = None;
        }
        state.seats.clone()
    }

    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    pub(crate) fn prepare_selection(&mut self) -> Result<(), BindingError> {
        let slot = self.state.borrow().seats[0].slot;
        self.add_pack_file(slot, ScriptKind::HeroSelection, "hero_selection")?;
        Ok(())
    }

    /// Loads each hero override, falling back to its generic override.
    pub fn load_pack(&mut self) -> Result<(), BindingError> {
        let seats = self.state.borrow().seats.clone();
        self.add_pack_file(seats[0].slot, ScriptKind::TeamDesires, "team_desires")?;
        for seat in seats {
            let hero = bota_core::hero_def(seat.hero)
                .expect("registered hero")
                .key
                .strip_prefix("npc_dota_hero_")
                .expect("hero key prefix");
            for (prefix, kind) in [
                ("bot", ScriptKind::Takeover),
                ("ability_item_usage", ScriptKind::AbilityItemUsage),
                ("item_purchase", ScriptKind::ItemPurchase),
            ] {
                if !self.add_pack_file(seat.slot, kind, &format!("{prefix}_{hero}"))? {
                    self.add_pack_file(seat.slot, kind, &format!("{prefix}_generic"))?;
                }
            }
            if !self
                .scripts
                .iter()
                .any(|script| script.slot == seat.slot && script.kind == ScriptKind::Takeover)
            {
                self.load_modes(seat.slot)?;
            }
        }
        Ok(())
    }

    fn load_modes(&mut self, slot: SlotId) -> Result<(), BindingError> {
        let seat = self
            .state
            .borrow()
            .seats
            .iter()
            .position(|seat| seat.slot == slot)
            .expect("configured seat");
        let hero = bota_core::hero_def(self.state.borrow().seats[seat].hero)
            .expect("registered hero")
            .key
            .strip_prefix("npc_dota_hero_")
            .expect("hero prefix");
        for (index, mode) in BOT_MODES.iter().enumerate() {
            let kind = ScriptKind::Mode((index + 1) as u8);
            if self
                .scripts
                .iter()
                .any(|script| script.slot == slot && script.kind == kind)
            {
                continue;
            }
            if !self.add_pack_file(slot, kind, &format!("mode_{mode}_{hero}"))? {
                self.add_pack_file(slot, kind, &format!("mode_{mode}_generic"))?;
            }
        }
        self.schedules[seat].modes_loaded = true;
        Ok(())
    }

    fn add_pack_file(
        &mut self,
        slot: SlotId,
        kind: ScriptKind,
        stem: &str,
    ) -> Result<bool, BindingError> {
        let modules = self
            .modules
            .clone()
            .ok_or(BindingError::Configuration("a module bank is required"))?;
        let name = format!("bots/{stem}");
        let Some(source) = modules.get(&name) else {
            return Ok(false);
        };
        self.add_script(slot, kind, &name, source)?;
        Ok(true)
    }

    fn initialize(&mut self, index: usize) -> Result<(), Error> {
        if self.scripts[index].initialized {
            return Ok(());
        }
        self.hook_activity.initializations = self.hook_activity.initializations.saturating_add(1);
        let values = self
            .lua
            .call(self.scripts[index].function, &[self.scripts[index].name])?;
        let environment = if let Some(Value::Table(table)) = values.first() {
            *table
        } else {
            let values = self
                .lua
                .call(self.get_environment, &[self.scripts[index].function])?;
            match values.first() {
                Some(Value::Table(table)) => *table,
                _ => return Err(runtime_error("script environment is not a table")),
            }
        };
        if environment != self.scripts[index].environment {
            self.lua.pin(Value::Table(environment))?;
            self.lua
                .unpin(Value::Table(self.scripts[index].environment))?;
            self.scripts[index].environment = environment;
        }
        self.scripts[index].initialized = true;
        Ok(())
    }

    fn call_hook(
        &mut self,
        index: usize,
        hook: &'static str,
        args: &[Value],
    ) -> Result<Vec<Value>, Error> {
        self.initialize(index)?;
        let environment = self.scripts[index].environment;
        let vm = self.lua.vm();
        let key = vm.new_string(hook.as_bytes()).map_err(callback_error)?;
        let function = vm.raw_get(environment, key).map_err(callback_error)?;
        if function == Value::Nil {
            return Ok(Vec::new());
        }
        self.hook_activity.scripted = self.hook_activity.scripted.saturating_add(1);
        self.lua.call(function, args)
    }

    fn run_team(&mut self) -> Result<(), Error> {
        let index = self
            .scripts
            .iter()
            .position(|script| script.kind == ScriptKind::TeamDesires);
        let Some(index) = index else {
            self.state.borrow_mut().team_desires = crate::state::TeamDesires::default();
            return Ok(());
        };
        let mut first_error = self.call_hook(index, "TeamThink", &[]).err();
        let push = self.lane_desires(index, "UpdatePushLaneDesires");
        let defend = self.lane_desires(index, "UpdateDefendLaneDesires");
        match (push, defend) {
            (Ok(push), Ok(defend)) => {
                self.state.borrow_mut().team_desires = crate::state::TeamDesires { push, defend };
            }
            (push, defend) => {
                for error in [push.err(), defend.err()].into_iter().flatten() {
                    self.record_team_error(&mut first_error, error);
                }
            }
        }
        for hook in [
            "UpdateFarmLaneDesires",
            "UpdateRoamDesire",
            "UpdateRoshanDesire",
        ] {
            if let Err(error) = self.call_hook(index, hook, &[]) {
                self.record_team_error(&mut first_error, error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn record_team_error(&mut self, first: &mut Option<Error>, error: Error) {
        if first.is_none() {
            *first = Some(error);
        } else {
            self.hook_activity.suppressed_team_errors =
                self.hook_activity.suppressed_team_errors.saturating_add(1);
        }
    }

    fn lane_desires(
        &mut self,
        index: usize,
        hook: &'static str,
    ) -> Result<Option<crate::state::LaneDesires>, Error> {
        let values = self.call_hook(index, hook, &[])?;
        let Some(&value) = values.first().filter(|&&value| value != Value::Nil) else {
            return Ok(None);
        };
        let decoded = self.lua.call(self.lane_desire_reader, &[value])?;
        match decoded.as_slice() {
            [Value::Number(top), Value::Number(mid), Value::Number(bot)] => {
                Ok(Some(crate::state::LaneDesires::from_engine_order([
                    *top, *mid, *bot,
                ])))
            }
            _ => Err(runtime_error("invalid host lane-desire decoder result")),
        }
    }

    fn run_seat(&mut self, seat: usize, slot: SlotId, tick: u32) -> Result<(), Error> {
        let takeover = self
            .scripts
            .iter()
            .position(|script| script.slot == slot && script.kind == ScriptKind::Takeover);
        let full_takeover = match takeover {
            Some(index) => self.has_hook(index, "Think")?,
            None => false,
        };
        let minion = self.minion_script(slot, takeover)?;
        if full_takeover {
            self.call_hook(takeover.expect("takeover hook"), "Think", &[])?;
        } else {
            if !self.schedules[seat].modes_loaded && self.modules.is_some() {
                self.load_modes(slot).map_err(binding_error)?;
            }
            self.run_mode(seat, slot, tick)?;
        }
        let usage = self
            .scripts
            .iter()
            .position(|script| script.slot == slot && script.kind == ScriptKind::AbilityItemUsage);
        let mut default_item = false;
        for (hook, default) in [
            (
                "AbilityUsageThink",
                Some(crate::default_bot::AuxiliaryHook::Ability),
            ),
            ("ItemUsageThink", None),
            (
                "CourierUsageThink",
                Some(crate::default_bot::AuxiliaryHook::Courier),
            ),
            ("BuybackUsageThink", None),
            (
                "AbilityLevelUpThink",
                Some(crate::default_bot::AuxiliaryHook::Level),
            ),
        ] {
            if let Some(index) = usage
                && self.has_hook(index, hook)?
            {
                self.call_hook(index, hook, &[])?;
            } else if !full_takeover
                && !(hook == "CourierUsageThink" && minion.is_some())
                && let Some(default) = default
            {
                self.call_auxiliary(default)?;
            } else if !full_takeover && hook == "ItemUsageThink" {
                default_item = true;
            }
        }
        if default_item {
            self.call_auxiliary(crate::default_bot::AuxiliaryHook::Item)?;
        }
        let purchase = self
            .scripts
            .iter()
            .position(|script| script.slot == slot && script.kind == ScriptKind::ItemPurchase);
        if let Some(index) = purchase
            && self.has_hook(index, "ItemPurchaseThink")?
        {
            self.call_hook(index, "ItemPurchaseThink", &[])?;
        } else if !full_takeover {
            self.call_auxiliary(crate::default_bot::AuxiliaryHook::Purchase)?;
        }
        self.run_minions(slot, minion)
    }

    fn call_auxiliary(&mut self, hook: crate::default_bot::AuxiliaryHook) -> Result<(), Error> {
        self.hook_activity.native = self.hook_activity.native.saturating_add(1);
        self.lua.call(
            self.default_auxiliary,
            &[Value::Number(f64::from(hook as u8))],
        )?;
        Ok(())
    }

    fn run_mode(&mut self, seat: usize, slot: SlotId, tick: u32) -> Result<(), Error> {
        if self.schedules[seat]
            .evaluated
            .is_none_or(|last| tick.saturating_sub(last) >= MODE_INTERVAL)
        {
            let mut best = None;
            let mut desire = 0.0;
            for mode in 1..=BOT_MODES.len() as u8 {
                let index = self.scripts.iter().position(|script| {
                    script.slot == slot && script.kind == ScriptKind::Mode(mode)
                });
                let values = if let Some(index) = index {
                    self.call_hook(index, "GetDesire", &[])?
                } else {
                    Vec::new()
                };
                let value = match values.first() {
                    None | Some(Value::Nil) => self.call_default(mode, false)?,
                    Some(Value::Number(value))
                        if value.is_finite() && (0.0..=1.0).contains(value) =>
                    {
                        *value
                    }
                    _ => {
                        return Err(runtime_error(
                            "mode desire must be a finite number between zero and one",
                        ));
                    }
                };
                if value > desire {
                    best = Some(ActiveMode {
                        mode,
                        script: index,
                    });
                    desire = value;
                }
            }
            self.transition(seat, best, desire)?;
            self.schedules[seat].evaluated = Some(tick);
        }
        if let Some(active) = self.schedules[seat].active {
            if let Some(index) = active.script
                && self.has_hook(index, "Think")?
            {
                self.call_hook(index, "Think", &[])?;
            } else {
                self.call_default(active.mode, true)?;
            }
        }
        Ok(())
    }

    fn call_default(&mut self, mode: u8, thinking: bool) -> Result<f64, Error> {
        self.hook_activity.native = self.hook_activity.native.saturating_add(1);
        let result = self.lua.call(
            self.default_mode,
            &[Value::Number(f64::from(mode)), Value::Boolean(thinking)],
        )?;
        Ok(match result.first() {
            Some(Value::Number(value)) => *value,
            _ => 0.0,
        })
    }

    fn has_hook(&mut self, index: usize, hook: &str) -> Result<bool, Error> {
        self.initialize(index)?;
        let environment = self.scripts[index].environment;
        let key = self.lua.string(hook.as_bytes())?;
        Ok(self
            .lua
            .vm()
            .raw_get(environment, key)
            .map_err(callback_error)?
            != Value::Nil)
    }

    fn transition(
        &mut self,
        seat: usize,
        next: Option<ActiveMode>,
        desire: f64,
    ) -> Result<(), Error> {
        if self.schedules[seat].active != next {
            if let Some(ActiveMode {
                script: Some(index),
                ..
            }) = self.schedules[seat].active
            {
                self.call_hook(index, "OnEnd", &[])?;
            }
            self.schedules[seat].active = None;
            self.state.borrow_mut().bots[seat].mode = 0;
            if let Some(ActiveMode {
                script: Some(index),
                ..
            }) = next
            {
                self.call_hook(index, "OnStart", &[])?;
            }
            self.schedules[seat].active = next;
        }
        let mode = next.map_or(0, |active| active.mode);
        self.state.borrow_mut().bots[seat].mode = mode;
        self.state.borrow_mut().bots[seat].desire = desire;
        Ok(())
    }

    fn minion_script(
        &mut self,
        slot: SlotId,
        takeover: Option<usize>,
    ) -> Result<Option<usize>, Error> {
        let dedicated = self
            .scripts
            .iter()
            .position(|script| script.slot == slot && script.kind == ScriptKind::Minion);
        for index in [dedicated, takeover].into_iter().flatten() {
            if self.has_hook(index, "MinionThink")? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn run_minions(&mut self, slot: SlotId, index: Option<usize>) -> Result<(), Error> {
        let Some(index) = index else {
            return Ok(());
        };
        let units: Vec<_> = self
            .state
            .borrow()
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.units)
            .filter(|known| {
                known.visible
                    && known.unit.kind == bota_proto::UnitKind::Courier
                    && known.unit.owner == Some(slot)
            })
            .map(|known| known.unit.id)
            .collect();
        for unit in units {
            let value = self
                .state
                .borrow_mut()
                .handle(self.lua.vm(), Some(unit))
                .map_err(callback_error)?;
            self.call_hook(index, "MinionThink", &[value])?;
        }
        Ok(())
    }

    fn failure(
        &mut self,
        index: usize,
        slot: SlotId,
        tick: u32,
        mut error: Error,
    ) -> ScriptFailure {
        truncate_error(&mut error);
        let schedule = &mut self.schedules[index];
        let report = if matches!(error, Error::Limit(Limit::FuelPerCall | Limit::FuelPerTick)) {
            schedule.fuel_exhaustions = schedule.fuel_exhaustions.saturating_add(1);
            let report = schedule
                .last_warning
                .is_none_or(|last| tick.saturating_sub(last) >= WARNING_INTERVAL);
            if report {
                schedule.last_warning = Some(tick);
            }
            report
        } else {
            let mut message = error.to_string();
            truncate(&mut message, MAX_DIAGNOSTIC_BYTES);
            let unseen = !self.diagnostics.contains(&message);
            if unseen && self.diagnostics.len() < MAX_DIAGNOSTICS {
                self.diagnostics.push(message);
                true
            } else {
                false
            }
        };
        ScriptFailure {
            slot,
            error,
            fuel_exhaustions: schedule.fuel_exhaustions,
            report,
        }
    }
}

fn validate_seats(team: Team, seats: &[BotSeat]) -> Result<(), BindingError> {
    if team == Team::Neutral || seats.is_empty() || seats.len() > 10 {
        return Err(BindingError::Configuration(
            "a Lua team needs one to ten seats on a playing side",
        ));
    }
    for (index, seat) in seats.iter().enumerate() {
        if bota_core::hero_def(seat.hero).is_none()
            || seats[..index].iter().any(|other| other.slot == seat.slot)
        {
            return Err(BindingError::Configuration(
                "Lua seats need distinct slots and registered heroes",
            ));
        }
    }
    Ok(())
}

fn truncate(message: &mut String, limit: usize) {
    if message.len() > limit {
        let mut end = limit;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
}

fn truncate_error(error: &mut Error) {
    match error {
        Error::Runtime { message, traceback } => {
            truncate(message, MAX_DIAGNOSTIC_BYTES);
            traceback.truncate(16);
            for frame in traceback {
                truncate(frame, 512);
            }
        }
        Error::Syntax { chunk, message, .. } => {
            truncate(chunk, 256);
            truncate(message, MAX_DIAGNOSTIC_BYTES);
        }
        Error::Limit(_) => {}
    }
}

fn runtime_error(message: &str) -> Error {
    Error::Runtime {
        message: message.to_owned(),
        traceback: Vec::new(),
    }
}

fn lane_desire_values(vm: &mut Vm, args: Args, raw_next: Value) -> Result<usize, LuaError> {
    let Value::Table(table) = vm.arg(args, 0) else {
        return Err(vm.error(
            "team lane desires must be a table of three finite numbers between zero and one",
        ));
    };
    let mut values = [Value::Nil; 3];
    for (index, output) in values.iter_mut().enumerate() {
        let value = vm.raw_get(table, Value::Number((index + 1) as f64))?;
        if !matches!(value, Value::Number(number) if number.is_finite() && (0.0..=1.0).contains(&number))
        {
            return Err(vm.error(
                "team lane desires must be a table of three finite numbers between zero and one",
            ));
        }
        *output = match value {
            Value::Number(0.0) => Value::Number(0.0),
            _ => value,
        };
    }
    let mut key = Value::Nil;
    for count in 0..=3 {
        let entry = vm.call(raw_next, &[Value::Table(table), key])?;
        key = entry.first().copied().unwrap_or(Value::Nil);
        if key == Value::Nil && count == 3 {
            return vm.results(&values);
        }
        if key == Value::Nil || count == 3 {
            return Err(vm.error("team lane desires must have exactly three array entries"));
        }
    }
    unreachable!("the fourth raw iteration always returns")
}

fn binding_error(error: BindingError) -> Error {
    match error {
        BindingError::Lua(error) => error,
        BindingError::Callback(error) => callback_error(error),
        other => runtime_error(&other.to_string()),
    }
}

fn callback_error(error: luagee::LuaError) -> Error {
    match error {
        luagee::LuaError::Limit(limit) => Error::Limit(limit),
        other => runtime_error(&format!("host operation failed: {other:?}")),
    }
}

pub(crate) const BOT_MODES: &[&str] = &[
    "laning",
    "attack",
    "roam",
    "retreat",
    "secret_shop",
    "side_shop",
    "push_tower_top",
    "push_tower_mid",
    "push_tower_bot",
    "defend_tower_top",
    "defend_tower_mid",
    "defend_tower_bot",
    "assemble",
    "team_roam",
    "farm",
    "defend_ally",
    "evasive_maneuvers",
    "roshan",
    "item",
    "ward",
    "rune",
    "outpost",
];

#[cfg(test)]
#[path = "team/desire_tests.rs"]
mod desire_tests;

#[cfg(test)]
#[path = "team/default_fuel_tests.rs"]
mod default_fuel_tests;
