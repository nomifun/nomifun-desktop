//! Adapter from the engine tool loop to the NomiFun
//! Capability Kernel.
//!
//! The adapter owns no handlers and no capability catalog.  It receives an
//! already compiled Snapshot and a SessionCapabilityState from the platform,
//! then projects a model Tool Call into the Kernel's canonical invocation
//! contract. This keeps capability ownership outside every engine.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, EffectClass, PrincipalRef, ScopeKey, StrictJsonValue,
    ToolPresentationKind,
};
use nomifun_agent_kernel::{
    ActiveCapabilitySetSnapshot, CapabilityInvocationRequest, CompiledSnapshot, KernelError,
    KernelRegistry, MaterializedRegistry, SessionCapabilityState,
};
use nomifun_chat_model_broker::ChatToolDefinition;
use tokio_util::sync::CancellationToken;

use crate::error::EngineToolError;
use crate::tool::{
    EngineEffectClass, EngineToolBinding, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult, input_schema_digest,
};

/// A model-facing Tool definition explicitly mapped to one canonical
/// Capability action.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineToolExposure {
    pub definition: ChatToolDefinition,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
}

/// Compile the model Tool surface from one immutable Snapshot and one exact
/// canonical Store-committed active subset and generation.
///
/// The caller supplies only explicit exposures. This function never scans the
/// registry to auto-add capabilities, so a newly installed Plugin cannot
/// silently expand an existing AgentSession.
pub fn compile_engine_tool_plan(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    exposures: impl IntoIterator<Item = EngineToolExposure>,
) -> Result<EngineToolPlan, EngineToolError> {
    validate_snapshot_registry(snapshot, active, registry)?;

    let mut bindings = Vec::new();
    for mut exposure in exposures {
        let resolved = snapshot.resolved_capability(&exposure.capability_id).ok_or_else(|| {
            EngineToolError::ToolPlan(format!(
                "capability {} is outside the compiled Snapshot",
                exposure.capability_id.as_ref()
            ))
        })?;
        if !active.active.contains(&exposure.capability_id) {
            return Err(EngineToolError::ToolPlan(format!(
                "capability {} is not active in generation {}",
                exposure.capability_id.as_ref(),
                active.generation
            )));
        }
        let materialized = registry
            .capability(&exposure.capability_id)
            .ok_or_else(|| {
                EngineToolError::ToolPlan(format!(
                    "capability {} is not materialized",
                    exposure.capability_id.as_ref()
                ))
            })?;
        if materialized.schema_digest != resolved.schema_digest {
            return Err(EngineToolError::ToolPlan(format!(
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
                EngineToolError::ToolPlan(format!(
                    "action {} is not declared by capability {}",
                    exposure.action_id.as_ref(),
                    exposure.capability_id.as_ref()
                ))
            })?;
        if matches!(
            action.presentation,
            ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
        ) {
            return Err(EngineToolError::ToolPlan(format!(
                "action {} is not eligible for the model function-tool surface",
                exposure.action_id.as_ref()
            )));
        }
        let policy = snapshot.policy(&exposure.capability_id).ok_or_else(|| {
            EngineToolError::ToolPlan(format!(
                "capability {} has no compiled authority policy",
                exposure.capability_id.as_ref()
            ))
        })?;
        if !policy.allowed_actions.contains(&exposure.action_id) {
            return Err(EngineToolError::ToolPlan(format!(
                "action {} is outside the Snapshot allowlist",
                exposure.action_id.as_ref()
            )));
        }
        let (effect_class, parallel_safe) = engine_effect(action.effect_class);
        exposure.definition.input_schema = crate::model_tool_schema(&exposure.definition.input_schema);
        let schema_digest = input_schema_digest(&exposure.definition.input_schema)?;
        bindings.push(EngineToolBinding {
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
    EngineToolPlan::new(bindings)
}

/// A Kernel-backed Tool invoker for one immutable AgentSession Snapshot.
///
/// `CompiledSnapshot` and `SessionCapabilityState` must come from the same
/// AgentSession application service. The adapter deliberately does not
/// compile presets, scan the global registry, or resolve native paths.
pub struct KernelEngineToolInvoker {
    registry: Arc<KernelRegistry>,
    snapshot: Arc<CompiledSnapshot>,
    active_capabilities: Arc<SessionCapabilityState>,
    session_owner: PrincipalRef,
    state_scope_key: ScopeKey,
    admitted_session: Option<(AgentSessionId, EngineToolPlan)>,
}

impl KernelEngineToolInvoker {
    /// Low-level trusted host adapter for non-Session/custom scopes. Engine
    /// driver assembly should use for_session to pin the admitted surface.
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
            admitted_session: None,
        }
    }

    /// Application assembly fixes the Session and full selected tool surface.
    /// Only the frozen enabled capabilities may be invoked. No model supplied
    /// binding may rewrite the admitted name/schema/action mapping or expand
    /// the Session's authority.
    pub fn for_session(
        registry: Arc<KernelRegistry>,
        snapshot: Arc<CompiledSnapshot>,
        active_capabilities: Arc<SessionCapabilityState>,
        session_owner: PrincipalRef,
        session_id: AgentSessionId,
        plan: EngineToolPlan,
    ) -> Result<Self, EngineToolError> {
        if session_id.as_ref().trim().is_empty()
            || session_id.as_ref().trim() != session_id.as_ref()
        {
            return Err(EngineToolError::InvalidContract(
                "empty or noncanonical Session id".into(),
            ));
        }
        let scope = ScopeKey::from(format!("session:{}", session_id.as_ref()));
        let mut port = Self::new(
            registry,
            snapshot,
            active_capabilities,
            session_owner,
            scope,
        );
        port.admitted_session = Some((session_id, plan));
        Ok(port)
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
        invocation: EngineToolInvocation,
    ) -> Result<EngineToolResult, EngineToolError> {
        invocation
            .call
            .validate()
            .map_err(|error| EngineToolError::InvalidModelEvent(error.to_string()))?;
        crate::tool::parse_completed_arguments(&invocation.call)?;
        if invocation.call.name != invocation.binding.model_name {
            return Err(EngineToolError::ToolInvocation(
                "model call differs from its canonical mapping".into(),
            ));
        }
        if let Some((session_id, plan)) = &self.admitted_session {
            if &invocation.agent_session_id != session_id
                || invocation.principal != self.session_owner
                || invocation.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
                || plan.binding(&invocation.call.name) != Some(&invocation.binding)
            {
                return Err(EngineToolError::ToolInvocation(
                    "invocation differs from admitted Session/tool surface".into(),
                ));
            }
        }
        let active = self.active_capabilities.snapshot().map_err(kernel_error)?;
        let materialized = self.registry.snapshot().map_err(kernel_error)?;
        validate_snapshot_registry(&self.snapshot, &active, &materialized)?;
        validate_binding_contract(
            &self.snapshot,
            &active,
            &materialized,
            &invocation.binding,
            invocation.active_set_generation,
        )?;
        let is_process = is_workspace_process_action(
            &invocation.binding.capability_id,
            &invocation.binding.action_id,
        );
        let is_file_write = invocation.binding.capability_id.as_ref() == "workspace.files"
            && matches!(invocation.binding.action_id.as_ref(), "workspace.files/write" | "workspace.files/patch");
        let request = CapabilityInvocationRequest {
            principal: invocation.principal,
            session_owner: self.session_owner.clone(),
            agent_session_id: invocation.agent_session_id,
            turn_id: invocation.turn_operation_id,
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
            .map_err(|error| kernel_error_for_action(error, is_process, is_file_write))?;
        Ok(EngineToolResult::text(
            invocation.call.call_id,
            serde_json::to_string(&output.0).map_err(|error| {
                EngineToolError::ToolInvocation(format!(
                    "Kernel result could not be serialized: {error}"
                ))
            })?,
            // A successfully dispatched command is not necessarily a
            // successful command. Preserve nonzero exit as model feedback.
            process_result_is_error(is_process, &output.0),
        ))
    }
}

fn is_workspace_process_action(capability_id: &CapabilityId, action_id: &ActionId) -> bool {
    capability_id.as_ref() == nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID
        && nomifun_agent_domain_wave2::WORKSPACE_PROCESS_ACTION_IDS
            .contains(&action_id.as_ref())
}

fn process_result_is_error(is_process: bool, output: &serde_json::Value) -> bool {
    is_process
        && output.get("state").and_then(serde_json::Value::as_str) != Some("running")
        && output.get("success").and_then(serde_json::Value::as_bool) != Some(true)
}

#[async_trait]
impl EngineToolInvoker for KernelEngineToolInvoker {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        // Once dispatched, cancellation must not drop an uncertain side
        // effect. The application must poll this inside its owned task group
        // and persist the result even if an engine stops waiting for it.
        self.invoke_kernel(invocation).await
    }
}

fn kernel_error(error: KernelError) -> EngineToolError {
    let code = error.canonical_code().0;
    let message = match code.as_str() {
        "AGENT_EXECUTION_ALREADY_ACTIVE" => "This conversation already has an active AgentExecution. Do not start sibling collaboration calls; put all independent tasks in one agent/delegate request with strategy=parallel and use synthesize=true when a downstream Agent must combine them.".to_owned(),
        _ => error.to_string(),
    };
    EngineToolError::CapabilityKernel {
        code,
        message,
    }
}

/// Keep host diagnostics private while returning actionable, fixed guidance
/// for the common process launch failures seen by coding Agents. The raw
/// command, cwd, environment and owner error never cross the model boundary.
fn kernel_error_for_action(error: KernelError, is_process: bool, is_file_write: bool) -> EngineToolError {
    if is_file_write && let Some(failure) = error.capability_execution_failure() {
        return EngineToolError::CapabilityKernel {
            code: error.canonical_code().0,
            message: workspace_write_feedback(&failure.message),
        };
    }
    if is_process && error.canonical_code().as_ref() == "CAPABILITY_UNAVAILABLE" {
        let detail = error.capability_execution_failure()
            .map(|failure| failure.message.as_str()).unwrap_or_default();
        let guidance = if detail.contains("process spawn failed") {
            "Process launch failed. The command field must contain only the executable; put options in args. Example: {\"command\":\"bun\",\"args\":[\"test\",\"tests/example.test.js\"]}. Do not send the whole command line as command. Verify the executable is available. No successful launch was reported; change the request before retrying."
        } else if detail.contains("invalid working directory") || detail.contains("cwd must be") {
            "Process cwd is unavailable. Use a normalized workspace-relative directory, then retry with a new call."
        } else if detail.contains("process controls cannot change launch parameters") {
            "A process control call cannot change command, args, cwd or env. Use the existing process_id only."
        } else {
            "The process owner could not complete this call. Its effects may be uncertain; inspect current state before any changed retry."
        };
        return EngineToolError::CapabilityKernel {
            code: "CAPABILITY_UNAVAILABLE".into(),
            message: guidance.into(),
        };
    }
    kernel_error(error)
}

/// Return bounded repair guidance, never raw host paths or file contents. Patch
/// publication indices are safe structured owner observations and must survive
/// the Kernel's otherwise deliberately opaque capability failure projection.
fn workspace_write_feedback(detail: &str) -> String {
    let report = serde_json::from_str::<serde_json::Value>(detail).ok();
    let cause = report.as_ref().and_then(|value| value["cause"].as_str()).unwrap_or(detail);
    let guidance = if cause.contains("parent") || cause.contains("ancestor") {
        "Workspace write parent is unavailable or is not a directory. Missing workspace directories are created automatically. Inspect the relative path and existing ancestors for a file/directory conflict or unavailable link; do not retry the same payload or use a shell mkdir workaround."
    } else if cause.contains("expected_source") || cause.contains("precondition") || cause.contains("changed") {
        "Patch source changed or its source precondition did not match. Re-read every target and replan using the fresh source digest or observed absence. Do not remove the source guard to force the patch."
    } else if cause.contains("hunk") || cause.contains("line count") || cause.contains("mismatch") {
        "Patch hunk does not match its declared ranges or the current source. old_lines must equal context plus remove lines; new_lines must equal context plus add lines. Re-read the targets, correct the line counts and exact text, then replan before retrying."
    } else {
        "Workspace file operation failed. Inspect the target with read_file (missing_ok=true for a possibly absent file), check the workspace-relative path and payload, and replan before a changed retry. Failure alone does not prove that no file changed."
    };
    if let Some(report) = report.filter(|value| value["kind"] == "workspace_patch_failed" && value["version"] == 1) {
        let mut observation = serde_json::Map::new();
        let source = &report["observation"];
        if source["failed_file"].is_null() || source["failed_file"].as_u64().is_some_and(|index| index < 64) {
            observation.insert("failed_file".into(), source["failed_file"].clone());
        }
        for name in ["published", "restored", "restore_published_unconfirmed", "retained_created",
            "skipped_changed_or_unreadable", "rollback_failed", "temporary_cleanup_unconfirmed"] {
            let Some(indices) = source[name].as_array().filter(|indices| indices.len() <= 64
                && indices.iter().all(|index| index.as_u64().is_some_and(|index| index < 64))) else {
                return guidance.into();
            };
            observation.insert(name.into(), serde_json::Value::Array(indices.clone()));
        }
        return serde_json::json!({
            "kind":"workspace_patch_failed", "version":1,
            "journal_settlement": if report["journal_settlement"] == "settled" { "settled" } else { "unconfirmed" },
            "observation": observation, "recovery": guidance,
        }).to_string();
    }
    guidance.into()
}

fn validate_snapshot_registry(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
) -> Result<(), EngineToolError> {
    if active.resolved_snapshot_ref != *snapshot.snapshot_ref() {
        return Err(EngineToolError::ToolPlan(
            "active capability set belongs to a different Snapshot".to_owned(),
        ));
    }
    if !active
        .active
        .is_subset(&snapshot.content().capability_allowlist)
    {
        return Err(EngineToolError::ToolPlan(
            "active capability set exceeds the Snapshot's frozen ceiling".to_owned(),
        ));
    }
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(EngineToolError::ToolPlan(
            "materialized capability registry differs from the compiled Snapshot".to_owned(),
        ));
    }
    Ok(())
}

fn validate_binding_contract(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    binding: &EngineToolBinding,
    active_set_generation: u64,
) -> Result<(), EngineToolError> {
    binding.validate()?;
    if active.generation != active_set_generation || !active.active.contains(&binding.capability_id)
    {
        return Err(EngineToolError::ToolPlan(format!(
            "capability {} is not active in generation {}",
            binding.capability_id.as_ref(),
            active_set_generation
        )));
    }
    let resolved = snapshot.resolved_capability(&binding.capability_id).ok_or_else(|| {
        EngineToolError::ToolPlan(format!(
            "capability {} is outside the compiled Snapshot",
            binding.capability_id.as_ref()
        ))
    })?;
    if resolved.schema_digest != binding.capability_contract_digest {
        return Err(EngineToolError::ToolPlan(format!(
            "capability {} digest differs from the Tool binding",
            binding.capability_id.as_ref()
        )));
    }
    let materialized = registry.capability(&binding.capability_id).ok_or_else(|| {
        EngineToolError::ToolPlan(format!(
            "capability {} is not materialized",
            binding.capability_id.as_ref()
        ))
    })?;
    if materialized.schema_digest != binding.capability_contract_digest {
        return Err(EngineToolError::ToolPlan(format!(
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
            EngineToolError::ToolPlan(format!(
                "action {} is not materialized for capability {}",
                binding.action_id.as_ref(),
                binding.capability_id.as_ref()
            ))
        })?;
    if action.input_schema != binding.canonical_input_schema_ref {
        return Err(EngineToolError::ToolPlan(format!(
            "action {} canonical input schema differs from the Tool binding",
            binding.action_id.as_ref()
        )));
    }
    if matches!(
        action.presentation,
        ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
    ) || engine_effect(action.effect_class) != (binding.effect_class, binding.parallel_safe)
    {
        return Err(EngineToolError::ToolPlan(format!(
            "action {} presentation or effect classification differs from the canonical action",
            binding.action_id.as_ref()
        )));
    }
    let policy = snapshot.policy(&binding.capability_id).ok_or_else(|| {
        EngineToolError::ToolPlan(format!(
            "capability {} has no compiled authority policy",
            binding.capability_id.as_ref()
        ))
    })?;
    if !policy.allowed_actions.contains(&binding.action_id)
        || policy.resource_binding_ids != binding.resource_binding_ids
    {
        return Err(EngineToolError::ToolPlan(format!(
            "capability {} authority projection differs from the Tool binding",
            binding.capability_id.as_ref()
        )));
    }
    Ok(())
}

fn engine_effect(effect: EffectClass) -> (EngineEffectClass, bool) {
    match effect {
        EffectClass::Pure | EffectClass::ReadLocal => (EngineEffectClass::ReadOnly, true),
        EffectClass::ReadSensitive => (EngineEffectClass::ReadOnly, false),
        EffectClass::WriteReversible
        | EffectClass::WriteDurable
        | EffectClass::ExecuteLocal
        | EffectClass::Destructive => (EngineEffectClass::ManagedEffect, false),
        EffectClass::ExternalTransmit | EffectClass::Irreversible | EffectClass::Physical => {
            (EngineEffectClass::ExternalUncertainEffect, false)
        }
    }
}

#[cfg(test)]
#[path = "kernel_tests.rs"]
mod kernel_tests;
