//! Authenticated `/api/creative-studio/*` project, asset, workflow, and archive
//! routes. Their management surfaces are owner-only — mounted behind the app's
//! authenticated router (same auth extractor as the knowledge routes). The
//! multipart upload route raises the body limit to [`MAX_ASSET_BYTES`]; every
//! other route rides the app's default limit.
//!
//! The **read-only binary serve** route ([`serve_file`]) instead lives on the
//! auth-EXEMPT public router ([`workshop_public_routes`]): `<img>` / `<video>` /
//! `new Image()` loads carry
//! no custom-header API, so under the desktop's `TrustLocalToken` policy they
//! cannot present the `x-nomi-local-trust` header — the authenticated router
//! would 403 every asset thumbnail. It is GET-only, serves an opaque
//! unguessable UUIDv7 id (a capability URL, not
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
use tower_http::limit::RequestBodyLimitLayer;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, WorkshopAssetId};

use crate::MAX_ASSET_BYTES;
use crate::archive::MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES;
use crate::creative_studio::{CreativeProjectDocument, CreativeProjectSummary};
use crate::dto::WorkshopAsset;
use crate::prompt_catalog::CreativePromptCatalogPage;
use crate::service::{
    AssetPatch, AssetQuery, NewAssetUpload, NewTextAsset, PromptCatalogAssetOrigin,
};
use crate::state::WorkshopRouterState;
use crate::workflow::{CreativeWorkflowDefinitionV1, MAX_WORKFLOW_DEFINITION_BYTES};
use crate::workflow_run::{
    CreativeWorkflowRunAggregateV1, CreativeWorkflowRunCreateRequest,
    MAX_WORKFLOW_RUN_AGGREGATE_BYTES,
};

