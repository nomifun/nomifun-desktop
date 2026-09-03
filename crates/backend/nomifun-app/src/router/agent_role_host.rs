//! Typed first-party Browser/Computer role-host adapters.
//!
//! This module is an application-owned seam for the canonical
//! `system.browser_use` and `system.computer_use` roles. It intentionally
//! accepts already-resolved resources and provider identity; it does not
//! discover resources, interpret model selectors, call Gateway, or construct
//! a legacy facade. The platform hosts can be connected by the composition
//! owner once the canonical Agent dispatcher is ready.

#![allow(dead_code)]

use std::fmt;
#[cfg(any(feature = "browser-use", feature = "computer-use"))]
use std::sync::Arc;

use nomifun_agent_contracts::{
    ExactRoleProviderRef, PrincipalRef, ResolvedSnapshotRef, ResourceKind, TypedResourceBinding,
    TypedResourceBindings,
};
#[cfg(any(feature = "browser-use", feature = "computer-use"))]
use nomifun_agent_contracts::StrictJsonValue;
#[cfg(any(feature = "browser-use", feature = "computer-use"))]
use nomifun_agent_domain_wave2::{
    Wave2HostPort, Wave2HostPortError, Wave2HostRequest, Wave2TypedCapabilityOperation,
    Wave2TypedHostRequest,
};
#[cfg(feature = "computer-use")]
use nomifun_agent_domain_wave2::{
    Wave2ContextCapabilityOperation, Wave2ContextHostPort, Wave2ContextHostRequest,
};
#[cfg(feature = "computer-use")]
use nomifun_agent_kernel::ContextContributionResult;
#[cfg(feature = "browser-use")]
use nomifun_agent_kernel::{
    ContextContributionResult as BrowserContextContributionResult, ResourceProviderResult,
};
#[cfg(feature = "browser-use")]
use nomifun_agent_domain_wave2::{
    Wave2ContextCapabilityOperation as BrowserContextOperation, Wave2ContextHostPort as BrowserContextHostPort,
    Wave2ContextHostRequest as BrowserContextHostRequest, Wave2ResourceCapabilityOperation,
    Wave2OperationToolHostPort, Wave2OperationToolHostRequest, Wave2ResourceHostPort,
    Wave2ResourceHostRequest,
};

pub(crate) const BROWSER_ROLE_ID: &str = "system.browser_use";
pub(crate) const COMPUTER_ROLE_ID: &str = "system.computer_use";
pub(crate) const BROWSER_RESOURCE_KIND: &str = "browser";
pub(crate) const COMPUTER_RESOURCE_KIND: &str = "computer";

#[async_trait::async_trait]
#[cfg(any(feature = "browser-use", feature = "computer-use"))]
pub(crate) trait RoleHostInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, RoleHostError>;
}

/// Fixed provider adapter mounted into one Wave2 registration. Provider
/// selection remains in KernelRegistry; this type only translates the
/// compatibility envelope after that selection has already happened.
#[cfg(any(feature = "browser-use", feature = "computer-use"))]
pub(crate) struct RoleHostPortAdapter {
    invoker: Arc<dyn RoleHostInvoker>,
}

#[cfg(any(feature = "browser-use", feature = "computer-use"))]
impl RoleHostPortAdapter {
    pub(crate) fn new(invoker: Arc<dyn RoleHostInvoker>) -> Self {
        Self { invoker }
    }
}

#[cfg(any(feature = "browser-use", feature = "computer-use"))]
impl Wave2HostPort for RoleHostPortAdapter {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<StrictJsonValue, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        let typed = match request.into_typed() {
            Ok(request) => request,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let invoker = Arc::clone(&self.invoker);
        Box::pin(async move {
            invoker
                .invoke(typed)
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))
        })
    }
}

/// Typed failure returned by the role host before or after a concrete provider
/// call. The code is stable for callers; the message is diagnostic only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RoleHostError {
    InvalidContext(&'static str),
    ProviderMismatch,
    SnapshotMismatch,
    RegistryGenerationMismatch,
    ResourceCardinality,
    ResourceOwnerMismatch,
    ResourceOperationDenied(&'static str),
    ResourceIdentityMismatch,
    StaleObservationGeneration,
    ProviderUnavailable,
    ProviderFailure(String),
}

impl RoleHostError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::InvalidContext(_) => "ROLE_HOST_INVALID_CONTEXT",
            Self::ProviderMismatch => "ROLE_HOST_PROVIDER_MISMATCH",
            Self::SnapshotMismatch => "ROLE_HOST_SNAPSHOT_MISMATCH",
            Self::RegistryGenerationMismatch => "ROLE_HOST_REGISTRY_GENERATION_MISMATCH",
            Self::ResourceCardinality => "ROLE_HOST_RESOURCE_CARDINALITY",
            Self::ResourceOwnerMismatch => "ROLE_HOST_RESOURCE_OWNER_MISMATCH",
            Self::ResourceOperationDenied(_) => "ROLE_HOST_RESOURCE_OPERATION_DENIED",
            Self::ResourceIdentityMismatch => "ROLE_HOST_RESOURCE_IDENTITY_MISMATCH",
            Self::StaleObservationGeneration => "ROLE_HOST_STALE_OBSERVATION_GENERATION",
            Self::ProviderUnavailable => "ROLE_HOST_PROVIDER_UNAVAILABLE",
            Self::ProviderFailure(_) => "ROLE_HOST_PROVIDER_FAILURE",
        }
    }
}

impl fmt::Display for RoleHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContext(field) => {
                write!(formatter, "role host context is invalid: {field}")
            }
            Self::ProviderMismatch => {
                formatter.write_str("role provider does not match the frozen provider lock")
            }
            Self::SnapshotMismatch => {
                formatter.write_str("invocation snapshot does not match the frozen snapshot")
            }
            Self::RegistryGenerationMismatch => {
                formatter.write_str("invocation registry generation is not the frozen generation")
            }
            Self::ResourceCardinality => {
                formatter.write_str("role invocation requires exactly one matching resource")
            }
            Self::ResourceOwnerMismatch => {
                formatter.write_str("resource owner does not match the invocation principal")
            }
            Self::ResourceOperationDenied(operation) => {
                write!(formatter, "resource does not grant `{operation}`")
            }
            Self::ResourceIdentityMismatch => {
                formatter.write_str("resource identity does not match the bound provider target")
            }
            Self::StaleObservationGeneration => {
                formatter.write_str("computer observation generation is stale or missing")
            }
            Self::ProviderUnavailable => formatter.write_str("role provider is unavailable"),
            Self::ProviderFailure(message) => write!(formatter, "role provider failed: {message}"),
        }
    }
}

impl std::error::Error for RoleHostError {}

/// Server-derived facts frozen by the Agent Snapshot and by the trusted host.
/// None of these values are populated from model-visible operation JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleHostContext {
    pub principal: PrincipalRef,
    pub runtime_instance_id: String,
    pub owner_lease_id: String,
    pub snapshot: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub provider: ExactRoleProviderRef,
    pub resource_bindings: TypedResourceBindings,
}

