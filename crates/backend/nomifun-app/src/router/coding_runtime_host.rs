//! Coding runtime on the production Conversation owner, Broker and Kernel.
//! No SessionStore, private transcript, provider client or native tool bypass.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_agent_contracts::*;
use nomifun_agent_control_plane::{AgentControlPlane, AuthenticatedOwner};
use nomifun_agent_kernel::{CompilerEnvironment, KernelRegistry, SessionCapabilityState};
use nomifun_ai_agent::coding_runtime::{CodingAgentRuntime, CodingRuntimeHost};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{RuntimeEngineDescriptor, RuntimeEngineFactory};
use nomifun_api_types::RuntimeEngineBinding;
use nomifun_chat_model_broker::*;
use nomifun_coding_engine::*;
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::nomi_core_session::{
    NomiCoreSessionOwner, compile_nomi_plugin_snapshot, session_metadata,
};

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Coding host: {value}"))
}

pub(crate) fn descriptor() -> RuntimeEngineDescriptor {
    RuntimeEngineDescriptor {
        family_id: "nomifun.coding".into(),
        build_id: format!("{}-host1", env!("CARGO_PKG_VERSION")),
        build_digest: format!(
            "{:x}",
            Sha256::digest(
                concat!(
                    include_str!("../../../../../Cargo.lock"),
                    include_str!("../../../nomifun-coding-engine/src/engine.rs"),
                    include_str!("../../../nomifun-coding-engine/src/turn.rs"),
                    include_str!("../../../nomifun-coding-engine/src/kernel.rs"),
                    include_str!("../../../nomifun-ai-agent/src/coding_runtime.rs"),
                    include_str!("coding_runtime_host.rs")
                )
                .as_bytes()
            )
        ),
        display_name: "Coding (workspace tools)".into(),
        host_contract_version: nomifun_api_types::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["coding".into()],
    }
}

