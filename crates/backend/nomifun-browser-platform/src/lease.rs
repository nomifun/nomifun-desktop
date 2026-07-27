use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::{
    BrowserErrorCode, BrowserOperationKind, BrowserPlatformError, BrowserSurface,
    CallerIdentity, Clock, OwnerLeaseId,
};

const MINIMUM_TTL_MS: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerLease {
    pub lease_id: OwnerLeaseId,
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub runtime_instance_id: String,
    /// The first trusted capability bound to this lease establishes its
    /// surface and operation ceiling.  `None` is retained only for legacy
    /// leases issued before a caller was available; `BrowserSessionHub::bind`
    /// upgrades such leases atomically before exposing a client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<BrowserSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_operations: Option<BTreeSet<BrowserOperationKind>>,
    pub issued_at_ms: u64,
    pub renewed_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone)]
pub struct OwnerLeaseService {
    clock: Arc<dyn Clock>,
    ttl_ms: u64,
    leases: Arc<Mutex<HashMap<OwnerLeaseId, OwnerLease>>>,
}

impl OwnerLeaseService {
    pub fn new(clock: Arc<dyn Clock>, ttl_ms: u64) -> Self {
        Self {
            clock,
            ttl_ms: ttl_ms.max(MINIMUM_TTL_MS),
            leases: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn issue(
        &self,
        user_id: impl Into<String>,
        conversation_id: Option<String>,
        runtime_instance_id: impl Into<String>,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let user_id = user_id.into();
        let runtime_instance_id = runtime_instance_id.into();
        validate_owner_fields(&user_id, &runtime_instance_id)?;

        let now_ms = self.clock.now_ms();
        let lease = OwnerLease {
            lease_id: OwnerLeaseId::new(),
            user_id,
            conversation_id,
            runtime_instance_id,
            surface: None,
            allowed_operations: None,
            issued_at_ms: now_ms,
            renewed_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(self.ttl_ms),
        };
        lock_authority(&self.leases)?.insert(lease.lease_id.clone(), lease.clone());
        Ok(lease)
    }

    pub fn renew(
        &self,
        lease_id: &OwnerLeaseId,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        let Some(lease) = leases.get_mut(lease_id) else {
            return Err(owner_lease_expired());
        };
        if lease.expires_at_ms <= now_ms {
            leases.remove(lease_id);
            return Err(owner_lease_expired());
        }

        lease.renewed_at_ms = now_ms;
        lease.expires_at_ms = now_ms.saturating_add(self.ttl_ms);
        Ok(lease.clone())
    }

    pub fn revoke(&self, lease_id: &OwnerLeaseId) -> bool {
        lock_unpoisoned(&self.leases).remove(lease_id).is_some()
    }

    pub fn validate(
        &self,
        caller: &CallerIdentity,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        caller.validate(now_ms)?;

        let mut leases = lock_authority(&self.leases)?;
        let Some(lease) = leases.get(&caller.owner_lease_id) else {
            return Err(owner_lease_expired());
        };
        if lease.expires_at_ms <= now_ms {
            leases.remove(&caller.owner_lease_id);
            return Err(owner_lease_expired());
        }
        if lease.user_id != caller.user_id
            || lease.runtime_instance_id != caller.runtime_instance_id
            || lease.conversation_id != caller.conversation_id
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser capability does not match its owner lease.",
                false,
                "Request a fresh browser capability from the application.",
            ));
        }
        validate_policy_binding(lease, caller)?;
        Ok(lease.clone())
    }

    /// Bind or narrow the trusted policy carried by a lease.
    ///
    /// A lease is deliberately monotonic: a later capability may reduce its
    /// operation set, but it may never broaden it or change surfaces in place.
    /// This makes an old, broader `CallerIdentity` fail closed after a
    /// capability is renewed with a narrower scope, without requiring a
    /// second mutable generation field in every wire identity.
    pub fn bind_policy(
        &self,
        caller: &CallerIdentity,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        caller.validate(now_ms)?;
        let mut leases = lock_authority(&self.leases)?;
        let Some(lease) = leases.get_mut(&caller.owner_lease_id) else {
            return Err(owner_lease_expired());
        };
        if lease.expires_at_ms <= now_ms {
            leases.remove(&caller.owner_lease_id);
            return Err(owner_lease_expired());
        }
        if lease.user_id != caller.user_id
            || lease.runtime_instance_id != caller.runtime_instance_id
            || lease.conversation_id != caller.conversation_id
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser capability does not match its owner lease.",
                false,
                "Request a fresh browser capability from the application.",
            ));
        }
        if caller.allowed_operations.is_empty() {
            return Err(empty_policy_scope_error());
        }

        match (lease.surface, lease.allowed_operations.as_ref()) {
            (None, None) => {
                lease.surface = Some(caller.surface);
                lease.allowed_operations = Some(caller.allowed_operations.clone());
            }
            (Some(surface), Some(allowed)) => {
                if surface != caller.surface
                    || !caller.allowed_operations.is_subset(allowed)
                {
                    return Err(policy_narrowing_error());
                }
                // Persist a narrower scope so stale broader identities bound
                // to the same owner are rejected on their next Hub call.
                if caller.allowed_operations.len() < allowed.len() {
                    lease.allowed_operations = Some(caller.allowed_operations.clone());
                }
            }
            _ => {
                return Err(incomplete_policy_error());
            }
        }
        Ok(lease.clone())
    }

    /// Removes expired leases and returns the number removed.
    pub fn sweep(&self) -> usize {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_unpoisoned(&self.leases);
        let before = leases.len();
        leases.retain(|_, lease| lease.expires_at_ms > now_ms);
        before - leases.len()
    }

    /// Removes expired leases and returns their opaque IDs.
    ///
    /// The Hub needs the exact owner identity to clean the corresponding
    /// lanes. Returning runtime IDs would be too broad: a replacement
    /// capability may legitimately reuse a runtime identifier while an older
    /// lease is being swept.
    pub fn sweep_expired_ids(&self) -> Vec<OwnerLeaseId> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_unpoisoned(&self.leases);
        let expired = leases
            .iter()
            .filter(|(_, lease)| lease.expires_at_ms <= now_ms)
            .map(|(lease_id, _)| lease_id.clone())
            .collect::<Vec<_>>();
        for lease_id in &expired {
            leases.remove(lease_id);
        }
        expired
    }
}

