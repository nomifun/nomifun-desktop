//! Typed first-party Computer role-host adapters.
//!
//! Conversation Browser uses its native Workspace owner. An unbound v2
//! provider fails closed.

use std::fmt;
#[cfg(feature = "computer-use")]
use std::sync::Arc;

#[cfg(feature = "computer-use")]
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_contracts::{
    ExactRoleProviderRef, PrincipalRef, ResolvedSnapshotRef, ResourceKind, TypedResourceBinding,
    TypedResourceBindings,
};
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
            dyn std::future::Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                + Send
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
    UnsupportedInput(String),
    #[cfg(windows)]
    InputNotApplied(String),
    #[cfg(feature = "computer-use")]
    PermissionRequired(nomi_computer::permissions::MissingComputerPermissions),
    ProviderUnavailable,
    ProviderFailure(String),
    EffectSettlementFailure(String),
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
            Self::UnsupportedInput(_) => "ROLE_HOST_UNSUPPORTED_INPUT",
            #[cfg(windows)]
            Self::InputNotApplied(_) => "ROLE_HOST_INPUT_NOT_APPLIED",
            #[cfg(feature = "computer-use")]
            Self::PermissionRequired(_) => "SYSTEM_PERMISSION_REQUIRED",
            Self::ProviderUnavailable => "ROLE_HOST_PROVIDER_UNAVAILABLE",
            Self::ProviderFailure(_) => "ROLE_HOST_PROVIDER_FAILURE",
            Self::EffectSettlementFailure(_) => "CAPABILITY_UNAVAILABLE",
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
            Self::UnsupportedInput(message) => {
                write!(
                    formatter,
                    "computer input was rejected before native effects: {message}"
                )
            }
            #[cfg(windows)]
            Self::InputNotApplied(message) => write!(formatter, "computer input was not executed: {message}"),
            #[cfg(feature = "computer-use")]
            Self::PermissionRequired(missing) => {
                write!(formatter, "{}", serde_json::json!({
                    "schema": "system-permission-required.v1",
                    "capability": "computer",
                    "action_id": missing.action_id,
                    "operation": missing.operation,
                    "permissions": missing.permissions,
                    "execution": "not_executed",
                }))
            }
            Self::ProviderUnavailable => formatter.write_str("role provider is unavailable"),
            Self::ProviderFailure(message) => write!(formatter, "role provider failed: {message}"),
            Self::EffectSettlementFailure(message) => {
                write!(formatter, "role effect settlement failed: {message}")
            }
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
        .ok_or(RoleHostError::InvalidContext(
            "operation input must be an object",
        ))
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
    #[cfg(test)]
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use nomi_computer::tool::ComputerTool;
    use nomi_types::tool::ToolResult;

    #[async_trait::async_trait]
    pub(crate) trait ComputerToolPort: Send + Sync {
        fn clear_cached_observation(&self);

        /// Fixtures supply explicit unknown state; native hosts probe current
        /// grants only when an authorized invocation reaches this boundary.
        fn permission_status(&self) -> nomi_computer::permissions::PermissionStatus {
            nomi_computer::permissions::PermissionStatus {
                accessibility: None,
                screen_recording: None,
            }
        }

        async fn execute_authorized(&self, action_id: &str, input: serde_json::Value)
            -> ToolResult;
    }

    #[async_trait::async_trait]
    impl ComputerToolPort for ComputerTool {
        fn clear_cached_observation(&self) {
            ComputerTool::clear_cached_observation(self);
        }

        fn permission_status(&self) -> nomi_computer::permissions::PermissionStatus {
            nomi_computer::permissions::permission_status()
        }

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
        pub(super) fn action_id(&self) -> &'static str {
            match self {
                Self::Observe(request) => request.action_id,
                Self::Input(_) => nomi_computer::capability::COMPUTER_INPUT_ACTION_ID,
                Self::Launch(_) => nomi_computer::capability::COMPUTER_LAUNCH_ACTION_ID,
            }
        }

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

    /// One physical ComputerTool has one lock and one observation authority,
    /// shared by every Session Host that borrows its native cache.
    pub(super) struct ComputerTargetOwner {
        tool: Arc<dyn ComputerToolPort>,
        state: Mutex<ComputerTargetState>,
        // Explicit fixture-only call sequences. Production always reads the
        // exact committed canonical call and never falls back to this clock.
        #[cfg(test)]
        fixture_call_seq: AtomicU64,
    }

    impl ComputerTargetOwner {
        pub(super) fn new(tool: Arc<dyn ComputerToolPort>) -> Self {
            Self {
                tool,
                state: Mutex::new(ComputerTargetState::default()),
                #[cfg(test)]
                fixture_call_seq: AtomicU64::new(0),
            }
        }
    }

    #[derive(Default)]
    struct ComputerTargetState {
        observation: Option<ComputerObservationLease>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ComputerExecutionIdentity {
        session_id: nomifun_agent_contracts::AgentSessionId,
        turn_id: nomifun_agent_contracts::OperationId,
        execution_generation: u64,
        execution_fence: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ComputerObservationScope {
        execution: ComputerExecutionIdentity,
        context: RoleHostContext,
        resource: TypedResourceBinding,
    }

    struct ComputerObservationLease {
        generation: u64,
        scope: ComputerObservationScope,
        kind: ComputerObservationKind,
    }

    /// Trusted Kernel invocation identity, separate from model operation JSON.
    /// The Store resolves it while holding the physical target lock.
    pub(super) struct ComputerInvocationSource {
        pub session_id: nomifun_agent_contracts::AgentSessionId,
        pub turn_id: nomifun_agent_contracts::OperationId,
        pub operation_id: nomifun_agent_contracts::OperationId,
        pub capability_id: nomifun_agent_contracts::CapabilityId,
        pub action_id: nomifun_agent_contracts::ActionId,
    }

    enum ComputerCallAuthority<'a> {
        Canonical(
            &'a nomifun_agent_session::AgentSessionStore,
            ComputerInvocationSource,
        ),
        #[cfg(test)]
        Fixture,
    }

    pub(crate) struct ComputerRoleHost {
        target: Arc<ComputerTargetOwner>,
        session_id: nomifun_agent_contracts::AgentSessionId,
        target_resource_id: String,
        expected_provider: ExactRoleProviderRef,
        expected_snapshot: ResolvedSnapshotRef,
        expected_registry_generation: u64,
    }

    impl ComputerRoleHost {
        fn check_permission(&self, action_id: &str, operation: &str) -> Result<(), RoleHostError> {
            match nomi_computer::permissions::missing_permissions_for_operation(
                action_id,
                operation,
                self.target.tool.permission_status(),
            ) {
                Some(missing) => Err(RoleHostError::PermissionRequired(missing)),
                None => Ok(()),
            }
        }

        pub(super) fn new(
            target: Arc<ComputerTargetOwner>,
            session_id: nomifun_agent_contracts::AgentSessionId,
            target_resource_id: String,
            expected_provider: ExactRoleProviderRef,
            expected_snapshot: ResolvedSnapshotRef,
            expected_registry_generation: u64,
        ) -> Self {
            Self {
                target,
                session_id,
                target_resource_id,
                expected_provider,
                expected_snapshot,
                expected_registry_generation,
            }
        }

        #[cfg(test)]
        pub(super) fn new_with_executor(
            tool: Arc<dyn ComputerToolPort>,
            target_resource_id: String,
            expected_provider: ExactRoleProviderRef,
            expected_snapshot: ResolvedSnapshotRef,
            expected_registry_generation: u64,
        ) -> Self {
            Self::new(
                Arc::new(ComputerTargetOwner::new(tool)),
                uuid::Uuid::now_v7().to_string().into(),
                target_resource_id,
                expected_provider,
                expected_snapshot,
                expected_registry_generation,
            )
        }

        #[cfg(test)]
        pub(super) fn seed_fixture_call_seq_for_test(&self, generation: u64) {
            self.target
                .fixture_call_seq
                .store(generation, Ordering::Release);
        }

        #[cfg(test)]
        pub(super) fn fixture_call_seq_for_test(&self) -> u64 {
            self.target.fixture_call_seq.load(Ordering::Acquire)
        }

        fn validate_input_observation(
            lease: Option<&ComputerObservationLease>,
            scope: &ComputerObservationScope,
            action: &str,
            parameters: &serde_json::Map<String, serde_json::Value>,
            expected_generation: u64,
        ) -> Result<(), RoleHostError> {
            let lease = lease.ok_or(RoleHostError::StaleObservationGeneration)?;
            if expected_generation == 0
                || expected_generation != lease.generation
                || *scope != lease.scope
            {
                return Err(RoleHostError::StaleObservationGeneration);
            }
            let compatible = match action {
                "click_element"
                | "right_click_element"
                | "double_click_element"
                | "set_element_value" => lease.kind == ComputerObservationKind::Accessibility,
                "left_click" | "right_click" | "middle_click" | "double_click" | "triple_click"
                | "mouse_move" | "left_click_drag" => {
                    lease.kind == ComputerObservationKind::Screenshot
                }
                "focus_window" => lease.kind == ComputerObservationKind::WindowList,
                "scroll" if parameters.contains_key("x") || parameters.contains_key("y") => {
                    lease.kind == ComputerObservationKind::Screenshot
                }
                "type" | "key" | "scroll" => true,
                _ => false,
            };
            compatible
                .then_some(())
                .ok_or(RoleHostError::StaleObservationGeneration)
        }

        #[cfg(test)]
        pub(super) async fn invoke(
            &self,
            context: RoleHostContext,
            operation: ComputerRoleOperation,
        ) -> Result<ComputerRoleResult, RoleHostError> {
            self.invoke_with_authority(context, operation, ComputerCallAuthority::Fixture)
                .await
        }

        pub(super) async fn invoke_canonical(
            &self,
            context: RoleHostContext,
            operation: ComputerRoleOperation,
            store: &nomifun_agent_session::AgentSessionStore,
            source: ComputerInvocationSource,
        ) -> Result<ComputerRoleResult, RoleHostError> {
            self.invoke_with_authority(
                context,
                operation,
                ComputerCallAuthority::Canonical(store, source),
            )
            .await
        }

        async fn invoke_with_authority(
            &self,
            context: RoleHostContext,
            operation: ComputerRoleOperation,
            authority: ComputerCallAuthority<'_>,
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

            let mut target = self.target.state.lock().await;
            let (execution, call_seq) = match authority {
                ComputerCallAuthority::Canonical(store, source) => {
                    if source.session_id != self.session_id
                        || source.capability_id.as_ref() != "computer"
                        || source.action_id.as_ref() != operation.action_id()
                    {
                        return Err(RoleHostError::InvalidContext(
                            "computer canonical invocation identity",
                        ));
                    }
                    let cause = store
                        .native_tool_call_causation(
                            &context.principal,
                            &source.session_id,
                            &source.turn_id,
                            &source.operation_id,
                            &source.capability_id,
                            &source.action_id,
                        )
                        .await
                        .map_err(|error| {
                            RoleHostError::EffectSettlementFailure(error.to_string())
                        })?;
                    if cause.execution_scope.resolved_snapshot_ref() != &context.snapshot {
                        return Err(RoleHostError::SnapshotMismatch);
                    }
                    (
                        ComputerExecutionIdentity {
                            session_id: source.session_id,
                            turn_id: source.turn_id,
                            execution_generation: cause.execution_scope.generation(),
                            execution_fence: cause.execution_scope.fence(),
                        },
                        cause.tool_call.seq,
                    )
                }
                #[cfg(test)]
                ComputerCallAuthority::Fixture => {
                    let seq = self
                        .target
                        .fixture_call_seq
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                            current.checked_add(1)
                        })
                        .map_err(|_| {
                            target.observation = None;
                            self.target.tool.clear_cached_observation();
                            RoleHostError::StaleObservationGeneration
                        })?
                        .checked_add(1)
                        .ok_or(RoleHostError::StaleObservationGeneration)?;
                    (
                        ComputerExecutionIdentity {
                            session_id: self.session_id.clone(),
                            turn_id: "fixture-turn".into(),
                            execution_generation: 1,
                            execution_fence: 0,
                        },
                        seq,
                    )
                }
            };
            let scope = ComputerObservationScope {
                execution,
                context: context.clone(),
                resource: binding.clone(),
            };
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
                        (
                            nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                            "cursor_position",
                        ) => None,
                        (nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID, "wait") => None,
                        _ => {
                            return Err(RoleHostError::InvalidContext(
                                "computer observation action",
                            ))
                        }
                    };
                    // Once an authorized refresh starts, the previous native
                    // cache and lease cannot survive even a failed observation.
                    target.observation = None;
                    self.target.tool.clear_cached_observation();
                    self.check_permission(request.action_id, native_action)?;
                    let result = self
                        .target
                        .tool
                        .execute_authorized(request.action_id, serde_json::Value::Object(input))
                        .await;
                    if result.is_error {
                        return Err(RoleHostError::ProviderFailure(result.content));
                    }
                    let generation = match observation_kind {
                        Some(kind) => {
                            target.observation = Some(ComputerObservationLease {
                                generation: call_seq,
                                scope,
                                kind,
                            });
                            call_seq
                        }
                        None => 0,
                    };
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
                    Self::validate_input_observation(
                        target.observation.as_ref(),
                        &scope,
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
                    #[cfg(windows)]
                    let focus_request = request.action == "focus_window";
                    input.insert(
                        "action".to_owned(),
                        serde_json::Value::String(request.action),
                    );
                    // Input may partially change the desktop even if the OS
                    // reports an error. Re-observe before any subsequent
                    // action; never mint authority from an effect result.
                    target.observation = None;
                    if let Err(error) = self.check_permission(
                        nomi_computer::capability::COMPUTER_INPUT_ACTION_ID,
                        input["action"].as_str().expect("trusted action inserted above"),
                    ) {
                        self.target.tool.clear_cached_observation();
                        return Err(error);
                    }
                    let result = self
                        .target
                        .tool
                        .execute_authorized(
                            nomi_computer::capability::COMPUTER_INPUT_ACTION_ID,
                            serde_json::Value::Object(input),
                        )
                        .await;
                    self.target.tool.clear_cached_observation();
                    if result.is_error {
                        if nomi_computer::tool::is_proven_unsupported_input_rejection(&result) {
                            return Err(RoleHostError::UnsupportedInput(result.content));
                        }
                        #[cfg(windows)]
                        if focus_request && nomi_computer::tool::is_proven_not_applied_focus_rejection(&result) {
                            return Err(RoleHostError::InputNotApplied(result.content));
                        }
                        if nomi_computer::tool::is_proven_stale_input_rejection(&result) {
                            return Err(RoleHostError::StaleObservationGeneration);
                        }
                        #[cfg(windows)]
                        if crate::windows_computer::is_proven_observation_rejection(&result) {
                            return Err(RoleHostError::StaleObservationGeneration);
                        }
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
                    target.observation = None;
                    let result = self
                        .target
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
                    self.target.tool.clear_cached_observation();
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

#[cfg(all(feature = "computer-use", any(test, windows)))]
pub(crate) use computer::ComputerToolPort;

#[cfg(feature = "computer-use")]
pub(crate) struct ComputerRoleInvoker {
    target: Arc<computer::ComputerTargetOwner>,
    effect_store: nomifun_agent_session::AgentSessionStore,
    hosts: tokio::sync::Mutex<std::collections::HashMap<String, Arc<ComputerRoleHost>>>,
}

#[cfg(feature = "computer-use")]
impl ComputerRoleInvoker {
    pub(crate) fn new(
        tool: Arc<nomi_computer::tool::ComputerTool>,
        effect_store: nomifun_agent_session::AgentSessionStore,
    ) -> Self {
        #[cfg(windows)]
        let tool: Arc<dyn computer::ComputerToolPort> = Arc::new(crate::windows_computer::WindowsComputerPort::new(tool));
        Self {
            target: Arc::new(computer::ComputerTargetOwner::new(tool)),
            effect_store,
            hosts: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    #[cfg(test)]
    fn new_with_test_executor(
        tool: Arc<dyn ComputerToolPort>,
        effect_store: nomifun_agent_session::AgentSessionStore,
    ) -> Self {
        Self {
            target: Arc::new(computer::ComputerTargetOwner::new(tool)),
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
                    Arc::clone(&self.target),
                    agent_session_id.clone(),
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
            Wave2TypedCapabilityOperation::ComputerInput { .. } => {
                Some(nomifun_agent_session::EffectStrategy::ExternalUncertainEffect)
            }
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
                    return Err(RoleHostError::InvalidContext("computer launch action"));
                }
                let target = object
                    .remove("target")
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(RoleHostError::InvalidContext("computer launch target"))?;
                let app = match object.remove("app") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(app)) => Some(app),
                    Some(_) => {
                        return Err(RoleHostError::InvalidContext("computer launch app"));
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
            (Some(input), Some(strategy)) => {
                match super::agent_wave2_host::begin_wave2_exclusive_effect(
                    &self.effect_store,
                    &effect_context,
                    &effect_binding,
                    input,
                    strategy,
                )
                .await
                .map_err(|error| RoleHostError::EffectSettlementFailure(error.to_string()))?
                {
                    super::agent_wave2_host::Wave2EffectAdmission::Replay(result) => {
                        return Ok(result)
                    }
                    super::agent_wave2_host::Wave2EffectAdmission::Reserved(reservation) => {
                        Some((reservation, strategy))
                    }
                }
            }
            _ => None,
        };
        let source = computer::ComputerInvocationSource {
            session_id: effect_context.agent_session_id.clone(),
            turn_id: effect_context.turn_id.clone(),
            operation_id: effect_context.operation_id.clone(),
            capability_id: effect_context.capability_id.clone(),
            action_id: effect_context.action_id.clone(),
        };
        let result = match host
            .invoke_canonical(context, operation, &self.effect_store, source)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                if let Some((reservation, strategy)) = reservation.as_ref() {
                    let effect_error = Wave2HostPortError::new(error.code(), error.to_string());
                    if strategy.is_external_uncertain()
                        && matches!(error, RoleHostError::ProviderFailure(_))
                    {
                        super::agent_wave2_host::finish_wave2_uncertain_effect(
                            reservation,
                            effect_context.action_id.as_ref(),
                            &effect_error,
                        )
                        .await
                        .map_err(|terminal| {
                            RoleHostError::EffectSettlementFailure(terminal.to_string())
                        })?;
                    } else {
                        super::agent_wave2_host::finish_wave2_failed_effect(
                            reservation,
                            effect_context.action_id.as_ref(),
                            &effect_error,
                        )
                        .await
                        .map_err(|terminal| {
                            RoleHostError::EffectSettlementFailure(terminal.to_string())
                        })?;
                    }
                }
                return Err(error);
            }
        };
        let output = StrictJsonValue(serde_json::json!({
            "generation": result.generation,
            "result": result.result
        }));
        if let Some((reservation, _)) = reservation.as_ref() {
            super::agent_wave2_host::finish_wave2_succeeded_effect(
                reservation,
                effect_context.action_id.as_ref(),
                &output,
            )
            .await
            .map_err(|error| RoleHostError::EffectSettlementFailure(error.to_string()))?;
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
        stale_input: std::sync::atomic::AtomicBool,
        uncertain_input: std::sync::atomic::AtomicBool,
        deny_accessibility: std::sync::atomic::AtomicBool,
        deny_screen_recording: std::sync::atomic::AtomicBool,
    }

    #[cfg(feature = "computer-use")]
    const UNSUPPORTED_INPUT_FIXTURE_RESULT: &str = "COMPUTER_INPUT_UNSUPPORTED_NO_EFFECT: set_element_value was rejected before native mutation. No pointer or keyboard fallback was admitted.";

    #[cfg(all(feature = "computer-use", windows))]
    const FOCUS_NOT_APPLIED_FIXTURE_RESULT: &str = "COMPUTER_FOCUS_NOT_APPLIED: Windows denied SetForegroundWindow before any restore, show, UIA or synthetic input. The OS foreground lock requires a real activation condition, such as the user activating NomiFun. No native focus effect was applied; acquire a fresh observation before a new call.";

    #[cfg(feature = "computer-use")]
    #[derive(Clone, Copy)]
    enum InputEffectFixtureReply {
        UnsupportedBeforeEffect,
        BackendAfterEffect,
        UnsupportedHintAfterEffect,
        #[cfg(windows)]
        WindowsFocusNotApplied,
        #[cfg(windows)]
        WindowsFocusAcknowledgementUnknown,
    }

    #[cfg(feature = "computer-use")]
    struct InputEffectComputerPort {
        reply: InputEffectFixtureReply,
        input_calls: std::sync::atomic::AtomicUsize,
        native_effects: std::sync::atomic::AtomicUsize,
    }

    #[cfg(feature = "computer-use")]
    #[async_trait::async_trait]
    impl ComputerToolPort for InputEffectComputerPort {
        fn clear_cached_observation(&self) {
            // Explicit fixture port with no actual AX or capture cache.
        }

        async fn execute_authorized(
            &self,
            action_id: &str,
            _: serde_json::Value,
        ) -> nomi_types::tool::ToolResult {
            use std::sync::atomic::Ordering;
            if action_id == nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID
                || action_id == nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID {
                return nomi_types::tool::ToolResult::text("fixture observation: [7] textfield");
            }
            assert_eq!(
                action_id,
                nomi_computer::capability::COMPUTER_INPUT_ACTION_ID
            );
            self.input_calls.fetch_add(1, Ordering::SeqCst);
            match self.reply {
                InputEffectFixtureReply::UnsupportedBeforeEffect => {
                    nomi_types::tool::ToolResult::error(UNSUPPORTED_INPUT_FIXTURE_RESULT)
                }
                InputEffectFixtureReply::BackendAfterEffect => {
                    self.native_effects.fetch_add(1, Ordering::SeqCst);
                    nomi_types::tool::ToolResult::error(
                        "fixture backend lost its reply after native SetValue admission",
                    )
                }
                InputEffectFixtureReply::UnsupportedHintAfterEffect => {
                    self.native_effects.fetch_add(1, Ordering::SeqCst);
                    nomi_types::tool::ToolResult::error(format!("native mutation already ran; backend hint: {UNSUPPORTED_INPUT_FIXTURE_RESULT}"))
                }
                #[cfg(windows)]
                InputEffectFixtureReply::WindowsFocusNotApplied => {
                    nomi_types::tool::ToolResult::error(FOCUS_NOT_APPLIED_FIXTURE_RESULT)
                }
                #[cfg(windows)]
                InputEffectFixtureReply::WindowsFocusAcknowledgementUnknown => {
                    self.native_effects.fetch_add(1, Ordering::SeqCst);
                    nomi_types::tool::ToolResult::error("Windows accepted foreground activation but the bounded native acknowledgement failed; the effect may have occurred. Do not automatically retry.")
                }
            }
        }
    }

    #[cfg(feature = "computer-use")]
    #[derive(Clone, Debug)]
    struct SharedComputerCache {
        field: &'static str,
        native_generation: usize,
        geometry: nomi_computer::screen::CaptureGeometry,
    }

    #[cfg(feature = "computer-use")]
    #[derive(Default)]
    struct StatefulSharedComputerToolPort {
        observations: std::sync::atomic::AtomicUsize,
        cache: std::sync::Mutex<Option<SharedComputerCache>>,
        inputs: std::sync::Mutex<Vec<serde_json::Value>>,
        fail_next_observation: std::sync::atomic::AtomicBool,
    }

    #[cfg(feature = "computer-use")]
    #[async_trait::async_trait]
    impl ComputerToolPort for StatefulSharedComputerToolPort {
        fn clear_cached_observation(&self) {
            *self.cache.lock().unwrap() = None;
        }

        async fn execute_authorized(
            &self,
            action_id: &str,
            input: serde_json::Value,
        ) -> nomi_types::tool::ToolResult {
            use std::sync::atomic::Ordering;
            if action_id == nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID
                || action_id == nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID
            {
                if self.fail_next_observation.swap(false, Ordering::SeqCst) {
                    *self.cache.lock().unwrap() = None;
                    return nomi_types::tool::ToolResult::error(
                        "shared native observation refresh failed",
                    );
                }
                let generation = self.observations.fetch_add(1, Ordering::SeqCst) + 1;
                let cache = SharedComputerCache {
                    field: if generation == 1 { "fieldA" } else { "fieldB" },
                    native_generation: generation,
                    geometry: if generation == 1 {
                        nomi_computer::screen::CaptureGeometry {
                            img_w: 1000,
                            img_h: 800,
                            logical_w: 1000,
                            logical_h: 800,
                            origin_x: 0,
                            origin_y: 0,
                        }
                    } else {
                        nomi_computer::screen::CaptureGeometry {
                            img_w: 500,
                            img_h: 400,
                            logical_w: 1000,
                            logical_h: 800,
                            origin_x: 1500,
                            origin_y: -300,
                        }
                    },
                };
                *self.cache.lock().unwrap() = Some(cache.clone());
                return nomi_types::tool::ToolResult::text(format!(
                    "native snapshot {}: [1] textfield {:?}",
                    cache.native_generation, cache.field,
                ));
            }
            if action_id == nomi_computer::capability::COMPUTER_INPUT_ACTION_ID {
                // Match the real ComputerTool seam: model ref 1 and pointer
                // coordinates are resolved against its one latest cache.
                // Record only; never create an AX engine or synthesize input.
                let cache = self
                    .cache
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or(SharedComputerCache {
                        field: "no-observation",
                        native_generation: 0,
                        geometry: nomi_computer::screen::CaptureGeometry {
                            img_w: 0,
                            img_h: 0,
                            logical_w: 0,
                            logical_h: 0,
                            origin_x: 0,
                            origin_y: 0,
                        },
                    });
                let mut admitted = serde_json::json!({
                    "field": cache.field, "native_generation": cache.native_generation,
                    "input": input.clone(),
                });
                if let (Some(x), Some(y)) = (input["x"].as_i64(), input["y"].as_i64()) {
                    let g = cache.geometry;
                    let (x, y) = nomi_computer::scale::map_llm_coord(
                        x as i32,
                        y as i32,
                        g.img_w,
                        g.img_h,
                        g.logical_w,
                        g.logical_h,
                    );
                    admitted["screen_target"] = serde_json::json!([g.origin_x + x, g.origin_y + y]);
                }
                self.inputs.lock().unwrap().push(admitted.clone());
                return nomi_types::tool::ToolResult::text(admitted.to_string());
            }
            nomi_types::tool::ToolResult::error("unexpected shared-cache fixture action")
        }
    }

    #[cfg(feature = "computer-use")]
    #[async_trait::async_trait]
    impl ComputerToolPort for FakeComputerToolPort {
        fn clear_cached_observation(&self) {
            // This port has no capture/snapshot cache; it only counts calls.
        }

        fn permission_status(&self) -> nomi_computer::permissions::PermissionStatus {
            use std::sync::atomic::Ordering;
            nomi_computer::permissions::PermissionStatus {
                accessibility: Some(!self.deny_accessibility.load(Ordering::SeqCst)),
                screen_recording: Some(!self.deny_screen_recording.load(Ordering::SeqCst)),
            }
        }

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
            if self.uncertain_input.load(Ordering::SeqCst)
                && input.get("action").and_then(serde_json::Value::as_str) != Some("observe")
            {
                return nomi_types::tool::ToolResult::error(
                    "Accessibility action on [7] failed: effect outcome uncertain: native AX \
                     request was admitted but its reply timed out. No pixel fallback was \
                     performed; re-run observe before using element refs.",
                );
            }
            if self.stale_input.load(Ordering::SeqCst)
                && input.get("action").and_then(serde_json::Value::as_str) != Some("observe")
            {
                return nomi_types::tool::ToolResult::error(
                    "Accessibility action on [7] failed: stale reference: focus changed. No \
                     pixel fallback was performed; re-run observe before using element refs.",
                );
            }
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

    fn context(role_id: &str, bindings: TypedResourceBindings) -> RoleHostContext {
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
    fn computer_host(tool: Arc<dyn ComputerToolPort>) -> (Arc<ComputerRoleHost>, RoleHostContext) {
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

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn permission_required_is_typed_before_native_observation_or_input() {
        use std::sync::atomic::Ordering;
        use nomi_computer::permissions::ComputerPermissionKind;
        let tool = Arc::new(FakeComputerToolPort::default());
        tool.deny_accessibility.store(true, Ordering::SeqCst);
        let (host, context) = computer_host(tool.clone());
        let error = host.invoke(context.clone(), ComputerRoleOperation::Observe(computer_observe()))
            .await.unwrap_err();
        assert_eq!(error.code(), "SYSTEM_PERMISSION_REQUIRED");
        let RoleHostError::PermissionRequired(missing) = &error else { panic!("{error}") };
        assert_eq!(missing.permissions, vec![ComputerPermissionKind::Accessibility]);
        assert_eq!(missing.operation, "observe");
        let encoded: serde_json::Value = serde_json::from_str(&error.to_string()).unwrap();
        assert_eq!(encoded["schema"], "system-permission-required.v1");
        assert_eq!(encoded["execution"], "not_executed");
        assert_eq!(encoded["permissions"], serde_json::json!(["accessibility"]));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);

        // Capturing pixels uses its own grant, independently from AX/input.
        let pixels = host.invoke(context.clone(), ComputerRoleOperation::Observe(ComputerObserve {
            action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
            input: serde_json::json!({"action":"screenshot"}),
        })).await.unwrap();
        let input = ComputerInput {
            action: "left_click".to_owned(),
            parameters: serde_json::json!({"x":10,"y":10}),
            expected_generation: pixels.generation,
        };
        let error = host.invoke(context.clone(), ComputerRoleOperation::Input(input.clone()))
            .await.unwrap_err();
        assert!(matches!(error, RoleHostError::PermissionRequired(_)));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1, "no native input was admitted");

        // Grant restoration does not resurrect an input lease or replay it.
        tool.deny_accessibility.store(false, Ordering::SeqCst);
        let error = host.invoke(context.clone(), ComputerRoleOperation::Input(input.clone()))
            .await.unwrap_err();
        assert_eq!(error, RoleHostError::StaleObservationGeneration);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
        let pixels = host.invoke(context.clone(), ComputerRoleOperation::Observe(ComputerObserve {
            action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
            input: serde_json::json!({"action":"screenshot"}),
        })).await.unwrap();
        host.invoke(context, ComputerRoleOperation::Input(ComputerInput {
            expected_generation: pixels.generation,
            ..input
        })).await.unwrap();
        assert_eq!(tool.calls.load(Ordering::SeqCst), 3);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn missing_screen_grant_neither_requires_accessibility_nor_blocks_launch() {
        use std::sync::atomic::Ordering;
        use nomi_computer::permissions::ComputerPermissionKind;
        let tool = Arc::new(FakeComputerToolPort::default());
        tool.deny_screen_recording.store(true, Ordering::SeqCst);
        let (host, context) = computer_host(tool.clone());
        let error = host.invoke(context.clone(), ComputerRoleOperation::Observe(ComputerObserve {
            action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
            input: serde_json::json!({"action":"screenshot"}),
        })).await.unwrap_err();
        let RoleHostError::PermissionRequired(missing) = error else { panic!("{error}") };
        assert_eq!(missing.permissions, vec![ComputerPermissionKind::ScreenRecording]);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        host.invoke(context.clone(), ComputerRoleOperation::Observe(computer_observe()))
            .await.unwrap();
        tool.deny_accessibility.store(true, Ordering::SeqCst);
        host.invoke(context.clone(), ComputerRoleOperation::Launch(ComputerLaunch {
            target: "fixture-app".to_owned(),
            app: None,
        })).await.unwrap();
        for operation in ["cursor_position", "list_windows", "wait"] {
            host.invoke(context.clone(), ComputerRoleOperation::Observe(ComputerObserve {
                action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                input: serde_json::json!({"action":operation}),
            })).await.unwrap();
        }
        assert_eq!(tool.calls.load(Ordering::SeqCst), 5);
    }

    #[cfg(feature = "computer-use")]
    async fn shared_computer_hosts(
        tool: Arc<dyn ComputerToolPort>,
    ) -> (
        ComputerRoleInvoker,
        Arc<ComputerRoleHost>,
        RoleHostContext,
        Arc<ComputerRoleHost>,
        RoleHostContext,
    ) {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(database.pool().clone())
            .await
            .unwrap();
        let invoker = ComputerRoleInvoker::new_with_test_executor(tool, store);
        let a_id = nomifun_agent_contracts::AgentSessionId::from(uuid::Uuid::now_v7().to_string());
        let b_id = nomifun_agent_contracts::AgentSessionId::from(uuid::Uuid::now_v7().to_string());
        let mut a = context(
            COMPUTER_ROLE_ID,
            vec![binding(
                COMPUTER_RESOURCE_KIND,
                &["observe", "input", "launch"],
            )],
        );
        a.runtime_instance_id = format!("agent-session:{}", a_id.as_ref());
        a.owner_lease_id = a.runtime_instance_id.clone();
        let mut b = a.clone();
        b.runtime_instance_id = format!("agent-session:{}", b_id.as_ref());
        b.owner_lease_id = b.runtime_instance_id.clone();
        let a_host = invoker
            .host_for(
                &a_id,
                &a.snapshot,
                a.registry_generation,
                a.provider.clone(),
                "resource".to_owned(),
            )
            .await;
        let b_host = invoker
            .host_for(
                &b_id,
                &b.snapshot,
                b.registry_generation,
                b.provider.clone(),
                "resource".to_owned(),
            )
            .await;
        (invoker, a_host, a, b_host, b)
    }

    #[cfg(feature = "computer-use")]
    struct ComputerFixtureStateCapture {
        captured: Arc<std::sync::Mutex<Option<nomifun_agent_domain_wave2::Wave2StateHandle>>>,
    }

    #[cfg(feature = "computer-use")]
    impl Wave2HostPort for ComputerFixtureStateCapture {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                    + Send
                    + 'a,
            >,
        > {
            let captured = Arc::clone(&self.captured);
            Box::pin(async move {
                *captured.lock().unwrap() = Some(request.context.state);
                Ok(StrictJsonValue(serde_json::json!({})))
            })
        }
    }

    #[cfg(feature = "computer-use")]
    async fn computer_fixture_state_handle() -> nomifun_agent_domain_wave2::Wave2StateHandle {
        use nomifun_agent_contracts::{
            ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload,
            AgentSessionId, CapabilityId, CapabilityRef, CapabilitySelection, CorrelationId,
            DigestHex, IdempotencyKey, OperationId, PresetRevisionRef, ResourceBindingId,
            ResourceId, ResourceKind, RuntimeProfileKind, RuntimeTarget, ScopeKey, UserId,
            VersionString,
        };
        use nomifun_agent_kernel::{
            AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest, CompilerEnvironment,
            InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
            SessionCapabilityState,
        };
        use std::collections::{BTreeMap, BTreeSet};
        let captured = Arc::new(std::sync::Mutex::new(None));
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                nomifun_agent_domain_wave2::registrations_with_host_port(Arc::new(
                    ComputerFixtureStateCapture {
                        captured: Arc::clone(&captured),
                    },
                ))
                .expect("Wave 2 registrations"),
            )
            .expect("publish Wave 2 registrations");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave2-host-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("wave2-host-workspace"),
            resource_kind: ResourceKind::from("workspace"),
            resource_id: ResourceId::from("wave2-host-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let action = ActionId::from("workspace.files/read");
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from("workspace.files"),
                },
                action_allowlist: BTreeSet::from([action.clone()]),
            }],

            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Wave 2 host test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };
        let contribution_locks = vec![materialized
            .capability(&CapabilityId::from("workspace.files"))
            .expect("materialized workspace.files Module")
            .contribution_lock
            .clone()];
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave2-host-test"),
                revision: 1,
                revision_digest: DigestHex::from(""),
            },
            payload,
            contribution_locks,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().expect("revision digest");
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(
                    nomifun_agent_domain_wave2::CONTRACT_VERSION,
                ),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("test-target"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave2-host-test".to_owned(),
            },
            CompileRequest {
                revision,
                principal: principal.clone(),
                scene: "wave2-host-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave2-host-resolve"),
            },
        )
        .expect("compile selected capability")
        .with_target_resource_bindings(&principal, vec![binding])
        .expect("bind selected target resource");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        registry
            .invoke(
                &snapshot,
                &active,
                CapabilityInvocationRequest {
                    principal: principal.clone(),
                    session_owner: principal,
                    agent_session_id: AgentSessionId::from(uuid::Uuid::now_v7().to_string()),
                    turn_id: OperationId::from("wave2-host-turn"),
                    operation_id: OperationId::from("wave2-host-operation"),
                    idempotency_key: IdempotencyKey::from("wave2-host-idempotency"),
                    correlation_id: CorrelationId::from("wave2-host-correlation"),
                    resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: CapabilityId::from("workspace.files"),
                    action_id: action,
                    resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
                        "wave2-host-workspace",
                    )]),
                    state_scope_key: ScopeKey::from("session:wave2-host"),
                    input: StrictJsonValue(serde_json::json!({"path": "fixture.txt"})),
                },
            )
            .await
            .expect("state projection invocation");
        let handle = captured
            .lock()
            .expect("state capture mutex")
            .take()
            .expect("host received the state handle");
        handle
    }

    /// Real canonical Session/Turn/native-lease facts paired with an explicit
    /// fake physical Computer port. This proves Host authorization and durable
    /// stamps, not OS input, TCC grants, or native accessibility execution.
    #[cfg(feature = "computer-use")]
    struct CanonicalComputerFixture {
        store: nomifun_agent_session::AgentSessionStore,
        session_id: nomifun_agent_contracts::AgentSessionId,
        turn_id: nomifun_agent_contracts::OperationId,
        started: nomifun_agent_contracts::EventId,
        lease: nomifun_agent_session::NativeExecutionLease,
        context: RoleHostContext,
    }

    #[cfg(feature = "computer-use")]
    impl CanonicalComputerFixture {
        fn event(
            &self,
            key: &str,
            kind: &str,
            correlation: &str,
            cause: nomifun_agent_contracts::EventId,
            payload: serde_json::Value,
        ) -> nomifun_agent_contracts::SessionEventAppend {
            use nomifun_agent_contracts::{
                SemanticSessionEventDraft, SessionEventKind, SessionEventPayloadRef,
            };
            let identity = format!("canonical-computer:{}:{key}", self.session_id.as_ref());
            nomifun_agent_contracts::SessionEventAppend {
                agent_session_id: self.session_id.clone(),
                event_id: identity.clone().into(),
                producer_id: "runtime_supervisor".into(),
                idempotency_key: identity.into(),
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind(kind.into()),
                    kind_version: 1,
                    correlation_id: correlation.into(),
                    causation_event_id: Some(cause),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
                },
            }
        }

        async fn new(key: &str) -> Self {
            let database = nomifun_db::init_database_memory().await.unwrap();
            let store =
                nomifun_agent_session::AgentSessionStore::from_pool(database.pool().clone())
                    .await
                    .unwrap();
            Self::new_in_store(key, store).await
        }

        async fn new_in_store(key: &str, store: nomifun_agent_session::AgentSessionStore) -> Self {
            use nomifun_agent_contracts::{
                AgentBindingValue, AgentSessionLiveRecord, AgentSessionMetadata, PresetRevisionRef,
            };
            let session_id =
                nomifun_agent_contracts::AgentSessionId::from(uuid::Uuid::now_v7().to_string());
            let mut context = context(
                COMPUTER_ROLE_ID,
                vec![binding(
                    COMPUTER_RESOURCE_KIND,
                    &["observe", "input", "launch"],
                )],
            );
            context.runtime_instance_id = format!("agent-session:{}", session_id.as_ref());
            context.owner_lease_id = context.runtime_instance_id.clone();
            let session = AgentSessionLiveRecord {
                agent_session_id: session_id.clone(),
                owner_ref: context.principal.clone(),
                metadata: AgentSessionMetadata {
                    purpose: Default::default(),
                    app_project_id: None,
                    companion: None,
                    title: None,
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: AgentBindingValue {
                    preset_revision_ref: PresetRevisionRef {
                        preset_id: "computer-fixture".into(),
                        revision: 1,
                        revision_digest: "b".repeat(64).into(),
                    },
                    resolved_snapshot_ref: context.snapshot.clone(),
                    typed_resource_bindings: context.resource_bindings.clone(),
                    binding_version: 1,
                },
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            };
            let created = store
                .create_session(nomifun_agent_session::CreateSessionRequest::new(
                    session,
                    1,
                    format!("{key}:create"),
                    "session_api".into(),
                    format!("{key}:create").into(),
                    format!("{key}:create").into(),
                ))
                .await
                .unwrap();
            use nomifun_agent_contracts::{
                SemanticSessionEventDraft, SessionEventKind, SessionEventPayloadRef,
            };
            store
                .append_event(&nomifun_agent_contracts::SessionEventAppend {
                    agent_session_id: session_id.clone(),
                    event_id: format!("{key}:ready").into(),
                    producer_id: "runtime_supervisor".into(),
                    idempotency_key: format!("{key}:ready").into(),
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind("session/ready".into()),
                        kind_version: 1,
                        correlation_id: format!("{key}:ready").into(),
                        causation_event_id: Some(created.opening_ack.event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                            serde_json::json!({}),
                        )),
                    },
                })
                .await
                .unwrap();
            let turn_id: nomifun_agent_contracts::OperationId = format!("{key}:turn-1").into();
            let started = store
                .start_turn(
                    &session_id,
                    "session_api".into(),
                    format!("{key}:input-1").into(),
                    turn_id.clone(),
                    StrictJsonValue(
                        serde_json::json!({"content":"exercise canonical computer authority"}),
                    ),
                )
                .await
                .unwrap()
                .1
                .ack
                .unwrap()
                .event_id;
            let lease = store
                .claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
                    owner: context.principal.clone(),
                    agent_session_id: session_id.clone(),
                    operation_id: turn_id.clone(),
                    snapshot: context.snapshot.clone(),
                    active_set_generation: 0,
                    holder: format!("{key}:holder-1"),
                    expected_fence: 0,
                    checkpoint: None,
                })
                .await
                .unwrap();
            Self {
                store,
                session_id,
                turn_id,
                started,
                lease,
                context,
            }
        }

        async fn host(&self, invoker: &ComputerRoleInvoker) -> Arc<ComputerRoleHost> {
            invoker
                .host_for(
                    &self.session_id,
                    &self.context.snapshot,
                    self.context.registry_generation,
                    self.context.provider.clone(),
                    "resource".to_owned(),
                )
                .await
        }

        async fn call(
            &self,
            key: &str,
            operation: &ComputerRoleOperation,
        ) -> computer::ComputerInvocationSource {
            let operation_id: nomifun_agent_contracts::OperationId =
                format!("{}:tool:{key}", self.turn_id.as_ref()).into();
            let event = self.event(
                key,
                "tool/call-started",
                key,
                self.started.clone(),
                serde_json::json!({"operation_id":operation_id,"call_id":key,
                    "capability_id":"computer","action_id":operation.action_id()}),
            );
            self.store
                .append_native_event(&self.lease, &event, None)
                .await
                .unwrap();
            computer::ComputerInvocationSource {
                session_id: self.session_id.clone(),
                turn_id: self.turn_id.clone(),
                operation_id,
                capability_id: "computer".into(),
                action_id: operation.action_id().into(),
            }
        }

        async fn invoke(
            &self,
            host: &ComputerRoleHost,
            key: &str,
            operation: ComputerRoleOperation,
        ) -> Result<computer::ComputerRoleResult, RoleHostError> {
            let source = self.call(key, &operation).await;
            host.invoke_canonical(self.context.clone(), operation, &self.store, source)
                .await
        }

        async fn invoke_actual_invoker(
            &self,
            invoker: &ComputerRoleInvoker,
            state: nomifun_agent_domain_wave2::Wave2StateHandle,
            key: &str,
            operation: ComputerRoleOperation,
        ) -> Result<StrictJsonValue, RoleHostError> {
            let source = self.call(key, &operation).await;
            let typed = match operation {
                ComputerRoleOperation::Observe(request) => {
                    if request.action_id == nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID {
                        Wave2TypedCapabilityOperation::ComputerObserve {
                            input: StrictJsonValue(request.input),
                        }
                    } else {
                        Wave2TypedCapabilityOperation::ComputerA11yObserve {
                            input: StrictJsonValue(request.input),
                        }
                    }
                }
                ComputerRoleOperation::Input(request) => {
                    let mut input = request.parameters.as_object().unwrap().clone();
                    input.insert("action".to_owned(), serde_json::json!(request.action));
                    input.insert(
                        "expected_generation".to_owned(),
                        serde_json::json!(request.expected_generation),
                    );
                    Wave2TypedCapabilityOperation::ComputerInput {
                        input: StrictJsonValue(serde_json::Value::Object(input)),
                    }
                }
                ComputerRoleOperation::Launch(_) => {
                    panic!("effect fixture admits observation and input only")
                }
            };
            RoleHostInvoker::invoke(
                invoker,
                Wave2TypedHostRequest {
                    context: nomifun_agent_domain_wave2::Wave2HostContext {
                        principal: self.context.principal.clone(),
                        agent_session_id: source.session_id,
                        turn_id: source.turn_id,
                        idempotency_key: format!(
                            "fixture-tool:{}",
                            nomifun_agent_contracts::digest_bytes(
                                source.operation_id.as_ref().as_bytes()
                            )
                            .as_ref()
                        )
                        .into(),
                        correlation_id: self.turn_id.as_ref().into(),
                        operation_id: source.operation_id,
                        resolved_snapshot_ref: self.context.snapshot.clone(),
                        registry_generation: self.context.registry_generation,
                        capability_id: source.capability_id,
                        action_id: source.action_id,
                        role_provider: Some(self.context.provider.clone()),
                        state,
                        resource_bindings: self.context.resource_bindings.clone(),
                    },
                    operation: typed,
                },
            )
            .await
        }

        async fn finish_and_start_successor(&mut self, output: serde_json::Value) {
            use nomifun_agent_contracts::digest_bytes;
            let tool = self
                .store
                .tool_call_causation(
                    &self.session_id,
                    &self.turn_id,
                    &format!("{}:tool:old-observe", self.turn_id.as_ref()).into(),
                    &"computer".into(),
                    &nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID.into(),
                )
                .await
                .unwrap();
            let result = self.event("old-tool-result", "tool/result-recorded", "old-observe", tool.event_id,
                serde_json::json!({"operation_id":format!("{}:tool:old-observe",self.turn_id.as_ref()),
                    "call_id":"old-observe","output":output,"error":null}));
            self.store
                .append_native_observation(&self.lease, &result, None)
                .await
                .unwrap();
            let message = self.event(
                "old-assistant-completed",
                "message/completed",
                "old-assistant",
                self.started.clone(),
                serde_json::json!({"part_count":0,"content_digest":digest_bytes(b"")}),
            );
            let terminal = self.event(
                "old-turn-completed",
                "turn/completed",
                self.turn_id.as_ref(),
                self.started.clone(),
                serde_json::json!({}),
            );
            self.store
                .append_native_chat_completion(&self.lease, &message, &terminal)
                .await
                .unwrap();
            self.turn_id = format!("{}:turn-2", self.session_id.as_ref()).into();
            self.started = self
                .store
                .start_turn(
                    &self.session_id,
                    "session_api".into(),
                    format!("{}:input-2", self.session_id.as_ref()).into(),
                    self.turn_id.clone(),
                    StrictJsonValue(serde_json::json!({"content":"independent successor turn"})),
                )
                .await
                .unwrap()
                .1
                .ack
                .unwrap()
                .event_id;
            self.lease = self
                .store
                .claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
                    owner: self.context.principal.clone(),
                    agent_session_id: self.session_id.clone(),
                    operation_id: self.turn_id.clone(),
                    snapshot: self.context.snapshot.clone(),
                    active_set_generation: 0,
                    holder: "successor-fixture-holder".into(),
                    expected_fence: 0,
                    checkpoint: None,
                })
                .await
                .unwrap();
        }
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_canonical_session_scope_rejects_shared_sequence_and_preserves_peer_authority()
    {
        let a = CanonicalComputerFixture::new("canonical-computer-a").await;
        let b =
            CanonicalComputerFixture::new_in_store("canonical-computer-b", a.store.clone()).await;
        let tool = Arc::new(StatefulSharedComputerToolPort::default());
        let invoker = ComputerRoleInvoker::new_with_test_executor(tool.clone(), a.store.clone());
        let a_host = a.host(&invoker).await;
        let b_host = b.host(&invoker).await;
        let old = a
            .invoke(
                &a_host,
                "a-observe",
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let fresh = b
            .invoke(
                &b_host,
                "b-observe",
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        // Canonical event seq is per Session. Equal numbers confer no shared
        // authority, so this also rejects a model that guesses B's exact stamp.
        assert_eq!(old.generation, fresh.generation);
        let input = |generation, text: &str| {
            ComputerRoleOperation::Input(ComputerInput {
                action: "set_element_value".to_owned(),
                parameters: serde_json::json!({"ref":1,"text":text}),
                expected_generation: generation,
            })
        };
        let stale = a
            .invoke(
                &a_host,
                "a-old-token",
                input(old.generation, "from-a-old-token"),
            )
            .await;
        let guessed = a
            .invoke(
                &a_host,
                "a-guessed-b-token",
                input(fresh.generation, "from-a-guessed-b-token"),
            )
            .await;
        assert_eq!(
            stale.unwrap_err(),
            RoleHostError::StaleObservationGeneration
        );
        assert_eq!(
            guessed.unwrap_err(),
            RoleHostError::StaleObservationGeneration
        );
        assert!(tool.inputs.lock().unwrap().is_empty());
        b.invoke(
            &b_host,
            "b-fresh-token",
            input(fresh.generation, "from-b-fresh-token"),
        )
        .await
        .unwrap();
        let inputs = tool.inputs.lock().unwrap().clone();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["field"], "fieldB");
        assert_eq!(inputs[0]["input"]["text"], "from-b-fresh-token");
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_canonical_observation_requires_exact_unsettled_call_before_executor() {
        use std::sync::atomic::Ordering;

        let fixture = CanonicalComputerFixture::new("canonical-computer-call").await;
        let tool = Arc::new(FakeComputerToolPort::default());
        let invoker =
            ComputerRoleInvoker::new_with_test_executor(tool.clone(), fixture.store.clone());
        let host = fixture.host(&invoker).await;
        let observe = || ComputerRoleOperation::Observe(computer_observe());
        let missing = computer::ComputerInvocationSource {
            session_id: fixture.session_id.clone(),
            turn_id: fixture.turn_id.clone(),
            operation_id: "missing-call".into(),
            capability_id: "computer".into(),
            action_id: nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID.into(),
        };
        assert!(matches!(
            host.invoke_canonical(fixture.context.clone(), observe(), &fixture.store, missing)
                .await,
            Err(RoleHostError::EffectSettlementFailure(_))
        ));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        let first_source = fixture.call("first-observe", &observe()).await;
        let first_operation = first_source.operation_id.clone();
        let first = host
            .invoke_canonical(
                fixture.context.clone(),
                observe(),
                &fixture.store,
                first_source,
            )
            .await
            .unwrap();
        let first_cause = fixture
            .store
            .tool_call_causation(
                &fixture.session_id,
                &fixture.turn_id,
                &first_operation,
                &"computer".into(),
                &nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID.into(),
            )
            .await
            .unwrap();
        assert_eq!(first.generation, first_cause.seq);
        let result = fixture.event(
            "first-result",
            "tool/result-recorded",
            "first-observe",
            first_cause.event_id,
            serde_json::json!({"operation_id":first_operation,"call_id":"first-observe",
                "output":{"generation":first.generation,"result":first.result},"error":null}),
        );
        fixture
            .store
            .append_native_observation(&fixture.lease, &result, None)
            .await
            .unwrap();
        let replay_source = computer::ComputerInvocationSource {
            session_id: fixture.session_id.clone(),
            turn_id: fixture.turn_id.clone(),
            operation_id: first_operation,
            capability_id: "computer".into(),
            action_id: nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID.into(),
        };
        assert!(matches!(
            host.invoke_canonical(
                fixture.context.clone(),
                observe(),
                &fixture.store,
                replay_source
            )
            .await,
            Err(RoleHostError::EffectSettlementFailure(_))
        ));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
        let second = fixture
            .invoke(&host, "second-observe", observe())
            .await
            .unwrap();
        assert!(second.generation > first.generation);
        let ambiguous = fixture.call("ambiguous-observe", &observe()).await;
        let duplicate = fixture.event("ambiguous-duplicate", "tool/call-started", "ambiguous-duplicate", fixture.started.clone(),
            serde_json::json!({"operation_id":ambiguous.operation_id,"call_id":"duplicate",
                "capability_id":"computer","action_id":nomi_computer::capability::COMPUTER_A11Y_OBSERVE_ACTION_ID}));
        fixture
            .store
            .append_native_event(&fixture.lease, &duplicate, None)
            .await
            .unwrap();
        assert!(matches!(
            host.invoke_canonical(
                fixture.context.clone(),
                observe(),
                &fixture.store,
                ambiguous
            )
            .await,
            Err(RoleHostError::EffectSettlementFailure(_))
        ));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
        // Refusing missing/completed/ambiguous calls does not revoke the
        // current legitimate observation, since no new native Observe began.
        fixture
            .invoke(
                &host,
                "current-input",
                ComputerRoleOperation::Input(ComputerInput {
                    action: "set_element_value".to_owned(),
                    parameters: serde_json::json!({"ref":7,"text":"current"}),
                    expected_generation: second.generation,
                }),
            )
            .await
            .unwrap();
        assert_eq!(tool.calls.load(Ordering::SeqCst), 3);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_unsupported_input_records_failed_zero_effect_and_backend_stays_unknown(
    ) {
        use nomifun_agent_session::{AgentEffectState, EffectStrategy};
        use std::sync::atomic::Ordering;

        // The required opaque state handle is obtained through the public
        // Kernel API using a capture-only port. It is not native or workspace
        // evidence. All Computer calls below use the actual Role Invoker and
        // real canonical Store/Turn/native lease/effect admission/settlement.
        let state = computer_fixture_state_handle().await;
        for (key, reply, rejected_before_effect) in [
            (
                "unsupported-input-effect",
                InputEffectFixtureReply::UnsupportedBeforeEffect,
                true,
            ),
            (
                "backend-after-effect",
                InputEffectFixtureReply::BackendAfterEffect,
                false,
            ),
            (
                "unsupported-hint-after-effect",
                InputEffectFixtureReply::UnsupportedHintAfterEffect,
                false,
            ),
            #[cfg(windows)]
            ("windows-focus-not-applied", InputEffectFixtureReply::WindowsFocusNotApplied, true),
            #[cfg(windows)]
            ("windows-focus-unknown", InputEffectFixtureReply::WindowsFocusAcknowledgementUnknown, false),
        ] {
            let focus_input = match reply {
                #[cfg(windows)]
                InputEffectFixtureReply::WindowsFocusNotApplied | InputEffectFixtureReply::WindowsFocusAcknowledgementUnknown => true,
                _ => false,
            };
            let fixture = CanonicalComputerFixture::new(key).await;
            let tool = Arc::new(InputEffectComputerPort {
                reply,
                input_calls: Default::default(),
                native_effects: Default::default(),
            });
            let invoker =
                ComputerRoleInvoker::new_with_test_executor(tool.clone(), fixture.store.clone());
            let observed = fixture
                .invoke_actual_invoker(
                    &invoker,
                    state.clone(),
                    "observe-before-input",
                    ComputerRoleOperation::Observe(if focus_input {
                        ComputerObserve { action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                            input: serde_json::json!({"action":"list_windows"}) }
                    } else { computer_observe() }),
                )
                .await
                .unwrap();
            let generation = observed.0["generation"].as_u64().unwrap();
            assert!(generation > 0);
            let input = fixture
                .invoke_actual_invoker(
                    &invoker,
                    state.clone(),
                    "input-once",
                    ComputerRoleOperation::Input(ComputerInput {
                        action: if focus_input { "focus_window" } else { "set_element_value" }.to_owned(),
                        parameters: if focus_input { serde_json::json!({"window_id":42}) } else { serde_json::json!({"ref":7,"text":"complete replacement"}) },
                        expected_generation: generation,
                    }),
                )
                .await;
            if rejected_before_effect {
                #[cfg(windows)]
                if focus_input {
                    assert_eq!(input.as_ref().unwrap_err(), &RoleHostError::InputNotApplied(FOCUS_NOT_APPLIED_FIXTURE_RESULT.into()));
                }
                if !focus_input {
                    assert_eq!(
                        input.unwrap_err(),
                        RoleHostError::UnsupportedInput(UNSUPPORTED_INPUT_FIXTURE_RESULT.to_owned())
                    );
                }
                assert_eq!(
                    tool.native_effects.load(Ordering::SeqCst),
                    0,
                    "pre-action Unsupported admitted a mutation"
                );
            } else {
                assert!(
                    matches!(input, Err(RoleHostError::ProviderFailure(_))),
                    "a dispatched error was falsely reduced to zero effect: {input:?}"
                );
                assert_eq!(tool.native_effects.load(Ordering::SeqCst), 1);
            }
            assert_eq!(
                tool.input_calls.load(Ordering::SeqCst),
                1,
                "input was retried"
            );
            let effects = fixture
                .store
                .list_effects(&fixture.session_id)
                .await
                .unwrap();
            let [effect] = effects.as_slice() else {
                panic!("expected exactly one real canonical input effect");
            };
            assert_eq!(effect.strategy, EffectStrategy::ExternalUncertainEffect);
            assert_eq!(
                effect.action_id.as_ref(),
                nomi_computer::capability::COMPUTER_INPUT_ACTION_ID
            );
            assert_eq!(effect.turn_id, fixture.turn_id);
            assert_eq!(
                effect.operation_id.as_ref(),
                format!("{}:tool:input-once", fixture.turn_id.as_ref())
            );
            assert_eq!(
                effect.resource_binding_id.as_ref(),
                Some(&fixture.context.resource_bindings[0].binding_id)
            );
            assert_eq!(
                effect.state,
                if rejected_before_effect {
                    AgentEffectState::Rejected
                } else {
                    AgentEffectState::Unknown
                }
            );
            assert_eq!(
                fixture
                    .store
                    .has_unsettled_effects(&fixture.session_id)
                    .await
                    .unwrap(),
                !rejected_before_effect
            );
            let page = fixture
                .store
                .read_events(&fixture.session_id, None, 200)
                .await
                .unwrap();
            let started = page
                .events
                .iter()
                .find(|event| event.event_id == effect.started_event_id)
                .unwrap();
            let cause_id = started.causation_event_id.as_ref().unwrap();
            assert!(page
                .events
                .iter()
                .any(|event| event.event_id == *cause_id && event.kind.0 == "tool/call-started"));
            let terminal = page
                .events
                .iter()
                .find(|event| Some(&event.event_id) == effect.terminal_event_id.as_ref())
                .unwrap();
            assert_eq!(
                terminal.kind.0,
                if rejected_before_effect {
                    "effect/failed"
                } else {
                    "effect/uncertain"
                }
            );
            if rejected_before_effect {
                assert!(!page
                    .events
                    .iter()
                    .any(|event| event.kind.0 == "effect/uncertain"));
                assert_eq!(
                    effect.bounded_observation.as_ref().unwrap()["error"]["code"],
                    if focus_input { "ROLE_HOST_INPUT_NOT_APPLIED" } else { "ROLE_HOST_UNSUPPORTED_INPUT" }
                );
            }
            // Any admitted input consumes the observation, including a proven
            // unsupported refusal. No result mints a new input lease.
            if rejected_before_effect {
                let host = fixture.host(&invoker).await;
                let stale = fixture
                    .invoke(
                        &host,
                        "old-token-after-refusal",
                        ComputerRoleOperation::Input(ComputerInput {
                            action: if focus_input { "focus_window" } else { "set_element_value" }.to_owned(),
                            parameters: if focus_input { serde_json::json!({"window_id":42}) } else { serde_json::json!({"ref":7,"text":"must not execute"}) },
                            expected_generation: generation,
                        }),
                    )
                    .await;
                assert_eq!(
                    stale.unwrap_err(),
                    RoleHostError::StaleObservationGeneration
                );
                assert_eq!(tool.input_calls.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_missing_permission_records_rejected_zero_effect() {
        use nomifun_agent_session::{AgentEffectState, EffectStrategy};
        use std::sync::atomic::Ordering;
        let state = computer_fixture_state_handle().await;
        let fixture = CanonicalComputerFixture::new("missing-permission-input-effect").await;
        let tool = Arc::new(FakeComputerToolPort::default());
        let invoker = ComputerRoleInvoker::new_with_test_executor(tool.clone(), fixture.store.clone());
        let observed = fixture.invoke_actual_invoker(
            &invoker,
            state.clone(),
            "observe-before-permission-revocation",
            ComputerRoleOperation::Observe(computer_observe()),
        ).await.unwrap();
        let generation = observed.0["generation"].as_u64().unwrap();
        tool.deny_accessibility.store(true, Ordering::SeqCst);
        let error = fixture.invoke_actual_invoker(
            &invoker,
            state,
            "permission-blocked-input",
            ComputerRoleOperation::Input(ComputerInput {
                action: "set_element_value".to_owned(),
                parameters: serde_json::json!({"ref":7,"text":"must not execute"}),
                expected_generation: generation,
            }),
        ).await.unwrap_err();
        assert_eq!(error.code(), "SYSTEM_PERMISSION_REQUIRED");
        assert!(matches!(error, RoleHostError::PermissionRequired(_)));
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1, "only the prior observation ran");
        let effects = fixture.store.list_effects(&fixture.session_id).await.unwrap();
        let [effect] = effects.as_slice() else { panic!("expected the canonical input effect") };
        assert_eq!(effect.strategy, EffectStrategy::ExternalUncertainEffect);
        assert_eq!(effect.state, AgentEffectState::Rejected);
        assert!(!fixture.store.has_unsettled_effects(&fixture.session_id).await.unwrap());
        assert_eq!(effect.bounded_observation.as_ref().unwrap()["error"]["code"], "SYSTEM_PERMISSION_REQUIRED");
        let events = fixture.store.read_events(&fixture.session_id, None, 200).await.unwrap();
        assert!(events.events.iter().any(|event| Some(&event.event_id) == effect.terminal_event_id.as_ref() && event.kind.0 == "effect/failed"));
        assert!(!events.events.iter().any(|event| event.kind.0 == "effect/uncertain"));

        tool.deny_accessibility.store(false, Ordering::SeqCst);
        let host = fixture.host(&invoker).await;
        let error = fixture.invoke(
            &host,
            "do-not-reuse-pre-permission-observation",
            ComputerRoleOperation::Input(ComputerInput {
                action: "set_element_value".to_owned(),
                parameters: serde_json::json!({"ref":7,"text":"must not replay"}),
                expected_generation: generation,
            }),
        ).await.unwrap_err();
        assert_eq!(error, RoleHostError::StaleObservationGeneration);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
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

    #[test]
    fn effect_settlement_failure_is_not_projected_as_a_provider_failure() {
        let error = RoleHostError::EffectSettlementFailure(
            "canonical terminal observation is uncommitted".to_owned(),
        );
        assert_eq!(error.code(), "CAPABILITY_UNAVAILABLE");
        assert!(error.to_string().contains("role effect settlement failed"));
        assert!(!error.to_string().contains("role provider failed"));
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn computer_invoker_serializes_peer_observations_on_physical_target() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (_invoker, a_host, a, b_host, b) = shared_computer_hosts(tool.clone()).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for (host, context) in [(a_host, a), (b_host, b)] {
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                host.invoke(context, ComputerRoleOperation::Observe(computer_observe()))
                    .await
                    .unwrap()
                    .generation
            }));
        }
        barrier.wait().await;
        let mut generations = Vec::new();
        for task in tasks {
            generations.push(task.await.unwrap());
        }
        generations.sort_unstable();
        assert_eq!(generations, vec![1, 2]);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            tool.max_active.load(Ordering::SeqCst),
            1,
            "peer Hosts share one native cache lock"
        );
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
    async fn computer_provider_stale_rejection_keeps_the_typed_no_effect_error() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        tool.stale_input.store(true, Ordering::SeqCst);
        let (host, context) = computer_host(tool.clone());
        let observed = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let error = host
            .invoke(
                context,
                ComputerRoleOperation::Input(ComputerInput {
                    action: "set_element_value".to_owned(),
                    parameters: serde_json::json!({ "ref": 7, "text": "forbidden" }),
                    expected_generation: observed.generation,
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
    async fn computer_provider_uncertain_input_does_not_become_a_zero_effect_rejection() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        tool.uncertain_input.store(true, Ordering::SeqCst);
        let (host, context) = computer_host(tool.clone());
        let observed = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let error = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 7 }),
                    expected_generation: observed.generation,
                }),
            )
            .await
            .unwrap_err();
        // ComputerRoleInvoker's external-input strategy settles ProviderFailure
        // through finish_wave2_uncertain_effect. StaleObservationGeneration is
        // reserved for a proven rejection before native input was admitted.
        assert!(matches!(&error, RoleHostError::ProviderFailure(message)
            if message.contains("effect outcome uncertain")));
        let old_token = host
            .invoke(
                context,
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 7 }),
                    expected_generation: observed.generation,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(old_token, RoleHostError::StaleObservationGeneration);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn unbound_computer_rejects_every_operation_before_native_executor_admission() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        let operations = [
            ComputerRoleOperation::Observe(computer_observe()),
            ComputerRoleOperation::Input(ComputerInput {
                action: "click_element".to_owned(),
                parameters: serde_json::json!({ "ref": 7 }),
                expected_generation: 1,
            }),
            ComputerRoleOperation::Launch(ComputerLaunch {
                target: "NomiNeverLaunchWithoutComputerBinding".to_owned(),
                app: None,
            }),
        ];
        for bindings in [
            Vec::new(),
            vec![binding("workspace", &["observe", "input", "launch"])],
        ] {
            let mut unbound = context.clone();
            unbound.resource_bindings = bindings;
            for operation in &operations {
                let error = host
                    .invoke(unbound.clone(), operation.clone())
                    .await
                    .unwrap_err();
                assert_eq!(error, RoleHostError::ResourceCardinality);
                assert_eq!(error.code(), "ROLE_HOST_RESOURCE_CARDINALITY");
            }
        }
        assert_eq!(
            tool.calls.load(Ordering::SeqCst),
            0,
            "an optional unbound device never grants native tool authority"
        );
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
    async fn computer_observation_generation_never_recycles_after_input_and_refresh() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        let input = |generation| {
            ComputerRoleOperation::Input(ComputerInput {
                action: "click_element".to_owned(),
                parameters: serde_json::json!({ "ref": 1 }),
                expected_generation: generation,
            })
        };
        let old = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let initial_input = host
            .invoke(context.clone(), input(old.generation))
            .await
            .unwrap();
        assert_eq!(initial_input.generation, 0);
        let fresh = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();

        let before_replay = tool.calls.load(Ordering::SeqCst);
        let replay = host.invoke(context.clone(), input(old.generation)).await;
        let after_replay = tool.calls.load(Ordering::SeqCst);

        // A broken implementation may have admitted and invalidated the old
        // replay. Establish a fresh positive control before asserting the
        // captured failure, so rejecting every input cannot satisfy this test.
        let current = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let current_input = host
            .invoke(context, input(current.generation))
            .await
            .unwrap();
        assert_eq!(current_input.generation, 0);
        assert_eq!(
            tool.calls.load(Ordering::SeqCst),
            after_replay + 2,
            "a fresh current generation must still reach the executor exactly once"
        );

        assert!(fresh.generation > old.generation,
            "observation token recycled: old={}, fresh={}, old replay={replay:?}, executor calls {before_replay}->{after_replay}",
            old.generation, fresh.generation);
        assert_eq!(
            replay.unwrap_err(),
            RoleHostError::StaleObservationGeneration
        );
        assert_eq!(
            after_replay, before_replay,
            "an old observation token must not re-enter the native executor after refresh"
        );
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_observation_generation_exhaustion_never_wraps_or_preserves_authority() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(FakeComputerToolPort::default());
        let (host, context) = computer_host(tool.clone());
        // This overflow is an explicit no-Store fixture boundary. Production
        // has no Atomic counter: its stamp is the exact positive SQLite event
        // seq, with Store decoding/admission failures closed before the port.
        host.seed_fixture_call_seq_for_test(u64::MAX - 1);
        let last = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        assert_eq!(last.generation, u64::MAX);
        let exhausted = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await;
        let counter_after_exhaustion = host.fixture_call_seq_for_test();
        let before_input = tool.calls.load(Ordering::SeqCst);
        let input = host
            .invoke(
                context.clone(),
                ComputerRoleOperation::Input(ComputerInput {
                    action: "click_element".to_owned(),
                    parameters: serde_json::json!({ "ref": 1 }),
                    expected_generation: last.generation,
                }),
            )
            .await;
        let after_input = tool.calls.load(Ordering::SeqCst);
        let subsequent = host
            .invoke(context, ComputerRoleOperation::Observe(computer_observe()))
            .await;

        assert!(exhausted.is_err(),
            "exhausted observation minted a token: {exhausted:?}; stored counter={counter_after_exhaustion}; subsequent={subsequent:?}");
        assert_eq!(
            counter_after_exhaustion,
            u64::MAX,
            "observation counter must never wrap"
        );
        assert_eq!(
            input.unwrap_err(),
            RoleHostError::StaleObservationGeneration
        );
        assert_eq!(
            after_input, before_input,
            "exhaustion must revoke input before executor admission"
        );
        assert!(
            subsequent.is_err(),
            "an exhausted host must not mint another observation token"
        );
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_session_ref_cache_cannot_cross_session_authority() {
        let tool = Arc::new(StatefulSharedComputerToolPort::default());
        let (_invoker, a_host, a, b_host, b) = shared_computer_hosts(tool.clone()).await;
        assert!(
            !Arc::ptr_eq(&a_host, &b_host),
            "the actual Invoker must create distinct Session hosts"
        );
        let a_observed = a_host
            .invoke(
                a.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        b_host
            .invoke(
                b.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let b_observed = b_host
            .invoke(
                b.clone(),
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let input = |generation, text: &str| {
            ComputerRoleOperation::Input(ComputerInput {
                action: "set_element_value".to_owned(),
                parameters: serde_json::json!({ "ref": 1, "text": text }),
                expected_generation: generation,
            })
        };
        let old_a = a_host
            .invoke(a.clone(), input(a_observed.generation, "from-A-old-token"))
            .await;
        let guessed_b = a_host
            .invoke(a, input(b_observed.generation, "from-A-guessed-peer-token"))
            .await;
        let before_b_control = tool.inputs.lock().unwrap().clone();
        // A rejected call must not invalidate B's still-current observation.
        let b_control = b_host
            .invoke(b, input(b_observed.generation, "from-B-current-token"))
            .await;
        let all_inputs = tool.inputs.lock().unwrap().clone();

        assert!(matches!(&old_a, Err(RoleHostError::StaleObservationGeneration)),
            "Session A's old token consumed Session B's latest ref cache: {old_a:?}; inputs={before_b_control:?}");
        assert_eq!(
            guessed_b.unwrap_err(),
            RoleHostError::StaleObservationGeneration,
            "knowing B's generation must not grant A B's observation authority"
        );
        assert!(
            before_b_control.is_empty(),
            "peer inputs entered the executor: {before_b_control:?}"
        );
        assert!(
            b_control.is_ok(),
            "rejecting A revoked B's current authority: {b_control:?}"
        );
        assert_eq!(all_inputs.len(), 1);
        assert_eq!(all_inputs[0]["field"], "fieldB");
        assert_eq!(all_inputs[0]["input"]["text"], "from-B-current-token");
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_session_pixel_cache_cannot_cross_session_authority() {
        let tool = Arc::new(StatefulSharedComputerToolPort::default());
        let (_invoker, a_host, a, b_host, b) = shared_computer_hosts(tool.clone()).await;
        let screenshot = || {
            ComputerRoleOperation::Observe(ComputerObserve {
                action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                input: serde_json::json!({ "action": "screenshot" }),
            })
        };
        let a_observed = a_host.invoke(a.clone(), screenshot()).await.unwrap();
        let b_observed = b_host.invoke(b.clone(), screenshot()).await.unwrap();
        let input = |generation| {
            ComputerRoleOperation::Input(ComputerInput {
                action: "left_click".to_owned(),
                parameters: serde_json::json!({ "x": 10, "y": 20 }),
                expected_generation: generation,
            })
        };
        let a_input = a_host.invoke(a, input(a_observed.generation)).await;
        let before_b_control = tool.inputs.lock().unwrap().clone();
        let b_input = b_host.invoke(b, input(b_observed.generation)).await;
        let all_inputs = tool.inputs.lock().unwrap().clone();

        assert!(matches!(&a_input, Err(RoleHostError::StaleObservationGeneration)),
            "Session A's image coordinates consumed B's scale/origin: {a_input:?}; inputs={before_b_control:?}");
        assert!(
            before_b_control.is_empty(),
            "a stale image admitted pointer input"
        );
        assert!(
            b_input.is_ok(),
            "rejecting A revoked B's current pixel observation"
        );
        assert_eq!(all_inputs.len(), 1);
        assert_eq!(
            all_inputs[0]["screen_target"],
            serde_json::json!([1521, -259])
        );
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_failed_observation_refresh_revokes_previous_authority() {
        use std::sync::atomic::Ordering;

        let tool = Arc::new(StatefulSharedComputerToolPort::default());
        let (_invoker, a_host, a, _b_host, _b) = shared_computer_hosts(tool.clone()).await;
        let screenshot = || {
            ComputerRoleOperation::Observe(ComputerObserve {
                action_id: nomi_computer::capability::COMPUTER_OBSERVE_ACTION_ID,
                input: serde_json::json!({ "action": "screenshot" }),
            })
        };
        let observed = a_host.invoke(a.clone(), screenshot()).await.unwrap();
        tool.fail_next_observation.store(true, Ordering::SeqCst);
        let failed = a_host.invoke(a.clone(), screenshot()).await;
        assert!(matches!(failed, Err(RoleHostError::ProviderFailure(_))));
        let input = a_host
            .invoke(
                a,
                ComputerRoleOperation::Input(ComputerInput {
                    action: "left_click".to_owned(),
                    parameters: serde_json::json!({ "x": 10, "y": 20 }),
                    expected_generation: observed.generation,
                }),
            )
            .await;
        let admitted = tool.inputs.lock().unwrap().clone();
        assert!(
            matches!(&input, Err(RoleHostError::StaleObservationGeneration)),
            "a failed refresh preserved the previous token: input={input:?}; inputs={admitted:?}"
        );
        assert!(
            admitted.is_empty(),
            "failed observation left pointer authority live"
        );
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    async fn computer_invoker_rebuild_rejects_previous_factory_observation_token() {
        use std::sync::atomic::Ordering;

        let mut fixture = CanonicalComputerFixture::new("computer-cold-factory").await;
        let old_tool = Arc::new(StatefulSharedComputerToolPort::default());
        let old_invoker =
            ComputerRoleInvoker::new_with_test_executor(old_tool.clone(), fixture.store.clone());
        let old_host = fixture.host(&old_invoker).await;
        let old_observation = fixture
            .invoke(
                &old_host,
                "old-observe",
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        // Persist this exact output on its canonical tool/result-recorded
        // predecessor and close the original Turn before starting a new one.
        // The physical provider remains an explicit fake, not native evidence.
        let previous_output = serde_json::json!({
            "generation": old_observation.generation,
            "result": old_observation.result,
        });
        fixture
            .finish_and_start_successor(previous_output.clone())
            .await;
        let history = fixture
            .store
            .read_events(&fixture.session_id, None, 256)
            .await
            .unwrap();
        let old_result = history
            .events
            .iter()
            .find(|event| event.kind.0 == "tool/result-recorded")
            .unwrap();
        let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(old_result) =
            &old_result.payload
        else {
            panic!("the fixture's exact tool output must remain canonical inline data");
        };
        assert_eq!(old_result.0["output"], previous_output);
        let retained_generation = old_result.0["output"]["generation"].as_u64().unwrap();
        assert!(history
            .events
            .iter()
            .any(|event| event.kind.0 == "turn/completed"));
        drop(old_host);
        drop(old_invoker);
        drop(old_tool);

        let new_tool = Arc::new(StatefulSharedComputerToolPort::default());
        // The new native cache's ref 1 is a different current field. Native
        // snapshot numbering is independent of the model-visible Host stamp.
        new_tool.observations.store(1, Ordering::SeqCst);
        let new_invoker =
            ComputerRoleInvoker::new_with_test_executor(new_tool.clone(), fixture.store.clone());
        let new_host = fixture.host(&new_invoker).await;
        let fresh = fixture
            .invoke(
                &new_host,
                "fresh-observe",
                ComputerRoleOperation::Observe(computer_observe()),
            )
            .await
            .unwrap();
        let input = |generation, text: &str| {
            ComputerRoleOperation::Input(ComputerInput {
                action: "set_element_value".to_owned(),
                parameters: serde_json::json!({ "ref": 1, "text": text }),
                expected_generation: generation,
            })
        };
        let old_replay = fixture
            .invoke(
                &new_host,
                "old-token-replay",
                input(retained_generation, "previous-factory-history-token"),
            )
            .await;
        let before_current_control = new_tool.inputs.lock().unwrap().clone();
        let current_control = fixture
            .invoke(
                &new_host,
                "fresh-token-control",
                input(fresh.generation, "current-factory-token"),
            )
            .await;
        let all_inputs = new_tool.inputs.lock().unwrap().clone();

        assert!(
            fresh.generation > retained_generation,
            "a canonical fact sequence must survive a factory rebuild"
        );
        assert!(matches!(&old_replay, Err(RoleHostError::StaleObservationGeneration)),
            "a rebuilt factory reused an older observation stamp: retained={}, fresh={}, replay={old_replay:?}; inputs={before_current_control:?}; current={current_control:?}",
            retained_generation, fresh.generation);
        assert!(
            before_current_control.is_empty(),
            "an older factory's token entered native input"
        );
        assert!(
            current_control.is_ok(),
            "rejecting old history must preserve the fresh authority"
        );
        assert_eq!(all_inputs.len(), 1);
        assert_eq!(all_inputs[0]["field"], "fieldB");
        assert_eq!(all_inputs[0]["input"]["text"], "current-factory-token");
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
