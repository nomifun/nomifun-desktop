use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{BrowserErrorCode, BrowserPlatformError, Clock};

/// The part of browser identity state represented by a canonical snapshot.
///
/// The wire names are intentionally explicit: consumers must not infer that a
/// browser snapshot is a complete Chromium profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotComponentCoverage {
    AllOrigins,
    CurrentOrigin,
    NotIncluded,
}

/// Declared coverage of an authenticated-replica identity snapshot.
///
/// Every included component is an end-to-end restoration claim, not merely a
/// statement that capture bytes exist in the opaque payload. In particular,
/// origin-scoped storage may only be declared when an adapter can prove it is
/// restored before page scripts observe it. Unsupported components must remain
/// `NotIncluded`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCoverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_origin: Option<String>,
    pub cookies: SnapshotComponentCoverage,
    pub local_storage: SnapshotComponentCoverage,
    pub indexed_db: SnapshotComponentCoverage,
    pub cache: SnapshotComponentCoverage,
    pub service_workers: SnapshotComponentCoverage,
    pub session_storage: SnapshotComponentCoverage,
    pub passkeys: SnapshotComponentCoverage,
    pub device_bound_credentials: SnapshotComponentCoverage,
}

impl SnapshotCoverage {
    /// Builds origin-scoped coverage for an adapter that can prove both
    /// capture and pre-script restoration of localStorage and IndexedDB.
    ///
    /// This remains available for future complete restoration paths. Callers
    /// must not use it solely because those bytes were present during capture.
    pub fn current_origin(current_origin: impl Into<String>) -> Self {
        Self {
            current_origin: Some(current_origin.into()),
            cookies: SnapshotComponentCoverage::AllOrigins,
            local_storage: SnapshotComponentCoverage::CurrentOrigin,
            indexed_db: SnapshotComponentCoverage::CurrentOrigin,
            cache: SnapshotComponentCoverage::NotIncluded,
            service_workers: SnapshotComponentCoverage::NotIncluded,
            session_storage: SnapshotComponentCoverage::NotIncluded,
            passkeys: SnapshotComponentCoverage::NotIncluded,
            device_bound_credentials: SnapshotComponentCoverage::NotIncluded,
        }
    }

    /// Coverage for paths that can prove cookie restoration only.
    ///
    /// The opaque payload may still retain origin-scoped state for trusted
    /// persistence or a future restoration implementation; that does not make
    /// the state part of the declared Replica coverage.
    pub fn cookies_only() -> Self {
        Self {
            current_origin: None,
            cookies: SnapshotComponentCoverage::AllOrigins,
            local_storage: SnapshotComponentCoverage::NotIncluded,
            indexed_db: SnapshotComponentCoverage::NotIncluded,
            cache: SnapshotComponentCoverage::NotIncluded,
            service_workers: SnapshotComponentCoverage::NotIncluded,
            session_storage: SnapshotComponentCoverage::NotIncluded,
            passkeys: SnapshotComponentCoverage::NotIncluded,
            device_bound_credentials: SnapshotComponentCoverage::NotIncluded,
        }
    }
}

/// Metadata for the latest canonical identity state published by a trusted
/// Primary capture path.
///
/// Snapshot contents remain owned by the host adapter/vault. This platform
/// object carries only the authoritative generation, publication time, and
/// declared coverage needed to admit AuthenticatedReplica lanes safely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalIdentitySnapshot {
    pub generation: u64,
    pub issued_at_ms: u64,
    pub coverage: SnapshotCoverage,
}

/// Opaque authenticated identity bytes held only inside trusted process
/// boundaries. Debug output is always redacted and the type is deliberately
/// not serializable, preventing cookies or site storage from leaking through
/// inventory/API DTOs.
#[derive(Clone, PartialEq)]
pub struct IdentitySnapshotPayload(Arc<serde_json::Value>);

impl IdentitySnapshotPayload {
    pub fn from_json(value: serde_json::Value) -> Self {
        Self(Arc::new(value))
    }

    pub fn as_json(&self) -> &serde_json::Value {
        self.0.as_ref()
    }
}

impl fmt::Debug for IdentitySnapshotPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentitySnapshotPayload(<redacted>)")
    }
}

#[derive(Clone)]
pub(crate) struct IdentityGenerationCoordinator {
    clock: Arc<dyn Clock>,
    state: Arc<Mutex<IdentityGenerationState>>,
}

#[derive(Default)]
struct IdentityGenerationState {
    current: Option<CanonicalIdentitySnapshot>,
    current_payload: Option<IdentitySnapshotPayload>,
    last_issued_at_ms: u64,
    replica_admission_open: bool,
}