pub fn workshop_routes(state: WorkshopRouterState) -> Router {
    // The asset upload route carries its own (larger) body limit. Disable the
    // app's global `DefaultBodyLimit` on it, then cap at MAX_ASSET_BYTES.
    let upload_router = Router::new()
        .route("/api/creative-studio/assets/upload", post(upload_asset))
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

    let workflow_write_router = Router::new()
        .route(
            "/api/creative-studio/workflows",
            post(create_creative_workflow),
        )
        .route(
            "/api/creative-studio/workflows/{workflow_id}",
            axum::routing::put(save_creative_workflow),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_WORKFLOW_DEFINITION_BYTES + 64 * 1024,
        ))
        .with_state(state.clone());

    let workflow_run_write_router = Router::new()
        .route(
            "/api/creative-studio/workflow-runs",
            post(create_creative_workflow_run),
        )
        .route(
            "/api/creative-studio/workflow-runs/{workflow_run_id}",
            axum::routing::put(save_creative_workflow_run),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_WORKFLOW_RUN_AGGREGATE_BYTES + 64 * 1024,
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
        .route(
            "/api/creative-studio/prompts",
            get(list_prompt_catalog),
        )
        .route(
            "/api/creative-studio/prompts/sync",
            post(sync_prompt_catalog),
        )
        .route(
            "/api/creative-studio/workflows",
            get(list_creative_workflows),
        )
        .route(
            "/api/creative-studio/workflows/{workflow_id}",
            get(get_creative_workflow).delete(delete_creative_workflow),
        )
        .route(
            "/api/creative-studio/workflow-runs",
            get(list_creative_workflow_runs),
        )
        .route(
            "/api/creative-studio/workflow-runs/{workflow_run_id}",
            get(get_creative_workflow_run),
        )
        .route(
            "/api/creative-studio/assets",
            get(list_assets).post(create_text_asset),
        )
        .route(
            "/api/creative-studio/assets/{asset_id}",
            get(get_asset).patch(patch_asset).delete(delete_asset),
        )
        .route(
            "/api/creative-studio/collections/rename",
            post(rename_collection),
        )
        .with_state(state)
        .merge(workflow_write_router)
        .merge(workflow_run_write_router)
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
        .route("/api/creative-studio/files/{asset_id}", get(serve_file))
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

// ── attributed prompt catalog ─────────────────────────────────────────────

async fn list_prompt_catalog(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CreativePromptCatalogPage>>, AppError> {
    let catalog = state.service.list_prompt_catalog().await?;
    Ok(Json(ApiResponse::ok(catalog)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyncPromptCatalogRequest {
    #[serde(default)]
    force: bool,
}

async fn sync_prompt_catalog(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<SyncPromptCatalogRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativePromptCatalogPage>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let catalog = state.service.sync_prompt_catalog(request.force).await?;
    Ok(Json(ApiResponse::ok(catalog)))
}

// ── canonical Creative Studio workflows ────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeWorkflowListResponse {
    workflows: Vec<CreativeWorkflowDefinitionV1>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeWorkflowResponse {
    workflow: CreativeWorkflowDefinitionV1,
}

async fn list_creative_workflows(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CreativeWorkflowListResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeWorkflowListResponse {
        workflows: state.service.list_creative_workflows().await?,
    })))
}

async fn get_creative_workflow(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(workflow_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeWorkflowResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeWorkflowResponse {
        workflow: state.service.get_creative_workflow(&workflow_id).await?,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeWorkflowRequest {
    workflow: CreativeWorkflowDefinitionV1,
}

async fn create_creative_workflow(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeWorkflowRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let workflow = state
        .service
        .create_creative_workflow(request.workflow)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeWorkflowResponse { workflow })),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeWorkflowRequest {
    expected_revision: String,
    workflow: CreativeWorkflowDefinitionV1,
}

async fn save_creative_workflow(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(workflow_id): Path<String>,
    body: Result<Json<SaveCreativeWorkflowRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeWorkflowResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let workflow = state
        .service
        .save_creative_workflow(
            &workflow_id,
            &request.expected_revision,
            request.workflow,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeWorkflowResponse {
        workflow,
    })))
}

async fn delete_creative_workflow(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(workflow_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_creative_workflow(&workflow_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── durable Creative Studio workflow runs ─────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeWorkflowRunQuery {
    workflow_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeWorkflowRunListResponse {
    runs: Vec<CreativeWorkflowRunAggregateV1>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeWorkflowRunResponse {
    run: CreativeWorkflowRunAggregateV1,
}

async fn list_creative_workflow_runs(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<CreativeWorkflowRunQuery>,
) -> Result<Json<ApiResponse<CreativeWorkflowRunListResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeWorkflowRunListResponse {
        runs: state
            .service
            .list_creative_workflow_runs(query.workflow_id.as_deref())
            .await?,
    })))
}

async fn get_creative_workflow_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(workflow_run_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeWorkflowRunResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeWorkflowRunResponse {
        run: state
            .service
            .get_creative_workflow_run(&workflow_run_id)
            .await?,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeWorkflowRunRequest {
    request: CreativeWorkflowRunCreateRequest,
}

async fn create_creative_workflow_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeWorkflowRunRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let run = state
        .service
        .create_creative_workflow_run(request.request)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeWorkflowRunResponse { run })),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeWorkflowRunRequest {
    expected_revision: String,
    run: CreativeWorkflowRunAggregateV1,
}

async fn save_creative_workflow_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(workflow_run_id): Path<String>,
    body: Result<Json<SaveCreativeWorkflowRunRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeWorkflowRunResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let run = state
        .service
        .save_creative_workflow_run(
            &workflow_run_id,
            &request.expected_revision,
            request.run,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeWorkflowRunResponse { run })))
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

async fn get_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(asset_id): Path<WorkshopAssetId>,
) -> Result<Json<ApiResponse<WorkshopAsset>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.service.get_asset(asset_id.as_str()).await?,
    )))
}

/// Fields extracted from a `/api/creative-studio/assets/upload` multipart request.
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
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    origin: Option<PromptCatalogAssetOrigin>,
}

