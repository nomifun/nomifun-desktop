//! Adapter from the isolated Coding Engine tool loop to the NomiFun
//! Capability Kernel.
//!
//! The adapter owns no handlers and no capability catalog.  It receives an
//! already compiled Snapshot and a SessionCapabilityState from the platform,
//! then projects a model Tool Call into the Kernel's canonical invocation
//! contract.  This keeps Plugin/MiniApp/Wave2 ownership outside the Coding
//! Engine.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, CapabilityId, EffectClass, PrincipalRef, ScopeKey, StrictJsonValue,
    ToolPresentationKind,
};
use nomifun_agent_kernel::{
    ActiveCapabilitySetSnapshot, CapabilityInvocationRequest, CompiledSnapshot, KernelError,
    KernelRegistry, MaterializedRegistry, SessionCapabilityState,
};
use nomifun_chat_model_broker::ChatToolDefinition;
use tokio_util::sync::CancellationToken;

use crate::error::CodingEngineError;
use crate::tool::{
    input_schema_digest, CodingEffectClass, CodingToolBinding, CodingToolInvocation,
    CodingToolInvoker, CodingToolPlan, CodingToolResult,
};

/// A model-facing Tool definition explicitly mapped to one canonical
/// Capability action.
#[derive(Clone, Debug, PartialEq)]
pub struct CodingToolExposure {
    pub definition: ChatToolDefinition,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
}

/// Compile the model Tool surface from one immutable Snapshot and one exact
/// active-set generation.
///
/// The caller supplies only explicit exposures. This function never scans the
/// registry to auto-add capabilities, so a newly installed Plugin cannot
/// silently expand an existing AgentSession.
pub fn compile_coding_tool_plan(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    exposures: impl IntoIterator<Item = CodingToolExposure>,
) -> Result<CodingToolPlan, CodingEngineError> {
    validate_snapshot_registry(snapshot, active, registry)?;

    let mut bindings = Vec::new();
    for exposure in exposures {
        let resolved = resolved_capability(snapshot, &exposure.capability_id).ok_or_else(|| {
            CodingEngineError::ToolPlan(format!(
                "capability {} is outside the compiled Snapshot",
                exposure.capability_id.as_ref()
            ))
        })?;
        if !active.active.contains(&exposure.capability_id) {
            return Err(CodingEngineError::ToolPlan(format!(
                "capability {} is not active in generation {}",
                exposure.capability_id.as_ref(),
                active.generation
            )));
        }
        let materialized = registry
            .capability(&exposure.capability_id)
            .ok_or_else(|| {
                CodingEngineError::ToolPlan(format!(
                    "capability {} is not materialized",
                    exposure.capability_id.as_ref()
                ))
            })?;
        if materialized.manifest.version != resolved.capability.version
            || materialized.schema_digest != resolved.schema_digest
        {
            return Err(CodingEngineError::ToolPlan(format!(
                "capability {} materialization differs from the compiled Snapshot",
                exposure.capability_id.as_ref()
            )));
        }
        let action = materialized
            .manifest
            .contributions
            .actions
            .iter()
            .find(|action| action.action_id == exposure.action_id)
            .ok_or_else(|| {
                CodingEngineError::ToolPlan(format!(
                    "action {} is not declared by capability {}",
                    exposure.action_id.as_ref(),
                    exposure.capability_id.as_ref()
                ))
            })?;
        if matches!(
            action.presentation,
            ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
        ) {
            return Err(CodingEngineError::ToolPlan(format!(
                "action {} is not eligible for the Coding function-tool surface",
                exposure.action_id.as_ref()
            )));
        }
        let policy = snapshot
            .policy(&exposure.capability_id)
            .ok_or_else(|| {
                CodingEngineError::ToolPlan(format!(
                    "capability {} has no compiled authority policy",
                    exposure.capability_id.as_ref()
                ))
            })?;
        if !policy.allowed_actions.contains(&exposure.action_id) {
            return Err(CodingEngineError::ToolPlan(format!(
                "action {} is outside the Snapshot allowlist",
                exposure.action_id.as_ref()
            )));
        }
        let (effect_class, parallel_safe) = coding_effect(action.effect_class);
        let schema_digest = input_schema_digest(&exposure.definition.input_schema)?;
        bindings.push(CodingToolBinding {
            model_name: exposure.definition.name.clone(),
            definition: exposure.definition,
            schema_digest,
            canonical_input_schema_ref: action.input_schema.clone(),
            capability_contract_digest: resolved.schema_digest.clone(),
            capability_id: exposure.capability_id,
            action_id: exposure.action_id,
            resource_binding_ids: policy.resource_binding_ids.clone(),
            effect_class,
            parallel_safe,
        });
    }
    CodingToolPlan::new(bindings)
}

