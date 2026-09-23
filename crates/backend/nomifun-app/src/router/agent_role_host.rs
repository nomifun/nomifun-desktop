//! Typed first-party Computer role-host adapters.
//!
//! Conversation Browser uses its native Workspace owner. An unbound v2
//! provider fails closed.

use std::fmt;
#[cfg(feature = "computer-use")]
use std::sync::Arc;

use nomifun_agent_contracts::{
    ExactRoleProviderRef, PrincipalRef, ResolvedSnapshotRef, ResourceKind, TypedResourceBinding,
    TypedResourceBindings,
};
#[cfg(feature = "computer-use")]
use nomifun_agent_contracts::StrictJsonValue;
#[cfg(feature = "computer-use")]
use nomifun_agent_domain_wave2::{
    Wave2HostPort, Wave2HostPortError, Wave2HostRequest, Wave2TypedCapabilityOperation,
    Wave2TypedHostRequest,
};
pub(crate) const COMPUTER_ROLE_ID: &str = "system.computer_use";
pub(crate) const COMPUTER_RESOURCE_KIND: &str = "computer";

/// Select the exact bundled native Computer provider for this desktop boot.
/// OS permission readiness is a separate live Resource fact and may narrow
/// execution, but it must not remove the provider contract from compilation.
#[cfg(feature = "computer-use")]
pub(crate) fn installation_binding(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
) -> anyhow::Result<
    std::collections::BTreeMap<
        nomifun_agent_contracts::ExecutionRoleId,
        nomifun_agent_contracts::InstallationRoleBinding,
    >,
> {
    use nomifun_agent_contracts::{
        InstallationRoleBinding, PluginSourceKind, RoleProviderSelection,
    };
    let role_id = nomifun_agent_domain_wave2::COMPUTER_EXECUTION_ROLE_ID.into();
    let mount_id = nomifun_agent_domain_wave2::COMPUTER_A11Y_MOUNT_ID.into();
    let installed = registry.role_provider(&role_id, &mount_id).ok_or_else(|| {
        anyhow::anyhow!("Native Computer Role Provider is missing from the host registry")
    })?;
    anyhow::ensure!(
        installed.provider.role.key.role_id == role_id,
        "Native Computer Provider has the wrong Role contract"
    );
    anyhow::ensure!(
        installed.source.source_kind == PluginSourceKind::Bundled,
        "Native Computer Provider must be bundled"
    );
    Ok(std::collections::BTreeMap::from([(
        role_id,
        InstallationRoleBinding {
            selection: RoleProviderSelection {
                role: installed.provider.role.clone(),
                provider_mount_id: installed.provider.mount_id.clone(),
            },
            binding_version: 1,
            updated_at_ms: 0,
        },
    )]))
}

#[async_trait::async_trait]
#[cfg(feature = "computer-use")]
pub(crate) trait RoleHostInvoker: Send + Sync {
    async fn invoke(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, RoleHostError>;
}

/// Fixed provider adapter mounted into one Wave2 registration. Provider
/// selection remains in KernelRegistry; this type only translates the
/// compatibility envelope after that selection has already happened.
#[cfg(feature = "computer-use")]
pub(crate) struct RoleHostPortAdapter {
    invoker: Arc<dyn RoleHostInvoker>,
}

#[cfg(feature = "computer-use")]
impl RoleHostPortAdapter {
    pub(crate) fn new(invoker: Arc<dyn RoleHostInvoker>) -> Self {
        Self { invoker }
    }
}

#[cfg(feature = "computer-use")]
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

#[cfg(feature = "computer-use")]
mod computer {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use nomi_computer::tool::ComputerTool;
    use nomi_types::tool::ToolResult;

    #[async_trait::async_trait]
    pub(crate) trait ComputerToolPort: Send + Sync {
        async fn execute_authorized(
            &self,
            action_id: &str,
            input: serde_json::Value,
        ) -> ToolResult;
    }

