//! Authenticated `/api/creative-studio/*` project routes plus the legacy
//! `/api/workshop/*` asset/canvas handlers (contract §3.1/§3.2). Their
//! management surfaces (list/create/patch/delete, doc read/write, upload,
//! agent-ops) are owner-only — mounted behind the app's authenticated router
//! (same auth extractor as the knowledge routes). The multipart upload route
//! raises the body limit to [`MAX_ASSET_BYTES`]; every other route rides the
//! app's default limit.
//!
//! The two **read-only binary serve** routes ([`serve_file`] +
//! [`serve_canvas_thumb`]) instead live on the auth-EXEMPT public router
//! ([`workshop_public_routes`]): `<img>` / `<video>` / `new Image()` loads carry
//! no custom-header API, so under the desktop's `TrustLocalToken` policy they
//! cannot present the `x-nomi-local-trust` header — the authenticated router
//! would 403 every asset thumbnail and canvas gallery image. They are GET-only,
//! serve opaque unguessable UUIDv7 ids (a capability URL, not
//! an enumeration surface), keep the service's traversal sandbox, and never
//! extract `CurrentUser` (see the note on the public router).

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Extension, Json, Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::Value;
use tower_http::limit::RequestBodyLimitLayer;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, WorkshopAssetId, WorkshopCanvasId};

use crate::MAX_ASSET_BYTES;
use crate::agent_ops::PendingOp;
use crate::archive::MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES;
use crate::creative_studio::{CreativeProjectDocument, CreativeProjectSummary};
use crate::dto::{WorkshopAsset, WorkshopCanvasMeta};
use crate::service::{AssetPatch, AssetQuery, NewAssetUpload, NewTextAsset};
use crate::state::WorkshopRouterState;

pub fn workshop_routes(state: WorkshopRouterState) -> Router {
    // The asset upload route carries its own (larger) body limit. Disable the
    // app's global `DefaultBodyLimit` on it, then cap at MAX_ASSET_BYTES.
    let upload_router = Router::new()
        .route("/api/workshop/assets/upload", post(upload_asset))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(MAX_ASSET_BYTES))
        .with_state(state.clone());

    let archive_import_router = Router::new()
        .route(
            "/api/creative-studio/projects/import",
            post(import_creative_project_archive),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES,
        ))
        .with_state(state.clone());

    Router::new()
        .route(
            "/api/creative-studio/projects",
            get(list_creative_projects).post(create_creative_project),
        )
        .route(
            "/api/creative-studio/projects/{project_id}",
            get(get_creative_project)
                .patch(rename_creative_project)
                .delete(delete_creative_project),
        )
        .route(
            "/api/creative-studio/projects/{project_id}/document",
            axum::routing::put(save_creative_project),
        )
        .route(
            "/api/creative-studio/projects/{project_id}/archive",
            get(export_creative_project_archive),
        )
        .route("/api/workshop/canvases", get(list_canvases).post(create_canvas))
        .route(
            "/api/workshop/canvases/{canvas_id}",
            get(get_canvas).patch(patch_canvas).delete(delete_canvas),
        )
        .route("/api/workshop/canvases/{canvas_id}/doc", axum::routing::put(put_doc))
        .route(
            "/api/workshop/canvases/{canvas_id}/pending-ops",
            get(get_pending_ops),
        )
        .route(
            "/api/workshop/canvases/{canvas_id}/pending-ops/ack",
            post(ack_pending_ops),
        )
        .route("/api/workshop/assets", get(list_assets).post(create_text_asset))
        .route(
            "/api/workshop/assets/{asset_id}",
            axum::routing::patch(patch_asset).delete(delete_asset),
        )
        .route("/api/workshop/collections/rename", post(rename_collection))
        .with_state(state)
        .merge(archive_import_router)
        .merge(upload_router)
}

