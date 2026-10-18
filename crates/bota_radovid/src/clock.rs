//! Milliseconds on a clock that only moves forward, where `Instant` is not
//! available (the browser).

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub(crate) fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}