    #[async_trait::async_trait]
    impl ComputerToolPort for ComputerTool {
        async fn execute_authorized(
            &self,
            action_id: &str,
            input: serde_json::Value,
        ) -> ToolResult {
            ComputerTool::execute_authorized(self, action_id, input).await
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct ComputerObserve {
        pub action_id: &'static str,
        pub input: serde_json::Value,
    }

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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ComputerObservationKind {
        Screenshot,
        Accessibility,
        WindowList,
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
        tool: Arc<dyn ComputerToolPort>,
        target_resource_id: String,
        expected_provider: ExactRoleProviderRef,
        expected_snapshot: ResolvedSnapshotRef,
        expected_registry_generation: u64,
        target_lock: Mutex<()>,
        observation_generation: AtomicU64,
        observation_kind: std::sync::Mutex<Option<ComputerObservationKind>>,
    }

    impl ComputerRoleHost {
        pub(crate) fn new(
            tool: Arc<ComputerTool>,
            target_resource_id: String,
            expected_provider: ExactRoleProviderRef,
            expected_snapshot: ResolvedSnapshotRef,
            expected_registry_generation: u64,
        ) -> Self {
            Self::new_with_executor(
                tool,
                target_resource_id,
                expected_provider,
                expected_snapshot,
                expected_registry_generation,
            )
        }

        pub(crate) fn new_with_executor(
            tool: Arc<dyn ComputerToolPort>,
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
                observation_kind: std::sync::Mutex::new(None),
            }
        }

        fn record_observation(&self, kind: ComputerObservationKind) -> u64 {
            let generation = self
                .observation_generation
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1);
            *self
                .observation_kind
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(kind);
            generation
        }

        fn invalidate_observation(&self) {
            self.observation_generation.store(0, Ordering::Release);
            *self
                .observation_kind
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }

        fn validate_input_observation(
            &self,
            action: &str,
            parameters: &serde_json::Map<String, serde_json::Value>,
            expected_generation: u64,
        ) -> Result<(), RoleHostError> {
            let current = self.observation_generation.load(Ordering::Acquire);
            let kind = *self
                .observation_kind
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if current == 0 || expected_generation != current {
                return Err(RoleHostError::StaleObservationGeneration);
            }
            let compatible = match action {
                "click_element" | "right_click_element" | "double_click_element"
                | "set_element_value" => kind == Some(ComputerObservationKind::Accessibility),
                "left_click" | "right_click" | "middle_click" | "double_click"
                | "triple_click" | "mouse_move" | "left_click_drag" => {
                    kind == Some(ComputerObservationKind::Screenshot)
                }
                "focus_window" => kind == Some(ComputerObservationKind::WindowList),
                "scroll" if parameters.contains_key("x") || parameters.contains_key("y") => {
                    kind == Some(ComputerObservationKind::Screenshot)
                }
                "type" | "key" | "scroll" => kind.is_some(),
                _ => false,
            };
            compatible
                .then_some(())
                .ok_or(RoleHostError::StaleObservationGeneration)
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
                ComputerRoleOperation::Observe(request) => {
                    let input = require_object(request.input)?;
                    reject_computer_control_fields(&input)?;
                    let native_action = input
                        .get("action")
                        .and_then(serde_json::Value::as_str)
                        .ok_or(RoleHostError::InvalidContext("computer observation action"))?;
                    let observation_kind = match (request.action_id, native_action) {
                        (nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID, "observe") => {
                            Some(ComputerObservationKind::Accessibility)
                        }
                        (nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID, "screenshot") => {
                            Some(ComputerObservationKind::Screenshot)
                        }
                        (nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID, "list_windows") => {
                            Some(ComputerObservationKind::WindowList)
                        }
                        (nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID, "cursor_position") => None,
                        (nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID, "wait") => {
                            self.invalidate_observation();
                            None
                        }
                        _ => return Err(RoleHostError::InvalidContext("computer observation action")),
                    };
                    let result = self
                        .tool
                        .execute_authorized(
                            request.action_id,
                            serde_json::Value::Object(input),
                        )
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    let generation = observation_kind
                        .map(|kind| self.record_observation(kind))
                        .unwrap_or(0);
                    Ok(ComputerRoleResult {
                        generation,
                        result: tool_result_value(result),
                    })
                }
                ComputerRoleOperation::Input(request) => {
                    if request.action.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("computer action"));
                    }
                    let mut input = require_object(request.parameters)?;
                    reject_computer_control_fields(&input)?;
                    self.validate_input_observation(
                        &request.action,
                        &input,
                        request.expected_generation,
                    )?;
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
                    ) {
                        return Err(RoleHostError::InvalidContext(
                            "unsupported computer/input native operation",
                        ));
                    }
                    input.insert(
                        "action".to_owned(),
                        serde_json::Value::String(request.action),
                    );
                    // Input may partially change the desktop even if the OS
                    // reports an error. Re-observe before any subsequent
                    // action; never mint authority from an effect result.
                    self.invalidate_observation();
                    let result = self
                        .tool
                        .execute_authorized(
                            nomi_computer::capability::COMPUTER_INPUT_ACTION_ID,
                            serde_json::Value::Object(input),
                        )
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    Ok(ComputerRoleResult {
                        generation: 0,
                        result: tool_result_value(result),
                    })
                }
                ComputerRoleOperation::Launch(request) => {
                    if request.target.trim().is_empty() {
                        return Err(RoleHostError::InvalidContext("computer launch target"));
                    }
                    self.invalidate_observation();
                    let result = self
                        .tool
                        .execute_authorized(
                            nomi_computer::capability::COMPUTER_LAUNCH_ACTION_ID,
                            serde_json::json!({
                                "action": "launch",
                                "target": request.target,
                                "app": request.app,
                            }),
                        )
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    Ok(ComputerRoleResult {
                        generation: 0,
                        result: tool_result_value(result),
                    })
                }
            }
        }
    }
}

#[cfg(feature = "computer-use")]
pub(crate) use computer::{
    ComputerInput, ComputerLaunch, ComputerObserve, ComputerRoleHost, ComputerRoleOperation,
};

#[cfg(all(test, feature = "computer-use"))]
use computer::ComputerToolPort;

#[cfg(feature = "computer-use")]
pub(crate) struct ComputerRoleInvoker {
    tool: Arc<nomi_computer::tool::ComputerTool>,
    effect_store: nomifun_agent_session::AgentSessionStore,
    hosts: tokio::sync::Mutex<std::collections::HashMap<String, Arc<ComputerRoleHost>>>,
}

