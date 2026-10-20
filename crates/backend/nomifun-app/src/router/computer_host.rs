//! Computer's typed Kernel adapter and canonical Effect settlement.
//! Native state belongs to the private worker, never a host observation cache.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding};
use nomifun_agent_domain_wave2::{
    Wave2HostContext, Wave2HostPort, Wave2HostPortError, Wave2HostRequest,
    Wave2TypedHostRequest,
};
use nomifun_computer::result::ResultEffectOutcome;
use nomifun_computer::surface::{self, FrozenTool, RouteTier};
use nomifun_computer::worker::{CallError, CallErrorKind, Computer};

pub(crate) fn worker_binary() -> std::path::PathBuf {
    std::env::var_os("NOMIFUN_COMPUTER_WORKER").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap_or_default()
            .with_file_name(if cfg!(windows) { "nomifun-computer-worker.exe" } else { "nomifun-computer-worker" }))
}

#[async_trait::async_trait]
pub(crate) trait RoleHostInvoker: Send + Sync {
    async fn invoke(&self, request: Wave2TypedHostRequest)
        -> Result<StrictJsonValue, Wave2HostPortError>;
}

pub(crate) struct RoleHostPortAdapter(pub Arc<dyn RoleHostInvoker>);

impl Wave2HostPort for RoleHostPortAdapter {
    fn invoke<'a>(&'a self, request: Wave2HostRequest) -> std::pin::Pin<Box<
        dyn std::future::Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a,
    >> {
        Box::pin(async move { self.0.invoke(request.into_typed()?).await })
    }
}

pub(crate) struct ComputerRoleInvoker {
    computer: Arc<Computer>,
    effects: nomifun_agent_session::AgentSessionStore,
    pool: nomifun_db::SqlitePool,
}

impl ComputerRoleInvoker {
    pub(crate) fn new(computer: Arc<Computer>, effects: nomifun_agent_session::AgentSessionStore, pool: nomifun_db::SqlitePool) -> Self {
        Self { computer, effects, pool }
    }
}

fn reject(message: impl Into<String>) -> Wave2HostPortError {
    Wave2HostPortError::new("COMPUTER_INVOCATION_REJECTED", message)
}

fn resource<'a>(context: &'a Wave2HostContext, tool: &FrozenTool)
    -> Result<&'a TypedResourceBinding, Wave2HostPortError>
{
    let provider = context.role_provider.as_ref().ok_or_else(|| reject("Computer provider is missing"))?;
    if context.capability_id.as_ref() != "computer"
        || provider.role.key.role_id.as_ref() != nomifun_agent_domain_wave2::COMPUTER_EXECUTION_ROLE_ID
        || provider.package.id.as_ref() != nomifun_agent_domain_wave2::COMPUTER_PACKAGE_ID
        || provider.mount_id.as_ref() != nomifun_agent_domain_wave2::COMPUTER_MOUNT_ID
        || context.principal.principal_id.trim().is_empty()
        || context.principal.principal_kind.trim().is_empty()
        || context.agent_session_id.as_ref().trim().is_empty()
        || context.action_id.as_ref() != tool.action_id
        || !context.authorized_actions.contains(&context.action_id)
    {
        return Err(reject("Computer invocation differs from its admitted provider or Action"));
    }
    let matching = context.resource_bindings.iter()
        .filter(|binding| binding.resource_kind.as_ref() == "computer")
        .collect::<Vec<_>>();
    let [binding] = matching.as_slice() else {
        return Err(reject("Computer requires exactly one bound desktop"));
    };
    if binding.owner_id != context.principal.principal_id
        || binding.binding_id.as_ref().trim().is_empty()
        || binding.resource_id.as_ref() != super::nomi_core_resource_bindings::LOCAL_COMPUTER_RESOURCE_ID
        || !binding.operations.contains(tool.resource_operation.as_str())
    {
        return Err(reject("Computer desktop ownership or operation does not match its binding"));
    }
    Ok(binding)
}

