use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Query, State};
use axum::routing::{get, post};
use std::path::Path;
use tower_http::limit::RequestBodyLimitLayer;

use nomifun_api_types::{
    ApiResponse, BrowseDirectoryQuery, BrowseDirectoryResponse, CancelZipRequest, CopyFilesRequest, CopyFilesResponse,
    CreateTempFileRequest, DirOrFileResponse, FetchRemoteImageRequest, FileChangeInfoResponse, FileMetadataResponse,
    FileWatchRequest, GetFileMetadataRequest, GetFilesByDirRequest, GetImageBase64Request, ListWorkspaceFilesRequest,
    ReadFileBufferRequest, ReadFileRequest, RemoveEntryRequest, RenameRequest, RenameResponse, SnapshotBaselineRequest,
    SnapshotCompareResponse, SnapshotDiscardRequest, SnapshotInfoResponse, SnapshotStageRequest,
    SnapshotWorkspaceRequest, WorkspaceFlatFileResponse, WorkspaceOfficeWatchRequest, WriteFileRequest, ZipRequest,
};
use nomifun_common::AppError;
use nomifun_common::constants::UPLOAD_MAX_SIZE;

use crate::browse;
use crate::traits::{FileServiceRef, FileWatchServiceRef, SnapshotServiceRef};
use crate::types::{
    CompareResult, CopyResult, DirOrFile, FileChangeInfo, FileMetadata, SnapshotInfo, SnapshotMode, WorkspaceFlatFile,
    ZipEntry,
};

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for all file-related route handlers.
#[derive(Clone)]
pub struct FileRouterState {
    pub file_service: FileServiceRef,
    pub watch_service: FileWatchServiceRef,
    pub snapshot_service: SnapshotServiceRef,
    pub allowed_roots: Vec<std::path::PathBuf>,
    /// Roots permitted by the shallow `/api/fs/browse` endpoint. This is
    /// typically wider than `allowed_roots` (it includes `cwd`, Windows
    /// drive letters, and `/` on Unix) because the WebUI host-file picker
    /// legitimately needs to reach outside any single workspace.
    pub browse_roots: Vec<std::path::PathBuf>,
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the file router with all `/api/fs/*` routes.
///
/// All routes require authentication (applied by the caller).
pub fn file_routes(state: FileRouterState) -> Router {
    // Upload route carries its own body-size limit (UPLOAD_MAX_SIZE, 30 MB).
    // We first disable the global `DefaultBodyLimit` that `nomifun-app`
    // installs (otherwise the `Multipart` extractor would cap the body at
    // `BODY_LIMIT`), then apply `RequestBodyLimitLayer` as the sole hard
    // cap. The layers are added in outer->inner order via `.layer()`.
    let upload_router = Router::new()
        .route("/api/fs/upload", post(upload_file))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(UPLOAD_MAX_SIZE))
        .with_state(state.clone());

    Router::new()
        // A. Core file operations
        .route("/api/fs/browse", get(browse_directory))
        .route("/api/fs/dir", post(get_files_by_dir))
        .route("/api/fs/list", post(list_workspace_files))
        .route("/api/fs/metadata", post(get_file_metadata))
        .route("/api/fs/read", post(read_file))
        .route("/api/fs/read-buffer", post(read_file_buffer))
        .route("/api/fs/write", post(write_file))
        .route("/api/fs/copy", post(copy_files))
        .route("/api/fs/remove", post(remove_entry))
        .route("/api/fs/rename", post(rename_entry))
        .route("/api/fs/temp", post(create_temp_file))
        .route("/api/fs/image-base64", post(get_image_base64))
        .route("/api/fs/fetch-remote-image", post(fetch_remote_image))
        .route("/api/fs/zip", post(create_zip))
        .route("/api/fs/zip/cancel", post(cancel_zip))
        // D. File watch
        .route("/api/fs/watch/start", post(start_watch))
        .route("/api/fs/watch/stop", post(stop_watch))
        .route("/api/fs/watch/stop-all", post(stop_all_watches))
        .route("/api/fs/office-watch/start", post(start_office_watch))
        .route("/api/fs/office-watch/stop", post(stop_office_watch))
        // E. Workspace snapshot
        .route("/api/fs/snapshot/init", post(snapshot_init))
        .route("/api/fs/snapshot/info", post(snapshot_info))
        .route("/api/fs/snapshot/compare", post(snapshot_compare))
        .route("/api/fs/snapshot/baseline", post(snapshot_baseline))
        .route("/api/fs/snapshot/stage", post(snapshot_stage_file))
        .route("/api/fs/snapshot/stage-all", post(snapshot_stage_all))
        .route("/api/fs/snapshot/unstage", post(snapshot_unstage_file))
        .route("/api/fs/snapshot/unstage-all", post(snapshot_unstage_all))
        .route("/api/fs/snapshot/discard", post(snapshot_discard))
        .route("/api/fs/snapshot/reset", post(snapshot_reset))
        .route("/api/fs/snapshot/branches", post(snapshot_branches))
        .route("/api/fs/snapshot/dispose", post(snapshot_dispose))
        .with_state(state)
        .merge(upload_router)
}