#[cfg(feature = "computer-use")]
impl ComputerRoleInvoker {
    pub(crate) fn new(
        tool: Arc<nomi_computer::tool::ComputerTool>,
        effect_store: nomifun_agent_session::AgentSessionStore,
    ) -> Self {
        Self {
            tool,
            effect_store,
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
        let effect_input = match &request.operation {
            Wave2TypedCapabilityOperation::ComputerInput { input }
            | Wave2TypedCapabilityOperation::ComputerLaunch { input } => Some(input.clone()),
            _ => None,
        };
        let effect_strategy = match &request.operation {
            Wave2TypedCapabilityOperation::ComputerInput { .. } => Some(
                nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
            ),
            Wave2TypedCapabilityOperation::ComputerLaunch { .. } => {
                Some(nomifun_agent_session::EffectStrategy::ManagedEffect)
            }
            _ => None,
        };
        let effect_context = request.context.clone();
        let effect_binding = resource.clone();
        let operation = match request.operation {
            Wave2TypedCapabilityOperation::ComputerObserve { input } => {
                ComputerRoleOperation::Observe(ComputerObserve {
                    action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                    input: input.0,
                })
            }
            Wave2TypedCapabilityOperation::ComputerA11yObserve { input } => {
                ComputerRoleOperation::Observe(ComputerObserve {
                    action_id: nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID,
                    input: input.0,
                })
            }
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
                if object
                    .remove("action")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .as_deref()
                    != Some("launch")
                {
                    return Err(RoleHostError::InvalidContext(
                        "computer launch action",
                    ));
                }
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
                            "unsupported computer/launch field",
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
        let reservation = match (effect_input.as_ref(), effect_strategy) {
            (Some(input), Some(strategy)) => match super::agent_wave2_host::begin_wave2_exclusive_effect(
                &self.effect_store,
                &effect_context,
                &effect_binding,
                input,
                strategy,
            )
            .await
            .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?
            {
                super::agent_wave2_host::Wave2EffectAdmission::Replay(result) => return Ok(result),
                super::agent_wave2_host::Wave2EffectAdmission::Reserved(reservation) => {
                    Some((reservation, strategy))
                }
            },
            _ => None,
        };
        let result = match host.invoke(context, operation).await {
            Ok(result) => result,
            Err(error) => {
                if let Some((reservation, strategy)) = reservation.as_ref() {
                    let effect_error = Wave2HostPortError::new(error.code(), error.to_string());
                    let completion = if strategy.is_external_uncertain()
                        && matches!(error, RoleHostError::ProviderFailure(_))
                    {
                        super::agent_wave2_host::Wave2EffectCompletion::Uncertain(&effect_error)
                    } else {
                        super::agent_wave2_host::Wave2EffectCompletion::Failed(&effect_error)
                    };
                    super::agent_wave2_host::finish_wave2_effect(reservation, completion)
                        .await
                        .map_err(|terminal| RoleHostError::ProviderFailure(terminal.to_string()))?;
                }
                return Err(error);
            }
        };
        let output = StrictJsonValue(serde_json::json!({
            "generation": result.generation,
            "result": result.result
        }));
        if let Some((reservation, _)) = reservation.as_ref() {
            super::agent_wave2_host::finish_wave2_effect(
                reservation,
                super::agent_wave2_host::Wave2EffectCompletion::Succeeded(&output),
            )
            .await
            .map_err(|error| RoleHostError::ProviderFailure(error.to_string()))?;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentModuleId, DigestHex, PackageId, PackageRef, RoleContractKey, VersionString,
    };

    #[cfg(feature = "computer-use")]
    #[derive(Default)]
    struct FakeComputerToolPort {
        active: std::sync::atomic::AtomicUsize,
        max_active: std::sync::atomic::AtomicUsize,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[cfg(feature = "computer-use")]
    #[async_trait::async_trait]
    impl ComputerToolPort for FakeComputerToolPort {
        async fn execute_authorized(
            &self,
            _action_id: &str,
            input: serde_json::Value,
        ) -> nomi_types::tool::ToolResult {
            use std::sync::atomic::Ordering;

            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            nomi_types::tool::ToolResult::text(
                input
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
            )
        }
    }

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
            mount_id: AgentModuleId::from("test-mount"),
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

    #[cfg(feature = "computer-use")]
    fn computer_host(
        tool: Arc<dyn ComputerToolPort>,
    ) -> (Arc<ComputerRoleHost>, RoleHostContext) {
        let context = context(
            COMPUTER_ROLE_ID,
            vec![binding(
                COMPUTER_RESOURCE_KIND,
                &["observe", "input", "launch"],
            )],
        );
        let host = Arc::new(ComputerRoleHost::new_with_executor(
            tool,
            "resource".to_owned(),
            context.provider.clone(),
            context.snapshot.clone(),
            context.registry_generation,
        ));
        (host, context)
    }

    #[cfg(feature = "computer-use")]
    fn computer_observe() -> ComputerObserve {
        ComputerObserve {
            action_id: nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID,
            input: serde_json::json!({"action":"observe"}),
        }
    }

    #[test]
    fn exact_resource_requires_one_owned_granted_binding() {
        let context = context(
            COMPUTER_ROLE_ID,
            vec![binding(COMPUTER_RESOURCE_KIND, &["navigate"])],
        );
        let resource = exact_resource(
            &context.resource_bindings,
            COMPUTER_RESOURCE_KIND,
            &context.principal,
            "navigate",
        )
        .expect("exact resource");
        assert_eq!(resource.owner_id, "owner");
        assert_eq!(resource.resource_id.as_ref(), "resource");

        assert_eq!(
            exact_resource(
                &context.resource_bindings,
                COMPUTER_RESOURCE_KIND,
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
                    binding(COMPUTER_RESOURCE_KIND, &["navigate"]),
                    binding(COMPUTER_RESOURCE_KIND, &["navigate"]),
                ],
                COMPUTER_RESOURCE_KIND,
                &context.principal,
                "navigate",
            )
            .unwrap_err()
            .code(),
            "ROLE_HOST_RESOURCE_CARDINALITY"
        );
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn computer_observations_are_serialized_and_monotonic() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        let start = Arc::new(tokio::sync::Barrier::new(3));

        let first = {
            let host = Arc::clone(&host);
            let context = context.clone();
            let start = Arc::clone(&start);
            tokio::spawn(async move {
                start.wait().await;
                host.invoke(context, ComputerRoleOperation::Observe(computer_observe()))
                    .await
                    .unwrap()
                    .generation
            })
        };
        let second = {
            let host = Arc::clone(&host);
            let context = context.clone();
            let start = Arc::clone(&start);
            tokio::spawn(async move {
                start.wait().await;
                host.invoke(context, ComputerRoleOperation::Observe(computer_observe()))
                    .await
                    .unwrap()
                    .generation
            })
        };

        start.wait().await;
        let mut generations = vec![first.await.unwrap(), second.await.unwrap()];
        generations.sort_unstable();
        assert_eq!(generations, vec![1, 2]);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
        assert_eq!(tool.max_active.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_input_rejects_stale_observation_generation_before_provider_call() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        let first = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let current = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        assert_eq!((first.generation, current.generation), (1, 2));

        let error = host
            .invoke(
                context,
                ComputerRoleOperation::Input(ComputerInput {
                    action: "wait".to_owned(),
                    parameters: serde_json::json!({ "seconds": 0 }),
                    expected_generation: first.generation,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error, RoleHostError::StaleObservationGeneration);
        assert_eq!(error.code(), "ROLE_HOST_STALE_OBSERVATION_GENERATION");
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_effect_invalidates_observation_and_never_mints_a_replacement_token() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        let observed = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let acted = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 1 }),
                    expected_generation: observed.generation,
                }),
            )
            .await
            .unwrap();
        assert_eq!(acted.generation, 0);

        let error = host
            .invoke(
                context,
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 1 }),
                    expected_generation: observed.generation,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error, RoleHostError::StaleObservationGeneration);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_observation_tokens_are_native_surface_specific() {
        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool);
        let screenshot = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(ComputerObserve {
                    action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                    input: serde_json::json!({ "action": "screenshot" }),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            host.invoke(
                context.clone(),
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 1 }),
                    expected_generation: screenshot.generation,
                }),
            )
            .await
            .unwrap_err(),
            RoleHostError::StaleObservationGeneration,
        );
        let waited = host
            .invoke(
                context,
                ComputerRoleOperation::Observe(ComputerObserve {
                    action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                    input: serde_json::json!({ "action": "wait", "seconds": 0 }),
                }),
            )
            .await
            .unwrap();
        assert_eq!(waited.generation, 0);
    }

    #[test]
    fn context_rejects_provider_and_generation_drift() {
        let expected = context(COMPUTER_ROLE_ID, Vec::new());
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