/// Auth-EXEMPT read-only binary serve routes (see the module doc). GET-only, two
/// prefixes only, opaque unguessable ids. Merged into the app's public router
/// next to the other auth-exempt serve routes (logos / office proxy / companion
/// figure images). Every write / list / delete route stays under auth in
/// [`workshop_routes`].
///
/// These handlers MUST NOT extract `Extension<CurrentUser>`: `<img>`/`<video>`
/// loads carry no trust header, so `trust_resolve_middleware` injects no
/// `CurrentUser` and that extractor would 500 the very requests this router
/// exists to serve.
pub fn workshop_public_routes(state: WorkshopRouterState) -> Router {
    Router::new()
        .route("/api/workshop/files/{asset_id}", get(serve_file))
        .route("/api/workshop/canvas-thumbs/{canvas_id}", get(serve_canvas_thumb))
        .with_state(state)
}

/// `Cache-Control` for served binaries: privately cacheable for an hour. Ids are
/// content-immutable capability URLs, but `private` keeps shared proxies from
/// caching a user's media.
const SERVE_CACHE_CONTROL: &str = "private, max-age=3600";

// ── canonical Creative Studio projects ─────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeProjectListResponse {
    projects: Vec<CreativeProjectSummary>,
}

async fn list_creative_projects(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CreativeProjectListResponse>>, AppError> {
    let projects = state.service.list_creative_projects().await?;
    Ok(Json(ApiResponse::ok(CreativeProjectListResponse {
        projects,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeProjectRequest {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeProjectResponse {
    project: CreativeProjectSummary,
}

async fn create_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeProjectRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let project = state.service.create_creative_project(req.title).await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeProjectResponse { project })),
    ))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeProjectDetailResponse {
    project: CreativeProjectSummary,
    document: CreativeProjectDocument,
}

async fn get_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeProjectDetailResponse>>, AppError> {
    let detail = state.service.get_creative_project(&project_id).await?;
    Ok(Json(ApiResponse::ok(CreativeProjectDetailResponse {
        project: detail.project,
        document: detail.document,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameCreativeProjectRequest {
    title: String,
}

async fn rename_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
    body: Result<Json<RenameCreativeProjectRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeProjectResponse>>, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let project = state
        .service
        .rename_creative_project(&project_id, &req.title)
        .await?;
    Ok(Json(ApiResponse::ok(CreativeProjectResponse { project })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeProjectRequest {
    expected_revision: String,
    document: CreativeProjectDocument,
}

async fn save_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
    body: Result<Json<SaveCreativeProjectRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeProjectResponse>>, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let project = state
        .service
        .save_creative_project(&project_id, &req.expected_revision, &req.document)
        .await?;
    Ok(Json(ApiResponse::ok(CreativeProjectResponse { project })))
}

async fn delete_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_creative_project(&project_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn export_creative_project_archive(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Response, AppError> {
    let archive = state
        .service
        .export_creative_project_archive(&project_id)
        .await?;
    Ok((
        [
            (header::CONTENT_TYPE, archive.mime.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", archive.file_name),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        Body::from(archive.bytes),
    )
        .into_response())
}

async fn import_creative_project_archive(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    bytes: Bytes,
) -> Result<impl IntoResponse, AppError> {
    let project = state
        .service
        .import_creative_project_archive(bytes.to_vec())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeProjectResponse { project })),
    ))
}

// ── canvases ────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct CanvasListResponse {
    canvases: Vec<WorkshopCanvasMeta>,
}

async fn list_canvases(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CanvasListResponse>>, AppError> {
    let canvases = state.service.list_canvases().await?;
    Ok(Json(ApiResponse::ok(CanvasListResponse { canvases })))
}

#[derive(Deserialize)]
struct CreateCanvasRequest {
    #[serde(default)]
    title: Option<String>,
}

async fn create_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCanvasRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    // Body is optional — an empty POST creates a default-titled canvas.
    let title = body.ok().and_then(|Json(req)| req.title);
    let meta = state.service.create_canvas(title).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(meta))))
}

#[derive(serde::Serialize)]
struct CanvasDetailResponse {
    meta: WorkshopCanvasMeta,
    doc: Value,
}

async fn get_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
) -> Result<Json<ApiResponse<CanvasDetailResponse>>, AppError> {
    let c = state.service.get_canvas(canvas_id.as_str()).await?;
    // This REST route is the editor's canvas-doc load path (CanvasPage). Mark
    // the canvas "open" now so an agent's concurrent apply_ops in the gap before
    // the first pending-ops poll is queued for this editor rather than written
    // straight to canvas.json and then clobbered by the editor's first autosave.
    // The gateway agent reads via `service.get_canvas` directly and never hits
    // this handler, so it is not falsely marked open.
    state.service.mark_canvas_open(canvas_id.as_str());
    Ok(Json(ApiResponse::ok(CanvasDetailResponse { meta: c.meta, doc: c.doc })))
}

#[derive(Deserialize)]
struct PutDocRequest {
    doc: Value,
}

#[derive(serde::Serialize)]
struct PutDocResponse {
    updated_at: i64,
}

async fn put_doc(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
    body: Result<Json<PutDocRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<PutDocResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let updated_at = state.service.save_doc(canvas_id.as_str(), &req.doc).await?;
    Ok(Json(ApiResponse::ok(PutDocResponse { updated_at })))
}

// ── 画布助手 (agent-op) pending queue ─────────────────────────────────────────

#[derive(serde::Serialize)]
struct PendingOpsResponse {
    ops: Vec<PendingOp>,
}

/// Drain the pending agent ops for an open canvas (idempotent — ops stay until
/// acked). Polling this also registers the canvas as "open" so the agent's writes
/// route to this frontend rather than the backend direct applier.
async fn get_pending_ops(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
) -> Result<Json<ApiResponse<PendingOpsResponse>>, AppError> {
    let ops = state.service.take_pending_ops(canvas_id.as_str()).await?;
    Ok(Json(ApiResponse::ok(PendingOpsResponse { ops })))
}

#[derive(Deserialize)]
struct AckOpsRequest {
    #[serde(default)]
    op_ids: Vec<String>,
}

#[derive(serde::Serialize)]
struct AckOpsResponse {
    acked: usize,
}

/// Acknowledge (remove) agent ops the frontend has applied.
async fn ack_pending_ops(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
    body: Result<Json<AckOpsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<AckOpsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.ack_agent_ops(canvas_id.as_str(), &req.op_ids);
    Ok(Json(ApiResponse::ok(AckOpsResponse { acked: req.op_ids.len() })))
}

#[derive(Deserialize)]
struct PatchCanvasRequest {
    #[serde(default)]
    title: Option<String>,
    /// Set the canvas gallery thumbnail from this asset (append-only over the
    /// original `{ title }` contract).
    #[serde(default)]
    thumbnail_asset_id: Option<String>,
}

async fn patch_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
    body: Result<Json<PatchCanvasRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<WorkshopCanvasMeta>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let meta = state
        .service
        .patch_canvas(canvas_id.as_str(), req.title, req.thumbnail_asset_id)
        .await?;
    Ok(Json(ApiResponse::ok(meta)))
}

async fn delete_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<WorkshopCanvasId>,
) -> Result<StatusCode, AppError> {
    state.service.delete_canvas(canvas_id.as_str()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// AUTH-EXEMPT (mounted on [`workshop_public_routes`]): no `CurrentUser`
/// extractor. Serves a canvas gallery thumbnail (JPEG).
async fn serve_canvas_thumb(
    State(state): State<WorkshopRouterState>,
    Path(canvas_id): Path<WorkshopCanvasId>,
) -> Result<Response, AppError> {
    let served = state
        .service
        .serve_canvas_thumbnail(canvas_id.as_str())
        .await?;
    Ok((
        [
            (header::CONTENT_TYPE, served.mime),
            (header::CACHE_CONTROL, SERVE_CACHE_CONTROL.to_string()),
        ],
        Body::from(served.bytes),
    )
        .into_response())
}

// ── assets ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ListAssetsQuery {
    kind: Option<String>,
    collection: Option<String>,
    q: Option<String>,
    in_library: Option<String>,
    /// Append-only (M10a): `ungrouped=1` returns only assets with no collection
    /// (`collection IS NULL OR ''`). Mutually exclusive with `collection` — when
    /// set, `collection` is ignored so the two never fight.
    #[serde(default)]
    ungrouped: Option<String>,
    /// Append-only (asset-library page): exact-match filter on one tag.
    #[serde(default)]
    tag: Option<String>,
    /// Append-only (asset-library page): result ordering token
    /// (`created_desc`|`created_asc`|`updated_desc`|`name_asc`|`size_desc`).
    /// Unknown/absent → newest-created first.
    #[serde(default)]
    sort: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[derive(serde::Serialize)]
struct AssetListResponse {
    items: Vec<WorkshopAsset>,
    total: i64,
}

fn parse_bool_flag(v: &str) -> bool {
    matches!(v.trim(), "1" | "true" | "True" | "TRUE" | "yes")
}

/// Map a `sort` query token to an [`AssetSort`]. Unknown/empty → the default
/// (newest-created first).
fn parse_asset_sort(v: &str) -> nomifun_db::AssetSort {
    use nomifun_db::AssetSort;
    match v.trim() {
        "created_asc" => AssetSort::CreatedAsc,
        "updated_desc" => AssetSort::UpdatedDesc,
        "name_asc" => AssetSort::TitleAsc,
        "size_desc" => AssetSort::SizeDesc,
        _ => AssetSort::CreatedDesc,
    }
}

async fn list_assets(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListAssetsQuery>,
) -> Result<Json<ApiResponse<AssetListResponse>>, AppError> {
    let ungrouped = query.ungrouped.as_deref().map(parse_bool_flag).unwrap_or(false);
    let page = state
        .service
        .list_assets(AssetQuery {
            kind: query.kind.filter(|s| !s.trim().is_empty()),
            // `ungrouped` wins over `collection` (contract: mutually exclusive).
            collection: if ungrouped {
                None
            } else {
                query.collection.filter(|s| !s.trim().is_empty())
            },
            q: query.q,
            in_library: query.in_library.as_deref().map(parse_bool_flag),
            ungrouped,
            tag: query.tag.filter(|s| !s.trim().is_empty()),
            sort: query.sort.as_deref().map(parse_asset_sort).unwrap_or_default(),
            page: query.page.unwrap_or(1),
            page_size: query.page_size.unwrap_or(30),
        })
        .await?;
    Ok(Json(ApiResponse::ok(AssetListResponse { items: page.items, total: page.total })))
}

/// Fields extracted from a `/api/workshop/assets/upload` multipart request.
struct UploadFields {
    bytes: Vec<u8>,
    file_name: Option<String>,
    content_type: Option<String>,
    title: Option<String>,
    collection: Option<String>,
    tags: Option<Vec<String>>,
    in_library: Option<bool>,
}

/// Parse a `tags` form value: a JSON array string, else comma-separated.
fn parse_tags_field(raw: &str) -> Vec<String> {
    if let Ok(v) = serde_json::from_str::<Vec<String>>(raw) {
        return v.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    }
    raw.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

async fn extract_upload(mut multipart: Multipart) -> Result<UploadFields, AppError> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut title: Option<String> = None;
    let mut collection: Option<String> = None;
    let mut tags: Option<Vec<String>> = None;
    let mut in_library: Option<bool> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        match field.name().unwrap_or("") {
            "file" => {
                file_name = field.file_name().map(str::to_string).filter(|s| !s.trim().is_empty());
                content_type = field.content_type().map(str::to_string);
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("failed to read file: {e}")))?
                        .to_vec(),
                );
            }
            "title" => title = read_text(field).await?.filter(|s| !s.trim().is_empty()),
            "collection" => collection = read_text(field).await?.filter(|s| !s.trim().is_empty()),
            "tags" => tags = read_text(field).await?.map(|t| parse_tags_field(&t)),
            "in_library" => in_library = read_text(field).await?.map(|t| parse_bool_flag(&t)),
            _ => {}
        }
    }

    let bytes = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
    Ok(UploadFields { bytes, file_name, content_type, title, collection, tags, in_library })
}

