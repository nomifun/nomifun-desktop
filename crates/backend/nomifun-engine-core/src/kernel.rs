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
        cancellation: CancellationToken,
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
        if let Some(result) = process_host_os_mapping_error(
            is_process,
            invocation.binding.action_id.as_ref(),
            &invocation.binding.definition,
            &invocation.call.call_id,
        ) {
            // This check deliberately follows full Snapshot/binding validation
            // but precedes registry/owner invocation. A stale command mapping
            // is an observed non-dispatch, never a reason to try both OSes.
            return Ok(result);
        }
        let process_action_id = invocation.binding.action_id.clone();
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
            .invoke_controlled(
                &self.snapshot,
                &active,
                request,
                nomifun_agent_kernel::InvocationControl::new(cancellation),
            )
            .await
            .map_err(|error| {
                kernel_error_for_action(error, is_process, is_file_write)
            })?;
        Ok(EngineToolResult::text(
            invocation.call.call_id,
            serde_json::to_string(&output.0).map_err(|error| {
                EngineToolError::ToolInvocation(format!(
                    "Kernel result could not be serialized: {error}"
                ))
            })?,
            // A successfully dispatched command is not necessarily a
            // successful command. Preserve nonzero exit as model feedback.
            process_result_is_error(is_process, process_action_id.as_ref(), &output.0),
        ))
    }
}

fn is_workspace_process_action(capability_id: &CapabilityId, action_id: &ActionId) -> bool {
    capability_id.as_ref() == nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID
        && nomifun_agent_domain_wave2::WORKSPACE_PROCESS_ACTION_IDS
            .contains(&action_id.as_ref())
}

fn process_result_is_error(
    is_process: bool,
    action_id: &str,
    output: &serde_json::Value,
) -> bool {
    if !is_process || output.get("state").and_then(serde_json::Value::as_str) == Some("running") {
        return false;
    }
    // `success=false` on a Cancelled process describes user-code exit, not
    // whether the cancel/control observation succeeded. Exact owner proof that
    // the tree was reaped fulfills `cancel_process` and a later `poll_process`.
    if matches!(action_id, "workspace.process/cancel" | "workspace.process/poll")
        && output.get("state").and_then(serde_json::Value::as_str) == Some("cancelled")
        && output
            .pointer("/cleanup/reaped")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return false;
    }
    output.get("success").and_then(serde_json::Value::as_bool) != Some(true)
}

const PROCESS_HOST_OS_DESCRIPTION_PREFIX: &str = "Workspace process host OS: ";

fn process_host_os_mapping_error(
    is_process: bool,
    action_id: &str,
    definition: &ChatToolDefinition,
    call_id: &nomifun_chat_model_broker::ToolCallId,
) -> Option<EngineToolResult> {
    if !is_process
        || !matches!(
            action_id,
            "workspace.process/start"
        )
    {
        return None;
    }
    let advertised = definition
        .description
        .strip_prefix(PROCESS_HOST_OS_DESCRIPTION_PREFIX)?
        .split_once('.')?
        .0;
    let actual = std::env::consts::OS;
    if advertised == actual {
        return None;
    }
    Some(EngineToolResult::text(
        call_id.clone(),
        serde_json::json!({
            "schema":"nomifun.process-start-observation.v1",
            "status":"not_executed",
            "state":"not_started",
            "code":"HOST_OS_COMMAND_MAPPING_ERROR",
            "advertised_host_os":advertised,
            "actual_host_os":actual,
            "user_code_started":false,
            "success":false,
            "message":"The process tool's advertised host OS differs from its executing owner. No owner dispatch occurred. Do not try another platform command; refresh the Session/tool surface."
        })
        .to_string(),
        true,
    ))
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
        self.invoke_kernel(invocation, cancellation).await
    }
}

fn kernel_error(error: KernelError) -> EngineToolError {
    let code = error.canonical_code().0;
    if code == "INVOCATION_CANCELLED" {
        return EngineToolError::Cancelled;
    }
    let message = match code.as_str() {
        "AGENT_EXECUTION_ALREADY_ACTIVE" => "This conversation already has an active AgentExecution. Preserve its exact execution identity. Put independent child work in one strategy=parallel tasks array, then use agent_wait and agent_status to join the accepted work and integrate it in the parent; do not submit accepted work again.".to_owned(),
        _ => error.capability_execution_failure().map(|failure| owner_failure_detail(&failure.message, 4096))
            .unwrap_or_else(|| error.to_string()),
    };
    EngineToolError::CapabilityKernel {
        code,
        message,
        details: error.capability_execution_failure().and_then(|failure| owner_failure_details(failure.details.as_ref())),
    }
}