impl RoleHostContext {
    fn validate_common(
        &self,
        expected_provider: &ExactRoleProviderRef,
        expected_snapshot: &ResolvedSnapshotRef,
        expected_registry_generation: u64,
    ) -> Result<(), RoleHostError> {
        if self.principal.principal_kind.trim().is_empty()
            || self.principal.principal_id.trim().is_empty()
        {
            return Err(RoleHostError::InvalidContext("principal"));
        }
        if self.runtime_instance_id.trim().is_empty() {
            return Err(RoleHostError::InvalidContext("runtime instance"));
        }
        if self.owner_lease_id.trim().is_empty() {
            return Err(RoleHostError::InvalidContext("owner lease"));
        }
        if self.snapshot != *expected_snapshot {
            return Err(RoleHostError::SnapshotMismatch);
        }
        if self.registry_generation != expected_registry_generation {
            return Err(RoleHostError::RegistryGenerationMismatch);
        }
        if self.provider != *expected_provider {
            return Err(RoleHostError::ProviderMismatch);
        }
        Ok(())
    }
}

fn exact_owned_resource<'a>(
    bindings: &'a [TypedResourceBinding],
    expected_kind: &'static str,
    principal: &PrincipalRef,
) -> Result<&'a TypedResourceBinding, RoleHostError> {
    let matching = bindings
        .iter()
        .filter(|binding| binding.resource_kind == ResourceKind::from(expected_kind))
        .collect::<Vec<_>>();
    let [binding] = matching.as_slice() else {
        return Err(RoleHostError::ResourceCardinality);
    };
    if binding.owner_id != principal.principal_id {
        return Err(RoleHostError::ResourceOwnerMismatch);
    }
    if binding.binding_id.as_ref().trim().is_empty()
        || binding.resource_id.as_ref().trim().is_empty()
    {
        return Err(RoleHostError::InvalidContext("resource identity"));
    }
    Ok(binding)
}

fn exact_resource<'a>(
    bindings: &'a [TypedResourceBinding],
    expected_kind: &'static str,
    principal: &PrincipalRef,
    operation: &'static str,
) -> Result<&'a TypedResourceBinding, RoleHostError> {
    let binding = exact_owned_resource(bindings, expected_kind, principal)?;
    if !binding.operations.contains(operation) {
        return Err(RoleHostError::ResourceOperationDenied(operation));
    }
    Ok(binding)
}

fn require_object(
    value: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, RoleHostError> {
    value
        .as_object()
        .cloned()
        .ok_or(RoleHostError::InvalidContext("operation input must be an object"))
}

fn reject_computer_control_fields(
    input: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), RoleHostError> {
    const TRUSTED_FIELDS: &[&str] = &[
        "user_id",
        "runtime_instance_id",
        "owner_lease_id",
        "resource_id",
        "resource_binding_id",
        "target_id",
        "generation",
        "expected_generation",
    ];
    if let Some(field) = TRUSTED_FIELDS
        .iter()
        .find(|field| input.contains_key(**field))
    {
        return Err(RoleHostError::InvalidContext(field));
    }
    Ok(())
}

fn reject_browser_control_fields(
    input: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), RoleHostError> {
    const TRUSTED_FIELDS: &[&str] = &[
        "user_id",
        "runtime_instance_id",
        "owner_lease_id",
        "surface",
        "identity",
        "identity_mode",
        "authenticated",
        "auth_identity",
        "profile",
        "account",
        "lane",
        "lane_name",
        "lane_id",
        "resource_id",
        "resource_binding_id",
        "target_id",
        "tab_id",
        "frame_id",
        "browser_epoch",
        "expected_browser_epoch",
        "ref_generation",
    ];
    if let Some(field) = TRUSTED_FIELDS
        .iter()
        .find(|field| input.contains_key(**field))
    {
        return Err(RoleHostError::InvalidContext(field));
    }
    Ok(())
}

#[cfg(feature = "browser-use")]
mod browser {
    use super::*;

    use nomifun_browser_platform::{
        BrowserLaneClient, BrowserLaneId, BrowserOperation, BrowserOperationKind,
        BrowserOperationResult, CallerIdentity,
    };

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct BrowserNavigate {
        pub url: String,
        pub new_tab: bool,
        pub expected_browser_epoch: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct BrowserObserve {
        pub max_depth: Option<u64>,
        pub expected_browser_epoch: u64,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct BrowserAct {
        pub action: String,
        pub parameters: serde_json::Value,
        pub expected_browser_epoch: u64,
        pub ref_generation: Option<u64>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct BrowserRenderContent {
        pub expected_browser_epoch: u64,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum BrowserRoleOperation {
        Navigate(BrowserNavigate),
        Observe(BrowserObserve),
        Act(BrowserAct),
        RenderContent(BrowserRenderContent),
    }

    impl BrowserRoleOperation {
        fn resource_operation(&self) -> &'static str {
            match self {
                Self::Navigate(_) => "navigate",
                Self::Observe(_) => "observe",
                Self::Act(_) => "interact",
                Self::RenderContent(_) => "navigate",
            }
        }

        fn act_kind(action: &str) -> Result<BrowserOperationKind, RoleHostError> {
            match action {
                "back" | "forward" | "reload" => Ok(BrowserOperationKind::Navigate),
                "get_page_text"
                | "search_page"
                | "find_elements"
                | "get_dropdown_options"
                | "cursor" => Ok(BrowserOperationKind::Observe),
                "screenshot" => Ok(BrowserOperationKind::Screenshot),
                "tabs" | "switch_tab" | "close_tab" | "open_link_new_tab" => {
                    Ok(BrowserOperationKind::Tabs)
                }
                "click"
                | "type"
                | "set_value"
                | "select_option"
                | "press_key"
                | "hover"
                | "scroll"
                | "wait" => Ok(BrowserOperationKind::Act),
                _ => Err(RoleHostError::InvalidContext("unsupported browser.act action")),
            }
        }

        fn act_may_modify_identity(action: &str) -> bool {
            matches!(
                action,
                "click" | "type" | "set_value" | "select_option" | "press_key"
            )
        }

        pub(crate) fn into_platform_operation(self) -> Result<BrowserOperation, RoleHostError> {
            match self {
                Self::Navigate(request) => {
                    if request.url.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("browser URL"));
                    }
                    Ok(BrowserOperation {
                        kind: BrowserOperationKind::Navigate,
                        action: "navigate".to_owned(),
                        input: serde_json::json!({
                            "url": request.url,
                            "new_tab": request.new_tab,
                        }),
                        expected_browser_epoch: Some(request.expected_browser_epoch),
                        target_id: None,
                        frame_id: None,
                        ref_generation: None,
                        may_modify_identity: false,
                    })
                }
                Self::Observe(request) => Ok(BrowserOperation {
                    kind: BrowserOperationKind::Observe,
                    action: "observe".to_owned(),
                    input: request
                        .max_depth
                        .map(|max_depth| serde_json::json!({ "max_depth": max_depth }))
                        .unwrap_or_else(|| serde_json::json!({})),
                    expected_browser_epoch: Some(request.expected_browser_epoch),
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                }),
                Self::Act(request) => {
                    if request.action.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("browser action"));
                    }
                    let input = require_object(request.parameters)?;
                    reject_browser_control_fields(&input)?;
                    let kind = Self::act_kind(&request.action)?;
                    let may_modify_identity = Self::act_may_modify_identity(&request.action);
                    Ok(BrowserOperation {
                        kind,
                        action: request.action,
                        input: serde_json::Value::Object(input),
                        expected_browser_epoch: Some(request.expected_browser_epoch),
                        target_id: None,
                        frame_id: None,
                        ref_generation: request.ref_generation,
                        may_modify_identity,
                    })
                }
                Self::RenderContent(request) => Ok(BrowserOperation {
                    kind: BrowserOperationKind::Debug,
                    action: "rendered_html".to_owned(),
                    input: serde_json::json!({}),
                    expected_browser_epoch: Some(request.expected_browser_epoch),
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                }),
            }
        }
    }

    /// Browser adapter bound to one already-authorized Hub client.
    ///
    /// The resource id is interpreted as the exact Hub Lane id. A caller that
    /// needs to create a Lane must do that in the resource owner/composition
    /// path and publish the resulting id into the Snapshot binding; this
    /// adapter never opens a lane from a model selector.
    pub(crate) struct BrowserRoleHost {
        client: BrowserLaneClient,
        lane_id: BrowserLaneId,
        expected_provider: ExactRoleProviderRef,
        expected_snapshot: ResolvedSnapshotRef,
        expected_registry_generation: u64,
    }

    impl BrowserRoleHost {
        pub(crate) fn new(
            client: BrowserLaneClient,
            lane_id: BrowserLaneId,
            expected_provider: ExactRoleProviderRef,
            expected_snapshot: ResolvedSnapshotRef,
            expected_registry_generation: u64,
        ) -> Self {
            Self {
                client,
                lane_id,
                expected_provider,
                expected_snapshot,
                expected_registry_generation,
            }
        }

        pub(crate) fn caller(&self) -> &CallerIdentity {
            self.client.caller()
        }

        pub(crate) async fn invoke(
            &self,
            context: RoleHostContext,
            operation: BrowserRoleOperation,
        ) -> Result<BrowserOperationResult, RoleHostError> {
            if context.provider.role.key.role_id.as_ref() != BROWSER_ROLE_ID {
                return Err(RoleHostError::InvalidContext("browser role id"));
            }
            context.validate_common(
                &self.expected_provider,
                &self.expected_snapshot,
                self.expected_registry_generation,
            )?;
            if self.client.caller().user_id != context.principal.principal_id {
                return Err(RoleHostError::ResourceOwnerMismatch);
            }
            if self.client.caller().runtime_instance_id != context.runtime_instance_id
                || self.client.caller().owner_lease_id.as_str() != context.owner_lease_id
            {
                return Err(RoleHostError::ResourceIdentityMismatch);
            }
            if !matches!(
                self.client.caller().surface,
                nomifun_browser_platform::BrowserSurface::Native
                    | nomifun_browser_platform::BrowserSurface::Acp
            ) {
                return Err(RoleHostError::ProviderUnavailable);
            }
            let _binding = exact_resource(
                &context.resource_bindings,
                BROWSER_RESOURCE_KIND,
                &context.principal,
                operation.resource_operation(),
            )?;
            let platform_operation = operation.into_platform_operation()?;

            // BrowserSessionHub owns exact-lane serialization. Adding an app
            // mutex here would either serialize unrelated lanes or hold a
            // synchronous guard across browser I/O.
            self.client
                .execute(&self.lane_id, platform_operation)
                .await
                .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))
        }
    }

}