pub(crate) fn factory(
    owner: Arc<NomiCoreSessionOwner>,
    control_plane: Arc<AgentControlPlane>,
    kernel: Arc<KernelRegistry>,
    environment: CompilerEnvironment,
    pool: SqlitePool,
    encryption_key: [u8; 32],
) -> RuntimeEngineFactory {
    let owner = Arc::downgrade(&owner);
    let broker = super::chat_broker_host::ChatBrokerHostComposition::for_nomi_core(
        pool.clone(),
        encryption_key,
    );
    Arc::new(move |options, binding| {
        let (owner, control_plane, kernel, environment, pool, broker) = (
            owner.clone(),
            control_plane.clone(),
            kernel.clone(),
            environment.clone(),
            pool.clone(),
            broker.clone(),
        );
        Box::pin(async move {
            let owner = owner
                .upgrade()
                .ok_or_else(|| error("Session owner has shut down"))?;
            let response = owner
                .get_session(&options.user_id, &options.conversation_id)
                .await?;
            validate_session_extra(&response.extra)?;
            if super::runtime_engines::binding_from_extra(&response.extra)?.as_ref()
                != Some(&binding)
            {
                return Err(error(
                    "runtime options differ from the durable engine binding",
                ));
            }
            let authenticated = AuthenticatedOwner(UserId::from(options.user_id.clone()));
            let metadata =
                session_metadata(&response, &authenticated).map_err(|e| error(e.message))?;
            let dto =
                serde_json::from_value(serde_json::to_value(&metadata.binding).map_err(error)?)
                    .map_err(error)?;
            let (mut agent_binding, revision, snapshot) = control_plane
                .saved_binding_artifacts(&authenticated.0, &dto)
                .await
                .map_err(error)?;
            validate_supported_snapshot(&snapshot)?;
            let route = snapshot
                .content
                .chat_route_identity
                .clone()
                .ok_or_else(|| error("snapshot has no exact Chat route"))?;
            let principal = PrincipalRef {
                principal_kind: "user".into(),
                principal_id: options.user_id.clone(),
            };
            let session_id = AgentSessionId::from(options.conversation_id.clone());
            let workspace_tools = snapshot.content.initial_capabilities.iter().any(|item| {
                super::nomi_core_wave2::coding_capability_ids().contains(&item.capability.id)
            });
            if workspace_tools {
                let resources = agent_binding
                    .typed_resource_bindings
                    .iter()
                    .filter(|resource| {
                        resource.resource_kind.as_ref() == nomifun_file::WORKSPACE_RESOURCE_KIND
                    })
                    .collect::<Vec<_>>();
                let [authority] = resources.as_slice() else {
                    return Err(error(
                        "workspace tools require one server-resolved workspace resource",
                    ));
                };
                let workspace = super::nomi_core_wave2::session_workspace_binding(
                    &options.workspace,
                    &principal,
                    &session_id,
                    authority,
                )?;
                agent_binding.typed_resource_bindings =
                    super::nomi_core_wave2::with_session_workspace_binding(
                        agent_binding.typed_resource_bindings,
                        workspace,
                    );
            }
            let compiled = Arc::new(compile_nomi_plugin_snapshot(
                &kernel,
                &environment,
                agent_binding,
                revision,
                snapshot,
                &principal,
            )?);
            let active = Arc::new(SessionCapabilityState::new(&compiled));
            let active_snapshot = active.snapshot().map_err(error)?;
            let registry = kernel.snapshot().map_err(error)?;
            let mut exposures = standard_coding_tool_exposures(StandardCodingToolLevel::Full);
            exposures.retain(|item| active_snapshot.active.contains(&item.capability_id));
            // Use the owner's canonical input schemas, never the convenience
            // presentation schema as an independent permission contract.
            for exposure in &mut exposures {
                let capability = registry
                    .capability(&exposure.capability_id)
                    .ok_or_else(|| error("tool is not materialized"))?;
                let action = capability
                    .manifest
                    .contributions
                    .actions
                    .iter()
                    .find(|action| action.action_id == exposure.action_id)
                    .ok_or_else(|| error("tool action is unavailable"))?;
                exposure.definition.input_schema =
                    nomifun_agent_domain_wave2::resolve_action_schema(
                        exposure.capability_id.as_ref(),
                        &action.input_schema,
                    )
                    .map_err(error)?;
            }
            let plan = compile_coding_tool_plan(&compiled, &active_snapshot, &registry, exposures)
                .map_err(error)?;
            let tools = Arc::new(JoinedTools::new(Arc::new(KernelCodingToolInvoker::new(
                kernel,
                compiled.clone(),
                active,
                principal.clone(),
                ScopeKey::from(format!("session:{}", options.conversation_id)),
            ))));
            let host = Arc::new(ConversationCodingHost {
                owner,
                pool,
                options: options.clone(),
                binding: binding.clone(),
                snapshot_ref: compiled.snapshot_ref().clone(),
                route,
                plan,
                principal,
                generation: active_snapshot.generation,
                active: tokio::sync::Mutex::new(None),
                tools: tools.clone(),
            });
            let model_invoke = broker.build_model_invoke(reqwest::Client::new());
            let model = Arc::new(BrokerCodingModelPort::new(
                broker
                    .build_broker(host.clone(), model_invoke, BrokerRetryPolicy::default())
                    .map_err(error)?,
            ));
            let build = CodingEngineBuild {
                family_id: binding.family_id.clone().into(),
                build_id: binding.build_id.clone().into(),
                build_digest: binding.build_digest.clone().into(),
                display_name: descriptor().display_name,
                supported_profiles: vec![CodingRuntimeProfile::Coding],
            };
            let engine = Arc::new(CodingEngine::new(build).map_err(error)?);
            let engine_binding = EngineBinding::new(
                session_id,
                RuntimeBindingId::from(format!("conversation-runtime:{}", options.conversation_id)),
                binding.family_id.into(),
                binding.build_id.into(),
                binding.build_digest.into(),
                CodingRuntimeProfile::Coding,
                compiled.snapshot_ref().clone(),
            )
            .map_err(error)?;
            let runtime =
                CodingAgentRuntime::new(&options, engine, engine_binding, model, tools, host)?;
            Ok(Arc::new(runtime) as Arc<dyn nomifun_ai_agent::RegisteredAgentRuntime>)
        })
    })
}