fn call_error(error: &CallError) -> Wave2HostPortError {
    let message = error.permission_required.as_ref()
        .map(ToString::to_string).unwrap_or_else(|| error.message.clone());
    Wave2HostPortError::new(error.code.clone(), message)
}

#[async_trait::async_trait]
impl RoleHostInvoker for ComputerRoleInvoker {
    async fn invoke(&self, request: Wave2TypedHostRequest)
        -> Result<StrictJsonValue, Wave2HostPortError>
    {
        let (action, input) = request.operation.computer_action()
            .ok_or_else(|| reject("Operation is not a Computer Action"))?;
        let tool = surface::tool(action).ok_or_else(|| reject("Computer Action is not frozen"))?;
        let context = &request.context;
        let binding = resource(context, tool)?;
        // Registry generation and the exact Provider lock were checked by the
        // Kernel. Reprove the original canonical call and Snapshot before I/O.
        let cause = self.effects.native_tool_call_causation(
            &context.principal, &context.agent_session_id, &context.turn_id,
            &context.operation_id, &context.capability_id, &context.action_id,
        ).await.map_err(|error| reject(error.to_string()))?;
        if cause.execution_scope.resolved_snapshot_ref() != &context.resolved_snapshot_ref {
            return Err(reject("Computer call Snapshot does not match canonical causation"));
        }
        let authorized = context.authorized_actions.iter()
            .map(|action| action.as_ref().to_owned()).collect::<BTreeSet<_>>();
        let prepared = surface::prepare(action, input.0.clone(), &authorized, RouteTier::Full)
            .map_err(|error| Wave2HostPortError::new(error.code, error.message))?;
        if context.control.is_cancelled() {
            return Err(Wave2HostPortError::new("INVOCATION_CANCELLED", "Computer call cancelled before dispatch"));
        }
        let strategy = match prepared.tool.resource_operation.as_str() {
            "launch" => Some(nomifun_agent_session::EffectStrategy::ManagedEffect),
            "input" => Some(nomifun_agent_session::EffectStrategy::ExternalUncertainEffect),
            _ => None,
        };
        let reservation = if let Some(strategy) = strategy {
            match super::agent_wave2_host::begin_wave2_exclusive_effect(
                &self.effects, context, binding, input, strategy,
            ).await? {
                super::agent_wave2_host::Wave2EffectAdmission::Replay(result) => return Ok(result),
                super::agent_wave2_host::Wave2EffectAdmission::Reserved(reservation) => Some(reservation),
            }
        } else { None };
        // Display metadata supplies the cursor caption only. Authorization and
        // runtime context remain the canonical call and exact Snapshot above.
        let agent_name = sqlx::query_scalar::<_, String>(
            "SELECT json_extract(p.display_json, '$.display_name') FROM agent_presets p \
             JOIN agent_runtime_snapshots s ON json_extract(s.content_json, '$.preset_revision_ref.preset_id') = p.preset_id \
             WHERE s.snapshot_id = ? AND s.snapshot_digest = ?",
        ).bind(context.resolved_snapshot_ref.snapshot_id.as_ref())
            .bind(context.resolved_snapshot_ref.snapshot_digest.as_ref())
            .fetch_optional(&self.pool).await.ok().flatten()
            .filter(|name| !name.trim().is_empty()).unwrap_or_else(|| "NomiFun".into());
        let result = self.computer.call(
            context.agent_session_id.as_ref(), &agent_name, action, prepared.arguments,
            &authorized, true, context.control.clone(),
        ).await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let failure = call_error(&error);
                if let Some(reservation) = &reservation {
                    if error.kind == CallErrorKind::Unknown {
                        super::agent_wave2_host::finish_wave2_uncertain_effect(reservation, action, &failure).await?;
                    } else {
                        super::agent_wave2_host::finish_wave2_failed_effect(reservation, action, &failure).await?;
                    }
                }
                return Err(failure);
            }
        };
        let normalized = nomifun_computer::result::normalize(&result.result, true);
        let output = StrictJsonValue(result.result);
        if let Some(reservation) = &reservation {
            match normalized.effect {
                ResultEffectOutcome::Succeeded => {
                    super::agent_wave2_host::finish_wave2_succeeded_effect(reservation, action, &output).await?;
                }
                ResultEffectOutcome::Failed => {
                    let failure = Wave2HostPortError::new("COMPUTER_TOOL_REFUSED", output.0.to_string());
                    super::agent_wave2_host::finish_wave2_failed_effect(reservation, action, &failure).await?;
                }
                ResultEffectOutcome::Unknown => {
                    let failure = Wave2HostPortError::new("COMPUTER_OUTCOME_UNKNOWN", output.0.to_string());
                    super::agent_wave2_host::finish_wave2_uncertain_effect(reservation, action, &failure).await?;
                }
            }
        }
        Ok(output)
    }
}

