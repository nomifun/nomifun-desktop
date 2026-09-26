//! Unified Plugin Action/Binding adapters for the production Agent engine.
//!
//! This is deliberately a consumer-side projection. Plugin authors publish
//! only inline-schema Actions and Bindings; the Agent engine receives its own
//! frozen tool, context and hook contracts without making Plugin Core depend
//! on the Agent Package/Role/Provider graph.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, PluginActionEffect,
    StrictJsonValue, digest_payload,
};
use nomifun_ai_agent::context_contributor::TurnContext;
use nomifun_ai_agent::runtime_model_middleware_contract::{
    BeforeModelInput, MAX_INPUT_BYTES as MAX_MODEL_HOOK_INPUT_BYTES,
    MAX_PATCH_BYTES, ModelRequestPatch, ModelToolMetadata,
};
use nomifun_ai_agent::runtime_tool_middleware_contract::{
    BeforeToolDecision, BeforeToolInput, MAX_INPUT_BYTES as MAX_TOOL_HOOK_INPUT_BYTES,
    decode_decision,
};
use nomifun_chat_model_broker::{
    ChatContentPart, ChatModelError, ChatModelErrorCode, ChatModelRequest, ChatRetryDirective,
    ChatRole, ChatToolChoice, EngineModelPort, EngineModelStream,
};
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineEffectClass, EngineToolBinding, EngineToolError, EngineToolInvocation,
    EngineToolInvoker, EngineToolPlan, EngineToolResult, input_schema_digest,
};
use nomifun_plugin_platform::{
    AgentPluginBindings, BoundPluginAction, PluginActionAvailability, PluginBindingError,
    PluginCancellation, PluginDispatchOptions,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const MAX_PLUGIN_CONTEXT_BYTES: usize = 64 * 1024;
const CANCEL_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Agent Plugin Binding: {message}"))
}

#[derive(Clone, Debug)]
struct FrozenAction {
    stable_action_id: String,
    artifact_digest: DigestHex,
    action_id: String,
    name: String,
    description: String,
    input_schema: StrictJsonValue,
    effect: PluginActionEffect,
}

impl FrozenAction {
    fn from_bound(action: BoundPluginAction) -> Option<Self> {
        if action.availability != PluginActionAvailability::Available {
            return None;
        }
        Some(Self {
            stable_action_id: action.stable_action_id,
            artifact_digest: action.publication.artifact.digest,
            action_id: action.publication.action_id,
            name: action.publication.action.name,
            description: action.publication.action.description,
            input_schema: action.publication.action.input,
            effect: action.publication.action.effect,
        })
    }

    fn options(&self, cancellation: PluginCancellation) -> PluginDispatchOptions {
        PluginDispatchOptions {
            expected_artifact_digest: Some(self.artifact_digest.clone()),
            cancellation,
            call_chain: Vec::new(),
        }
    }
}

/// One immutable Agent-session view of currently active Plugin Bindings.
///
/// Updating, disabling or deleting a Plugin does not mutate this view. Every
/// invocation is fenced against the live registry, so a stale Session gets an
/// explicit unavailable/artifact-changed result and must be reopened.
#[derive(Clone)]
pub(crate) struct FrozenAgentPluginBindings {
    bindings: AgentPluginBindings,
    tool_plan: EngineToolPlan,
    tools: Arc<BTreeMap<String, FrozenAction>>,
    contexts: Arc<[FrozenAction]>,
    before_model: Arc<[FrozenAction]>,
    before_tool: Arc<[FrozenAction]>,
}

impl FrozenAgentPluginBindings {
    pub(crate) fn has_unscoped_tool_hooks(&self) -> bool {
        self.before_tool.iter().any(|action| action.effect != PluginActionEffect::Read)
    }