/// Explicit first production surface. Never silently omit unsupported
/// middleware, on-demand activation, Skills, MCP or long-lived process owners.
pub(crate) fn validate_supported_snapshot(
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), AppError> {
    let supported = super::nomi_core_wave2::coding_capability_ids();
    let unsupported = snapshot
        .content
        .initial_capabilities
        .iter()
        .filter(|item| !supported.contains(&item.capability.id))
        .map(|item| item.capability.id.as_ref())
        .collect::<Vec<_>>();
    if !unsupported.is_empty()
        || !snapshot.content.on_demand_capabilities.is_empty()
        || !snapshot.content.skill_locks.is_empty()
        || !snapshot.content.initial_miniapp_capabilities.is_empty()
        || !snapshot.content.on_demand_miniapp_capabilities.is_empty()
    {
        return Err(error(format!(
            "this Coding build supports initial workspace tools only; unsupported capabilities: {}. On-demand, Skills and MiniApps require another installed build",
            unsupported.join(", ")
        )));
    }
    Ok(())
}

fn validate_session_extra(extra: &Value) -> Result<(), AppError> {
    for key in ["skills", "mcp_server_ids", "mcp_servers"] {
        if extra
            .get(key)
            .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
        {
            return Err(error(format!(
                "this Coding build does not support {key}; deselect these capabilities explicitly"
            )));
        }
    }
    Ok(())
}

struct ActiveTurn {
    root: String,
    operation: String,
    epoch: i64,
    sequence: i64,
    cancellation: CancellationToken,
}

struct ConversationCodingHost {
    owner: Arc<NomiCoreSessionOwner>,
    pool: SqlitePool,
    options: AgentRuntimeBuildOptions,
    binding: RuntimeEngineBinding,
    snapshot_ref: ResolvedSnapshotRef,
    route: ChatRouteSelection,
    plan: CodingToolPlan,
    principal: PrincipalRef,
    generation: u64,
    active: tokio::sync::Mutex<Option<ActiveTurn>>,
    tools: Arc<JoinedTools>,
}

impl ConversationCodingHost {
    async fn append_record(
        &self,
        root: &str,
        payload: String,
        model_operation: Option<&str>,
        terminal: bool,
    ) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("event without admitted turn"))?;
        if root != turn.root {
            return Err(error("event root mismatch"));
        }
        sqlx::query("INSERT INTO conversation_runtime_events (conversation_id, turn_operation_id, sequence, event_json, model_operation_id, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&self.options.conversation_id).bind(&turn.operation).bind(turn.sequence + 1).bind(payload).bind(model_operation).bind(nomifun_common::now_ms()).execute(&self.pool).await.map_err(error)?;
        turn.sequence += 1;
        if terminal {
            *active = None;
        }
        Ok(())
    }
}

