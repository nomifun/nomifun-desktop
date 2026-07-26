use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BrowserErrorCode, BrowserLaneId, BrowserOperationKind, BrowserPlatformError,
    BrowserSurface, CallerIdentity, Clock, OwnerLeaseId,
};

const MINIMUM_TTL_MS: u64 = 1;
const DEFAULT_TOKEN_TOMBSTONE_TTL_MS: u64 = 60_000;

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
        validate_policy_binding(&lease, caller)?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlLease {
    pub lease_id: String,
    pub lane_id: BrowserLaneId,
    pub user_id: String,
    pub issued_at_ms: u64,
    pub renewed_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone)]
pub struct ControlLeaseService {
    clock: Arc<dyn Clock>,
    ttl_ms: u64,
    leases: Arc<Mutex<HashMap<BrowserLaneId, ControlLease>>>,
}

impl ControlLeaseService {
    pub fn new(clock: Arc<dyn Clock>, ttl_ms: u64) -> Self {
        Self {
            clock,
            ttl_ms: ttl_ms.max(MINIMUM_TTL_MS),
            leases: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Acquires the sole user-control lease for a lane. Re-acquiring as the
    /// current user is idempotent and renews the existing lease.
    pub fn acquire(
        &self,
        lane_id: BrowserLaneId,
        user_id: impl Into<String>,
    ) -> Result<ControlLease, BrowserPlatformError> {
        let user_id = user_id.into();
        validate_user_and_lane(&user_id, &lane_id)?;

        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        if let Some(current) = leases.get_mut(&lane_id) {
            if current.expires_at_ms <= now_ms {
                leases.remove(&lane_id);
            } else if current.user_id == user_id {
                current.renewed_at_ms = now_ms;
                current.expires_at_ms = now_ms.saturating_add(self.ttl_ms);
                return Ok(current.clone());
            } else {
                return Err(controlled_by_another_user(&lane_id));
            }
        }

        let lease = ControlLease {
            lease_id: uuid::Uuid::now_v7().to_string(),
            lane_id: lane_id.clone(),
            user_id,
            issued_at_ms: now_ms,
            renewed_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(self.ttl_ms),
        };
        leases.insert(lane_id, lease.clone());
        Ok(lease)
    }

    pub fn renew(
        &self,
        lane_id: &BrowserLaneId,
        user_id: &str,
        lease_id: &str,
    ) -> Result<ControlLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        let Some(current) = leases.get_mut(lane_id) else {
            return Err(control_lease_invalid(lane_id));
        };
        if current.expires_at_ms <= now_ms {
            leases.remove(lane_id);
            return Err(control_lease_invalid(lane_id));
        }
        if current.user_id != user_id || current.lease_id != lease_id {
            return Err(controlled_by_another_user(lane_id));
        }

        current.renewed_at_ms = now_ms;
        current.expires_at_ms = now_ms.saturating_add(self.ttl_ms);
        Ok(current.clone())
    }

    /// Releases a lease only when all binding facts match. A missing or stale
    /// lease is treated as an idempotent no-op.
    pub fn release(&self, lane_id: &BrowserLaneId, user_id: &str, lease_id: &str) -> bool {
        let mut leases = lock_unpoisoned(&self.leases);
        let should_remove = leases
            .get(lane_id)
            .is_some_and(|lease| lease.user_id == user_id && lease.lease_id == lease_id);
        if should_remove {
            leases.remove(lane_id);
        }
        should_remove
    }

    pub fn validate(
        &self,
        lane_id: &BrowserLaneId,
        user_id: &str,
        lease_id: &str,
    ) -> Result<ControlLease, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        let Some(current) = leases.get(lane_id) else {
            return Err(control_lease_invalid(lane_id));
        };
        if current.expires_at_ms <= now_ms {
            leases.remove(lane_id);
            return Err(control_lease_invalid(lane_id));
        }
        if current.user_id != user_id || current.lease_id != lease_id {
            return Err(controlled_by_another_user(lane_id));
        }
        Ok(current.clone())
    }

