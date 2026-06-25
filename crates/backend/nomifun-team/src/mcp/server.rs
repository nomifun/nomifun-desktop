use std::net::SocketAddr;
use std::sync::{Arc, Weak};

use nomifun_api_types::{TeamMcpPhase, TeamMcpStatusPayload, WebSocketMessage};
use nomifun_realtime::EventBroadcaster;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::error::TeamError;
use crate::scheduler::TeammateManager;
use crate::service::TeamSessionService;
use crate::session::SpawnAgentRequest;
use crate::types::{TeammateRole, TeammateStatus};

use super::protocol::{
    INVALID_PARAMS, INVALID_REQUEST, JsonRpcResponse, METHOD_NOT_FOUND, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION,
    read_request, write_response,
};
use super::tools::{
    RenameAgentInput, SendMessageInput, ShutdownAgentInput, SpawnAgentInput, TaskCreateInput, TaskUpdateInput,
    all_tool_descriptors, handle_team_describe_assistant, handle_team_list_models,
};

// ---------------------------------------------------------------------------
// TeamMcpServer
// ---------------------------------------------------------------------------

pub struct TeamMcpServer {
    addr: SocketAddr,
    http_addr: SocketAddr,
    auth_token: String,
    shutdown_tx: watch::Sender<bool>,
}

impl TeamMcpServer {
    pub async fn start(
        auth_token: String,
        scheduler: Arc<TeammateManager>,
        team_id: String,
        broadcaster: Arc<dyn EventBroadcaster>,
        service: Weak<TeamSessionService>,
    ) -> Result<Self, TeamError> {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                broadcast_mcp_status(
                    broadcaster.as_ref(),
                    TeamMcpStatusPayload {
                        team_id: team_id.clone(),
                        slot_id: String::new(),
                        phase: TeamMcpPhase::TcpError,
                        port: None,
                        server_count: None,
                        error: Some(e.to_string()),
                    },
                );
                return Err(TeamError::InvalidRequest(format!("Failed to bind TCP: {e}")));
            }
        };
        let addr = listener
            .local_addr()
            .map_err(|e| TeamError::InvalidRequest(format!("Failed to get local addr: {e}")))?;

        broadcast_mcp_status(
            broadcaster.as_ref(),
            TeamMcpStatusPayload {
                team_id: team_id.clone(),
                slot_id: String::new(),
                phase: TeamMcpPhase::TcpReady,
                port: Some(addr.port()),
                server_count: None,
                error: None,
            },
        );

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let token = auth_token.clone();
        let sched_for_tcp = scheduler.clone();
        let service_for_tcp = service.clone();
        let team_id_for_tcp = team_id.clone();
        tokio::spawn(accept_loop(
            listener,
            token,
            sched_for_tcp,
            service_for_tcp,
            team_id_for_tcp,
            shutdown_rx.clone(),
        ));

        // HTTP MCP endpoint for agents that prefer http transport.
        let http_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| TeamError::InvalidRequest(format!("Failed to bind HTTP: {e}")))?;
        let http_addr = http_listener
            .local_addr()
            .map_err(|e| TeamError::InvalidRequest(format!("Failed to get HTTP addr: {e}")))?;

        let http_token = auth_token.clone();
        let http_sched = scheduler.clone();
        let http_service = service.clone();
        let http_team_id = team_id.clone();
        tokio::spawn(http_mcp_loop(
            http_listener,
            http_token,
            http_sched,
            http_service,
            http_team_id,
            shutdown_rx,
        ));

        debug!(
            tcp_port = addr.port(),
            http_port = http_addr.port(),
            "Team MCP Server started"
        );

        Ok(Self {
            addr,
            http_addr,
            auth_token,
            shutdown_tx,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn http_port(&self) -> u16 {
        self.http_addr.port()
    }

    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
        debug!(port = self.addr.port(), "Team MCP Server stop requested");
    }
}

