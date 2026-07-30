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
    let paced = previous_ms.saturating_add(elapsed_ms);
    if wall_ms >= paced {
        return wall_ms;
    }
    // The wall clock is behind the monotonic pace (NTP/sleep-wake stepped it
    // backwards after a higher sample was already returned). Never go
    // backwards, but advance at half rate so this clock re-converges with
    // real wall time. Capability expiries are minted elsewhere from raw wall
    // seconds; a permanently-ahead clock would reject still-valid
    // capabilities for the rest of the process lifetime.
    let catch_up = (elapsed_ms / 2).max(u64::from(elapsed_ms > 0));
    previous_ms.saturating_add(catch_up).max(wall_ms)
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
        let next = next_monotonic_time(10_000, 25, 9_000);
        assert!(
            next > 10_000,
            "the clock must keep progressing while the wall clock is behind"
        );
        assert!(next <= 10_025, "a rollback must not accelerate the clock");
    }

    #[test]
    fn system_clock_reconverges_with_wall_time_after_a_backwards_step() {
        // The wall clock stepped back one second after the higher time was
        // already sampled; both then advance in 100ms real-time steps.
        let mut now = 10_000_u64;
        let mut wall = 9_000_u64;
        for _ in 0..100 {
            wall += 100;
            let next = next_monotonic_time(now, 100, wall);
            assert!(next > now, "the clock must never stall during catch-up");
            now = next;
        }
        assert_eq!(
            now, wall,
            "the ratcheted clock must re-converge with real wall time"
        );
    }

    #[test]
    fn system_clock_accepts_forward_wall_clock_correction() {
        assert_eq!(next_monotonic_time(10_000, 25, 11_000), 11_000);
    }
}
