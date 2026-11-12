//! Whole games as jobs: for bots that need no batched inference.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{GymError, MAX_PLANS, MAX_THREADS};

/// Runs `job` on every index below `jobs` (at most [`MAX_PLANS`]) over `threads`
/// (`1..=MAX_THREADS`) threads, each taking the next index as it frees up; returns the results
/// in index order, so they never depend on the thread count. A panicking job is reported as
/// [`GymError::Panicked`] with the lowest index that panicked, after the others finished.
pub fn run_indexed<T: Send>(
    jobs: usize,
    threads: usize,
    job: impl Fn(usize) -> T + Sync,
) -> Result<Vec<T>, GymError> {
    if jobs > MAX_PLANS {
        return Err(GymError::Config("run_indexed takes at most MAX_PLANS jobs"));
    }
    if !(1..=MAX_THREADS).contains(&threads) {
        return Err(GymError::Config(
            "run_indexed threads must be 1..=MAX_THREADS",
        ));
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<T, ()>>>> = Mutex::new((0..jobs).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(jobs) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= jobs {
                        break;
                    }
                    let result = catch_unwind(AssertUnwindSafe(|| job(index))).map_err(|_| ());
                    results.lock().unwrap_or_else(|poison| poison.into_inner())[index] =
                        Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner())
        .into_iter()
        .enumerate()
        .map(|(position, result)| match result.expect("every job ran") {
            Ok(value) => Ok(value),
            Err(()) => Err(GymError::Panicked { position }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_in_index_order_for_any_thread_count() {
        for threads in [1, 3, 8] {
            let squares = run_indexed(20, threads, |index| index * index).expect("no panic");
            assert_eq!(
                squares,
                (0..20).map(|index| index * index).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn the_lowest_panicking_index_is_reported_after_the_rest_ran() {
        let ran = AtomicUsize::new(0);
        let result = run_indexed(10, 4, |index| {
            ran.fetch_add(1, Ordering::Relaxed);
            assert!(index != 3 && index != 7, "job {index} fails");
            index
        });
        assert_eq!(result, Err(GymError::Panicked { position: 3 }));
        assert_eq!(ran.load(Ordering::Relaxed), 10);
    }
}
