//! Domain-separated loopback capabilities.
//!
//! A long-lived root secret belongs to the backend process. Child processes
//! receive only a short-lived HMAC over immutable, versioned claims. The
//! domain is part of the MAC input (not caller-controlled JSON), so a token
//! issued for one loopback service can never authorize another.

use std::collections::HashMap;
use std::sync::{
    Arc, RwLock, Weak,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::{ConversationId, TerminalId, UserId};

type HmacSha256 = Hmac<Sha256>;

pub const LOOPBACK_CAPABILITY_VERSION: u16 = 2;
pub const LOOPBACK_CAPABILITY_TTL_SECS: u64 = 12 * 60 * 60;
pub const LOOPBACK_CAPABILITY_RENEWAL_MARGIN_SECS: u64 = 5 * 60;
pub const LOOPBACK_CAPABILITY_RENEW_PATH: &str = "/capability/renew";
pub const LOOPBACK_CAPABILITY_REVOKE_PATH: &str = "/capability/revoke";
const CLOCK_SKEW_SECS: u64 = 30;
const ISSUER_SECRET_BYTES: usize = 32;

/// Mint a process-local HMAC issuer secret from 256 bits of OS randomness.
/// The returned text is safe to keep in memory/config objects; it must never be
/// serialized or passed to a child process.
fn generate_loopback_issuer_secret() -> Result<String, String> {
    let mut bytes = [0_u8; ISSUER_SECRET_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("failed to generate loopback issuer secret: {error}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn generate_loopback_lease_id() -> Result<String, LoopbackCapabilityError> {
    generate_loopback_issuer_secret().map_err(|_| LoopbackCapabilityError::Malformed)
}

fn mac_for(secret: &[u8], domain: &str, payload: &[u8]) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&(domain.len() as u64).to_be_bytes());
    mac.update(domain.as_bytes());
    mac.update(&(payload.len() as u64).to_be_bytes());
    mac.update(payload);
    mac
}

/// Derive a URL-safe bearer token for an immutable, domain-separated payload.
fn derive_scoped_auth_token(secret: &[u8], domain: &str, payload: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(mac_for(secret, domain, payload).finalize().into_bytes())
}

/// Verify a scoped bearer token in constant time.
fn verify_scoped_auth_token(
    secret: &[u8],
    domain: &str,
    payload: &[u8],
    presented: &str,
) -> bool {
    let Ok(tag) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(presented) else {
        return false;
    };
    mac_for(secret, domain, payload).verify_slice(&tag).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopbackSessionKind {
    Conversation,
    Terminal,
    ExternalProcess,
}

impl LoopbackSessionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Terminal => "terminal",
            Self::ExternalProcess => "external_process",
        }
    }
}

/// Identity that a backend issuer resolved before spawning a child process.
/// `conversation_id` is explicit so downstream services never infer a
/// conversation from an unrelated session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackSessionBinding {
    pub kind: LoopbackSessionKind,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

impl<'de> Deserialize<'de> for LoopbackSessionBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireBinding {
            kind: LoopbackSessionKind,
            session_id: String,
            #[serde(default)]
            conversation_id: Option<String>,
        }

        let wire = WireBinding::deserialize(deserializer)?;
        let binding = Self {
            kind: wire.kind,
            session_id: wire.session_id,
            conversation_id: wire.conversation_id,
        };
        binding
            .validate_identity()
            .map_err(serde::de::Error::custom)?;
        Ok(binding)
    }
}

impl LoopbackSessionBinding {
    pub fn conversation(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            kind: LoopbackSessionKind::Conversation,
            session_id: id.clone(),
            conversation_id: Some(id),
        }
    }

    pub fn terminal(id: impl Into<String>) -> Self {
        Self {
            kind: LoopbackSessionKind::Terminal,
            session_id: id.into(),
            conversation_id: None,
        }
    }

    /// A locally authenticated third-party process connected through the
    /// owner-only knowledge broker. The process id is generated by the broker;
    /// it is never accepted from the external client.
    pub fn external_process(id: impl Into<String>) -> Self {
        Self {
            kind: LoopbackSessionKind::ExternalProcess,
            session_id: id.into(),
            conversation_id: None,
        }
    }

    fn validate_identity(&self) -> Result<(), LoopbackCapabilityError> {
        match self.kind {
            LoopbackSessionKind::Conversation
                if self.conversation_id.as_deref() == Some(self.session_id.as_str())
                    && ConversationId::parse(&self.session_id).is_ok() =>
            {
                Ok(())
            }
            LoopbackSessionKind::Terminal
                if self.conversation_id.is_none()
                    && TerminalId::parse(&self.session_id).is_ok() =>
            {
                Ok(())
            }
            LoopbackSessionKind::ExternalProcess
                if self.conversation_id.is_none()
                    && canonical_required(&self.session_id) =>
            {
                Ok(())
            }
            _ => Err(LoopbackCapabilityError::InvalidIdentity),
        }
    }
}

/// Common signed envelope. `S` is the domain-specific, server-authoritative
/// scope (for example requirement ownership or mounted knowledge-base ids).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackCapabilityClaims<S> {
    pub version: u16,
    pub issued_at_unix_secs: u64,
    pub expires_at_unix_secs: u64,
    pub lease_id: String,
    pub nonce: String,
    pub user_id: UserId,
    pub session: LoopbackSessionBinding,
    pub allowed_tools: Vec<String>,
    pub scope: S,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopbackCapabilityError {
    Malformed,
    UnsupportedVersion,
    InvalidIdentity,
    InvalidLifetime,
    Expired,
    InvalidToolScope,
    InvalidToken,
    CapacityExceeded,
}

/// Child-to-parent request for a fresh short-lived access credential. The
/// child submits no identity, tool, or domain scope: the issuer restores the
/// complete immutable authorization from its process-local lease registry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackCapabilityRenewalRequest {
    pub lease_id: String,
    pub renewal_proof: String,
}

impl std::fmt::Debug for LoopbackCapabilityRenewalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackCapabilityRenewalRequest")
            .field("lease_id", &self.lease_id)
            .field("renewal_proof", &"[REDACTED]")
            .finish()
    }
}

/// Fresh access returned by the process-local issuer after renewal. This is
/// safe to serialize to one bridge child; it never contains the root secret or
/// the renewal proof.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopbackCapabilityAccess<C> {
    pub token: String,
    pub claims: C,
}


