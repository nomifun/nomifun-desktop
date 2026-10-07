//! Real-provider acceptance for in-conversation Plugin creation with the
//! official General Agent. The run uses only the product HTTP surface: the
//! same turn request, owner approval and conversation-card continuation calls
//! the desktop renderer makes. A headless Agent tool is requested because this
//! acceptance has no renderer to execute UI verification steps.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomifun_app::{AttachedChromeProviderService, DesktopHostServices, DesktopServer};
use nomifun_browser_platform::runtime::{
    BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime, WorkspaceError,
};
use nomifun_browser_platform::workspace::BrowserResourceService;
use reqwest::{Client, Method};
use serde_json::{Value, json};

use super::{
    BODY_LIMIT, LOCAL_API_DEADLINE, SmokeFailure, TURN_COMMAND_DEADLINE, ensure_secret_not_persisted,
    live_model, required_secret_from_stdin,
};

/// The owner's request: one headless Agent tool, used in this conversation.
const REQUEST: &str = "请创建一个插件：它只有后台服务、没有界面，提供一个 Agent 工具，把输入文本中每个英文单词的首字母改成大写，其余字符保持不变（例如 hello nomi fun 变成 Hello Nomi Fun）。安装完成后，请在当前会话里直接调用这个新工具处理“hello nomi fun”，并告诉我结果。";
const EXPECTED_INPUT: &str = "hello nomi fun";
const EXPECTED_RESULT: &str = "Hello Nomi Fun";
// Conversation-card prompts, verbatim from the zh-CN renderer locale.
const RESUME_PROMPT: &str = "已授权草稿 {{id}} 的实际权限。请继续这个已有草稿，完成试运行、业务验证、安装与交付检查，不要新建草稿。";
const CONTINUE_TASK_PROMPT: &str = "请继续原插件任务，完成尚未通过的必需功能、验证、安装和交付检查，保留已有成果。";
const CONTINUE_CURRENT_PROMPT: &str = "插件工具已接入当前会话。请直接调用计划中当前会话用例对应的插件工具（使用计划中的输入）并核对输出，然后给出结果；不要重新创建或安装插件。";
const RUN_DEADLINE: Duration = Duration::from_secs(50 * 60);
const PAUSE_PROOF_DEADLINE: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_secs(2);
const MAX_APPROVALS: u32 = 3;
const MAX_TASK_CONTINUES: u32 = 2;
const MAX_CURRENT_CONTINUES: u32 = 2;

struct UnavailableBrowser;

#[async_trait]
impl BrowserRuntimeFactory for UnavailableBrowser {
    async fn create(
        &self,
        _request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        Err(WorkspaceError::NativeUnavailable)
    }
}

/// Owner interventions the run needed; each mirrors one conversation-card click.
#[derive(Default)]
struct Interventions {
    approvals: u32,
    task_continues: u32,
    current_continues: u32,
}

async fn request(
    client: &Client,
    server: &DesktopServer,
    phase: &'static str,
    method: Method,
    path: &str,
    body: Option<Value>,
    deadline: Duration,
) -> Result<Value, SmokeFailure> {
    let url = format!("http://127.0.0.1:{}{path}", server.loopback_port());
    let mut request = client
        .request(method, url)
        .header("x-nomi-local-trust", server.local_trust_secret());
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = tokio::time::timeout(deadline, request.send())
        .await
        .map_err(|_| SmokeFailure::new(phase, "PLUGIN_HTTP_TIMEOUT", 408))?
        .map_err(|_| SmokeFailure::new(phase, "PLUGIN_HTTP_FAILED", 502))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| SmokeFailure::new(phase, "PLUGIN_HTTP_BODY_FAILED", 502))?;
    if bytes.len() > BODY_LIMIT {
        return Err(SmokeFailure::new(phase, "PLUGIN_HTTP_BODY_TOO_LARGE", 502));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| SmokeFailure::new(phase, "PLUGIN_HTTP_BODY_INVALID", 502))?;
    if !status.is_success() {
        return Err(SmokeFailure::http(
            phase,
            axum::http::StatusCode::from_u16(status.as_u16())
                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
            &value,
        ));
    }
    super::envelope_data(phase, value)
}