/// Model presentation narrows the frozen authority for this route. It does not
/// add Actions or change the Kernel's canonical provider/resource checks.
pub(super) fn project_exposure(
    mut exposure: nomifun_engine_core::EngineToolExposure,
    tier: RouteTier,
) -> Option<nomifun_engine_core::EngineToolExposure> {
    if exposure.capability_id.as_ref() != "computer" { return Some(exposure); }
    let tool = surface::tools_for_tier(tier, "en-US").into_iter()
        .find(|tool| tool.action_id == exposure.action_id.as_ref())?;
    exposure.definition.name = tool.name;
    exposure.definition.description = tool.description;
    exposure.definition.input_schema = StrictJsonValue(tool.input_schema);
    // Catalog pressure and ToolSearch own visibility within this exact plan.
    // Recompilation must retain a presentation deferral instead of resetting
    // it to the frozen initial-loading preference during install validation.
    exposure.definition.deferred |= tool.deferred;
    Some(exposure)
}

pub(super) struct ComputerModelTools {
    pub inner: Arc<dyn nomifun_engine_core::EngineToolInvoker>,
    pub snapshot: Arc<nomifun_agent_kernel::CompiledSnapshot>,
    pub active: Arc<nomifun_agent_kernel::SessionCapabilityState>,
    pub attempt: Arc<ComputerAttemptSurface>,
}

struct AttemptCall {
    name: String,
    action: String,
    arguments: nomifun_agent_contracts::DigestHex,
}
struct CurrentAttempt {
    causality: nomifun_chat_model_broker::ChatCausality,
    route: nomifun_chat_model_broker::ResolvedChatRoute,
    tier: RouteTier,
    calls: BTreeMap<String, AttemptCall>,
}

/// Only the current response's presentation receipt. Consuming it cannot grant
/// an Action: every call still traverses the original Kernel and Effect owner.
pub(super) struct ComputerAttemptSurface {
    session: nomifun_agent_contracts::AgentSessionId,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    tools: OnceLock<BTreeMap<String, String>>,
    current: Mutex<Option<CurrentAttempt>>,
}

impl ComputerAttemptSurface {
    pub(super) fn new(session: nomifun_agent_contracts::AgentSessionId, snapshot: nomifun_agent_contracts::ResolvedSnapshotRef) -> Self {
        Self { session, snapshot, tools: OnceLock::new(), current: Mutex::new(None) }
    }

    pub(super) fn bind_plan(&self, plan: &nomifun_engine_core::EngineToolPlan) -> Result<(), nomifun_engine_core::EngineToolError> {
        let mut tools = BTreeMap::new();
        for definition in plan.model_definitions() {
            let binding = plan.binding(&definition.name).expect("validated plan definition has binding");
            if binding.capability_id.as_ref() != "computer" { continue; }
            let frozen = surface::tool(binding.action_id.as_ref()).filter(|tool| tool.model_tool == definition.name)
                .ok_or_else(|| nomifun_engine_core::EngineToolError::ToolInvocation("Computer presentation does not match its admitted tool identity".into()))?;
            tools.insert(definition.name, frozen.action_id.clone());
        }
        if let Some(existing) = self.tools.get() {
            if existing == &tools { return Ok(()); }
        }
        self.tools.set(tools).map_err(|_| nomifun_engine_core::EngineToolError::ToolInvocation("Computer presentation plan was already bound".into()))
    }

