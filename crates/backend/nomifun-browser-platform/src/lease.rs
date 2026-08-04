use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    BrowserErrorCode, BrowserOperationKind, BrowserPlatformError, BrowserSurface,
    CallerIdentity, Clock, MAX_BROWSER_IDENTITY_FIELD_BYTES, OwnerLeaseId,
    TaskResourceFamilyKey,
};

const MINIMUM_TTL_MS: u64 = 1;
/// Per logical task-family bound for retained owner generations. Expired
/// entries deliberately remain charged until the Hub's authoritative sweep
/// consumes their exact cleanup authority.
const MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerLease {
    pub lease_id: OwnerLeaseId,
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub runtime_instance_id: String,
    /// Immutable resource family established by the first trusted bind.
    /// Cleanup still uses the exact lease/runtime; this value is quota-only.
    #[serde(skip)]
    pub task_resource_family_key: Option<TaskResourceFamilyKey>,
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
    accepting: Arc<AtomicBool>,
}

impl OwnerLeaseService {
    pub fn new(clock: Arc<dyn Clock>, ttl_ms: u64) -> Self {
        Self {
            clock,
            ttl_ms: ttl_ms.max(MINIMUM_TTL_MS),
            leases: Arc::new(Mutex::new(HashMap::new())),
            accepting: Arc::new(AtomicBool::new(true)),
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
        validate_owner_fields(
            &user_id,
            conversation_id.as_deref(),
            &runtime_instance_id,
        )?;

        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }

        // The trusted pre-bind boundary does not have the richer caller family
        // yet. Conversation-scoped runtimes share one task family; callers
        // without a conversation fall back to their exact runtime. Count and
        // insert under the same lock so concurrent issuance cannot overshoot.
        let family_discriminator = conversation_id
            .as_deref()
            .unwrap_or(runtime_instance_id.as_str());
        let active_for_family = leases
            .values()
            .filter(|existing| {
                existing.user_id == user_id
                    && existing
                        .conversation_id
                        .as_deref()
                        .unwrap_or(existing.runtime_instance_id.as_str())
                        == family_discriminator
            })
            .count();
        if active_for_family >= MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            return Err(owner_lease_task_capacity_error(
                active_for_family,
                conversation_id.is_some(),
            ));
        }