/// A Kernel-backed Tool invoker for one immutable AgentSession Snapshot.
///
/// `CompiledSnapshot` and `SessionCapabilityState` must come from the same
/// AgentSession application service. The adapter deliberately does not
/// compile presets, scan the global registry, or resolve native paths.
pub struct KernelCodingToolInvoker {
    registry: Arc<KernelRegistry>,
    snapshot: Arc<CompiledSnapshot>,
    active_capabilities: Arc<SessionCapabilityState>,
    session_owner: PrincipalRef,
    state_scope_key: ScopeKey,
}

impl KernelCodingToolInvoker {
    pub fn new(
        registry: Arc<KernelRegistry>,
        snapshot: Arc<CompiledSnapshot>,
        active_capabilities: Arc<SessionCapabilityState>,
        session_owner: PrincipalRef,
        state_scope_key: ScopeKey,
    ) -> Self {
        Self {
            registry,
            snapshot,
            active_capabilities,
            session_owner,
            state_scope_key,
        }
    }

    pub fn snapshot(&self) -> &CompiledSnapshot {
        &self.snapshot
    }

    pub fn session_owner(&self) -> &PrincipalRef {
        &self.session_owner
    }

    pub fn state_scope_key(&self) -> &ScopeKey {
        &self.state_scope_key
    }

    async fn invoke_kernel(
        &self,
        invocation: CodingToolInvocation,
    ) -> Result<CodingToolResult, CodingEngineError> {
        let active = self
            .active_capabilities
            .snapshot()
            .map_err(kernel_error)?;
        let materialized = self.registry.snapshot().map_err(kernel_error)?;
        validate_snapshot_registry(&self.snapshot, &active, &materialized)?;
        validate_binding_contract(
            &self.snapshot,
            &active,
            &materialized,
            &invocation.binding,
            invocation.active_set_generation,
        )?;
        let request = CapabilityInvocationRequest {
            principal: invocation.principal,
            session_owner: self.session_owner.clone(),
            agent_session_id: invocation.agent_session_id,
            operation_id: invocation.operation_id,
            idempotency_key: invocation.idempotency_key,
            correlation_id: invocation.correlation_id,
            resolved_snapshot_ref: invocation.resolved_snapshot_ref,
            active_set_generation: invocation.active_set_generation,
            capability_id: invocation.binding.capability_id,
            action_id: invocation.binding.action_id,
            resource_binding_ids: invocation.binding.resource_binding_ids,
            state_scope_key: self.state_scope_key.clone(),
            input: StrictJsonValue(invocation.call.arguments.0),
        };
        let output = self
            .registry
            .invoke(&self.snapshot, &active, request)
            .await
            .map_err(kernel_error)?;
        Ok(CodingToolResult::text(
            invocation.call.call_id,
            serde_json::to_string(&output.0).map_err(|error| {
                CodingEngineError::ToolInvocation(format!(
                    "Kernel result could not be serialized: {error}"
                ))
            })?,
            false,
        ))
    }
}

#[async_trait]
impl CodingToolInvoker for KernelCodingToolInvoker {
    async fn invoke(
        &self,
        invocation: CodingToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<CodingToolResult, CodingEngineError> {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        tokio::select! {
            _ = cancellation.cancelled() => Err(CodingEngineError::Cancelled),
            result = self.invoke_kernel(invocation) => result,
        }
    }
}

fn kernel_error(error: KernelError) -> CodingEngineError {
    CodingEngineError::CapabilityKernel {
        code: error.canonical_code().0,
        message: error.to_string(),
    }
}

fn validate_snapshot_registry(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
) -> Result<(), CodingEngineError> {
    if active.resolved_snapshot_ref != *snapshot.snapshot_ref() {
        return Err(CodingEngineError::ToolPlan(
            "active capability set belongs to a different Snapshot".to_owned(),
        ));
    }
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(CodingEngineError::ToolPlan(
            "materialized capability registry differs from the compiled Snapshot".to_owned(),
        ));
    }
    Ok(())
}