impl IdentityGenerationCoordinator {
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Arc::new(Mutex::new(IdentityGenerationState::default())),
        }
    }

    /// Publishes a new canonical snapshot and assigns its generation.
    ///
    /// There is deliberately no generation argument: only this coordinator
    /// can advance the canonical sequence.
    pub(crate) fn publish_snapshot(
        &self,
        coverage: SnapshotCoverage,
        payload: IdentitySnapshotPayload,
    ) -> Result<CanonicalIdentitySnapshot, BrowserPlatformError> {
        let mut state = self.lock_state()?;
        let generation = state
            .current
            .as_ref()
            .map_or(Ok(1), |snapshot| snapshot.generation.checked_add(1).ok_or_else(
                identity_generation_exhausted,
            ))?;
        // Wall clocks may move backwards. Publication metadata remains
        // monotonic so it can be compared deterministically in inventory/UI.
        let issued_at_ms = self.clock.now_ms().max(state.last_issued_at_ms);
        let snapshot = CanonicalIdentitySnapshot {
            generation,
            issued_at_ms,
            coverage,
        };
        state.last_issued_at_ms = issued_at_ms;
        state.current = Some(snapshot.clone());
        state.current_payload = Some(payload);
        state.replica_admission_open = true;
        Ok(snapshot)
    }

    /// Retains the generation watermark while making its payload unusable for
    /// existing or newly opened replicas. A later successful publication
    /// advances the generation and reopens replica admission.
    pub(crate) fn invalidate_current_snapshot(
        &self,
    ) -> Result<Option<CanonicalIdentitySnapshot>, BrowserPlatformError> {
        let mut state = self.lock_state()?;
        state.replica_admission_open = false;
        state.current_payload = None;
        Ok(state.current.clone())
    }

    pub(crate) fn current_snapshot(
        &self,
    ) -> Result<Option<CanonicalIdentitySnapshot>, BrowserPlatformError> {
        Ok(self.lock_state()?.current.clone())
    }

    pub(crate) fn require_published_snapshot(
        &self,
    ) -> Result<CanonicalIdentitySnapshot, BrowserPlatformError> {
        let state = self.lock_state()?;
        let current = state.current.as_ref().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::NeedsPrimaryIdentity,
                "No authenticated browser identity snapshot is available.",
                true,
                "Open the Primary browser identity and publish a fresh snapshot.",
            )
            .with_metadata(json!({
                "current_generation": null,
                "snapshot_available": false,
                "refresh_required": true,
            }))
        })?;
        if !state.replica_admission_open {
            return Err(stale_snapshot_requires_primary_error(current));
        }
        Ok(current.clone())
    }

    /// Returns the published snapshot only when `requested_generation` is the
    /// current canonical generation.
    pub(crate) fn require_current_snapshot(
        &self,
        requested_generation: u64,
    ) -> Result<CanonicalIdentitySnapshot, BrowserPlatformError> {
        let state = self.lock_state()?;
        let current = state.current.as_ref().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::NeedsPrimaryIdentity,
                "No authenticated browser identity snapshot is available.",
                true,
                "Open the Primary browser identity and publish a fresh snapshot.",
            )
            .with_metadata(json!({
                "requested_generation": requested_generation,
                "current_generation": null,
                "snapshot_available": false,
                "refresh_required": true,
            }))
        })?;
        if !state.replica_admission_open {
            return Err(invalidated_identity_error(requested_generation, current));
        }

        if current.generation == requested_generation {
            return Ok(current.clone());
        }

        let generation_relation = if requested_generation < current.generation {
            "older"
        } else {
            "newer"
        };
        Err(BrowserPlatformError::new(
            BrowserErrorCode::IdentityReplicaStale,
            "The authenticated browser identity generation is not current.",
            true,
            "Reopen the replica from the browser session hub.",
        )
        .with_metadata(json!({
            "requested_generation": requested_generation,
            "current_generation": current.generation,
            "snapshot_available": true,
            "snapshot_issued_at_ms": current.issued_at_ms,
            "generation_relation": generation_relation,
            "refresh_required": true,
        })))
    }

    pub(crate) fn require_current_payload(
        &self,
        requested_generation: u64,
    ) -> Result<IdentitySnapshotPayload, BrowserPlatformError> {
        let state = self.lock_state()?;
        let current = state.current.as_ref().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::NeedsPrimaryIdentity,
                "No authenticated browser identity snapshot is available.",
                true,
                "Open the Primary browser identity and publish a fresh snapshot.",
            )
            .with_metadata(json!({
                "requested_generation": requested_generation,
                "current_generation": null,
                "snapshot_available": false,
                "refresh_required": true,
            }))
        })?;
        if !state.replica_admission_open {
            return Err(invalidated_identity_error(requested_generation, current));
        }
        if current.generation != requested_generation {
            return Err(stale_identity_error(requested_generation, current));
        }
        state.current_payload.clone().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::NeedsPrimaryIdentity,
                "The authenticated browser identity payload is unavailable.",
                true,
                "Capture and publish the Primary browser identity again.",
            )
            .with_metadata(json!({
                "requested_generation": requested_generation,
                "current_generation": current.generation,
                "snapshot_available": false,
                "refresh_required": true,
            }))
        })
    }

    fn lock_state(
        &self,
    ) -> Result<MutexGuard<'_, IdentityGenerationState>, BrowserPlatformError> {
        self.state.lock().map_err(|_| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "Browser identity coordination is temporarily unavailable.",
                true,
                "Retry after the browser platform is ready.",
            )
        })
    }
}