    pub(crate) fn freeze(
        bindings: AgentPluginBindings,
        restricted: bool,
    ) -> Result<Self, AppError> {
        if restricted {
            return Ok(Self {
                bindings,
                tool_plan: EngineToolPlan::default(),
                tools: Arc::new(BTreeMap::new()),
                contexts: Arc::from(Vec::<FrozenAction>::new()),
                before_model: Arc::from(Vec::<FrozenAction>::new()),
                before_tool: Arc::from(Vec::<FrozenAction>::new()),
            });
        }

        let snapshot = bindings.snapshot().map_err(failure)?;
        let tool_actions = available(snapshot.tools);
        let context_actions = available(snapshot.contexts);
        let before_model = available(snapshot.before_model);
        let before_tool = available(snapshot.before_tool);

        let mut tools = BTreeMap::new();
        let mut plan = Vec::with_capacity(tool_actions.len());
        for action in tool_actions {
            let model_name = model_name(&action.stable_action_id);
            let schema_digest = input_schema_digest(&action.input_schema).map_err(failure)?;
            let contract_digest = digest_payload(&json!({
                "stable_action_id": action.stable_action_id,
                "artifact_digest": action.artifact_digest,
                "action_id": action.action_id,
                "effect": action.effect,
                "input_schema": action.input_schema,
            }))
            .map_err(failure)?;
            let schema_ref = CanonicalSchemaRef::from(format!(
                "schema://nomifun/unified-plugin/{}@1#{}",
                &model_name,
                schema_digest.as_ref(),
            ));
            let effect_class = match action.effect {
                PluginActionEffect::Read => EngineEffectClass::ReadOnly,
                PluginActionEffect::Write => EngineEffectClass::ManagedEffect,
                PluginActionEffect::External => EngineEffectClass::ExternalUncertainEffect,
            };
            let description = format!(
                "{}: {}\nUnified Plugin Action {}. The active Artifact and enabled state are rechecked at dispatch.",
                action.name, action.description, action.stable_action_id,
            );
            plan.push(EngineToolBinding {
                model_name: model_name.clone(),
                definition: nomifun_chat_model_broker::ChatToolDefinition {
                    name: model_name.clone(),
                    description,
                    input_schema: action.input_schema.clone(),
                    deferred: false,
                },
                schema_digest,
                canonical_input_schema_ref: schema_ref,
                capability_contract_digest: contract_digest,
                capability_id: CapabilityId::from(action.stable_action_id.clone()),
                action_id: ActionId::from(action.action_id.clone()),
                resource_binding_ids: BTreeSet::new(),
                effect_class,
                // A per-Plugin Service process and its storage are the
                // concurrency authority; the Agent never upgrades a read
                // declaration into parallel execution on its own.
                parallel_safe: false,
            });
            if tools.insert(model_name, action).is_some() {
                return Err(failure("duplicate model route for Plugin Action"));
            }
        }

        Ok(Self {
            bindings,
            tool_plan: EngineToolPlan::new(plan).map_err(failure)?,
            tools: Arc::new(tools),
            contexts: Arc::from(context_actions),
            before_model: Arc::from(before_model),
            before_tool: Arc::from(before_tool),
        })
    }

    pub(crate) fn tool_plan(&self) -> EngineToolPlan {
        self.tool_plan.clone()
    }

    pub(crate) fn contains_tool_binding(&self, binding: &EngineToolBinding) -> bool {
        self.tool_plan.binding(&binding.model_name) == Some(binding)
    }

    pub(crate) fn retain_active(
        &self,
        full: &EngineToolPlan,
        active: &BTreeSet<CapabilityId>,
    ) -> Result<EngineToolPlan, AppError> {
        EngineToolPlan::new(full.model_definitions().into_iter().filter_map(|definition| {
            let binding = full.binding(&definition.name)?;
            (active.contains(&binding.capability_id) || self.contains_tool_binding(binding))
                .then(|| binding.clone())
        }))
        .map_err(failure)
    }