#[async_trait]
impl CodingRuntimeHost for ConversationCodingHost {
    async fn prepare_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
    ) -> Result<CodingTurnRequest, AppError> {
        let root = message
            .source_message_id
            .as_deref()
            .unwrap_or(&message.msg_id);
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT r.operation_id, c.admission_epoch, m.content FROM conversations c \
             JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
             JOIN messages m ON m.message_id = r.message_id AND m.conversation_id = c.conversation_id \
             WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' \
             AND r.user_id = c.user_id AND r.conversation_id = c.conversation_id \
             AND r.message_id = ? AND r.kind = 'turn' AND r.status = 'accepted'")
            .bind(&self.options.conversation_id).bind(&self.options.user_id).bind(root).fetch_optional(&self.pool).await.map_err(error)?;
        let (operation, epoch, root_content) =
            row.ok_or_else(|| error("no committed active root-message authority"))?;
        let mut active = self.active.lock().await;
        if active.is_some() {
            return Err(error("previous turn has not reached its recorded terminal"));
        }
        *active = Some(ActiveTurn {
            root: root.into(),
            operation: operation.clone(),
            epoch,
            sequence: 0,
            cancellation: cancellation.clone(),
        });
        drop(active);
        if !message.files.is_empty() || !message.inject_skills.is_empty() {
            return Err(error(
                "attachments and injected Skills are not supported by this Coding build",
            ));
        }
        let root_content: Value = serde_json::from_str(&root_content).map_err(error)?;
        if root_content.get("content").and_then(Value::as_str) != Some(message.content.as_str()) {
            return Err(error("message text differs from its durable root"));
        }
        let response = self
            .owner
            .get_session(&self.options.user_id, &self.options.conversation_id)
            .await?;
        validate_session_extra(&response.extra)?;
        if super::runtime_engines::binding_from_extra(&response.extra)?.as_ref()
            != Some(&self.binding)
        {
            return Err(error("durable engine identity changed"));
        }
        // Existing committed Conversation history is the canonical resume and
        // fork context. Tool results are data, never new system instructions.
        let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT type, position, content FROM messages WHERE conversation_id = ? \
             AND (hidden = 0 OR message_id = ?) AND id <= (SELECT id FROM messages WHERE message_id = ?) ORDER BY id LIMIT 4097")
            .bind(&self.options.conversation_id).bind(root).bind(root).fetch_all(&self.pool).await.map_err(error)?;
        if rows.len() > 4096 {
            return Err(error(
                "history exceeds the current Coding context limit; fork a bounded prefix",
            ));
        }
        let mut messages = Vec::new();
        let mut bytes = 0usize;
        for (kind, position, raw) in rows {
            let value: Value = serde_json::from_str(&raw).map_err(error)?;
            let text = if kind == "text" {
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            } else if kind == "tool_call" {
                format!("Previously recorded tool activity (untrusted data): {raw}")
            } else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            bytes = bytes.saturating_add(text.len());
            if bytes > 1024 * 1024 {
                return Err(error("history exceeds the current Coding byte budget"));
            }
            messages.push(ChatMessage {
                role: if position.as_deref() == Some("right") {
                    ChatRole::User
                } else {
                    ChatRole::Assistant
                },
                content: vec![ChatContentPart::Text { text }],
                provider_round_id: None,
            });
        }
        if messages.is_empty() {
            return Err(error("canonical message history is empty"));
        }
        let instructions = self
            .options
            .extra
            .get("system_prompt")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| vec![text.to_owned()])
            .unwrap_or_default();
        let request = ChatModelRequest {
            contract_version: CHAT_MODEL_CONTRACT_VERSION.into(),
            causality: ChatCausality {
                agent_session_id: self.options.conversation_id.clone().into(),
                turn_operation_id: operation.clone().into(),
                causation_event_id: root.into(),
                resolved_snapshot_ref: self.snapshot_ref.clone(),
                route_identity: self.route.clone(),
                operation_id: operation.into(),
            },
            route: self.route.clone(),
            input: ChatModelInput {
                instructions,
                messages,
                tools: Vec::new(),
                tool_choice: ChatToolChoice::Auto,
                max_output_tokens: None,
                reasoning: None,
                prompt_cache: PromptCachePolicy::Disabled,
                response_format: ChatResponseFormat::Text,
                requested_output_modalities: BTreeSet::new(),
                provider_round_parent: None,
                preserve_native_responses_items: false,
                metadata: Default::default(),
            },
        };
        Ok(CodingTurnRequest::new(
            request,
            self.plan.clone(),
            self.principal.clone(),
            self.generation,
        ))
    }

    async fn record_event(
        &self,
        message: &SendMessageData,
        event: &CodingEngineEvent,
    ) -> Result<(), AppError> {
        let model_operation = match event {
            CodingEngineEvent::ModelStepStarted { operation_id, .. } => Some(operation_id.as_ref()),
            _ => None,
        };
        let payload = serde_json::to_string(event).map_err(error)?;
        let terminal = matches!(
            event,
            CodingEngineEvent::TurnCompleted { .. }
                | CodingEngineEvent::TurnCancelled { .. }
                | CodingEngineEvent::TurnFailed { .. }
        );
        self.append_record(
            message
                .source_message_id
                .as_deref()
                .unwrap_or(&message.msg_id),
            payload,
            model_operation,
            terminal,
        )
        .await
    }

    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError> {
        self.tools.join().await?;
        // Effects already admitted may settle after the model loop was
        // cancelled. Preserve their outcomes before publishing cancellation;
        // a cancelled turn never implies its filesystem effects were undone.
        let settled = self
            .tools
            .settled
            .lock()
            .map_err(|_| error("tool outcomes poisoned"))?
            .clone();
        for payload in &settled {
            self.append_record(
                message
                    .source_message_id
                    .as_deref()
                    .unwrap_or(&message.msg_id),
                payload.clone(),
                None,
                false,
            )
            .await?;
        }
        self.tools
            .settled
            .lock()
            .map_err(|_| error("tool outcomes poisoned"))?
            .clear();
        Ok(())
    }
    async fn cleanup_session(&self) -> Result<(), AppError> {
        self.tools.join().await
    }
}

