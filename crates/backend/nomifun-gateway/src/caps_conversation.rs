//! Conversation-domain capabilities (registry form): list / status / send /
//! create / update / delete. All self-protection guards from the legacy tool
//! are preserved (no self-injection, no self-model-change, no self-deletion),
//! and nomi sessions still get a model at creation via the shared resolution
//! chain so downstream consumers never see a model-less nomi conversation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nomifun_api_types::{
    CreateConversationRequest, ListConversationsQuery, ListMessagesQuery, SendMessageRequest,
    UpdateConversationRequest,
};
use nomifun_common::{AgentType, AppError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier, ProgressSink, Surface};
use crate::server::ok;
use crate::tools_provider;

const DEFAULT_LIST_LIMIT: u32 = 50;
const DEFAULT_MESSAGE_LIMIT: u32 = 5;
/// Per-message content budget in status output — keeps a busy transcript from
/// blowing up the calling agent's context.
const MESSAGE_SNIPPET_CHARS: usize = 500;

#[derive(Deserialize, JsonSchema)]
struct ListConversationsParams {
    /// Maximum number of conversations to return (default 50).
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct ConversationStatusParams {
    /// The id of the conversation to inspect.
    conversation_id: i64,
    /// How many recent messages to include (default 5, max 50).
    #[serde(default)]
    message_limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct SendToConversationParams {
    /// The id of the TARGET conversation (not your own).
    conversation_id: i64,
    /// The message or task prompt to inject.
    content: String,
    /// When true the message is hidden from the visible history (use for
    /// background task prompts, like AutoWork does).
    #[serde(default)]
    hidden: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateConversationParams {
    /// Optional display name for the new conversation.
    #[serde(default)]
    name: Option<String>,
    /// Agent type: "nomi" (default) or "acp". NOT for terminals — any
    /// terminal/shell intent must go through nomi_create_terminal instead.
    #[serde(default)]
    agent_type: Option<String>,
    /// ACP backend vendor when agent_type is "acp" (e.g. "claude", "codex", "gemini").
    #[serde(default)]
    backend: Option<String>,
    /// Provider id for nomi sessions (from nomi_list_providers). Omit to
    /// auto-resolve: your own companion model → first configured provider.
    #[serde(default)]
    provider_id: Option<String>,
    /// Model id for nomi sessions. Omit to auto-resolve (see provider_id).
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateConversationParams {
    /// The id of the conversation to update (from nomi_list_conversations).
    conversation_id: i64,
    /// New display name (omit to keep).
    #[serde(default)]
    name: Option<String>,
    /// Pin (true) or unpin (false) the conversation in the sidebar.
    #[serde(default)]
    pinned: Option<bool>,
    /// New provider id (nomi conversations only; from nomi_list_providers).
    #[serde(default)]
    provider_id: Option<String>,
    /// New model id (nomi conversations only).
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteConversationParams {
    /// The id of the conversation to delete. Confirm the target with the user
    /// before calling — deletion also kills its agent and cron bindings.
    conversation_id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct AgentRunParams {
    /// The goal / task to delegate. A fresh autonomous NomiFun (nomi) agent is
    /// spun up to accomplish it end-to-end.
    goal: String,
    /// Optional absolute workspace directory for the run. Omit for an
    /// auto-provisioned temp workspace.
    #[serde(default)]
    workspace: Option<String>,
    /// Optional model id for the agent (provider auto-resolved). Omit to use the
    /// default nomi model.
    #[serde(default)]
    model: Option<String>,
    /// Max seconds to wait for completion before returning a `{status:"running"}`
    /// handle (default 300, clamped 5..1800). Poll nomi_agent_result afterwards.
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct AgentResultParams {
    /// The conversation id returned by nomi_agent_run.
    conversation_id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct WhoamiParams {}

fn error_value(e: AppError) -> Value {
    json!({ "error": e.to_string() })
}

fn require_user(ctx: &CallerCtx) -> Result<&str, Value> {
    if ctx.user_id.is_empty() {
        Err(json!({ "error": "missing caller user identity" }))
    } else {
        Ok(&ctx.user_id)
    }
}

async fn list(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ListConversationsParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u,
        Err(e) => return e,
    };
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
        let runtime = deps.conversation_service.runtime_summary_for(&conv.id.to_string()).await;
        items.push(json!({
            "id": conv.id,
            "name": conv.name,
            "agent_type": conv.r#type,
            "status": conv.status,
            "runtime_state": runtime.state,
            "pending_confirmations": runtime.pending_confirmations,
            "source": conv.source,
            "pinned": conv.pinned,
            "is_companion_companion": conv.extra.get("companionSession").and_then(Value::as_bool).unwrap_or(false),
            "companion_id": conv.extra.get("companionId").and_then(Value::as_str),
            "is_self": conv.id.to_string() == ctx.conversation_id,
            "modified_at": conv.modified_at,
        }));
    }
    ok(json!({ "total": resp.total, "conversations": items }))
}

async fn status(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ConversationStatusParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let id = p.conversation_id.to_string();
    let id = id.as_str();
    let conv = match deps.conversation_service.get(user_id, id).await {
        Ok(c) => c,
        Err(e) => return error_value(e),
    };
    let runtime = deps.conversation_service.runtime_summary_for(id).await;
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
        "id": conv.id,
        "name": conv.name,
        "agent_type": conv.r#type,
        "status": conv.status,
        "runtime": runtime,
        "recent_messages": messages_json,
    }))
}

async fn send(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: SendToConversationParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    let id = p.conversation_id.to_string();
    if !ctx.conversation_id.is_empty() && id == ctx.conversation_id {
        return json!({ "error": "self_injection_forbidden: you cannot send a message into your own conversation" });
    }
    let req = SendMessageRequest {
        content: p.content,
        files: vec![],
        inject_skills: vec![],
        hidden: p.hidden.unwrap_or(false),
        origin: Some("companion".into()),
        channel_platform: None,
    };
    match deps.conversation_service.send_message(&user_id, &id, req, &deps.task_manager).await {
        Ok(msg_id) => ok(json!({
            "msg_id": msg_id,
            "note": "message accepted; the target session processes it asynchronously — use nomi_conversation_status to follow progress"
        })),
        Err(AppError::Conflict(m)) => json!({
            "error": format!("busy: the target conversation is already running a turn ({m}); check nomi_conversation_status and retry later")
        }),
        Err(e) => error_value(e),
    }
}

async fn create(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CreateConversationParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    let agent_type_str = p.agent_type.unwrap_or_else(|| "nomi".to_owned());
    if agent_type_str == "terminal" {
        return json!({
            "error": "terminal sessions are not conversations: use nomi_create_terminal (preset shell | claude | codex | gemini) for any terminal/shell intent"
        });
    }
    let agent_type: AgentType = match serde_json::from_value(json!(agent_type_str)) {
        Ok(t) => t,
        Err(_) => return json!({ "error": format!("invalid agent_type '{agent_type_str}'") }),
    };
    let mut extra = json!({});
    if let Some(backend) = p.backend {
        extra["backend"] = json!(backend);
    }
    let mut model = None;
    let mut model_source = None;
    if agent_type == AgentType::Nomi {
        match tools_provider::resolve_nomi_model(&deps, &ctx, p.provider_id.as_deref(), p.model.as_deref()).await {
            Ok((m, source)) => {
                model = Some(m);
                model_source = Some(source);
            }
            Err(e) => return e,
        }
    }
    let req = CreateConversationRequest {
        r#type: agent_type,
        name: p.name,
        model,
        source: None,
        channel_chat_id: None,
        extra,
    };
    match deps.conversation_service.create(&user_id, req).await {
        Ok(resp) => ok(json!({
            "id": resp.id,
            "name": resp.name,
            "agent_type": resp.r#type,
            "model": resp.model,
            "model_source": model_source,
        })),
        Err(e) => error_value(e),
    }
}

/// Await an agent turn to completion (or until `timeout`), polling every `poll`.
/// The turn is claimed synchronously inside `send_message` before it returns, so
/// `is_processing` is reliably true on the first poll. Returns true if the turn
/// finished, false on timeout (the run keeps going; poll nomi_agent_result). An
/// already-finished turn returns immediately (the first check happens before any
/// sleep). Used both to await an unsubscribable turn (coarse poll) and to let a
/// just-finished turn settle before reading its final message (fine poll).
async fn await_turn(deps: &GatewayDeps, conv_id: &str, timeout: Duration, poll: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let summary = deps.conversation_service.runtime_summary_for(conv_id).await;
        if !summary.is_processing {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(poll).await;
    }
}

/// Walk a serialized message list (desc-ordered) and return the newest assistant
/// reply text — the first object with `position == "left"` and `type == "text"`,
/// whose `content` is shaped `{"content": "<text>"}`.
fn latest_assistant_text(v: &Value) -> Option<String> {
    match v {
        Value::Array(arr) => arr.iter().find_map(latest_assistant_text),
        Value::Object(map) => {
            let is_assistant_text = map.get("position").and_then(Value::as_str) == Some("left")
                && map.get("type").and_then(Value::as_str) == Some("text");
            if is_assistant_text
                && let Some(text) = map.get("content").and_then(|c| c.get("content")).and_then(Value::as_str)
            {
                return Some(text.to_owned());
            }
            map.values().find_map(latest_assistant_text)
        }
        _ => None,
    }
}

/// Read the final assistant text of a (finished) conversation, if any.
async fn read_final_text(deps: &GatewayDeps, user_id: &str, conv_id: &str) -> Option<String> {
    let messages = deps
        .conversation_service
        .list_messages(
            user_id,
            conv_id,
            ListMessagesQuery {
                page: Some(1),
                page_size: Some(10),
                order: Some("desc".to_owned()),
                content_mode: None,
                cursor: None,
            },
        )
        .await
        .ok()?;
    let v = serde_json::to_value(&messages).ok()?;
    latest_assistant_text(&v)
}

/// Subscribe to a turn's event stream. The agent instance is built inside
/// `send_message`'s spawned task, so poll briefly for it. `None` if it never
/// appears (caller falls back to polling completion).
async fn subscribe_turn(
    deps: &GatewayDeps,
    conv_id: &str,
    wait: Duration,
) -> Option<tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent>> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(agent) = deps.task_manager.get_task(conv_id) {
            return Some(agent.subscribe());
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Forward the turn's events to `progress` until a terminal event
/// (finish/turn_completed/error) or `deadline`. Events are forwarded as their
/// serialized JSON (tagged `{"type": .., "data": ..}`); terminal detection uses
/// the `type` tag so it never couples to the event struct internals. Returns
/// true if the turn finished, false on timeout.
async fn drain_stream(
    rx: &mut tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent>,
    progress: &ProgressSink,
    deadline: Instant,
) -> bool {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(ev)) => {
                let v = serde_json::to_value(&ev).unwrap_or_else(|_| json!({}));
                let terminal = v
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| matches!(t, "finish" | "turn_completed" | "error"));
                let _ = progress.send(v).await;
                if terminal {
                    return true;
                }
            }
            Ok(Err(RecvError::Lagged(_))) => continue,
            // Channel closed = the agent instance was dropped at turn end.
            Ok(Err(RecvError::Closed)) => return true,
            Err(_timeout) => return false,
        }
    }
}

