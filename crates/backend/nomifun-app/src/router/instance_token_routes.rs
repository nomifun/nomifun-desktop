//! Installation-scoped Remote access-token endpoints (local desktop only).
//!
//! POST /api/webui/access-token  — mint/rotate (plaintext returned once)
//! DELETE /api/webui/access-token — revoke
//! GET /api/webui/access-token    — status `{ configured }`

use std::sync::Arc;

use axum::extract::State;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use axum::{Json, Router};
use nomifun_api_types::ApiResponse;
use nomifun_auth::{
    AuthState, InstanceOwnerState, auth_middleware, require_instance_owner_middleware,
    require_local_trust_middleware,
};
use nomifun_common::AppError;
use nomifun_db::IInstanceTokenRepository;

#[derive(Clone)]
pub struct InstanceTokenRouterState {
    pub provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
    pub token_repo: Arc<dyn IInstanceTokenRepository>,
    pub token_validator: Arc<nomifun_auth::InstanceTokenValidator>,
    pub admission: Arc<nomifun_auth::RemoteAuthAdmissionFence>,
}

#[derive(serde::Serialize)]
struct AccessTokenMintResponse {
    /// Plaintext token — shown exactly once, never persisted nor re-emitted.
    token: String,
    /// Advisory only: authentication is valid without a model, but Agent tools
    /// need at least one enabled provider with a chat model.
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
}

#[derive(serde::Serialize)]
struct AccessTokenStatusResponse {
    configured: bool,
}

async fn has_enabled_provider(state: &InstanceTokenRouterState) -> bool {
    state
        .provider_repo
        .list()
        .await
        .is_ok_and(|providers| providers.iter().any(|provider| provider.enabled))
}

async fn mint(
    State(state): State<InstanceTokenRouterState>,
) -> Result<Json<ApiResponse<AccessTokenMintResponse>>, AppError> {
    let _admission = state.admission.acquire_auth_mutation().await;
    let token = nomifun_auth::generate_random_hex_secret();
    let hash = nomifun_auth::token_sha256_hex(&token);
    state.token_repo.set(&hash).await?;
    state.token_validator.set_token(hash);

    let warning = (!has_enabled_provider(&state).await).then(|| {
        "本机尚未配置启用的 provider；令牌可以连接 NomiFun Desktop，但需要模型的 Agent 能力会在配置 provider 前失败。"
            .to_owned()
    });
    Ok(Json(ApiResponse::ok(AccessTokenMintResponse { token, warning })))
}

async fn revoke(
    State(state): State<InstanceTokenRouterState>,
) -> Result<Json<ApiResponse<AccessTokenStatusResponse>>, AppError> {
    let _admission = state.admission.acquire_auth_mutation().await;
    state.token_repo.clear().await?;
    state.token_validator.clear_token();
    Ok(Json(ApiResponse::ok(AccessTokenStatusResponse { configured: false })))
}

async fn status(
    State(state): State<InstanceTokenRouterState>,
) -> Result<Json<ApiResponse<AccessTokenStatusResponse>>, AppError> {
    let _admission = state.admission.acquire_request_admission().await;
    Ok(Json(ApiResponse::ok(AccessTokenStatusResponse {
        configured: state.token_validator.is_configured(),
    })))
}

pub fn instance_token_routes(state: InstanceTokenRouterState) -> Router {
    Router::new()
        .route("/api/webui/access-token", get(status).post(mint).delete(revoke))
        .route_layer(from_fn(require_local_trust_middleware))
        .with_state(state)
}

/// Installation-token management for authenticated standalone Web hosts.
///
/// Desktop keeps the local-trust variant above so its own webview can manage
/// the token without a login. A network Web host has no local-trust secret and
/// must instead require the authenticated installation owner.
pub fn instance_token_routes_authenticated(
    state: InstanceTokenRouterState,
    auth_state: AuthState,
    owner_state: InstanceOwnerState,
) -> Router {
    Router::new()
        .route("/api/webui/access-token", get(status).post(mint).delete(revoke))
        .route_layer(from_fn_with_state(
            owner_state,
            require_instance_owner_middleware,
        ))
        .route_layer(from_fn_with_state(auth_state, auth_middleware))
        .with_state(state)
}
