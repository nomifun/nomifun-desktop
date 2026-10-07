//! Real-provider smoke for the official General Agent in a desktop-capable host.
//! Browser and Computer owners are present, but the task uses only chat/files.

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
    BODY_LIMIT, LOCAL_API_DEADLINE, POLL_INTERVAL, TURN_COMMAND_DEADLINE, SmokeFailure, ensure_secret_not_persisted,
    first_durable_error_code, live_model, required_secret_from_stdin,
};

const REPLY_MARKER: &str = "NOMIFUN_GENERAL_DESKTOP_LIVE_OK";
const FILE_MARKER: &str = "NOMIFUN_GENERAL_DESKTOP_FILE_OK";

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

async fn request(
    client: &Client,
    server: &DesktopServer,
    phase: &'static str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, SmokeFailure> {
    let url = format!("http://127.0.0.1:{}{path}", server.loopback_port());
    let mut request = client
        .request(method, url)
        .header("x-nomi-local-trust", server.local_trust_secret());
    if let Some(body) = body {
        request = request.json(&body);
    }
    let deadline = if phase == "general.turn" { TURN_COMMAND_DEADLINE } else { LOCAL_API_DEADLINE };
    let response = tokio::time::timeout(deadline, request.send())
        .await
        .map_err(|_| SmokeFailure::new(phase, "DESKTOP_HTTP_TIMEOUT", 408))?
        .map_err(|_| SmokeFailure::new(phase, "DESKTOP_HTTP_FAILED", 502))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| SmokeFailure::new(phase, "DESKTOP_HTTP_BODY_FAILED", 502))?;
    if bytes.len() > BODY_LIMIT {
        return Err(SmokeFailure::new(phase, "DESKTOP_HTTP_BODY_TOO_LARGE", 502));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| SmokeFailure::new(phase, "DESKTOP_HTTP_BODY_INVALID", 502))?;
    if !status.is_success() {
        return Err(SmokeFailure::http(
            phase,
            axum::http::StatusCode::from_u16(status.as_u16()).unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
            &value,
        ));
    }
    super::envelope_data(phase, value)
}

async fn messages_after(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    after_seq: u64,
) -> Result<(Vec<Value>, u64), SmokeFailure> {
    let mut cursor = after_seq;
    let mut all = Vec::new();
    for _ in 0..32 {
        let page = request(
            client,
            server,
            "general.messages",
            Method::GET,
            &format!("/api/agent-sessions/{session_id}/messages?after_seq={cursor}&limit=100"),
            None,
        )
        .await?;
        let next = page.pointer("/next_cursor/seq").and_then(Value::as_u64)
            .ok_or_else(|| SmokeFailure::new("general.messages", "GENERAL_MESSAGE_CURSOR_MISSING", 502))?;
        let page_messages = page.get("messages").and_then(Value::as_array)
            .ok_or_else(|| SmokeFailure::new("general.messages", "GENERAL_MESSAGES_MISSING", 502))?;
        all.extend(page_messages.iter().cloned());
        if next == cursor {
            return Ok((all, cursor));
        }
        if next < cursor {
            return Err(SmokeFailure::new("general.messages", "GENERAL_MESSAGE_CURSOR_REGRESSED", 409));
        }
        cursor = next;
    }
    Err(SmokeFailure::new("general.messages", "GENERAL_MESSAGE_PAGE_LIMIT", 503))
}