    /// Returns the current non-expired lease, if any.
    ///
    /// Reading this authority is security-sensitive: an unavailable authority
    /// must remain distinct from an authoritative `None`, otherwise an Agent
    /// could resume while user control is still in force.
    pub fn current(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<Option<ControlLease>, BrowserPlatformError> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_authority(&self.leases)?;
        Ok(match leases.get(lane_id) {
            Some(lease) if lease.expires_at_ms > now_ms => Some(lease.clone()),
            Some(_) => {
                leases.remove(lane_id);
                None
            }
            None => None,
        })
    }

    pub fn has_active(&self, lane_id: &BrowserLaneId) -> Result<bool, BrowserPlatformError> {
        Ok(self.current(lane_id)?.is_some())
    }

    /// Revokes user control as part of authoritative lane cleanup.
    pub fn revoke_lane(&self, lane_id: &BrowserLaneId) -> bool {
        lock_unpoisoned(&self.leases).remove(lane_id).is_some()
    }

    /// Removes expired leases and returns the affected lane IDs so callers can
    /// publish authoritative control-state changes.
    pub fn sweep_expired(&self) -> Vec<BrowserLaneId> {
        let now_ms = self.clock.now_ms();
        let mut leases = lock_unpoisoned(&self.leases);
        let mut expired: Vec<_> = leases
            .iter()
            .filter(|(_, lease)| lease.expires_at_ms <= now_ms)
            .map(|(lane_id, _)| lane_id.clone())
            .collect();
        expired.sort();
        for lane_id in &expired {
            leases.remove(lane_id);
        }
        expired
    }

    /// Removes expired leases and returns the number removed.
    pub fn sweep(&self) -> usize {
        self.sweep_expired().len()
    }

    #[cfg(test)]
    pub(crate) fn poison_authority_for_test(&self) {
        let leases = Arc::clone(&self.leases);
        assert!(
            std::thread::spawn(move || {
                let _guard = leases.lock().expect("control lease authority lock");
                panic!("intentional control lease authority poison");
            })
            .join()
            .is_err()
        );
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerGrant {
    /// Single-use bearer token. Services retain only its SHA-256 digest.
    pub token: String,
    pub user_id: String,
    pub lane_id: BrowserLaneId,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

impl fmt::Debug for ViewerGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ViewerGrant")
            .field("token", &"<redacted>")
            .field("user_id", &self.user_id)
            .field("lane_id", &self.lane_id)
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewerTokenState {
    Active,
    Consumed { at_ms: u64 },
    Expired { at_ms: u64 },
}

#[derive(Clone, Debug)]
struct ViewerTokenRecord {
    user_id: String,
    lane_id: BrowserLaneId,
    issued_at_ms: u64,
    expires_at_ms: u64,
    state: ViewerTokenState,
}

#[derive(Clone)]
pub struct ViewerTokenService {
    clock: Arc<dyn Clock>,
    ttl_ms: u64,
    tombstone_ttl_ms: u64,
    records: Arc<Mutex<HashMap<[u8; 32], ViewerTokenRecord>>>,
}

impl ViewerTokenService {
    pub fn new(clock: Arc<dyn Clock>, ttl_ms: u64) -> Self {
        Self::with_tombstone_ttl(clock, ttl_ms, DEFAULT_TOKEN_TOMBSTONE_TTL_MS)
    }

    pub fn with_tombstone_ttl(
        clock: Arc<dyn Clock>,
        ttl_ms: u64,
        tombstone_ttl_ms: u64,
    ) -> Self {
        Self {
            clock,
            ttl_ms: ttl_ms.max(MINIMUM_TTL_MS),
            tombstone_ttl_ms: tombstone_ttl_ms.max(MINIMUM_TTL_MS),
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn issue(
        &self,
        user_id: impl Into<String>,
        lane_id: BrowserLaneId,
    ) -> Result<ViewerGrant, BrowserPlatformError> {
        let user_id = user_id.into();
        validate_user_and_lane(&user_id, &lane_id)?;

        let mut random_bytes = [0_u8; 32];
        getrandom::getrandom(&mut random_bytes).map_err(|_| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "A secure viewer token could not be created.",
                true,
                "Retry opening the browser viewer.",
            )
        })?;
        let token = hex::encode(random_bytes);
        let digest = token_digest(&token);
        let now_ms = self.clock.now_ms();
        let record = ViewerTokenRecord {
            user_id: user_id.clone(),
            lane_id: lane_id.clone(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(self.ttl_ms),
            state: ViewerTokenState::Active,
        };

        // A collision is cryptographically negligible. Refusing it is safer
        // than overwriting a still-valid grant.
        match lock_authority(&self.records)?.entry(digest) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(record.clone());
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "A unique viewer token could not be created.",
                    true,
                    "Retry opening the browser viewer.",
                ));
            }
        }

        Ok(ViewerGrant {
            token,
            user_id,
            lane_id,
            issued_at_ms: record.issued_at_ms,
            expires_at_ms: record.expires_at_ms,
        })
    }

