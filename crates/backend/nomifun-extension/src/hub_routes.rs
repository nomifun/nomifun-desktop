use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, State};
use axum::routing::{get, post};

use nomifun_api_types::{
    ApiResponse, HubExtensionListItem, HubOperationResponse, HubUpdateInfo as ApiHubUpdateInfo, InstallExtensionRequest,
};
use nomifun_common::AppError;

use crate::hub::index_manager::HubIndexManager;
use crate::hub::installer::HubInstaller;

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for Hub route handlers.
#[derive(Clone)]
pub struct HubRouterState {
    pub index_manager: HubIndexManager,
    pub installer: HubInstaller,
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the Hub router with all `/api/hub/*` routes.
///
/// All routes require authentication (applied by the caller).
pub fn hub_routes(state: HubRouterState) -> Router {
    Router::new()
        .route("/api/hub/extensions", get(get_hub_extensions))
        .route("/api/hub/install", post(install_extension))
        .route("/api/hub/retry-install", post(retry_install))
        .route("/api/hub/check-updates", post(check_updates))
        .route("/api/hub/update", post(update_extension))
        .route("/api/hub/uninstall", post(uninstall_extension))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/hub/extensions` — get Hub extension list with statuses.
async fn get_hub_extensions(
    State(state): State<HubRouterState>,
) -> Result<Json<ApiResponse<Vec<HubExtensionListItem>>>, AppError> {
    let entries = state.index_manager.load_index().await;
    let items: Vec<HubExtensionListItem> = entries
        .into_iter()
        .map(|e| {
            let status_str = serde_json::to_value(e.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "notInstalled".to_string());
            HubExtensionListItem {
                name: e.name,
                version: e.version,
                display_name: e.display_name,
                description: e.description,
                author: e.author,
                icon: e.icon,
                tags: e.tags,
                bundled: e.bundled,
                status: status_str,
            }
        })
        .collect();
    Ok(Json(ApiResponse::ok(items)))
}

/// `POST /api/hub/install` — install an extension from the Hub.
async fn install_extension(
    State(state): State<HubRouterState>,
    body: Result<Json<InstallExtensionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<HubOperationResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.installer.install(&req.name).await;
    Ok(Json(ApiResponse::ok(HubOperationResponse {
        success: result.success,
        msg: result.msg,
    })))
}

/// `POST /api/hub/retry-install` — retry a failed installation.
async fn retry_install(
    State(state): State<HubRouterState>,
    body: Result<Json<InstallExtensionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<HubOperationResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.installer.retry_install(&req.name).await;
    Ok(Json(ApiResponse::ok(HubOperationResponse {
        success: result.success,
        msg: result.msg,
    })))
}

/// `POST /api/hub/check-updates` — check for available updates.
async fn check_updates(
    State(state): State<HubRouterState>,
) -> Result<Json<ApiResponse<Vec<ApiHubUpdateInfo>>>, AppError> {
    let updates = state.installer.check_updates().await;
    let resp: Vec<ApiHubUpdateInfo> = updates
        .into_iter()
        .map(|u| ApiHubUpdateInfo {
            name: u.name,
            current_version: u.current_version,
            latest_version: u.latest_version,
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

/// `POST /api/hub/update` — update an installed extension.
async fn update_extension(
    State(state): State<HubRouterState>,
    body: Result<Json<InstallExtensionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<HubOperationResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.installer.update(&req.name).await;
    Ok(Json(ApiResponse::ok(HubOperationResponse {
        success: result.success,
        msg: result.msg,
    })))
}

/// `POST /api/hub/uninstall` — uninstall an extension.
async fn uninstall_extension(
    State(state): State<HubRouterState>,
    body: Result<Json<InstallExtensionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<HubOperationResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.installer.uninstall(&req.name).await;
    Ok(Json(ApiResponse::ok(HubOperationResponse {
        success: result.success,
        msg: result.msg,
    })))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ExtensionRegistry;
    use crate::state::ExtensionStateStore;
    use nomifun_realtime::BroadcastEventBus;
    use std::sync::Arc;

    fn make_state() -> HubRouterState {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStateStore::new(tmp.path().join("states.json"));
        let bus = Arc::new(BroadcastEventBus::new(64));
        let hub_dir = tmp.path().to_path_buf();
        std::mem::forget(tmp);
        let registry = ExtensionRegistry::new(store, bus, "1.0.0".into());
        let index_manager = HubIndexManager::new(hub_dir, registry.clone());
        let installer = HubInstaller::new(index_manager.clone(), registry);
        HubRouterState {
            index_manager,
            installer,
        }
    }

    #[test]
    fn hub_routes_builds_router() {
        let state = make_state();
        let _router = hub_routes(state);
    }
}