// ---------------------------------------------------------------------------
// A. Core file operations — handlers
// ---------------------------------------------------------------------------

/// `GET /api/fs/browse` — shallow directory listing for the WebUI host-file
/// picker. Runs on the Tokio blocking pool because it does synchronous
/// filesystem I/O.
async fn browse_directory(
    State(state): State<FileRouterState>,
    Query(query): Query<BrowseDirectoryQuery>,
) -> Result<Json<ApiResponse<BrowseDirectoryResponse>>, AppError> {
    let show_files = matches!(query.show_files.as_deref(), Some("true") | Some("1"));
    let raw_path = query.path.clone();
    let roots = state.browse_roots.clone();

    let response = tokio::task::spawn_blocking(move || browse::browse(raw_path.as_deref(), show_files, &roots))
        .await
        .map_err(|e| AppError::Internal(format!("browse task failed: {}", e)))??;

    Ok(Json(ApiResponse::ok(response)))
}

async fn get_files_by_dir(
    State(state): State<FileRouterState>,
    body: Result<Json<GetFilesByDirRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<DirOrFileResponse>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let items = state.file_service.get_files_by_dir(&req.dir, &req.root).await?;
    let response: Vec<DirOrFileResponse> = items.into_iter().map(to_dir_or_file_response).collect();
    Ok(Json(ApiResponse::ok(response)))
}

async fn list_workspace_files(
    State(state): State<FileRouterState>,
    body: Result<Json<ListWorkspaceFilesRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<WorkspaceFlatFileResponse>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let items = state.file_service.list_workspace_files(&req.root).await?;
    let response: Vec<WorkspaceFlatFileResponse> = items.into_iter().map(to_flat_file_response).collect();
    Ok(Json(ApiResponse::ok(response)))
}

async fn get_file_metadata(
    State(state): State<FileRouterState>,
    body: Result<Json<GetFileMetadataRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<FileMetadataResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let meta = state
        .file_service
        .get_file_metadata(&req.path, req.workspace.as_deref().map(Path::new))
        .await?;
    Ok(Json(ApiResponse::ok(to_metadata_response(meta))))
}

async fn read_file(
    State(state): State<FileRouterState>,
    body: Result<Json<ReadFileRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Option<String>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let content = state
        .file_service
        .read_file(&req.path, req.workspace.as_deref().map(Path::new))
        .await?;
    Ok(Json(ApiResponse::ok(content)))
}

async fn read_file_buffer(
    State(state): State<FileRouterState>,
    body: Result<Json<ReadFileBufferRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Option<String>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let data = state
        .file_service
        .read_file_buffer(&req.path, req.workspace.as_deref().map(Path::new))
        .await?;
    // Binary data is base64-encoded for JSON transport.
    let encoded = data.map(|bytes| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    });
    Ok(Json(ApiResponse::ok(encoded)))
}