/// Delegate a goal to a fresh autonomous nomi agent. Streams the agent's
/// events (text / tool-call deltas) through `progress` as they arrive (the
/// streaming `/tool/stream` path); the buffered path returns only the final
/// result. Either way: on completion returns the final assistant text; on
/// timeout returns a `{status:"running"}` handle (poll `nomi_agent_result`).
async fn agent_run(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: AgentRunParams, progress: ProgressSink) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    if p.goal.trim().is_empty() {
        return json!({ "error": "goal must not be empty" });
    }
    // A nomi conversation must get a model at creation.
    let model = match tools_provider::resolve_nomi_model(&deps, &ctx, None, p.model.as_deref()).await {
        Ok((m, _source)) => Some(m),
        Err(e) => return e,
    };
    // yolo: unattended Remote runs have no approval UI — without yolo a tool call
    // would park forever. desktopGateway: entitle the delegated agent to the full
    // platform tool set (the "外部伙伴" experience). We call create() directly
    // (not via the HTTP route), so these extra keys are honored, not stripped.
    let mut extra = json!({ "session_mode": "yolo", "desktopGateway": true });
    if let Some(ws) = p.workspace.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        extra["workspace"] = json!(ws);
    }
    let create_req = CreateConversationRequest {
        r#type: AgentType::Nomi,
        name: Some("Remote agent run".to_owned()),
        model,
        source: None,
        channel_chat_id: None,
        extra,
    };
    let conv = match deps.conversation_service.create(&user_id, create_req).await {
        Ok(c) => c,
        Err(e) => return error_value(e),
    };
    let id = conv.id.to_string();
    let send_req = SendMessageRequest {
        content: p.goal,
        files: vec![],
        inject_skills: vec![],
        hidden: false,
        origin: Some("remote".into()),
        channel_platform: None,
    };
    if let Err(e) = deps
        .conversation_service
        .send_message(&user_id, &id, send_req, &deps.task_manager)
        .await
    {
        return json!({ "error": format!("failed to start agent run: {e}"), "conversation_id": conv.id });
    }
    let timeout = Duration::from_secs(p.timeout_secs.unwrap_or(300).clamp(5, 1800));
    let deadline = Instant::now() + timeout;
    // Prefer streaming the live event broadcast; fall back to polling completion
    // if the instance can't be subscribed in time. Final text is read from the
    // DB either way, so missed early deltas never corrupt the result.
    let finished = match subscribe_turn(&deps, &id, Duration::from_secs(5)).await {
        Some(mut rx) => drain_stream(&mut rx, &progress, deadline).await,
        None => await_turn(&deps, &id, timeout, Duration::from_millis(500)).await,
    };
    if finished {
        // The terminal broadcast event ("finish"/Closed) can fire a few ms before
        // the final assistant `text` message is committed: a reasoning model
        // persists its visible answer LAST (after the `thinking` message), right
        // at turn end, so reading immediately can miss it and return null.
        // `nomi_agent_result` never hits this because it gates on the runtime turn
        // having fully released (`is_processing == false`), by which point the
        // message is listable. Mirror that here — settle (bounded, fine-grained)
        // before reading. An already-settled turn returns at once (no added latency).
        let _ = await_turn(&deps, &id, Duration::from_secs(5), Duration::from_millis(25)).await;
        let text = read_final_text(&deps, &user_id, &id).await;
        ok(json!({ "conversation_id": conv.id, "status": "completed", "text": text }))
    } else {
        ok(json!({
            "conversation_id": conv.id,
            "status": "running",
            "note": "agent run still in progress after timeout; poll nomi_agent_result with this conversation_id"
        }))
    }
}