/// Preserve the admitted owner's real failure and typed code. Credentials are
/// redacted with the existing shared sanitizer; local paths and OS causes are
/// necessary task evidence. A launch failure does not prove malformed argv,
/// administrator identity, an absent privacy grant, or that nothing changed.
fn kernel_error_for_action(
    error: KernelError,
    is_process: bool,
    is_file_write: bool,
) -> EngineToolError {
    if let Some(failure) = error.capability_execution_failure()
        && let Some(message) = settlement_loss_feedback(&failure.message)
    {
        return EngineToolError::CapabilityKernel {
            code: error.canonical_code().0,
            message: message.to_owned(),
            details: owner_failure_details(failure.details.as_ref()),
        };
    }
    if is_file_write && let Some(failure) = error.capability_execution_failure() {
        return EngineToolError::CapabilityKernel {
            code: error.canonical_code().0,
            message: workspace_write_feedback(&failure.message),
            details: owner_failure_details(failure.details.as_ref()),
        };
    }
    if is_process && let Some(failure) = error.capability_execution_failure() {
        return EngineToolError::CapabilityKernel {
            code: error.canonical_code().0,
            message: owner_failure_detail(&failure.message, 4096),
            details: owner_failure_details(failure.details.as_ref()),
        };
    }
    kernel_error(error)
}

fn owner_failure_detail(detail: &str, limit: usize) -> String {
    let redacted = nomi_redact::redact_secrets(detail);
    if redacted.len() <= limit { return redacted.into_owned(); }
    let suffix = "...[truncated]";
    let mut boundary = limit.saturating_sub(suffix.len()).min(redacted.len());
    while !redacted.is_char_boundary(boundary) { boundary -= 1; }
    format!("{}{suffix}", &redacted[..boundary])
}

/// Project only the native acquisition diagnostic carried by the admitted
/// owner. These fields are never inferred from the human-readable message.
/// Unknown diagnostic shapes do not change the original failed invocation.
fn owner_failure_details(details: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    const MAX_DETAILS_BYTES: usize = 8 * 1024;
    let details = details?.as_object()?;
    if let Some(system) = details.get("system_execution_error") {
        let mut failure: nomifun_agent_contracts::SystemExecutionFailure = serde_json::from_value(system.clone()).ok()?;
        failure.message = owner_failure_detail(&failure.message, 1024);
        if !failure.validate() { return None; }
        let projected = serde_json::json!({"system_execution_error": failure});
        return (serde_json::to_vec(&projected).ok()?.len() <= MAX_DETAILS_BYTES).then_some(projected);
    }
    let native = details.get("native_file_error")?.as_object()?;
    let code = native.get("code")?.as_str()?;
    if !matches!(code, "INVALID_PAYLOAD" | "FILE_CHANGED_DURING_READ"
        | "NATIVE_FILE_ACQUIRE_UNAVAILABLE" | "SYSTEM_ACCESS_DENIED"
        | "RESOURCE_NOT_FOUND" | "NATIVE_FILE_READ_FAILED") { return None; }
    let message = native.get("message")?.as_str()?;
    let kind = native.get("kind")?;
    if !kind.is_null() && !kind.as_str().is_some_and(|value|
        !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_alphabetic())) {
        return None;
    }
    let raw_os_error = native.get("raw_os_error")?;
    if !raw_os_error.is_null() && !raw_os_error.as_i64().is_some_and(|value| i32::try_from(value).is_ok()) {
        return None;
    }
    let mut projected = serde_json::json!({"native_file_error": {
        "code": code,
        "message": owner_failure_detail(message, 2048),
        "kind": kind,
        "raw_os_error": raw_os_error,
    }});
    // JSON control-character escaping can expand a 2 KiB message sixfold.
    // A 1 KiB UTF-8 message plus these fixed, bounded fields fits below 8 KiB.
    if serde_json::to_vec(&projected).ok()?.len() > MAX_DETAILS_BYTES {
        projected["native_file_error"]["message"] = owner_failure_detail(message, 1024).into();
    }
    (serde_json::to_vec(&projected).ok()?.len() <= MAX_DETAILS_BYTES).then_some(projected)
}

fn settlement_loss_feedback(detail: &str) -> Option<&'static str> {
    if detail.contains("owner reported success")
        && detail.contains("canonical terminal observation could not be committed")
    {
        return Some("The workspace owner reported success, but the canonical receipt was not stored. The requested effect may already be present. Do not retry automatically or report that nothing changed. Re-read the affected owner state and reconcile the pending effect before continuing.");
    }
    if detail.contains("owner failed with")
        && detail.contains("canonical failure observation could not be committed")
    {
        return Some("The owner reported failure, but the canonical failure receipt was not stored. The effect remains pending because recovery cannot replay the failure safely. Do not retry automatically. Inspect current owner state and reconcile the pending effect before a changed request.");
    }
    if detail.contains("owner outcome is unknown")
        && detail.contains("canonical uncertain observation could not be committed")
    {
        return Some("The owner outcome is unknown, and the canonical uncertainty receipt was not stored. The effect remains pending. Do not retry automatically. Reconcile the external owner and re-read affected state before continuing.");
    }
    None
}