fn validate_owner_fields(
    user_id: &str,
    runtime_instance_id: &str,
) -> Result<(), BrowserPlatformError> {
    if user_id.trim().is_empty() || runtime_instance_id.trim().is_empty() {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "An owner lease requires a user and runtime instance.",
            false,
            "Request a fresh browser capability from the application.",
        ));
    }
    Ok(())
}

fn owner_lease_expired() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OwnerLeaseExpired,
        "The browser owner lease has expired or was revoked.",
        false,
        "Request a fresh browser capability.",
    )
}

fn validate_policy_binding(
    lease: &OwnerLease,
    caller: &CallerIdentity,
) -> Result<(), BrowserPlatformError> {
    let (Some(surface), Some(allowed)) =
        (lease.surface, lease.allowed_operations.as_ref())
    else {
        return Err(incomplete_policy_error());
    };
    if allowed.is_empty() || caller.allowed_operations.is_empty() {
        return Err(empty_policy_scope_error());
    }
    if surface != caller.surface || !caller.allowed_operations.is_subset(allowed) {
        return Err(policy_narrowing_error());
    }
    Ok(())
}

fn incomplete_policy_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser owner policy is incomplete.",
        false,
        "Bind a fresh browser capability before using this owner lease.",
    )
}

fn empty_policy_scope_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser owner policy must allow at least one operation.",
        false,
        "Request a fresh capability with an explicit browser operation scope.",
    )
}

fn policy_narrowing_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser capability is broader than the current owner policy.",
        false,
        "Request a fresh capability with the current browser operation scope.",
    )
}

fn lease_state_unavailable() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "Browser lease authority is temporarily unavailable.",
        true,
        "Retry after the browser platform has restarted its lease authority.",
    )
}