    pub(super) fn clear(&self) {
        *self.current.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub(super) fn has_tools(&self) -> bool {
        self.tools.get().is_some_and(|tools| !tools.is_empty())
    }

    fn presents_computer(&self, request: &nomifun_chat_model_broker::ChatModelRequest) -> bool {
        request.input.tools.iter().any(|definition| self.tools.get().is_some_and(|tools| tools.contains_key(&definition.name)))
    }

    fn tier(route: &nomifun_chat_model_broker::ResolvedChatRoute, adapter: &BTreeSet<nomifun_chat_model_broker::ChatModelFeature>) -> RouteTier {
        use nomifun_chat_model_broker::ChatModelFeature;
        RouteTier::from_capabilities(route.features.contains(&ChatModelFeature::ToolCalls) && adapter.contains(&ChatModelFeature::ToolCalls),
            route.features.contains(&ChatModelFeature::ImageInput) && adapter.contains(&ChatModelFeature::ImageInput))
    }

    fn require_request(&self, request: &nomifun_chat_model_broker::ChatModelRequest) -> Result<(), nomifun_chat_model_broker::ChatModelError> {
        if request.causality.agent_session_id != self.session || request.causality.resolved_snapshot_ref != self.snapshot {
            return Err(nomifun_chat_model_broker::ChatModelError::invalid_request("Computer presentation request belongs to another Session or Snapshot"));
        }
        Ok(())
    }

    fn consume(&self, invocation: &nomifun_engine_core::EngineToolInvocation) -> Result<RouteTier, nomifun_engine_core::EngineToolError> {
        let failure = || nomifun_engine_core::EngineToolError::ToolInvocation("Computer call has no matching current model-attempt presentation receipt".into());
        let digest = nomifun_agent_contracts::digest_payload(&invocation.call.arguments.0).map_err(|_| failure())?;
        let mut state = self.current.lock().map_err(|_| failure())?;
        let current = state.as_mut().ok_or_else(failure)?;
        let call = current.calls.get(invocation.call.call_id.as_ref()).ok_or_else(failure)?;
        if current.causality.agent_session_id != invocation.agent_session_id
            || current.causality.turn_operation_id != invocation.turn_operation_id
            || current.causality.resolved_snapshot_ref != invocation.resolved_snapshot_ref
            || call.name != invocation.call.name || call.action != invocation.binding.action_id.as_ref()
            || call.arguments != digest
        { return Err(failure()); }
        let tier = current.tier;
        current.calls.remove(invocation.call.call_id.as_ref());
        if current.calls.is_empty() { *state = None; }
        Ok(tier)
    }
}

impl nomifun_chat_model_broker::ChatAttemptProjection for ComputerAttemptSurface {
    fn project(&self, request: &nomifun_chat_model_broker::ChatModelRequest, route: &nomifun_chat_model_broker::ResolvedChatRoute,
        adapter: &BTreeSet<nomifun_chat_model_broker::ChatModelFeature>) -> Result<nomifun_chat_model_broker::ChatModelRequest, nomifun_chat_model_broker::ChatModelError> {
        self.require_request(request)?;
        if !self.presents_computer(request) { return Ok(request.clone()); }
        let tier = Self::tier(route, adapter);
        let presented = surface::tools_for_tier(tier, "en-US");
        let mut projected = request.clone();
        projected.input.tools = request.input.tools.iter().filter_map(|definition| {
            let Some(action) = self.tools.get().and_then(|tools| tools.get(&definition.name)) else {
                return Some(definition.clone());
            };
            let selected = presented.iter().find(|tool| &tool.action_id == action)?;
            Some(nomifun_chat_model_broker::ChatToolDefinition { name: selected.name.clone(), description: selected.description.clone(),
                input_schema: StrictJsonValue(selected.input_schema.clone()), deferred: definition.deferred })
        }).collect();
        if projected.input.tools.is_empty() {
            projected.input.tool_choice = nomifun_chat_model_broker::ChatToolChoice::None;
            projected.input.parallel_tool_calls = None;
        }
        if projected.input.tools.iter().any(|definition| self.tools.get().is_some_and(|tools| tools.contains_key(&definition.name))) {
            projected.input.instructions.push(surface::instructions(tier, "en-US"));
            projected.input.parallel_tool_calls = Some(false);
        }
        Ok(projected)
    }

