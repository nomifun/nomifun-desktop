//! Product-level Browser Module, Resource, and Provider contracts.
//!
//! An Agent grant and a Browser provider are deliberately separate inputs.
//! A provider describes implementation availability; it can never create
//! Action authority. Every operation is admitted only when the immutable
//! AgentSession Action allowlist *and* the exact Browser Resource binding both
//! permit it.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::runtime::{BrowserResourceKey, WorkspaceError};

pub const BROWSER_MODULE_ID: &str = "browser";
pub const BROWSER_RESOURCE_KIND: &str = "browser";

pub const BROWSER_OBSERVE_ACTION_ID: &str = "browser/observe";
pub const BROWSER_NAVIGATE_ACTION_ID: &str = "browser/navigate";
pub const BROWSER_ACT_ACTION_ID: &str = "browser/act";
pub const BROWSER_RENDER_CONTENT_ACTION_ID: &str = "browser/render_content";
pub const BROWSER_DOWNLOAD_ACTION_ID: &str = "browser/download";
pub const BROWSER_UPLOAD_ACTION_ID: &str = "browser/upload";
pub const BROWSER_EVALUATE_ACTION_ID: &str = "browser/evaluate";

pub const BROWSER_ACTION_IDS: [&str; 7] = [
    BROWSER_OBSERVE_ACTION_ID,
    BROWSER_NAVIGATE_ACTION_ID,
    BROWSER_ACT_ACTION_ID,
    BROWSER_RENDER_CONTENT_ACTION_ID,
    BROWSER_DOWNLOAD_ACTION_ID,
    BROWSER_UPLOAD_ACTION_ID,
    BROWSER_EVALUATE_ACTION_ID,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCapabilityAction {
    Observe,
    Navigate,
    Act,
    RenderContent,
    Download,
    Upload,
    Evaluate,
}

impl BrowserCapabilityAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Observe => BROWSER_OBSERVE_ACTION_ID,
            Self::Navigate => BROWSER_NAVIGATE_ACTION_ID,
            Self::Act => BROWSER_ACT_ACTION_ID,
            Self::RenderContent => BROWSER_RENDER_CONTENT_ACTION_ID,
            Self::Download => BROWSER_DOWNLOAD_ACTION_ID,
            Self::Upload => BROWSER_UPLOAD_ACTION_ID,
            Self::Evaluate => BROWSER_EVALUATE_ACTION_ID,
        }
    }

    pub const fn resource_operation(self) -> BrowserResourceOperation {
        match self {
            Self::Observe => BrowserResourceOperation::Observe,
            Self::Navigate => BrowserResourceOperation::Navigate,
            Self::Act => BrowserResourceOperation::Act,
            Self::RenderContent => BrowserResourceOperation::RenderContent,
            Self::Download => BrowserResourceOperation::Download,
            Self::Upload => BrowserResourceOperation::Upload,
            Self::Evaluate => BrowserResourceOperation::Evaluate,
        }
    }

    pub fn parse(action_id: &str) -> Option<Self> {
        match action_id {
            BROWSER_OBSERVE_ACTION_ID => Some(Self::Observe),
            BROWSER_NAVIGATE_ACTION_ID => Some(Self::Navigate),
            BROWSER_ACT_ACTION_ID => Some(Self::Act),
            BROWSER_RENDER_CONTENT_ACTION_ID => Some(Self::RenderContent),
            BROWSER_DOWNLOAD_ACTION_ID => Some(Self::Download),
            BROWSER_UPLOAD_ACTION_ID => Some(Self::Upload),
            BROWSER_EVALUATE_ACTION_ID => Some(Self::Evaluate),
            _ => None,
        }
    }

    pub const fn all() -> [Self; 7] {
        [
            Self::Observe,
            Self::Navigate,
            Self::Act,
            Self::RenderContent,
            Self::Download,
            Self::Upload,
            Self::Evaluate,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserResourceOperation {
    Observe,
    Navigate,
    Act,
    RenderContent,
    Download,
    Upload,
    Evaluate,
}

impl BrowserResourceOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Navigate => "navigate",
            Self::Act => "act",
            Self::RenderContent => "render_content",
            Self::Download => "download",
            Self::Upload => "upload",
            Self::Evaluate => "evaluate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "observe" => Some(Self::Observe),
            "navigate" => Some(Self::Navigate),
            "act" => Some(Self::Act),
            "render_content" => Some(Self::RenderContent),
            "download" => Some(Self::Download),
            "upload" => Some(Self::Upload),
            "evaluate" => Some(Self::Evaluate),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProviderKind {
    /// Platform-owned embedded provider. Windows injects WebView2; macOS
    /// injects the independent CEF child NSView host. The product contract
    /// deliberately does not offer a renderer/backend selector.
    Managed,
    /// User-connected installation Chrome. This does not replace the managed
    /// native host and is not a separate Agent Capability.
    AttachedChrome,
}

/// Exact implementation selected by the trusted host.
///
/// `implemented_actions` is availability metadata, not an Action grant. The
/// provider has no API for mutating an AgentSession allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserProviderDescriptor {
    provider_id: String,
    kind: BrowserProviderKind,
    immutable_identity: String,
    implemented_actions: BTreeSet<BrowserCapabilityAction>,
}

impl BrowserProviderDescriptor {
    pub fn new(
        provider_id: impl Into<String>,
        kind: BrowserProviderKind,
        immutable_identity: impl Into<String>,
        implemented_actions: impl IntoIterator<Item = BrowserCapabilityAction>,
    ) -> Result<Self, BrowserContractError> {
        let provider_id = provider_id.into();
        let immutable_identity = immutable_identity.into();
        if !bounded_identity(&provider_id) || !bounded_identity(&immutable_identity) {
            return Err(BrowserContractError::InvalidProvider);
        }
        let implemented_actions = implemented_actions.into_iter().collect::<BTreeSet<_>>();
        if implemented_actions.is_empty() {
            return Err(BrowserContractError::InvalidProvider);
        }
        Ok(Self {
            provider_id,
            kind,
            immutable_identity,
            implemented_actions,
        })
    }

    pub fn managed(
        provider_id: impl Into<String>,
        immutable_identity: impl Into<String>,
    ) -> Result<Self, BrowserContractError> {
        Self::new(
            provider_id,
            BrowserProviderKind::Managed,
            immutable_identity,
            BrowserCapabilityAction::all(),
        )
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub const fn kind(&self) -> BrowserProviderKind {
        self.kind
    }

    pub fn immutable_identity(&self) -> &str {
        &self.immutable_identity
    }

    pub fn implements(&self, action: BrowserCapabilityAction) -> bool {
        self.implemented_actions.contains(&action)
    }

    pub fn implemented_actions(
        &self,
    ) -> impl Iterator<Item = BrowserCapabilityAction> + '_ {
        self.implemented_actions.iter().copied()
    }
}

/// One exact Browser Resource selected for one AgentSession.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserResourceBinding {
    binding_id: String,
    resource_id: String,
    owner_id: String,
    provider: BrowserProviderDescriptor,
    operations: BTreeSet<BrowserResourceOperation>,
}