#[cfg(feature = "browser-use")]
#[allow(unused_imports)]
pub(crate) use browser::{
    BrowserAct, BrowserNavigate, BrowserObserve, BrowserRenderContent, BrowserRoleHost,
    BrowserRoleOperation,
};

#[cfg(feature = "browser-use")]
pub(crate) struct BoundBrowserRoleInvoker {
    host: BrowserRoleHost,
}

#[cfg(feature = "browser-use")]
impl BoundBrowserRoleInvoker {
    pub(crate) fn new(
        client: nomifun_browser_platform::BrowserLaneClient,
        lane_id: nomifun_browser_platform::BrowserLaneId,
        provider: ExactRoleProviderRef,
        snapshot: ResolvedSnapshotRef,
        registry_generation: u64,
    ) -> Self {
        Self {
            host: BrowserRoleHost::new(client, lane_id, provider, snapshot, registry_generation),
        }
    }
}

#[cfg(feature = "browser-use")]
#[async_trait::async_trait]
impl RoleHostInvoker for BoundBrowserRoleInvoker {
    async fn invoke(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, RoleHostError> {
        let provider = request
            .context
            .role_provider
            .clone()
            .ok_or(RoleHostError::ProviderUnavailable)?;
        let caller = self.host.caller();
        let operation = match request.operation {
            Wave2TypedCapabilityOperation::BrowserNavigate { input } => {
                let object = require_object(input.0)?;
                BrowserRoleOperation::Navigate(BrowserNavigate {
                    url: object
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .ok_or(RoleHostError::InvalidContext("browser URL"))?
                        .to_owned(),
                    new_tab: object
                        .get("new_tab")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    expected_browser_epoch: object
                        .get("expected_browser_epoch")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(RoleHostError::InvalidContext(
                            "expected_browser_epoch",
                        ))?,
                })
            }
            Wave2TypedCapabilityOperation::BrowserAct { input } => {
                let mut object = require_object(input.0)?;
                let action = object
                    .remove("action")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("browser action"))?;
                let expected_browser_epoch = object
                    .remove("expected_browser_epoch")
                    .and_then(|value| value.as_u64())
                    .ok_or(RoleHostError::InvalidContext(
                        "expected_browser_epoch",
                    ))?;
                let ref_generation = object
                    .remove("ref_generation")
                    .and_then(|value| value.as_u64());
                BrowserRoleOperation::Act(BrowserAct {
                    action,
                    parameters: serde_json::Value::Object(object),
                    expected_browser_epoch,
                    ref_generation,
                })
            }
            Wave2TypedCapabilityOperation::BrowserRenderContent { input } => {
                let object = require_object(input.0)?;
                BrowserRoleOperation::RenderContent(BrowserRenderContent {
                    expected_browser_epoch: object
                        .get("expected_browser_epoch")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(RoleHostError::InvalidContext(
                            "expected_browser_epoch",
                        ))?,
                })
            }
            _ => {
                return Err(RoleHostError::InvalidContext(
                    "operation is not a Browser role member",
                ));
            }
        };
        let context = RoleHostContext {
            principal: request.context.principal,
            runtime_instance_id: caller.runtime_instance_id.clone(),
            owner_lease_id: caller.owner_lease_id.as_str().to_owned(),
            snapshot: request.context.resolved_snapshot_ref,
            registry_generation: request.context.registry_generation,
            provider,
            resource_bindings: request.context.resource_bindings,
        };
        let result = self.host.invoke(context, operation).await?;
        Ok(StrictJsonValue(serde_json::json!({
            "output": result.output,
            "tabs": result.tabs,
            "active_tab_id": result.active_tab_id,
            "active_frame_id": result.active_frame_id,
            "ref_generation": result.ref_generation
        })))
    }
}

#[cfg(feature = "browser-use")]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BrowserResourceKey {
    scope_key: String,
    provider_digest: String,
    binding_id: String,
}

#[cfg(feature = "browser-use")]
struct BrowserRoleResource {
    identity: nomifun_agent_kernel::ResourceHandleIdentity,
    client: nomifun_browser_platform::BrowserLaneClient,
    lane_id: nomifun_browser_platform::BrowserLaneId,
    hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    renewal: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    released: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "browser-use")]