async fn wait_ready(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    after_seq: u64,
    phase: &'static str,
) -> Result<(Vec<Value>, u64), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    loop {
        let (messages, cursor) = messages_after(client, server, session_id, after_seq).await?;
        if let Some(code) = first_durable_error_code(&messages) {
            return Err(SmokeFailure::new(phase, code, 422));
        }
        let observed = request(
            client,
            server,
            phase,
            Method::GET,
            &format!("/api/agent-sessions/{session_id}"),
            None,
        ).await?;
        if observed.pointer("/head/status").and_then(Value::as_str) == Some("ready") {
            return Ok((messages, cursor));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new(phase, "GENERAL_TURN_DEADLINE_EXCEEDED", 408));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn send_turn(
    client: &Client,
    server: &DesktopServer,
    session_id: &str,
    prompt: String,
) -> Result<(), SmokeFailure> {
    request(
        client,
        server,
        "general.turn",
        Method::POST,
        &format!("/api/agent-sessions/{session_id}/turns"),
        Some(json!({
            "input": {"content": prompt},
            "idempotency_key": uuid::Uuid::now_v7().to_string(),
        })),
    ).await?;
    Ok(())
}

async fn run_chain(
    client: &Client,
    server: &DesktopServer,
    api_key: &str,
    model: &str,
    work_dir: &Path,
) -> Result<(), SmokeFailure> {
    let provider = request(client, server, "general.provider", Method::POST, "/api/providers", Some(json!({
        "platform": "stepfun-plan",
        "name": "Live General desktop smoke",
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
    }))).await?;
    let provider_id = provider.get("provider_id").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("general.provider", "GENERAL_PROVIDER_ID_MISSING", 502))?;
    let preset = request(client, server, "general.preset", Method::POST,
        "/api/agent-presets/from-template/assistant.general", Some(json!({
            "reuse_existing": false,
            "display_name": "Live General desktop smoke",
            "model": {"provider_id": provider_id, "model": model},
        }))).await?;
    let preset_id = preset.pointer("/preset/preset_id").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("general.preset", "GENERAL_PRESET_ID_MISSING", 502))?;
    let session = request(client, server, "general.session", Method::POST,
        "/api/agent-sessions", Some(json!({
            "preset_id": preset_id,
            "title": "Live General desktop smoke",
            "model": {"provider_id": provider_id, "model": model},
            "resource_selections": [
                {"resource_kind":"workspace","resource_id":"default-workspace"},
                {"resource_kind":"process_session","resource_id":"managed-process-session"},
                {"resource_kind":"project_memory","resource_id":"default-project-memory"},
                {"resource_kind":"browser","resource_id":"managed-browser"},
                {"resource_kind":"computer","resource_id":"local-desktop"},
                {"resource_kind":"scheduler","resource_id":"installation-scheduler"}
            ]
        }))).await?;
    let session_id = session.get("agent_session_id").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("general.session", "GENERAL_SESSION_ID_MISSING", 502))?;

    send_turn(client, server, session_id,
        format!("This checks the official General Agent with a real model. Do not call tools. Reply with exactly {REPLY_MARKER} and no other text.")).await?;
    let (first_messages, cursor) = wait_ready(client, server, session_id, 0, "general.reply").await?;
    if super::exact_assistant_marker_count(&first_messages, REPLY_MARKER) != 1 {
        return Err(SmokeFailure::new("general.reply", "GENERAL_REPLY_MARKER_MISSING", 422));
    }

    send_turn(client, server, session_id,
        format!("Use write_file to create general-desktop-smoke.txt in the workspace root containing exactly {FILE_MARKER}. Check it, then briefly reply.")).await?;
    let (file_messages, cursor) = wait_ready(client, server, session_id, cursor, "general.file").await?;
    if std::fs::read_to_string(work_dir.join("general-desktop-smoke.txt")).ok().as_deref() != Some(FILE_MARKER) {
        return Err(SmokeFailure::new("general.file", "GENERAL_FILE_MISSING_OR_WRONG", 422));
    }
    let listing = request(client, server, "general.listing", Method::GET,
        &format!("/api/agent-sessions/{session_id}/workspace?path=."), None).await?;
    if !listing.as_array().is_some_and(|entries| entries.iter().any(|item|
        item.get("name").and_then(Value::as_str) == Some("general-desktop-smoke.txt")
    )) {
        return Err(SmokeFailure::new("general.listing", "GENERAL_FILE_NOT_LISTED", 422));
    }
    if !file_messages.iter().any(|message|
        message.get("presentation_intent").and_then(Value::as_str) == Some("tool")
            && message.pointer("/projection/tool_summary/name").and_then(Value::as_str) == Some("write_file")
            && message.pointer("/projection/state").and_then(Value::as_str) == Some("recorded")
    ) {
        return Err(SmokeFailure::new("general.file", "GENERAL_WRITE_RECEIPT_MISSING", 422));
    }

    send_turn(client, server, session_id,
        "写一个贪吃蛇的游戏，保存为项目根目录的 snake_game.html。要求直接在浏览器打开即可游玩，支持方向键控制、计分和重新开始。".to_owned()).await?;
    let (game_messages, _) = wait_ready(client, server, session_id, cursor, "general.game").await?;
    let game = std::fs::read_to_string(work_dir.join("snake_game.html"))
        .map_err(|_| SmokeFailure::new("general.game", "GENERAL_GAME_FILE_MISSING", 422))?;
    let game_lower = game.to_lowercase();
    if game.len() < 1000
        || !game_lower.contains("<html")
        || !game_lower.contains("<script")
        || !game_lower.contains("canvas")
        || !game_lower.contains("keydown")
        || !(game_lower.contains("score") || game.contains("得分"))
        || !(game_lower.contains("restart") || game.contains("重新开始"))
    {
        return Err(SmokeFailure::new("general.game", "GENERAL_GAME_CONTENT_INCOMPLETE", 422));
    }
    let listing = request(client, server, "general.game_listing", Method::GET,
        &format!("/api/agent-sessions/{session_id}/workspace?path=."), None).await?;
    if !listing.as_array().is_some_and(|entries| entries.iter().any(|item|
        item.get("name").and_then(Value::as_str) == Some("snake_game.html")
    )) {
        return Err(SmokeFailure::new("general.game_listing", "GENERAL_GAME_NOT_LISTED", 422));
    }
    if !game_messages.iter().any(|message|
        message.get("presentation_intent").and_then(Value::as_str) == Some("tool")
            && message.pointer("/projection/tool_summary/name").and_then(Value::as_str) == Some("write_file")
            && message.pointer("/projection/state").and_then(Value::as_str) == Some("recorded")
    ) {
        return Err(SmokeFailure::new("general.game", "GENERAL_GAME_WRITE_RECEIPT_MISSING", 422));
    }
    Ok(())
}