    pub(crate) async fn context_for_turn(
        &self,
        turn: &TurnContext,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, AppError> {
        if self.contexts.is_empty() {
            return Ok(None);
        }
        let input = StrictJsonValue(json!({"phase": "context", "turn": turn}));
        let mut contributions = Vec::new();
        for action in self.contexts.iter() {
            let plugin_cancellation = PluginCancellation::new();
            let dispatch = self.bindings.invoke_context(
                &action.stable_action_id,
                input.clone(),
                action.options(plugin_cancellation.clone()),
            );
            match await_dispatch(dispatch, plugin_cancellation, cancellation.clone()).await {
                Ok(value) => contributions.push(json!({
                    "action": action.stable_action_id,
                    "value": value.0,
                })),
                Err(PluginBindingError::Canceled) if cancellation.is_cancelled() => {
                    return Err(failure("context contribution was canceled"));
                }
                Err(error) => {
                    // agent.context is explicitly continue-on-failure. Keep
                    // diagnostics free of Plugin/runtime payloads.
                    tracing::warn!(
                        action = %action.stable_action_id,
                        code = binding_error_code(&error),
                        "Unified Plugin context contribution was skipped"
                    );
                }
            }
        }
        if contributions.is_empty() {
            return Ok(None);
        }
        let encoded = serde_json::to_string(&contributions).map_err(failure)?;
        if encoded.len() > MAX_PLUGIN_CONTEXT_BYTES {
            return Err(failure("Plugin context exceeds the 64 KiB Agent budget"));
        }
        Ok(Some(format!(
            "<nomifun_plugin_context format=\"canonical-json\">\n{encoded}\n</nomifun_plugin_context>"
        )))
    }

    pub(crate) fn wrap_model(
        &self,
        inner: Arc<dyn EngineModelPort>,
    ) -> Arc<dyn EngineModelPort> {
        if self.before_model.is_empty() {
            inner
        } else {
            Arc::new(PluginBeforeModelPort {
                inner,
                bindings: self.bindings.clone(),
                actions: Arc::clone(&self.before_model),
            })
        }
    }

    pub(crate) fn wrap_tools(
        &self,
        inner: Arc<dyn EngineToolInvoker>,
    ) -> Arc<dyn EngineToolInvoker> {
        let inner: Arc<dyn EngineToolInvoker> = if self.tools.is_empty() {
            inner
        } else {
            Arc::new(PluginActionTools {
                inner,
                bindings: self.bindings.clone(),
                plan: self.tool_plan.clone(),
                tools: Arc::clone(&self.tools),
            })
        };
        if self.before_tool.is_empty() {
            inner
        } else {
            Arc::new(PluginBeforeToolGate {
                inner,
                bindings: self.bindings.clone(),
                actions: Arc::clone(&self.before_tool),
            })
        }
    }
}

fn available(actions: Vec<BoundPluginAction>) -> Vec<FrozenAction> {
    actions
        .into_iter()
        .filter_map(FrozenAction::from_bound)
        .collect()
}

fn model_name(stable_action_id: &str) -> String {
    let digest = Sha256::digest(stable_action_id.as_bytes());
    format!("plugin_{digest:x}")[..63].to_owned()
}

async fn await_dispatch<T>(
    future: impl std::future::Future<Output = Result<T, PluginBindingError>>,
    plugin_cancellation: PluginCancellation,
    cancellation: CancellationToken,
) -> Result<T, PluginBindingError> {
    tokio::pin!(future);
    tokio::select! {
        result = &mut future => result,
        () = cancellation.cancelled() => {
            plugin_cancellation.cancel();
            let _ = tokio::time::timeout(CANCEL_SETTLE_TIMEOUT, &mut future).await;
            Err(PluginBindingError::Canceled)
        }
    }
}

struct PluginActionTools {
    inner: Arc<dyn EngineToolInvoker>,
    bindings: AgentPluginBindings,
    plan: EngineToolPlan,
    tools: Arc<BTreeMap<String, FrozenAction>>,
}

#[async_trait]
impl EngineToolInvoker for PluginActionTools {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        let Some(action) = self.tools.get(&invocation.call.name) else {
            return self.inner.invoke(invocation, cancellation).await;
        };
        if self.plan.binding(&invocation.call.name) != Some(&invocation.binding) {
            return Err(EngineToolError::ToolInvocation(
                "Unified Plugin Action differs from its frozen Agent mapping".into(),
            ));
        }
        let plugin_cancellation = PluginCancellation::new();
        let dispatch = self.bindings.invoke_tool(
            &action.stable_action_id,
            invocation.call.arguments.clone(),
            action.options(plugin_cancellation.clone()),
        );
        let result = await_dispatch(dispatch, plugin_cancellation, cancellation).await;
        match result {
            Ok(value) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                serde_json::to_string(&value.0).map_err(|_| {
                    EngineToolError::ToolInvocation(
                        "Unified Plugin Action output could not be encoded".into(),
                    )
                })?,
                false,
            )),
            Err(PluginBindingError::Canceled) => Err(EngineToolError::Cancelled),
            Err(error) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                json!({
                    "code": binding_error_code(&error),
                    "message": binding_error_message(&error),
                    "retry_safe": matches!(error, PluginBindingError::Timeout),
                    "stable_action_id": action.stable_action_id,
                })
                .to_string(),
                true,
            )),
        }
    }
}

struct PluginBeforeToolGate {
    inner: Arc<dyn EngineToolInvoker>,
    bindings: AgentPluginBindings,
    actions: Arc<[FrozenAction]>,
}

