//! Bounded, opt-in embedding measurements. None of these values are script inputs.
use serde::Serialize;
use std::{cell::RefCell, rc::Rc};

pub const MAX_API_NAMES: usize = 512;
pub const MAX_RECORDED_TICKS: usize = 256;
#[derive(Clone, Debug)]
pub struct Histogram {
    buckets: [u64; 64],
    count: u64,
    min: Option<u64>,
    max: u64,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: [0; 64],
            count: 0,
            min: None,
            max: 0,
        }
    }
}
impl Serialize for Histogram {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_json::json!({"buckets":self.buckets.as_slice(),"count":self.count,"min":self.min,"median":self.percentile(50),"p99":self.percentile(99),"max":self.max}).serialize(serializer)
    }
}
impl Histogram {
    pub fn record(&mut self, value: u64) {
        let index = if value == 0 {
            0
        } else {
            (64 - value.leading_zeros()) as usize
        };
        let index = index.min(63);
        self.buckets[index] = self.buckets[index].saturating_add(1);
        self.count = self.count.saturating_add(1);
        self.min = Some(self.min.map_or(value, |old| old.min(value)));
        self.max = self.max.max(value);
    }
    /// Percentiles are upper bounds of base-two buckets, not exact samples.
    pub fn percentile(&self, percent: u64) -> u64 {
        let rank = self
            .count
            .saturating_mul(percent.min(100))
            .div_ceil(100)
            .max(1);
        let mut seen: u64 = 0;
        for (index, count) in self.buckets.iter().enumerate() {
            seen = seen.saturating_add(*count);
            if seen >= rank {
                return if index == 63 {
                    u64::MAX
                } else {
                    (1u64 << index).saturating_sub(1)
                }
                .min(self.max);
            }
        }
        0
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ApiCounter {
    pub name: &'static str,
    pub calls: u64,
    pub cpu_ns: u64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct TurnStatistics {
    pub tick: u32,
    pub fuel: u64,
    pub cpu_ns: u64,
    pub capture_ns: u64,
    pub handles_ns: u64,
    pub api: Vec<ApiCounter>,
}
pub(crate) type SharedStatistics = Rc<RefCell<TurnStatistics>>;
thread_local! {
    static REGISTERING: RefCell<Option<SharedStatistics>> = const { RefCell::new(None) };
}
pub(crate) fn registering<T>(stats: Option<SharedStatistics>, install: impl FnOnce() -> T) -> T {
    struct Reset(Option<SharedStatistics>);
    impl Drop for Reset {
        fn drop(&mut self) {
            REGISTERING.with(|cell| {
                cell.replace(self.0.take());
            });
        }
    }
    let _reset = Reset(REGISTERING.with(|cell| cell.replace(stats)));
    install()
}
pub(crate) fn instrument(
    name: &'static str,
    function: luagee::HostFunction,
) -> luagee::HostFunction {
    let Some(stats) = REGISTERING.with(|cell| cell.borrow().clone()) else {
        return function;
    };
    let index = {
        let mut data = stats.borrow_mut();
        if let Some(index) = data.api.iter().position(|entry| entry.name == name) {
            index
        } else if data.api.len() < MAX_API_NAMES {
            data.api.push(ApiCounter {
                name,
                calls: 0,
                cpu_ns: 0,
            });
            data.api.len() - 1
        } else {
            0
        }
    };
    Rc::new(move |vm, args| {
        let start = cpu_now();
        let result = function(vm, args);
        let elapsed = cpu_now().saturating_sub(start);
        let mut data = stats.borrow_mut();
        data.api[index].calls = data.api[index].calls.saturating_add(1);
        data.api[index].cpu_ns = data.api[index].cpu_ns.saturating_add(elapsed);
        result
    })
}
pub(crate) fn collector() -> SharedStatistics {
    Rc::new(RefCell::new(TurnStatistics {
        api: vec![ApiCounter {
            name: "other",
            calls: 0,
            cpu_ns: 0,
        }],
        ..Default::default()
    }))
}
#[cfg(test)]
thread_local! { static INJECTED_CPU: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
pub(crate) fn cpu_now() -> u64 {
    #[cfg(test)]
    if let Some(now) = INJECTED_CPU.with(std::cell::Cell::get) {
        return now;
    }
    #[cfg(all(feature = "native", target_os = "linux"))]
    {
        let time = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        (time.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(time.tv_nsec as u64)
    }
    #[cfg(not(all(feature = "native", target_os = "linux")))]
    {
        0
    }
}
pub(crate) struct HandleTimer(Option<(SharedStatistics, u64)>);
impl HandleTimer {
    pub fn new(stats: &Option<SharedStatistics>) -> Self {
        Self(stats.as_ref().map(|stats| (stats.clone(), cpu_now())))
    }
}
impl Drop for HandleTimer {
    fn drop(&mut self) {
        if let Some((stats, start)) = &self.0 {
            let elapsed = cpu_now().saturating_sub(*start);
            let mut data = stats.borrow_mut();
            data.handles_ns = data.handles_ns.saturating_add(elapsed);
        }
    }
}
#[derive(Debug, Default, Serialize)]
pub struct TickList {
    pub total: u64,
    pub ticks: Vec<u32>,
}
impl TickList {
    pub fn record(&mut self, tick: u32) {
        self.total = self.total.saturating_add(1);
        if self.ticks.len() < MAX_RECORDED_TICKS {
            self.ticks.push(tick);
        }
    }
}
#[derive(Debug, Default, Serialize)]
pub struct RunStatistics {
    pub turns: u64,
    pub fuel: Histogram,
    pub cpu_ns: Histogram,
    pub capture_ns: Histogram,
    pub handles_ns: Histogram,
    pub api_ns: Histogram,
    pub script_ns: Histogram,
    pub api: Vec<ApiCounter>,
    pub late: TickList,
    pub discarded: TickList,
    pub fuel_exhaustions: TickList,
    pub script_failures: u64,
    pub first_error: Option<String>,
}
impl RunStatistics {
    pub fn failure(&mut self, error: &str) {
        self.script_failures = self.script_failures.saturating_add(1);
        if self.first_error.is_none() {
            self.first_error = Some(error.chars().take(1024).collect());
        }
    }
    pub fn record(&mut self, data: &TurnStatistics) {
        self.turns = self.turns.saturating_add(1);
        let api = data
            .api
            .iter()
            .fold(0u64, |sum, entry| sum.saturating_add(entry.cpu_ns));
        self.fuel.record(data.fuel);
        self.cpu_ns
            .record(data.cpu_ns.saturating_add(data.capture_ns));
        self.capture_ns.record(data.capture_ns);
        self.handles_ns.record(data.handles_ns);
        self.api_ns.record(api.saturating_sub(data.handles_ns));
        self.script_ns.record(data.cpu_ns.saturating_sub(api));
        for entry in &data.api {
            if let Some(old) = self.api.iter_mut().find(|old| old.name == entry.name) {
                old.calls = old.calls.saturating_add(entry.calls);
                old.cpu_ns = old.cpu_ns.saturating_add(entry.cpu_ns);
            } else if self.api.len() < MAX_API_NAMES {
                self.api.push(entry.clone());
            }
        }
    }
    pub fn write_atomic(
        &self,
        path: &std::path::Path,
        keys: crate::Keys,
        result: serde_json::Value,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let hist = |value: &Histogram| serde_json::json!({"buckets":value.buckets.as_slice(),"count":value.count,"min":value.min,"median":value.percentile(50),"p99":value.percentile(99),"max":value.max});
        let data = serde_json::json!({"hash_keys":keys.hash,"rng_seed":keys.random,"result":result,"statistics":self,
            "percentiles":{"fuel":hist(&self.fuel),"cpu_ns":hist(&self.cpu_ns)}});
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut entropy = [0u8; 8];
        #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
        getrandom::fill(&mut entropy).map_err(std::io::Error::other)?;
        let temp = parent.join(format!(
            ".embedding-summary-{}-{}",
            std::process::id(),
            u64::from_le_bytes(entropy)
        ));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            serde_json::to_writer(&mut file, &data)?;
            file.write_all(b"\n")?;
            file.flush()?;
            std::fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_timers_use_the_injected_thread_cpu_clock() {
        let stats = collector();
        let callback = registering(Some(stats.clone()), || {
            instrument(
                "GetTeam",
                Rc::new(|vm, _| {
                    INJECTED_CPU.with(|clock| clock.set(Some(170)));
                    vm.results(&[])
                }),
            )
        });
        let mut lua = luagee::Lua::new(
            crate::SCRIPT_LIMITS,
            crate::Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        let function = lua.function("clock_probe", callback).unwrap();
        INJECTED_CPU.with(|clock| clock.set(Some(100)));
        lua.call(function, &[]).unwrap();
        INJECTED_CPU.with(|clock| clock.set(None));
        assert_eq!(stats.borrow().api[1].calls, 1);
        assert_eq!(stats.borrow().api[1].cpu_ns, 70);
    }
    #[test]
    fn injected_samples_pin_histograms_and_bounded_diagnostics() {
        let mut stats = RunStatistics::default();
        for value in [0, 1, 2, 3, 4, 8, 1024, u64::MAX] {
            stats.record(&TurnStatistics {
                fuel: value,
                cpu_ns: value,
                ..Default::default()
            });
        }
        assert_eq!(stats.fuel.min, Some(0));
        assert_eq!(stats.fuel.percentile(50), 3);
        assert_eq!(stats.fuel.percentile(99), u64::MAX);
        for tick in 0..1000 {
            stats.late.record(tick);
            stats.failure("\"\\\n\0");
        }
        assert_eq!(stats.late.total, 1000);
        assert_eq!(stats.late.ticks.len(), 256);
        let text = serde_json::to_string(&stats).unwrap();
        assert!(text.contains("\\u0000"));
        assert_eq!(stats.first_error.as_deref(), Some("\"\\\n\0"));
    }
}