impl Drop for TeamMcpServer {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

fn broadcast_mcp_status(broadcaster: &dyn EventBroadcaster, payload: TeamMcpStatusPayload) {
    let event = WebSocketMessage::new(
        "team.mcpStatus",
        serde_json::to_value(payload).expect("serialize mcp status payload"),
    );
    broadcaster.broadcast(event);
}

// ---------------------------------------------------------------------------
// Accept loop
// ---------------------------------------------------------------------------

async fn accept_loop(
    listener: TcpListener,
    auth_token: String,
    scheduler: Arc<TeammateManager>,
    service: Weak<TeamSessionService>,
    team_id: String,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, peer)) => {
                        debug!(?peer, "New MCP connection");
                        let token = auth_token.clone();
                        let sched = Arc::clone(&scheduler);
                        let svc = service.clone();
                        let tid = team_id.clone();
                        tokio::spawn(handle_connection(stream, token, sched, svc, tid));
                    }
                    Err(e) => {
                        error!("Accept error: {e}");
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    debug!("MCP server shutting down");
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: TcpStream,
    auth_token: String,
    scheduler: Arc<TeammateManager>,
    service: Weak<TeamSessionService>,
    team_id: String,
) {
    let (mut reader, mut writer) = tokio::io::split(stream);

    let mut authenticated = false;
    let mut caller_slot_id: Option<String> = None;

    loop {
        let request = match read_request(&mut reader).await {
            Ok(req) => req,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                warn!("Read error: {e}");
                break;
            }
        };

        if request.id.is_none() {
            continue;
        }

        let response = if !authenticated {
            match handle_initialize(&request, &auth_token) {
                InitResult::Authenticated(slot_id, resp) => {
                    info!(team_id = %team_id, slot_id = %slot_id, "MCP agent authenticated");
                    authenticated = true;
                    caller_slot_id = Some(slot_id);
                    resp
                }
                InitResult::Response(resp) => {
                    warn!(team_id = %team_id, method = %request.method, "MCP auth rejected");
                    resp
                }
            }
        } else {
            handle_method(
                &request,
                &scheduler,
                &service,
                &team_id,
                caller_slot_id.as_deref().unwrap_or("unknown"),
            )
            .await
        };

        if write_response(&mut writer, &response).await.is_err() {
            warn!(team_id = %team_id, "MCP connection write failed, closing");
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Initialize / handshake
// ---------------------------------------------------------------------------

enum InitResult {
    Authenticated(String, JsonRpcResponse),
    Response(JsonRpcResponse),
}

fn handle_initialize(request: &super::protocol::JsonRpcRequest, auth_token: &str) -> InitResult {
    if request.method != "initialize" {
        return InitResult::Response(JsonRpcResponse::error(
            request.id,
            INVALID_REQUEST,
            "Expected 'initialize' as first request",
        ));
    }

    let params = request.params.as_ref();

    let token = params
        .and_then(|p| p.get("auth_token"))
        .or_else(|| params.and_then(|p| p.get("authToken")))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if token != auth_token {
        return InitResult::Response(JsonRpcResponse::error(
            request.id,
            INVALID_REQUEST,
            "Authentication failed: invalid auth_token",
        ));
    }

    let slot_id = params
        .and_then(|p| p.get("slot_id"))
        .or_else(|| params.and_then(|p| p.get("slotId")))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    let resp = JsonRpcResponse::success(
        request.id,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            },
            "capabilities": {
                "tools": {}
            }
        }),
    );

    InitResult::Authenticated(slot_id, resp)
}

// ---------------------------------------------------------------------------
// Method router
// ---------------------------------------------------------------------------

async fn handle_method(
    request: &super::protocol::JsonRpcRequest,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
) -> JsonRpcResponse {
    match request.method.as_str() {
        "notifications/initialized" => JsonRpcResponse::success(request.id, json!({})),
        "tools/list" => handle_tools_list(request.id),
        "tools/call" => handle_tools_call(request, scheduler, service, team_id, caller_slot_id).await,
        _ => JsonRpcResponse::error(
            request.id,
            METHOD_NOT_FOUND,
            format!("Unknown method: {}", request.method),
        ),
    }
}

fn handle_tools_list(id: Option<u64>) -> JsonRpcResponse {
    let tools = all_tool_descriptors();
    JsonRpcResponse::success(id, json!({ "tools": tools }))
}

// ---------------------------------------------------------------------------
// tools/call dispatcher
// ---------------------------------------------------------------------------