impl BrowserRoleResource {
    fn key(
        context: &nomifun_agent_domain_wave2::Wave2RoleMemberContext,
        binding: &TypedResourceBinding,
    ) -> BrowserResourceKey {
        BrowserResourceKey {
            scope_key: context.state_scope_key.as_ref().to_owned(),
            provider_digest: context
                .role_provider
                .contribution_digest
                .as_ref()
                .to_owned(),
            binding_id: binding.binding_id.as_ref().to_owned(),
        }
    }

    fn action_key(
        request: &Wave2TypedHostRequest,
        binding: &TypedResourceBinding,
    ) -> Result<BrowserResourceKey, RoleHostError> {
        let provider = request
            .context
            .role_provider
            .as_ref()
            .ok_or(RoleHostError::ProviderUnavailable)?;
        Ok(BrowserResourceKey {
            scope_key: format!("session:{}", request.context.agent_session_id.as_ref()),
            provider_digest: provider
                .contribution_digest
                .as_ref()
                .to_owned(),
            binding_id: binding.binding_id.as_ref().to_owned(),
        })
    }

    fn resolved_key(
        context: &nomifun_agent_kernel::ResolvedRoleMemberContext,
        binding: &TypedResourceBinding,
    ) -> BrowserResourceKey {
        BrowserResourceKey {
            scope_key: context.state_scope_key.as_ref().to_owned(),
            provider_digest: context
                .provider_lock
                .provider
                .contribution_digest
                .as_ref()
                .to_owned(),
            binding_id: binding.binding_id.as_ref().to_owned(),
        }
    }

    fn start_renewal(
        hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
        lease_id: nomifun_browser_platform::OwnerLeaseId,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if hub.renew_owner_lease(&lease_id).is_err() {
                    let _ = hub.revoke_owner_lease(&lease_id).await;
                    break;
                }
            }
        })
    }
}

#[cfg(feature = "browser-use")]
#[async_trait::async_trait]
impl nomifun_agent_kernel::ResourceHandle for BrowserRoleResource {
    fn identity(&self) -> &nomifun_agent_kernel::ResourceHandleIdentity {
        &self.identity
    }

    async fn release(&self) -> Result<(), nomifun_agent_kernel::KernelError> {
        use std::sync::atomic::Ordering;

        if self.released.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if let Some(task) = self
            .renewal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
        self.hub
            .revoke_owner_lease(&self.client.caller().owner_lease_id)
            .await
            .map(|_| ())
            .map_err(|error| nomifun_agent_kernel::KernelError::CapabilityExecution {
                reason: format!("Browser resource cleanup failed: {error}"),
            })
    }
}

#[cfg(feature = "browser-use")]
impl Drop for BrowserRoleResource {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;

        if self.released.load(Ordering::Acquire) {
            return;
        }
        if let Some(task) = self
            .renewal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
        let _ = self
            .client
            .handoff_bound_lane_cleanup(self.lane_id.clone());
    }
}

#[cfg(feature = "browser-use")]
pub(crate) struct BrowserRoleRuntime {
    hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    resources: tokio::sync::Mutex<
        std::collections::HashMap<
            BrowserResourceKey,
            std::sync::Weak<BrowserRoleResource>,
        >,
    >,
}