async fn write_file(
    State(state): State<FileRouterState>,
    body: Result<Json<WriteFileRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let workspace = req.workspace.unwrap_or_else(|| {
        std::path::Path::new(&req.path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let ok = state
        .file_service
        .write_file(&req.path, req.data.as_bytes(), &workspace)
        .await?;
    Ok(Json(ApiResponse::ok(ok)))
}

async fn copy_files(
    State(state): State<FileRouterState>,
    body: Result<Json<CopyFilesRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CopyFilesResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state
        .file_service
        .copy_files_to_workspace(&req.file_paths, &req.workspace, req.source_root.as_deref())
        .await?;
    Ok(Json(ApiResponse::ok(to_copy_response(result))))
}

async fn remove_entry(
    State(state): State<FileRouterState>,
    body: Result<Json<RemoveEntryRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let workspace = req.workspace.unwrap_or_else(|| {
        std::path::Path::new(&req.path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    state.file_service.remove_entry(&req.path, &workspace).await?;
    Ok(Json(ApiResponse::success()))
}

async fn rename_entry(
    State(state): State<FileRouterState>,
    body: Result<Json<RenameRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<RenameResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let new_path = state.file_service.rename_entry(&req.path, &req.new_name).await?;
    Ok(Json(ApiResponse::ok(RenameResponse { new_path })))
}

async fn create_temp_file(
    State(state): State<FileRouterState>,
    body: Result<Json<CreateTempFileRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let path = state.file_service.create_temp_file(&req.file_name).await?;
    Ok(Json(ApiResponse::ok(path)))
}

/// Fields extracted from a `/api/fs/upload` multipart request.
struct UploadMultipartFields {
    file_data: Vec<u8>,
    file_name: Option<String>,
    dispo_file_name: Option<String>,
    conversation_id: Option<String>,
}

/// Strip any directory component from a file name and reject empty results.
/// The returned name is guaranteed not to contain path separators; deeper
/// traversal validation happens in [`IFileService::create_upload_file`].
fn sanitize_upload_filename(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    let last = last.trim();
    if last.is_empty() { None } else { Some(last.to_owned()) }
}

async fn extract_upload_multipart(mut multipart: Multipart) -> Result<UploadMultipartFields, AppError> {
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut dispo_file_name: Option<String> = None;
    let mut conversation_id: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "file" => {
                // Capture the Content-Disposition filename (if any) before
                // consuming the field body — `field.file_name()` is only
                // available on the field metadata, not on the Bytes below.
                dispo_file_name = field.file_name().and_then(sanitize_upload_filename);
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("failed to read file: {e}")))?
                        .to_vec(),
                );
            }
            "file_name" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("failed to read file_name: {e}")))?;
                if let Some(name) = sanitize_upload_filename(&text) {
                    file_name = Some(name);
                }
            }
            "conversation_id" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("failed to read conversation_id: {e}")))?;
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    conversation_id = Some(trimmed.to_owned());
                }
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| AppError::BadRequest("missing 'file' field".to_owned()))?;

    Ok(UploadMultipartFields {
        file_data,
        file_name,
        dispo_file_name,
        conversation_id,
    })
}

async fn upload_file(
    State(state): State<FileRouterState>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let fields = extract_upload_multipart(multipart).await?;

    let file_name = fields.file_name.or(fields.dispo_file_name).ok_or_else(|| {
        AppError::BadRequest("missing file name: provide 'file_name' or a multipart filename".to_owned())
    })?;

    let path = state
        .file_service
        .create_upload_file(&file_name, &fields.file_data, fields.conversation_id.as_deref())
        .await?;
    Ok(Json(ApiResponse::ok(path)))
}

async fn get_image_base64(
    State(state): State<FileRouterState>,
    body: Result<Json<GetImageBase64Request>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let data_url = state
        .file_service
        .get_image_base64(&req.path, req.workspace.as_deref().map(Path::new))
        .await?;
    Ok(Json(ApiResponse::ok(data_url)))
}

async fn fetch_remote_image(
    State(state): State<FileRouterState>,
    body: Result<Json<FetchRemoteImageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let data_url = state.file_service.fetch_remote_image(&req.url).await;
    Ok(Json(ApiResponse::ok(data_url)))
}

async fn create_zip(
    State(state): State<FileRouterState>,
    body: Result<Json<ZipRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let entries: Vec<ZipEntry> = req.files.into_iter().map(to_zip_entry).collect();
    let ok = state
        .file_service
        .create_zip(&req.path, entries, req.request_id)
        .await?;
    Ok(Json(ApiResponse::ok(ok)))
}

async fn cancel_zip(
    State(state): State<FileRouterState>,
    body: Result<Json<CancelZipRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let ok = state.file_service.cancel_zip(&req.request_id).await;
    Ok(Json(ApiResponse::ok(ok)))
}

// ---------------------------------------------------------------------------
// D. File watch — handlers
// ---------------------------------------------------------------------------