impl<C: std::fmt::Debug> std::fmt::Debug for LoopbackCapabilityAccess<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackCapabilityAccess")
            .field("token", &"[REDACTED]")
            .field("claims", &self.claims)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoopbackCapabilityAuthorization<S> {
    version: u16,
    lease_id: String,
    user_id: UserId,
    session: LoopbackSessionBinding,
    allowed_tools: Vec<String>,
    scope: S,
}

impl<S: Clone> From<&LoopbackCapabilityClaims<S>> for LoopbackCapabilityAuthorization<S> {
    fn from(claims: &LoopbackCapabilityClaims<S>) -> Self {
        Self {
            version: claims.version,
            lease_id: claims.lease_id.clone(),
            user_id: claims.user_id.clone(),
            session: claims.session.clone(),
            allowed_tools: claims.allowed_tools.clone(),
            scope: claims.scope.clone(),
        }
    }
}

impl<S> LoopbackCapabilityAuthorization<S>
where
    S: Serialize + for<'de> Deserialize<'de>,
{
    fn into_claims_at(self, now: u64) -> Result<LoopbackCapabilityClaims<S>, LoopbackCapabilityError> {
        let claims = LoopbackCapabilityClaims {
            version: self.version,
            issued_at_unix_secs: now,
            expires_at_unix_secs: now.saturating_add(LOOPBACK_CAPABILITY_TTL_SECS),
            lease_id: self.lease_id,
            nonce: crate::generate_id(),
            user_id: self.user_id,
            session: self.session,
            allowed_tools: self.allowed_tools,
            scope: self.scope,
        };
        claims.validate_at(now)?;
        Ok(claims)
    }
}

#[derive(Clone)]
struct ActiveLoopbackLease {
    domain: String,
    authorization_json: String,
    user_id: UserId,
    session: LoopbackSessionBinding,
    request_budget: Arc<LoopbackRequestBudget>,
}

#[derive(Debug, Default)]
struct LoopbackRequestBudget {
    active: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LoopbackRequestBudgetFamilyKey([u8; 32]);

impl LoopbackRequestBudgetFamilyKey {
    fn new(
        domain: &str,
        user_id: &UserId,
        session: &LoopbackSessionBinding,
    ) -> Self {
        fn update_part(hash: &mut Sha256, value: &str) {
            sha2::Digest::update(hash, (value.len() as u64).to_be_bytes());
            sha2::Digest::update(hash, value.as_bytes());
        }

        let mut hash = <Sha256 as sha2::Digest>::new();
        update_part(&mut hash, domain);
        update_part(&mut hash, user_id.as_str());
        update_part(&mut hash, session.kind.as_str());
        update_part(&mut hash, &session.session_id);
        match session.conversation_id.as_deref() {
            Some(conversation_id) => {
                sha2::Digest::update(&mut hash, [1]);
                update_part(&mut hash, conversation_id);
            }
            None => sha2::Digest::update(&mut hash, [0]),
        }
        Self(sha2::Digest::finalize(hash).into())
    }
}

impl LoopbackRequestBudget {
    fn try_acquire(
        self: &Arc<Self>,
        lease_id: &str,
        max_in_flight: usize,
    ) -> Result<LoopbackRequestPermit, LoopbackCapabilityError> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active >= max_in_flight {
                return Err(LoopbackCapabilityError::CapacityExceeded);
            }
            match self.active.compare_exchange_weak(
                active,
                active.saturating_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(LoopbackRequestPermit {
                        budget: Arc::clone(self),
                        lease_id: Arc::from(lease_id),
                    });
                }
                Err(observed) => active = observed,
            }
        }
    }
}

/// Non-serializable authority for one verified in-flight request.
///
/// The permit is tied to the active lease object kept by
/// [`LoopbackCapabilityIssuer`]. It is intentionally non-cloneable and refunds
/// its slot on every completion path, including future cancellation and panic
/// unwinding.
pub struct LoopbackRequestPermit {
    budget: Arc<LoopbackRequestBudget>,
    lease_id: Arc<str>,
}

impl LoopbackRequestPermit {
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }
}

impl std::fmt::Debug for LoopbackRequestPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackRequestPermit")
            .field("lease_id", &self.lease_id)
            .finish_non_exhaustive()
    }
}

impl Drop for LoopbackRequestPermit {
    fn drop(&mut self) {
        let previous = self.budget.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "loopback request budget underflow");
    }
}

/// Non-serializable process authority for all access/renew/revoke operations
/// of one loopback service. Its registry is the source of truth for immutable
/// authorization; a child can never submit replacement scope during renewal.
pub struct LoopbackCapabilityIssuer {
    root_secret: Arc<str>,
    active_leases: RwLock<HashMap<String, ActiveLoopbackLease>>,
    /// Weak shells keep a family discoverable across complete lease
    /// revocation while an already-admitted request still owns the strong
    /// authority. Dead shells are pruned on every lease mutation, so rotating
    /// arbitrary family keys cannot grow this map permanently.
    request_budgets:
        RwLock<HashMap<LoopbackRequestBudgetFamilyKey, Weak<LoopbackRequestBudget>>>,
}

impl std::fmt::Debug for LoopbackCapabilityIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackCapabilityIssuer")
            .field("root_secret", &"[REDACTED]")
            .field(
                "active_leases",
                &self
                    .active_leases
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .len(),
            )
            .finish()
    }
}

impl LoopbackCapabilityIssuer {
    fn new(root_secret: impl Into<Arc<str>>) -> Self {
        Self {
            root_secret: root_secret.into(),
            active_leases: RwLock::new(HashMap::new()),
            request_budgets: RwLock::new(HashMap::new()),
        }
    }

    pub fn random() -> Result<Self, String> {
        Ok(Self::new(generate_loopback_issuer_secret()?))
    }

    fn renewal_domain(domain: &str) -> String {
        format!("{domain}:renew-v1")
    }

