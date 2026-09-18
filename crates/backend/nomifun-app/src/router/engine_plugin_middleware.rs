//! Hidden Plugin Product middleware on the source-integrated Engine path.
//!
//! Middleware is selected from the immutable Snapshot, ordered by the frozen
//! middleware list, executed through the same Plugin Product owner/receipt
//! path as ordinary actions, and never exposed as a model-callable tool.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, OperationId, PrincipalRef, ResolvedCapability, StrictJsonValue,
};
use nomifun_agent_contracts::chat_model::{ChatContentPart, ChatModelRequest, ChatRole};
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_ai_agent::context_contributor::TurnContext;
use nomifun_ai_agent::runtime_model_middleware_contract::{
    MAX_INPUT_BYTES as MAX_MODEL_INPUT_BYTES, MAX_PATCH_BYTES, ModelRequestPatch,
};
use nomifun_ai_agent::runtime_tool_middleware_contract::{
    BeforeToolDecision, MAX_INPUT_BYTES as MAX_TOOL_INPUT_BYTES,
};
use nomifun_chat_model_broker::{
    ChatModelError, ChatModelErrorCode, ChatRetryDirective, EngineModelPort, EngineModelStream,
};
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineToolError, EngineToolInvocation, EngineToolInvoker, EngineToolResult,
};
use nomifun_plugin_platform::runtime::PluginRuntimeCallCancellation;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::engine_plugin_product_tools::{PluginProductCallError, PluginProductOwner};

const MIDDLEWARE_DEADLINE: Duration = Duration::from_secs(5);

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine Plugin Product middleware: {message}"))
}

fn call_error(error: PluginProductCallError) -> String {
    match error {
        PluginProductCallError::Rejected(message) => message,
        PluginProductCallError::Unknown(message) => message,
    }
}