async fn start_watch(
    State(state): State<FileRouterState>,
    body: Result<Json<FileWatchRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.watch_service.start_watch(&req.file_path).await?;
    Ok(Json(ApiResponse::success()))
}

async fn stop_watch(
    State(state): State<FileRouterState>,
    body: Result<Json<FileWatchRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.watch_service.stop_watch(&req.file_path).await?;
    Ok(Json(ApiResponse::success()))
}

async fn stop_all_watches(State(state): State<FileRouterState>) -> Result<Json<ApiResponse<()>>, AppError> {
    state.watch_service.stop_all_watches().await?;
    Ok(Json(ApiResponse::success()))
}

async fn start_office_watch(
    State(state): State<FileRouterState>,
    body: Result<Json<WorkspaceOfficeWatchRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let allowed_roots: Vec<&Path> = state.allowed_roots.iter().map(std::path::PathBuf::as_path).collect();
    crate::path_safety::validate_path_with_extra_root(&req.workspace, &allowed_roots, Some(Path::new(&req.workspace)))?;
    state.watch_service.start_office_watch(&req.workspace).await?;
    Ok(Json(ApiResponse::success()))
}

async fn stop_office_watch(
    State(state): State<FileRouterState>,
    body: Result<Json<WorkspaceOfficeWatchRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.watch_service.stop_office_watch(&req.workspace).await?;
    Ok(Json(ApiResponse::success()))
}

// ---------------------------------------------------------------------------
// E. Workspace snapshot — handlers
// ---------------------------------------------------------------------------

async fn snapshot_init(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SnapshotInfoResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let info = state.snapshot_service.init(&req.workspace).await?;
    Ok(Json(ApiResponse::ok(to_snapshot_info_response(info))))
}

async fn snapshot_info(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SnapshotInfoResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let info = state.snapshot_service.get_info(&req.workspace).await?;
    Ok(Json(ApiResponse::ok(to_snapshot_info_response(info))))
}

async fn snapshot_compare(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SnapshotCompareResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let result = state.snapshot_service.compare(&req.workspace).await?;
    Ok(Json(ApiResponse::ok(to_compare_response(result))))
}

async fn snapshot_baseline(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotBaselineRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Option<String>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let content = state
        .snapshot_service
        .get_baseline_content(&req.workspace, &req.file_path)
        .await?;
    Ok(Json(ApiResponse::ok(content)))
}