fn validate_binding_contract(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    binding: &CodingToolBinding,
    active_set_generation: u64,
) -> Result<(), CodingEngineError> {
    binding.validate()?;
    if active.generation != active_set_generation
        || !active.active.contains(&binding.capability_id)
    {
        return Err(CodingEngineError::ToolPlan(format!(
            "capability {} is not active in generation {}",
            binding.capability_id.as_ref(),
            active_set_generation
        )));
    }
    let resolved = resolved_capability(snapshot, &binding.capability_id).ok_or_else(|| {
        CodingEngineError::ToolPlan(format!(
            "capability {} is outside the compiled Snapshot",
            binding.capability_id.as_ref()
        ))
    })?;
    if resolved.schema_digest != binding.capability_contract_digest {
        return Err(CodingEngineError::ToolPlan(format!(
            "capability {} digest differs from the Tool binding",
            binding.capability_id.as_ref()
        )));
    }
    let materialized = registry.capability(&binding.capability_id).ok_or_else(|| {
        CodingEngineError::ToolPlan(format!(
            "capability {} is not materialized",
            binding.capability_id.as_ref()
        ))
    })?;
    if materialized.schema_digest != binding.capability_contract_digest
        || materialized.manifest.version != resolved.capability.version
    {
        return Err(CodingEngineError::ToolPlan(format!(
            "capability {} materialization differs from the Tool binding",
            binding.capability_id.as_ref()
        )));
    }
    let action = materialized
        .manifest
        .contributions
        .actions
        .iter()
        .find(|action| action.action_id == binding.action_id)
        .ok_or_else(|| {
            CodingEngineError::ToolPlan(format!(
                "action {} is not materialized for capability {}",
                binding.action_id.as_ref(),
                binding.capability_id.as_ref()
            ))
        })?;
    if action.input_schema != binding.canonical_input_schema_ref {
        return Err(CodingEngineError::ToolPlan(format!(
            "action {} canonical input schema differs from the Tool binding",
            binding.action_id.as_ref()
        )));
    }
    let policy = snapshot.policy(&binding.capability_id).ok_or_else(|| {
        CodingEngineError::ToolPlan(format!(
            "capability {} has no compiled authority policy",
            binding.capability_id.as_ref()
        ))
    })?;
    if !policy.allowed_actions.contains(&binding.action_id)
        || policy.resource_binding_ids != binding.resource_binding_ids
    {
        return Err(CodingEngineError::ToolPlan(format!(
            "capability {} authority projection differs from the Tool binding",
            binding.capability_id.as_ref()
        )));
    }
    Ok(())
}

fn resolved_capability<'a>(
    snapshot: &'a CompiledSnapshot,
    capability_id: &CapabilityId,
) -> Option<&'a nomifun_agent_contracts::ResolvedCapability> {
    snapshot
        .content()
        .initial_capabilities
        .iter()
        .chain(snapshot.content().on_demand_capabilities.iter())
        .find(|resolved| &resolved.capability.id == capability_id)
}