async fn read_text(field: axum::extract::multipart::Field<'_>) -> Result<Option<String>, AppError> {
    field
        .text()
        .await
        .map(Some)
        .map_err(|e| AppError::BadRequest(format!("failed to read field: {e}")))
}

async fn upload_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let fields = extract_upload(multipart).await?;
    let file_name = fields
        .file_name
        .unwrap_or_else(|| "upload".to_string());
    let asset = state
        .service
        .upload_asset(NewAssetUpload {
            file_name,
            content_type: fields.content_type,
            bytes: fields.bytes,
            title: fields.title,
            collection: fields.collection,
            tags: fields.tags,
            in_library: fields.in_library,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(asset))))
}

#[derive(Deserialize)]
struct CreateTextAssetRequest {
    kind: String,
    title: String,
    #[serde(default)]
    text_content: String,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    in_library: Option<bool>,
}

async fn create_text_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateTextAssetRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if req.kind != "text" {
        return Err(AppError::BadRequest(
            "this endpoint only registers text assets; upload binaries via /api/workshop/assets/upload".into(),
        ));
    }
    let asset = state
        .service
        .create_text_asset(NewTextAsset {
            title: req.title,
            text_content: req.text_content,
            collection: req.collection,
            tags: req.tags,
            in_library: req.in_library,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(asset))))
}