fn text<'a>(phase: &'static str, value: &'a Value, pointer: &str, code: &'static str) -> Result<&'a str, SmokeFailure> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| SmokeFailure::new(phase, code, 502))
}

/// Drafts owned by this conversation, newest data from the product list.
async fn conversation_drafts(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<Vec<Value>, SmokeFailure> {
    let listed = request(client, server, "plugin.drafts", Method::GET, "/api/plugin-drafts", None, LOCAL_API_DEADLINE).await?;
    let drafts = listed
        .get("drafts")
        .and_then(Value::as_array)
        .ok_or_else(|| SmokeFailure::new("plugin.drafts", "PLUGIN_DRAFT_LIST_INVALID", 502))?;
    Ok(drafts
        .iter()
        .filter(|draft| draft.get("source_conversation_id").and_then(Value::as_str) == Some(session_id))
        .cloned()
        .collect())
}

async fn authoring(
    client: &Client,
    server: &DesktopServer,
    draft_id: &str,
) -> Result<Value, SmokeFailure> {
    request(client, server, "plugin.authoring", Method::GET,
        &format!("/api/plugin-drafts/{draft_id}/authoring"), None, LOCAL_API_DEADLINE).await
}

/// The request is headless; a queued UI verification command means the model
/// built a UI this acceptance cannot execute, so fail with a typed code.
async fn reject_ui_cases(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    for draft in conversation_drafts(client, server, session_id).await? {
        let draft_id = text("plugin.drafts", &draft, "/draft_id", "PLUGIN_DRAFT_ID_MISSING")?;
        let detail = authoring(client, server, draft_id).await?;
        if detail.get("commands").and_then(Value::as_array).is_some_and(|commands| !commands.is_empty()) {
            return Err(SmokeFailure::new("plugin.ui", "PLUGIN_UNEXPECTED_UI_CASE", 422));
        }
    }
    Ok(())
}

/// The canonical terminal of this Session's only Turn.
async fn turn_terminal(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<&'static str, SmokeFailure> {
    let mut cursor = 0_u64;
    let mut terminal = "none";
    for _ in 0..512 {
        let page = request(client, server, "plugin.events", Method::GET,
            &format!("/api/agent-sessions/{session_id}/events?after_seq={cursor}&limit=500"),
            None, LOCAL_API_DEADLINE).await?;
        let next = page.pointer("/next_cursor/seq").and_then(Value::as_u64)
            .ok_or_else(|| SmokeFailure::new("plugin.events", "PLUGIN_EVENT_CURSOR_MISSING", 502))?;
        for event in page.get("events").and_then(Value::as_array).into_iter().flatten() {
            match event.get("kind").and_then(Value::as_str) {
                Some("turn/completed") => terminal = "completed",
                Some("turn/failed") => terminal = "failed",
                Some("turn/cancelled") => terminal = "cancelled",
                _ => {}
            }
        }
        if next == cursor {
            return Ok(terminal);
        }
        if next < cursor {
            return Err(SmokeFailure::new("plugin.events", "PLUGIN_EVENT_CURSOR_REGRESSED", 409));
        }
        cursor = next;
    }
    Err(SmokeFailure::new("plugin.events", "PLUGIN_EVENT_PAGE_LIMIT", 503))
}

/// A pause can be continued only after cleanup is proven and its checkpoint
/// is retained, exactly the conditions the conversation card checks.
async fn resumable_pause(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<Value, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + PAUSE_PROOF_DEADLINE;
    loop {
        let execution = request(client, server, "plugin.execution", Method::GET,
            &format!("/api/agent-sessions/{session_id}/execution"), None, LOCAL_API_DEADLINE).await?;
        if execution.get("state").and_then(Value::as_str) == Some("paused")
            && execution.pointer("/pause/cleanup_proven").and_then(Value::as_bool) == Some(true)
            && execution.get("checkpoint_retained").and_then(Value::as_bool) == Some(true)
            && execution.get("checkpoint_digest").and_then(Value::as_str).is_some_and(|digest| !digest.is_empty())
            && execution.get("checkpoint_revision").and_then(Value::as_u64).is_some_and(|revision| revision >= 1)
        {
            return Ok(execution);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new("plugin.pause", "PLUGIN_PAUSE_NOT_RESUMABLE", 409));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Approve the pending code/permission confirmation like the conversation
/// card's checkbox and button, returning the approved draft and confirmation.
async fn approve_pending(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<(String, String), SmokeFailure> {
    for draft in conversation_drafts(client, server, session_id).await? {
        let draft_id = text("plugin.approve", &draft, "/draft_id", "PLUGIN_DRAFT_ID_MISSING")?.to_owned();
        let detail = authoring(client, server, &draft_id).await?;
        let Some(confirmation_id) = detail.pointer("/confirmation/confirmation_id").and_then(Value::as_str) else {
            continue;
        };
        let revision = detail.pointer("/draft/summary/revision").and_then(Value::as_u64)
            .ok_or_else(|| SmokeFailure::new("plugin.approve", "PLUGIN_DRAFT_REVISION_MISSING", 502))?;
        let approved = request(client, server, "plugin.approve", Method::POST,
            &format!("/api/plugin-drafts/{draft_id}/approve"),
            Some(json!({"expected_revision": revision, "confirmation_id": confirmation_id, "approved": true})),
            LOCAL_API_DEADLINE).await?;
        if approved.get("approved").and_then(Value::as_bool) != Some(true) {
            return Err(SmokeFailure::new("plugin.approve", "PLUGIN_APPROVAL_NOT_RECORDED", 409));
        }
        return Ok((draft_id, confirmation_id.to_owned()));
    }
    Err(SmokeFailure::new("plugin.approve", "PLUGIN_PAUSE_WITHOUT_CONFIRMATION", 409))
}

/// Resume the exact paused Turn with the card's input, never a second Turn.
async fn continue_paused(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    execution: &Value,
    idempotency_key: String,
    content: String,
) -> Result<(), SmokeFailure> {
    request(client, server, "plugin.continue", Method::POST,
        &format!("/api/agent-sessions/{session_id}/plugin-continuation"),
        Some(json!({
            "request": {
                "operation_id": execution["operation_id"],
                "idempotency_key": idempotency_key,
                "expected_pause_revision": execution["pause"]["revision"],
                "expected_checkpoint_revision": execution["checkpoint_revision"],
                "expected_checkpoint_digest": execution["checkpoint_digest"],
                "budget": {}
            },
            "input": {"content": content}
        })),
        TURN_COMMAND_DEADLINE).await?;
    Ok(())
}

async fn handle_pause(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    execution: &Value,
    interventions: &mut Interventions,
) -> Result<(), SmokeFailure> {
    let reason = execution.pointer("/pause/reason").and_then(Value::as_str).unwrap_or_default();
    let (key, prompt) = match reason {
        // The approval pause is a native owner-requested suspension.
        "EXECUTION_USER_REQUESTED" | "PLUGIN_AUTHORIZATION_REQUIRED" => {
            interventions.approvals += 1;
            if interventions.approvals > MAX_APPROVALS {
                return Err(SmokeFailure::new("plugin.approve", "PLUGIN_APPROVAL_LIMIT_EXCEEDED", 409));
            }
            let (draft_id, confirmation_id) = approve_pending(client, server, session_id).await?;
            (format!("plugin-approval:{confirmation_id}"), RESUME_PROMPT.replace("{{id}}", &draft_id))
        }
        "PLUGIN_CURRENT_CONVERSATION_PENDING" => {
            interventions.current_continues += 1;
            if interventions.current_continues > MAX_CURRENT_CONTINUES {
                return Err(SmokeFailure::new("plugin.continue", "PLUGIN_CURRENT_CONVERSATION_NOT_CONSUMED", 422));
            }
            (format!("plugin-continue:{}", uuid::Uuid::now_v7()), CONTINUE_CURRENT_PROMPT.to_owned())
        }
        "PLUGIN_DELIVERY_REQUIRED" | "PLUGIN_VERIFICATION_REQUIRED" => {
            interventions.task_continues += 1;
            if interventions.task_continues > MAX_TASK_CONTINUES {
                return Err(SmokeFailure::new("plugin.continue", reason.to_owned(), 422));
            }
            (format!("plugin-continue:{}", uuid::Uuid::now_v7()), CONTINUE_TASK_PROMPT.to_owned())
        }
        other => return Err(SmokeFailure::new("plugin.pause", other.to_owned(), 409)),
    };
    continue_paused(client, server, session_id, execution, key, prompt).await
}

/// Follow the Turn to its terminal, answering each host pause the way the
/// conversation card lets the owner answer it.
async fn drive(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    interventions: &mut Interventions,
) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + RUN_DEADLINE;
    let mut handled: Option<(String, u64)> = None;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new("plugin.wait", "PLUGIN_RUN_DEADLINE_EXCEEDED", 408));
        }
        reject_ui_cases(client, server, session_id).await?;
        let observed = request(client, server, "plugin.session", Method::GET,
            &format!("/api/agent-sessions/{session_id}"), None, LOCAL_API_DEADLINE).await?;
        match observed.pointer("/head/status").and_then(Value::as_str) {
            Some("ready") => {
                return match turn_terminal(client, server, session_id).await? {
                    "completed" => Ok(()),
                    "failed" => Err(SmokeFailure::new("plugin.turn", "PLUGIN_TURN_FAILED", 422)),
                    "cancelled" => Err(SmokeFailure::new("plugin.turn", "PLUGIN_TURN_CANCELLED", 422)),
                    _ => Err(SmokeFailure::new("plugin.turn", "PLUGIN_TURN_TERMINAL_MISSING", 502)),
                };
            }
            Some("paused") => {
                let execution = resumable_pause(client, server, session_id).await?;
                let identity = (
                    text("plugin.pause", &execution, "/operation_id", "PLUGIN_PAUSE_OPERATION_MISSING")?.to_owned(),
                    execution.pointer("/pause/revision").and_then(Value::as_u64)
                        .ok_or_else(|| SmokeFailure::new("plugin.pause", "PLUGIN_PAUSE_REVISION_MISSING", 502))?,
                );
                if handled.as_ref() != Some(&identity) {
                    handle_pause(client, server, session_id, &execution, interventions).await?;
                    handled = Some(identity);
                }
            }
            _ => {}
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Durable delivery facts, not model prose: the planned same-conversation case
/// carries the owner's sample, every recorded case passed, the installed
/// Plugin is the delivered artifact, and its planned Action is an Agent tool.
async fn verify_delivery(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    for draft in conversation_drafts(client, server, session_id).await? {
        let (Some(draft_id), Some(plugin_id), Some(delivered)) = (
            draft.get("draft_id").and_then(Value::as_str),
            draft.get("plugin_id").and_then(Value::as_str),
            draft.get("delivered_artifact_digest").and_then(Value::as_str),
        ) else {
            continue;
        };
        let detail = authoring(client, server, draft_id).await?;
        let verification = &detail["verification"];
        let case = &verification["plan"]["current_conversation_case"];
        if case.is_null() {
            continue;
        }
        if !case["input"].to_string().to_lowercase().contains(EXPECTED_INPUT) {
            return Err(SmokeFailure::new("plugin.verify", "PLUGIN_CASE_INPUT_MISMATCH", 422));
        }
        if !case["expected_output"].to_string().contains(EXPECTED_RESULT) {
            return Err(SmokeFailure::new("plugin.verify", "PLUGIN_CASE_OUTPUT_MISMATCH", 422));
        }
        let cases = verification["cases"].as_object()
            .filter(|cases| !cases.is_empty())
            .ok_or_else(|| SmokeFailure::new("plugin.verify", "PLUGIN_CASES_MISSING", 422))?;
        if cases.values().any(|case| case["passed"] != json!(true)) {
            return Err(SmokeFailure::new("plugin.verify", "PLUGIN_CASE_FAILED", 422));
        }
        let action = case["action"].as_str()
            .ok_or_else(|| SmokeFailure::new("plugin.verify", "PLUGIN_CASE_ACTION_MISSING", 422))?;
        let published = verification["installed_observation"]["bindings"].as_array().is_some_and(|bindings|
            bindings.iter().any(|binding| binding["point"] == "agent.tool" && binding["action"] == action));
        if !published {
            return Err(SmokeFailure::new("plugin.verify", "PLUGIN_AGENT_TOOL_NOT_PUBLISHED", 422));
        }
        let plugin = request(client, server, "plugin.installed", Method::GET,
            &format!("/api/plugins/{plugin_id}"), None, LOCAL_API_DEADLINE).await?;
        let summary = &plugin["summary"];
        if summary["enabled"] != json!(true) || !summary["trashed_at_ms"].is_null() {
            return Err(SmokeFailure::new("plugin.installed", "PLUGIN_NOT_ENABLED", 422));
        }
        if summary.pointer("/active/artifact_digest").and_then(Value::as_str) != Some(delivered) {
            return Err(SmokeFailure::new("plugin.installed", "PLUGIN_ARTIFACT_NOT_DELIVERED", 422));
        }
        if summary["has_service"] != json!(true) || summary["has_ui"] != json!(false) {
            return Err(SmokeFailure::new("plugin.installed", "PLUGIN_SHAPE_NOT_HEADLESS", 422));
        }
        return Ok(());
    }
    Err(SmokeFailure::new("plugin.verify", "PLUGIN_CURRENT_CONVERSATION_DELIVERY_MISSING", 422))
}

/// The owner asked to be told the result; a completed assistant message must carry it.
async fn verify_reported_result(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let mut cursor = 0_u64;
    for _ in 0..64 {
        let page = request(client, server, "plugin.messages", Method::GET,
            &format!("/api/agent-sessions/{session_id}/messages?after_seq={cursor}&limit=100"),
            None, LOCAL_API_DEADLINE).await?;
        let reported = page.get("messages").and_then(Value::as_array).into_iter().flatten()
            .filter_map(super::assistant_text_projection)
            .any(|projection| projection.get("content").and_then(Value::as_str)
                .is_some_and(|content| content.contains(EXPECTED_RESULT)));
        if reported {
            return Ok(());
        }
        let next = page.pointer("/next_cursor/seq").and_then(Value::as_u64)
            .ok_or_else(|| SmokeFailure::new("plugin.messages", "PLUGIN_MESSAGE_CURSOR_MISSING", 502))?;
        if next <= cursor {
            return Err(SmokeFailure::new("plugin.result", "PLUGIN_RESULT_NOT_REPORTED", 422));
        }
        cursor = next;
    }
    Err(SmokeFailure::new("plugin.messages", "PLUGIN_MESSAGE_PAGE_LIMIT", 503))
}

async fn run_chain(
    client: &Client,
    server: &DesktopServer,
    api_key: &str,
    model: &str,
    interventions: &mut Interventions,
) -> Result<(), SmokeFailure> {
    let provider = request(client, server, "plugin.provider", Method::POST, "/api/providers", Some(json!({
        "platform": "stepfun-plan",
        "name": "Live General plugin smoke",
        "base_url": super::STEPFUN_PLAN_BASE_URL,
        "auth_scheme": "bearer",
        "credentials": {"api_keys": [api_key]},
        "enabled": true,
        "initial_model": {
            "model": model,
            "enabled": true,
            "capabilities": [{
                "task": "chat",
                "traits": [],
                "protocol": "openai.chat_text",
                "connection_role": "default",
                "provider_params": {"temperature": 0.0},
                "output_limit": 4096
            }]
        },
        "connections": []
    })), LOCAL_API_DEADLINE).await?;
    if super::value_contains(&provider, api_key) {
        return Err(SmokeFailure::new("plugin.provider", "PROVIDER_RESPONSE_EXPOSED_CREDENTIAL", 502));
    }
    let provider_id = text("plugin.provider", &provider, "/provider_id", "PLUGIN_PROVIDER_ID_MISSING")?.to_owned();
    let preset = request(client, server, "plugin.preset", Method::POST,
        "/api/agent-presets/from-template/assistant.general", Some(json!({
            "reuse_existing": false,
            "display_name": "Live General plugin smoke",
            "model": {"provider_id": provider_id, "model": model},
        })), LOCAL_API_DEADLINE).await?;
    let preset_id = text("plugin.preset", &preset, "/preset/preset_id", "PLUGIN_PRESET_ID_MISSING")?.to_owned();
    // The library's create entry runs this readiness check before launching.
    let preflight = request(client, server, "plugin.preflight", Method::POST,
        "/api/conversations/plugin-preflight",
        Some(json!({"selection": {"kind": "preset", "presetId": preset_id}})), LOCAL_API_DEADLINE).await?;
    if preflight.get("status").and_then(Value::as_str) != Some("ready") {
        return Err(SmokeFailure::new("plugin.preflight", "PLUGIN_PREFLIGHT_NOT_READY", 409));
    }
    let session = request(client, server, "plugin.session", Method::POST, "/api/agent-sessions", Some(json!({
        "preset_id": preset_id,
        "title": "Live General plugin smoke",
        "required_modules": ["plugin.development"],
        "model": {"provider_id": provider_id, "model": model},
        "resource_selections": [
            {"resource_kind": "workspace", "resource_id": "default-workspace"},
            {"resource_kind": "process_session", "resource_id": "managed-process-session"},
            {"resource_kind": "project_memory", "resource_id": "default-project-memory"},
            {"resource_kind": "browser", "resource_id": "managed-browser"},
            {"resource_kind": "computer", "resource_id": "local-desktop"},
            {"resource_kind": "scheduler", "resource_id": "installation-scheduler"}
        ]
    })), LOCAL_API_DEADLINE).await?;
    let session_id = text("plugin.session", &session, "/agent_session_id", "PLUGIN_SESSION_ID_MISSING")?.to_owned();
    request(client, server, "plugin.turn", Method::POST,
        &format!("/api/agent-sessions/{session_id}/turns"),
        Some(json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
            "input": {"content": REQUEST, "plugin_delivery": {}},
        })), TURN_COMMAND_DEADLINE).await?;
    drive(client, server, &session_id, interventions).await?;
    verify_delivery(client, server, &session_id).await?;
    verify_reported_result(client, server, &session_id).await
}

/// Content-free counters from the canonical journal after shutdown.
async fn emit_plugin_summary(root: &Path, interventions: &Interventions) {
    let db_path = root.join("data").join("nomifun-backend.db");
    let Ok(pool) = nomifun_db::sqlx::SqlitePool::connect(&format!("sqlite://{}", db_path.display())).await else {
        return;
    };
    let rows: Vec<Option<String>> = nomifun_db::sqlx::query_scalar(
        "SELECT inline_json FROM agent_events WHERE kind = 'runtime/progress-recorded' ORDER BY seq"
    ).fetch_all(&pool).await.unwrap_or_default();
    let flow_rows: Vec<Value> = rows.iter().flatten()
        .filter_map(|row| serde_json::from_str::<Value>(row).ok()).collect();
    let pauses: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind = 'turn/paused'")
        .fetch_one(&pool).await.unwrap_or(0);
    pool.close().await;
    let (mut steps, mut relays, mut rejected, mut tool_errors, mut plugin_tool_calls) = (0_u32, 0_u32, 0_u32, 0_u32, 0_u32);
    let mut development = std::collections::BTreeMap::<&'static str, u32>::new();
    const ACTIONS: [&str; 11] = ["list", "open", "read", "plan", "apply", "check", "preview", "test_action", "test_ui", "install", "inspect"];
    for value in rows.into_iter().flatten().filter_map(|row| serde_json::from_str::<Value>(&row).ok()) {
        match value.pointer("/event/event").and_then(Value::as_str).unwrap_or_default() {
            "model_step_started" => steps += 1,
            "completion_check_rejected" => relays += 1,
            "model_response_rejected" => rejected += 1,
            "tool_completed" if value.pointer("/event/result/is_error").and_then(Value::as_bool) == Some(true) => tool_errors += 1,
            "tool_started" => {
                let capability = value.pointer("/event/capability_id").and_then(Value::as_str).unwrap_or_default();
                let action = value.pointer("/event/action_id").and_then(Value::as_str).unwrap_or_default();
                if capability.starts_with("plugin:") {
                    plugin_tool_calls += 1;
                } else if let Some(name) = action.strip_prefix("plugin.development/")
                    .and_then(|name| ACTIONS.iter().find(|candidate| **candidate == name)) {
                    *development.entry(name).or_default() += 1;
                }
            }
            _ => {}
        }
    }
    let calls = ACTIONS.iter()
        .map(|name| format!("{name}={}", development.get(name).copied().unwrap_or(0)))
        .collect::<Vec<_>>().join(" ");
    eprintln!("NOMIFUN_LIVE_PLUGIN_SUMMARY steps={steps} relays={relays} protocol_rejections={rejected} tool_errors={tool_errors} pauses={pauses} approvals={} task_continues={} current_continues={} plugin_tool_calls={plugin_tool_calls} {calls}",
        interventions.approvals, interventions.task_continues, interventions.current_continues);
    eprintln!("NOMIFUN_LIVE_PLUGIN_FLOW flow={}", plugin_flow(&flow_rows));
}

/// Ordered, content-free tokens from a fixed vocabulary: each settled call by
/// Action (suffix `!CODE` on error), plus relays, reviews, rejections,
/// pauses and resumes. Error codes are host constants, never model text.
fn plugin_flow(rows: &[Value]) -> String {
    fn error_code(output: &str) -> String {
        if output.contains("INVALID_TOOL_ARGUMENTS") {
            if output.contains("observed_tool_error_count") || output.contains("observed_command_failure_count") {
                "COUNTERS".to_owned()
            } else {
                "ARGS".to_owned()
            }
        } else if output.contains("Call update_plan alone first") {
            "STALE_PLAN".to_owned()
        } else if output.contains("Completion omits recorded requirements") {
            "REQUIREMENTS".to_owned()
        } else if output.contains("every accepted input") {
            "COVERAGE".to_owned()
        } else if output.contains("The plan needs reconsideration") || output.contains("The plan is closed") {
            "PLAN_GATE".to_owned()
        } else if let Some(start) = output.find("PLUGIN_") {
            output[start..].chars().take_while(|ch| ch.is_ascii_uppercase() || *ch == '_').take(48).collect()
        } else if output.contains("Evidence") {
            "EVIDENCE".to_owned()
        } else if output.contains("Capability Kernel rejected") {
            "KERNEL".to_owned()
        } else {
            "ERR".to_owned()
        }
    }
    let mut names = std::collections::BTreeMap::<String, String>::new();
    let mut tokens = Vec::new();
    for value in rows {
        let event = &value["event"];
        match event["event"].as_str().unwrap_or_default() {
            "tool_call_completed" => {
                let (Some(id), Some(name)) = (event.pointer("/call/call_id").and_then(Value::as_str),
                    event.pointer("/call/name").and_then(Value::as_str)) else { continue };
                let token = match name {
                    "update_plan" => "PLAN",
                    "report_completion" => "REPORT",
                    "ToolSearch" => "SEARCH",
                    _ => "other",
                };
                names.insert(id.to_owned(), token.to_owned());
            }
            "tool_started" => {
                let (Some(id), Some(capability), Some(action)) = (event["call_id"].as_str(),
                    event["capability_id"].as_str(), event["action_id"].as_str()) else { continue };
                let token = if capability.starts_with("plugin:") {
                    "TOOL".to_owned()
                } else if let Some(name) = action.strip_prefix("plugin.development/") {
                    name.chars().filter(|ch| ch.is_ascii_lowercase() || *ch == '_').take(16).collect()
                } else {
                    match action {
                        "workspace.files/read" => "fread",
                        "workspace.files/write" | "workspace.files/patch" => "fwrite",
                        _ => "other",
                    }.to_owned()
                };
                names.insert(id.to_owned(), token);
            }
            "tool_completed" => {
                let Some(id) = event.pointer("/result/call_id").and_then(Value::as_str) else { continue };
                if id.starts_with("agent-instructions:") { continue; }
                let mut token = names.get(id).cloned().unwrap_or_else(|| "other".to_owned());
                if event.pointer("/result/is_error").and_then(Value::as_bool) == Some(true) {
                    let output = event.pointer("/result/output/0/text").and_then(Value::as_str).unwrap_or_default();
                    token = format!("{token}!{}", error_code(output));
                }
                tokens.push(token);
            }
            "completion_check_rejected" => tokens.push("RELAY".to_owned()),
            "completion_review" => tokens.push("REVIEW".to_owned()),
            "model_response_rejected" => tokens.push("REJECTED".to_owned()),
            "turn_paused" => tokens.push("PAUSE".to_owned()),
            "execution_resumed" => tokens.push("RESUME".to_owned()),
            "turn_completed" => tokens.push("DONE".to_owned()),
            "turn_failed" => tokens.push("FAILED".to_owned()),
            _ => {}
        }
    }
    let flow = tokens.join(",");
    flow.chars().take(1800).collect()
}

pub(super) async fn run() -> Result<(), SmokeFailure> {
    let model = live_model()?;
    let api_key = required_secret_from_stdin()?;
    let root = tempfile::tempdir().map_err(|_| SmokeFailure::new("plugin.bootstrap", "TEMP_ROOT_CREATE_FAILED", 500))?;
    let data_dir = root.path().join("data");
    let work_dir = root.path().join("work");
    let log_dir = root.path().join("logs");
    for path in [&data_dir, &work_dir, &log_dir] {
        std::fs::create_dir_all(path).map_err(|_| SmokeFailure::new("plugin.bootstrap", "TEMP_DIRECTORY_CREATE_FAILED", 500))?;
    }
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".to_owned(),
        port: 0,
        data_dir,
        work_dir: Some(work_dir),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        local: true,
        log_dir: Some(log_dir),
        log_level: Some("off".to_owned()),
        command: None,
    };
    let host = DesktopHostServices {
        browser_resources: Some(Arc::new(BrowserResourceService::new(Arc::new(UnavailableBrowser)))),
        attached_chrome: Some(AttachedChromeProviderService::new()),
        ..Default::default()
    };
    let (server, keep_alive) = DesktopServer::start_with_outcome(&cli, "", None, None, None, host)
        .await
        .map_err(|_| SmokeFailure::new("plugin.bootstrap", "DESKTOP_START_FAILED", 500))?;
    let client = Client::builder().timeout(Duration::from_secs(180)).build()
        .map_err(|_| SmokeFailure::new("plugin.bootstrap", "DESKTOP_HTTP_CLIENT_FAILED", 500))?;
    let mut interventions = Interventions::default();
    let result = run_chain(&client, &server, api_key.as_str(), &model, &mut interventions).await;
    drop(client);
    let shutdown = server.shutdown_all().await
        .map_err(|_| SmokeFailure::new("plugin.shutdown", "DESKTOP_SHUTDOWN_FAILED", 500));
    drop(server);
    drop(keep_alive);
    emit_plugin_summary(root.path(), &interventions).await;
    if result.is_err() {
        super::emit_live_runtime_progress_trace(root.path()).await;
        super::emit_live_turn_failure_trace(root.path()).await;
    }
    shutdown?;
    ensure_secret_not_persisted(root.path(), api_key.as_bytes()).await?;
    result
}