impl BrowserResourceBinding {
    pub fn from_operation_names<I, S>(
        binding_id: impl Into<String>,
        resource_id: impl Into<String>,
        owner_id: impl Into<String>,
        provider: BrowserProviderDescriptor,
        operations: I,
    ) -> Result<Self, BrowserContractError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let operations = operations
            .into_iter()
            .map(|operation| {
                BrowserResourceOperation::parse(operation.as_ref())
                    .ok_or(BrowserContractError::InvalidResourceBinding)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(binding_id, resource_id, owner_id, provider, operations)
    }

    pub fn new(
        binding_id: impl Into<String>,
        resource_id: impl Into<String>,
        owner_id: impl Into<String>,
        provider: BrowserProviderDescriptor,
        operations: impl IntoIterator<Item = BrowserResourceOperation>,
    ) -> Result<Self, BrowserContractError> {
        let binding_id = binding_id.into();
        let resource_id = resource_id.into();
        let owner_id = owner_id.into();
        let operations = operations.into_iter().collect::<BTreeSet<_>>();
        if !bounded_identity(&binding_id)
            || !bounded_identity(&resource_id)
            || !bounded_identity(&owner_id)
            || operations.is_empty()
        {
            return Err(BrowserContractError::InvalidResourceBinding);
        }
        Ok(Self {
            binding_id,
            resource_id,
            owner_id,
            provider,
            operations,
        })
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn provider(&self) -> &BrowserProviderDescriptor {
        &self.provider
    }

    pub fn operations(&self) -> impl Iterator<Item = BrowserResourceOperation> + '_ {
        self.operations.iter().copied()
    }

    pub fn permits(&self, action: BrowserCapabilityAction) -> bool {
        self.operations.contains(&action.resource_operation())
    }
}

/// Immutable Action and Resource authority captured from one resolved
/// AgentSession Snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSessionAuthority {
    principal_id: String,
    agent_session_id: String,
    granted_actions: BTreeSet<BrowserCapabilityAction>,
    resource: BrowserResourceBinding,
}

