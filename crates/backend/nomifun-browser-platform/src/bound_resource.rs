//! Provider-neutral result of binding an authorized AgentSession Browser Resource.

use std::sync::Arc;

use crate::{
    attached_browser::AuthorizedAttachedBrowserResource,
    product::{BrowserProviderKind, BrowserSessionAuthority},
    run_guard::{BrowserInputState, BrowserRunSnapshot},
    runtime::WorkspaceError,
    workspace::{BrowserResource, BrowserResourceSnapshot},
};

#[derive(Clone)]
pub enum BoundBrowserProviderResource {
    Managed(Arc<BrowserResource>),
    AttachedChrome(Arc<AuthorizedAttachedBrowserResource>),
}

impl BoundBrowserProviderResource {
    pub fn provider_kind(&self) -> BrowserProviderKind {
        self.authority().resource().provider().kind()
    }

    pub fn authority(&self) -> &BrowserSessionAuthority {
        match self {
            Self::Managed(resource) => resource.authority(),
            Self::AttachedChrome(resource) => resource.authority(),
        }
    }

    /// Provider-neutral REST/product snapshot. Attached Chrome has no managed
    /// native-runtime generation; its installation connection status is
    /// exposed by the provider endpoint, while this snapshot proves the exact
    /// AgentSession Resource binding selected by the canonical route.
    pub async fn snapshot(&self) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        match self {
            Self::Managed(resource) => resource.snapshot().await,
            Self::AttachedChrome(_) => Ok(self.inactive_snapshot()),
        }
    }

    pub fn inactive_snapshot(&self) -> BrowserResourceSnapshot {
        let authority = self.authority();
        BrowserResourceSnapshot {
            agent_session_id: authority.agent_session_id().to_owned(),
            resource_binding_id: authority.resource().binding_id().to_owned(),
            provider_id: authority.resource().provider().provider_id().to_owned(),
            provider_kind: authority.resource().provider().kind(),
            allowed_actions: crate::product::BrowserCapabilityAction::all()
                .into_iter()
                .filter(|action| authority.authorize(*action).is_ok())
                .map(|action| action.action_id().to_owned())
                .collect(),
            run: BrowserRunSnapshot {
                revision: 0,
                input_state: BrowserInputState::UserReady,
                input_gate_failed: false,
            },
            runtime: None,
        }
    }

}
