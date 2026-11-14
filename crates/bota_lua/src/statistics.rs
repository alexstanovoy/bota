//! Bounded, opt-in embedding measurements. None of these values are script inputs.
use serde::Serialize;
use std::{cell::RefCell, rc::Rc};

pub const MAX_API_NAMES: usize = 512;
const MAX_API_NAME_BYTES: usize = 64;
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
    pub api_handles_ns: u64,
    pub api_cpu_ns: u64,
    #[serde(skip)]
    pub(crate) api_depth: u32,
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
        if name.len() > MAX_API_NAME_BYTES {
            0
        } else if let Some(index) = data.api.iter().position(|entry| entry.name == name) {
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
        let (handles_before, outer) = {
            let mut data = stats.borrow_mut();
            let outer = data.api_depth == 0;
            data.api_depth += 1;
            (data.handles_ns, outer)
        };
        let start = cpu_now();
        let result = function(vm, args);
        let elapsed = cpu_now().saturating_sub(start);
        let mut data = stats.borrow_mut();
        data.api_depth -= 1;
        if outer {
            data.api_cpu_ns = data.api_cpu_ns.saturating_add(elapsed);
            data.api_handles_ns = data
                .api_handles_ns
                .saturating_add(data.handles_ns.saturating_sub(handles_before));
        }
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
    #[serde(skip)]
    recent: [Option<u32>; 4],
    #[serde(skip)]
    next: usize,
}
impl TickList {
    /// Coalesce late and script-failure discards for the bounded outstanding worker replies.
    pub fn record_once_recent(&mut self, tick: u32) {
        if self.recent.contains(&Some(tick)) {
            return;
        }
        self.recent[self.next] = Some(tick);
        self.next = (self.next + 1) % self.recent.len();
        self.record(tick);
    }
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
        let api = data.api_cpu_ns.saturating_sub(data.api_handles_ns);
        self.fuel.record(data.fuel);
        self.cpu_ns
            .record(data.cpu_ns.saturating_add(data.capture_ns));
        self.capture_ns.record(data.capture_ns);
        self.handles_ns.record(data.handles_ns);
        self.api_ns.record(api);
        self.script_ns.record(
            data.cpu_ns
                .saturating_sub(api)
                .saturating_sub(data.handles_ns),
        );
        if self.api.is_empty() {
            self.api.push(ApiCounter {
                name: "other",
                calls: 0,
                cpu_ns: 0,
            });
        }
        for entry in &data.api {
            let name = if entry.name.len() > MAX_API_NAME_BYTES {
                "other"
            } else {
                entry.name
            };
            let at = if let Some(at) = self.api.iter().position(|old| old.name == name) {
                at
            } else if self.api.len() < MAX_API_NAMES {
                self.api.push(ApiCounter {
                    name,
                    calls: 0,
                    cpu_ns: 0,
                });
                self.api.len() - 1
            } else {
                0
            };
            let old = &mut self.api[at];
            old.calls = old.calls.saturating_add(entry.calls);
            old.cpu_ns = old.cpu_ns.saturating_add(entry.cpu_ns);
        }
    }
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
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
        crate::native::AtomicOutput::write_atomic(path, |file| {
            serde_json::to_writer(&mut *file, &data)?;
            file.write_all(b"\n")
        })
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
    fn nested_and_external_handle_timers_partition_injected_cpu_exactly() {
        let stats = collector();
        INJECTED_CPU.with(|clock| clock.set(Some(0)));
        let outer = HandleTimer::new(&Some(stats.clone()));
        INJECTED_CPU.with(|clock| clock.set(Some(20)));
        drop(outer);
        let capture = stats.clone();
        let callback = registering(Some(stats.clone()), || {
            instrument(
                "GetBot",
                Rc::new(move |vm, _| {
                    let inner = HandleTimer::new(&Some(capture.clone()));
                    INJECTED_CPU.with(|clock| clock.set(Some(50)));
                    drop(inner);
                    INJECTED_CPU.with(|clock| clock.set(Some(60)));
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
        let function = lua.function("nested", callback).unwrap();
        lua.call(function, &[]).unwrap();
        INJECTED_CPU.with(|clock| clock.set(None));
        let mut data = stats.borrow().clone();
        data.cpu_ns = 100;
        data.capture_ns = 5;
        assert_eq!(data.api_handles_ns, 30);
        assert_eq!(data.api_cpu_ns, 40);
        assert_eq!(data.handles_ns, 50);
        let mut summary = RunStatistics::default();
        summary.record(&data);
        assert_eq!(
            summary.capture_ns.min.unwrap()
                + summary.handles_ns.min.unwrap()
                + summary.api_ns.min.unwrap()
                + summary.script_ns.min.unwrap(),
            105
        );
        assert_eq!(summary.script_ns.min, Some(40));
        assert_eq!(summary.api_ns.min, Some(10));
    }
    #[test]
    fn oversized_api_names_use_the_fixed_other_bucket() {
        let stats = collector();
        let name = Box::leak("x".repeat(513).into_boxed_str());
        let _callback = registering(Some(stats.clone()), || {
            instrument(name, Rc::new(|vm, _| vm.results(&[])))
        });
        assert_eq!(stats.borrow().api.len(), 1);
        assert_eq!(stats.borrow().api[0].name, "other");
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

#[cfg(test)]
mod partition_tests {
    use super::*;
    #[test]
    fn handle_time_outside_an_api_still_partitions_the_turn() {
        let mut stats = RunStatistics::default();
        stats.record(&TurnStatistics {
            cpu_ns: 100,
            capture_ns: 40,
            handles_ns: 50,
            ..Default::default()
        });
        assert_eq!(
            stats.capture_ns.min.unwrap()
                + stats.handles_ns.min.unwrap()
                + stats.api_ns.min.unwrap()
                + stats.script_ns.min.unwrap(),
            stats.cpu_ns.min.unwrap()
        );
    }
    #[test]
    fn more_than_the_catalogue_capacity_uses_one_other_bucket() {
        let data = TurnStatistics {
            api: (0..MAX_API_NAMES + 10)
                .map(|i| ApiCounter {
                    name: Box::leak(format!("function_{i}").into_boxed_str()),
                    calls: 1,
                    cpu_ns: 0,
                })
                .collect(),
            ..Default::default()
        };
        let mut stats = RunStatistics::default();
        stats.record(&data);
        assert_eq!(stats.api.len(), MAX_API_NAMES);
        assert_eq!(
            stats.api.iter().map(|entry| entry.calls).sum::<u64>(),
            (MAX_API_NAMES + 10) as u64
        );
    }
}

#[cfg(test)]
#[test]
fn script_and_late_discards_of_one_turn_are_counted_once() {
    let mut ticks = TickList::default();
    ticks.record_once_recent(1);
    ticks.record_once_recent(2);
    ticks.record_once_recent(1);
    ticks.record_once_recent(2);
    assert_eq!(ticks.total, 2);
    assert_eq!(ticks.ticks, [1, 2]);
}