fn selected(
    snapshot: &CompiledSnapshot,
    action_id: &str,
) -> Result<Vec<ResolvedCapability>, AppError> {
    let action_id = ActionId::from(action_id);
    let positions = snapshot
        .content()
        .middleware_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut selected = snapshot
        .content()
        .contributions()
        .filter(|capability| {
            capability
                .actions
                .iter()
                .any(|action| action.action_id == action_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    for capability in &selected {
        super::engine_plugin_product_tools::validate_capability(capability)?;
        if capability.contribution_lock.source_kind
            != nomifun_agent_contracts::ContributionSourceKind::PluginProductActiveRelease
            || capability.actions.len() != 1
            || capability.actions[0].action_id != action_id
            || !capability.action_allowlist.contains(&action_id)
            || !capability.required_resource_kinds.is_empty()
        {
            return Err(failure(format!(
                "{} does not freeze the exact hidden Action authority",
                capability.capability.id.as_ref()
            )));
        }
    }
    selected.sort_by_key(|capability| {
        (
            positions
                .get(&capability.capability.id)
                .copied()
                .unwrap_or(usize::MAX),
            capability.capability.id.clone(),
        )
    });
    if selected.len() > 32 {
        return Err(failure("middleware chain exceeds 32 entries"));
    }
    Ok(selected)
}

fn require_active(
    active: &SessionCapabilityState,
    snapshot: &CompiledSnapshot,
    capabilities: &[ResolvedCapability],
    generation: Option<u64>,
) -> Result<(), String> {
    let active = active
        .snapshot()
        .map_err(|_| "active capability state is unavailable".to_owned())?;
    if active.resolved_snapshot_ref != *snapshot.snapshot_ref()
        || generation.is_some_and(|generation| active.generation != generation)
        || capabilities
            .iter()
            .any(|capability| !active.active.contains(&capability.capability.id))
    {
        return Err("middleware authority differs from the active Session generation".into());
    }
    Ok(())
}

pub(crate) fn model_port(
    inner: Arc<dyn EngineModelPort>,
    owner: PluginProductOwner,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
) -> Result<Arc<dyn EngineModelPort>, AppError> {
    let middleware = selected(
        &snapshot,
        nomifun_agent_contracts::model_middleware::ACTION_ID,
    )?;
    if middleware.is_empty() {
        return Ok(inner);
    }
    Ok(Arc::new(ModelMiddlewarePort {
        inner,
        owner,
        snapshot,
        active,
        principal,
        session,
        middleware,
    }))
}

struct ModelMiddlewarePort {
    inner: Arc<dyn EngineModelPort>,
    owner: PluginProductOwner,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
    middleware: Vec<ResolvedCapability>,
}

fn model_error(message: impl Into<String>) -> ChatModelError {
    ChatModelError::new(
        ChatModelErrorCode::ProtocolViolation,
        message,
        ChatRetryDirective::Never,
    )
}

fn turn_context(request: &ChatModelRequest) -> TurnContext {
    let mut text = Vec::new();
    let mut image_media_types = Vec::new();
    if let Some(message) = request
        .input
        .messages
        .iter()
        .rev()
        .find(|message| message.role == ChatRole::User)
    {
        for part in &message.content {
            match part {
                ChatContentPart::Text { text: value } => text.push(value.as_str()),
                ChatContentPart::Image { media_type, .. } => {
                    image_media_types.push(media_type.clone());
                }
                _ => {}
            }
        }
    }
    TurnContext {
        turn_id: request.causality.turn_operation_id.as_ref().to_owned(),
        source_message_id: request.causality.causation_event_id.as_ref().to_owned(),
        text: text.join("\n"),
        image_media_types,
        cs_dialogue_id: request.input.metadata.get("cs_dialogue_id").cloned(),
    }
}

#[async_trait]
impl EngineModelPort for ModelMiddlewarePort {
    async fn open_stream(
        &self,
        mut request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineModelStream, ChatModelError> {
        request.validate().map_err(|error| model_error(error.to_string()))?;
        if cancellation.is_cancelled() {
            return Err(model_error("before_model was cancelled"));
        }
        if request.causality.agent_session_id != self.session
            || self.principal.principal_kind != "user"
            || request.causality.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
        {
            return Err(model_error("before_model Session authority mismatch"));
        }
        require_active(&self.active, &self.snapshot, &self.middleware, None)
            .map_err(model_error)?;
        self.owner
            .receipts
            .ensure_settled(&self.principal.principal_id, self.session.as_ref())
            .await
            .map_err(|_| model_error("before_model has an unsettled hosted effect"))?;

        let deadline = tokio::time::Instant::now() + MIDDLEWARE_DEADLINE;
        let turn = turn_context(&request);
        let mut system = request.input.instructions.join("\n");
        let mut tools = request.input.tools.clone();
        for capability in &self.middleware {
            let payload = json!({
                "phase": "before_model",
                "turn": &turn,
                "system": &system,
                "tools": tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                })).collect::<Vec<_>>(),
            });
            if nomifun_agent_contracts::canonical_json_bytes(&payload)
                .map_err(|error| model_error(error.to_string()))?
                .len()
                > MAX_MODEL_INPUT_BYTES
            {
                return Err(model_error("before_model input exceeds 256 KiB"));
            }
            let operation = OperationId::from(format!(
                "{}:before-model:{}",
                request.causality.operation_id.as_ref(),
                capability.capability.id.as_ref()
            ));
            let plugin_cancellation = PluginRuntimeCallCancellation::default();
            let cancellation_bridge = {
                let caller = cancellation.clone();
                let plugin = plugin_cancellation.clone();
                tokio::spawn(async move {
                    caller.cancelled().await;
                    plugin.cancel();
                })
            };
            let output = self
                .owner
                .invoke_hidden_action(
                    &self.principal.principal_id,
                    self.session.as_ref(),
                    &request.causality.turn_operation_id,
                    deadline,
                    capability,
                    &ActionId::from(nomifun_agent_contracts::model_middleware::ACTION_ID),
                    operation,
                    StrictJsonValue(payload),
                    plugin_cancellation,
                )
                .await;
            cancellation_bridge.abort();
            let output = output
            .map_err(|error| model_error(format!("before_model failed: {}", call_error(error))))?;
            let bytes = nomifun_agent_contracts::canonical_json_bytes(&output.0)
                .map_err(|error| model_error(error.to_string()))?;
            if bytes.len() > MAX_PATCH_BYTES {
                return Err(model_error("before_model patch exceeds 64 KiB"));
            }
            let patch: ModelRequestPatch = serde_json::from_value(output.0)
                .map_err(|_| model_error("before_model returned an invalid patch"))?;
            if let Some(names) = patch.tool_names {
                let mut seen = BTreeSet::new();
                let mut selected = Vec::with_capacity(names.len());
                for name in names {
                    if !seen.insert(name.clone()) {
                        return Err(model_error("before_model returned duplicate tool names"));
                    }
                    selected.push(
                        tools
                            .iter()
                            .find(|tool| tool.name == name)
                            .cloned()
                            .ok_or_else(|| {
                                model_error("before_model returned a tool outside the request")
                            })?,
                    );
                }
                tools = selected;
            }
            if let Some(replacement) = patch.system {
                system = replacement;
            }
        }
        request.input.instructions = vec![system];
        request.input.tools = tools;
        if request.input.tools.is_empty() {
            request.input.tool_choice = nomifun_chat_model_broker::ChatToolChoice::None;
        }
        request.validate().map_err(|error| model_error(error.to_string()))?;
        self.inner.open_stream(request, cancellation).await
    }
}

pub(crate) fn tool_invoker(
    inner: Arc<dyn EngineToolInvoker>,
    owner: PluginProductOwner,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
) -> Result<Arc<dyn EngineToolInvoker>, AppError> {
    let middleware = selected(
        &snapshot,
        nomifun_agent_contracts::tool_middleware::BEFORE_ACTION_ID,
    )?;
    if middleware.is_empty() {
        return Ok(inner);
    }
    Ok(Arc::new(ToolMiddlewareInvoker {
        inner,
        owner,
        snapshot,
        active,
        principal,
        session,
        middleware,
    }))
}

struct ToolMiddlewareInvoker {
    inner: Arc<dyn EngineToolInvoker>,
    owner: PluginProductOwner,
    snapshot: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    principal: PrincipalRef,
    session: AgentSessionId,
    middleware: Vec<ResolvedCapability>,
}

fn tool_middleware_failure(message: impl Into<String>) -> EngineToolError {
    EngineToolError::InvalidContract(format!("before_tool failed: {}", message.into()))
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "password"
            | "passwd"
            | "secret"
            | "clientsecret"
            | "apikey"
            | "accesskey"
            | "accesskeyid"
            | "secretaccesskey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "privatekey"
            | "credentials"
    ) || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("token")
        || normalized.ends_with("apikey")
}