pub(super) async fn run() -> Result<(), SmokeFailure> {
    let model = live_model()?;
    let api_key = required_secret_from_stdin()?;
    let root = tempfile::tempdir().map_err(|_| SmokeFailure::new("general.bootstrap", "TEMP_ROOT_CREATE_FAILED", 500))?;
    let data_dir = root.path().join("data");
    let work_dir = root.path().join("work");
    let log_dir = root.path().join("logs");
    for path in [&data_dir, &work_dir, &log_dir] {
        std::fs::create_dir_all(path).map_err(|_| SmokeFailure::new("general.bootstrap", "TEMP_DIRECTORY_CREATE_FAILED", 500))?;
    }
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".to_owned(),
        port: 0,
        data_dir,
        work_dir: Some(work_dir.clone()),
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
        .map_err(|_| SmokeFailure::new("general.bootstrap", "DESKTOP_START_FAILED", 500))?;
    let client = Client::builder().timeout(Duration::from_secs(180)).build()
        .map_err(|_| SmokeFailure::new("general.bootstrap", "DESKTOP_HTTP_CLIENT_FAILED", 500))?;
    let result = run_chain(&client, &server, api_key.as_str(), &model, &work_dir).await;
    drop(client);
    let shutdown = server.shutdown_all().await
        .map_err(|_| SmokeFailure::new("general.shutdown", "DESKTOP_SHUTDOWN_FAILED", 500));
    drop(server);
    drop(keep_alive);
    shutdown?;
    ensure_secret_not_persisted(root.path(), api_key.as_bytes()).await?;
    result
}