#[async_trait]
impl ChatCausalityGate for ConversationCodingHost {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        let reject = |reason: &str| {
            ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                reason,
                ChatRetryDirective::Never,
            )
        };
        let active = self.active.lock().await;
        let turn = active
            .as_ref()
            .ok_or_else(|| reject("no active Coding turn"))?;
        if turn.cancellation.is_cancelled()
            || causality.agent_session_id.as_ref() != self.options.conversation_id
            || causality.turn_operation_id.as_ref() != turn.operation
            || causality.causation_event_id.as_ref() != turn.root
            || causality.resolved_snapshot_ref != self.snapshot_ref
            || causality.route_identity != self.route
        {
            return Err(reject(
                "Coding request differs from its admitted Conversation authority",
            ));
        }
        // Claim the recorded model operation once, under the durable owner's
        // active generation. Retries inside the Broker retain this one claim.
        let result = sqlx::query("UPDATE conversation_runtime_events SET model_claimed = 1 \
            WHERE conversation_id = ? AND turn_operation_id = ? AND model_operation_id = ? AND model_claimed = 0 \
            AND EXISTS (SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
                WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' AND c.admission_epoch = ? \
                AND c.active_turn_operation_id = ? AND r.status = 'accepted' AND r.message_id = ?)")
            .bind(&self.options.conversation_id).bind(&turn.operation).bind(causality.operation_id.as_ref())
            .bind(&self.options.conversation_id).bind(&self.options.user_id).bind(turn.epoch).bind(&turn.operation).bind(&turn.root)
            .execute(&self.pool).await.map_err(|_| reject("cannot establish durable model authority"))?;
        if result.rows_affected() != 1 {
            return Err(reject(
                "model operation already claimed or Conversation generation fenced",
            ));
        }
        Ok(())
    }
}