fn identity_generation_exhausted() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser identity generation cannot be advanced.",
        false,
        "Restart the application before publishing another identity snapshot.",
    )
}

fn stale_identity_error(
    requested_generation: u64,
    current: &CanonicalIdentitySnapshot,
) -> BrowserPlatformError {
    let generation_relation = if requested_generation < current.generation {
        "older"
    } else {
        "newer"
    };
    BrowserPlatformError::new(
        BrowserErrorCode::IdentityReplicaStale,
        "The authenticated browser identity generation is not current.",
        true,
        "Reopen the replica from the browser session hub.",
    )
    .with_metadata(json!({
        "requested_generation": requested_generation,
        "current_generation": current.generation,
        "snapshot_available": true,
        "snapshot_issued_at_ms": current.issued_at_ms,
        "generation_relation": generation_relation,
        "refresh_required": true,
    }))
}

fn invalidated_identity_error(
    requested_generation: u64,
    current: &CanonicalIdentitySnapshot,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::IdentityReplicaStale,
        "The authenticated browser identity generation was invalidated after Primary refresh failed.",
        true,
        "Capture and publish the Primary browser identity again.",
    )
    .with_metadata(json!({
        "requested_generation": requested_generation,
        "current_generation": current.generation,
        "snapshot_available": true,
        "snapshot_issued_at_ms": current.issued_at_ms,
        "snapshot_stale": true,
        "generation_relation": "invalidated",
        "refresh_required": true,
    }))
}

