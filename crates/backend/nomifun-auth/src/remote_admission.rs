//! In-process mutation gate for Remote authentication.
//!
//! Request authentication is linearized by the short-lived synchronous
//! [`crate::InstanceTokenValidator`] state. Only token mint/rotate/revoke
//! operations need serialization around their durable repository mutation and
//! publication of the new validator state.
//!
//! This primitive owns no token, owner, Session, or repository state. Callers
//! remain responsible for authentication, persistence, and mapping a rejected
//! credential to their public error contract.

use std::fmt;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard};

/// Serializes token mint/rotate/revoke mutations.
#[derive(Clone, Default)]
pub struct RemoteAuthAdmissionFence {
    gate: Arc<Mutex<()>>,
}

impl RemoteAuthAdmissionFence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire exclusive authority for a token mutation.
    ///
    /// The caller retains this permit until the repository mutation commits and
    /// the corresponding in-process authentication state has been published.
    pub async fn acquire_auth_mutation(&self) -> RemoteAuthMutationPermit {
        RemoteAuthMutationPermit {
            _guard: self.gate.clone().lock_owned().await,
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

/// Exclusive capability held while Remote authentication is mutated.
#[must_use = "dropping the permit releases the auth-mutation fence"]
pub struct RemoteAuthMutationPermit {
    _guard: OwnedMutexGuard<()>,
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
    async fn token_mutations_are_serialized() {
        let fence = Arc::new(RemoteAuthAdmissionFence::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        let first = fence.acquire_auth_mutation().await;
        record(&order, "first_mutation_started");

        let (second_started_tx, second_started_rx) = oneshot::channel();
        let (second_committed_tx, mut second_committed_rx) = oneshot::channel();
        let second = {
            let fence = Arc::clone(&fence);
            let order = Arc::clone(&order);
            tokio::spawn(async move {
                second_started_tx
                    .send(())
                    .expect("mutation test receiver dropped");
                let _permit = fence.acquire_auth_mutation().await;
                record(&order, "second_mutation_committed");
                second_committed_tx
                    .send(())
                    .expect("mutation test receiver dropped");
            })
        };

        second_started_rx
            .await
            .expect("second mutation task did not start");
        assert!(
            tokio::time::timeout(BLOCKED_CHECK, &mut second_committed_rx)
                .await
                .is_err(),
            "second mutation committed while the first mutation held the gate"
        );

        record(&order, "first_mutation_committed");
        drop(first);

        tokio::time::timeout(TEST_DEADLINE, &mut second_committed_rx)
            .await
            .expect("second mutation did not acquire the gate")
            .expect("second mutation task dropped its completion signal");
        second.await.expect("second mutation task panicked");

        assert_eq!(
            *order.lock().expect("ordering lock poisoned"),
            [
                "first_mutation_started",
                "first_mutation_committed",
                "second_mutation_committed",
            ]
        );
    }

    #[tokio::test]
    async fn a_dropped_mutation_permit_reopens_the_gate() {
        let fence = RemoteAuthAdmissionFence::new();
        let first = fence.acquire_auth_mutation().await;
        drop(first);
        let _second = tokio::time::timeout(TEST_DEADLINE, fence.acquire_auth_mutation())
            .await
            .expect("dropping a mutation permit did not reopen the gate");
    }
}