    fn authorization_json<S>(
        claims: &LoopbackCapabilityClaims<S>,
    ) -> Result<String, LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::to_string(&LoopbackCapabilityAuthorization::from(claims))
            .map_err(|_| LoopbackCapabilityError::Malformed)
    }

    /// Call only while holding `active_leases` for write. This establishes the
    /// sole lock order (`active_leases` then `request_budgets`) used by lease
    /// mutations and makes lookup-or-insert atomic with lease publication.
    fn request_budget_for_family_locked(
        &self,
        domain: &str,
        user_id: &UserId,
        session: &LoopbackSessionBinding,
    ) -> Arc<LoopbackRequestBudget> {
        let key = LoopbackRequestBudgetFamilyKey::new(domain, user_id, session);
        let mut budgets = self
            .request_budgets
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        budgets.retain(|_, budget| budget.upgrade().is_some());
        if let Some(existing) = budgets.get(&key).and_then(Weak::upgrade) {
            return existing;
        }
        let budget = Arc::new(LoopbackRequestBudget::default());
        budgets.insert(key, Arc::downgrade(&budget));
        budget
    }

    /// Call only while holding `active_leases` for write; see the lock-order
    /// contract on [`Self::request_budget_for_family_locked`].
    fn prune_dead_request_budgets_locked(&self) {
        self.request_budgets
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, budget| budget.upgrade().is_some());
    }

    #[cfg(test)]
    fn request_budget_family_count(&self) -> usize {
        self.request_budgets
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// Activate one immutable authorization and return its short-lived access
    /// token plus a process-scoped renewal proof. A replacement issuance for
    /// the same domain/user/session revokes the previous lease deterministically.
    pub fn activate<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
    ) -> Result<(String, String), LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        self.activate_inner(domain, claims, true, None)
    }

    /// Activate one immutable authorization without replacing sibling leases
    /// for the same user/session.
    ///
    /// This is reserved for domains whose trusted scope contains an additional
    /// runtime/attempt identity and which explicitly support multiple concurrent
    /// owners inside one conversation. Callers that require the traditional
    /// single-child-per-session behavior should use [`Self::activate`].
    pub fn activate_concurrent<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
    ) -> Result<(String, String), LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        self.activate_inner(domain, claims, false, None)
    }

    /// Activate one immutable authorization without replacing sibling leases,
    /// while enforcing an exact domain/user/session concurrency ceiling.
    ///
    /// Counting and insertion happen under the same registry write lock, so
    /// racing issuers cannot exceed `max_active`. The ceiling is deliberately
    /// scoped to one user-visible session rather than the process: unrelated
    /// users and conversations retain independent capacity.
    pub fn activate_concurrent_bounded<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
        max_active: usize,
    ) -> Result<(String, String), LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        self.activate_inner(domain, claims, false, Some(max_active))
    }

    fn activate_inner<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
        replace_same_session: bool,
        max_concurrent: Option<usize>,
    ) -> Result<(String, String), LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        claims.validate_at(unix_time_secs())?;
        let authorization_json = Self::authorization_json(claims)?;
        let renewal_proof = derive_scoped_auth_token(
            self.root_secret.as_bytes(),
            &Self::renewal_domain(domain),
            authorization_json.as_bytes(),
        );
        let token = claims.derive_token(self.root_secret.as_bytes(), domain)?;

        let mut leases = self
            .active_leases
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Every runtime lease in one trusted user-visible session shares this
        // authority, including activate_concurrent(_bounded). A task therefore
        // cannot multiply the request limit by rotating or fanning out runtime
        // leases. The Weak registry also preserves continuity after all leases
        // are revoked while an old request permit is still alive.
        let request_budget = self.request_budget_for_family_locked(
            domain,
            &claims.user_id,
            &claims.session,
        );
        if replace_same_session {
            leases.retain(|_, lease| {
                lease.domain != domain
                    || lease.user_id != claims.user_id
                    || lease.session != claims.session
            });
        }
        if max_concurrent.is_some_and(|max_active| {
            leases
                .values()
                .filter(|lease| {
                    lease.domain == domain
                        && lease.user_id == claims.user_id
                        && lease.session == claims.session
                })
                .count()
                >= max_active
        }) {
            return Err(LoopbackCapabilityError::CapacityExceeded);
        }
        leases.insert(
            claims.lease_id.clone(),
            ActiveLoopbackLease {
                domain: domain.to_owned(),
                authorization_json,
                user_id: claims.user_id.clone(),
                session: claims.session.clone(),
                request_budget,
            },
        );
        self.prune_dead_request_budgets_locked();
        Ok((token, renewal_proof))
    }

    /// Verify short-lived access against signature, wall-clock expiry, active
    /// lease, domain, and the immutable authorization stored by the issuer.
    pub fn verify_access<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
        presented: &str,
    ) -> Result<(), LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        claims.validate_at(unix_time_secs())?;
        let authorization_json = Self::authorization_json(claims)?;
        let leases = self
            .active_leases
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(lease) = leases.get(&claims.lease_id) else {
            return Err(LoopbackCapabilityError::InvalidToken);
        };
        if lease.domain != domain || lease.authorization_json != authorization_json {
            return Err(LoopbackCapabilityError::InvalidToken);
        }
        claims.verify_token(self.root_secret.as_bytes(), domain, presented)
    }

    /// Verify access and acquire one request slot from the same active lease.
    ///
    /// Lease lookup, immutable-authorization comparison, token verification,
    /// and the atomic budget increment all happen while the authoritative
    /// registry read lock is held. A concurrent revoke/replacement therefore
    /// cannot create a check/acquire gap. The returned RAII permit refunds the
    /// slot on success, error propagation, cancellation, or panic unwinding.
    pub fn verify_access_and_acquire<S>(
        &self,
        domain: &str,
        claims: &LoopbackCapabilityClaims<S>,
        presented: &str,
        max_in_flight: usize,
    ) -> Result<LoopbackRequestPermit, LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        claims.validate_at(unix_time_secs())?;
        let authorization_json = Self::authorization_json(claims)?;
        let leases = self
            .active_leases
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(lease) = leases.get(&claims.lease_id) else {
            return Err(LoopbackCapabilityError::InvalidToken);
        };
        if lease.domain != domain || lease.authorization_json != authorization_json {
            return Err(LoopbackCapabilityError::InvalidToken);
        }
        claims.verify_token(self.root_secret.as_bytes(), domain, presented)?;
        lease.request_budget
            .try_acquire(&claims.lease_id, max_in_flight)
    }

    pub fn renew<S>(
        &self,
        domain: &str,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> Result<LoopbackCapabilityAccess<LoopbackCapabilityClaims<S>>, LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        self.renew_at(domain, request, unix_time_secs())
    }

    /// Clock-injected renewal seam used by deterministic expiry/sleep tests.
    pub fn renew_at<S>(
        &self,
        domain: &str,
        request: &LoopbackCapabilityRenewalRequest,
        now: u64,
    ) -> Result<LoopbackCapabilityAccess<LoopbackCapabilityClaims<S>>, LoopbackCapabilityError>
    where
        S: Clone + Serialize + for<'de> Deserialize<'de>,
    {
        if !canonical_required(&request.lease_id)
            || !canonical_required(&request.renewal_proof)
        {
            return Err(LoopbackCapabilityError::InvalidToken);
        }
        let lease = self
            .active_leases
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&request.lease_id)
            .cloned()
            .ok_or(LoopbackCapabilityError::InvalidToken)?;
        if lease.domain != domain
            || !verify_scoped_auth_token(
                self.root_secret.as_bytes(),
                &Self::renewal_domain(domain),
                lease.authorization_json.as_bytes(),
                &request.renewal_proof,
            )
        {
            return Err(LoopbackCapabilityError::InvalidToken);
        }
        let authorization: LoopbackCapabilityAuthorization<S> =
            serde_json::from_str(&lease.authorization_json)
                .map_err(|_| LoopbackCapabilityError::Malformed)?;
        let claims = authorization.into_claims_at(now)?;
        let token = claims.derive_token(self.root_secret.as_bytes(), domain)?;
        Ok(LoopbackCapabilityAccess { token, claims })
    }

    pub fn revoke(
        &self,
        domain: &str,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> Result<(), LoopbackCapabilityError> {
        let mut leases = self
            .active_leases
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(lease) = leases.get(&request.lease_id) else {
            return Err(LoopbackCapabilityError::InvalidToken);
        };
        if lease.domain != domain
            || !verify_scoped_auth_token(
                self.root_secret.as_bytes(),
                &Self::renewal_domain(domain),
                lease.authorization_json.as_bytes(),
                &request.renewal_proof,
            )
        {
            return Err(LoopbackCapabilityError::InvalidToken);
        }
        leases.remove(&request.lease_id);
        self.prune_dead_request_budgets_locked();
        Ok(())
    }

    /// Returns whether an immutable lease is still active for `domain`.
    ///
    /// Loopback service adapters use this read-only seam to reconcile
    /// domain-owned resources after a parent runtime drops its in-process lease
    /// guard. It exposes neither authorization claims nor renewal material.
    pub fn is_lease_active(&self, domain: &str, lease_id: &str) -> bool {
        self.active_leases
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(lease_id)
            .is_some_and(|lease| lease.domain == domain)
    }

    fn revoke_lease(&self, domain: &str, lease_id: &str) {
        let mut leases = self
            .active_leases
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if leases
            .get(lease_id)
            .is_some_and(|lease| lease.domain == domain)
        {
            leases.remove(lease_id);
            self.prune_dead_request_budgets_locked();
        }
    }
}

