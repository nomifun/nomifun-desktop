//! Bounded, in-memory lifecycle for ZIP requests, including early cancellation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nomifun_common::AppError;

const MAX_REQUESTS: usize = 1_024;
const MAX_REQUEST_ID_BYTES: usize = 256;
// Longer than the renderer's two-minute ZIP deadline, including delayed bodies.
const PENDING_CANCEL_TTL: Duration = Duration::from_secs(10 * 60);

enum RequestState {
    Active(Arc<AtomicBool>),
    CanceledBeforeStart(Instant),
}

#[derive(Default)]
struct RegistryState {
    requests: HashMap<String, RequestState>,
    admission_blocked_until: Option<Instant>,
}

impl RegistryState {
    fn cleanup(&mut self, now: Instant) {
        self.requests.retain(|_, state| match state {
            RequestState::Active(_) => true,
            RequestState::CanceledBeforeStart(expires) => *expires > now,
        });
        if self.admission_blocked_until.is_some_and(|deadline| deadline <= now) {
            self.admission_blocked_until = None;
        }
    }
}

pub(crate) struct ZipRequestRegistry {
    state: Mutex<RegistryState>,
    capacity: usize,
    pending_cancel_ttl: Duration,
}

impl Default for ZipRequestRegistry {
    fn default() -> Self {
        Self { state: Mutex::new(RegistryState::default()), capacity: MAX_REQUESTS, pending_cancel_ttl: PENDING_CANCEL_TTL }
    }
}

impl ZipRequestRegistry {
    pub(crate) fn register<'a>(&'a self, request_id: Option<&'a str>) -> Result<Option<ZipRegistration<'a>>, AppError> {
        self.register_at(request_id, Instant::now())
    }

    fn register_at<'a>(&'a self, request_id: Option<&'a str>, now: Instant) -> Result<Option<ZipRegistration<'a>>, AppError> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Some(id) = request_id {
            if id.is_empty() || id.len() > MAX_REQUEST_ID_BYTES {
                return Err(AppError::BadRequest("ZIP request_id must contain 1..=256 bytes".into()));
            }
            let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.cleanup(now);
            match state.requests.get(id) {
                Some(RequestState::Active(_)) => return Err(AppError::Conflict("ZIP request is already in progress".into())),
                Some(RequestState::CanceledBeforeStart(_)) => {
                    state.requests.remove(id);
                    return Ok(None);
                }
                None => {}
            }
            if state.requests.len() >= self.capacity || state.admission_blocked_until.is_some() {
                return Err(AppError::Conflict("ZIP request capacity reached; retry after pending requests settle".into()));
            }
            state.requests.insert(id.to_owned(), RequestState::Active(flag.clone()));
        }
        Ok(Some(ZipRegistration { registry: self, request_id, flag }))
    }

    pub(crate) fn cancel(&self, request_id: &str) -> bool {
        self.cancel_at(request_id, Instant::now())
    }

    fn cancel_at(&self, request_id: &str, now: Instant) -> bool {
        if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_BYTES {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.cleanup(now);
        if let Some(RequestState::Active(flag)) = state.requests.get(request_id) {
            flag.store(true, Ordering::Relaxed);
            return true;
        }
        let deadline = now + self.pending_cancel_ttl;
        if state.requests.contains_key(request_id) || state.requests.len() < self.capacity {
            state.requests.insert(request_id.to_owned(), RequestState::CanceledBeforeStart(deadline));
        } else {
            // Do not evict an early cancellation and later admit its delayed
            // request. Saturation closes new admissions for the same TTL.
            state.admission_blocked_until = Some(deadline);
        }
        // Preserve the API's existing answer: no active worker was found.
        false
    }
}

pub(crate) struct ZipRegistration<'a> {
    registry: &'a ZipRequestRegistry,
    request_id: Option<&'a str>,
    pub(crate) flag: Arc<AtomicBool>,
}

impl Drop for ZipRegistration<'_> {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::Relaxed);
        if let Some(id) = self.request_id {
            let mut state = self.registry.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if matches!(state.requests.get(id), Some(RequestState::Active(flag)) if Arc::ptr_eq(flag, &self.flag)) {
                state.requests.remove(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_cancellation_is_consumed_before_create() {
        let registry = ZipRequestRegistry::default();
        assert!(!registry.cancel("early"));
        assert!(registry.register(Some("early")).unwrap().is_none());
        assert!(registry.register(Some("early")).unwrap().is_some());
    }

    #[test]
    fn active_cancel_retains_registration_until_cleanup_then_id_can_be_reused() {
        let registry = ZipRequestRegistry::default();
        let first = registry.register(Some("active")).unwrap().unwrap();
        let old_flag = first.flag.clone();
        assert!(registry.cancel("active"));
        assert!(old_flag.load(Ordering::Relaxed));
        assert!(matches!(registry.register(Some("active")), Err(AppError::Conflict(_))));
        drop(first);
        let second = registry.register(Some("active")).unwrap().unwrap();
        assert!(!second.flag.load(Ordering::Relaxed));
        assert!(registry.cancel("active"));
    }

    #[test]
    fn pending_markers_expire_and_capacity_fails_closed_without_eviction() {
        let registry = ZipRequestRegistry {
            state: Mutex::new(RegistryState::default()), capacity: 1, pending_cancel_ttl: Duration::from_secs(10),
        };
        let now = Instant::now();
        assert!(!registry.cancel_at("first", now));
        assert!(!registry.cancel_at("overflow", now));
        assert_eq!(registry.state.lock().unwrap().requests.len(), 1);
        assert!(registry.register_at(Some("first"), now).unwrap().is_none());
        assert!(matches!(registry.register_at(Some("overflow"), now), Err(AppError::Conflict(_))));
        assert!(registry.register_at(Some("next"), now + Duration::from_secs(11)).unwrap().is_some());
    }
}