    fn begin_attempt(&self, request: &nomifun_chat_model_broker::ChatModelRequest, route: &nomifun_chat_model_broker::ResolvedChatRoute,
        adapter: &BTreeSet<nomifun_chat_model_broker::ChatModelFeature>) -> Result<(), nomifun_chat_model_broker::ChatModelError> {
        self.require_request(request)?;
        if !self.presents_computer(request) { self.clear(); return Ok(()); }
        *self.current.lock().map_err(|_| nomifun_chat_model_broker::ChatModelError::invalid_request("Computer attempt presentation lock failed"))? = Some(CurrentAttempt {
            causality: request.causality.clone(), route: route.clone(), tier: Self::tier(route, adapter), calls: BTreeMap::new(),
        });
        Ok(())
    }

    fn observe(&self, request: &nomifun_chat_model_broker::ChatModelRequest, route: &nomifun_chat_model_broker::ResolvedChatRoute,
        event: &nomifun_chat_model_broker::ChatModelEvent) -> Result<(), nomifun_chat_model_broker::ChatModelError> {
        use nomifun_chat_model_broker::{ChatModelError, ChatModelEvent};
        self.require_request(request)?;
        if matches!(event, ChatModelEvent::Completed { .. }) {
            let mut current = self.current.lock().map_err(|_| ChatModelError::invalid_request("Computer attempt presentation lock failed"))?;
            if current.as_ref().is_some_and(|attempt| attempt.causality == request.causality && attempt.calls.is_empty()) { *current = None; }
            return Ok(());
        }
        let ChatModelEvent::ToolCallCompleted { call } = event else { return Ok(()); };
        let Some(action) = self.tools.get().and_then(|tools| tools.get(&call.name)) else { return Ok(()); };
        if !request.input.tools.iter().any(|definition| definition.name == call.name) {
            return Err(ChatModelError::protocol_violation("Computer call was not presented for this attempted route"));
        }
        let mut state = self.current.lock().map_err(|_| ChatModelError::invalid_request("Computer attempt presentation lock failed"))?;
        let current = state.as_mut().filter(|current| current.causality == request.causality && current.route == *route)
            .ok_or_else(|| ChatModelError::protocol_violation("Computer response does not belong to the current attempted route"))?;
        // Matches the Runtime's existing per-model-step call bound; this is not
        // a task or desktop quota, and receipts disappear as calls are consumed.
        if current.calls.len() >= 64 || current.calls.contains_key(call.call_id.as_ref()) {
            return Err(ChatModelError::protocol_violation("Computer attempt call receipts exceed the current response boundary"));
        }
        current.calls.insert(call.call_id.as_ref().to_owned(), AttemptCall { name: call.name.clone(), action: action.clone(),
            arguments: nomifun_agent_contracts::digest_payload(&call.arguments.0).map_err(|_| ChatModelError::invalid_request("Computer call digest failed"))? });
        Ok(())
    }
}

#[async_trait::async_trait]
impl nomifun_engine_core::EngineToolInvoker for ComputerModelTools {
    async fn invoke(
        &self, mut invocation: nomifun_engine_core::EngineToolInvocation,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<nomifun_engine_core::EngineToolResult, nomifun_engine_core::EngineToolError> {
        use nomifun_chat_model_broker::ChatToolResultPart;
        use nomifun_engine_core::{EngineToolError, EngineToolResult};
        if invocation.binding.capability_id.as_ref() != "computer" {
            return self.inner.invoke(invocation, cancellation).await;
        }
        let tier = self.attempt.consume(&invocation)?;
        let failure = |message: String| EngineToolError::ToolInvocation(message);
        let active = self.active.snapshot().map_err(|error| failure(error.to_string()))?;
        if active.generation != invocation.active_set_generation
            || active.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || self.snapshot.resolved_capability(&invocation.binding.capability_id)
                .is_none_or(|capability| capability.contribution_lock.source_kind
                    != nomifun_agent_contracts::ContributionSourceKind::PlatformBuiltin)
        {
            return Err(failure("Computer model route no longer matches its admitted Snapshot".into()));
        }
        let authorized = self.snapshot.policy(&invocation.binding.capability_id)
            .ok_or_else(|| failure("Computer authority policy is missing".into()))?
            .allowed_actions.iter().map(|action| action.as_ref().to_owned()).collect();
        let mut prepared = surface::prepare(
            invocation.binding.action_id.as_ref(), invocation.call.arguments.0,
            &authorized, tier,
        ).map_err(|error| failure(error.to_string()))?;
        // The worker injects its own trusted limits after Kernel admission.
        // Keep model/canonical input free of those private runtime parameters.
        surface::strip_trusted(&mut prepared.arguments);
        invocation.call.arguments = StrictJsonValue(prepared.arguments);
        let result = self.inner.invoke(invocation, cancellation).await?;
        if result.is_error { return Ok(result); }
        let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
            return Err(failure("Computer result is not the admitted worker envelope".into()));
        };
        let raw = serde_json::from_str(text).map_err(|error| failure(format!("Computer result decode: {error}")))?;
        let normalized = nomifun_computer::result::normalize(&raw, tier == RouteTier::Full);
        let result = EngineToolResult {
            call_id: result.call_id,
            output: normalized.parts.into_iter().map(|part| match part {
                nomifun_computer::result::ResultPart::Text(text) => ChatToolResultPart::Text { text },
                nomifun_computer::result::ResultPart::Image { data, mime_type } =>
                    ChatToolResultPart::Image { media_type: mime_type, data_base64: data },
            }).collect(),
            is_error: normalized.is_error,
        };
        result.validate_for(&result.call_id)?;
        Ok(result)
    }
}

#[cfg(test)]
mod attempt_tests {
    use super::*;
    use nomifun_chat_model_broker::{ChatAttemptProjection, ChatContentPart, ChatModelFeature, ChatToolCall, ChatToolDefinition};
    use nomifun_engine_core::{EngineEffectClass, EngineToolBinding, EngineToolInvocation};
    use serde_json::json;