async fn snapshot_stage_file(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotStageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .snapshot_service
        .stage_file(&req.workspace, &req.file_path)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_stage_all(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.snapshot_service.stage_all(&req.workspace).await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_unstage_file(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotStageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .snapshot_service
        .unstage_file(&req.workspace, &req.file_path)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_unstage_all(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.snapshot_service.unstage_all(&req.workspace).await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_discard(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotDiscardRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .snapshot_service
        .discard_file(&req.workspace, &req.file_path, req.operation)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_reset(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotDiscardRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .snapshot_service
        .reset_file(&req.workspace, &req.file_path, req.operation)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn snapshot_branches(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let branches = state.snapshot_service.get_branches(&req.workspace).await?;
    Ok(Json(ApiResponse::ok(branches)))
}

async fn snapshot_dispose(
    State(state): State<FileRouterState>,
    body: Result<Json<SnapshotWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.snapshot_service.dispose(&req.workspace).await?;
    Ok(Json(ApiResponse::success()))
}

// ---------------------------------------------------------------------------
// Domain → DTO conversions
// ---------------------------------------------------------------------------

fn to_dir_or_file_response(d: DirOrFile) -> DirOrFileResponse {
    let children = if d.is_dir {
        Some(d.children.into_iter().map(to_dir_or_file_response).collect())
    } else {
        None
    };
    DirOrFileResponse {
        name: d.name,
        full_path: d.full_path,
        relative_path: d.relative_path,
        is_dir: d.is_dir,
        is_file: !d.is_dir,
        children,
    }
}

fn to_flat_file_response(f: WorkspaceFlatFile) -> WorkspaceFlatFileResponse {
    WorkspaceFlatFileResponse {
        name: f.name,
        full_path: f.full_path,
        relative_path: f.relative_path,
    }
}

fn to_metadata_response(m: FileMetadata) -> FileMetadataResponse {
    FileMetadataResponse {
        name: m.name,
        path: m.path,
        size: m.size,
        mime_type: m.mime_type,
        last_modified: m.last_modified,
        is_directory: if m.is_directory { Some(true) } else { None },
    }
}

fn to_copy_response(r: CopyResult) -> CopyFilesResponse {
    CopyFilesResponse {
        copied_files: r.copied_files,
        failed_files: r.failed_files,
    }
}

fn to_zip_entry(e: nomifun_api_types::ZipFileEntry) -> ZipEntry {
    if let Some(content) = e.content {
        ZipEntry::Text { name: e.name, content }
    } else if let Some(file_path) = e.file_path {
        ZipEntry::Disk {
            name: e.name,
            file_path,
        }
    } else {
        // Fallback: treat as empty text entry
        ZipEntry::Text {
            name: e.name,
            content: String::new(),
        }
    }
}

fn to_snapshot_info_response(info: SnapshotInfo) -> SnapshotInfoResponse {
    let (mode, reason) = match info.mode {
        SnapshotMode::GitRepo => (nomifun_api_types::SnapshotMode::GitRepo, None),
        SnapshotMode::Snapshot => (nomifun_api_types::SnapshotMode::Snapshot, None),
        SnapshotMode::Disabled { reason } => (nomifun_api_types::SnapshotMode::Disabled, Some(reason)),
    };
    SnapshotInfoResponse {
        mode,
        branch: info.branch,
        reason,
    }
}

fn to_file_change_response(c: FileChangeInfo) -> FileChangeInfoResponse {
    FileChangeInfoResponse {
        file_path: c.file_path,
        relative_path: c.relative_path,
        operation: c.operation,
    }
}

fn to_compare_response(r: CompareResult) -> SnapshotCompareResponse {
    SnapshotCompareResponse {
        staged: r.staged.into_iter().map(to_file_change_response).collect(),
        unstaged: r.unstaged.into_iter().map(to_file_change_response).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_or_file_response_conversion_file() {
        let d = DirOrFile {
            name: "test.txt".into(),
            full_path: "/ws/test.txt".into(),
            relative_path: "test.txt".into(),
            is_dir: false,
            children: vec![],
        };
        let r = to_dir_or_file_response(d);
        assert_eq!(r.name, "test.txt");
        assert!(!r.is_dir);
        assert!(r.is_file);
        assert!(r.children.is_none());
    }

    #[test]
    fn dir_or_file_response_conversion_dir_with_children() {
        let d = DirOrFile {
            name: "src".into(),
            full_path: "/ws/src".into(),
            relative_path: "src".into(),
            is_dir: true,
            children: vec![DirOrFile {
                name: "main.rs".into(),
                full_path: "/ws/src/main.rs".into(),
                relative_path: "src/main.rs".into(),
                is_dir: false,
                children: vec![],
            }],
        };
        let r = to_dir_or_file_response(d);
        assert!(r.is_dir);
        assert!(!r.is_file);
        let children = r.children.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "main.rs");
    }

    #[test]
    fn flat_file_response_conversion() {
        let f = WorkspaceFlatFile {
            name: "lib.rs".into(),
            full_path: "/ws/src/lib.rs".into(),
            relative_path: "src/lib.rs".into(),
        };
        let r = to_flat_file_response(f);
        assert_eq!(r.name, "lib.rs");
        assert_eq!(r.full_path, "/ws/src/lib.rs");
        assert_eq!(r.relative_path, "src/lib.rs");
    }

    #[test]
    fn metadata_response_conversion_file() {
        let m = FileMetadata {
            name: "readme.md".into(),
            path: "/ws/readme.md".into(),
            size: 1024,
            mime_type: "text/markdown".into(),
            last_modified: 1700000000000,
            is_directory: false,
        };
        let r = to_metadata_response(m);
        assert_eq!(r.name, "readme.md");
        assert_eq!(r.size, 1024);
        assert!(r.is_directory.is_none());
    }

    #[test]
    fn metadata_response_conversion_directory() {
        let m = FileMetadata {
            name: "src".into(),
            path: "/ws/src".into(),
            size: 0,
            mime_type: "".into(),
            last_modified: 1700000000000,
            is_directory: true,
        };
        let r = to_metadata_response(m);
        assert_eq!(r.is_directory, Some(true));
    }

    #[test]
    fn zip_entry_conversion_text() {
        let e = nomifun_api_types::ZipFileEntry {
            name: "a.txt".into(),
            content: Some("hello".into()),
            file_path: None,
        };
        let z = to_zip_entry(e);
        match z {
            ZipEntry::Text { name, content } => {
                assert_eq!(name, "a.txt");
                assert_eq!(content, "hello");
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn zip_entry_conversion_disk() {
        let e = nomifun_api_types::ZipFileEntry {
            name: "b.bin".into(),
            content: None,
            file_path: Some("/src/b.bin".into()),
        };
        let z = to_zip_entry(e);
        match z {
            ZipEntry::Disk { name, file_path } => {
                assert_eq!(name, "b.bin");
                assert_eq!(file_path, "/src/b.bin");
            }
            _ => panic!("expected Disk variant"),
        }
    }

    #[test]
    fn zip_entry_conversion_empty_fallback() {
        let e = nomifun_api_types::ZipFileEntry {
            name: "empty.txt".into(),
            content: None,
            file_path: None,
        };
        let z = to_zip_entry(e);
        match z {
            ZipEntry::Text { name, content } => {
                assert_eq!(name, "empty.txt");
                assert!(content.is_empty());
            }
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn snapshot_info_response_git_repo() {
        let info = SnapshotInfo {
            mode: SnapshotMode::GitRepo,
            branch: Some("main".into()),
        };
        let r = to_snapshot_info_response(info);
        assert_eq!(r.mode, nomifun_api_types::SnapshotMode::GitRepo);
        assert_eq!(r.branch, Some("main".into()));
    }

    #[test]
    fn snapshot_info_response_snapshot_mode() {
        let info = SnapshotInfo {
            mode: SnapshotMode::Snapshot,
            branch: None,
        };
        let r = to_snapshot_info_response(info);
        assert_eq!(r.mode, nomifun_api_types::SnapshotMode::Snapshot);
        assert!(r.branch.is_none());
    }

    #[test]
    fn snapshot_info_response_disabled_mode_carries_reason() {
        let info = SnapshotInfo {
            mode: SnapshotMode::Disabled {
                reason: "drive root".into(),
            },
            branch: None,
        };
        let r = to_snapshot_info_response(info);
        assert_eq!(r.mode, nomifun_api_types::SnapshotMode::Disabled);
        assert!(r.branch.is_none());
        assert_eq!(r.reason.as_deref(), Some("drive root"));
    }

    #[test]
    fn compare_response_conversion() {
        use nomifun_common::FileChangeOperation;
        let result = CompareResult {
            staged: vec![FileChangeInfo {
                file_path: "/ws/a.txt".into(),
                relative_path: "a.txt".into(),
                operation: FileChangeOperation::Create,
            }],
            unstaged: vec![FileChangeInfo {
                file_path: "/ws/b.txt".into(),
                relative_path: "b.txt".into(),
                operation: FileChangeOperation::Modify,
            }],
        };
        let r = to_compare_response(result);
        assert_eq!(r.staged.len(), 1);
        assert_eq!(r.staged[0].file_path, "/ws/a.txt");
        assert_eq!(r.staged[0].operation, FileChangeOperation::Create);
        assert_eq!(r.unstaged.len(), 1);
        assert_eq!(r.unstaged[0].operation, FileChangeOperation::Modify);
    }

    // ---- sanitize_upload_filename -----------------------------------------

    #[test]
    fn sanitize_upload_filename_strips_directory_components() {
        assert_eq!(sanitize_upload_filename("a/b/c.png").as_deref(), Some("c.png"));
        assert_eq!(sanitize_upload_filename("C:\\tmp\\d.jpg").as_deref(), Some("d.jpg"));
        assert_eq!(
            sanitize_upload_filename("  spaced.txt  ").as_deref(),
            Some("spaced.txt")
        );
    }

    #[test]
    fn sanitize_upload_filename_rejects_empty() {
        assert_eq!(sanitize_upload_filename(""), None);
        assert_eq!(sanitize_upload_filename("   "), None);
        assert_eq!(sanitize_upload_filename("/"), None);
        assert_eq!(sanitize_upload_filename("a/b/"), None);
    }

    #[test]
    fn sanitize_upload_filename_plain_passthrough() {
        assert_eq!(sanitize_upload_filename("image.png").as_deref(), Some("image.png"));
    }
}
