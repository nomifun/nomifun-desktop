//! In-process linearization fence for D-026 Remote authentication admission.
//!
//! A Remote request holds a shared permit from credential validation through
//! its durable admission commit. Token rotate or revoke holds an exclusive
//! permit through its durable commit and publication of the new authentication
//! state. Work that was durably admitted first may continue after releasing
//! its permit; a mutation that commits first is observed by every later
//! admission.
//!
//! This primitive owns no token, owner, Session, or repository state. Callers
//! remain responsible for authentication, persistence, and mapping a rejected
//! credential to their public error contract.

use std::fmt;
use std::sync::Arc;

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// Shared request-admission versus exclusive authentication-mutation fence.
///
/// The underlying Tokio lock is fair and write-preferring, so a queued auth
/// mutation cannot be starved by later request admissions.
#[derive(Clone, Default)]
pub struct RemoteAuthAdmissionFence {
    gate: Arc<RwLock<()>>,
}

impl RemoteAuthAdmissionFence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire shared authority for one Remote request admission.
    ///
    /// The caller should validate the presented credential while holding this
    /// permit and retain it until the request's durable admission transaction
    /// commits. The permit is not needed for work after durable admission.
    pub async fn acquire_request_admission(&self) -> RemoteRequestAdmissionPermit {
        RemoteRequestAdmissionPermit {
            _guard: self.gate.clone().read_owned().await,
        }
    }

    /// Acquire exclusive authority for a token rotate or revoke mutation.
    ///
    /// The caller should retain this permit until the auth mutation commits
    /// durably and the corresponding in-process authentication state has been
    /// published.
    pub async fn acquire_auth_mutation(&self) -> RemoteAuthMutationPermit {
        RemoteAuthMutationPermit {
            _guard: self.gate.clone().write_owned().await,
        }
    }
}

impl fmt::Debug for RemoteAuthAdmissionFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteAuthAdmissionFence")
            .finish_non_exhaustive()
    }
}

/// Shared capability held while a Remote request is being durably admitted.
#[must_use = "dropping the permit releases the request-admission fence"]
pub struct RemoteRequestAdmissionPermit {
    _guard: OwnedRwLockReadGuard<()>,
}

impl fmt::Debug for RemoteRequestAdmissionPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteRequestAdmissionPermit")
            .finish_non_exhaustive()
    }
}

/// Exclusive capability held while Remote authentication is mutated.
#[must_use = "dropping the permit releases the auth-mutation fence"]
pub struct RemoteAuthMutationPermit {
    _guard: OwnedRwLockWriteGuard<()>,
}

impl fmt::Debug for RemoteAuthMutationPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteAuthMutationPermit")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;

    const BLOCKED_CHECK: Duration = Duration::from_millis(20);
    const TEST_DEADLINE: Duration = Duration::from_secs(1);

    fn record(order: &Mutex<Vec<&'static str>>, event: &'static str) {
        order.lock().expect("ordering lock poisoned").push(event);
    }

    #[tokio::test]
    async fn request_first_holds_mutation_until_durable_admission_commits() {
        let fence = Arc::new(RemoteAuthAdmissionFence::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        let request_permit = fence.acquire_request_admission().await;
        record(&order, "request_authenticated");

        let (mutation_started_tx, mutation_started_rx) = oneshot::channel();
        let (mutation_committed_tx, mut mutation_committed_rx) = oneshot::channel();
        let mutation = {
            let fence = Arc::clone(&fence);
            let order = Arc::clone(&order);
            tokio::spawn(async move {
                mutation_started_tx
                    .send(())
                    .expect("request-first test receiver dropped");
                let _permit = fence.acquire_auth_mutation().await;
                record(&order, "auth_mutation_committed");
                mutation_committed_tx
                    .send(())
                    .expect("request-first test receiver dropped");
            })
        };

        mutation_started_rx
            .await
            .expect("auth mutation task did not start");
        assert!(
            tokio::time::timeout(BLOCKED_CHECK, &mut mutation_committed_rx)
                .await
                .is_err(),
            "auth mutation committed while request admission held a shared permit"
        );

        tokio::task::yield_now().await;
        record(&order, "request_admission_committed");
        drop(request_permit);

        tokio::time::timeout(TEST_DEADLINE, &mut mutation_committed_rx)
            .await
            .expect("auth mutation did not acquire the exclusive permit")
            .expect("auth mutation task dropped its completion signal");
        mutation.await.expect("auth mutation task panicked");

        assert_eq!(
            *order.lock().expect("ordering lock poisoned"),
            [
                "request_authenticated",
                "request_admission_committed",
                "auth_mutation_committed",
            ]
        );
    }

    #[tokio::test]
    async fn auth_first_rejects_old_credential_before_downstream_lookup() {
        let fence = Arc::new(RemoteAuthAdmissionFence::new());
        let old_credential_valid = Arc::new(AtomicBool::new(true));
        let downstream_lookups = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));

        let mutation_permit = fence.acquire_auth_mutation().await;
        let (request_started_tx, request_started_rx) = oneshot::channel();
        let (request_finished_tx, mut request_finished_rx) = oneshot::channel();
        let request = {
            let fence = Arc::clone(&fence);
            let old_credential_valid = Arc::clone(&old_credential_valid);
            let downstream_lookups = Arc::clone(&downstream_lookups);
            let order = Arc::clone(&order);
            tokio::spawn(async move {
                request_started_tx
                    .send(())
                    .expect("auth-first test receiver dropped");
                let _permit = fence.acquire_request_admission().await;
                let admitted = old_credential_valid.load(Ordering::Acquire);
                if admitted {
                    downstream_lookups.fetch_add(1, Ordering::AcqRel);
                    record(&order, "request_admission_committed");
                } else {
                    record(&order, "old_credential_rejected");
                }
                request_finished_tx
                    .send(admitted)
                    .expect("auth-first test receiver dropped");
            })
        };

        request_started_rx
            .await
            .expect("request admission task did not start");
        assert!(
            tokio::time::timeout(BLOCKED_CHECK, &mut request_finished_rx)
                .await
                .is_err(),
            "request admission passed an exclusive auth mutation permit"
        );

        old_credential_valid.store(false, Ordering::Release);
        record(&order, "auth_mutation_committed");
        drop(mutation_permit);

        let admitted = tokio::time::timeout(TEST_DEADLINE, &mut request_finished_rx)
            .await
            .expect("request did not acquire a shared permit after auth mutation")
            .expect("request task dropped its completion signal");
        request.await.expect("request admission task panicked");

        assert!(!admitted);
        assert_eq!(downstream_lookups.load(Ordering::Acquire), 0);
        assert_eq!(
            *order.lock().expect("ordering lock poisoned"),
            ["auth_mutation_committed", "old_credential_rejected"]
        );
    }

    #[tokio::test]
    async fn request_admission_permits_are_shared() {
        let fence = RemoteAuthAdmissionFence::new();
        let _first = fence.acquire_request_admission().await;

        let _second = tokio::time::timeout(TEST_DEADLINE, fence.acquire_request_admission())
            .await
            .expect("a request admission permit blocked another reader");
    }
}