/// Keep the concrete file failure, exact partial-publication observations and
/// bounded repair guidance together. The original owner receipt retains the
/// full result; model feedback redacts credentials without hiding OS causes.
fn workspace_write_feedback(detail: &str) -> String {
    let report = serde_json::from_str::<serde_json::Value>(detail).ok();
    let cause = report.as_ref().and_then(|value| value["cause"].as_str()).unwrap_or(detail);
    let guidance = if cause.contains("parent") || cause.contains("ancestor") {
        "Local file write parent is unavailable or is not a directory. Missing parent directories are created automatically. Inspect the exact native absolute or Session-default-directory relative path and existing ancestors for a file/directory conflict or unavailable link; do not retry the same payload blindly."
    } else if cause.contains("expected_source") || cause.contains("precondition") || cause.contains("changed") {
        "Patch source changed or its source precondition did not match. Re-read every target and replan using the fresh source digest or observed absence. Do not remove the source guard to force the patch."
    } else if cause.contains("hunk") || cause.contains("line count") || cause.contains("mismatch") {
        "Patch hunk does not match its declared ranges or the current source. old_lines must equal context plus remove lines; new_lines must equal context plus add lines. Re-read the targets, correct the line counts and exact text, then replan before retrying."
    } else {
        "Local file operation failed. Inspect the exact native absolute or Session-default-directory relative target with read_file (missing_ok=true for genuine absence) and replan from the actual cause. A system access denial, independent privacy grant and unavailable platform feature require their actual respective paths; administrator identity is not a universal remedy. Failure alone does not prove that no file changed."
    };
    if let Some(report) = report.as_ref().filter(|value| value["kind"] == "workspace_patch_failed" && value["version"] == 1) {
        let mut observation = serde_json::Map::new();
        let source = &report["observation"];
        if source["failed_file"].is_null() || source["failed_file"].as_u64().is_some_and(|index| index < 64) {
            observation.insert("failed_file".into(), source["failed_file"].clone());
        }
        for name in ["published", "restored", "restore_published_unconfirmed", "retained_created",
            "skipped_changed_or_unreadable", "rollback_failed", "temporary_cleanup_unconfirmed"] {
            let Some(indices) = source[name].as_array().filter(|indices| indices.len() <= 64
                && indices.iter().all(|index| index.as_u64().is_some_and(|index| index < 64))) else {
                return format!("{}\n{guidance}", owner_failure_detail(cause, 1024));
            };
            observation.insert(name.into(), serde_json::Value::Array(indices.clone()));
        }
        if let Some(value) = source.get("unverified_publications") {
            let Some(indices) = value.as_array().filter(|indices| indices.len() <= 64
                && indices.iter().all(|index| index.as_u64().is_some_and(|index| index < 64))) else {
                    return format!("{}\n{guidance}", owner_failure_detail(cause, 1024));
                };
            observation.insert("unverified_publications".into(), serde_json::Value::Array(indices.clone()));
        }
        let published_count = observation["published"].as_array().expect("validated indices").len();
        let restored_count = observation["restored"].as_array().expect("validated indices").len();
        let effect_warning = if published_count > 0 {
            "Files were published before this error. Do not report that no files changed. "
        } else { "" };
        let mut feedback = serde_json::json!({
            "kind":"workspace_patch_failed", "version":1,
            "cause":owner_failure_detail(cause, 768),
            "journal_settlement": if report["journal_settlement"] == "settled" { "settled" } else { "unconfirmed" },
            "index_base":0, "observed_published_count":published_count, "confirmed_restored_count":restored_count,
            "observation": observation,
            "recovery":format!("{effect_warning}{guidance} Indices are positions in request.files; observations are historical."),
        });
        if feedback.to_string().len() > 2048 {
            // Keep every bounded index even for the largest owner report.
            feedback["recovery"] = serde_json::json!(format!("{effect_warning}Indices refer to request.files. Re-read current state; do not retry unchanged."));
        }
        let mut cause_limit = 768;
        while feedback.to_string().len() > 2048 && cause_limit > 16 {
            cause_limit /= 2;
            feedback["cause"] = serde_json::json!(owner_failure_detail(cause, cause_limit));
        }
        return feedback.to_string();
    }
    format!("{}\n{guidance}", owner_failure_detail(cause, 1024))
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