fn coding_effect(effect: EffectClass) -> (CodingEffectClass, bool) {
    match effect {
        EffectClass::Pure | EffectClass::ReadLocal => (CodingEffectClass::ReadOnly, true),
        EffectClass::ReadSensitive => (CodingEffectClass::ReadOnly, false),
        EffectClass::WriteReversible
        | EffectClass::WriteDurable
        | EffectClass::ExecuteLocal
        | EffectClass::Destructive => (CodingEffectClass::ManagedEffect, false),
        EffectClass::ExternalTransmit | EffectClass::Irreversible | EffectClass::Physical => {
            (CodingEffectClass::ExternalUncertainEffect, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    use super::*;
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, AgentSessionId,
        CapabilityExposure, CapabilityRef, CapabilitySelection, CorrelationId, DigestHex,
        IdempotencyKey, OperationId, PresetRevisionRef,
        ResourceBindingId, ResourceId, ResourceKind, RuntimeProfileKind, RuntimeTarget, UserId,
        VersionString, digest_payload,
    };
    use nomifun_agent_domain_wave2::{
        CONTRACT_VERSION, Wave2HostPort, Wave2HostPortError, Wave2HostRequest, action_id,
        registrations_with_host_port,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CompileRequest, CompilerEnvironment, InMemoryPluginStatePersistence,
        MaterializationPolicy,
    };
    use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};
    use serde_json::json;

    type SeenInvocation = (String, String, String, Vec<String>);

    struct RecordingHost {
        seen: Arc<Mutex<Vec<SeenInvocation>>>,
    }

    impl Wave2HostPort for RecordingHost {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let seen = Arc::clone(&self.seen);
            Box::pin(async move {
                seen.lock().unwrap().push((
                    request.context.capability_id.as_ref().to_owned(),
                    request.context.action_id.as_ref().to_owned(),
                    request.context.agent_session_id.as_ref().to_owned(),
                    request
                        .context
                        .resource_bindings
                        .iter()
                        .map(|binding| binding.binding_id.as_ref().to_owned())
                        .collect(),
                ));
                Ok(StrictJsonValue(json!({
                    "path": request.operation_input().0["path"],
                    "content": "owner-result"
                })))
            })
        }
    }

    trait Wave2RequestInput {
        fn operation_input(&self) -> &StrictJsonValue;
    }

    impl Wave2RequestInput for Wave2HostRequest {
        fn operation_input(&self) -> &StrictJsonValue {
            match &self.operation {
                nomifun_agent_domain_wave2::Wave2CapabilityOperation::WorkspaceExecution {
                    input,
                }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::Ssh { input }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::McpConnectors {
                    input,
                }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::Browser { input }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::ComputerA11y {
                    input,
                } => input,
            }
        }
    }

    struct KernelFixture {
        registry: Arc<KernelRegistry>,
        materialized: Arc<MaterializedRegistry>,
        snapshot: Arc<CompiledSnapshot>,
        active: Arc<SessionCapabilityState>,
        principal: PrincipalRef,
        action_id: ActionId,
        binding_id: ResourceBindingId,
        seen: Arc<Mutex<Vec<SeenInvocation>>>,
    }

    fn kernel_fixture() -> KernelFixture {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(
            KernelRegistry::new(
                MaterializationPolicy::stable(CONTRACT_VERSION),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        let materialized = registry
            .replace_all(
                registrations_with_host_port(Arc::new(RecordingHost {
                    seen: Arc::clone(&seen),
                }))
                .unwrap(),
            )
            .unwrap();
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "coding-owner".to_owned(),
        };
        let capability_id = CapabilityId::from("fs.read");
        let action_id = action_id(capability_id.as_ref()).unwrap();
        let binding_id = ResourceBindingId::from("workspace-binding");
        let binding = nomifun_agent_contracts::TypedResourceBinding {
            binding_id: binding_id.clone(),
            resource_kind: ResourceKind::from("workspace"),
            resource_id: ResourceId::from("workspace-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: capability_id,
                    version: VersionString::from(CONTRACT_VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([action_id.clone()]),
                resource_binding_refs: vec![binding_id.clone()],
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: StrictJsonValue(json!({})),
            }],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: vec![binding],
            persona: "Coding Engine Kernel test".to_owned(),
            instructions: "Read one file.".to_owned(),
            context_policy: StrictJsonValue(json!({})),
            execution_constraints: StrictJsonValue(json!({})),
            runtime_budget: StrictJsonValue(json!({})),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("coding-kernel-test"),
                revision: 1,
                revision_digest: digest_payload(&payload).unwrap(),
            },
            payload,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = Arc::new(
            AgentPresetCompiler::compile(
                &materialized,
                &CompilerEnvironment {
                    resolver_version: VersionString::from(CONTRACT_VERSION),
                    required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                    required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                    runtime_feature_inventory_digest: DigestHex::from("runtime"),
                    available_runtime_features: BTreeSet::new(),
                    canonical_schema_manifest_digest: DigestHex::from("schema"),
                    target_contribution_manifest_digest: DigestHex::from("target"),
                    host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
                    host_surface: "desktop".to_owned(),
                    availability_evidence_revision: "coding-kernel-test".to_owned(),
                },
                CompileRequest {
                    revision,
                    principal: principal.clone(),
                    scene: "coding-kernel-test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("resolve"),
                },
            )
            .unwrap(),
        );
        let active = Arc::new(SessionCapabilityState::new(&snapshot));
        KernelFixture {
            registry,
            materialized,
            snapshot,
            active,
            principal,
            action_id,
            binding_id,
            seen,
        }
    }

    fn read_exposure(action_id: ActionId) -> CodingToolExposure {
        CodingToolExposure {
            definition: ChatToolDefinition {
                name: "read_file".to_owned(),
                description: "Read a UTF-8 file from the bound workspace.".to_owned(),
                input_schema: StrictJsonValue(json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": {"type": "string"}
                    },
                    "required": ["path"]
                })),
                deferred: false,
            },
            capability_id: CapabilityId::from("fs.read"),
            action_id,
        }
    }

    #[test]
    fn adapter_type_keeps_session_scope_explicit() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KernelCodingToolInvoker>();
    }

    #[tokio::test]
    async fn compiled_plan_invokes_the_kernel_with_exact_snapshot_authority() {
        let fixture = kernel_fixture();
        let active = fixture.active.snapshot().unwrap();
        let plan = compile_coding_tool_plan(
            &fixture.snapshot,
            &active,
            &fixture.materialized,
            [read_exposure(fixture.action_id.clone())],
        )
        .unwrap();
        let binding = plan.binding("read_file").unwrap().clone();
        assert_eq!(
            binding.resource_binding_ids,
            BTreeSet::from([fixture.binding_id.clone()])
        );
        assert_eq!(binding.effect_class, CodingEffectClass::ReadOnly);
        assert!(binding.parallel_safe);

        let invoker = KernelCodingToolInvoker::new(
            Arc::clone(&fixture.registry),
            Arc::clone(&fixture.snapshot),
            Arc::clone(&fixture.active),
            fixture.principal.clone(),
            ScopeKey::from("session:coding-kernel-test"),
        );
        let result = invoker
            .invoke(
                CodingToolInvocation {
                    agent_session_id: AgentSessionId::from("coding-session"),
                    principal: fixture.principal,
                    resolved_snapshot_ref: fixture.snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    turn_operation_id: OperationId::from("turn"),
                    operation_id: OperationId::from("turn:tool:call-1"),
                    idempotency_key: IdempotencyKey::from("coding-tool:call-1"),
                    correlation_id: CorrelationId::from("coding-tool:call-1"),
                    call: ChatToolCall {
                        call_id: ToolCallId::from("call-1"),
                        name: "read_file".to_owned(),
                        arguments: StrictJsonValue(json!({"path": "README.md"})),
                        provider_metadata: None,
                    },
                    binding,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(result.output_text().contains("owner-result"));
        assert_eq!(
            *fixture.seen.lock().unwrap(),
            vec![(
                "fs.read".to_owned(),
                "fs.read.invoke".to_owned(),
                "coding-session".to_owned(),
                vec!["workspace-binding".to_owned()]
            )]
        );
    }

    #[test]
    fn plan_compilation_rejects_unselected_capabilities() {
        let fixture = kernel_fixture();
        let active = fixture.active.snapshot().unwrap();
        let error = compile_coding_tool_plan(
            &fixture.snapshot,
            &active,
            &fixture.materialized,
            [CodingToolExposure {
                definition: ChatToolDefinition {
                    name: "write_file".to_owned(),
                    description: "Write a file.".to_owned(),
                    input_schema: StrictJsonValue(json!({"type": "object"})),
                    deferred: false,
                },
                capability_id: CapabilityId::from("fs.write"),
                action_id: action_id("fs.write").unwrap(),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, CodingEngineError::ToolPlan(_)));
    }
}