    fn fixture() -> (ComputerAttemptSurface, nomifun_chat_model_broker::ChatModelRequest, nomifun_chat_model_broker::ResolvedChatRoute, BTreeSet<ChatModelFeature>) {
        let sample = nomifun_chat_model_broker::recorded_conformance_fixtures().remove(0);
        let mut request = sample.request;
        request.input.instructions = vec!["Base agent instructions".into()];
        request.input.tools = surface::tools_for_tier(RouteTier::Full, "en-US").into_iter().map(|tool| ChatToolDefinition {
            name: tool.name, description: tool.description, input_schema: StrictJsonValue(tool.input_schema), deferred: tool.deferred,
        }).collect();
        request.input.tool_choice = nomifun_chat_model_broker::ChatToolChoice::Auto;
        let owner = ComputerAttemptSurface::new(request.causality.agent_session_id.clone(), request.causality.resolved_snapshot_ref.clone());
        owner.tools.set(request.input.tools.iter().map(|definition| (definition.name.clone(), surface::tool(&definition.name).unwrap().action_id.clone())).collect()).unwrap();
        (owner, request, sample.route, BTreeSet::from([ChatModelFeature::ToolCalls, ChatModelFeature::ImageInput]))
    }

    fn invocation(request: &nomifun_chat_model_broker::ChatModelRequest, call: ChatToolCall) -> EngineToolInvocation {
        let tool = surface::tool(&call.name).unwrap();
        let definition = request.input.tools.iter().find(|definition| definition.name == call.name).unwrap().clone();
        EngineToolInvocation {
            agent_session_id: request.causality.agent_session_id.clone(),
            principal: nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: "fixture".into() },
            resolved_snapshot_ref: request.causality.resolved_snapshot_ref.clone(), active_set_generation: 1,
            turn_operation_id: request.causality.turn_operation_id.clone(), operation_id: "fixture-tool".into(),
            idempotency_key: "fixture-call".into(), correlation_id: "fixture-turn".into(), call,
            binding: EngineToolBinding { model_name: definition.name.clone(),
                schema_digest: nomifun_engine_core::input_schema_digest(&definition.input_schema).unwrap(),
                definition, canonical_input_schema_ref: format!("schema://computer/{}/input", tool.driver_tool).into(),
                capability_contract_digest: "a".repeat(64).into(), capability_id: "computer".into(), action_id: tool.action_id.clone().into(),
                resource_binding_ids: BTreeSet::new(), effect_class: EngineEffectClass::ReadOnly, parallel_safe: false },
        }
    }