async fn handle_tools_call(
    request: &super::protocol::JsonRpcRequest,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
) -> JsonRpcResponse {
    let params = match request.params.as_ref() {
        Some(p) => p,
        None => {
            return JsonRpcResponse::error(request.id, INVALID_PARAMS, "Missing params for tools/call");
        }
    };

    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return JsonRpcResponse::error(request.id, INVALID_PARAMS, "Missing 'name' in tools/call params");
        }
    };

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let caller_role = match scheduler.get_agent(caller_slot_id).await {
        Ok(agent) => agent.role,
        Err(_) => TeammateRole::Teammate,
    };

    info!(
        team_id = %team_id,
        caller = %caller_slot_id,
        tool = %tool_name,
        "MCP tools/call invoked"
    );

    let result = dispatch_tool(
        tool_name,
        &arguments,
        scheduler,
        service,
        team_id,
        caller_slot_id,
        caller_role,
    )
    .await;

    match &result {
        Ok(_) => info!(team_id = %team_id, tool = %tool_name, caller = %caller_slot_id, "MCP tool call succeeded"),
        Err(e) => {
            warn!(team_id = %team_id, tool = %tool_name, caller = %caller_slot_id, error = %e, "MCP tool call failed")
        }
    }

    match result {
        Ok(content) => JsonRpcResponse::success(
            request.id,
            json!({
                "content": [{ "type": "text", "text": content }]
            }),
        ),
        Err(err_msg) => JsonRpcResponse::success(
            request.id,
            json!({
                "content": [{ "type": "text", "text": err_msg }],
                "isError": true
            }),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

pub(crate) async fn dispatch_tool(
    tool_name: &str,
    arguments: &Value,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
    caller_role: TeammateRole,
) -> Result<String, String> {
    match tool_name {
        "team_send_message" => exec_send_message(arguments, scheduler, service, team_id, caller_slot_id).await,
        "team_spawn_agent" => exec_spawn_agent(arguments, service, team_id, caller_slot_id, caller_role).await,
        "team_task_create" => exec_task_create(arguments, scheduler).await,
        "team_task_update" => exec_task_update(arguments, scheduler).await,
        "team_task_list" => exec_task_list(scheduler).await,
        "team_members" => exec_members(scheduler).await,
        "team_rename_agent" => exec_rename_agent(arguments, scheduler, service, team_id).await,
        "team_shutdown_agent" => {
            exec_shutdown_agent(arguments, scheduler, service, team_id, caller_slot_id, caller_role).await
        }
        "team_list_models" => exec_list_models(arguments, service).await,
        "team_describe_assistant" => exec_describe_assistant(arguments).await,
        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

async fn exec_list_models(args: &Value, service: &Weak<TeamSessionService>) -> Result<String, String> {
    let agent_type_filter = args.get("agent_type").and_then(Value::as_str);

    let value = match service.upgrade() {
        Some(svc) => svc.list_models_from_db(agent_type_filter).await,
        None => handle_team_list_models(args),
    };
    serde_json::to_string_pretty(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn exec_describe_assistant(args: &Value) -> Result<String, String> {
    Ok(handle_team_describe_assistant(args))
}

// ---------------------------------------------------------------------------
// Individual tool handlers
// ---------------------------------------------------------------------------

async fn resolve_agent_target(scheduler: &TeammateManager, target: &str) -> Result<String, String> {
    let agents = scheduler.list_agents().await;
    if agents.iter().any(|a| a.slot_id == target) {
        return Ok(target.to_owned());
    }
    let query = target.to_lowercase();
    let hits: Vec<_> = agents.iter().filter(|a| a.name.to_lowercase() == query).collect();
    match hits.len() {
        0 => Err(format!("No agent matches '{target}'")),
        1 => Ok(hits[0].slot_id.clone()),
        _ => Err(format!("Multiple agents match '{target}'")),
    }
}

async fn exec_send_message(
    args: &Value,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
) -> Result<String, String> {
    let input: SendMessageInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    let trimmed = input.message.trim();
    if trimmed == "shutdown_approved" {
        debug!(from = caller_slot_id, "shutdown_approved intercepted");
        scheduler.notify_shutdown_acknowledged(caller_slot_id);

        // Deferred cleanup: kill process, delete conversation, remove from team DB.
        // Spawned so the MCP response can be sent back before the process is killed.
        let slot = caller_slot_id.to_owned();
        let tid = team_id.to_owned();
        let svc_weak = service.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Some(svc) = svc_weak.upgrade() {
                let user_id = svc.get_session_user_id(&tid).await.unwrap_or_default();
                if let Err(e) = svc.remove_agent(&user_id, &tid, &slot).await {
                    warn!(slot_id = %slot, error = %e, "shutdown cleanup failed");
                } else {
                    info!(slot_id = %slot, "agent fully removed after shutdown_approved");
                }
            }
        });

        return Ok(json!({"status": "shutdown_approved_received"}).to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("shutdown_rejected:") {
        let reason = rest.trim();
        scheduler
            .notify_shutdown_rejected(caller_slot_id, reason)
            .await
            .map_err(|e| e.to_string())?;
        debug!(from = caller_slot_id, reason, "shutdown_rejected handled");
        return Ok(format!("shutdown_rejected: {reason}"));
    }

    let resolved_to = if input.to == "*" {
        "*".to_owned()
    } else {
        resolve_agent_target(scheduler, &input.to).await?
    };

    let action = crate::scheduler::SchedulerAction::SendMessage {
        to: resolved_to.clone(),
        message: input.message,
    };
    scheduler
        .execute_action(caller_slot_id, &action)
        .await
        .map_err(|e| e.to_string())?;

    // Always notify target agent(s). If the event loop is in the drain
    // loop (working), the notify permit will be consumed on next iteration.
    // If idle/waiting, it wakes immediately.
    if let Some(svc) = service.upgrade() {
        let targets = if resolved_to == "*" {
            scheduler
                .list_agents()
                .await
                .iter()
                .filter(|a| a.slot_id != caller_slot_id)
                .map(|a| a.slot_id.clone())
                .collect::<Vec<_>>()
        } else {
            vec![resolved_to.clone()]
        };
        for target in &targets {
            if let Err(e) = svc.wake_agent_in_session(team_id, target).await {
                debug!(team_id, target = target.as_str(), error = %e, "wake after send_message failed (non-fatal)");
            }
        }
    }

    Ok(format!("Message sent to {}", input.to))
}

async fn exec_spawn_agent(
    args: &Value,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
    caller_role: TeammateRole,
) -> Result<String, String> {
    // Lead-only at the MCP dispatch layer. `TeamSession::spawn_agent` also
    // re-checks via `TeamError::LeaderOnly`, but the dispatch-level string
    // keeps the user-visible "Only Lead ..." phrasing that the MCP client
    // (and existing protocol tests) expect.
    if caller_role != TeammateRole::Lead {
        return Err("Only Lead can spawn agents".into());
    }
    let input: SpawnAgentInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    // Requested name — normalization / emptiness / uniqueness live in
    // `TeamSession::spawn_agent` so we do not double-validate here.
    let requested_name = input.name.clone();

    // `agent_type` is the Nomi-spec field name; `backend` is the legacy
    // phase-1 alias. Either (or neither — session then inherits from the
    // caller) is accepted.
    let agent_type = input.agent_type.or(input.backend);

    // Dynamic capability check happens in `TeamSession::spawn_agent` which
    // queries both the hard whitelist and persisted MCP capabilities.

    let req = SpawnAgentRequest {
        name: requested_name.clone(),
        agent_type,
        custom_agent_id: input.custom_agent_id,
        model: input.model,
    };

    let service = service
        .upgrade()
        .ok_or_else(|| "Team service not available; cannot spawn agent".to_string())?;

    service
        .spawn_agent_in_session(team_id, caller_slot_id, req)
        .await
        .map(|agent| format!("Agent '{}' spawned (slot_id={})", agent.name, agent.slot_id))
        .map_err(|e| e.to_string())
}

async fn exec_task_create(args: &Value, scheduler: &TeammateManager) -> Result<String, String> {
    let input: TaskCreateInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    let action = crate::scheduler::SchedulerAction::TaskCreate {
        subject: input.subject.clone(),
        description: input.description,
        owner: input.owner,
        blocked_by: input.blocked_by.unwrap_or_default(),
    };
    scheduler
        .execute_action("system", &action)
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("Task '{}' created", input.subject))
}

async fn exec_task_update(args: &Value, scheduler: &TeammateManager) -> Result<String, String> {
    let input: TaskUpdateInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    let action = crate::scheduler::SchedulerAction::TaskUpdate {
        task_id: input.task_id.clone(),
        status: input.status,
        description: input.description,
        owner: input.owner,
        blocked_by: input.blocked_by,
    };
    scheduler
        .execute_action("system", &action)
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("Task '{}' updated", input.task_id))
}

async fn exec_task_list(scheduler: &TeammateManager) -> Result<String, String> {
    let tasks = scheduler.list_tasks().await.map_err(|e| e.to_string())?;
    let output: Vec<Value> = tasks
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "subject": t.subject,
                "description": t.description,
                "status": t.status,
                "owner": t.owner,
                "blocked_by": t.blocked_by,
                "blocks": t.blocks,
            })
        })
        .collect();
    serde_json::to_string_pretty(&output).map_err(|e| format!("Serialization error: {e}"))
}