        let lease = OwnerLease {
            lease_id: OwnerLeaseId::new(),
            user_id,
            conversation_id,
            runtime_instance_id,
            task_resource_family_key: None,
            surface: None,
            allowed_operations: None,
            issued_at_ms: now_ms,
            renewed_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(self.ttl_ms),
        };
        leases.insert(lease.lease_id.clone(), lease.clone());
        Ok(lease)
    }

    pub fn renew(
        &self,
        lease_id: &OwnerLeaseId,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        let Some(lease) = leases.get_mut(lease_id) else {
            return Err(owner_lease_expired());
        };
        if lease.expires_at_ms <= now_ms {
            // Keep the exact opaque id until the authoritative Hub sweep can
            // drain it and close every Lane owned by this lease. Removing it
            // here would make a failed late renew consume cleanup authority.
            return Err(owner_lease_expired());
        }

        lease.renewed_at_ms = now_ms;
        lease.expires_at_ms = now_ms.saturating_add(self.ttl_ms);
        Ok(lease.clone())
    }

    pub fn revoke(&self, lease_id: &OwnerLeaseId) -> bool {
        lock_unpoisoned(&self.leases).remove(lease_id).is_some()
    }

    /// Permanently closes this lease authority and revokes every outstanding
    /// capability.
    ///
    /// The accepting flag is changed before taking the registry lock, while
    /// issue/renew re-check it under that same lock. Therefore an operation
    /// that won the lock first is subsequently cleared, and one that wins it
    /// later fails closed; no lease can appear behind terminal shutdown.
    pub fn stop_accepting_and_clear(&self) -> usize {
        self.accepting.store(false, Ordering::Release);
        let mut leases = lock_unpoisoned(&self.leases);
        let revoked = leases.len();
        leases.clear();
        revoked
    }

    pub fn validate(
        &self,
        caller: &CallerIdentity,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        caller.validate(now_ms)?;

        let leases = lock_authority(&self.leases)?;
        let Some(lease) = leases.get(&caller.owner_lease_id) else {
            return Err(owner_lease_expired());
        };
        if lease.expires_at_ms <= now_ms {
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
        if lease.task_resource_family_key.as_ref()
            != Some(&caller.task_resource_family_key())
        {
            return Err(resource_family_binding_error());
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
        let family_key = caller.task_resource_family_key();
        let first_family_bind = {
            let Some(lease) = leases.get(&caller.owner_lease_id) else {
                return Err(owner_lease_expired());
            };
            if lease.expires_at_ms <= now_ms {
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
            if lease
                .task_resource_family_key
                .as_ref()
                .is_some_and(|bound| bound != &family_key)
            {
                return Err(resource_family_binding_error());
            }
            match (lease.surface, lease.allowed_operations.as_ref()) {
                (None, None) => {}
                (Some(surface), Some(allowed)) => {
                    if surface != caller.surface
                        || !caller.allowed_operations.is_subset(allowed)
                    {
                        return Err(policy_narrowing_error());
                    }
                }
                _ => return Err(incomplete_policy_error()),
            }
            lease.task_resource_family_key.is_none()
        };

        // `issue` can only see the pre-bind conversation/runtime boundary.
        // Execution and Remote identities establish their real logical task
        // family here, so runtime rotation must be fenced under this same map
        // lock before any policy field is changed. Expired-but-unswept leases
        // deliberately count: the Hub has not yet consumed their exact cleanup
        // authority. A narrowed/idempotent rebind of an already sealed lease
        // does not reserve another family slot.
        if first_family_bind {
            let active_for_bound_family = leases
                .values()
                .filter(|existing| {
                    existing.user_id == caller.user_id
                        && existing.task_resource_family_key.as_ref() == Some(&family_key)
                })
                .count();
            if active_for_bound_family >= MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
                return Err(owner_lease_bound_family_capacity_error(
                    active_for_bound_family,
                ));
            }
        }

        let lease = leases
            .get_mut(&caller.owner_lease_id)
            .expect("validated owner lease must remain under the registry lock");

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
        if lease.task_resource_family_key.is_none() {
            lease.task_resource_family_key = Some(family_key);
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
    conversation_id: Option<&str>,
    runtime_instance_id: &str,
) -> Result<(), BrowserPlatformError> {
    if user_id.trim().is_empty()
        || user_id.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
        || runtime_instance_id.trim().is_empty()
        || runtime_instance_id.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
        || conversation_id.is_some_and(|value| {
            value.trim().is_empty()
                || value.len() > MAX_BROWSER_IDENTITY_FIELD_BYTES
        })
    {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "An owner lease requires bounded user, conversation, and runtime identifiers.",
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

fn owner_lease_task_capacity_error(
    active_owner_lease_count: usize,
    conversation_scoped: bool,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "This browser task has reached its owner-lease generation limit.",
        true,
        "Wait for an existing owner lease to be revoked or swept, then retry.",
    )
    .with_metadata(json!({
        "reason_code": "browser_owner_lease_task_capacity",
        "capacity_scope": "task",
        "prebind_family_key_kind": if conversation_scoped {
            "conversation"
        } else {
            "runtime"
        },
        "active_owner_lease_count": active_owner_lease_count,
        "max_active_owner_leases": MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY,
        "expired_leases_count_toward_limit": true,
        "retry_delay_ms": 250,
    }))
}

fn owner_lease_bound_family_capacity_error(
    active_owner_lease_count: usize,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "This browser task has reached its bound owner-lease generation limit.",
        true,
        "Wait for an existing owner lease to be revoked or swept, then retry.",
    )
    .with_metadata(json!({
        "reason_code": "browser_owner_lease_bound_task_capacity",
        "capacity_scope": "task",
        "family_key_kind": "sealed",
        "active_owner_lease_count": active_owner_lease_count,
        "max_active_owner_leases": MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY,
        "expired_leases_count_toward_limit": true,
        "retry_delay_ms": 250,
    }))
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

fn resource_family_binding_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser capability does not match its sealed resource family.",
        false,
        "Request a fresh owner lease for a different logical browser task.",
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
    use std::sync::Barrier;
    use std::thread;

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

    fn caller_for_family(
        lease: &OwnerLease,
        surface: BrowserSurface,
        execution_id: Option<&str>,
        remote_connection_id: Option<&str>,
    ) -> CallerIdentity {
        let mut caller = caller_for(lease, u64::MAX);
        caller.surface = surface;
        caller.execution_id = execution_id.map(str::to_owned);
        caller.remote_connection_id = remote_connection_id.map(str::to_owned);
        caller
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
    fn overlong_owner_fields_fail_before_any_lease_authority_is_inserted() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let overlong_ascii = "x".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES + 1);
        let overlong_utf8 =
            "é".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES / 2 + 1);

        for result in [
            service.issue(&overlong_ascii, None, "runtime"),
            service.issue("user", None, &overlong_ascii),
            service.issue(
                "user",
                Some(overlong_ascii.clone()),
                "runtime",
            ),
            service.issue("user", Some(overlong_utf8), "runtime"),
        ] {
            assert_eq!(
                result.unwrap_err().code,
                BrowserErrorCode::InvalidCallerIdentity
            );
        }
        assert!(
            lock_unpoisoned(&service.leases).is_empty(),
            "invalid owner fields must be rejected before map insertion"
        );

        let exact_utf8 = "é".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES / 2);
        let lease = service
            .issue(
                exact_utf8.clone(),
                Some(exact_utf8.clone()),
                exact_utf8,
            )
            .expect("the UTF-8 byte boundary must remain valid");
        assert_eq!(lock_unpoisoned(&service.leases).len(), 1);
        assert!(service.revoke(&lease.lease_id));
    }

    #[test]
    fn overlong_caller_fields_cannot_bind_or_validate_lease_authority() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let lease = service.issue("user", None, "runtime").unwrap();
        let baseline = caller_for(&lease, 5_000);
        let overlong = "x".repeat(MAX_BROWSER_IDENTITY_FIELD_BYTES + 1);

        let mut invalid_callers = Vec::new();
        let mut caller = baseline.clone();
        caller.user_id = overlong.clone();
        invalid_callers.push(caller);
        let mut caller = baseline.clone();
        caller.runtime_instance_id = overlong.clone();
        invalid_callers.push(caller);
        let mut caller = baseline.clone();
        caller.conversation_id = Some(overlong.clone());
        invalid_callers.push(caller);
        let mut caller = baseline;
        caller.owner_lease_id = OwnerLeaseId(overlong);
        invalid_callers.push(caller);

        for caller in invalid_callers {
            assert_eq!(
                service.bind_policy(&caller).unwrap_err().code,
                BrowserErrorCode::InvalidCallerIdentity
            );
            assert_eq!(
                service.validate(&caller).unwrap_err().code,
                BrowserErrorCode::InvalidCallerIdentity
            );
        }
        let stored = lock_unpoisoned(&service.leases)
            .get(&lease.lease_id)
            .cloned()
            .expect("the original unbound lease must remain present");
        assert_eq!(stored.surface, None);
        assert_eq!(stored.allowed_operations, None);
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
    fn owner_lease_seals_execution_and_remote_resource_families() {
        let clock = ManualClock::new(1_000);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 100);

        let execution_lease = service.issue("user", None, "runtime-a").unwrap();
        let execution = caller_for(&execution_lease, 5_000);
        let execution_bound = service.bind_policy(&execution).unwrap();
        assert_eq!(
            execution_bound.task_resource_family_key.as_ref(),
            Some(&execution.task_resource_family_key())
        );
        let mut changed_execution = execution.clone();
        changed_execution.execution_id = Some("execution-2".to_owned());
        assert_eq!(
            service.bind_policy(&changed_execution).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        assert_eq!(
            service.validate(&changed_execution).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        let remote_lease = service.issue("user", None, "runtime-b").unwrap();
        let mut remote = caller_for(&remote_lease, 5_000);
        remote.surface = BrowserSurface::Remote;
        remote.execution_id = None;
        remote.remote_connection_id = Some("remote-a".to_owned());
        let remote_bound = service.bind_policy(&remote).unwrap();
        let mut changed_remote = remote.clone();
        changed_remote.remote_connection_id = Some("remote-b".to_owned());
        assert_eq!(
            service.bind_policy(&changed_remote).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        assert_eq!(
            service.validate(&changed_remote).unwrap_err().code,
            BrowserErrorCode::InvalidCallerIdentity
        );

        clock.advance(50);
        let renewed = service.renew(&remote_lease.lease_id).unwrap();
        assert_eq!(
            renewed.task_resource_family_key,
            remote_bound.task_resource_family_key,
            "renewal must retain the sealed family"
        );
        assert!(service.validate(&remote).is_ok());
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
    fn failed_access_to_expired_leases_cannot_consume_sweep_cleanup_authority() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 20);
        let renew_lease = service.issue("user", None, "runtime-renew").unwrap();
        let validate_lease = service.issue("user", None, "runtime-validate").unwrap();
        let bind_lease = service.issue("user", None, "runtime-bind").unwrap();

        clock.set(30);
        assert_eq!(
            service.renew(&renew_lease.lease_id).unwrap_err().code,
            BrowserErrorCode::OwnerLeaseExpired
        );
        assert_eq!(
            service
                .validate(&caller_for(&validate_lease, 100))
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );
        assert_eq!(
            service
                .bind_policy(&caller_for(&bind_lease, 100))
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );

        let mut expired = service.sweep_expired_ids();
        expired.sort();
        let mut expected = vec![
            renew_lease.lease_id,
            validate_lease.lease_id,
            bind_lease.lease_id,
        ];
        expected.sort();
        assert_eq!(expired, expected);
        assert!(lock_unpoisoned(&service.leases).is_empty());
    }

    #[test]
    fn owner_lease_issue_admits_first_32_generations_and_rejects_the_33rd() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            service
                .issue(
                    "user-a",
                    Some("conversation-a".to_owned()),
                    format!("runtime-{index}"),
                )
                .expect("the first 32 task-family generations must be admitted");
        }

        let overflow = service
            .issue(
                "user-a",
                Some("conversation-a".to_owned()),
                "runtime-overflow",
            )
            .expect_err("the 33rd task-family generation must be rejected");
        assert_eq!(overflow.code, BrowserErrorCode::BrowserCapacityQueued);
        assert!(overflow.retryable);
        assert_eq!(
            overflow.metadata,
            json!({
                "reason_code": "browser_owner_lease_task_capacity",
                "capacity_scope": "task",
                "prebind_family_key_kind": "conversation",
                "active_owner_lease_count": 32,
                "max_active_owner_leases": 32,
                "expired_leases_count_toward_limit": true,
                "retry_delay_ms": 250,
            })
        );
        assert_eq!(lock_unpoisoned(&service.leases).len(), 32);
    }

    #[test]
    fn owner_lease_task_capacity_recovers_only_after_revoke_or_authoritative_sweep() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 20);
        let mut leases = Vec::new();
        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            leases.push(
                service
                    .issue(
                        "user",
                        Some("conversation".to_owned()),
                        format!("runtime-{index}"),
                    )
                    .unwrap(),
            );
        }
        assert_eq!(
            service
                .issue(
                    "user",
                    Some("conversation".to_owned()),
                    "runtime-blocked",
                )
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued
        );

        assert!(service.revoke(&leases[0].lease_id));
        service
            .issue(
                "user",
                Some("conversation".to_owned()),
                "runtime-after-revoke",
            )
            .expect("exact revoke must release one task-family slot");

        clock.set(30);
        assert_eq!(
            service
                .issue(
                    "user",
                    Some("conversation".to_owned()),
                    "runtime-expired-but-unswept",
                )
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued,
            "expired generations must retain cleanup authority and capacity until sweep"
        );
        assert_eq!(service.sweep_expired_ids().len(), 32);
        service
            .issue(
                "user",
                Some("conversation".to_owned()),
                "runtime-after-sweep",
            )
            .expect("authoritative sweep must release expired task-family slots");
    }

    #[test]
    fn owner_lease_task_capacity_isolated_by_user_conversation_and_runtime_fallback() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            service
                .issue(
                    "user-a",
                    Some("conversation-a".to_owned()),
                    format!("conversation-runtime-{index}"),
                )
                .unwrap();
        }
        assert_eq!(
            service
                .issue(
                    "user-a",
                    Some("conversation-a".to_owned()),
                    "conversation-overflow",
                )
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued
        );
        service
            .issue(
                "user-b",
                Some("conversation-a".to_owned()),
                "other-user-runtime",
            )
            .expect("the same conversation string under another user is independent");
        service
            .issue(
                "user-a",
                Some("conversation-b".to_owned()),
                "other-conversation-runtime",
            )
            .expect("another conversation under the same user is independent");

        for _ in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            service
                .issue("runtime-user", None, "runtime-a")
                .unwrap();
        }
        let runtime_overflow = service
            .issue("runtime-user", None, "runtime-a")
            .expect_err("conversation-less leases must be capped by exact runtime");
        assert_eq!(
            runtime_overflow.code,
            BrowserErrorCode::BrowserCapacityQueued
        );
        assert_eq!(
            runtime_overflow.metadata["prebind_family_key_kind"],
            "runtime"
        );
        service
            .issue("runtime-user", None, "runtime-b")
            .expect("a different fallback runtime is an independent task family");
    }

    #[test]
    fn concurrent_owner_lease_issue_cannot_overshoot_task_family_limit() {
        const ATTEMPTS: usize = 64;

        let clock = ManualClock::new(10);
        let service = Arc::new(OwnerLeaseService::new(Arc::new(clock), 100));
        let start = Arc::new(Barrier::new(ATTEMPTS + 1));
        let mut workers = Vec::with_capacity(ATTEMPTS);
        for index in 0..ATTEMPTS {
            let service = Arc::clone(&service);
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                start.wait();
                service.issue(
                    "user",
                    Some("conversation".to_owned()),
                    format!("runtime-{index}"),
                )
            }));
        }
        start.wait();

        let mut admitted = 0;
        let mut capacity_rejected = 0;
        for worker in workers {
            match worker.join().expect("lease issuer thread must not panic") {
                Ok(_) => admitted += 1,
                Err(error) => {
                    assert_eq!(error.code, BrowserErrorCode::BrowserCapacityQueued);
                    assert_eq!(error.metadata["active_owner_lease_count"], 32);
                    capacity_rejected += 1;
                }
            }
        }
        assert_eq!(admitted, MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY);
        assert_eq!(capacity_rejected, ATTEMPTS - admitted);
        assert_eq!(
            lock_unpoisoned(&service.leases).len(),
            MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY
        );
    }

    #[test]
    fn bound_execution_family_caps_runtime_rotation_without_mutating_rejected_lease() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let mut bound_callers = Vec::new();

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            let lease = service
                .issue("user", None, format!("execution-runtime-{index}"))
                .unwrap();
            let mut caller = caller_for_family(
                &lease,
                BrowserSurface::Native,
                Some("shared-execution"),
                None,
            );
            if index == 0 {
                caller
                    .allowed_operations
                    .insert(BrowserOperationKind::Observe);
            }
            service.bind_policy(&caller).unwrap();
            bound_callers.push(caller);
        }

        let overflow_lease = service
            .issue("user", None, "execution-runtime-overflow")
            .unwrap();
        let overflow_caller = caller_for_family(
            &overflow_lease,
            BrowserSurface::Native,
            Some("shared-execution"),
            None,
        );
        let overflow = service.bind_policy(&overflow_caller).unwrap_err();
        assert_eq!(overflow.code, BrowserErrorCode::BrowserCapacityQueued);
        assert!(overflow.retryable);
        assert_eq!(
            overflow.metadata["reason_code"],
            "browser_owner_lease_bound_task_capacity"
        );
        let rejected = lock_unpoisoned(&service.leases)
            .get(&overflow_lease.lease_id)
            .cloned()
            .unwrap();
        assert_eq!(rejected.task_resource_family_key, None);
        assert_eq!(rejected.surface, None);
        assert_eq!(rejected.allowed_operations, None);

        let mut narrowed = bound_callers[0].clone();
        narrowed
            .allowed_operations
            .remove(&BrowserOperationKind::Observe);
        service
            .bind_policy(&narrowed)
            .expect("an already-counted lease may narrow at family capacity");
        assert!(service.revoke(&bound_callers[1].owner_lease_id));
        service
            .bind_policy(&overflow_caller)
            .expect("exact revoke releases one sealed-family slot");
    }

    #[test]
    fn bound_remote_family_caps_runtime_rotation_and_isolates_remote_ids() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            let lease = service
                .issue("remote-user", None, format!("remote-runtime-{index}"))
                .unwrap();
            service
                .bind_policy(&caller_for_family(
                    &lease,
                    BrowserSurface::Remote,
                    None,
                    Some("remote-connection-a"),
                ))
                .unwrap();
        }

        let overflow_lease = service
            .issue("remote-user", None, "remote-runtime-overflow")
            .unwrap();
        assert_eq!(
            service
                .bind_policy(&caller_for_family(
                    &overflow_lease,
                    BrowserSurface::Remote,
                    None,
                    Some("remote-connection-a"),
                ))
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued
        );

        let other_remote = service
            .issue("remote-user", None, "remote-runtime-other")
            .unwrap();
        service
            .bind_policy(&caller_for_family(
                &other_remote,
                BrowserSurface::Remote,
                None,
                Some("remote-connection-b"),
            ))
            .expect("a different trusted Remote connection is an independent family");
        let other_user = service
            .issue("other-user", None, "remote-runtime-other-user")
            .unwrap();
        service
            .bind_policy(&caller_for_family(
                &other_user,
                BrowserSurface::Remote,
                None,
                Some("remote-connection-a"),
            ))
            .expect("the same Remote id under another user is an independent family");
    }

    #[test]
    fn bound_runtime_and_conversation_families_keep_their_existing_isolation() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            let lease = service.issue("runtime-user", None, "runtime-a").unwrap();
            service
                .bind_policy(&caller_for_family(
                    &lease,
                    BrowserSurface::System,
                    None,
                    None,
                ))
                .unwrap_or_else(|error| panic!("runtime bind {index} failed: {error}"));
        }
        assert_eq!(
            service
                .issue("runtime-user", None, "runtime-a")
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued
        );
        let other_runtime = service
            .issue("runtime-user", None, "runtime-b")
            .unwrap();
        service
            .bind_policy(&caller_for_family(
                &other_runtime,
                BrowserSurface::System,
                None,
                None,
            ))
            .expect("a different runtime fallback remains an independent family");

        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            let lease = service
                .issue(
                    "conversation-user",
                    Some("conversation-a".to_owned()),
                    format!("conversation-runtime-{index}"),
                )
                .unwrap();
            service
                .bind_policy(&caller_for_family(
                    &lease,
                    BrowserSurface::Native,
                    Some("ignored-by-conversation"),
                    None,
                ))
                .unwrap();
        }
        assert_eq!(
            service
                .issue(
                    "conversation-user",
                    Some("conversation-a".to_owned()),
                    "conversation-overflow",
                )
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued
        );
        let other_conversation = service
            .issue(
                "conversation-user",
                Some("conversation-b".to_owned()),
                "conversation-other",
            )
            .unwrap();
        service
            .bind_policy(&caller_for_family(
                &other_conversation,
                BrowserSurface::Native,
                Some("ignored-by-conversation"),
                None,
            ))
            .expect("another conversation remains an independent family");
    }

    #[test]
    fn expired_unswept_bound_family_leases_still_hold_capacity() {
        let clock = ManualClock::new(10);
        let service = OwnerLeaseService::new(Arc::new(clock.clone()), 20);
        for index in 0..MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY {
            let lease = service
                .issue("user", None, format!("expired-runtime-{index}"))
                .unwrap();
            service
                .bind_policy(&caller_for_family(
                    &lease,
                    BrowserSurface::Native,
                    Some("expiring-execution"),
                    None,
                ))
                .unwrap();
        }

        clock.set(25);
        let replacement = service
            .issue("user", None, "replacement-runtime")
            .unwrap();
        let replacement_caller = caller_for_family(
            &replacement,
            BrowserSurface::Native,
            Some("expiring-execution"),
            None,
        );
        clock.set(30);
        assert_eq!(
            service
                .bind_policy(&replacement_caller)
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserCapacityQueued,
            "expired generations retain exact cleanup authority until sweep"
        );
        assert_eq!(service.sweep_expired_ids().len(), 32);
        service
            .bind_policy(&replacement_caller)
            .expect("authoritative sweep releases sealed-family capacity");
    }

    #[test]
    fn concurrent_bound_family_sealing_cannot_overshoot_limit() {
        const ATTEMPTS: usize = 64;

        let clock = ManualClock::new(10);
        let service = Arc::new(OwnerLeaseService::new(Arc::new(clock), 100));
        let callers = (0..ATTEMPTS)
            .map(|index| {
                let lease = service
                    .issue("user", None, format!("concurrent-runtime-{index}"))
                    .unwrap();
                caller_for_family(
                    &lease,
                    BrowserSurface::Native,
                    Some("concurrent-execution"),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let start = Arc::new(Barrier::new(ATTEMPTS + 1));
        let mut workers = Vec::with_capacity(ATTEMPTS);
        for caller in callers {
            let service = Arc::clone(&service);
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                start.wait();
                service.bind_policy(&caller)
            }));
        }
        start.wait();

        let mut admitted = 0;
        let mut rejected = 0;
        for worker in workers {
            match worker.join().expect("lease binder thread must not panic") {
                Ok(_) => admitted += 1,
                Err(error) => {
                    assert_eq!(error.code, BrowserErrorCode::BrowserCapacityQueued);
                    rejected += 1;
                }
            }
        }
        assert_eq!(admitted, MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY);
        assert_eq!(rejected, ATTEMPTS - admitted);
        let leases = lock_unpoisoned(&service.leases);
        assert_eq!(
            leases
                .values()
                .filter(|lease| lease.task_resource_family_key.is_some())
                .count(),
            MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY
        );
        assert!(leases.values().filter(|lease| lease.surface.is_none()).all(
            |lease| lease.allowed_operations.is_none()
                && lease.task_resource_family_key.is_none()
        ));
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

    #[test]
    fn terminal_lease_authority_revokes_existing_and_rejects_issue_and_renew() {
        let clock = ManualClock::new(100);
        let service = OwnerLeaseService::new(Arc::new(clock), 100);
        let first = service.issue("user", None, "runtime-1").unwrap();
        let second = service.issue("user", None, "runtime-2").unwrap();

        assert_eq!(service.stop_accepting_and_clear(), 2);
        assert_eq!(service.stop_accepting_and_clear(), 0);
        assert_eq!(
            service.renew(&first.lease_id).unwrap_err().code,
            BrowserErrorCode::BrowserShuttingDown
        );
        assert_eq!(
            service.issue("user", None, "runtime-3").unwrap_err().code,
            BrowserErrorCode::BrowserShuttingDown
        );
        assert_eq!(
            service
                .validate(&caller_for(&second, 1_000))
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );
    }

}
