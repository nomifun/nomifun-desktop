//! `/api/agent/model-failover` — read/write the global Phase-3 model-failover
//! config (plan D8, review #6/#12).
//!
//! The frontend (`ipcBridge.ts` `agentModelFailover`) calls GET/PUT here to edit
//! the failover queue. The config is one JSON blob in `client_preferences` under
//! `agent.model_failover`; the storage helpers live in `nomifun_conversation`
//! (`get_global_failover_config` / `set_global_failover_config`). This module is
//! a thin authenticated router around them — `nomifun-app` is the only crate that
//! depends on both `nomifun-conversation` and the SQLite client-pref repo, so the
//! route is registered here rather than in the agent-listing router (which lives
//! in `nomifun-ai-agent`, below conversation in the dependency graph).

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, State};
use axum::routing::get;
use axum::Router;

use nomifun_api_types::{ApiResponse, ModelFailoverConfig};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_conversation::model_failover::{get_global_failover_config, set_global_failover_config};
use nomifun_db::IClientPreferenceRepository;

/// Router state: the client-preference repo that backs the global failover config.
#[derive(Clone)]
pub struct ModelFailoverRouterState {
    pub client_prefs: Arc<dyn IClientPreferenceRepository>,
}

/// Mounts `GET`/`PUT /api/agent/model-failover` (path must match the frontend's
/// `agentModelFailover` exactly).
pub fn model_failover_routes(state: ModelFailoverRouterState) -> Router {
    Router::new()
        .route("/api/agent/model-failover", get(get_config).put(put_config))
        .with_state(state)
}

/// GET — return the saved global failover config (defaults to disabled when
/// unset / malformed; see `get_global_failover_config`).
async fn get_config(
    State(state): State<ModelFailoverRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<ModelFailoverConfig>>, AppError> {
    let cfg = get_global_failover_config(&state.client_prefs).await;
    Ok(Json(ApiResponse::ok(cfg)))
}

/// PUT — persist the given config and echo back the saved value.
async fn put_config(
    State(state): State<ModelFailoverRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ModelFailoverConfig>, JsonRejection>,
) -> Result<Json<ApiResponse<ModelFailoverConfig>>, AppError> {
    let Json(cfg) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_global_failover_config(&state.client_prefs, &cfg).await?;
    Ok(Json(ApiResponse::ok(cfg)))
}