async fn create_text_asset(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateTextAssetRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if req.kind != "text" {
        return Err(AppError::BadRequest(
            "this endpoint only registers text assets; upload binaries via /api/creative-studio/assets/upload".into(),
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
            origin: req.origin,
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
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::WorkshopService;
    use crate::workflow::{
        CreativeWorkflowMetadata, CreativeWorkflowOutputPlan, CreativeWorkflowPromptSource,
        CreativeWorkflowStep, CreativeWorkflowTemplate, CreativeWorkflowTemplateSegment,
        CreativeWorkflowVariable, CreativeWorkflowVisibility,
    };

    async fn test_state() -> (WorkshopRouterState, CurrentUser, tempfile::TempDir) {
        let (state, user, data_dir, _database) = test_state_with_database().await;
        (state, user, data_dir)
    }

    async fn test_state_with_database() -> (
        WorkshopRouterState,
        CurrentUser,
        tempfile::TempDir,
        Arc<nomifun_db::Database>,
    ) {
        let database = Arc::new(nomifun_db::init_database_memory().await.unwrap());
        let repo: Arc<dyn IWorkshopRepository> =
            Arc::new(SqliteWorkshopRepository::new(database.pool().clone()));
        let data_dir = tempfile::tempdir().unwrap();
        let service = WorkshopService::start(data_dir.path(), repo);
        let user = CurrentUser {
            id: UserId::new(),
            username: "owner".into(),
        };
        (WorkshopRouterState::new(service), user, data_dir, database)
    }

    fn workflow_definition() -> CreativeWorkflowDefinitionV1 {
        let variable_id = nomifun_common::generate_id();
        let template_id = nomifun_common::generate_id();
        let render_id = nomifun_common::generate_id();
        let generate_id = nomifun_common::generate_id();
        CreativeWorkflowDefinitionV1 {
            id: nomifun_common::CreativeStudioWorkflowId::new().into_string(),
            revision: 1,
            metadata: CreativeWorkflowMetadata {
                name: "电商海报".into(),
                description: "固定结构".into(),
                category: "电商".into(),
                visibility: CreativeWorkflowVisibility::Private,
                tags: Vec::new(),
                created_at: 0,
                updated_at: 0,
            },
            output: CreativeWorkflowOutputPlan::SingleImage,
            variables: vec![CreativeWorkflowVariable::Text {
                id: variable_id.clone(),
                key: "product_name".into(),
                label: "产品名称".into(),
                description: String::new(),
                required: true,
                default_value: None,
                placeholder: String::new(),
                min_length: 0,
                max_length: 200,
            }],
            templates: vec![CreativeWorkflowTemplate {
                id: template_id.clone(),
                name: "主提示词".into(),
                segments: vec![
                    CreativeWorkflowTemplateSegment::Text { text: "为 ".into() },
                    CreativeWorkflowTemplateSegment::Variable { variable_id },
                    CreativeWorkflowTemplateSegment::Text { text: " 生成海报".into() },
                ],
            }],
            steps: vec![
                CreativeWorkflowStep::RenderTemplate {
                    id: render_id.clone(),
                    name: "渲染提示词".into(),
                    depends_on: Vec::new(),
                    enabled: true,
                    template_id: template_id.clone(),
                },
                CreativeWorkflowStep::GenerateImages {
                    id: generate_id,
                    name: "生成图片".into(),
                    depends_on: vec![render_id],
                    enabled: true,
                    prompt_source: CreativeWorkflowPromptSource::Template { template_id },
                    reference_variable_ids: Vec::new(),
                    generation: crate::workflow::CreativeWorkflowImageGenerationSettings {
                        model: None,
                        quality: crate::workflow::CreativeWorkflowImageQuality::Auto,
                        width: 1024,
                        height: 1024,
                        images_per_prompt: 1,
                    },
                },
            ],
        }
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
    async fn prompt_catalog_routes_are_owner_scoped_and_fail_closed() {
        let (state, user, _data_dir) = test_state().await;
        let app = workshop_routes(state).layer(Extension(user));

        let listed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/creative-studio/prompts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_json: Value = serde_json::from_slice(
            &axum::body::to_bytes(listed.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(listed_json["data"]["total"], 0);
        assert_eq!(listed_json["data"]["stale"], true);
        assert_eq!(listed_json["data"]["sources"].as_array().unwrap().len(), 7);

        let rejected = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/prompts/sync")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"force":false,"unexpected":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn creative_workflow_routes_cover_canonical_crud() {
        let (state, user, _data_dir) = test_state().await;
        let app = workshop_routes(state).layer(Extension(user));
        let definition = workflow_definition();
        let workflow_id = definition.id.clone();

        let create_body = serde_json::to_vec(&serde_json::json!({ "workflow": definition }))
            .unwrap();
        let created = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/workflows")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);

        let listed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/creative-studio/workflows")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);

        let detail = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/creative-studio/workflows/{workflow_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::OK);
        let detail_body = axum::body::to_bytes(detail.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail_json: serde_json::Value = serde_json::from_slice(&detail_body).unwrap();
        let mut replacement: CreativeWorkflowDefinitionV1 = serde_json::from_value(
            detail_json["data"]["workflow"].clone(),
        )
        .unwrap();
        replacement.revision = 2;
        replacement.metadata.name = "高端海报".into();

        let saved = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!("/api/creative-studio/workflows/{workflow_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "expectedRevision": "1",
                            "workflow": replacement,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);

        let deleted = app
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/creative-studio/workflows/{workflow_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn creative_workflow_run_routes_cover_idempotent_create_list_get_and_cas() {
        let (state, user, _data_dir, database) = test_state_with_database().await;
        let provider_id = nomifun_common::ProviderId::new().into_string();
        let credentials = nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &[0x24; 32],
        )
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, \
                 enabled, created_at, updated_at) \
             VALUES (?, 'openai', 'run-test', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
        )
        .bind(&provider_id)
        .bind(credentials)
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, 'image-model', 1, 0, NULL, 1, 1)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, \
                 allow_cross_origin_credentials, provider_params, created_at, updated_at) \
             VALUES (?, 'image-model', 'image_generation', '[]', 'openai.images', \
                     'default', 0, '{}', 1, 1)",
        )
        .bind(&provider_id)
        .execute(database.pool())
        .await
        .unwrap();

        let mut definition = workflow_definition();
        let variable_id = match &definition.variables[0] {
            CreativeWorkflowVariable::Text { id, .. } => id.clone(),
            _ => panic!("workflow fixture must contain a text variable"),
        };
        if let CreativeWorkflowStep::GenerateImages { generation, .. } = &mut definition.steps[1]
        {
            generation.model = Some(crate::workflow::CreativeWorkflowImageModelBinding {
                provider_id,
                model: "image-model".into(),
                task: crate::workflow::CreativeWorkflowImageTask::ImageGeneration,
            });
        }
        let definition = state
            .service
            .create_creative_workflow(definition)
            .await
            .unwrap();
        let workflow_id = definition.id.clone();
        let run_id = nomifun_common::CreativeStudioWorkflowRunId::new().into_string();
        let app = workshop_routes(state).layer(Extension(user));
        let create_json = serde_json::json!({
            "request": {
                "runId": run_id,
                "workflowId": workflow_id,
                "workflowRevision": 1,
                "inputs": [{
                    "type": "text",
                    "variableId": variable_id,
                    "value": "NomiFun"
                }],
                "referenceAssetIds": []
            }
        });
        let created = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/workflow-runs")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&create_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
            .await
            .unwrap();
        let created_json: Value = serde_json::from_slice(&created_body).unwrap();
        let mut run: CreativeWorkflowRunAggregateV1 =
            serde_json::from_value(created_json["data"]["run"].clone()).unwrap();

        let replay = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/workflow-runs")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&create_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::CREATED);

        let listed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/creative-studio/workflow-runs?workflowId={workflow_id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let detail = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/creative-studio/workflow-runs/{run_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::OK);

        run.revision = 2;
        run.record.status = crate::workflow_run::CreativeWorkflowRunStatus::Queued;
        run.record.task_ids = vec![nomifun_common::generate_id()];
        run.record.queued_at = Some(run.request.requested_at + 1);
        let saved = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!("/api/creative-studio/workflow-runs/{run_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "expectedRevision": "1",
                            "run": run
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
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

    #[tokio::test]
    async fn canonical_asset_management_routes_replace_legacy_namespace() {
        let (state, user, _data_dir) = test_state().await;
        let existing = state
            .service
            .create_text_asset(NewTextAsset {
                title: "existing".into(),
                text_content: "editable".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
                origin: None,
            })
            .await
            .unwrap();
        let app = workshop_routes(state).layer(Extension(user));

        let list = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/creative-studio/assets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);

        let create = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/assets")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"kind":"text","title":"created","text_content":"body"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);

        let patch = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/api/creative-studio/assets/{}",
                        existing.asset_id
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"renamed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(patch.status(), StatusCode::OK);

        let get = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/creative-studio/assets/{}",
                        existing.asset_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);

        let rename = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/collections/rename")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"from":"missing","to":"renamed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rename.status(), StatusCode::OK);

        let upload = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/assets/upload")
                    .header(header::CONTENT_TYPE, "multipart/form-data; boundary=asset-contract")
                    .body(Body::from("--asset-contract--\r\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::BAD_REQUEST);

        let delete = app
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/creative-studio/assets/{}",
                        existing.asset_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn canonical_asset_file_route_replaces_legacy_namespace() {
        let (state, _user, _data_dir) = test_state().await;
        let asset = state
            .service
            .create_text_asset(NewTextAsset {
                title: "route contract".into(),
                text_content: "canonical bytes".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
                origin: None,
            })
            .await
            .unwrap();
        let canonical = workshop_public_routes(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/creative-studio/files/{}", asset.asset_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(canonical.status(), StatusCode::OK);
    }
}