/// Main-process handle used by runtime/PTY teardown to revoke a child lease
/// even if the stdio bridge cannot perform its own best-effort revoke.
struct LoopbackCapabilityLeaseInner {
    issuer: Arc<LoopbackCapabilityIssuer>,
    domain: Arc<str>,
    lease_id: Arc<str>,
}

impl Drop for LoopbackCapabilityLeaseInner {
    fn drop(&mut self) {
        self.issuer.revoke_lease(&self.domain, &self.lease_id);
    }
}

/// Cloneable lifecycle guard for one active loopback lease. The final guard
/// dropping always revokes the lease, so a failed runtime/PTY build cannot
/// strand renewable authority in the process registry. Calling `revoke` on
/// any clone immediately revokes the shared lease; subsequent drops are
/// idempotent.
#[derive(Clone)]
pub struct LoopbackCapabilityLease {
    inner: Arc<LoopbackCapabilityLeaseInner>,
}

impl std::fmt::Debug for LoopbackCapabilityLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackCapabilityLease")
            .field("domain", &self.inner.domain)
            .field("lease_id", &self.inner.lease_id)
            .finish_non_exhaustive()
    }
}

impl LoopbackCapabilityLease {
    pub fn new(
        issuer: Arc<LoopbackCapabilityIssuer>,
        domain: impl Into<Arc<str>>,
        lease_id: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            inner: Arc::new(LoopbackCapabilityLeaseInner {
                issuer,
                domain: domain.into(),
                lease_id: lease_id.into(),
            }),
        }
    }

    pub fn revoke(&self) {
        self.inner
            .issuer
            .revoke_lease(&self.inner.domain, &self.inner.lease_id);
    }
}

/// Runtime-owned collection of loopback lease guards. Assemblers move the
/// guards into this set only after a child config is accepted; dropping a
/// partially built config or the final runtime/PTY set revokes automatically.
#[derive(Clone, Default)]
pub struct LoopbackCapabilityLeaseSet {
    leases: Vec<LoopbackCapabilityLease>,
}

impl std::fmt::Debug for LoopbackCapabilityLeaseSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackCapabilityLeaseSet")
            .field("len", &self.leases.len())
            .finish()
    }
}

impl LoopbackCapabilityLeaseSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, lease: LoopbackCapabilityLease) {
        self.leases.push(lease);
    }

    pub fn extend(
        &mut self,
        leases: impl IntoIterator<Item = LoopbackCapabilityLease>,
    ) {
        self.leases.extend(leases);
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    pub fn revoke_all(&self) {
        for lease in &self.leases {
            lease.revoke();
        }
    }
}

impl std::fmt::Display for LoopbackCapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed scoped capability claims",
            Self::UnsupportedVersion => "unsupported scoped capability version",
            Self::InvalidIdentity => "invalid scoped capability identity",
            Self::InvalidLifetime => "invalid scoped capability lifetime",
            Self::Expired => "scoped capability expired",
            Self::InvalidToolScope => "invalid scoped capability tool scope",
            Self::InvalidToken => "invalid scoped capability token",
            Self::CapacityExceeded => "scoped capability capacity exceeded",
        })
    }
}

impl std::error::Error for LoopbackCapabilityError {}

