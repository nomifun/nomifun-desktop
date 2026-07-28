use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Injectable millisecond clock used by leases, queue aging, and lifecycle
/// sweeps.  Tests can advance [`ManualClock`] without sleeping.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        static STATE: OnceLock<Mutex<SystemClockState>> = OnceLock::new();
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let sampled_at = Instant::now();
        let mut state = STATE
            .get_or_init(|| {
                Mutex::new(SystemClockState {
                    now_ms: wall_ms,
                    sampled_at,
                })
            })
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let elapsed_ms = sampled_at
            .saturating_duration_since(state.sampled_at)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        state.now_ms = next_monotonic_time(state.now_ms, elapsed_ms, wall_ms);
        state.sampled_at = sampled_at;
        state.now_ms
    }
}

struct SystemClockState {
    now_ms: u64,
    sampled_at: Instant,
}

fn next_monotonic_time(previous_ms: u64, elapsed_ms: u64, wall_ms: u64) -> u64 {
    previous_ms.saturating_add(elapsed_ms).max(wall_ms)
}

#[derive(Clone, Default)]
pub struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    pub fn new(now_ms: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now_ms)),
        }
    }

    pub fn set(&self, now_ms: u64) {
        self.now.store(now_ms, Ordering::Release);
    }

    pub fn advance(&self, delta_ms: u64) {
        self.now.fetch_add(delta_ms, Ordering::AcqRel);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_progresses_across_wall_clock_rollback() {
        assert_eq!(next_monotonic_time(10_000, 25, 9_000), 10_025);
    }

    #[test]
    fn system_clock_accepts_forward_wall_clock_correction() {
        assert_eq!(next_monotonic_time(10_000, 25, 11_000), 11_000);
    }
}
