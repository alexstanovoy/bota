//! The host's cache domains and opt-in thread pinning.
//!
//! Drivers and workers exchange every episode each round, so a pool of workers with its
//! drivers is best kept inside one last-level cache domain (one CCD of a multi-CCD CPU) when
//! the host has several. Pinning only places threads; it never changes results.

use crate::GymError;

/// Bound on parsed CPUs, far above any supported host.
const MAX_CPUS: usize = 1024;

/// CPU sets that share a last-level cache, in order of their lowest CPU. Falls back to one
/// domain of every available CPU when the host does not describe its caches.
pub fn cache_domains() -> Vec<Vec<usize>> {
    let mut domains = Vec::new();
    for domain in cpu_lists("cache/index3/shared_cpu_list").flatten() {
        if !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    if domains.is_empty() {
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        domains.push((0..cpus.min(MAX_CPUS)).collect());
    }
    domains.sort();
    domains
}

/// Available cores counting SMT siblings once: workers beyond one per physical core mostly
/// slow down whatever shares those cores. Falls back to every available CPU when the host does
/// not describe its SMT siblings.
pub fn physical_cores() -> usize {
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let mut cores = Vec::new();
    let mut logical = 0_usize;
    for siblings in cpu_lists("topology/thread_siblings_list") {
        logical += 1;
        if let Some(siblings) = siblings
            && !cores.contains(&siblings)
        {
            cores.push(siblings);
        }
    }
    if cores.is_empty() {
        return available;
    }
    (available / logical.div_ceil(cores.len())).max(1)
}

/// The CPUs of each of `groups` worker pools: one cache domain each when pinning, none
/// otherwise.
pub fn group_cpus(groups: usize, pin: bool) -> Result<Vec<Option<Vec<usize>>>, GymError> {
    if !pin {
        return Ok(vec![None; groups]);
    }
    let domains = cache_domains();
    if domains.len() < groups {
        return Err(GymError::Config("pinning needs a cache domain per group"));
    }
    Ok(domains.into_iter().take(groups).map(Some).collect())
}

/// Restricts the calling thread to `cpus`.
#[cfg(target_os = "linux")]
pub fn pin_current_thread(cpus: &[usize]) -> Result<(), GymError> {
    let mut set = rustix::thread::CpuSet::new();
    for &cpu in cpus {
        if cpu >= rustix::thread::CpuSet::MAX_CPU {
            return Err(GymError::Config("a CPU to pin to is beyond the CPU set"));
        }
        set.set(cpu);
    }
    rustix::thread::sched_setaffinity(None, &set)
        .map_err(|_| GymError::Config("the thread could not be pinned"))
}

/// Restricts the calling thread to `cpus`.
#[cfg(not(target_os = "linux"))]
pub fn pin_current_thread(_: &[usize]) -> Result<(), GymError> {
    Err(GymError::Config("pinning is supported on Linux only"))
}

/// The parsed CPU list in `file` under each present CPU's sysfs directory, in CPU order;
/// none when CPU 0 has no such file.
fn cpu_lists(file: &str) -> impl Iterator<Item = Option<Vec<usize>>> {
    (0..MAX_CPUS)
        .map_while(move |cpu| {
            let text = std::fs::read_to_string(format!("/sys/devices/system/cpu/cpu{cpu}/{file}"));
            (cpu > 0 || text.is_ok()).then_some(text)
        })
        .filter_map(|text| text.ok())
        .map(|text| parse_cpu_list(text.trim()))
}

/// Parses a sysfs CPU list such as `0-7,16-23`.
fn parse_cpu_list(text: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for range in text.split(',') {
        let (first, last): (usize, usize) = match range.split_once('-') {
            Some((first, last)) => (first.parse().ok()?, last.parse().ok()?),
            None => {
                let cpu = range.parse().ok()?;
                (cpu, cpu)
            }
        };
        if first > last || last >= MAX_CPUS {
            return None;
        }
        cpus.extend(first..=last);
    }
    (!cpus.is_empty()).then_some(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists_parse_ranges_and_reject_malformed_text() {
        assert_eq!(parse_cpu_list("0-3,8"), Some(vec![0, 1, 2, 3, 8]));
        for text in ["", "3-1", "a", "0-2000"] {
            assert_eq!(parse_cpu_list(text), None, "{text}");
        }
        assert!(!cache_domains().is_empty());
        let available = std::thread::available_parallelism().unwrap().get();
        assert!((1..=available).contains(&physical_cores()));
    }
}