#[derive(Deserialize)]
struct PatchAssetRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    in_library: Option<bool>,
}

async fn patch_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(asset_id): Path<WorkshopAssetId>,
    body: Result<Json<PatchAssetRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<WorkshopAsset>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let patched = state
        .service
        .patch_asset(
            asset_id.as_str(),
            AssetPatch {
                title: req.title,
                collection: req.collection,
                tags: req.tags,
                in_library: req.in_library,
            },
        )
        .await?;
    Ok(Json(ApiResponse::ok(patched)))
}

async fn delete_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(asset_id): Path<WorkshopAssetId>,
) -> Result<StatusCode, AppError> {
    state.service.delete_asset(asset_id.as_str()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RenameCollectionRequest {
    from: String,
    /// The new collection name; a blank value ungroups the affected assets.
    #[serde(default)]
    to: String,
}

#[derive(serde::Serialize)]
struct RenameCollectionResponse {
    updated: u64,
}

/// Bulk-rename a collection across every asset that used it (management
/// surface). Returns the number of rows updated.
async fn rename_collection(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<RenameCollectionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<RenameCollectionResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let updated = state.service.rename_collection(&req.from, &req.to).await?;
    Ok(Json(ApiResponse::ok(RenameCollectionResponse { updated })))
}

#[derive(Deserialize)]
struct FileQuery {
    #[serde(default)]
    thumb: Option<String>,
}

/// AUTH-EXEMPT (mounted on [`workshop_public_routes`]): no `CurrentUser`
/// extractor. Serves an asset's original binary (or, with `?thumb=1`, its
/// thumbnail). Traversal-safe via the service; missing → 404.
async fn serve_file(
    State(state): State<WorkshopRouterState>,
    Path(asset_id): Path<WorkshopAssetId>,
    Query(query): Query<FileQuery>,
) -> Result<Response, AppError> {
    let thumb = query.thumb.as_deref().map(parse_bool_flag).unwrap_or(false);
    let served = state.service.serve_file(asset_id.as_str(), thumb).await?;
    Ok((
        [
            (header::CONTENT_TYPE, served.mime),
            (header::CACHE_CONTROL, SERVE_CACHE_CONTROL.to_string()),
        ],
        Body::from(served.bytes),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_common::UserId;
    use nomifun_db::{IWorkshopRepository, SqliteWorkshopRepository};

    use super::*;
    use crate::WorkshopService;

    async fn test_state() -> (WorkshopRouterState, CurrentUser, tempfile::TempDir) {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn IWorkshopRepository> =
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone()));
        let data_dir = tempfile::tempdir().unwrap();
        let service = WorkshopService::start(data_dir.path(), repo);
        let user = CurrentUser {
            id: UserId::new(),
            username: "owner".into(),
        };
        (WorkshopRouterState::new(service), user, data_dir)
    }

    #[tokio::test]
    async fn creative_project_handlers_cover_crud_and_revision_conflict() {
        let (state, user, _data_dir) = test_state().await;

        let created = create_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Ok::<_, JsonRejection>(Json(CreateCreativeProjectRequest {
                title: Some("路由项目".into()),
            })),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);
        let project = state
            .service
            .list_creative_projects()
            .await
            .unwrap()
            .remove(0);
        assert_eq!(project.revision, "1");

        let detail = get_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Path(project.project_id.clone()),
        )
        .await
        .unwrap();
        let document = detail.0.data.unwrap().document;
        assert_eq!(document.schema, "nomifun.creative-studio/v1");

        let saved = save_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Path(project.project_id.clone()),
            Ok::<_, JsonRejection>(Json(SaveCreativeProjectRequest {
                expected_revision: "1".into(),
                document: document.clone(),
            })),
        )
        .await
        .unwrap();
        assert_eq!(saved.0.data.unwrap().project.revision, "2");

        let stale = save_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Path(project.project_id.clone()),
            Ok::<_, JsonRejection>(Json(SaveCreativeProjectRequest {
                expected_revision: "1".into(),
                document,
            })),
        )
        .await
        .unwrap_err();
        assert!(matches!(stale, AppError::Conflict(_)));
        assert_eq!(stale.into_response().status(), StatusCode::CONFLICT);

        let missing_id = "0190f5fe-7c00-7a00-8abc-000000000199".to_owned();
        let missing = get_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Path(missing_id),
        )
        .await
        .unwrap_err();
        assert!(matches!(missing, AppError::NotFound(_)));
        assert_eq!(missing.into_response().status(), StatusCode::NOT_FOUND);

        let deleted = delete_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Path(project.project_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(deleted, StatusCode::NO_CONTENT);

        let gone = get_creative_project(
            State(state),
            Extension(user),
            Path(project.project_id),
        )
            .await
            .unwrap_err();
        assert!(matches!(gone, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn creative_project_archive_handlers_return_real_zip_and_imported_project() {
        let (state, user, _data_dir) = test_state().await;
        let project = state
            .service
            .create_creative_project(Some("路由归档".into()))
            .await
            .unwrap();

        let exported = export_creative_project_archive(
            State(state.clone()),
            Extension(user.clone()),
            Path(project.project_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(exported.status(), StatusCode::OK);
        assert_eq!(
            exported.headers().get(header::CONTENT_TYPE).unwrap(),
            crate::archive::CREATIVE_STUDIO_ARCHIVE_MIME
        );
        assert!(
            exported
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".nomifun-canvas.zip\"")
        );
        let archive_bytes = axum::body::to_bytes(
            exported.into_body(),
            MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES,
        )
        .await
        .unwrap();

        let imported = import_creative_project_archive(
            State(state.clone()),
            Extension(user),
            archive_bytes,
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(imported.status(), StatusCode::CREATED);
        let projects = state.service.list_creative_projects().await.unwrap();
        assert_eq!(projects.len(), 2);
        assert_ne!(projects[0].project_id, projects[1].project_id);
    }
}