#[cfg(feature = "browser-use")]
impl BrowserRoleRuntime {
    pub(crate) fn new(hub: Arc<nomifun_browser_platform::BrowserSessionHub>) -> Self {
        Self {
            hub,
            resources: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    async fn wait_until_running(
        client: &nomifun_browser_platform::BrowserLaneClient,
        lane_id: &nomifun_browser_platform::BrowserLaneId,
    ) -> Result<(), RoleHostError> {
        for _ in 0..120 {
            let status = client
                .status(lane_id)
                .await
                .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
            match status.lifecycle_state {
                nomifun_browser_platform::LaneLifecycleState::Running
                | nomifun_browser_platform::LaneLifecycleState::Frozen => return Ok(()),
                nomifun_browser_platform::LaneLifecycleState::Failed
                | nomifun_browser_platform::LaneLifecycleState::Stopping => {
                    return Err(RoleHostError::ProviderUnavailable);
                }
                nomifun_browser_platform::LaneLifecycleState::Queued
                | nomifun_browser_platform::LaneLifecycleState::Starting => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
        Err(RoleHostError::ProviderFailure(
            "Browser lane did not become ready before its 30 second deadline".to_owned(),
        ))
    }

    async fn acquire(
        &self,
        context: &nomifun_agent_kernel::ResolvedRoleMemberContext,
    ) -> Result<Arc<BrowserRoleResource>, RoleHostError> {
        let binding = exact_owned_resource(
            &context.resource_bindings,
            BROWSER_RESOURCE_KIND,
            &context.principal,
        )?;
        let key = BrowserRoleResource::resolved_key(context, binding);
        {
            let mut resources = self.resources.lock().await;
            resources.retain(|_, handle| handle.strong_count() > 0);
            if let Some(handle) = resources.get(&key).and_then(std::sync::Weak::upgrade) {
                return Ok(handle);
            }
        }

        let agent_session_id = context
            .agent_session_id
            .as_ref()
            .map(|session_id| session_id.as_ref().to_owned());
        let runtime_instance_id = agent_session_id
            .as_ref()
            .map(|session_id| format!("agent-session:{session_id}"))
            .unwrap_or_else(|| {
                format!("role-operation:{}", context.state_scope_key.as_ref())
            });
        let lease = self
            .hub
            .issue_owner_lease(
                context.principal.principal_id.clone(),
                agent_session_id.clone(),
                runtime_instance_id.clone(),
            )
            .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
        let caller = nomifun_browser_platform::CallerIdentity {
            user_id: context.principal.principal_id.clone(),
            conversation_id: agent_session_id.clone(),
            runtime_instance_id,
            agent_id: agent_session_id,
            companion_id: None,
            execution_id: None,
            step_id: None,
            attempt_id: None,
            remote_connection_id: None,
            surface: if context.agent_session_id.is_some() {
                nomifun_browser_platform::BrowserSurface::Acp
            } else {
                nomifun_browser_platform::BrowserSurface::System
            },
            owner_lease_id: lease.lease_id.clone(),
            capability_expires_at_ms: u64::MAX,
            allowed_operations: std::collections::BTreeSet::from([
                nomifun_browser_platform::BrowserOperationKind::Navigate,
                nomifun_browser_platform::BrowserOperationKind::Observe,
                nomifun_browser_platform::BrowserOperationKind::Act,
                nomifun_browser_platform::BrowserOperationKind::Screenshot,
                nomifun_browser_platform::BrowserOperationKind::Tabs,
                nomifun_browser_platform::BrowserOperationKind::Download,
                nomifun_browser_platform::BrowserOperationKind::Debug,
                nomifun_browser_platform::BrowserOperationKind::Manage,
                nomifun_browser_platform::BrowserOperationKind::Crawl,
            ]),
        };
        let client = match self.hub.bind(caller) {
            Ok(client) => client,
            Err(error) => {
                let _ = self.hub.revoke_owner_lease(&lease.lease_id).await;
                return Err(RoleHostError::ProviderFailure(error.to_string()));
            }
        };
        let opened = match client
            .open(
                Some("default"),
                if context.agent_session_id.is_some() {
                    nomifun_browser_platform::BrowserIdentityMode::Primary
                } else {
                    nomifun_browser_platform::BrowserIdentityMode::Anonymous
                },
                None,
            )
            .await
        {
            Ok(opened) => opened,
            Err(error) => {
                let _ = self.hub.revoke_owner_lease(&lease.lease_id).await;
                return Err(RoleHostError::ProviderFailure(error.to_string()));
            }
        };
        let lane_id = opened.lane().lane_id.clone();
        if let Err(error) = Self::wait_until_running(&client, &lane_id).await {
            let _ = self.hub.revoke_owner_lease(&lease.lease_id).await;
            return Err(error);
        }
        let handle = Arc::new(BrowserRoleResource {
            identity: nomifun_agent_kernel::ResourceHandleIdentity {
                binding_id: binding.binding_id.clone(),
                resource_kind: binding.resource_kind.clone(),
                resource_id: binding.resource_id.clone(),
            },
            client,
            lane_id,
            hub: Arc::clone(&self.hub),
            renewal: std::sync::Mutex::new(Some(BrowserRoleResource::start_renewal(
                Arc::clone(&self.hub),
                lease.lease_id,
            ))),
            released: std::sync::atomic::AtomicBool::new(false),
        });

        let mut resources = self.resources.lock().await;
        if let Some(existing) = resources.get(&key).and_then(std::sync::Weak::upgrade) {
            drop(resources);
            nomifun_agent_kernel::ResourceHandle::release(handle.as_ref())
                .await
                .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
            return Ok(existing);
        }
        resources.insert(key, Arc::downgrade(&handle));
        Ok(handle)
    }

    async fn action_resource(
        &self,
        request: &Wave2TypedHostRequest,
        operation: &'static str,
    ) -> Result<Arc<BrowserRoleResource>, RoleHostError> {
        let binding = exact_resource(
            &request.context.resource_bindings,
            BROWSER_RESOURCE_KIND,
            &request.context.principal,
            operation,
        )?;
        let key = BrowserRoleResource::action_key(request, binding)?;
        self.resources
            .lock()
            .await
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
            .ok_or(RoleHostError::ProviderUnavailable)
    }

    async fn context_resource(
        &self,
        context: &nomifun_agent_domain_wave2::Wave2RoleMemberContext,
        operation: &'static str,
    ) -> Result<Arc<BrowserRoleResource>, RoleHostError> {
        let binding = exact_resource(
            &context.resource_bindings,
            BROWSER_RESOURCE_KIND,
            &context.principal,
            operation,
        )?;
        let key = BrowserRoleResource::key(context, binding);
        self.resources
            .lock()
            .await
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
            .ok_or(RoleHostError::ProviderUnavailable)
    }

    async fn operation_resource(
        &self,
        context: &nomifun_agent_kernel::ResolvedRoleMemberContext,
        operation: &'static str,
    ) -> Result<Arc<BrowserRoleResource>, RoleHostError> {
        let binding = exact_resource(
            &context.resource_bindings,
            BROWSER_RESOURCE_KIND,
            &context.principal,
            operation,
        )?;
        let key = BrowserRoleResource::resolved_key(context, binding);
        self.resources
            .lock()
            .await
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
            .ok_or(RoleHostError::ProviderUnavailable)
    }
}

#[cfg(feature = "browser-use")]
#[async_trait::async_trait]
impl RoleHostInvoker for BrowserRoleRuntime {
    async fn invoke(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, RoleHostError> {
        let provider = request
            .context
            .role_provider
            .clone()
            .ok_or(RoleHostError::ProviderUnavailable)?;
        let required_operation = match &request.operation {
            Wave2TypedCapabilityOperation::BrowserNavigate { .. }
            | Wave2TypedCapabilityOperation::BrowserRenderContent { .. } => "navigate",
            Wave2TypedCapabilityOperation::BrowserAct { .. } => "interact",
            _ => {
                return Err(RoleHostError::InvalidContext(
                    "operation is not an implemented Browser role member",
                ));
            }
        };
        let resource = self
            .action_resource(&request, required_operation)
            .await?;
        let status = resource
            .client
            .status(&resource.lane_id)
            .await
            .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
        let host = BrowserRoleHost::new(
            resource.client.clone(),
            resource.lane_id.clone(),
            provider.clone(),
            request.context.resolved_snapshot_ref.clone(),
            request.context.registry_generation,
        );
        let caller = host.caller();
        let role_context = RoleHostContext {
            principal: request.context.principal.clone(),
            runtime_instance_id: caller.runtime_instance_id.clone(),
            owner_lease_id: caller.owner_lease_id.as_str().to_owned(),
            snapshot: request.context.resolved_snapshot_ref.clone(),
            registry_generation: request.context.registry_generation,
            provider,
            resource_bindings: request.context.resource_bindings.clone(),
        };

        if let Wave2TypedCapabilityOperation::BrowserRenderContent { input } =
            request.operation
        {
            let mut object = require_object(input.0)?;
            reject_browser_control_fields(&object)?;
            let url = object
                .remove("url")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or(RoleHostError::InvalidContext("browser URL"))?;
            if !object.is_empty() {
                return Err(RoleHostError::InvalidContext(
                    "unsupported browser.render_content field",
                ));
            }
            host.invoke(
                role_context.clone(),
                BrowserRoleOperation::Navigate(BrowserNavigate {
                    url,
                    new_tab: false,
                    expected_browser_epoch: status.browser_epoch,
                }),
            )
            .await?;
            let current = resource
                .client
                .status(&resource.lane_id)
                .await
                .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
            let rendered = host
                .invoke(
                    role_context,
                    BrowserRoleOperation::RenderContent(BrowserRenderContent {
                        expected_browser_epoch: current.browser_epoch,
                    }),
                )
                .await?;
            let final_url = current
                .tabs
                .iter()
                .find(|tab| tab.active)
                .and_then(|tab| tab.url.clone());
            return Ok(StrictJsonValue(serde_json::json!({
                "final_url": final_url,
                "html": rendered.output.get("html").cloned().unwrap_or(serde_json::Value::Null),
                "html_truncated": rendered.output
                    .get("html_truncated")
                    .cloned()
                    .unwrap_or(serde_json::Value::Bool(false))
            })));
        }

        let operation = match request.operation {
            Wave2TypedCapabilityOperation::BrowserNavigate { input } => {
                let mut object = require_object(input.0)?;
                reject_browser_control_fields(&object)?;
                let url = object
                    .remove("url")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("browser URL"))?;
                let new_tab = object
                    .remove("new_tab")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if !object.is_empty() {
                    return Err(RoleHostError::InvalidContext(
                        "unsupported browser.navigate field",
                    ));
                }
                BrowserRoleOperation::Navigate(BrowserNavigate {
                    url,
                    new_tab,
                    expected_browser_epoch: status.browser_epoch,
                })
            }
            Wave2TypedCapabilityOperation::BrowserAct { input } => {
                let mut object = require_object(input.0)?;
                let action = object
                    .remove("action")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("browser action"))?;
                let ref_generation = object
                    .remove("ref_generation")
                    .and_then(|value| value.as_u64());
                reject_browser_control_fields(&object)?;
                BrowserRoleOperation::Act(BrowserAct {
                    action,
                    parameters: serde_json::Value::Object(object),
                    expected_browser_epoch: status.browser_epoch,
                    ref_generation,
                })
            }
            _ => unreachable!("Browser operation was classified above"),
        };
        let result = host.invoke(role_context, operation).await?;
        Ok(StrictJsonValue(serde_json::json!({
            "output": result.output,
            "tabs": result.tabs,
            "active_tab_id": result.active_tab_id,
            "active_frame_id": result.active_frame_id,
            "ref_generation": result.ref_generation
        })))
    }
}

#[cfg(feature = "browser-use")]
impl BrowserContextHostPort for BrowserRoleRuntime {
    fn contribute<'a>(
        &'a self,
        request: BrowserContextHostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<BrowserContextContributionResult, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if matches!(request.operation, BrowserContextOperation::BrowserSiteMemory) {
                return Ok(BrowserContextContributionResult { value: None });
            }
            if !matches!(request.operation, BrowserContextOperation::BrowserObserve) {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_INVALID_CONTEXT",
                    "Browser provider received an unsupported context member",
                ));
            }
            let context = request.context;
            let resource = self
                .context_resource(&context, "observe")
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            let status = resource
                .client
                .status(&resource.lane_id)
                .await
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_PROVIDER_FAILURE",
                        error.to_string(),
                    )
                })?;
            let host = BrowserRoleHost::new(
                resource.client.clone(),
                resource.lane_id.clone(),
                context.role_provider.clone(),
                context.resolved_snapshot_ref.clone(),
                context.registry_generation,
            );
            let caller = host.caller();
            let result = host
                .invoke(
                    RoleHostContext {
                        principal: context.principal,
                        runtime_instance_id: caller.runtime_instance_id.clone(),
                        owner_lease_id: caller.owner_lease_id.as_str().to_owned(),
                        snapshot: context.resolved_snapshot_ref,
                        registry_generation: context.registry_generation,
                        provider: context.role_provider,
                        resource_bindings: context.resource_bindings,
                    },
                    BrowserRoleOperation::Observe(BrowserObserve {
                        max_depth: None,
                        expected_browser_epoch: status.browser_epoch,
                    }),
                )
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            Ok(BrowserContextContributionResult {
                value: Some(StrictJsonValue(serde_json::json!({
                    "output": result.output,
                    "tabs": result.tabs,
                    "active_tab_id": result.active_tab_id,
                    "active_frame_id": result.active_frame_id,
                    "ref_generation": result.ref_generation
                }))),
            })
        })
    }
}

