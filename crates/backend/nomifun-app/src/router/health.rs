//! Health check + Guide MCP status diagnostic + MCP registration template endpoints.

use axum::Json;
use nomifun_api_types::{ApiResponse, GuideMcpConfig};
use nomifun_common::AppError;
use nomifun_terminal::AgentCli;
use serde::{Deserialize, Serialize};

use crate::commands::mcp_register_template::{RegisterTemplate, knowledge_register_template};
use crate::commands::register_knowledge::{RegisterOutcome, register_into_workpath};
use crate::commands::register_knowledge_global::{
    UnregisterOutcome, is_registered_global, register_global, unregister_global,
};

#[derive(Serialize)]
pub(super) struct HealthResponse {
    status: &'static str,
    version: &'static str,
    build_time: &'static str,
}

pub(super) async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        build_time: env!("BUILD_TIME"),
    })
}

#[derive(Serialize)]
pub(super) struct GuideMcpStatusResponse {
    running: bool,
    port: Option<u16>,
    binary_path: Option<String>,
}

pub(super) async fn guide_mcp_status(
    axum::extract::State(cfg): axum::extract::State<Option<GuideMcpConfig>>,
) -> Json<GuideMcpStatusResponse> {
    Json(match cfg {
        Some(c) => GuideMcpStatusResponse {
            running: true,
            port: Some(c.port),
            binary_path: Some(c.binary_path),
        },
        None => GuideMcpStatusResponse {
            running: false,
            port: None,
            binary_path: None,
        },
    })
}

/// GET /api/terminals/mcp-register-template — returns registration snippets for
/// the platform knowledge MCP bridge (no token/port baked in).
pub(super) async fn mcp_register_template_handler(
) -> Result<Json<ApiResponse<RegisterTemplate>>, AppError> {
    let nomicore_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nomicore".to_owned());
    Ok(Json(ApiResponse::ok(knowledge_register_template(&nomicore_path))))
}

// ---------------------------------------------------------------------------
// POST /api/terminals/register-knowledge — one-click registration
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct RegisterKnowledgeRequest {
    cwd: String,
    family: String,
}

/// Parse a family string to `AgentCli`, returning 400 on invalid input.
fn parse_family(s: &str) -> Result<AgentCli, AppError> {
    match s.to_ascii_lowercase().as_str() {
        "claude" => Ok(AgentCli::Claude),
        "codex" => Ok(AgentCli::Codex),
        "gemini" => Ok(AgentCli::Gemini),
        _ => Err(AppError::BadRequest(format!(
            "invalid family '{}': must be one of claude, codex, gemini",
            s
        ))),
    }
}

/// POST /api/terminals/register-knowledge — write/merge the platform knowledge
/// MCP into the CLI's auto-discovery file at the given cwd.
pub(super) async fn register_knowledge_handler(
    Json(req): Json<RegisterKnowledgeRequest>,
) -> Result<Json<ApiResponse<RegisterOutcome>>, AppError> {
    let family = parse_family(&req.family)?;
    let nomicore = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nomicore".to_owned());

    let outcome = register_into_workpath(&req.cwd, family, &nomicore).map_err(|e| {
        AppError::Internal(format!("register knowledge failed: {e}"))
    })?;

    Ok(Json(ApiResponse::ok(outcome)))
}

// ---------------------------------------------------------------------------
// Global (user-scope) knowledge MCP registration — any-directory CLI access
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct KnowledgeGlobalRequest {
    family: String,
}

/// POST /api/terminals/register-knowledge-global — register the knowledge MCP
/// into the family's USER-level config so a CLI started in ANY directory loads
/// it. Secret-free (the bridge resolves port/token from the beacon at runtime).
pub(super) async fn register_knowledge_global_handler(
    Json(req): Json<KnowledgeGlobalRequest>,
) -> Result<Json<ApiResponse<RegisterOutcome>>, AppError> {
    let family = parse_family(&req.family)?;
    let nomicore = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nomicore".to_owned());
    let outcome =
        register_global(family, &nomicore).map_err(|e| AppError::Internal(format!("register knowledge (global) failed: {e}")))?;
    Ok(Json(ApiResponse::ok(outcome)))
}

/// POST /api/terminals/unregister-knowledge-global — remove the global
/// registration (idempotent).
pub(super) async fn unregister_knowledge_global_handler(
    Json(req): Json<KnowledgeGlobalRequest>,
) -> Result<Json<ApiResponse<UnregisterOutcome>>, AppError> {
    let family = parse_family(&req.family)?;
    let outcome =
        unregister_global(family).map_err(|e| AppError::Internal(format!("unregister knowledge (global) failed: {e}")))?;
    Ok(Json(ApiResponse::ok(outcome)))
}

/// GET /api/terminals/knowledge-global-status — per-family registration state.
/// File-based for claude/gemini; codex reports `null` (unknown without invoking
/// the CLI).
pub(super) async fn knowledge_global_status_handler() -> Json<ApiResponse<serde_json::Value>> {
    let status = serde_json::json!({
        "claude": is_registered_global(AgentCli::Claude),
        "codex": is_registered_global(AgentCli::Codex),
        "gemini": is_registered_global(AgentCli::Gemini),
    });
    Json(ApiResponse::ok(status))
}