fn stale_snapshot_requires_primary_error(
    current: &CanonicalIdentitySnapshot,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::NeedsPrimaryIdentity,
        "The authenticated browser identity snapshot requires a fresh Primary capture.",
        true,
        "Capture and publish the Primary browser identity again.",
    )
    .with_metadata(json!({
        "current_generation": current.generation,
        "snapshot_available": true,
        "snapshot_issued_at_ms": current.issued_at_ms,
        "snapshot_stale": true,
        "refresh_required": true,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::ManualClock;

    fn payload(label: &str) -> IdentitySnapshotPayload {
        IdentitySnapshotPayload::from_json(json!({ "payload": label }))
    }

    #[test]
    fn publish_assigns_authoritative_monotonic_generations_and_timestamps() {
        let clock = Arc::new(ManualClock::new(1_000));
        let coordinator = IdentityGenerationCoordinator::new(clock.clone());

        let first = coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://one.example",
            ), payload("one"))
            .unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.issued_at_ms, 1_000);

        clock.set(2_000);
        let second = coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://two.example",
            ), payload("two"))
            .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(second.issued_at_ms, 2_000);
        assert_eq!(coordinator.current_snapshot().unwrap(), Some(second));
    }

    #[test]
    fn issued_at_does_not_regress_when_wall_clock_moves_backwards() {
        let clock = Arc::new(ManualClock::new(2_000));
        let coordinator = IdentityGenerationCoordinator::new(clock.clone());
        coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://example.test",
            ), payload("first"))
            .unwrap();

        clock.set(1_000);
        let second = coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://example.test",
            ), payload("second"))
            .unwrap();

        assert_eq!(second.issued_at_ms, 2_000);
    }

    #[test]
    fn concurrent_publishers_receive_one_strict_sequence() {
        let coordinator = IdentityGenerationCoordinator::new(Arc::new(ManualClock::new(7)));
        let handles = (0..32)
            .map(|_| {
                let coordinator = coordinator.clone();
                thread::spawn(move || {
                    coordinator
                        .publish_snapshot(SnapshotCoverage::current_origin(
                            "https://example.test",
                        ), payload("concurrent"))
                        .unwrap()
                        .generation
                })
            })
            .collect::<Vec<_>>();
        let generations = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<BTreeSet<_>>();

        assert_eq!(generations, (1..=32).collect());
    }

    #[test]
    fn current_generation_is_accepted_and_old_or_future_is_stale() {
        let coordinator = IdentityGenerationCoordinator::new(Arc::new(ManualClock::new(55)));
        coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://example.test",
            ), payload("first"))
            .unwrap();
        let current = coordinator
            .publish_snapshot(SnapshotCoverage::current_origin(
                "https://example.test",
            ), payload("second"))
            .unwrap();

        assert_eq!(
            coordinator.require_current_snapshot(2).unwrap(),
            current
        );

        for (requested, relation) in [(1, "older"), (3, "newer")] {
            let error = coordinator
                .require_current_snapshot(requested)
                .unwrap_err();
            assert_eq!(error.code, BrowserErrorCode::IdentityReplicaStale);
            assert!(error.retryable);
            assert_eq!(
                error.metadata,
                json!({
                    "requested_generation": requested,
                    "current_generation": 2,
                    "snapshot_available": true,
                    "snapshot_issued_at_ms": 55,
                    "generation_relation": relation,
                    "refresh_required": true,
                })
            );
        }
    }

    #[test]
    fn unpublished_snapshot_requires_primary_identity() {
        let coordinator = IdentityGenerationCoordinator::new(Arc::new(ManualClock::new(55)));
        let error = coordinator.require_current_snapshot(1).unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(
            error.metadata,
            json!({
                "requested_generation": 1,
                "current_generation": null,
                "snapshot_available": false,
                "refresh_required": true,
            })
        );
    }

    #[test]
    fn invalidated_generation_blocks_replicas_until_a_fresh_publish() {
        let coordinator = IdentityGenerationCoordinator::new(Arc::new(ManualClock::new(55)));
        let first = coordinator
            .publish_snapshot(
                SnapshotCoverage::cookies_only(),
                payload("first"),
            )
            .unwrap();

        assert_eq!(
            coordinator.invalidate_current_snapshot().unwrap(),
            Some(first.clone())
        );
        let existing = coordinator
            .require_current_snapshot(first.generation)
            .unwrap_err();
        assert_eq!(existing.code, BrowserErrorCode::IdentityReplicaStale);
        assert_eq!(existing.metadata["snapshot_stale"], true);
        assert_eq!(
            existing.metadata["generation_relation"],
            "invalidated"
        );

        let new_replica = coordinator.require_published_snapshot().unwrap_err();
        assert_eq!(new_replica.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(new_replica.metadata["current_generation"], 1);
        assert_eq!(new_replica.metadata["snapshot_stale"], true);

        let second = coordinator
            .publish_snapshot(
                SnapshotCoverage::cookies_only(),
                payload("second"),
            )
            .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(
            coordinator.require_current_snapshot(second.generation).unwrap(),
            second
        );
        assert_eq!(
            coordinator
                .require_current_snapshot(first.generation)
                .unwrap_err()
                .code,
            BrowserErrorCode::IdentityReplicaStale
        );
    }

    #[test]
    fn coverage_wire_contract_explicitly_excludes_unsupported_identity_state() {
        let coverage = SnapshotCoverage::current_origin("https://example.test");

        assert_eq!(
            serde_json::to_value(coverage).unwrap(),
            json!({
                "current_origin": "https://example.test",
                "cookies": "all_origins",
                "local_storage": "current_origin",
                "indexed_db": "current_origin",
                "cache": "not_included",
                "service_workers": "not_included",
                "session_storage": "not_included",
                "passkeys": "not_included",
                "device_bound_credentials": "not_included",
            })
        );

        assert_eq!(
            serde_json::to_value(SnapshotCoverage::cookies_only()).unwrap(),
            json!({
                "cookies": "all_origins",
                "local_storage": "not_included",
                "indexed_db": "not_included",
                "cache": "not_included",
                "service_workers": "not_included",
                "session_storage": "not_included",
                "passkeys": "not_included",
                "device_bound_credentials": "not_included",
            })
        );
    }

    #[test]
    fn identity_payload_debug_output_is_always_redacted() {
        let payload = IdentitySnapshotPayload::from_json(json!({
            "cookies": [{"name": "session", "value": "top-secret"}],
        }));

        let debug = format!("{payload:?}");
        assert_eq!(debug, "IdentitySnapshotPayload(<redacted>)");
        assert!(!debug.contains("top-secret"));
        assert!(!debug.contains("session"));
    }
}