fn lock_authority<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, BrowserPlatformError> {
    mutex.lock().map_err(|_| lease_state_unavailable())
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use crate::{BrowserOperationKind, BrowserSurface, ManualClock};

    fn caller_for(lease: &OwnerLease, capability_expires_at_ms: u64) -> CallerIdentity {
        CallerIdentity {
            user_id: lease.user_id.clone(),
            conversation_id: lease.conversation_id.clone(),
            runtime_instance_id: lease.runtime_instance_id.clone(),
            agent_id: Some("agent-1".to_owned()),
            companion_id: None,
            execution_id: Some("execution-1".to_owned()),
            step_id: None,
            attempt_id: None,
            remote_connection_id: None,
            surface: BrowserSurface::Native,
            owner_lease_id: lease.lease_id.clone(),
            capability_expires_at_ms,
            allowed_operations: BTreeSet::from([BrowserOperationKind::Navigate]),
        }
    }

    fn poison<T>(mutex: &Mutex<T>) {
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _guard = mutex.lock().unwrap();
                panic!("intentional lease authority poison");
            }))
            .is_err()
        );
    }

    #[test]
    fn owner_lease_is_bound_renewable_and_revocable() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 100);
        let lease = service
            .issue("user-1", Some("conversation-1".to_owned()), "runtime-1")
            .unwrap();
        let caller = caller_for(&lease, 5_000);
        let bound = service.bind_policy(&caller).unwrap();
        assert_eq!(service.validate(&caller).unwrap(), bound);

        let mut mismatched = caller.clone();
        mismatched.runtime_instance_id = "runtime-2".to_owned();
        assert_eq!(
            service.validate(&mismatched).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        clock.advance(50);
        let renewed = service.renew(&lease.lease_id).unwrap();
        assert_eq!(renewed.expires_at_ms, 1_150);
        clock.advance(99);
        assert!(service.validate(&caller).is_ok());

        assert!(service.revoke(&lease.lease_id));
        assert!(!service.revoke(&lease.lease_id));
        assert_eq!(
            service.validate(&caller).unwrap_err().code,
            BrowserErrorCode::OwnerLeaseExpired
        );
    }

    #[test]
    fn owner_lease_validate_rejects_unbound_and_partially_bound_policy() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);

        for (surface, allowed_operations) in [
            (None, None),
            (Some(BrowserSurface::Native), None),
            (
                None,
                Some(BTreeSet::from([BrowserOperationKind::Navigate])),
            ),
            (Some(BrowserSurface::Native), Some(BTreeSet::new())),
        ] {
            let lease = service.issue("user", None, "runtime").unwrap();
            {
                let mut leases = lock_unpoisoned(&service.leases);
                let stored = leases.get_mut(&lease.lease_id).unwrap();
                stored.surface = surface;
                stored.allowed_operations = allowed_operations.clone();
            }

            assert_eq!(
                service
                    .validate(&caller_for(&lease, 5_000))
                    .unwrap_err()
                    .code,
                BrowserErrorCode::InvalidCallerIdentity
            );
        }
    }

    #[test]
    fn owner_lease_bind_policy_rejects_empty_scope_without_mutating_policy() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let lease = service.issue("user", None, "runtime").unwrap();
        let mut empty = caller_for(&lease, 5_000);
        empty.allowed_operations.clear();

        assert_eq!(
            service.bind_policy(&empty).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        let stored = lock_unpoisoned(&service.leases)
            .get(&lease.lease_id)
            .cloned()
            .unwrap();
        assert_eq!(stored.surface, None);
        assert_eq!(stored.allowed_operations, None);

        let bound = service
            .bind_policy(&caller_for(&lease, 5_000))
            .unwrap();
        assert_eq!(
            service.validate(&empty).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        assert_eq!(
            service.bind_policy(&empty).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        assert_eq!(
            lock_unpoisoned(&service.leases)
                .get(&lease.lease_id)
                .cloned()
                .unwrap(),
            bound
        );
    }

    #[test]
    fn owner_lease_policy_can_bind_narrow_and_survive_renewal() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 100);
        let lease = service.issue("user", None, "runtime").unwrap();
        let mut broad = caller_for(&lease, 5_000);
        broad
            .allowed_operations
            .insert(BrowserOperationKind::Observe);

        let bound = service.bind_policy(&broad).unwrap();
        assert_eq!(bound.surface, Some(BrowserSurface::Native));
        assert_eq!(
            bound.allowed_operations,
            Some(broad.allowed_operations.clone())
        );

        let mut narrow = broad.clone();
        narrow
            .allowed_operations
            .remove(&BrowserOperationKind::Observe);
        let narrowed = service.bind_policy(&narrow).unwrap();
        assert_eq!(
            narrowed.allowed_operations,
            Some(narrow.allowed_operations.clone())
        );
        assert!(service.validate(&narrow).is_ok());
        assert_eq!(
            service.validate(&broad).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        clock.advance(50);
        let renewed = service.renew(&lease.lease_id).unwrap();
        assert_eq!(renewed.expires_at_ms, 1_150);
        assert_eq!(renewed.surface, narrowed.surface);
        assert_eq!(renewed.allowed_operations, narrowed.allowed_operations);
        assert!(service.validate(&narrow).is_ok());
    }

    #[test]
    fn owner_lease_expiry_and_sweep_use_the_injected_clock() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 20);
        let first = service.issue("user", None, "runtime-1").unwrap();
        let second = service.issue("user", None, "runtime-2").unwrap();

        clock.set(30);
        assert_eq!(service.sweep(), 2);
        assert_eq!(
            service.renew(&first.lease_id).unwrap_err().code,
            BrowserErrorCode::OwnerLeaseExpired
        );
        assert_eq!(
            service
                .validate(&caller_for(&second, 100))
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );
    }

    #[test]
    fn poisoned_owner_lease_authority_fails_closed() {
        let clock = ManualClock::new(100);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let lease = service.issue("user", None, "runtime").unwrap();
        let caller = caller_for(&lease, 1_000);
        service.bind_policy(&caller).unwrap();
        poison(&service.leases);

        assert_eq!(
            service.validate(&caller).unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.renew(&lease.lease_id).unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.issue("other", None, "other-runtime").unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
    }

}
