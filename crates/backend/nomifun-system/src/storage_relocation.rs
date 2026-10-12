//! The online API schedules an offline dataset move; it never changes live roots.
use axum::extract::{Json, State, rejection::JsonRejection};
use nomifun_agent_session::AgentSessionStore;
use nomifun_api_types::{ApiResponse, PrepareStorageRelocationRequest,
    StorageRelocationOperationRequest, StorageRelocationResponse, StorageRelocationBlocker};
use nomifun_common::{AppError, storage_relocation::{self as storage, RelocationPlan, RelocationPhase}};
use crate::SystemRouterState;

fn response(state: &SystemRouterState, plan: Option<&RelocationPlan>) -> StorageRelocationResponse {
    StorageRelocationResponse {
        operation_id: plan.map(|p| p.operation_id.clone()),
        phase: plan.map(|p| serde_json::to_value(p.phase).unwrap().as_str().unwrap().to_owned()).unwrap_or_else(|| "idle".into()),
        current_dir: state.data_dir.to_string_lossy().into_owned(),
        target_dir: plan.map(|p| p.destination.data_dir.to_string_lossy().into_owned()),
        total_bytes: plan.map_or(0, |p| p.total_bytes), file_count: plan.map_or(0, |p| p.file_count),
        blockers: Vec::new(), warnings: Vec::new(),
        restart_required: plan.is_some_and(|p| matches!(p.phase, RelocationPhase::AwaitingRestart | RelocationPhase::Copying | RelocationPhase::Verifying)
            || (p.phase == RelocationPhase::Failed && !p.cancellable)),
        cancellable: plan.is_some_and(|p| p.cancellable), error: plan.and_then(|p| p.error.clone()),
    }
}

async fn blockers(state: &SystemRouterState) -> Result<Vec<StorageRelocationBlocker>, AppError> {
    if state.work_dir_is_cli_override {
        return Ok(vec![StorageRelocationBlocker { code: "STARTUP_OVERRIDE".into(),
            message: "Remove the --work-dir startup option before relocating storage in Settings.".into() }]);
    }
    let sessions = AgentSessionStore::from_pool(state.database_pool.clone()).await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if let Err(error) = nomifun_db::validate_storage_relocation_paths(&state.database_pool, &state.data_dir, &state.work_dir).await {
        return Ok(vec![StorageRelocationBlocker { code: "OWNED_STORAGE_PATH_CONFLICT".into(), message: error.to_string() }]);
    }
    Ok(match sessions.validate_workspace_relocation(&state.data_dir, &state.work_dir).await {
        Ok(()) => Vec::new(),
        Err(error) => vec![StorageRelocationBlocker { code: "DATASET_BUSY_OR_UNSETTLED".into(), message: error.to_string() }],
    })
}

pub(crate) async fn prepare(State(state): State<SystemRouterState>, body: Result<Json<PrepareStorageRelocationRequest>, JsonRejection>)
    -> Result<Json<ApiResponse<StorageRelocationResponse>>, AppError> {
    let Json(request) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut result = response(&state, None);
    result.blockers = blockers(&state).await?;
    result.target_dir = Some(request.target_dir.clone());
    if !result.blockers.is_empty() { return Ok(Json(ApiResponse::ok(result))); }
    let control = storage::control_dir_for_root(&state.data_dir)?;
    let data = state.data_dir.clone(); let work = state.work_dir.clone();
    let target = std::path::PathBuf::from(request.target_dir);
    let plan = tokio::task::spawn_blocking(move || storage::request_storage_relocation(&control, &data, &work, &target))
        .await.map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::ok(response(&state, Some(&plan)))))
}

pub(crate) async fn start(State(state): State<SystemRouterState>, body: Result<Json<StorageRelocationOperationRequest>, JsonRejection>)
    -> Result<Json<ApiResponse<StorageRelocationResponse>>, AppError> {
    let Json(request) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let blocked = blockers(&state).await?;
    if !blocked.is_empty() { return Err(AppError::Conflict(blocked[0].message.clone())); }
    let control = storage::control_dir_for_root(&state.data_dir)?;
    let plan = storage::pending_storage_relocation(&control)?
        .ok_or_else(|| AppError::Conflict("no pending storage relocation".into()))?;
    if !nomifun_common::paths::paths_equivalent(&plan.source.data_dir, &state.data_dir)
        || !nomifun_common::paths::paths_equivalent(&plan.source.work_dir, &state.work_dir) {
        return Err(AppError::Conflict("storage plan belongs to another active dataset".into()));
    }
    let plan = storage::start_storage_relocation(&control, &request.operation_id)?;
    Ok(Json(ApiResponse::ok(response(&state, Some(&plan)))))
}

pub(crate) async fn status(State(state): State<SystemRouterState>) -> Result<Json<ApiResponse<StorageRelocationResponse>>, AppError> {
    let control = storage::control_dir_for_root(&state.data_dir)?;
    let plan = storage::storage_relocation_status(&control)?;
    Ok(Json(ApiResponse::ok(response(&state, plan.as_ref()))))
}

pub(crate) async fn cancel(State(state): State<SystemRouterState>, body: Result<Json<StorageRelocationOperationRequest>, JsonRejection>)
    -> Result<Json<ApiResponse<StorageRelocationResponse>>, AppError> {
    let Json(request) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let control = storage::control_dir_for_root(&state.data_dir)?;
    let plan = storage::cancel_storage_relocation(&control, &request.operation_id)?;
    Ok(Json(ApiResponse::ok(response(&state, Some(&plan)))))
}