async fn exec_members(scheduler: &TeammateManager) -> Result<String, String> {
    let agents = scheduler.list_agents().await;
    let output: Vec<Value> = agents
        .iter()
        .map(|a| {
            // `TeamAgent::status` is `None` for cold-start agents that have not
            // yet transitioned through `set_status` (e.g. the lead before its
            // first wake). The scheduler already tracks them as `Idle`
            // internally (see `TeammateManager::new`), and Nomi's
            // TeammateManager exposes `'idle'` as the initial value. Mirror
            // that here so MCP clients never see `null` and misread a live
            // teammate as offline.
            let status = a.status.unwrap_or(TeammateStatus::Idle);
            json!({
                "slot_id": a.slot_id,
                "name": a.name,
                "role": a.role,
                "status": status,
                "backend": a.backend,
                "model": a.model,
            })
        })
        .collect();
    serde_json::to_string_pretty(&output).map_err(|e| format!("Serialization error: {e}"))
}

async fn exec_rename_agent(
    args: &Value,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
) -> Result<String, String> {
    let input: RenameAgentInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    let resolved_slot = resolve_agent_target(scheduler, &input.slot_id).await?;

    if let Some(svc) = service.upgrade() {
        svc.rename_agent(team_id, &resolved_slot, &input.new_name)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        scheduler
            .rename_agent(&resolved_slot, &input.new_name)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(format!("Agent '{}' renamed to '{}'", input.slot_id, input.new_name))
}

async fn exec_shutdown_agent(
    args: &Value,
    scheduler: &TeammateManager,
    service: &Weak<TeamSessionService>,
    team_id: &str,
    caller_slot_id: &str,
    caller_role: TeammateRole,
) -> Result<String, String> {
    if caller_role != TeammateRole::Lead {
        return Err("Only Lead can shut down agents".into());
    }
    let input: ShutdownAgentInput = serde_json::from_value(args.clone()).map_err(|e| format!("Invalid params: {e}"))?;

    let target_slot_id = resolve_agent_target(scheduler, &input.slot_id).await?;
    let action = crate::scheduler::SchedulerAction::ShutdownAgent {
        slot_id: target_slot_id.clone(),
        reason: input.reason,
    };
    scheduler
        .execute_action(caller_slot_id, &action)
        .await
        .map_err(|e| e.to_string())?;

    // Wake the target agent so it reads the shutdown_request from its mailbox.
    if let Some(svc) = service.upgrade()
        && let Err(e) = svc.wake_agent_in_session(team_id, &target_slot_id).await
    {
        debug!(team_id, target = %target_slot_id, error = %e, "wake after shutdown_request failed (non-fatal)");
    }

    Ok(format!("Shutdown request sent to agent '{}'", target_slot_id))
}

// ---------------------------------------------------------------------------
// HTTP MCP endpoint (Streamable HTTP transport for MCP)
// ---------------------------------------------------------------------------

async fn http_mcp_loop(
    listener: TcpListener,
    auth_token: String,
    scheduler: Arc<TeammateManager>,
    service: Weak<TeamSessionService>,
    team_id: String,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let Ok((mut stream, peer)) = accept else { continue };
                info!(team_id = %team_id, ?peer, "HTTP MCP: new connection accepted");
                let _token = auth_token.clone();
                let sched = scheduler.clone();
                let svc = service.clone();
                let tid = team_id.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);

                    // Extract JSON body (after \r\n\r\n)
                    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                    let Ok(value): Result<Value, _> = serde_json::from_str(body) else {
                        let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        return;
                    };

                    // Handle JSON-RPC request
                    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
                    let id = value.get("id").cloned();

                    let result = match method {
                        "initialize" => {
                            json!({
                                "capabilities": { "tools": {} },
                                "protocolVersion": PROTOCOL_VERSION,
                                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
                            })
                        }
                        "notifications/initialized" => {
                            let resp = "HTTP/1.1 204 No Content\r\n\r\n";
                            let _ = stream.write_all(resp.as_bytes()).await;
                            return;
                        }
                        "tools/list" => {
                            let tools: Vec<Value> = all_tool_descriptors()
                                .iter()
                                .map(|d| json!({
                                    "name": d.name,
                                    "description": d.description,
                                    "inputSchema": d.input_schema,
                                }))
                                .collect();
                            json!({ "tools": tools })
                        }
                        "tools/call" => {
                            let params = value.get("params").cloned().unwrap_or(json!({}));
                            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
                            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                            let caller_slot_id = request.lines()
                                .find(|l| l.to_lowercase().starts_with("x-slot-id:"))
                                .and_then(|l| l.split_once(':').map(|(_, v)| v.trim()))
                                .unwrap_or("");
                            match dispatch_tool(
                                tool_name,
                                &arguments,
                                &sched,
                                &svc,
                                &tid,
                                caller_slot_id,
                                TeammateRole::Lead,
                            )
                            .await
                            {
                                Ok(text) => json!({ "content": [{"type": "text", "text": text}] }),
                                Err(text) => json!({ "content": [{"type": "text", "text": text}], "isError": true }),
                            }
                        }
                        _ => {
                            json!({"error": {"code": -32601, "message": "Method not found"}})
                        }
                    };

                    let response_body = if result.get("error").is_some() {
                        json!({"jsonrpc": "2.0", "id": id, "error": result["error"]})
                    } else {
                        json!({"jsonrpc": "2.0", "id": id, "result": result})
                    };
                    let body_bytes = serde_json::to_vec(&response_body).unwrap_or_default();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        body_bytes.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body_bytes).await;
                });
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — exec_spawn_agent dispatch-layer unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Non-Lead callers are rejected at the dispatch layer with the
    /// "Only Lead ..." phrasing. Service weak is never upgraded because
    /// the early role gate short-circuits.
    #[tokio::test]
    async fn exec_spawn_agent_rejects_non_lead() {
        let service: Weak<TeamSessionService> = Weak::new();
        let args = json!({ "name": "Helper", "agent_type": "claude" });
        let result = exec_spawn_agent(&args, &service, "team-1", "worker-1", TeammateRole::Teammate).await;
        let err = result.expect_err("non-Lead caller must be rejected");
        assert!(
            err.contains("Only Lead"),
            "error must keep legacy 'Only Lead' phrasing, got {err:?}"
        );
    }

    /// Malformed JSON body is rejected before the service is consulted.
    #[tokio::test]
    async fn exec_spawn_agent_rejects_malformed_args() {
        let service: Weak<TeamSessionService> = Weak::new();
        // `name` missing entirely — SpawnAgentInput requires it.
        let args = json!({ "agent_type": "claude" });
        let result = exec_spawn_agent(&args, &service, "team-1", "lead-1", TeammateRole::Lead).await;
        let err = result.expect_err("malformed args must be rejected");
        assert!(
            err.contains("Invalid params"),
            "must surface Invalid params for JSON deserialize failure, got {err:?}"
        );
    }

    /// Lead caller with a well-formed request but no live service (Weak
    /// cannot upgrade) surfaces the service-unavailable error rather than
    /// silently returning a fake success. This is the path exercised in
    /// tests where the MCP server is spun up without a real
    /// `TeamSessionService` — in production the Weak always upgrades.
    #[tokio::test]
    async fn exec_spawn_agent_reports_service_unavailable_when_weak_dead() {
        let service: Weak<TeamSessionService> = Weak::new();
        let args = json!({
            "name": "Helper",
            "agent_type": "claude",
            "model": "claude-sonnet-4"
        });
        let result = exec_spawn_agent(&args, &service, "team-1", "lead-1", TeammateRole::Lead).await;
        let err = result.expect_err("dead Weak<TeamSessionService> must not succeed");
        assert!(
            err.contains("Team service not available"),
            "dead service weak must surface the unavailable message, got {err:?}"
        );
    }

    /// The dispatch layer must accept both the new `agent_type` field and
    /// the legacy `backend` alias so existing phase-1 callers (that still
    /// send `backend`) do not regress.
    #[tokio::test]
    async fn exec_spawn_agent_accepts_legacy_backend_alias() {
        let service: Weak<TeamSessionService> = Weak::new();
        // Use `backend` (legacy) instead of `agent_type` — parsing must succeed
        // and we must reach the service-upgrade step (and then fail because
        // Weak::new cannot upgrade). If `backend` were rejected at parse time
        // the error would be "Invalid params".
        let args = json!({ "name": "Helper", "backend": "claude" });
        let result = exec_spawn_agent(&args, &service, "team-1", "lead-1", TeammateRole::Lead).await;
        let err = result.expect_err("dead Weak<TeamSessionService> must not succeed");
        assert!(
            err.contains("Team service not available"),
            "legacy 'backend' alias must parse through to service-upgrade step, got {err:?}"
        );
    }
}