#[async_trait]
impl EngineToolInvoker for PluginBeforeToolGate {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        let input = BeforeToolInput {
            phase: "before_tool",
            invocation_id: invocation.operation_id.as_ref().to_owned(),
            tool_call_id: invocation.call.call_id.as_ref().to_owned(),
            tool_name: invocation.call.name.clone(),
            arguments: invocation.call.arguments.0.clone(),
            redacted: false,
        };
        let value = serde_json::to_value(input).map_err(|_| {
            EngineToolError::ToolInvocation("before_tool input could not be encoded".into())
        })?;
        if serde_json::to_vec(&value).map_err(|_| {
            EngineToolError::ToolInvocation("before_tool input could not be measured".into())
        })?.len() > MAX_TOOL_HOOK_INPUT_BYTES {
            return Err(EngineToolError::ToolInvocation(
                "before_tool input exceeds 256 KiB".into(),
            ));
        }
        for action in self.actions.iter() {
            let plugin_cancellation = PluginCancellation::new();
            let dispatch = self.bindings.invoke_before_tool(
                &action.stable_action_id,
                StrictJsonValue(value.clone()),
                action.options(plugin_cancellation.clone()),
            );
            let output = await_dispatch(
                dispatch,
                plugin_cancellation,
                cancellation.clone(),
            )
            .await
            .map_err(|error| hook_tool_error("before_tool", action, &error))?;
            let bytes = serde_json::to_vec(&output.0).map_err(|_| {
                EngineToolError::ToolInvocation("before_tool output could not be encoded".into())
            })?;
            match decode_decision(&bytes).map_err(EngineToolError::ToolInvocation)? {
                BeforeToolDecision::Allow {} => {}
                BeforeToolDecision::Deny { reason } => {
                    return Err(EngineToolError::ToolInvocation(format!(
                        "before_tool denied {}: {reason}",
                        invocation.call.name,
                    )));
                }
            }
        }
        self.inner.invoke(invocation, cancellation).await
    }
}

struct PluginBeforeModelPort {
    inner: Arc<dyn EngineModelPort>,
    bindings: AgentPluginBindings,
    actions: Arc<[FrozenAction]>,
}