    #[test]
    fn actual_candidate_features_select_schema_and_one_instruction_without_touching_history() {
        let (owner, request, mut route, adapter) = fixture();
        for tier in [RouteTier::Full, RouteTier::NoImage, RouteTier::Unavailable] {
            route.features = match tier {
                RouteTier::Full => adapter.clone(), RouteTier::NoImage => BTreeSet::from([ChatModelFeature::ToolCalls]), RouteTier::Unavailable => BTreeSet::new(),
            };
            let projected = owner.project(&request, &route, &adapter).unwrap();
            assert_eq!(projected.input.messages, request.input.messages);
            assert_eq!(projected.input.tools.len(), surface::tools_for_tier(tier, "en-US").len());
            assert_eq!(projected.input.instructions.len(), if tier == RouteTier::Unavailable { 1 } else { 2 });
            if tier != RouteTier::Unavailable {
                assert_eq!(projected.input.instructions[1], surface::instructions(tier, "en-US"));
            } else {
                assert_eq!(projected.input.tool_choice, nomifun_chat_model_broker::ChatToolChoice::None);
            }
            if tier == RouteTier::NoImage {
                let window = surface::tool("get_window_state").unwrap();
                let definition = projected.input.tools.iter().find(|tool| tool.name == window.model_tool).unwrap();
                assert_eq!(definition.input_schema.0["properties"]["include_screenshot"]["const"], false);
                let pixels = surface::tool("get_desktop_state").unwrap();
                assert!(!projected.input.tools.iter().any(|tool| tool.name == pixels.model_tool));
            }
        }
    }