impl BrowserSessionAuthority {
    pub fn from_action_ids<I, S>(
        principal_id: impl Into<String>,
        agent_session_id: impl Into<String>,
        granted_action_ids: I,
        resource: BrowserResourceBinding,
    ) -> Result<Self, BrowserContractError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let granted_actions = granted_action_ids
            .into_iter()
            .map(|action_id| {
                BrowserCapabilityAction::parse(action_id.as_ref())
                    .ok_or(BrowserContractError::UnknownAction)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(principal_id, agent_session_id, granted_actions, resource)
    }

    pub fn new(
        principal_id: impl Into<String>,
        agent_session_id: impl Into<String>,
        granted_actions: impl IntoIterator<Item = BrowserCapabilityAction>,
        resource: BrowserResourceBinding,
    ) -> Result<Self, BrowserContractError> {
        let principal_id = principal_id.into();
        let agent_session_id = agent_session_id.into();
        if !bounded_identity(&principal_id) || !bounded_identity(&agent_session_id) {
            return Err(BrowserContractError::InvalidSessionIdentity);
        }
        if resource.owner_id != principal_id {
            return Err(BrowserContractError::ResourceOwnerMismatch);
        }
        let granted_actions = granted_actions.into_iter().collect::<BTreeSet<_>>();
        Ok(Self {
            principal_id,
            agent_session_id,
            granted_actions,
            resource,
        })
    }

    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    pub fn agent_session_id(&self) -> &str {
        &self.agent_session_id
    }

    pub fn resource(&self) -> &BrowserResourceBinding {
        &self.resource
    }

    pub fn granted_actions(&self) -> impl Iterator<Item = BrowserCapabilityAction> + '_ {
        self.granted_actions.iter().copied()
    }

    pub fn key(&self) -> BrowserResourceKey {
        BrowserResourceKey {
            principal_id: self.principal_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            resource_binding_id: self.resource.binding_id.clone(),
        }
    }