#[async_trait]
impl EngineModelPort for PluginBeforeModelPort {
    async fn open_stream(
        &self,
        mut request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineModelStream, ChatModelError> {
        for action in self.actions.iter() {
            let input = before_model_input(&request);
            let value = serde_json::to_value(input)
                .map_err(|_| hook_model_error("before_model input could not be encoded"))?;
            if serde_json::to_vec(&value)
                .map_err(|_| hook_model_error("before_model input could not be measured"))?
                .len()
                > MAX_MODEL_HOOK_INPUT_BYTES
            {
                return Err(hook_model_error("before_model input exceeds 256 KiB"));
            }
            let plugin_cancellation = PluginCancellation::new();
            let dispatch = self.bindings.invoke_before_model(
                &action.stable_action_id,
                StrictJsonValue(value),
                action.options(plugin_cancellation.clone()),
            );
            let output = await_dispatch(
                dispatch,
                plugin_cancellation,
                cancellation.clone(),
            )
            .await
            .map_err(|error| {
                hook_model_error(format!(
                    "before_model {} failed ({})",
                    action.stable_action_id,
                    binding_error_code(&error),
                ))
            })?;
            if serde_json::to_vec(&output.0)
                .map_err(|_| hook_model_error("before_model output could not be measured"))?
                .len()
                > MAX_PATCH_BYTES
            {
                return Err(hook_model_error("before_model patch exceeds 64 KiB"));
            }
            let patch: ModelRequestPatch = serde_json::from_value(output.0)
                .map_err(|_| hook_model_error("before_model returned an invalid patch"))?;
            apply_model_patch(&mut request, patch)?;
        }
        self.inner.open_stream(request, cancellation).await
    }
}

fn before_model_input(request: &ChatModelRequest) -> BeforeModelInput {
    BeforeModelInput {
        phase: "before_model",
        turn: turn_context(request),
        system: request.input.instructions.join("\n\n"),
        tools: request
            .input
            .tools
            .iter()
            .map(|tool| ModelToolMetadata {
                name: tool.name.clone(),
                description: tool.description.clone(),
            })
            .collect(),
    }
}

fn turn_context(request: &ChatModelRequest) -> TurnContext {
    let current_user = request
        .input
        .messages
        .iter()
        .rev()
        .find(|message| message.role == ChatRole::User);
    let mut text = Vec::new();
    let mut image_media_types = Vec::new();
    if let Some(message) = current_user {
        for part in &message.content {
            match part {
                ChatContentPart::Text { text: value } => text.push(value.clone()),
                ChatContentPart::Image { media_type, .. } => {
                    image_media_types.push(media_type.clone())
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

fn apply_model_patch(
    request: &mut ChatModelRequest,
    patch: ModelRequestPatch,
) -> Result<(), ChatModelError> {
    if let Some(system) = patch.system {
        request.input.instructions = if system.is_empty() {
            Vec::new()
        } else {
            vec![system]
        };
    }
    if let Some(tool_names) = patch.tool_names {
        let selected = tool_names.iter().cloned().collect::<BTreeSet<_>>();
        if selected.len() != tool_names.len()
            || selected
                .iter()
                .any(|name| !request.input.tools.iter().any(|tool| &tool.name == name))
        {
            return Err(hook_model_error(
                "before_model selected a duplicate or unavailable tool",
            ));
        }
        request
            .input
            .tools
            .retain(|tool| selected.contains(&tool.name));
        match &request.input.tool_choice {
            ChatToolChoice::Specific { name } if !selected.contains(name) => {
                return Err(hook_model_error(
                    "before_model removed the specifically required tool",
                ));
            }
            ChatToolChoice::Required if request.input.tools.is_empty() => {
                return Err(hook_model_error(
                    "before_model removed every required tool",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn hook_model_error(message: impl Into<String>) -> ChatModelError {
    ChatModelError::new(
        ChatModelErrorCode::CausalityRejected,
        message,
        ChatRetryDirective::Never,
    )
}

fn hook_tool_error(
    phase: &str,
    action: &FrozenAction,
    error: &PluginBindingError,
) -> EngineToolError {
    EngineToolError::ToolInvocation(format!(
        "{phase} {} failed ({})",
        action.stable_action_id,
        binding_error_code(error),
    ))
}

fn binding_error_code(error: &PluginBindingError) -> &'static str {
    match error {
        PluginBindingError::ActionNotFound(_)
        | PluginBindingError::ActionUnavailable { .. }
        | PluginBindingError::ActionNotBound { .. } => "PLUGIN_ACTION_UNAVAILABLE",
        PluginBindingError::ArtifactFence(_) => "PLUGIN_ACTION_ARTIFACT_CHANGED",
        PluginBindingError::RecursiveCall(_) | PluginBindingError::CallDepthExceeded => {
            "PLUGIN_ACTION_CALL_CHAIN_REJECTED"
        }
        PluginBindingError::Canceled => "PLUGIN_ACTION_CANCELED",
        PluginBindingError::Timeout => "PLUGIN_ACTION_TIMEOUT",
        PluginBindingError::InputSchema(_) => "PLUGIN_ACTION_INPUT_INVALID",
        PluginBindingError::OutputSchema(_) => "PLUGIN_ACTION_OUTPUT_INVALID",
        PluginBindingError::Runtime { .. } | PluginBindingError::RuntimeTask(_) => {
            "PLUGIN_ACTION_FAILED"
        }
        PluginBindingError::InvalidContract(_)
        | PluginBindingError::UnsupportedBinding(_)
        | PluginBindingError::DuplicateBindingOwner(_)
        | PluginBindingError::MultiplicityConflict(_)
        | PluginBindingError::Poisoned => "PLUGIN_BINDING_INVALID",
    }
}

fn binding_error_message(error: &PluginBindingError) -> &'static str {
    match error {
        PluginBindingError::ActionNotFound(_)
        | PluginBindingError::ActionUnavailable { .. }
        | PluginBindingError::ActionNotBound { .. } => {
            "The configured Plugin Action is unavailable. Reopen Plugin or Agent settings."
        }
        PluginBindingError::ArtifactFence(_) => {
            "The Plugin changed after this Agent Session was opened. Reopen the Session."
        }
        PluginBindingError::Timeout => "The Plugin Action timed out.",
        PluginBindingError::InputSchema(_) => "The Plugin Action rejected its input contract.",
        PluginBindingError::OutputSchema(_) => "The Plugin Action returned invalid output.",
        PluginBindingError::RecursiveCall(_) | PluginBindingError::CallDepthExceeded => {
            "The Plugin Action call chain was rejected."
        }
        PluginBindingError::Canceled => "The Plugin Action was canceled.",
        _ => "The Plugin Action failed safely.",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use futures_util::stream;
    use nomifun_agent_contracts::{
        AgentSessionId, ChatRouteIdentity, CorrelationId, EventId, IdempotencyKey, ModelRouteId,
        OperationId,
        PluginActionManifest, PluginActionPublication, PluginArtifactRef, PluginBindingPoint,
        PluginId, ResolvedSnapshotRef, VersionString,
    };
    use nomifun_chat_model_broker::{
        ChatCausality, ChatMessage, ChatModelEvent, ChatModelInput, ChatResponseFormat,
        ChatToolCall, ChatToolDefinition, PromptCachePolicy, ToolCallId,
        CHAT_MODEL_CONTRACT_VERSION,
    };
    use nomifun_engine_core::EngineToolInvoker;
    use nomifun_plugin_platform::{
        InMemoryPluginBindingRegistry, PluginActionCallError, PluginActionInvocation,
        PluginActionRegistration, PluginActionRuntimePort,
    };
    use serde_json::Value;
    use tokio::sync::Notify;

    use super::*;

    const ARTIFACT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Default)]
    struct RecordingRuntime {
        outputs: Mutex<BTreeMap<String, StrictJsonValue>>,
        calls: Mutex<Vec<(String, Option<PluginBindingPoint>, StrictJsonValue, Vec<String>)>>,
        block_action: Mutex<Option<String>>,
        entered: Notify,
        observed_cancellation: AtomicBool,
    }

    impl RecordingRuntime {
        fn output(&self, stable_action_id: &str, output: Value) {
            self.outputs
                .lock()
                .unwrap()
                .insert(stable_action_id.into(), StrictJsonValue(output));
        }
    }

    #[async_trait]
    impl PluginActionRuntimePort for RecordingRuntime {
        async fn invoke(
            &self,
            invocation: PluginActionInvocation,
        ) -> Result<StrictJsonValue, PluginActionCallError> {
            self.calls.lock().unwrap().push((
                invocation.stable_action_id.clone(),
                invocation.binding_point,
                invocation.input,
                invocation.call_chain,
            ));
            if self.block_action.lock().unwrap().as_deref()
                == Some(invocation.stable_action_id.as_str())
            {
                self.entered.notify_one();
                invocation.cancellation.cancelled().await;
                self.observed_cancellation.store(true, Ordering::Release);
                return Err(PluginActionCallError::new("CANCELED", "fixture canceled"));
            }
            self.outputs
                .lock()
                .unwrap()
                .get(&invocation.stable_action_id)
                .cloned()
                .ok_or_else(|| PluginActionCallError::new("NO_FIXTURE", "missing fixture"))
        }
    }

    struct NeverTool;
    #[async_trait]
    impl EngineToolInvoker for NeverTool {
        async fn invoke(
            &self,
            _: EngineToolInvocation,
            _: CancellationToken,
        ) -> Result<EngineToolResult, EngineToolError> {
            Err(EngineToolError::ToolInvocation(
                "Plugin Action incorrectly reached the Kernel".into(),
            ))
        }
    }

    #[derive(Default)]
    struct CaptureModel(Mutex<Option<ChatModelRequest>>);

    #[async_trait]
    impl EngineModelPort for CaptureModel {
        async fn open_stream(
            &self,
            request: ChatModelRequest,
            _: CancellationToken,
        ) -> Result<EngineModelStream, ChatModelError> {
            *self.0.lock().unwrap() = Some(request);
            Ok(Box::pin(stream::empty::<Result<ChatModelEvent, ChatModelError>>()))
        }
    }

    fn plugin_id() -> PluginId {
        PluginId::from("019b0000-0000-7000-8000-000000000123")
    }

    fn stable(action_id: &str) -> String {
        format!("plugin:{}/{action_id}", plugin_id().as_ref())
    }

    fn publication(
        action_id: &str,
        point: PluginBindingPoint,
    ) -> PluginActionRegistration {
        PluginActionPublication {
            plugin_id: plugin_id(),
            artifact: PluginArtifactRef {
                digest: DigestHex::from(ARTIFACT),
                package_id: "local.agent-bindings".into(),
                version: "1.0.0".into(),
            },
            action_id: action_id.into(),
            action: PluginActionManifest {
                name: action_id.replace('_', " "),
                description: format!("{action_id} fixture"),
                input: StrictJsonValue(json!({"type":"object"})),
                output: StrictJsonValue(json!({"type":"object"})),
                effect: PluginActionEffect::Read,
            },
            bindings: BTreeSet::from([point]),
        }
        .into()
    }

    fn fixture() -> (
        Arc<RecordingRuntime>,
        InMemoryPluginBindingRegistry,
        FrozenAgentPluginBindings,
    ) {
        let runtime = Arc::new(RecordingRuntime::default());
        runtime.output(&stable("lookup"), json!({"answer": 42}));
        runtime.output(&stable("context"), json!({"memory": "remember"}));
        runtime.output(
            &stable("before_model"),
            json!({"system": "patched system"}),
        );
        runtime.output(&stable("before_tool"), json!({"decision": "allow"}));
        let registry = InMemoryPluginBindingRegistry::new(runtime.clone());
        super::super::plugin_ports::register_plugin_binding_owners(&registry).unwrap();
        registry
            .replace_plugin(
                plugin_id(),
                DigestHex::from(ARTIFACT),
                true,
                vec![
                    publication("lookup", PluginBindingPoint::AgentTool),
                    publication("context", PluginBindingPoint::AgentContext),
                    publication("before_model", PluginBindingPoint::AgentBeforeModel),
                    publication("before_tool", PluginBindingPoint::AgentBeforeTool),
                ],
            )
            .unwrap();
        let frozen = FrozenAgentPluginBindings::freeze(
            AgentPluginBindings::new(registry.clone()),
            false,
        )
        .unwrap();
        (runtime, registry, frozen)
    }

    fn route() -> ChatRouteIdentity {
        ChatRouteIdentity::new(
            "preset-revision",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            ModelRouteId::from("route"),
            1,
        )
    }

    fn request(tool: ChatToolDefinition) -> ChatModelRequest {
        let route = route();
        ChatModelRequest {
            contract_version: VersionString::from(
                CHAT_MODEL_CONTRACT_VERSION,
            ),
            causality: ChatCausality {
                agent_session_id: AgentSessionId::from("session"),
                turn_operation_id: OperationId::from("turn"),
                causation_event_id: EventId::from("event"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "b".repeat(64).into(),
                },
                route_identity: route.clone(),
                operation_id: OperationId::from("model"),
            },
            route,
            input: ChatModelInput {
                instructions: vec!["original system".into()],
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: vec![ChatContentPart::Text { text: "hello".into() }],
                    provider_round_id: None,
                }],
                tools: vec![tool],
                tool_choice: ChatToolChoice::Auto,
                max_output_tokens: None,
                reasoning: None,
                prompt_cache: PromptCachePolicy::Disabled,
                response_format: ChatResponseFormat::Text,
                requested_output_modalities: BTreeSet::new(),
                provider_round_parent: None,
                preserve_native_responses_items: false,
                metadata: BTreeMap::new(),
            },
        }
    }

    fn invocation(binding: EngineToolBinding) -> EngineToolInvocation {
        EngineToolInvocation {
            agent_session_id: AgentSessionId::from("session"),
            principal: nomifun_agent_contracts::PrincipalRef {
                principal_kind: "user".into(),
                principal_id: "owner".into(),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "b".repeat(64).into(),
            },
            active_set_generation: 0,
            turn_operation_id: OperationId::from("turn"),
            operation_id: OperationId::from("operation"),
            idempotency_key: IdempotencyKey::from("idempotency"),
            correlation_id: CorrelationId::from("correlation"),
            call: ChatToolCall {
                call_id: ToolCallId::from("call"),
                name: binding.model_name.clone(),
                arguments: StrictJsonValue(json!({"query": "nomifun"})),
                provider_metadata: None,
            },
            binding,
        }
    }

    #[tokio::test]
    async fn real_agent_surface_discovers_invokes_and_runs_all_binding_phases() {
        let (runtime, _registry, frozen) = fixture();
        let plan = frozen.tool_plan();
        assert_eq!(plan.len(), 1);
        let definition = plan.model_definitions().pop().unwrap();
        let binding = plan.binding(&definition.name).unwrap().clone();
        assert_eq!(binding.capability_id.as_ref(), stable("lookup"));
        assert!(definition.description.contains(&stable("lookup")));

        let context = frozen
            .context_for_turn(
                &TurnContext {
                    turn_id: "turn".into(),
                    source_message_id: "event".into(),
                    text: "hello".into(),
                    image_media_types: Vec::new(),
                    cs_dialogue_id: None,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(context.contains(&stable("context")));
        assert!(context.contains("remember"));

        let model = Arc::new(CaptureModel::default());
        let wrapped_model = frozen.wrap_model(model.clone());
        let _stream = wrapped_model
            .open_stream(request(definition), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            model.0.lock().unwrap().as_ref().unwrap().input.instructions,
            vec!["patched system"]
        );

        let tools = frozen.wrap_tools(Arc::new(NeverTool));
        let result = tools
            .invoke(invocation(binding), CancellationToken::new())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output_text().contains("42"));

        let calls = runtime.calls.lock().unwrap();
        for point in [
            PluginBindingPoint::AgentContext,
            PluginBindingPoint::AgentBeforeModel,
            PluginBindingPoint::AgentBeforeTool,
            PluginBindingPoint::AgentTool,
        ] {
            assert!(calls.iter().any(|(_, actual, _, _)| *actual == Some(point)));
        }
        assert!(calls.iter().all(|(stable, _, _, chain)| chain.last() == Some(stable)));
    }

    #[tokio::test]
    async fn stale_session_tool_reports_unavailable_instead_of_rebinding() {
        let (_runtime, registry, mut frozen) = fixture();
        let definition = frozen.tool_plan().model_definitions().pop().unwrap();
        let binding = frozen
            .tool_plan()
            .binding(&definition.name)
            .unwrap()
            .clone();
        registry
            .replace_plugin(
                plugin_id(),
                DigestHex::from(ARTIFACT),
                false,
                vec![publication("lookup", PluginBindingPoint::AgentTool)],
            )
            .unwrap();
        // This assertion targets the model-visible Action tombstone. A real
        // stale Session that also froze a fail-closed before_tool hook is
        // rejected by that hook first, which is separately intentional.
        frozen.before_tool = Arc::from(Vec::<FrozenAction>::new());
        let result = frozen
            .wrap_tools(Arc::new(NeverTool))
            .invoke(invocation(binding), CancellationToken::new())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output_text().contains("PLUGIN_ACTION_UNAVAILABLE"));
        assert!(result.output_text().contains(&stable("lookup")));
    }

    #[tokio::test]
    async fn artifact_update_is_fenced_and_engine_cancellation_reaches_runtime() {
        let (runtime, registry, mut frozen) = fixture();
        let definition = frozen.tool_plan().model_definitions().pop().unwrap();
        let binding = frozen
            .tool_plan()
            .binding(&definition.name)
            .unwrap()
            .clone();
        frozen.before_tool = Arc::from(Vec::<FrozenAction>::new());

        let mut updated = publication("lookup", PluginBindingPoint::AgentTool);
        updated.publication.artifact.digest = DigestHex::from("c".repeat(64));
        updated.publication.artifact.version = "2.0.0".into();
        registry
            .replace_plugin(
                plugin_id(),
                DigestHex::from("c".repeat(64)),
                true,
                vec![updated],
            )
            .unwrap();
        let result = frozen
            .wrap_tools(Arc::new(NeverTool))
            .invoke(invocation(binding.clone()), CancellationToken::new())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result
            .output_text()
            .contains("PLUGIN_ACTION_ARTIFACT_CHANGED"));

        // Restore the frozen Artifact, then prove Engine cancellation crosses
        // the adapter and reaches the retained Plugin runtime task.
        registry
            .replace_plugin(
                plugin_id(),
                DigestHex::from(ARTIFACT),
                true,
                vec![publication("lookup", PluginBindingPoint::AgentTool)],
            )
            .unwrap();
        *runtime.block_action.lock().unwrap() = Some(stable("lookup"));
        let cancellation = CancellationToken::new();
        let call = tokio::spawn({
            let tools = frozen.wrap_tools(Arc::new(NeverTool));
            let cancellation = cancellation.clone();
            async move { tools.invoke(invocation(binding), cancellation).await }
        });
        runtime.entered.notified().await;
        cancellation.cancel();
        assert!(matches!(call.await.unwrap(), Err(EngineToolError::Cancelled)));
        assert!(runtime.observed_cancellation.load(Ordering::Acquire));
    }

    #[test]
    fn restricted_session_has_no_plugin_tools_or_hooks() {
        let (_runtime, registry, _) = fixture();
        let frozen = FrozenAgentPluginBindings::freeze(
            AgentPluginBindings::new(registry),
            true,
        )
        .unwrap();
        assert!(frozen.tool_plan().is_empty());
        assert!(frozen.contexts.is_empty());
        assert!(frozen.before_model.is_empty());
        assert!(frozen.before_tool.is_empty());
    }
}