#[cfg(feature = "browser-use")]
impl Wave2ResourceHostPort for BrowserRoleRuntime {
    fn acquire<'a>(
        &'a self,
        request: Wave2ResourceHostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<ResourceProviderResult, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if !matches!(
                request.operation,
                Wave2ResourceCapabilityOperation::BrowserIdentity
            ) {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_INVALID_CONTEXT",
                    "Browser provider received an unsupported resource member",
                ));
            }
            let handle = self
                .acquire(&request.context)
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            Ok(ResourceProviderResult { handle })
        })
    }
}

#[cfg(feature = "browser-use")]
impl Wave2OperationToolHostPort for BrowserRoleRuntime {
    fn invoke<'a>(
        &'a self,
        request: Wave2OperationToolHostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<StrictJsonValue, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if request.context.provider_lock.provider.role.key.role_id != BROWSER_ROLE_ID.into() {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_PROVIDER_MISMATCH",
                    "non-Agent Browser operation used a different execution Role",
                ));
            }
            let Wave2TypedCapabilityOperation::BrowserRenderContent { input } =
                request.operation
            else {
                return Err(Wave2HostPortError::unavailable(
                    "this first-party operation host currently exposes only browser.render_content",
                ));
            };
            let mut object = require_object(input.0)
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            reject_browser_control_fields(&object)
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            let url = object
                .remove("url")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_INVALID_CONTEXT",
                        "browser.render_content requires a URL",
                    )
                })?;
            if !object.is_empty() {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_INVALID_CONTEXT",
                    "unsupported browser.render_content field",
                ));
            }
            let resource = self
                .operation_resource(&request.context, "navigate")
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            let initial = resource
                .client
                .status(&resource.lane_id)
                .await
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_PROVIDER_FAILURE",
                        error.to_string(),
                    )
                })?;
            resource
                .client
                .execute(
                    &resource.lane_id,
                    BrowserRoleOperation::Navigate(BrowserNavigate {
                        url,
                        new_tab: false,
                        expected_browser_epoch: initial.browser_epoch,
                    })
                    .into_platform_operation()
                    .map_err(|error| {
                        Wave2HostPortError::new(error.code(), error.to_string())
                    })?,
                )
                .await
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_PROVIDER_FAILURE",
                        error.to_string(),
                    )
                })?;
            let current = resource
                .client
                .status(&resource.lane_id)
                .await
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_PROVIDER_FAILURE",
                        error.to_string(),
                    )
                })?;
            let rendered = resource
                .client
                .execute(
                    &resource.lane_id,
                    BrowserRoleOperation::RenderContent(BrowserRenderContent {
                        expected_browser_epoch: current.browser_epoch,
                    })
                    .into_platform_operation()
                    .map_err(|error| {
                        Wave2HostPortError::new(error.code(), error.to_string())
                    })?,
                )
                .await
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "ROLE_HOST_PROVIDER_FAILURE",
                        error.to_string(),
                    )
                })?;
            let final_url = current
                .tabs
                .iter()
                .find(|tab| tab.active)
                .and_then(|tab| tab.url.clone());
            Ok(StrictJsonValue(serde_json::json!({
                "final_url": final_url,
                "html": rendered.output.get("html").cloned().unwrap_or(serde_json::Value::Null),
                "html_truncated": rendered.output
                    .get("html_truncated")
                    .cloned()
                    .unwrap_or(serde_json::Value::Bool(false))
            })))
        })
    }
}

