//! Conversation-domain capabilities (registry form): list / status / send /
//! create / update / delete. Creation is companion-only on this Agent-facing
//! surface. All self-protection guards from the legacy tool are preserved (no
//! self-injection, no self-model-change, no self-deletion), and nomi sessions
//! still get a model at creation via the shared resolution chain so downstream
//! consumers never see a model-less nomi conversation.

use std::sync::Arc;

use nomifun_api_types::{
    CreateConversationRequest, ListConversationsQuery, ListMessagesQuery, SendMessageRequest,
    UpdateConversationRequest,
};
use nomifun_common::{
    AgentType, AppError, CompanionId, ConversationId, ProviderWithModel, RemoteAgentId,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::id_schema::ModelRefParam;
use crate::provider_support;
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

const DEFAULT_LIST_LIMIT: u32 = 50;
const DEFAULT_MESSAGE_LIMIT: u32 = 5;
/// Per-message content budget in status output — keeps a busy transcript from
/// blowing up the calling agent's context.
const MESSAGE_SNIPPET_CHARS: usize = 500;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListConversationsParams {
    /// Maximum number of conversations to return (default 50).
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConversationStatusParams {
    /// The id of the conversation to inspect.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    conversation_id: ConversationId,
    /// How many recent messages to include (default 5, max 50).
    #[serde(default)]
    message_limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendToConversationParams {
    /// The id of the TARGET conversation (not your own).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    conversation_id: ConversationId,
    /// The message or task prompt to inject.
    content: String,
    /// When true the message is hidden from the visible history (use for
    /// background task prompts, like AutoWork does).
    #[serde(default)]
    hidden: Option<bool>,
    /// When true (default false), you will automatically receive a completion
    /// receipt message in YOUR conversation once the target finishes this
    /// task — use it for fire-and-forget delegation instead of polling
    /// nomi_conversation_status.
    #[serde(default)]
    notify_back: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateSummonParams {
    /// The companion to summon — normally your own id (from nomi_whoami).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    companion_id: CompanionId,
    /// Hand-picked memory ids to load read-only (pre-select with
    /// recall_memories; the owner can trim them later in the summon panel).
    #[serde(default)]
    memory_ids: Vec<String>,
    /// Active skills to EXCLUDE from materialization (default: none — the
    /// full active skill set loads).
    #[serde(default)]
    skill_exclusions: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateConversationParams {
    /// Optional display name for the new conversation.
    #[serde(default)]
    name: Option<String>,
    /// Agent type. "nomi" — the native executor — is the only accepted value
    /// and the default, so omit it. NOT for terminals: any terminal/shell
    /// intent must go through nomi_create_terminal instead.
    #[serde(default)]
    agent_type: Option<String>,
    /// Exact provider/model pair for the new session. Omit to auto-resolve:
    /// your own companion model → first configured provider.
    #[serde(default)]
    model: Option<ModelRefParam>,
    /// Retired parameter. Remote agents are no longer an engine — passing it is
    /// rejected. Kept declared only so a stale caller gets that explanation
    /// instead of an unknown-field parse failure.
    #[serde(default)]
    #[schemars(schema_with = "crate::id_schema::optional_canonical_uuid_v7_schema")]
    remote_agent_id: Option<RemoteAgentId>,
    /// Absolute project path the user gave you. Sets the conversation's
    /// workspace ("project session", grouped under that workpath in the
    /// sidebar). Omit for an auto-provisioned workspace.
    #[serde(default)]
    workpath: Option<String>,
    /// Summon a companion into the new work session (spec 召唤伙伴): loads its
    /// skills plus the selected memories read-only. The server stamps
    /// summoned_at.
    #[serde(default)]
    summon: Option<CreateSummonParams>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateConversationParams {
    /// The id of the conversation to update (from nomi_list_conversations).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    conversation_id: ConversationId,
    /// New display name (omit to keep).
    #[serde(default)]
    name: Option<String>,
    /// Pin (true) or unpin (false) the conversation in the sidebar.
    #[serde(default)]
    pinned: Option<bool>,
    /// Exact replacement provider/model pair (nomi conversations only).
    #[serde(default)]
    model: Option<ModelRefParam>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteConversationParams {
    /// The id of the conversation to delete. Confirm the target with the user
    /// before calling — deletion also kills its agent and cron bindings.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    conversation_id: ConversationId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopConversationParams {
    /// The id of the conversation whose current turn should be stopped
    /// (from nomi_list_conversations / nomi_conversation_status).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    conversation_id: ConversationId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WhoamiParams {}

fn error_value(e: AppError) -> Value {
    json!({ "error": e.to_string() })
}

fn require_conversation_creator(ctx: &CallerCtx) -> Result<(), Value> {
    if ctx.companion_id.is_some() || ctx.remote {
        Ok(())
    } else {
        Err(json!({
            "error": "conversation_creation_forbidden: top-level conversations may only be created by the user, scheduled jobs, or a companion; use nomi_delegate for multi-Agent work inside the current conversation"
        }))
    }
}

async fn list(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ListConversationsParams) -> Value {
    let user_id = ctx.user_id.as_str();
    let query = ListConversationsQuery {
        limit: Some(p.limit.unwrap_or(DEFAULT_LIST_LIMIT)),
        ..Default::default()
    };
    // Exclude the companion's own work-partner single sessions from the page + total.
    let resp = match deps.conversation_service.list(user_id, query, true).await {
        Ok(r) => r,
        Err(e) => return error_value(e),
    };
    let mut items = Vec::with_capacity(resp.items.len());
    for conv in resp.items {
        let runtime = deps
            .conversation_service
            .runtime_summary_for(conv.conversation_id.as_str())
            .await;
        let companion_id = match conv.extra.get("companion_id") {
            None => None,
            Some(Value::String(value)) => match CompanionId::parse(value) {
                Ok(id) => Some(id),
                Err(error) => {
                    return json!({"error": format!("conversation {} has invalid companion_id: {error}", conv.conversation_id)});
                }
            },
            Some(_) => {
                return json!({"error": format!("conversation {} has non-string companion_id", conv.conversation_id)});
            }
        };
        items.push(json!({
            "conversation_id": conv.conversation_id,
            "name": conv.name,
            "agent_type": conv.r#type,
            "status": conv.status,
            "runtime_state": runtime.state,
            "pending_confirmations": runtime.pending_confirmations,
            "source": conv.source,
            "pinned": conv.pinned,
            "is_companion_companion": conv.extra.get("companion_session").and_then(Value::as_bool).unwrap_or(false),
            "companion_id": companion_id,
            "is_self": ctx.conversation_id.as_ref().is_some_and(|id| conv.conversation_id.as_str() == id.as_str()),
            "modified_at": conv.modified_at,
        }));
    }
    ok(json!({ "total": resp.total, "conversations": items }))
}

/// Restart-isolation ("stuck") assessment for one conversation (spec D3).
///
/// A conversation is stuck when its durable status is still `running` but no
/// live runtime exists in this process: after a backend restart the running
/// authority is fail-closed quarantined and nothing will finish it without an
/// explicit stop. Returns the flag plus the remote-facing unlock hint.
fn stuck_assessment(
    is_persisted_running: bool,
    has_runtime: bool,
) -> (bool, Option<&'static str>) {
    let stuck = is_persisted_running && !has_runtime;
    (
        stuck,
        stuck.then_some(
            "该会话因后端重启被保护性挂起，可用 nomi_stop_conversation 解除后重试",
        ),
    )
}

async fn status(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ConversationStatusParams) -> Value {
    let user_id = ctx.user_id.as_str();
    let id = p.conversation_id.as_str();
    let conv = match deps.conversation_service.get(user_id, id).await {
        Ok(c) => c,
        Err(e) => return error_value(e),
    };
    let runtime = deps.conversation_service.runtime_summary_for(id).await;
    let (stuck, stuck_hint) = stuck_assessment(
        conv.status == nomifun_common::ConversationStatus::Running,
        runtime.has_runtime,
    );
    let last_receipt = match deps
        .conversation_service
        .latest_completed_turn_receipt(user_id, id)
        .await
    {
        Ok(receipt) => receipt,
        Err(e) => return error_value(e),
    };
    let message_limit = p.message_limit.unwrap_or(DEFAULT_MESSAGE_LIMIT).clamp(1, 50);
    let messages = match deps
        .conversation_service
        .list_messages(
            user_id,
            id,
            ListMessagesQuery {
                page: Some(1),
                page_size: Some(message_limit),
                order: Some("desc".to_owned()),
                content_mode: None,
                cursor: None,
                day: None,
            },
        )
        .await
    {
        Ok(m) => m,
        Err(e) => return error_value(e),
    };
    let messages_json = match serde_json::to_value(&messages) {
        Ok(v) => truncate_message_contents(v),
        Err(e) => return json!({ "error": format!("failed to serialize messages: {e}") }),
    };
    ok(json!({
        "conversation_id": conv.conversation_id,
        "name": conv.name,
        "agent_type": conv.r#type,
        "status": conv.status,
        "runtime": runtime,
        "stuck": stuck,
        "stuck_hint": stuck_hint,
        "last_result_error_code": last_receipt
            .as_ref()
            .and_then(|receipt| receipt.result_error_code.clone()),
        "recent_messages": messages_json,
    }))
}

async fn send(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: SendToConversationParams) -> Value {
    let user_id = ctx.user_id.as_str().to_owned();
    let id = p.conversation_id.into_string();
    if ctx.conversation_id.as_ref().is_some_and(|caller| id == caller.as_str()) {
        return json!({ "error": "self_injection_forbidden: you cannot send a message into your own conversation" });
    }
    let Some(operation_id) = ctx.operation_id.as_deref() else {
        return json!({
            "error": "missing_idempotency_key: conversation send requires an authenticated operation identity"
        });
    };
    // Spec D2: register the completion receipt BEFORE the send — a fast
    // target turn may otherwise complete before the registration lands and
    // the observer would find nothing to take. An orphaned registration from
    // a failed send is inert (nothing ever completes that operation).
    let mut notify_note: Option<&'static str> = None;
    if p.notify_back.unwrap_or(false) {
        match ctx.conversation_id.as_ref() {
            None => {
                notify_note =
                    Some("notify_back ignored: the caller has no conversation to notify");
            }
            Some(requester) => {
                match deps
                    .conversation_service
                    .register_delivery_notify(&user_id, &id, operation_id, requester.as_str())
                    .await
                {
                    Ok(nomifun_conversation::DeliveryNotifyRegistration::Registered) => {
                        notify_note = Some(
                            "notify_back registered: you will receive a receipt message when the target finishes",
                        );
                    }
                    Ok(
                        nomifun_conversation::DeliveryNotifyRegistration::RefusedDeliveryNotifyOrigin,
                    ) => {
                        notify_note = Some(
                            "notify_back ignored: a delivery-notify receipt turn cannot register further receipts (loop guard)",
                        );
                    }
                    Err(e) => return error_value(e),
                }
            }
        }
    }
    let req = SendMessageRequest {
        content: p.content,
        files: vec![],
        inject_skills: vec![],
        hidden: p.hidden.unwrap_or(false),
        origin: Some("companion".into()),
        channel_platform: None,
    };
    match deps
        .conversation_service
        .send_message_with_idempotency_key(
            &user_id,
            &id,
            operation_id,
            req,
            &deps.runtime_registry,
        )
        .await
    {
        Ok(delivery) => ok(json!({
            "msg_id": delivery.message_id,
            "replayed": delivery.replayed,
            "completed": delivery.completed,
            "result_ok": delivery.result_ok,
            "result_text": delivery.result_text,
            "result_error": delivery.result_error,
            "notify_back": notify_note,
            "note": "message accepted; the target session processes it asynchronously — use nomi_conversation_status to follow progress"
        })),
        Err(AppError::Conflict(m)) => json!({
            "error": format!("busy: the target conversation is already running a turn ({m}); check nomi_conversation_status and retry later")
        }),
        Err(e) => error_value(e),
    }
}

/// Normalize the `workpath` create param (spec §B6 反向召唤入口 1).
fn normalized_workpath(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("workpath must be a non-empty project path".into());
    }
    Ok(trimmed.to_owned())
}

/// Compose the server-stamped `extra.summon` value from the create param
/// (spec §B6 反向召唤入口 2). Memory ids are validated + deduped; the caller
/// never controls `summoned_at`.
fn summon_extra_value(summon: &CreateSummonParams, summoned_at: i64) -> Result<Value, String> {
    let mut memory_ids: Vec<String> = Vec::with_capacity(summon.memory_ids.len());
    for id in &summon.memory_ids {
        nomifun_common::CompanionMemoryId::parse(id.as_str())
            .map_err(|error| format!("invalid summon memory id '{id}': {error}"))?;
        if !memory_ids.contains(id) {
            memory_ids.push(id.clone());
        }
    }
    let mut skill_exclusions: Vec<String> = Vec::with_capacity(summon.skill_exclusions.len());
    for name in &summon.skill_exclusions {
        let name = name.trim().to_owned();
        if !name.is_empty() && !skill_exclusions.contains(&name) {
            skill_exclusions.push(name);
        }
    }
    Ok(json!({
        "companion_id": summon.companion_id,
        "memory_ids": memory_ids,
        "skill_exclusions": skill_exclusions,
        "summoned_at": summoned_at,
    }))
}

/// Validate the `agent_type` create param against the single surviving engine.
///
/// The param outlives the multi-engine era on purpose. `CreateConversationParams`
/// is `deny_unknown_fields`, so dropping the field would turn a caller that still
/// sends `agent_type: "nomi"` out of habit into an opaque deserialization
/// failure. Keeping it declared lets that call succeed and lets every other value
/// come back with an actionable message. `"terminal"` keeps its own redirect
/// because a terminal is not a conversation on this surface.
fn validated_agent_type(raw: Option<&str>) -> Result<AgentType, String> {
    let raw = raw.unwrap_or(AgentType::Nomi.serde_name());
    if raw == AgentType::Nomi.serde_name() {
        return Ok(AgentType::Nomi);
    }
    if raw == "terminal" {
        return Err(
            "terminal sessions are not conversations: use nomi_create_terminal (preset shell | claude | codex | gemini) for any terminal/shell intent"
                .to_owned(),
        );
    }
    Err(format!(
        "invalid agent_type '{raw}': the only conversation engine is 'nomi' (the native executor), \
         and it is the default — omit agent_type entirely. For a terminal or agent-CLI session use \
         nomi_create_terminal instead."
    ))
}

async fn create(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CreateConversationParams) -> Value {
    if let Err(error) = require_conversation_creator(&ctx) {
        return error;
    }
    let user_id = ctx.user_id.as_str().to_owned();
    let agent_type = match validated_agent_type(p.agent_type.as_deref()) {
        Ok(t) => t,
        Err(error) => return json!({ "error": error }),
    };
    let mut extra = json!({});
    if p.remote_agent_id.is_some() {
        return json!({ "error": "remote_agent_id is no longer supported" });
    }
    // Project session (spec §B6): a user-given path becomes the workspace —
    // the sidebar groups it under that workpath drawer; `custom_workspace` is
    // derived client-side from a non-empty non-temporary workspace.
    if let Some(workpath) = p.workpath.as_deref() {
        match normalized_workpath(workpath) {
            Ok(path) => extra["workspace"] = json!(path),
            Err(e) => return json!({ "error": e }),
        }
    }
    // Reverse summon (spec §B6): create the work session already carrying the
    // companion's capability pack; the owner can trim it later in the summon
    // panel.
    if let Some(summon) = p.summon.as_ref() {
        match summon_extra_value(summon, nomifun_common::now_ms()) {
            Ok(value) => extra["summon"] = value,
            Err(e) => return json!({ "error": e }),
        }
    }
    let requested_model = p.model.map(ProviderWithModel::from);
    let (model, model_source) =
        match provider_support::resolve_nomi_model(&deps, &ctx, requested_model.as_ref()).await {
            Ok((m, source)) => (Some(m), Some(source)),
            Err(e) => return e,
        };
    let req = CreateConversationRequest {
        r#type: agent_type,
        name: p.name,
        model,
        source: None,
        channel_chat_id: None,
        preset_id: None,
        preset_overrides: None,
        delegation_policy: Default::default(),
        execution_model_pool: None,
        decision_policy: Default::default(),
        execution_template_id: None,
        extra,
    };
    match deps.conversation_service.create(&user_id, req).await {
        Ok(resp) => ok(json!({
            "conversation_id": resp.conversation_id,
            "name": resp.name,
            "agent_type": resp.r#type,
            "model": resp.model,
            "model_source": model_source,
        })),
        Err(e) => error_value(e),
    }
}

/// Reflect the calling session's authenticated identity and surface. Remote
/// installation tokens report `principal = nomifun_desktop` and deliberately
/// keep `companion_id = null`.
async fn whoami(_deps: Arc<GatewayDeps>, ctx: CallerCtx, _p: WhoamiParams) -> Value {
    ok(json!({
        "user_id": ctx.user_id,
        "companion_id": ctx.companion_id,
        "principal": if ctx.remote { "nomifun_desktop" } else { "session" },
        "surface": format!("{:?}", ctx.surface()),
        "remote": ctx.remote,
        "channel_platform": ctx.channel_platform,
    }))
}

async fn update(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: UpdateConversationParams) -> Value {
    let user_id = ctx.user_id.as_str().to_owned();
    let id = p.conversation_id.into_string();
    if p.name.is_none() && p.pinned.is_none() && p.model.is_none() {
        return json!({ "error": "nothing to update: provide at least one of name / pinned / model" });
    }
    let mut model = None;
    if let Some(requested_model) = p.model.map(ProviderWithModel::from) {
        if ctx.conversation_id.as_ref().is_some_and(|caller| id == caller.as_str()) {
            return json!({
                "error": "self_model_change_forbidden: changing your own conversation's model would terminate your current turn; the owner can change it from the desktop UI"
            });
        }
        match provider_support::resolve_explicit_model(&deps, requested_model).await {
            Ok(m) => model = Some(m),
            Err(e) => return e,
        }
    }
    let model_changed = model.is_some();
    let req = UpdateConversationRequest {
        name: p.name,
        pinned: p.pinned,
        model,
        delegation_policy: None,
        execution_model_pool: None,
        decision_policy: None,
        execution_template_id: None,
        extra: None,
    };
    match deps.conversation_service.update(&user_id, &id, req, &deps.runtime_registry).await {
        Ok(resp) => ok(json!({
            "conversation_id": resp.conversation_id,
            "name": resp.name,
            "pinned": resp.pinned,
            "model": resp.model,
            "note": model_changed.then_some(
                "model changed: any running task in that conversation was terminated; it restarts with the new model on the next message"
            ),
        })),
        Err(e) => error_value(e),
    }
}

async fn delete(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: DeleteConversationParams) -> Value {
    let user_id = ctx.user_id.as_str().to_owned();
    let id = p.conversation_id.into_string();
    if ctx.conversation_id.as_ref().is_some_and(|caller| id == caller.as_str()) {
        return json!({ "error": "self_deletion_forbidden: you cannot delete your own conversation" });
    }
    match deps.conversation_service.delete(&user_id, &id).await {
        Ok(()) => ok(json!({ "deleted": id })),
        Err(e) => error_value(e),
    }
}

/// Map the pre-stop persisted status onto the tool's `{stopped, previous_status}`
/// contract: only a conversation that was durably `running` counts as stopped.
fn stop_outcome(previous_status: &str) -> (bool, &str) {
    (previous_status == "running", previous_status)
}

async fn stop(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: StopConversationParams) -> Value {
    let user_id = ctx.user_id.as_str().to_owned();
    let id = p.conversation_id.into_string();
    if ctx.conversation_id.as_ref().is_some_and(|caller| id == caller.as_str()) {
        return json!({ "error": "self_stop_forbidden: stopping your own conversation would cancel the turn you are answering from; ask the owner to stop it from the desktop" });
    }
    let conv = match deps.conversation_service.get(&user_id, &id).await {
        Ok(c) => c,
        Err(e) => return error_value(e),
    };
    let previous_status = match serde_json::to_value(conv.status) {
        Ok(Value::String(status)) => status,
        _ => return json!({ "error": "conversation status could not be serialized" }),
    };
    // Same safe service path as the desktop stop button (POST
    // /api/conversations/{id}/cancel): the stop tombstone, exact-generation
    // teardown, and durable finalization all stay owned by the service. This
    // tool never mutates receipts or lifecycle rows directly.
    if let Err(e) = deps
        .conversation_service
        .cancel(&user_id, &id, &deps.runtime_registry)
        .await
    {
        return error_value(e);
    }
    let (stopped, previous_status) = stop_outcome(&previous_status);
    ok(json!({
        "stopped": stopped,
        "previous_status": previous_status,
    }))
}

/// Cap every `content` string inside the serialized message list so a long
/// transcript cannot flood the calling agent.
fn truncate_message_contents(mut value: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for (k, item) in map.iter_mut() {
                    if k == "content" {
                        if let Value::String(s) = item
                            && s.chars().count() > MESSAGE_SNIPPET_CHARS
                        {
                            let truncated: String = s.chars().take(MESSAGE_SNIPPET_CHARS).collect();
                            *item = Value::String(format!("{truncated}…[truncated]"));
                        } else {
                            walk(item);
                        }
                    } else {
                        walk(item);
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    walk(item);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<ListConversationsParams, _, _>(
        CapabilityMeta::new(
            "nomi_list_conversations",
            "conversation",
            "List the desktop's conversations with their live runtime state.",
            DangerTier::Read,
        ),
        list,
    ));
    out.push(Capability::new::<ConversationStatusParams, _, _>(
        CapabilityMeta::new(
            "nomi_conversation_status",
            "conversation",
            "Runtime summary + the tail of a conversation's transcript (live progress snapshot).",
            DangerTier::Read,
        ),
        status,
    ));
    out.push(Capability::new::<SendToConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_send_to_conversation",
            "conversation",
            "Inject a message (or a hidden task prompt) into another session. Set notify_back=true to automatically receive a completion receipt in your own conversation when the target finishes.",
            DangerTier::Write,
        ),
        send,
    ));
    out.push(Capability::new::<CreateConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_create_conversation",
            "conversation",
            "Open a fresh desktop session on behalf of the calling companion. Every conversation runs on the native nomi executor, so there is no engine or vendor to choose — just pass model to pin an exact provider/model pair, or omit it to inherit your own companion model. Pass workpath when the user gave a project path (creates a project session in that directory), and summon to load your own skills + hand-picked memories (read-only) into the new session — pre-select memory_ids with recall_memories. For a terminal or agent-CLI session use nomi_create_terminal; for multi-Agent work inside the current conversation, use nomi_delegate.",
            DangerTier::Write,
        ),
        create,
    ));
    out.push(Capability::new::<UpdateConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_update_conversation",
            "conversation",
            "Rename / pin / change model of a conversation (not your own model).",
            DangerTier::Write,
        ),
        update,
    ));
    out.push(Capability::new::<DeleteConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_delete_conversation",
            "conversation",
            "Delete a conversation (cascades: agent kill, cron unbind, knowledge unmount). Confirm first.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        delete,
    ));
    out.push(Capability::new::<StopConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_stop_conversation",
            "conversation",
            "Stop a conversation's current turn — including one left protectively suspended (stuck) by a backend restart. Same safe path as the desktop stop button.",
            DangerTier::Destructive,
        ),
        stop,
    ));
    out.push(Capability::new::<WhoamiParams, _, _>(
        CapabilityMeta::new(
            "nomi_whoami",
            "conversation",
            "Identity of the calling session: installation user, optional companion id, and surface. Remote installation-token callers report principal=nomifun_desktop and companion_id=null.",
            DangerTier::Read,
        ),
        whoami,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::UserId;

    const TEST_COMPANION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
    const TEST_MEMORY_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000101";

    #[test]
    fn create_conversation_schema_exposes_workpath_and_summon() {
        let mut caps = Vec::new();
        register(&mut caps);
        let cap = caps
            .iter()
            .find(|cap| cap.meta.name == "nomi_create_conversation")
            .expect("nomi_create_conversation must be registered");
        let properties = cap.input_schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("workpath"));
        assert!(properties.contains_key("summon"));
        // Per-vendor engine selection is gone: the schema must not re-advertise
        // an agent catalog id or a backend vendor to the model.
        assert!(!properties.contains_key("agent_id"));
        assert!(!properties.contains_key("backend"));
        assert_eq!(
            cap.input_schema.get("additionalProperties"),
            Some(&json!(false))
        );
    }

    #[test]
    fn create_conversation_summary_does_not_advertise_engine_selection() {
        let mut caps = Vec::new();
        register(&mut caps);
        let cap = caps
            .iter()
            .find(|cap| cap.meta.name == "nomi_create_conversation")
            .expect("nomi_create_conversation must be registered");
        let summary = cap.meta.summary.to_lowercase();
        for dead in ["acp", "agent_id", "remote_agent_id", "openclaw"] {
            assert!(
                !summary.contains(dead),
                "the create-conversation summary must not mention '{dead}': {}",
                cap.meta.summary
            );
        }
    }

    #[test]
    fn agent_type_accepts_only_nomi_and_redirects_terminal() {
        // Omitted and explicit "nomi" both resolve to the native executor — the
        // explicit form must keep working because the params are
        // deny_unknown_fields and a stale caller still sends it.
        assert_eq!(validated_agent_type(None).unwrap(), AgentType::Nomi);
        assert_eq!(validated_agent_type(Some("nomi")).unwrap(), AgentType::Nomi);

        let terminal = validated_agent_type(Some("terminal")).unwrap_err();
        assert!(
            terminal.contains("nomi_create_terminal"),
            "the terminal redirect must name the tool to use: {terminal}"
        );

        for retired in ["acp", "remote", "openclaw", "claude", ""] {
            let error = validated_agent_type(Some(retired)).unwrap_err();
            assert!(
                error.contains(&format!("invalid agent_type '{retired}'")),
                "the error must quote the rejected value: {error}"
            );
            assert!(
                error.contains("'nomi'"),
                "the error must name the one valid value: {error}"
            );
        }
    }

    #[test]
    fn retired_params_still_deserialize_so_they_can_be_answered_not_rejected_by_serde() {
        // `deny_unknown_fields` makes an undeclared field a parse error, which
        // reaches the caller as opaque serde text. `agent_type` and
        // `remote_agent_id` therefore stay DECLARED after their engines were
        // removed, so `create` can answer them itself. Deleting either field
        // silently downgrades those explanations back to parse noise.
        let parsed: CreateConversationParams = serde_json::from_value(json!({
            "agent_type": "acp",
            "remote_agent_id": "0190f5fe-7c00-7a00-8abc-012345678901",
        }))
        .unwrap();
        assert_eq!(parsed.agent_type.as_deref(), Some("acp"));
        assert_eq!(
            parsed.remote_agent_id.as_ref().map(RemoteAgentId::as_str),
            Some("0190f5fe-7c00-7a00-8abc-012345678901")
        );
        // …and the value that got this far is still refused, with a message.
        assert!(validated_agent_type(parsed.agent_type.as_deref()).is_err());
    }

    #[test]
    fn create_conversation_params_parse_workpath_and_summon() {
        let parsed: CreateConversationParams = serde_json::from_value(json!({
            "name": "重构任务",
            "workpath": "C:/code/project",
            "summon": {
                "companion_id": TEST_COMPANION_ID,
                "memory_ids": [TEST_MEMORY_ID],
                "skill_exclusions": ["heavy-refactor"],
            },
        }))
        .unwrap();
        assert_eq!(parsed.workpath.as_deref(), Some("C:/code/project"));
        let summon = parsed.summon.unwrap();
        assert_eq!(summon.companion_id.as_str(), TEST_COMPANION_ID);
        assert_eq!(summon.memory_ids, vec![TEST_MEMORY_ID]);

        // The summon sub-object is a closed contract: clients can never stamp
        // summoned_at (server-owned) or smuggle unknown fields.
        for invalid in [
            json!({ "summon": { "companion_id": TEST_COMPANION_ID, "summoned_at": 1 } }),
            json!({ "summon": { "companion_id": "not-an-id" } }),
            json!({ "summon": {} }),
        ] {
            assert!(
                serde_json::from_value::<CreateConversationParams>(invalid.clone()).is_err(),
                "must reject {invalid}"
            );
        }
    }

    #[test]
    fn summon_extra_value_stamps_validates_and_dedups() {
        let summon: CreateSummonParams = serde_json::from_value(json!({
            "companion_id": TEST_COMPANION_ID,
            "memory_ids": [TEST_MEMORY_ID, TEST_MEMORY_ID],
            "skill_exclusions": [" heavy-refactor ", "heavy-refactor", "  "],
        }))
        .unwrap();
        let value = summon_extra_value(&summon, 42).unwrap();
        assert_eq!(value["companion_id"], TEST_COMPANION_ID);
        assert_eq!(value["memory_ids"], json!([TEST_MEMORY_ID]));
        assert_eq!(value["skill_exclusions"], json!(["heavy-refactor"]));
        assert_eq!(value["summoned_at"], 42, "server stamp wins");

        let bad: CreateSummonParams = serde_json::from_value(json!({
            "companion_id": TEST_COMPANION_ID,
            "memory_ids": ["not-a-memory-id"],
        }))
        .unwrap();
        assert!(summon_extra_value(&bad, 42).is_err());
    }

    #[test]
    fn workpath_normalization_rejects_blank() {
        assert_eq!(normalized_workpath("  C:/code/x  ").unwrap(), "C:/code/x");
        assert!(normalized_workpath("   ").is_err());
    }

    #[test]
    fn stop_conversation_is_a_registered_destructive_capability() {
        let mut caps = Vec::new();
        register(&mut caps);
        let cap = caps
            .iter()
            .find(|cap| cap.meta.name == "nomi_stop_conversation")
            .expect("nomi_stop_conversation must be registered");
        assert_eq!(cap.meta.domain, "conversation");
        assert_eq!(cap.meta.danger, DangerTier::Destructive);
        let properties = cap.input_schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("conversation_id"));
        assert!(
            properties.contains_key("confirm"),
            "a Destructive capability must expose the confirm gate field"
        );
        assert_eq!(
            cap.input_schema.get("additionalProperties"),
            Some(&json!(false))
        );
    }

    #[test]
    fn stop_conversation_requires_confirm_on_remote_surface() {
        let mut caps = Vec::new();
        register(&mut caps);
        let cap = caps
            .iter()
            .find(|cap| cap.meta.name == "nomi_stop_conversation")
            .expect("nomi_stop_conversation must be registered");
        assert_eq!(
            crate::registry::decide(&cap.meta, Surface::Remote, false),
            crate::registry::Decision::Confirm,
            "Remote surface without confirm must be refused"
        );
        assert_eq!(
            crate::registry::decide(&cap.meta, Surface::Remote, true),
            crate::registry::Decision::Allow,
        );
    }

    #[test]
    fn stop_conversation_params_require_a_canonical_conversation_id() {
        let params: StopConversationParams = serde_json::from_value(json!({
            "conversation_id": "0190f5fe-7c00-7a00-8abc-012345678901"
        }))
        .unwrap();
        assert_eq!(
            params.conversation_id.as_str(),
            "0190f5fe-7c00-7a00-8abc-012345678901"
        );
        assert!(
            serde_json::from_value::<StopConversationParams>(
                json!({"conversation_id": "conversation-1"})
            )
            .is_err()
        );
    }

    #[test]
    fn send_params_accept_optional_notify_back() {
        let params: SendToConversationParams = serde_json::from_value(json!({
            "conversation_id": "0190f5fe-7c00-7a00-8abc-012345678901",
            "content": "do the thing",
            "notify_back": true
        }))
        .unwrap();
        assert_eq!(params.notify_back, Some(true));

        // Default stays off.
        let params: SendToConversationParams = serde_json::from_value(json!({
            "conversation_id": "0190f5fe-7c00-7a00-8abc-012345678901",
            "content": "do the thing"
        }))
        .unwrap();
        assert_eq!(params.notify_back, None);

        // The registered capability schema exposes the field.
        let mut caps = Vec::new();
        register(&mut caps);
        let cap = caps
            .iter()
            .find(|cap| cap.meta.name == "nomi_send_to_conversation")
            .expect("nomi_send_to_conversation must be registered");
        assert!(
            cap.input_schema["properties"]
                .as_object()
                .unwrap()
                .contains_key("notify_back")
        );
    }

    #[test]
    fn stop_outcome_maps_previous_status_to_stopped_flag() {
        assert_eq!(
            stop_outcome("running"),
            (true, "running"),
            "stopping a running conversation reports stopped=true"
        );
        assert_eq!(
            stop_outcome("finished"),
            (false, "finished"),
            "an idle conversation reports stopped=false with its previous status"
        );
        assert_eq!(stop_outcome("pending"), (false, "pending"));
    }

    #[test]
    fn stuck_assessment_flags_only_durable_running_without_runtime() {
        let (stuck, hint) = stuck_assessment(true, false);
        assert!(stuck, "durable running with no live runtime is the restart-isolated state");
        assert_eq!(
            hint,
            Some("该会话因后端重启被保护性挂起，可用 nomi_stop_conversation 解除后重试"),
        );

        for (is_persisted_running, has_runtime) in [(true, true), (false, false), (false, true)] {
            let (stuck, hint) = stuck_assessment(is_persisted_running, has_runtime);
            assert!(
                !stuck && hint.is_none(),
                "({is_persisted_running}, {has_runtime}) must not be reported stuck"
            );
        }
    }

    #[test]
    fn truncate_caps_long_content_strings() {
        let long = "x".repeat(2000);
        let v = json!({"items": [{"content": long, "other": "keep"}]});
        let out = truncate_message_contents(v);
        let content = out["items"][0]["content"].as_str().unwrap();
        assert!(content.chars().count() < 600);
        assert!(content.ends_with("…[truncated]"));
        assert_eq!(out["items"][0]["other"], "keep");
    }

    #[test]
    fn truncate_keeps_short_content_untouched() {
        let v = json!({"content": "short"});
        let out = truncate_message_contents(v);
        assert_eq!(out["content"], "short");
    }

    #[test]
    fn conversation_params_require_typed_ids_and_fixed_model_object() {
        assert!(
            serde_json::from_value::<ConversationStatusParams>(json!({
                "conversation_id": "conversation-1"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreateConversationParams>(json!({
                "agent_type": "nomi",
                "provider_id": "0190f5fe-7c00-7a00-8abc-012345678904",
                "model": "model-a"
            }))
            .is_err()
        );
        let params: CreateConversationParams = serde_json::from_value(json!({
            "agent_type": "nomi",
            "model": {
                "provider_id": "0190f5fe-7c00-7a00-8abc-012345678904",
                "model": "model-a"
            }
        }))
        .unwrap();
        assert_eq!(
            params.model.as_ref().map(|model| model.provider_id.as_str()),
            Some("0190f5fe-7c00-7a00-8abc-012345678904")
        );
        assert!(
            serde_json::from_value::<CreateConversationParams>(json!({
                "agent_type": "nomi",
                "model": {
                    "provider_id": "0190f5fe-7c00-7a00-8abc-012345678904",
                    "model": "model-a",
                    "use_model": "retired-alias"
                }
            }))
            .is_err()
        );
    }

    #[test]
    fn top_level_creation_requires_a_companion_identity() {
        let plain = CallerCtx {
            conversation_id: Some(
                ConversationId::parse("0190f5fe-7c00-7a00-8abc-012345678901")
                    .unwrap(),
            ),
            user_id: UserId::parse("0190f5fe-7c00-7a00-8abc-012345678902")
                .unwrap(),
            ..Default::default()
        };
        let error = require_conversation_creator(&plain).unwrap_err();
        assert!(error["error"]
            .as_str()
            .is_some_and(|message| message.contains("conversation_creation_forbidden")));

        let companion = CallerCtx {
            companion_id: Some(
                CompanionId::parse("0190f5fe-7c00-7a00-8abc-012345678903")
                    .unwrap(),
            ),
            ..plain
        };
        assert!(require_conversation_creator(&companion).is_ok());
    }
}