    /// Validates and atomically consumes a viewer token. The supplied bearer
    /// token is hashed before lookup and is never retained.
    pub fn consume(
        &self,
        token: &str,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<ViewerGrant, BrowserPlatformError> {
        let digest = token_digest(token);
        let now_ms = self.clock.now_ms();
        let mut records = lock_authority(&self.records)?;
        let Some(record) = records.get_mut(&digest) else {
            return Err(viewer_token_error(BrowserErrorCode::ViewerTokenInvalid));
        };

        if record.user_id != user_id || &record.lane_id != lane_id {
            return Err(viewer_token_error(BrowserErrorCode::ViewerTokenInvalid));
        }

        match record.state {
            ViewerTokenState::Consumed { .. } => {
                return Err(viewer_token_error(BrowserErrorCode::ViewerTokenConsumed));
            }
            ViewerTokenState::Expired { .. } => {
                return Err(viewer_token_error(BrowserErrorCode::ViewerTokenExpired));
            }
            ViewerTokenState::Active if record.expires_at_ms <= now_ms => {
                record.state = ViewerTokenState::Expired { at_ms: now_ms };
                return Err(viewer_token_error(BrowserErrorCode::ViewerTokenExpired));
            }
            ViewerTokenState::Active => {
                record.state = ViewerTokenState::Consumed { at_ms: now_ms };
            }
        }

        Ok(ViewerGrant {
            token: token.to_owned(),
            user_id: record.user_id.clone(),
            lane_id: record.lane_id.clone(),
            issued_at_ms: record.issued_at_ms,
            expires_at_ms: record.expires_at_ms,
        })
    }

    /// Revokes every viewer grant and tombstone for a closed lane.
    pub fn revoke_lane(&self, lane_id: &BrowserLaneId) -> usize {
        let mut records = lock_unpoisoned(&self.records);
        let before = records.len();
        records.retain(|_, record| &record.lane_id != lane_id);
        before - records.len()
    }

    /// Marks newly expired grants and eventually removes terminal tombstones.
    /// Tombstones preserve precise `expired` and `consumed` errors for a bounded
    /// period without retaining the raw bearer token.
    pub fn sweep(&self) -> usize {
        let now_ms = self.clock.now_ms();
        let mut records = lock_unpoisoned(&self.records);
        let mut changed = 0;

        for record in records.values_mut() {
            if record.state == ViewerTokenState::Active && record.expires_at_ms <= now_ms {
                record.state = ViewerTokenState::Expired { at_ms: now_ms };
                changed += 1;
            }
        }

        let before = records.len();
        records.retain(|_, record| match record.state {
            ViewerTokenState::Active => true,
            ViewerTokenState::Consumed { at_ms } | ViewerTokenState::Expired { at_ms } => {
                now_ms < at_ms.saturating_add(self.tombstone_ttl_ms)
            }
        });
        changed + (before - records.len())
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

fn validate_user_and_lane(
    user_id: &str,
    lane_id: &BrowserLaneId,
) -> Result<(), BrowserPlatformError> {
    if user_id.trim().is_empty() || lane_id.as_str().trim().is_empty() {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "A browser lease requires a user and lane.",
            false,
            "Refresh the browser inventory and try again.",
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

fn controlled_by_another_user(lane_id: &BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::LaneControlledByUser,
        "The browser lane is currently controlled by another user session.",
        true,
        "Wait for control to be returned or request it from the current user.",
    )
    .for_lane(lane_id.clone())
}

fn control_lease_invalid(lane_id: &BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "The browser control lease is missing, expired, or no longer current.",
        false,
        "Acquire control of the browser lane again.",
    )
    .for_lane(lane_id.clone())
}

fn viewer_token_error(code: BrowserErrorCode) -> BrowserPlatformError {
    let (message, retryable, next_action) = match code {
        BrowserErrorCode::ViewerTokenExpired => (
            "The browser viewer token has expired.",
            true,
            "Request a new viewer token.",
        ),
        BrowserErrorCode::ViewerTokenConsumed => (
            "The browser viewer token has already been used.",
            true,
            "Request a new viewer token.",
        ),
        _ => (
            "The browser viewer token is invalid.",
            false,
            "Request a new viewer token from the application.",
        ),
    };
    BrowserPlatformError::new(code, message, retryable, next_action)
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

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
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

    #[test]
    fn control_lease_is_exclusive_and_same_user_acquire_is_idempotent() {
        let clock = ManualClock::new(100);
        let service = ControlLeaseService::new(Arc::new(clock.clone()), 30);
        let lane = BrowserLaneId::new();
        let first = service.acquire(lane.clone(), "user-1").unwrap();

        clock.advance(10);
        let reacquired = service.acquire(lane.clone(), "user-1").unwrap();
        assert_eq!(reacquired.lease_id, first.lease_id);
        assert_eq!(reacquired.expires_at_ms, 140);
        assert_eq!(
            service
                .acquire(lane.clone(), "user-2")
                .unwrap_err()
                .code,
            BrowserErrorCode::LaneControlledByUser
        );

        assert!(service
            .validate(&lane, "user-1", &first.lease_id)
            .is_ok());
        assert!(!service.release(&lane, "user-2", &first.lease_id));
        assert!(service.release(&lane, "user-1", &first.lease_id));
        assert!(service.current(&lane).unwrap().is_none());
    }

    #[test]
    fn expired_control_lease_can_be_acquired_by_another_user() {
        let clock = ManualClock::new(500);
        let service = ControlLeaseService::new(Arc::new(clock.clone()), 10);
        let lane = BrowserLaneId::new();
        let other_lane = BrowserLaneId::new();
        let old = service.acquire(lane.clone(), "user-1").unwrap();
        service
            .acquire(other_lane.clone(), "user-1")
            .unwrap();

        clock.advance(10);
        let mut expected_expired = vec![lane.clone(), other_lane.clone()];
        expected_expired.sort();
        assert_eq!(service.sweep_expired(), expected_expired);
        assert_eq!(
            service
                .validate(&lane, "user-1", &old.lease_id)
                .unwrap_err()
                .code,
            BrowserErrorCode::OperationNotAllowed
        );
        let next = service.acquire(lane.clone(), "user-2").unwrap();
        assert_eq!(next.user_id, "user-2");
        assert!(service.has_active(&lane).unwrap());
        assert!(service.revoke_lane(&lane));
        assert!(!service.has_active(&lane).unwrap());
    }

    #[test]
    fn poisoned_control_lease_authority_fails_closed() {
        let service = ControlLeaseService::new(Arc::new(ManualClock::new(100)), 100);
        let lane = BrowserLaneId::new();
        let lease = service.acquire(lane.clone(), "user").unwrap();
        service.poison_authority_for_test();

        assert_eq!(
            service
                .validate(&lane, "user", &lease.lease_id)
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.acquire(lane.clone(), "user").unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.current(&lane).unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.has_active(&lane).unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable,
            "an unreadable control-lease authority must not be treated as no active lease"
        );
    }

    #[test]
    fn viewer_token_is_bound_and_single_use() {
        let clock = ManualClock::new(1_000);
        let service = ViewerTokenService::new(Arc::new(clock), 100);
        let lane = BrowserLaneId::new();
        let grant = service.issue("user-1", lane.clone()).unwrap();

        assert_eq!(
            service
                .consume(&grant.token, "user-2", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenInvalid
        );
        let consumed = service
            .consume(&grant.token, "user-1", &lane)
            .unwrap();
        assert_eq!(consumed.expires_at_ms, grant.expires_at_ms);
        assert_eq!(
            service
                .consume(&grant.token, "user-1", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenConsumed
        );

        let digest = token_digest(&grant.token);
        assert!(lock_unpoisoned(&service.records).contains_key(&digest));
    }

    #[test]
    fn viewer_token_expiry_is_distinct_without_sleeping() {
        let clock = ManualClock::new(20);
        let service = ViewerTokenService::new(Arc::new(clock.clone()), 10);
        let lane = BrowserLaneId::new();
        let grant = service.issue("user", lane.clone()).unwrap();

        clock.set(30);
        assert_eq!(
            service
                .consume(&grant.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenExpired
        );
        assert_eq!(
            service
                .consume(&grant.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenExpired
        );
    }

    #[test]
    fn poisoned_viewer_token_authority_fails_closed() {
        let service = ViewerTokenService::new(Arc::new(ManualClock::new(100)), 100);
        let lane = BrowserLaneId::new();
        let grant = service.issue("user", lane.clone()).unwrap();
        poison(&service.records);

        assert_eq!(
            service
                .consume(&grant.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserUnavailable
        );
        assert_eq!(
            service.issue("user", lane).unwrap_err().code,
            BrowserErrorCode::BrowserUnavailable
        );
    }

    #[test]
    fn viewer_tombstones_are_bounded() {
        let clock = ManualClock::new(100);
        let service =
            ViewerTokenService::with_tombstone_ttl(Arc::new(clock.clone()), 10, 20);
        let lane = BrowserLaneId::new();
        let consumed = service.issue("user", lane.clone()).unwrap();
        service
            .consume(&consumed.token, "user", &lane)
            .unwrap();
        let expired = service.issue("user", lane.clone()).unwrap();

        clock.advance(10);
        assert_eq!(service.sweep(), 1);
        assert_eq!(
            service
                .consume(&expired.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenExpired
        );

        clock.advance(20);
        assert_eq!(service.sweep(), 2);
        assert_eq!(
            service
                .consume(&consumed.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenInvalid
        );
    }

    #[test]
    fn closing_a_lane_revokes_all_viewer_tokens() {
        let clock = ManualClock::new(100);
        let service = ViewerTokenService::new(Arc::new(clock), 100);
        let lane = BrowserLaneId::new();
        let other_lane = BrowserLaneId::new();
        let first = service.issue("user", lane.clone()).unwrap();
        service.issue("user", lane.clone()).unwrap();
        let other = service.issue("user", other_lane.clone()).unwrap();

        assert_eq!(service.revoke_lane(&lane), 2);
        assert_eq!(
            service
                .consume(&first.token, "user", &lane)
                .unwrap_err()
                .code,
            BrowserErrorCode::ViewerTokenInvalid
        );
        assert!(service
            .consume(&other.token, "user", &other_lane)
            .is_ok());
    }
}