#[cfg(feature = "computer-use")]
mod computer {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use nomi_computer::tool::ComputerTool;
    use nomi_tools::Tool;
    use nomi_types::tool::ToolResult;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct ComputerObserve;

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct ComputerInput {
        pub action: String,
        pub parameters: serde_json::Value,
        pub expected_generation: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct ComputerLaunch {
        pub target: String,
        pub app: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum ComputerRoleOperation {
        Observe(ComputerObserve),
        Input(ComputerInput),
        Launch(ComputerLaunch),
    }

    impl ComputerRoleOperation {
        fn resource_operation(&self) -> &'static str {
            match self {
                Self::Observe(_) => "observe",
                Self::Input(_) => "input",
                Self::Launch(_) => "launch",
            }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct ComputerRoleResult {
        pub generation: u64,
        pub result: serde_json::Value,
    }

    fn tool_result_value(result: ToolResult) -> serde_json::Value {
        let mut output = serde_json::json!({ "text": result.content });
        if !result.images.is_empty() {
            output["images"] = serde_json::Value::Array(
                result
                    .images
                    .into_iter()
                    .map(|image| {
                        serde_json::json!({
                            "media_type": image.media_type,
                            "data": image.data,
                        })
                    })
                    .collect(),
            );
        }
        output
    }

    /// Direct first-party ComputerTool adapter. The single target is
    /// serialized because ComputerTool owns the latest screenshot and a11y
    /// snapshot/ref cache.
    pub(crate) struct ComputerRoleHost {
        tool: Arc<ComputerTool>,
        target_resource_id: String,
        expected_provider: ExactRoleProviderRef,
        expected_snapshot: ResolvedSnapshotRef,
        expected_registry_generation: u64,
        target_lock: Mutex<()>,
        observation_generation: AtomicU64,
    }

    impl ComputerRoleHost {
        pub(crate) fn new(
            tool: Arc<ComputerTool>,
            target_resource_id: String,
            expected_provider: ExactRoleProviderRef,
            expected_snapshot: ResolvedSnapshotRef,
            expected_registry_generation: u64,
        ) -> Self {
            Self {
                tool,
                target_resource_id,
                expected_provider,
                expected_snapshot,
                expected_registry_generation,
                target_lock: Mutex::new(()),
                observation_generation: AtomicU64::new(0),
            }
        }

        pub(crate) async fn invoke(
            &self,
            context: RoleHostContext,
            operation: ComputerRoleOperation,
        ) -> Result<ComputerRoleResult, RoleHostError> {
            if context.provider.role.key.role_id.as_ref() != COMPUTER_ROLE_ID {
                return Err(RoleHostError::InvalidContext("computer role id"));
            }
            context.validate_common(
                &self.expected_provider,
                &self.expected_snapshot,
                self.expected_registry_generation,
            )?;
            let binding = exact_resource(
                &context.resource_bindings,
                COMPUTER_RESOURCE_KIND,
                &context.principal,
                operation.resource_operation(),
            )?;
            if binding.resource_id.as_ref() != self.target_resource_id {
                return Err(RoleHostError::ResourceIdentityMismatch);
            }

            let _guard = self.target_lock.lock().await;
            match operation {
                ComputerRoleOperation::Observe(_) => {
                    let result = self.tool.execute(serde_json::json!({ "action": "observe" })).await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    let generation = self
                        .observation_generation
                        .fetch_add(1, Ordering::AcqRel)
                        .saturating_add(1);
                    Ok(ComputerRoleResult {
                        generation,
                        result: tool_result_value(result),
                    })
                }
                ComputerRoleOperation::Input(request) => {
                    if request.action.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("computer action"));
                    }
                    let current = self.observation_generation.load(Ordering::Acquire);
                    if current == 0 || request.expected_generation != current {
                        return Err(RoleHostError::StaleObservationGeneration);
                    }
                    let mut input = require_object(request.parameters)?;
                    reject_computer_control_fields(&input)?;
                    if !matches!(
                        request.action.as_str(),
                        "click_element"
                            | "right_click_element"
                            | "double_click_element"
                            | "set_element_value"
                            | "left_click"
                            | "right_click"
                            | "middle_click"
                            | "double_click"
                            | "triple_click"
                            | "mouse_move"
                            | "left_click_drag"
                            | "type"
                            | "key"
                            | "scroll"
                            | "focus_window"
                            | "wait"
                    ) {
                        return Err(RoleHostError::InvalidContext(
                            "unsupported computer.input action",
                        ));
                    }
                    input.insert(
                        "action".to_owned(),
                        serde_json::Value::String(request.action),
                    );
                    let result = self
                        .tool
                        .execute(serde_json::Value::Object(input))
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    let generation = self
                        .observation_generation
                        .fetch_add(1, Ordering::AcqRel)
                        .saturating_add(1);
                    Ok(ComputerRoleResult {
                        generation,
                        result: tool_result_value(result),
                    })
                }
                ComputerRoleOperation::Launch(request) => {
                    if request.target.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("computer launch target"));
                    }
                    let result = self
                        .tool
                        .execute(serde_json::json!({
                            "action": "launch",
                            "target": request.target,
                            "app": request.app,
                        }))
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    let generation = self
                        .observation_generation
                        .fetch_add(1, Ordering::AcqRel)
                        .saturating_add(1);
                    Ok(ComputerRoleResult {
                        generation,
                        result: tool_result_value(result),
                    })
                }
            }
        }
    }
}

#[cfg(feature = "computer-use")]
#[allow(unused_imports)]
pub(crate) use computer::{
    ComputerInput, ComputerLaunch, ComputerObserve, ComputerRoleHost, ComputerRoleOperation,
    ComputerRoleResult,
};

#[cfg(feature = "computer-use")]
pub(crate) struct ComputerRoleInvoker {
    tool: Arc<nomi_computer::tool::ComputerTool>,
    hosts: tokio::sync::Mutex<std::collections::HashMap<String, Arc<ComputerRoleHost>>>,
}

#[cfg(feature = "computer-use")]
impl ComputerRoleInvoker {
    pub(crate) fn new(tool: Arc<nomi_computer::tool::ComputerTool>) -> Self {
        Self {
            tool,
            hosts: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    async fn host_for(
        &self,
        agent_session_id: &nomifun_agent_contracts::AgentSessionId,
        snapshot: &ResolvedSnapshotRef,
        registry_generation: u64,
        provider: ExactRoleProviderRef,
        resource_id: String,
    ) -> Arc<ComputerRoleHost> {
        let key = format!(
            "{}:{}:{}",
            agent_session_id.as_ref(),
            snapshot.snapshot_digest.as_ref(),
            resource_id
        );
        let mut hosts = self.hosts.lock().await;
        hosts
            .entry(key)
            .or_insert_with(|| {
                Arc::new(ComputerRoleHost::new(
                    Arc::clone(&self.tool),
                    resource_id,
                    provider,
                    snapshot.clone(),
                    registry_generation,
                ))
            })
            .clone()
    }
}

#[cfg(feature = "computer-use")]
#[async_trait::async_trait]
impl RoleHostInvoker for ComputerRoleInvoker {
    async fn invoke(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, RoleHostError> {
        let provider = request
            .context
            .role_provider
            .as_ref()
            .cloned()
            .ok_or(RoleHostError::ProviderUnavailable)?;
        let resource = exact_resource(
            &request.context.resource_bindings,
            COMPUTER_RESOURCE_KIND,
            &request.context.principal,
            match &request.operation {
                Wave2TypedCapabilityOperation::ComputerInput { .. } => "input",
                Wave2TypedCapabilityOperation::ComputerLaunch { .. } => "launch",
                _ => "observe",
            },
        )?;
        let host = self
            .host_for(
                &request.context.agent_session_id,
                &request.context.resolved_snapshot_ref,
                request.context.registry_generation,
                provider.clone(),
                resource.resource_id.as_ref().to_owned(),
            )
            .await;
        let operation = match request.operation {
            Wave2TypedCapabilityOperation::ComputerInput { input } => {
                let mut object = require_object(input.0)?;
                let action = object
                    .remove("action")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("computer action"))?;
                let expected_generation = object
                    .remove("expected_generation")
                    .and_then(|value| value.as_u64())
                    .ok_or(RoleHostError::InvalidContext("expected_generation"))?;
                ComputerRoleOperation::Input(ComputerInput {
                    action,
                    parameters: serde_json::Value::Object(object),
                    expected_generation,
                })
            }
            Wave2TypedCapabilityOperation::ComputerLaunch { input } => {
                let mut object = require_object(input.0)?;
                let target = object
                    .remove("target")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("computer launch target"))?;
                let app = match object.remove("app") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(app)) => Some(app),
                    Some(_) => {
                        return Err(RoleHostError::InvalidContext(
                            "computer launch app",
                        ));
                    }
                };
                if !object.is_empty() {
                    return Err(RoleHostError::InvalidContext(
                        "unsupported computer.launch field",
                    ));
                }
                ComputerRoleOperation::Launch(ComputerLaunch { target, app })
            }
            _ => {
                return Err(RoleHostError::InvalidContext(
                    "operation is not a Computer action member",
                ));
            }
        };
        let session_id = request.context.agent_session_id.as_ref();
        let context = RoleHostContext {
            principal: request.context.principal,
            runtime_instance_id: format!("agent-session:{session_id}"),
            owner_lease_id: format!("agent-session:{session_id}"),
            snapshot: request.context.resolved_snapshot_ref,
            registry_generation: request.context.registry_generation,
            provider,
            resource_bindings: request.context.resource_bindings,
        };
        let result = host.invoke(context, operation).await?;
        Ok(StrictJsonValue(serde_json::json!({
            "generation": result.generation,
            "result": result.result
        })))
    }
}

