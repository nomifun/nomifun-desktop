//! Health check and owner-local external MCP registration endpoints.

use axum::Json;
use nomifun_api_types::ApiResponse;
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

pub(super) async fn mcp_register_template_handler(
) -> Result<Json<ApiResponse<RegisterTemplate>>, AppError> {
    Ok(Json(ApiResponse::ok(knowledge_register_template(
        &current_nomicore_path(),
    ))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RegisterKnowledgeRequest {
    cwd: String,
    family: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KnowledgeGlobalRequest {
    family: String,
}

pub(super) async fn register_knowledge_handler(
    Json(request): Json<RegisterKnowledgeRequest>,
) -> Result<Json<ApiResponse<RegisterOutcome>>, AppError> {
    let family = parse_family(&request.family)?;
    let nomicore = current_nomicore_path();
    let outcome = tokio::task::spawn_blocking(move || {
        register_into_workpath(&request.cwd, family, &nomicore)
    })
    .await
    .map_err(|error| AppError::Internal(format!("registration task failed: {error}")))?
    .map_err(|error| AppError::Internal(format!("register knowledge failed: {error}")))?;
    Ok(Json(ApiResponse::ok(outcome)))
}

pub(super) async fn register_knowledge_global_handler(
    Json(request): Json<KnowledgeGlobalRequest>,
) -> Result<Json<ApiResponse<RegisterOutcome>>, AppError> {
    let family = parse_family(&request.family)?;
    let nomicore = current_nomicore_path();
    let outcome = tokio::task::spawn_blocking(move || register_global(family, &nomicore))
        .await
        .map_err(|error| AppError::Internal(format!("registration task failed: {error}")))?
        .map_err(|error| AppError::Internal(format!("register knowledge failed: {error}")))?;
    Ok(Json(ApiResponse::ok(outcome)))
}

pub(super) async fn unregister_knowledge_global_handler(
    Json(request): Json<KnowledgeGlobalRequest>,
) -> Result<Json<ApiResponse<UnregisterOutcome>>, AppError> {
    let family = parse_family(&request.family)?;
    let outcome = tokio::task::spawn_blocking(move || unregister_global(family))
        .await
        .map_err(|error| AppError::Internal(format!("unregistration task failed: {error}")))?
        .map_err(|error| AppError::Internal(format!("unregister knowledge failed: {error}")))?;
    Ok(Json(ApiResponse::ok(outcome)))
}

pub(super) async fn knowledge_global_status_handler(
) -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::ok(serde_json::json!({
        "claude": is_registered_global(AgentCli::Claude),
        "codex": is_registered_global(AgentCli::Codex),
        "gemini": is_registered_global(AgentCli::Gemini),
    })))
}

fn parse_family(raw: &str) -> Result<AgentCli, AppError> {
    match raw.to_ascii_lowercase().as_str() {
        "claude" => Ok(AgentCli::Claude),
        "codex" => Ok(AgentCli::Codex),
        "gemini" => Ok(AgentCli::Gemini),
        _ => Err(AppError::BadRequest(format!(
            "invalid family {raw:?}; expected claude, codex, or gemini"
        ))),
    }
}

fn current_nomicore_path() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nomicore".into())
}
