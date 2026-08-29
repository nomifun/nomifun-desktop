use std::fmt;

use async_trait::async_trait;
use nomifun_agent_contracts::{ConnectionConfigRef, DigestHex, ModelRouteId};

use crate::contracts::{
    ChatCausality, ChatModelError, ChatProtocol, ChatRouteSelection, ProviderCredentialRef,
    ProviderIdRef, ResolvedChatRoute, ResolvedChatRouteSet,
};

#[async_trait]
pub trait ChatCausalityGate: Send + Sync {
    /// Validate committed turn authority, canonical cause, operation uniqueness,
    /// terminal/cancel fences, and primary-vs-shadow ownership.
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError>;
}

#[async_trait]
pub trait ChatRouteResolver: Send + Sync {
    /// Resolve one exact route revision plus its deterministic failover order.
    async fn resolve(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatModelError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialTarget {
    pub model_route_id: ModelRouteId,
    pub model_route_revision: u64,
    pub provider_id: ProviderIdRef,
    pub protocol: ChatProtocol,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
}

impl CredentialTarget {
    pub fn for_route(route: &ResolvedChatRoute) -> Self {
        Self {
            model_route_id: route.model_route_id.clone(),
            model_route_revision: route.model_route_revision,
            provider_id: route.provider_id.clone(),
            protocol: route.protocol,
            connection_config_ref: route.connection_config_ref.clone(),
            config_revision_digest: route.config_revision_digest.clone(),
        }
    }
}

/// Opaque, non-serializable credential authority returned by the centralized
/// store after it validates the credential reference against the exact route
/// target. It intentionally contains no public secret material.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialLease {
    credential_ref: ProviderCredentialRef,
    target: CredentialTarget,
    opaque_handle: String,
}

impl CredentialLease {
    pub fn new(
        credential_ref: ProviderCredentialRef,
        target: CredentialTarget,
        opaque_handle: impl Into<String>,
    ) -> Self {
        Self {
            credential_ref,
            target,
            opaque_handle: opaque_handle.into(),
        }
    }

    pub fn credential_ref(&self) -> &ProviderCredentialRef {
        &self.credential_ref
    }

    pub fn target(&self) -> &CredentialTarget {
        &self.target
    }

    /// Handle understood by the process-local provider transport. This is not
    /// credential material and is never serialized into model/runtime state.
    pub fn opaque_handle(&self) -> &str {
        &self.opaque_handle
    }

    pub fn validates_route(&self, route: &ResolvedChatRoute) -> bool {
        self.credential_ref == route.credential_ref
            && self.target == CredentialTarget::for_route(route)
            && !self.opaque_handle.is_empty()
    }
}

impl fmt::Debug for CredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialLease")
            .field("credential_ref", &self.credential_ref)
            .field("target", &self.target)
            .field("opaque_handle", &"[opaque]")
            .finish()
    }
}

#[async_trait]
pub trait ProviderCredentialStore: Send + Sync {
    /// Resolve an opaque reference for one exact provider/connection target.
    /// Implementations must fail synchronously on missing or mismatched refs.
    async fn lease(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
    ) -> Result<CredentialLease, ChatModelError>;
}