fn redact(value: &mut Value, redacted: &mut bool) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive_key(key) {
                    *value = Value::String("[REDACTED]".into());
                    *redacted = true;
                } else {
                    redact(value, redacted);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact(value, redacted);
            }
        }
        Value::String(text) => {
            let safe = nomi_redact::redact_secrets_owned(text.clone());
            *redacted |= safe != *text;
            *text = safe;
        }
        _ => {}
    }
}

#[async_trait]
impl EngineToolInvoker for ToolMiddlewareInvoker {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        invocation.binding.validate()?;
        invocation
            .call
            .validate()
            .map_err(|error| tool_middleware_failure(error.to_string()))?;
        nomifun_engine_core::parse_completed_arguments(&invocation.call)?;
        let validator = jsonschema::validator_for(&invocation.binding.definition.input_schema.0)
            .map_err(|_| tool_middleware_failure("target input schema is invalid"))?;
        if !validator.is_valid(&invocation.call.arguments.0) {
            return Err(EngineToolError::ToolInvocation(
                "JSON Schema validation failed; target tool was not executed and before_tool was not run"
                    .into(),
            ));
        }
        if invocation.principal != self.principal
            || invocation.agent_session_id != self.session
            || invocation.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
        {
            return Err(tool_middleware_failure("Session authority mismatch"));
        }
        require_active(
            &self.active,
            &self.snapshot,
            &self.middleware,
            Some(invocation.active_set_generation),
        )
        .map_err(tool_middleware_failure)?;
        self.owner
            .receipts
            .ensure_settled(&self.principal.principal_id, self.session.as_ref())
            .await
            .map_err(|_| tool_middleware_failure("HOSTED_EFFECT_UNPROVEN"))?;
        let mut arguments = invocation.call.arguments.0.clone();
        let raw_input = json!({
            "phase": "before_tool",
            "invocation_id": invocation.operation_id.as_ref(),
            "tool_call_id": invocation.call.call_id.as_ref(),
            "tool_name": invocation.call.name,
            "arguments": &arguments,
            "redacted": false,
        });
        if nomifun_agent_contracts::canonical_json_bytes(&raw_input)
            .map_err(|error| tool_middleware_failure(error.to_string()))?
            .len()
            > MAX_TOOL_INPUT_BYTES
        {
            return Err(tool_middleware_failure(
                "input exceeds 256 KiB; target tool was not executed",
            ));
        }
        let mut was_redacted = false;
        redact(&mut arguments, &mut was_redacted);
        let deadline = tokio::time::Instant::now() + MIDDLEWARE_DEADLINE;
        for capability in &self.middleware {
            let payload = json!({
                "phase": "before_tool",
                "invocation_id": invocation.operation_id.as_ref(),
                "tool_call_id": invocation.call.call_id.as_ref(),
                "tool_name": invocation.call.name,
                "arguments": &arguments,
                "redacted": was_redacted,
            });
            if nomifun_agent_contracts::canonical_json_bytes(&payload)
                .map_err(|error| tool_middleware_failure(error.to_string()))?
                .len()
                > MAX_TOOL_INPUT_BYTES
            {
                return Err(tool_middleware_failure(
                    "redacted input exceeds 256 KiB",
                ));
            }
            let operation = OperationId::from(format!(
                "{}:before-tool:{}",
                invocation.operation_id.as_ref(),
                capability.capability.id.as_ref()
            ));
            let plugin_cancellation = PluginRuntimeCallCancellation::default();
            let cancellation_bridge = {
                let caller = cancellation.clone();
                let plugin = plugin_cancellation.clone();
                tokio::spawn(async move {
                    caller.cancelled().await;
                    plugin.cancel();
                })
            };
            let output = self
                .owner
                .invoke_hidden_action(
                    &self.principal.principal_id,
                    self.session.as_ref(),
                    &invocation.turn_operation_id,
                    deadline,
                    capability,
                    &ActionId::from(
                        nomifun_agent_contracts::tool_middleware::BEFORE_ACTION_ID,
                    ),
                    operation,
                    StrictJsonValue(payload),
                    plugin_cancellation,
                )
                .await;
            cancellation_bridge.abort();
            let output = output.map_err(|error| {
                tool_middleware_failure(format!(
                    "service failed: {}; target tool was not executed",
                    call_error(error)
                ))
            })?;
            let decision: BeforeToolDecision = serde_json::from_value(output.0).map_err(|_| {
                tool_middleware_failure(
                    "returned an invalid decision; target tool was not executed",
                )
            })?;
            decision.validate().map_err(tool_middleware_failure)?;
            if let BeforeToolDecision::Deny { reason } = decision {
                return Err(EngineToolError::ToolInvocation(format!(
                    "before_tool denied target tool: {reason}"
                )));
            }
        }
        self.inner.invoke(invocation, cancellation).await
    }
}