    #[test]
    fn current_attempt_receipt_is_exact_single_use_and_replaced_by_the_next_request() {
        let (owner, request, mut route, adapter) = fixture();
        route.features = BTreeSet::from([ChatModelFeature::ToolCalls]);
        let projected = owner.project(&request, &route, &adapter).unwrap();
        let tool = surface::tool("list_apps").unwrap();
        let call = ChatToolCall { call_id: "first".into(), name: tool.model_tool.clone(), arguments: StrictJsonValue(json!({})), provider_metadata: None };
        let invocation = invocation(&projected, call.clone());
        assert!(owner.consume(&invocation).is_err(), "no hook response can never obtain a Full fallback");
        owner.begin_attempt(&projected, &route, &adapter).unwrap();
        owner.observe(&projected, &route, &nomifun_chat_model_broker::ChatModelEvent::ToolCallCompleted { call: call.clone() }).unwrap();
        let mut foreign = invocation.clone(); foreign.turn_operation_id = "another-turn".into();
        assert!(owner.consume(&foreign).is_err());
        foreign = invocation.clone(); foreign.call.arguments = StrictJsonValue(json!({"changed":true}));
        assert!(owner.consume(&foreign).is_err());
        assert_eq!(owner.consume(&invocation).unwrap(), RouteTier::NoImage);
        assert!(owner.consume(&invocation).is_err());
        owner.begin_attempt(&projected, &route, &adapter).unwrap();
        owner.observe(&projected, &route, &nomifun_chat_model_broker::ChatModelEvent::ToolCallCompleted { call }).unwrap();
        let mut next = projected.clone(); next.causality.operation_id = "next-model-request".into();
        owner.begin_attempt(&next, &route, &adapter).unwrap();
        assert!(owner.consume(&invocation).is_err(), "the previous response is not a cross-request execution ledger");
        owner.clear();
        assert!(owner.current.lock().unwrap().is_none());
    }

    #[test]
    fn an_existing_image_remains_required_after_no_image_tool_projection() {
        let (owner, mut request, mut route, adapter) = fixture();
        request.input.messages.push(nomifun_chat_model_broker::ChatMessage { role: nomifun_chat_model_broker::ChatRole::User,
            content: vec![ChatContentPart::Image { media_type: "image/png".into(), data_base64: "AA==".into() }], provider_round_id: None });
        route.features = BTreeSet::from([ChatModelFeature::ToolCalls]);
        let projected = owner.project(&request, &route, &adapter).unwrap();
        assert_eq!(projected.input.messages, request.input.messages);
        assert!(projected.input.required_features().contains(&ChatModelFeature::ImageInput));
        assert!(!route.features.is_superset(&projected.input.required_features()));
    }

    #[test]
    fn a_foreign_tool_with_a_computer_like_name_keeps_its_own_schema_and_no_computer_receipt() {
        let (_, mut request, mut route, adapter) = fixture();
        let frozen = surface::tool("get_window_state").unwrap();
        request.input.tools = vec![ChatToolDefinition { name: frozen.model_tool.clone(), description: "A different admitted domain".into(),
            input_schema: StrictJsonValue(json!({"type":"object","properties":{"custom":{"type":"string"}},"required":["custom"]})), deferred: false }];
        let owner = ComputerAttemptSurface::new(request.causality.agent_session_id.clone(), request.causality.resolved_snapshot_ref.clone());
        let definition = request.input.tools[0].clone();
        let binding = EngineToolBinding { model_name: definition.name.clone(), definition: definition.clone(),
            schema_digest: nomifun_engine_core::input_schema_digest(&definition.input_schema).unwrap(), canonical_input_schema_ref: "schema://other-domain/input".into(),
            capability_contract_digest: "b".repeat(64).into(), capability_id: "other.domain".into(), action_id: "other.domain/custom".into(),
            resource_binding_ids: BTreeSet::new(), effect_class: EngineEffectClass::ReadOnly, parallel_safe: true };
        owner.bind_plan(&nomifun_engine_core::EngineToolPlan::new([binding]).unwrap()).unwrap();
        assert!(!owner.has_tools());
        for features in [BTreeSet::new(), BTreeSet::from([ChatModelFeature::ToolCalls])] {
            route.features = features;
            let projected = owner.project(&request, &route, &adapter).unwrap();
            assert_eq!(projected, request, "candidate projection cannot identify another domain by a Computer-like name");
            owner.begin_attempt(&projected, &route, &adapter).unwrap();
            owner.observe(&projected, &route, &nomifun_chat_model_broker::ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "foreign".into(), name: frozen.model_tool.clone(), arguments: StrictJsonValue(json!({"custom":"value"})), provider_metadata: None,
            }}).unwrap();
            assert!(owner.current.lock().unwrap().is_none());
        }
    }
}