    pub fn authorize(&self, action: BrowserCapabilityAction) -> Result<(), WorkspaceError> {
        if !self.granted_actions.contains(&action) || !self.resource.permits(action) {
            return Err(WorkspaceError::ActionDenied);
        }
        if !self.resource.provider.implements(action) {
            return Err(WorkspaceError::UnsupportedAction);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BrowserContractError {
    #[error("Browser provider identity is invalid")]
    InvalidProvider,
    #[error("Browser Resource binding is invalid")]
    InvalidResourceBinding,
    #[error("Browser AgentSession identity is invalid")]
    InvalidSessionIdentity,
    #[error("Browser Resource belongs to another principal")]
    ResourceOwnerMismatch,
    #[error("Browser Action ID is not part of the Browser Module")]
    UnknownAction,
}

fn bounded_identity(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: BrowserProviderKind) -> BrowserProviderDescriptor {
        BrowserProviderDescriptor::new(
            match kind {
                BrowserProviderKind::Managed => "managed",
                BrowserProviderKind::AttachedChrome => "attached-chrome",
            },
            kind,
            "immutable-provider-lock",
            BrowserCapabilityAction::all(),
        )
        .unwrap()
    }

    fn resource(kind: BrowserProviderKind) -> BrowserResourceBinding {
        BrowserResourceBinding::new(
            "binding-1",
            "resource-1",
            "alice",
            provider(kind),
            BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation),
        )
        .unwrap()
    }

    #[test]
    fn canonical_browser_surface_excludes_provider_controls_and_web_research() {
        assert_eq!(
            BROWSER_ACTION_IDS,
            [
                "browser/observe",
                "browser/navigate",
                "browser/act",
                "browser/render_content",
                "browser/download",
                "browser/upload",
                "browser/evaluate",
            ]
        );
        for non_action in ["web.research/search"] {
            assert_eq!(BrowserCapabilityAction::parse(non_action), None);
        }
    }

    #[test]
    fn provider_and_resource_availability_never_grant_actions() {
        let authority = BrowserSessionAuthority::new(
            "alice",
            "01990101-0000-7000-8000-000000000001",
            [],
            resource(BrowserProviderKind::Managed),
        )
        .unwrap();
        for action in BrowserCapabilityAction::all() {
            assert_eq!(authority.authorize(action), Err(WorkspaceError::ActionDenied));
        }
    }

    #[test]
    fn action_grant_and_resource_operation_are_both_required() {
        let resource = BrowserResourceBinding::new(
            "binding-1",
            "resource-1",
            "alice",
            provider(BrowserProviderKind::Managed),
            [BrowserResourceOperation::Observe],
        )
        .unwrap();
        let narrowed = BrowserSessionAuthority::new(
            "alice",
            "session-1",
            [BrowserCapabilityAction::Observe, BrowserCapabilityAction::Navigate],
            resource.clone(),
        )
        .unwrap();
        assert_eq!(
            narrowed.authorize(BrowserCapabilityAction::Navigate),
            Err(WorkspaceError::ActionDenied)
        );
        let authority = BrowserSessionAuthority::new(
            "alice",
            "session-1",
            [BrowserCapabilityAction::Observe],
            resource,
        )
        .unwrap();
        assert_eq!(authority.authorize(BrowserCapabilityAction::Observe), Ok(()));
        assert_eq!(
            authority.authorize(BrowserCapabilityAction::Navigate),
            Err(WorkspaceError::ActionDenied)
        );
        assert_eq!(
            authority.authorize(BrowserCapabilityAction::Act),
            Err(WorkspaceError::ActionDenied)
        );
    }

    #[test]
    fn managed_and_attached_providers_use_the_same_module_authority() {
        for kind in [BrowserProviderKind::Managed, BrowserProviderKind::AttachedChrome] {
            let authority = BrowserSessionAuthority::new(
                "alice",
                "delegated-agent-session",
                [BrowserCapabilityAction::Observe],
                resource(kind),
            )
            .unwrap();
            assert_eq!(authority.authorize(BrowserCapabilityAction::Observe), Ok(()));
            assert_eq!(authority.agent_session_id(), "delegated-agent-session");
        }
    }

    #[test]
    fn provider_support_can_only_narrow_an_existing_action_grant() {
        let attached = BrowserProviderDescriptor::new(
            "attached-chrome",
            BrowserProviderKind::AttachedChrome,
            "attached-lock",
            [BrowserCapabilityAction::Observe],
        )
        .unwrap();
        let resource = BrowserResourceBinding::new(
            "binding",
            "connection",
            "alice",
            attached,
            [
                BrowserResourceOperation::Observe,
                BrowserResourceOperation::Evaluate,
            ],
        )
        .unwrap();
        let authority = BrowserSessionAuthority::new(
            "alice",
            "session",
            [
                BrowserCapabilityAction::Observe,
                BrowserCapabilityAction::Evaluate,
            ],
            resource,
        )
        .unwrap();
        assert_eq!(
            authority.authorize(BrowserCapabilityAction::Evaluate),
            Err(WorkspaceError::UnsupportedAction)
        );
    }

    #[test]
    fn resource_keys_isolate_agent_sessions_and_bindings() {
        let first = BrowserSessionAuthority::new(
            "alice",
            "session-a",
            [BrowserCapabilityAction::Observe],
            resource(BrowserProviderKind::Managed),
        )
        .unwrap();
        let second = BrowserSessionAuthority::new(
            "alice",
            "session-b",
            [BrowserCapabilityAction::Observe],
            resource(BrowserProviderKind::Managed),
        )
        .unwrap();
        assert_ne!(first.key(), second.key());
        assert_eq!(first.key().agent_session_id, "session-a");
    }
}