/// Kernel calls run in owned tasks. Cancellation closes the caller, but never
/// drops an in-flight filesystem/VCS effect and calls that cleanup. The host
/// joins all tasks before terminal publication or workspace lease release.
struct JoinedTools {
    inner: Arc<dyn CodingToolInvoker>,
    tasks: Mutex<Vec<Shared<BoxFuture<'static, Result<(), String>>>>>,
    failed: std::sync::atomic::AtomicBool,
    settled: Arc<Mutex<Vec<String>>>,
}
impl JoinedTools {
    fn new(inner: Arc<dyn CodingToolInvoker>) -> Self {
        Self {
            inner,
            tasks: Mutex::new(Vec::new()),
            failed: false.into(),
            settled: Arc::new(Mutex::new(Vec::new())),
        }
    }
    async fn join(&self) -> Result<(), AppError> {
        // Keep completion witnesses in the owner if a teardown caller itself
        // times out. A retry must wait for those same tasks, not an empty list.
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| error("tool ownership lock poisoned"))?
            .clone();
        for task in tasks {
            if task.await.is_err() {
                self.failed
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
        self.tasks
            .lock()
            .map_err(|_| error("tool ownership lock poisoned"))?
            .retain(|task| task.peek().is_none());
        if self.failed.load(std::sync::atomic::Ordering::Acquire) {
            Err(error("tool task panicked; cleanup cannot be proven"))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl CodingToolInvoker for JoinedTools {
    async fn invoke(
        &self,
        invocation: CodingToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<CodingToolResult, CodingEngineError> {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let inner = self.inner.clone();
        let settled = self.settled.clone();
        {
            let mut tasks = self
                .tasks
                .lock()
                .map_err(|_| CodingEngineError::ToolInvocation("tool owner poisoned".into()))?;
            let task = tokio::spawn(async move {
                let identity = (
                    invocation.operation_id.clone(),
                    invocation.call.call_id.clone(),
                );
                let result = inner.invoke(invocation, CancellationToken::new()).await;
                let payload = serde_json::json!({
                    "event": "host_tool_settled", "operation_id": identity.0, "call_id": identity.1,
                    "result": result.as_ref().ok(), "error": result.as_ref().err().map(ToString::to_string),
                }).to_string();
                settled
                    .lock()
                    .expect("tool outcomes lock poisoned")
                    .push(payload);
                let _ = tx.send(result);
            });
            tasks.push(
                async move { task.await.map_err(|error| error.to_string()) }
                    .boxed()
                    .shared(),
            );
        }
        tokio::select! {
            _ = cancellation.cancelled() => Err(CodingEngineError::Cancelled),
            result = rx => result.map_err(|_| CodingEngineError::ToolInvocation("tool task panicked".into()))?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NeverInvoke;
    #[async_trait]
    impl CodingToolInvoker for NeverInvoke {
        async fn invoke(
            &self,
            _: CodingToolInvocation,
            _: CancellationToken,
        ) -> Result<CodingToolResult, CodingEngineError> {
            panic!("not used")
        }
    }

    #[tokio::test]
    async fn teardown_timeout_retains_the_same_completion_witness() {
        let owner = JoinedTools::new(Arc::new(NeverInvoke));
        let release = CancellationToken::new();
        let token = release.clone();
        let task = tokio::spawn(async move {
            token.cancelled().await;
        });
        owner.tasks.lock().unwrap().push(
            async move { task.await.map_err(|e| e.to_string()) }
                .boxed()
                .shared(),
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        assert_eq!(owner.tasks.lock().unwrap().len(), 1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        release.cancel();
        owner.join().await.unwrap();
        owner.join().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_caller_retains_effect_until_exit_and_records_its_outcome() {
        struct DelayedEffect {
            started: CancellationToken,
            release: CancellationToken,
        }
        #[async_trait]
        impl CodingToolInvoker for DelayedEffect {
            async fn invoke(
                &self,
                invocation: CodingToolInvocation,
                cancellation: CancellationToken,
            ) -> Result<CodingToolResult, CodingEngineError> {
                self.started.cancel();
                self.release.cancelled().await;
                assert!(
                    !cancellation.is_cancelled(),
                    "admitted effects must not lose their completion witness"
                );
                Ok(CodingToolResult::text(
                    invocation.call.call_id,
                    "effect committed",
                    false,
                ))
            }
        }
        let started = CancellationToken::new();
        let release = CancellationToken::new();
        let owner = Arc::new(JoinedTools::new(Arc::new(DelayedEffect {
            started: started.clone(),
            release: release.clone(),
        })));
        let invocation = CodingToolInvocation {
            agent_session_id: "session".into(), principal: PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "a".repeat(64).into() },
            active_set_generation: 1, turn_operation_id: "turn".into(), operation_id: "tool-operation".into(), idempotency_key: "key".into(), correlation_id: "correlation".into(),
            call: ChatToolCall { call_id: "call".into(), name: "write_file".into(), arguments: StrictJsonValue(serde_json::json!({})), provider_metadata: Default::default() },
            binding: serde_json::from_value(serde_json::json!({
                "model_name":"write_file", "definition":{"name":"write_file","description":"fixture","input_schema":{}},
                "schema_digest":"a".repeat(64), "canonical_input_schema_ref":"fixture", "capability_contract_digest":"b".repeat(64),
                "capability_id":"fs.write", "action_id":"write", "resource_binding_ids":[], "effect_class":"managed_effect", "parallel_safe":false
            })).unwrap(),
        };
        let cancellation = CancellationToken::new();
        let caller_owner = owner.clone();
        let token = cancellation.clone();
        let caller = tokio::spawn(async move { caller_owner.invoke(invocation, token).await });
        tokio::time::timeout(std::time::Duration::from_secs(1), started.cancelled())
            .await
            .unwrap();
        cancellation.cancel();
        assert!(matches!(
            caller.await.unwrap(),
            Err(CodingEngineError::Cancelled)
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), owner.join())
                .await
                .is_err()
        );
        release.cancel();
        owner.join().await.unwrap();
        let settled = owner.settled.lock().unwrap();
        assert_eq!(settled.len(), 1);
        let event: Value = serde_json::from_str(&settled[0]).unwrap();
        assert_eq!(event["operation_id"], "tool-operation");
        assert_eq!(event["result"]["is_error"], false);
        assert!(event.to_string().contains("effect committed"));
    }

    #[tokio::test]
    async fn tool_panic_permanently_refuses_cleanup_proof() {
        let owner = JoinedTools::new(Arc::new(NeverInvoke));
        let task = tokio::spawn(async { panic!("fixture tool panic") });
        owner.tasks.lock().unwrap().push(
            async move { task.await.map_err(|e| e.to_string()) }
                .boxed()
                .shared(),
        );
        assert!(owner.join().await.is_err());
        assert!(owner.join().await.is_err());
    }
}
