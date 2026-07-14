//! Per-companion Remote access-token endpoints (local desktop client only).
//!
//! POST /api/webui/companions/{id}/access-token  — mint (plaintext returned once)
//! DELETE …                                       — revoke
//! GET …                                          — status { configured }
//!
//! Lives in nomifun-app (not nomifun-auth) because the mint-time model-availability
//! guard needs `CompanionService`, and nomifun-companion already depends on
//! nomifun-auth (so auth must not depend back on companion).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::middleware::from_fn;
use axum::routing::get;
use axum::{Json, Router};
use nomifun_api_types::ApiResponse;
use nomifun_auth::require_local_trust_middleware;
use nomifun_common::AppError;
use nomifun_db::ICompanionTokenRepository;

#[derive(Clone)]
pub struct CompanionTokenRouterState {
    pub companion_service: Arc<nomifun_companion::CompanionService>,
    pub provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
    pub token_repo: Arc<dyn ICompanionTokenRepository>,
    pub token_validator: Arc<nomifun_auth::CompanionTokenValidator>,
}

#[derive(serde::Serialize)]
struct AccessTokenMintResponse {
    /// Plaintext token — shown exactly once, never persisted nor re-emitted.
    token: String,
    /// The companion this token is bound to.
    companion_id: String,
    /// Advisory warning when the companion has no resolvable model (the token
    /// still mints; model-dependent capabilities like nomi_delegate will fail
    /// until a model/provider is configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

#[derive(serde::Serialize)]
struct AccessTokenStatusResponse {
    configured: bool,
}

/// True if `nomi_delegate` and related Agent capabilities can resolve a model for this
/// companion: either the companion's own profile model is set, or any provider is
/// enabled (the first-enabled-provider fallback in the gateway's resolution chain).
async fn companion_model_resolvable(
    state: &CompanionTokenRouterState,
    profile: &nomifun_companion::CompanionProfileConfig,
) -> bool {
    if profile.model.is_configured() {
        return true;
    }
    match state.provider_repo.list().await {
        Ok(providers) => providers.iter().any(|p| p.enabled),
        Err(_) => false,
    }
}

async fn mint(
    State(state): State<CompanionTokenRouterState>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<AccessTokenMintResponse>>, AppError> {
    // 404 if the companion does not exist.
    let profile = state.companion_service.get_companion(&companion_id).await?;

    let token = nomifun_auth::generate_random_hex_secret();
    let hash = nomifun_auth::token_sha256_hex(&token);
    state.token_repo.upsert_for_companion(&companion_id, &hash).await?;
    state.token_validator.insert_token(companion_id.clone(), hash);

    let warning = if companion_model_resolvable(&state, &profile).await {
        None
    } else {
        Some(
            "该伙伴尚未配置可用模型，且本机无启用的 provider；外部调用 nomi_delegate 等需要模型的能力会失败。请先在桌面应用「模型管理」(/models) 配置 provider 与模型，并为该伙伴指定模型。"
                .to_string(),
        )
    };

    Ok(Json(ApiResponse::ok(AccessTokenMintResponse { token, companion_id, warning })))
}

async fn revoke(
    State(state): State<CompanionTokenRouterState>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<AccessTokenStatusResponse>>, AppError> {
    state.token_repo.delete_for_companion(&companion_id).await?;
    state.token_validator.remove_token(&companion_id);
    Ok(Json(ApiResponse::ok(AccessTokenStatusResponse { configured: false })))
}

async fn status(
    State(state): State<CompanionTokenRouterState>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<AccessTokenStatusResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(AccessTokenStatusResponse {
        configured: state.token_validator.is_configured_for(&companion_id),
    })))
}

pub fn companion_token_routes(state: CompanionTokenRouterState) -> Router {
    Router::new()
        .route(
            "/api/webui/companions/{companion_id}/access-token",
            get(status).post(mint).delete(revoke),
        )
        .route_layer(from_fn(require_local_trust_middleware))
        .with_state(state)
}
