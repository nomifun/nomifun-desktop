use crate::state::OfficeRouterState;
use axum::{
    Router,
    extract::{Extension, Json, Path, Query, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nomifun_api_types::{
    ApiResponse, GetSnapshotContentRequest, ListSnapshotsRequest, OfficePreviewResponse,
    PreviewSnapshotInfoDto, SaveSnapshotRequest, SnapshotContentResponse,
    StartOfficePreviewRequest, StopOfficePreviewRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_file::path_safety::validate_path_with_extra_root;
use std::{
    path::{Path as FsPath, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};

const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(5);
static AUTHORIZATION_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

pub fn office_routes(state: OfficeRouterState) -> Router {
    Router::new()
        .route("/api/office-preview/start", post(start_preview))
        .route(
            "/api/office-preview/status/{request_id}",
            get(preview_status),
        )
        .route("/api/office-preview/stop", post(stop_preview))
        .route("/api/preview-history/list", post(list_snapshots))
        .route("/api/preview-history/save", post(save_snapshot))
        .route(
            "/api/preview-history/get-content",
            post(get_snapshot_content),
        )
        .with_state(state)
}

/// Authenticated starts grant a revocable, random bearer capability to one
/// immutable preview. No caller-controlled upstream, port or filesystem tail.
pub fn office_artifact_routes(state: OfficeRouterState) -> Router {
    Router::new()
        .route("/api/office-preview-artifacts/{capability}", get(artifact))
        .route("/api/office-preview-artifacts/{capability}/", get(artifact))
        .with_state(state)
}

async fn start_preview(
    State(state): State<OfficeRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<StartOfficePreviewRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<OfficePreviewResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    nomifun_common::validate_uuidv7(&request.request_id)
        .map_err(|_| AppError::BadRequest("request_id must be a canonical UUIDv7.".into()))?;
    let source =
        validate_office_path(&state, &request.file_path, request.workspace.as_deref()).await?;
    Ok(Json(ApiResponse::ok(state.preview_service.start(
        &user.id,
        &request.request_id,
        source,
        request.doc_type,
    )?)))
}
async fn preview_status(
    State(state): State<OfficeRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(request_id): Path<String>,
) -> Result<Json<ApiResponse<OfficePreviewResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.preview_service.status(&user.id, &request_id)?,
    )))
}
async fn stop_preview(
    State(state): State<OfficeRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<StopOfficePreviewRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    state.preview_service.stop(&user.id, &request.request_id)?;
    Ok(Json(ApiResponse::success()))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactQuery {
    request_id: String,
    nonce: String,
}
async fn artifact(
    State(state): State<OfficeRouterState>,
    Path(capability): Path<String>,
    Query(query): Query<ArtifactQuery>,
) -> Result<Response, AppError> {
    let reader = state
        .preview_service
        .artifact_reader(&capability, &query.request_id, &query.nonce)
        .await?;
    let mut response = (
        StatusCode::OK,
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(reader)),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        "text/html; charset=utf-8".parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    // The frame is opaque-origin and read-only. Office's inline renderer may
    // paint its own document; it cannot connect, navigate, submit, or import
    // scripts/fonts/images from uncontrolled origins or application endpoints.
    headers.insert(header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-src 'none'; sandbox allow-scripts".parse().unwrap());
    Ok(response)
}

// -- Snapshot handlers ----------------------------------------------------

async fn list_snapshots(
    State(state): State<OfficeRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ListSnapshotsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<PreviewSnapshotInfoDto>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let snapshots = state.snapshot_service.list(&req.target).await?;
    Ok(Json(ApiResponse::ok(snapshots)))
}

async fn save_snapshot(
    State(state): State<OfficeRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<SaveSnapshotRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<PreviewSnapshotInfoDto>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let info = state
        .snapshot_service
        .save(&req.target, &req.content)
        .await?;
    Ok(Json(ApiResponse::ok(info)))
}

async fn get_snapshot_content(
    State(state): State<OfficeRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<GetSnapshotContentRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Option<SnapshotContentResponse>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state
        .snapshot_service
        .get_content(&req.target, &req.snapshot_id)
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn validate_office_path(
    state: &OfficeRouterState,
    file_path: &str,
    workspace: Option<&str>,
) -> Result<PathBuf, AppError> {
    let roots = state.allowed_roots.clone();
    let file_path = file_path.to_owned();
    let workspace = workspace.map(str::to_owned);
    run_path_validation(
        move || {
            let allowed_roots: Vec<&FsPath> = roots.iter().map(PathBuf::as_path).collect();
            validate_path_with_extra_root(
                &file_path,
                &allowed_roots,
                workspace.as_deref().map(FsPath::new),
            )
        },
        AUTHORIZATION_TIMEOUT,
    )
    .await
}

/// Path metadata can block on a network filesystem. It never owns a preview
/// lease or creates a job: a timeout/cancel drops its receiver, so late metadata
/// completion cannot start rendering. Admission stays on the blocking worker
/// until the actual I/O ends, bounding hung filesystem workers independently
/// of the Tokio request/runtime threads.
async fn run_path_validation(
    validation: impl FnOnce() -> Result<PathBuf, AppError> + Send + 'static,
    deadline: Duration,
) -> Result<PathBuf, AppError> {
    let operation = async {
        let admission = AUTHORIZATION_SLOTS
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Internal("Preview path authorization is unavailable.".into()))?;
        tokio::task::spawn_blocking(move || {
            let _admission = admission;
            validation()
        })
        .await
        .map_err(|_| {
            AppError::Internal("Preview path authorization stopped unexpectedly.".into())
        })?
    };
    tokio::time::timeout(deadline, operation).await.map_err(|_| AppError::Timeout("Document path validation took too long. Check that the file location is accessible and retry.".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn slow_path_metadata_does_not_block_runtime_or_deliver_late_authorization() {
        let started = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let worker_started = started.clone();
        let worker_completed = completed.clone();
        let result = run_path_validation(
            move || {
                worker_started.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(100));
                worker_completed.store(true, Ordering::Release);
                Ok(PathBuf::from("/late/authorized/source.pptx"))
            },
            Duration::from_millis(20),
        )
        .await;
        assert!(matches!(result, Err(AppError::Timeout(_))));
        // Even if a busy host schedules the worker after its caller deadline,
        // its eventual metadata result has no live receiver or job authority.
        // The current-thread Tokio test was able to deliver its deadline while
        // the metadata worker remained blocked, and its result is already lost.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !completed.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(started.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn metadata_authorization_preserves_path_error_semantics() {
        let result = run_path_validation(
            || Err(AppError::Forbidden("outside the allowed sandbox".into())),
            Duration::from_secs(1),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }
}