#[cfg(feature = "computer-use")]
impl Wave2ContextHostPort for ComputerRoleInvoker {
    fn contribute<'a>(
        &'a self,
        request: Wave2ContextHostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<ContextContributionResult, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let context = request.context;
            if context.role_provider.role.key.role_id.as_ref() != COMPUTER_ROLE_ID {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_INVALID_CONTEXT",
                    "computer context request has the wrong role",
                ));
            }
            if !matches!(
                request.operation,
                Wave2ContextCapabilityOperation::ComputerObserve
                    | Wave2ContextCapabilityOperation::A11yObserve
            ) {
                return Err(Wave2HostPortError::new(
                    "ROLE_HOST_INVALID_CONTEXT",
                    "computer provider received an unsupported context member",
                ));
            }
            let binding = exact_resource(
                &context.resource_bindings,
                COMPUTER_RESOURCE_KIND,
                &context.principal,
                "observe",
            )
            .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            let host = self
                .host_for(
                    &context.agent_session_id,
                    &context.resolved_snapshot_ref,
                    context.registry_generation,
                    context.role_provider.clone(),
                    binding.resource_id.as_ref().to_owned(),
                )
                .await;
            let role_context = RoleHostContext {
                principal: context.principal,
                runtime_instance_id: format!(
                    "agent-session:{}",
                    context.agent_session_id.as_ref()
                ),
                owner_lease_id: format!("agent-session:{}", context.agent_session_id.as_ref()),
                snapshot: context.resolved_snapshot_ref,
                registry_generation: context.registry_generation,
                provider: context.role_provider,
                resource_bindings: context.resource_bindings,
            };
            let result = host
                .invoke(role_context, ComputerRoleOperation::Observe(ComputerObserve))
                .await
                .map_err(|error| Wave2HostPortError::new(error.code(), error.to_string()))?;
            Ok(ContextContributionResult {
                value: Some(StrictJsonValue(serde_json::json!({
                    "generation": result.generation,
                    "result": result.result
                }))),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        DigestHex, PackageId, PackageRef, PluginMountId, RoleContractKey, VersionString,
    };

    fn provider(role_id: &str) -> ExactRoleProviderRef {
        ExactRoleProviderRef {
            role: nomifun_agent_contracts::ExactRoleContractRef {
                key: RoleContractKey {
                    role_id: role_id.into(),
                    contract_version: VersionString::from("1.0.0"),
                },
                contract_digest: DigestHex::from("a".repeat(64)),
            },
            package: PackageRef {
                id: PackageId::from("test.package"),
                version: VersionString::from("1.0.0"),
            },
            mount_id: PluginMountId::from("test-mount"),
            contribution_digest: DigestHex::from("b".repeat(64)),
        }
    }

    fn context(
        role_id: &str,
        bindings: TypedResourceBindings,
    ) -> RoleHostContext {
        RoleHostContext {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "owner".to_owned(),
            },
            runtime_instance_id: "runtime".to_owned(),
            owner_lease_id: "lease".to_owned(),
            snapshot: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "c".repeat(64).into(),
            },
            registry_generation: 7,
            provider: provider(role_id),
            resource_bindings: bindings,
        }
    }

    fn binding(kind: &str, operations: &[&str]) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: "binding".into(),
            resource_kind: kind.into(),
            resource_id: "resource".into(),
            owner_id: "owner".to_owned(),
            operations: operations.iter().map(|value| (*value).to_owned()).collect(),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        }
    }

    #[test]
    fn exact_resource_requires_one_owned_granted_binding() {
        let context = context(
            BROWSER_ROLE_ID,
            vec![binding(BROWSER_RESOURCE_KIND, &["navigate"])],
        );
        let resource = exact_resource(
            &context.resource_bindings,
            BROWSER_RESOURCE_KIND,
            &context.principal,
            "navigate",
        )
        .expect("exact resource");
        assert_eq!(resource.owner_id, "owner");
        assert_eq!(resource.resource_id.as_ref(), "resource");

        assert_eq!(
            exact_resource(
                &context.resource_bindings,
                BROWSER_RESOURCE_KIND,
                &context.principal,
                "interact",
            )
            .unwrap_err()
            .code(),
            "ROLE_HOST_RESOURCE_OPERATION_DENIED"
        );
        assert_eq!(
            exact_resource(
                &[
                    binding(BROWSER_RESOURCE_KIND, &["navigate"]),
                    binding(BROWSER_RESOURCE_KIND, &["navigate"]),
                ],
                BROWSER_RESOURCE_KIND,
                &context.principal,
                "navigate",
            )
            .unwrap_err()
            .code(),
            "ROLE_HOST_RESOURCE_CARDINALITY"
        );
    }

    #[test]
    fn context_rejects_provider_and_generation_drift() {
        let expected = context(BROWSER_ROLE_ID, Vec::new());
        let mut drifted = expected.clone();
        drifted.registry_generation = 8;
        assert_eq!(
            drifted
                .validate_common(&expected.provider, &expected.snapshot, 7)
                .unwrap_err()
                .code(),
            "ROLE_HOST_REGISTRY_GENERATION_MISMATCH"
        );
        drifted.registry_generation = 7;
        drifted.provider.mount_id = "other-mount".into();
        assert_eq!(
            drifted
                .validate_common(&expected.provider, &expected.snapshot, 7)
                .unwrap_err()
                .code(),
            "ROLE_HOST_PROVIDER_MISMATCH"
        );
    }

    #[test]
    fn model_input_cannot_supply_role_routing_or_generation_fields() {
        let browser = require_object(serde_json::json!({
            "text": "hello",
            "lane_id": "model-selected",
        }))
        .unwrap();
        assert_eq!(
            reject_browser_control_fields(&browser).unwrap_err().code(),
            "ROLE_HOST_INVALID_CONTEXT"
        );

        let computer = require_object(serde_json::json!({
            "ref": 1,
            "expected_generation": 99,
        }))
        .unwrap();
        assert_eq!(
            reject_computer_control_fields(&computer)
                .unwrap_err()
                .code(),
            "ROLE_HOST_INVALID_CONTEXT"
        );
    }
}