/// Fetch the result (or running status) of a delegated agent run.
async fn agent_result(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: AgentResultParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    let id = p.conversation_id.to_string();
    let summary = deps.conversation_service.runtime_summary_for(&id).await;
    if summary.is_processing {
        return ok(json!({ "conversation_id": p.conversation_id, "status": "running" }));
    }
    let text = read_final_text(&deps, &user_id, &id).await;
    ok(json!({ "conversation_id": p.conversation_id, "status": "completed", "text": text }))
}

/// Reflect the calling session's identity: which companion it is bound to, the
/// surface it arrived on, and the user scope. For an external partner this
/// answers "who am I acting as?"; for tests it proves companion binding reached
/// dispatch.
async fn whoami(_deps: Arc<GatewayDeps>, ctx: CallerCtx, _p: WhoamiParams) -> Value {
    ok(json!({
        "user_id": ctx.user_id,
        "companion_id": ctx.companion_id,
        "surface": format!("{:?}", ctx.surface()),
        "remote": ctx.remote,
        "channel_platform": ctx.channel_platform,
    }))
}

async fn update(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: UpdateConversationParams) -> Value {
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    let id = p.conversation_id.to_string();
    if p.name.is_none() && p.pinned.is_none() && p.provider_id.is_none() && p.model.is_none() {
        return json!({ "error": "nothing to update: provide at least one of name / pinned / provider_id+model" });
    }
    let mut model = None;
    if p.provider_id.is_some() || p.model.is_some() {
        if !ctx.conversation_id.is_empty() && id == ctx.conversation_id {
            return json!({
                "error": "self_model_change_forbidden: changing your own conversation's model would terminate your current turn; the owner can change it from the desktop UI"
            });
        }
        match tools_provider::resolve_explicit_model(&deps, p.provider_id.as_deref(), p.model.as_deref()).await {
            Ok(m) => model = Some(m),
            Err(e) => return e,
        }
    }
    let model_changed = model.is_some();
    let req = UpdateConversationRequest {
        name: p.name,
        pinned: p.pinned,
        model,
        extra: None,
    };
    match deps.conversation_service.update(&user_id, &id, req, &deps.task_manager).await {
        Ok(resp) => ok(json!({
            "id": resp.id,
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
    let user_id = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    let id = p.conversation_id.to_string();
    if !ctx.conversation_id.is_empty() && id == ctx.conversation_id {
        return json!({ "error": "self_deletion_forbidden: you cannot delete your own conversation" });
    }
    match deps.conversation_service.delete(&user_id, &id).await {
        Ok(()) => ok(json!({ "deleted": id })),
        Err(e) => error_value(e),
    }
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
            "Inject a message (or a hidden task prompt) into another session.",
            DangerTier::Write,
        ),
        send,
    ));
    out.push(Capability::new::<CreateConversationParams, _, _>(
        CapabilityMeta::new(
            "nomi_create_conversation",
            "conversation",
            "Open a fresh desktop session (nomi or acp). nomi sessions get a model at creation.",
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
    out.push(Capability::new_streaming::<AgentRunParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_run",
            "agent",
            "Delegate a goal to a fresh autonomous NomiFun agent: spins up a nomi session (yolo, full platform tools), runs it to completion, and returns the final answer. Streams the agent's progress over /tool/stream (or the SSE REST endpoint); buffered callers get the final result. Long runs return a {status:\"running\"} handle — poll nomi_agent_result.",
            DangerTier::Write,
        ),
        agent_run,
    ));
    out.push(Capability::new::<AgentResultParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_result",
            "agent",
            "Fetch the result (or running status) of a goal delegated via nomi_agent_run, by conversation id.",
            DangerTier::Read,
        ),
        agent_result,
    ));
    out.push(Capability::new::<WhoamiParams, _, _>(
        CapabilityMeta::new(
            "nomi_whoami",
            "conversation",
            "Identity of the calling session: the bound companion id, surface (Desktop/Channel/Remote), and user. Lets an external partner confirm which companion it is acting as.",
            DangerTier::Read,
        ),
        whoami,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