pub fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn canonical_required(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

impl<S> LoopbackCapabilityClaims<S>
where
    S: Serialize + for<'de> Deserialize<'de>,
{
    pub fn issue(
        user_id: impl AsRef<str>,
        session: LoopbackSessionBinding,
        allowed_tools: impl IntoIterator<Item = impl Into<String>>,
        scope: S,
    ) -> Result<Self, LoopbackCapabilityError> {
        Self::issue_at(
            user_id,
            session,
            allowed_tools,
            scope,
            unix_time_secs(),
            LOOPBACK_CAPABILITY_TTL_SECS,
        )
    }

    pub fn issue_at(
        user_id: impl AsRef<str>,
        session: LoopbackSessionBinding,
        allowed_tools: impl IntoIterator<Item = impl Into<String>>,
        scope: S,
        now: u64,
        ttl_secs: u64,
    ) -> Result<Self, LoopbackCapabilityError> {
        let mut allowed_tools: Vec<String> = allowed_tools.into_iter().map(Into::into).collect();
        allowed_tools.sort();
        allowed_tools.dedup();
        let user_id = UserId::parse(user_id.as_ref())
            .map_err(|_| LoopbackCapabilityError::InvalidIdentity)?;
        let claims = Self {
            version: LOOPBACK_CAPABILITY_VERSION,
            issued_at_unix_secs: now,
            expires_at_unix_secs: now.saturating_add(ttl_secs),
            lease_id: generate_loopback_lease_id()?,
            nonce: crate::generate_id(),
            user_id,
            session,
            allowed_tools,
            scope,
        };
        claims.validate_at(now)?;
        Ok(claims)
    }

    /// Validate the immutable authorization plus structural lifetime without
    /// requiring the access credential to still be current. Bridge bootstrap
    /// uses this only to recover `lease_id` before asking the issuer to restore
    /// authoritative scope; normal access must use [`Self::validate_at`].
    pub fn validate_renewable_shape(&self) -> Result<(), LoopbackCapabilityError> {
        if self.version != LOOPBACK_CAPABILITY_VERSION {
            return Err(LoopbackCapabilityError::UnsupportedVersion);
        }
        if !canonical_required(&self.lease_id)
            || !canonical_required(&self.nonce)
            || !canonical_required(&self.session.session_id)
        {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        self.session.validate_identity()?;
        if self.expires_at_unix_secs <= self.issued_at_unix_secs
            || self.expires_at_unix_secs - self.issued_at_unix_secs
                > LOOPBACK_CAPABILITY_TTL_SECS
        {
            return Err(LoopbackCapabilityError::InvalidLifetime);
        }
        if self.allowed_tools.is_empty()
            || self
                .allowed_tools
                .iter()
                .any(|tool| !canonical_required(tool))
            || self
                .allowed_tools
                .windows(2)
                .any(|pair| pair[0].as_str() >= pair[1].as_str())
        {
            return Err(LoopbackCapabilityError::InvalidToolScope);
        }
        Ok(())
    }

    pub fn validate_at(&self, now: u64) -> Result<(), LoopbackCapabilityError> {
        self.validate_renewable_shape()?;
        if self.issued_at_unix_secs > now.saturating_add(CLOCK_SKEW_SECS) {
            return Err(LoopbackCapabilityError::InvalidLifetime);
        }
        if now >= self.expires_at_unix_secs {
            return Err(LoopbackCapabilityError::Expired);
        }
        Ok(())
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools
            .binary_search_by(|candidate| candidate.as_str().cmp(tool))
            .is_ok()
    }

    fn to_json(&self) -> Result<String, LoopbackCapabilityError> {
        serde_json::to_string(self).map_err(|_| LoopbackCapabilityError::Malformed)
    }

    fn derive_token(
        &self,
        root_secret: &[u8],
        domain: &str,
    ) -> Result<String, LoopbackCapabilityError> {
        Ok(derive_scoped_auth_token(
            root_secret,
            domain,
            self.to_json()?.as_bytes(),
        ))
    }

    fn verify_token(
        &self,
        root_secret: &[u8],
        domain: &str,
        presented: &str,
    ) -> Result<(), LoopbackCapabilityError> {
        self.validate_at(unix_time_secs())?;
        let json = self.to_json()?;
        if verify_scoped_auth_token(root_secret, domain, json.as_bytes(), presented) {
            Ok(())
        } else {
            Err(LoopbackCapabilityError::InvalidToken)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const TEST_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Scope {
        resource: String,
    }

    fn claims(now: u64) -> LoopbackCapabilityClaims<Scope> {
        LoopbackCapabilityClaims::issue_at(
            TEST_USER_ID,
            LoopbackSessionBinding::conversation(TEST_CONVERSATION_ID),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap()
    }

    #[test]
    fn token_is_bound_to_secret_domain_and_payload() {
        let now = unix_time_secs();
        let claims = claims(now);
        let token = claims.derive_token(b"secret", "knowledge-v1").unwrap();
        assert!(claims.verify_token(b"secret", "knowledge-v1", &token).is_ok());
        assert!(claims.verify_token(b"other", "knowledge-v1", &token).is_err());
        assert!(claims.verify_token(b"secret", "requirement-v1", &token).is_err());

        let mut forged = claims;
        forged.scope.resource = "all".into();
        assert!(forged.verify_token(b"secret", "knowledge-v1", &token).is_err());
    }

    #[test]
    fn expired_future_and_cross_session_claims_fail_closed() {
        let now = 10_000;
        let expired = claims(now);
        assert_eq!(expired.validate_at(now + 61), Err(LoopbackCapabilityError::Expired));

        let mut future = claims(now);
        future.issued_at_unix_secs = now + CLOCK_SKEW_SECS + 1;
        future.expires_at_unix_secs = future.issued_at_unix_secs + 1;
        assert_eq!(future.validate_at(now), Err(LoopbackCapabilityError::InvalidLifetime));

        let mut mismatched = claims(now);
        mismatched.session.session_id = "0190f5fe-7c00-7a00-8000-000000000003".into();
        assert_eq!(mismatched.validate_at(now), Err(LoopbackCapabilityError::InvalidIdentity));
    }

    #[test]
    fn duplicate_or_empty_tool_scopes_are_rejected() {
        let now = 10_000;
        let mut empty = claims(now);
        empty.allowed_tools.clear();
        assert_eq!(empty.validate_at(now), Err(LoopbackCapabilityError::InvalidToolScope));

        let mut duplicate = claims(now);
        duplicate.allowed_tools = vec!["read".into(), "read".into()];
        assert_eq!(duplicate.validate_at(now), Err(LoopbackCapabilityError::InvalidToolScope));
    }

    #[test]
    fn external_process_binding_requires_no_conversation_identity() {
        let now = 10_000;
        let external = LoopbackCapabilityClaims::issue_at(
            TEST_USER_ID,
            LoopbackSessionBinding::external_process("external-random"),
            ["read"],
            Scope {
                resource: "kb".into(),
            },
            now,
            60,
        )
        .unwrap();
        assert_eq!(external.session.kind, LoopbackSessionKind::ExternalProcess);
        assert!(external.validate_at(now).is_ok());

        let mut forged = external;
        forged.session.conversation_id = Some("conversation".into());
        assert_eq!(
            forged.validate_at(now),
            Err(LoopbackCapabilityError::InvalidIdentity)
        );
    }

    #[test]
    fn session_binding_deserialization_rejects_prefixed_or_mismatched_entity_ids() {
        let legacy_prefixed = serde_json::json!({
            "kind": "conversation",
            "session_id": "conv_0190f5fe-7c00-7a00-8000-000000000001",
            "conversation_id": "conv_0190f5fe-7c00-7a00-8000-000000000001"
        });
        assert!(serde_json::from_value::<LoopbackSessionBinding>(legacy_prefixed).is_err());

        let mismatched = serde_json::json!({
            "kind": "conversation",
            "session_id": TEST_CONVERSATION_ID,
            "conversation_id": "0190f5fe-7c00-7a00-8000-000000000003"
        });
        assert!(serde_json::from_value::<LoopbackSessionBinding>(mismatched).is_err());
    }

    #[test]
    fn capability_claim_deserialization_rejects_noncanonical_user_id() {
        let mut value = serde_json::to_value(claims(unix_time_secs())).unwrap();
        value["user_id"] = serde_json::json!("1");
        assert!(serde_json::from_value::<LoopbackCapabilityClaims<Scope>>(value).is_err());
    }

    #[test]
    fn issuer_secret_uses_256_bits_of_os_randomness() {
        let first = generate_loopback_issuer_secret().unwrap();
        let second = generate_loopback_issuer_secret().unwrap();
        assert_ne!(first, second);
        assert_eq!(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(first)
                .unwrap()
                .len(),
            ISSUER_SECRET_BYTES
        );
    }

    #[test]
    fn active_lease_renews_expired_access_without_trusting_child_scope() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let original = claims(now);
        let (token, renewal_proof) = issuer.activate("knowledge-v2", &original).unwrap();
        assert!(issuer
            .verify_access("knowledge-v2", &original, &token)
            .is_ok());

        let request = LoopbackCapabilityRenewalRequest {
            lease_id: original.lease_id.clone(),
            renewal_proof,
        };
        let expired = issuer
            .renew_at::<Scope>(
                "knowledge-v2",
                &request,
                now.saturating_sub(LOOPBACK_CAPABILITY_TTL_SECS + 1),
            )
            .unwrap();
        assert_eq!(
            issuer.verify_access("knowledge-v2", &expired.claims, &expired.token),
            Err(LoopbackCapabilityError::Expired)
        );

        let renewed = issuer.renew::<Scope>("knowledge-v2", &request).unwrap();
        assert_eq!(renewed.claims.lease_id, original.lease_id);
        assert_eq!(renewed.claims.user_id, original.user_id);
        assert_eq!(renewed.claims.session, original.session);
        assert_eq!(renewed.claims.allowed_tools, original.allowed_tools);
        assert_eq!(renewed.claims.scope, original.scope);
        assert_ne!(renewed.claims.nonce, original.nonce);
        assert!(issuer
            .verify_access("knowledge-v2", &renewed.claims, &renewed.token)
            .is_ok());

        let mut forged = renewed.claims;
        forged.scope.resource = "other".into();
        let forged_token = forged
            .derive_token(issuer.root_secret.as_bytes(), "knowledge-v2")
            .unwrap();
        assert_eq!(
            issuer.verify_access("knowledge-v2", &forged, &forged_token),
            Err(LoopbackCapabilityError::InvalidToken)
        );
    }

    #[test]
    fn renewal_is_domain_bound_revocable_and_invalid_after_root_rotation() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let original = claims(now);
        let (_, renewal_proof) = issuer.activate("gateway-v2", &original).unwrap();
        let request = LoopbackCapabilityRenewalRequest {
            lease_id: original.lease_id.clone(),
            renewal_proof,
        };

        assert!(issuer.renew::<Scope>("knowledge-v2", &request).is_err());
        let rotated = LoopbackCapabilityIssuer::new("different-root");
        assert!(rotated.renew::<Scope>("gateway-v2", &request).is_err());

        issuer.revoke("gateway-v2", &request).unwrap();
        assert!(issuer.renew::<Scope>("gateway-v2", &request).is_err());
    }

    #[test]
    fn replacement_issuance_revokes_the_previous_same_session_lease() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let (_, first_proof) = issuer.activate("gateway-v2", &first).unwrap();
        let second = claims(now);
        issuer.activate("gateway-v2", &second).unwrap();

        assert!(issuer
            .renew::<Scope>(
                "gateway-v2",
                &LoopbackCapabilityRenewalRequest {
                    lease_id: first.lease_id,
                    renewal_proof: first_proof,
                },
            )
            .is_err());
    }

    #[test]
    fn bounded_concurrent_activation_is_scoped_by_domain_user_and_session() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        issuer
            .activate_concurrent_bounded("browser-v1", &first, 1)
            .unwrap();

        assert_eq!(
            issuer.activate_concurrent_bounded("browser-v1", &claims(now), 1),
            Err(LoopbackCapabilityError::CapacityExceeded)
        );
        issuer
            .activate_concurrent_bounded("other-domain", &claims(now), 1)
            .expect("a separate domain must retain independent capacity");

        let other_user = LoopbackCapabilityClaims::issue_at(
            "0190f5fe-7c00-7a00-8000-000000000003",
            LoopbackSessionBinding::conversation(TEST_CONVERSATION_ID),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap();
        issuer
            .activate_concurrent_bounded("browser-v1", &other_user, 1)
            .expect("a separate user must retain independent capacity");

        let other_session = LoopbackCapabilityClaims::issue_at(
            TEST_USER_ID,
            LoopbackSessionBinding::conversation(
                "0190f5fe-7c00-7a00-8000-000000000004",
            ),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap();
        issuer
            .activate_concurrent_bounded("browser-v1", &other_session, 1)
            .expect("a separate session must retain independent capacity");
    }

    #[test]
    fn verified_request_budget_is_exact_and_drop_restores_capacity() {
        const LIMIT: usize = 8;

        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let claims = claims(now);
        let (token, _) = issuer.activate("gateway-v2", &claims).unwrap();
        let mut permits = Vec::new();
        for _ in 0..LIMIT {
            permits.push(
                issuer
                    .verify_access_and_acquire(
                        "gateway-v2",
                        &claims,
                        &token,
                        LIMIT,
                    )
                    .expect("each slot up to the exact limit must be admitted"),
            );
        }
        assert!(matches!(
            issuer.verify_access_and_acquire(
                "gateway-v2",
                &claims,
                &token,
                LIMIT,
            ),
            Err(LoopbackCapabilityError::CapacityExceeded)
        ));

        let released = permits.pop().unwrap();
        assert_eq!(released.lease_id(), claims.lease_id);
        drop(released);
        permits.push(
            issuer
                .verify_access_and_acquire(
                    "gateway-v2",
                    &claims,
                    &token,
                    LIMIT,
                )
                .expect("dropping a permit must immediately refund its slot"),
        );
        drop(permits);
        assert!(
            issuer
                .verify_access_and_acquire(
                    "gateway-v2",
                    &claims,
                    &token,
                    LIMIT,
                )
                .is_ok()
        );
    }

    #[test]
    fn concurrent_leases_in_one_task_share_the_request_budget() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let second = claims(now);
        let (first_token, _) = issuer
            .activate_concurrent("gateway-v2", &first)
            .unwrap();
        let (second_token, _) = issuer
            .activate_concurrent("gateway-v2", &second)
            .unwrap();

        let first_permit = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            )
            .unwrap();
        assert!(matches!(
            issuer.verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            ),
            Err(LoopbackCapabilityError::CapacityExceeded)
        ));
        assert!(matches!(
            issuer.verify_access_and_acquire(
                "gateway-v2",
                &second,
                &second_token,
                1,
            ),
            Err(LoopbackCapabilityError::CapacityExceeded)
        ));
        drop(first_permit);
        let second_permit = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &second,
                &second_token,
                1,
            )
            .expect("a sibling runtime must recover the shared task capacity");
        assert_eq!(second_permit.lease_id(), second.lease_id);
    }

    #[test]
    fn request_budget_families_are_isolated_by_domain_user_and_session() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let other_domain = claims(now);
        let other_user = LoopbackCapabilityClaims::issue_at(
            "0190f5fe-7c00-7a00-8000-000000000003",
            LoopbackSessionBinding::conversation(TEST_CONVERSATION_ID),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap();
        let other_session = LoopbackCapabilityClaims::issue_at(
            TEST_USER_ID,
            LoopbackSessionBinding::conversation(
                "0190f5fe-7c00-7a00-8000-000000000004",
            ),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap();
        let (first_token, _) = issuer
            .activate_concurrent("gateway-v2", &first)
            .unwrap();
        let (domain_token, _) = issuer
            .activate_concurrent("other-domain", &other_domain)
            .unwrap();
        let (user_token, _) = issuer
            .activate_concurrent("gateway-v2", &other_user)
            .unwrap();
        let (session_token, _) = issuer
            .activate_concurrent("gateway-v2", &other_session)
            .unwrap();

        let _first = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            )
            .unwrap();
        let _domain = issuer
            .verify_access_and_acquire(
                "other-domain",
                &other_domain,
                &domain_token,
                1,
            )
            .expect("another domain must have independent capacity");
        let _user = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &other_user,
                &user_token,
                1,
            )
            .expect("another user must have independent capacity");
        let _session = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &other_session,
                &session_token,
                1,
            )
            .expect("another session must have independent capacity");
    }

    #[test]
    fn thirty_two_runtime_leases_still_share_exactly_eight_request_slots() {
        const LEASES: usize = 32;
        const ATTEMPTS_PER_LEASE: usize = 8;
        const REQUEST_LIMIT: usize = 8;

        let now = unix_time_secs();
        let issuer = Arc::new(LoopbackCapabilityIssuer::new("root-secret"));
        let mut accesses = Vec::new();
        for _ in 0..LEASES {
            let claims = claims(now);
            let (token, _) = issuer
                .activate_concurrent_bounded("gateway-v2", &claims, LEASES)
                .unwrap();
            accesses.push((claims, token));
        }
        let start = Arc::new(std::sync::Barrier::new(LEASES + 1));
        let mut threads = Vec::new();
        for (claims, token) in accesses {
            let issuer = Arc::clone(&issuer);
            let start = Arc::clone(&start);
            threads.push(std::thread::spawn(move || {
                start.wait();
                (0..ATTEMPTS_PER_LEASE)
                    .map(|_| {
                        issuer.verify_access_and_acquire(
                            "gateway-v2",
                            &claims,
                            &token,
                            REQUEST_LIMIT,
                        )
                    })
                    .collect::<Vec<_>>()
            }));
        }
        start.wait();
        let results: Vec<_> = threads
            .into_iter()
            .flat_map(|thread| {
                thread
                    .join()
                    .expect("runtime lease request contender panicked")
            })
            .collect();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            REQUEST_LIMIT
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(LoopbackCapabilityError::CapacityExceeded)))
                .count(),
            LEASES * ATTEMPTS_PER_LEASE - REQUEST_LIMIT
        );
    }

    #[test]
    fn replacement_lease_cannot_reset_a_sessions_in_flight_budget() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let (first_token, _) = issuer.activate("gateway-v2", &first).unwrap();
        let first_permit = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            )
            .unwrap();

        let replacement = claims(now);
        let (replacement_token, _) = issuer
            .activate("gateway-v2", &replacement)
            .unwrap();
        assert!(matches!(
            issuer.verify_access_and_acquire(
                "gateway-v2",
                &replacement,
                &replacement_token,
                1,
            ),
            Err(LoopbackCapabilityError::CapacityExceeded)
        ));
        drop(first_permit);
        assert!(
            issuer
                .verify_access_and_acquire(
                    "gateway-v2",
                    &replacement,
                    &replacement_token,
                    1,
                )
                .is_ok(),
            "the replacement must inherit and later recover the same budget"
        );
    }

    #[test]
    fn full_revocation_and_reissue_cannot_reset_a_live_request_budget() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let (first_token, first_proof) = issuer
            .activate_concurrent("gateway-v2", &first)
            .unwrap();
        let second = claims(now);
        let (_, second_proof) = issuer
            .activate_concurrent("gateway-v2", &second)
            .unwrap();
        let old_permit = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            )
            .unwrap();

        issuer
            .revoke(
                "gateway-v2",
                &LoopbackCapabilityRenewalRequest {
                    lease_id: first.lease_id.clone(),
                    renewal_proof: first_proof,
                },
            )
            .unwrap();
        issuer
            .revoke(
                "gateway-v2",
                &LoopbackCapabilityRenewalRequest {
                    lease_id: second.lease_id,
                    renewal_proof: second_proof,
                },
            )
            .unwrap();

        let replacement = claims(now);
        let (replacement_token, _) = issuer
            .activate_concurrent("gateway-v2", &replacement)
            .unwrap();
        assert!(matches!(
            issuer.verify_access_and_acquire(
                "gateway-v2",
                &replacement,
                &replacement_token,
                1,
            ),
            Err(LoopbackCapabilityError::CapacityExceeded)
        ));
        drop(old_permit);
        assert!(
            issuer
                .verify_access_and_acquire(
                    "gateway-v2",
                    &replacement,
                    &replacement_token,
                    1,
                )
                .is_ok(),
            "an old permit, not an active lease, must preserve family continuity"
        );
    }

    #[test]
    fn dead_weak_request_budget_families_converge_on_lease_mutation() {
        let now = unix_time_secs();
        let issuer = LoopbackCapabilityIssuer::new("root-secret");
        let first = claims(now);
        let (first_token, first_proof) = issuer
            .activate("gateway-v2", &first)
            .unwrap();
        let permit = issuer
            .verify_access_and_acquire(
                "gateway-v2",
                &first,
                &first_token,
                1,
            )
            .unwrap();
        issuer
            .revoke(
                "gateway-v2",
                &LoopbackCapabilityRenewalRequest {
                    lease_id: first.lease_id,
                    renewal_proof: first_proof,
                },
            )
            .unwrap();
        assert_eq!(issuer.request_budget_family_count(), 1);
        drop(permit);
        assert_eq!(
            issuer.request_budget_family_count(),
            1,
            "Drop stays lock-free; the dead Weak shell is pruned by the next mutation"
        );

        let other = LoopbackCapabilityClaims::issue_at(
            "0190f5fe-7c00-7a00-8000-000000000003",
            LoopbackSessionBinding::conversation(
                "0190f5fe-7c00-7a00-8000-000000000004",
            ),
            ["read"],
            Scope {
                resource: "alpha".into(),
            },
            now,
            60,
        )
        .unwrap();
        let (_, other_proof) = issuer.activate("gateway-v2", &other).unwrap();
        assert_eq!(
            issuer.request_budget_family_count(),
            1,
            "activation must prune the dead old key before publishing the new family"
        );
        issuer
            .revoke(
                "gateway-v2",
                &LoopbackCapabilityRenewalRequest {
                    lease_id: other.lease_id,
                    renewal_proof: other_proof,
                },
            )
            .unwrap();
        assert_eq!(issuer.request_budget_family_count(), 0);
    }

    #[test]
    fn racing_request_acquisition_cannot_exceed_the_exact_family_limit() {
        const LIMIT: usize = 8;
        const CONTENDERS: usize = 64;

        let now = unix_time_secs();
        let issuer = Arc::new(LoopbackCapabilityIssuer::new("root-secret"));
        let claims = claims(now);
        let (token, _) = issuer.activate("gateway-v2", &claims).unwrap();
        let mut threads = Vec::new();
        for _ in 0..CONTENDERS {
            let issuer = Arc::clone(&issuer);
            let claims = claims.clone();
            let token = token.clone();
            threads.push(std::thread::spawn(move || {
                issuer.verify_access_and_acquire(
                    "gateway-v2",
                    &claims,
                    &token,
                    LIMIT,
                )
            }));
        }
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().expect("request contender panicked"))
            .collect();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            LIMIT
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(LoopbackCapabilityError::CapacityExceeded)
                    )
                })
                .count(),
            CONTENDERS - LIMIT
        );
        drop(results);
        assert!(
            issuer
                .verify_access_and_acquire(
                    "gateway-v2",
                    &claims,
                    &token,
                    LIMIT,
                )
                .is_ok(),
            "dropping every racing result must refund every admitted slot"
        );
    }

    #[test]
    fn final_lease_guard_drop_revokes_but_intermediate_clone_drop_does_not() {
        let now = unix_time_secs();
        let issuer = Arc::new(LoopbackCapabilityIssuer::new("root-secret"));
        let original = claims(now);
        let (_, renewal_proof) = issuer.activate("gateway-v2", &original).unwrap();
        let request = LoopbackCapabilityRenewalRequest {
            lease_id: original.lease_id.clone(),
            renewal_proof,
        };
        let lease = LoopbackCapabilityLease::new(
            issuer.clone(),
            "gateway-v2",
            original.lease_id.clone(),
        );
        let runtime_lease = lease.clone();
        assert!(issuer.is_lease_active("gateway-v2", &original.lease_id));
        assert!(!issuer.is_lease_active("other-domain", &original.lease_id));

        // Dropping a partially consumed child config is safe once the runtime
        // has accepted a clone of its guard.
        drop(lease);
        assert!(issuer.renew::<Scope>("gateway-v2", &request).is_ok());

        // Runtime construction failure (or normal teardown) drops the last
        // guard and deterministically removes renewable authority.
        drop(runtime_lease);
        assert!(!issuer.is_lease_active("gateway-v2", &original.lease_id));
        assert_eq!(
            issuer.renew::<Scope>("gateway-v2", &request),
            Err(LoopbackCapabilityError::InvalidToken)
        );
    }

    #[test]
    fn explicit_revoke_from_any_clone_revokes_the_shared_lease_immediately() {
        let now = unix_time_secs();
        let issuer = Arc::new(LoopbackCapabilityIssuer::new("root-secret"));
        let original = claims(now);
        let (_, renewal_proof) = issuer.activate("gateway-v2", &original).unwrap();
        let request = LoopbackCapabilityRenewalRequest {
            lease_id: original.lease_id.clone(),
            renewal_proof,
        };
        let lease = LoopbackCapabilityLease::new(
            issuer.clone(),
            "gateway-v2",
            original.lease_id.clone(),
        );
        let clone = lease.clone();
        clone.revoke();

        assert_eq!(
            issuer.renew::<Scope>("gateway-v2", &request),
            Err(LoopbackCapabilityError::InvalidToken)
        );
        drop(lease);
        drop(clone);
    }
}
