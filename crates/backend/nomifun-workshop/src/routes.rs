//! Authenticated `/api/creative-studio/*` project, asset, template, and archive
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
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tower_http::limit::RequestBodyLimitLayer;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, WorkshopAssetId};

use crate::MAX_ASSET_BYTES;
use crate::archive::MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES;
use crate::creative_agent_ops::{
    CreativeAgentOp, CreativeAgentOpResult, MAX_CREATIVE_AGENT_OPS_PER_CALL,
};
use crate::creative_studio::{
    CreativeCanvasDocument, CreativeCanvasSummary, CreativeProjectDocument, CreativeProjectSummary,
};
use crate::dto::WorkshopAsset;
use crate::prompt_catalog::CreativePromptCatalogPage;
use crate::service::{
    AssetPatch, AssetQuery, CreativeProjectAgentKickoff, NewAssetUpload, NewTextAsset,
    TextAssetOrigin,
};
use crate::state::WorkshopRouterState;
use crate::template::{CreativeTemplateDefinitionV1, MAX_TEMPLATE_DEFINITION_BYTES};
use crate::template_draft::template_draft_run_request;
use crate::template_run::{
    CreativeTemplateRunAggregateV1, CreativeTemplateRunCreateRequest,
    MAX_TEMPLATE_RUN_AGGREGATE_BYTES,
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
        .route(
            "/api/creative-studio/canvases/import",
            post(import_creative_canvas_archive),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES,
        ))
        .with_state(state.clone());

    let template_write_router = Router::new()
        .route(
            "/api/creative-studio/templates",
            post(create_creative_template),
        )
        .route(
            "/api/creative-studio/templates/{template_id}",
            axum::routing::put(save_creative_template),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_TEMPLATE_DEFINITION_BYTES + 64 * 1024,
        ))
        .with_state(state.clone());

    let template_run_write_router = Router::new()
        .route(
            "/api/creative-studio/template-runs",
            post(create_creative_template_run),
        )
        .route(
            "/api/creative-studio/template-runs/{template_run_id}",
            axum::routing::put(save_creative_template_run),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            MAX_TEMPLATE_RUN_AGGREGATE_BYTES + 64 * 1024,
        ))
        .with_state(state.clone());

    let legacy_project_alias_router = Router::new()
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
            "/api/creative-studio/projects/{project_id}/agent-ops",
            post(apply_creative_agent_ops),
        )
        .route(
            "/api/creative-studio/projects/{project_id}/archive",
            get(export_creative_project_archive),
        )
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response.headers_mut().insert(
                    header::HeaderName::from_static("deprecation"),
                    HeaderValue::from_static("true"),
                );
                response.headers_mut().insert(
                    header::LINK,
                    HeaderValue::from_static(
                        "</api/creative-studio/canvases>; rel=\"successor-version\"",
                    ),
                );
                response
            },
        ))
        .with_state(state.clone());

    Router::new()
        .route(
            "/api/creative-studio/canvases",
            get(list_creative_canvases).post(create_creative_canvas),
        )
        .route(
            "/api/creative-studio/canvases/{canvas_id}",
            get(get_creative_canvas)
                .patch(rename_creative_canvas)
                .delete(delete_creative_canvas),
        )
        .route(
            "/api/creative-studio/canvases/{canvas_id}/document",
            axum::routing::put(save_creative_canvas),
        )
        .route(
            "/api/creative-studio/canvases/{canvas_id}/agent-ops",
            post(apply_creative_canvas_agent_ops),
        )
        .route(
            "/api/creative-studio/canvases/{canvas_id}/archive",
            get(export_creative_canvas_archive),
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
            "/api/creative-studio/template-drafts",
            post(create_template_draft),
        )
        .route(
            "/api/creative-studio/templates",
            get(list_creative_templates),
        )
        .route(
            "/api/creative-studio/templates/{template_id}",
            get(get_creative_template).delete(delete_creative_template),
        )
        .route(
            "/api/creative-studio/template-runs",
            get(list_creative_template_runs),
        )
        .route(
            "/api/creative-studio/template-runs/{template_run_id}",
            get(get_creative_template_run),
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
            "/api/creative-studio/prompt-library-assets/remove",
            post(remove_prompt_library_assets),
        )
        .route(
            "/api/creative-studio/collections/rename",
            post(rename_collection),
        )
        .with_state(state)
        .merge(legacy_project_alias_router)
        .merge(template_write_router)
        .merge(template_run_write_router)
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

/// User content can now be permanently deleted. Revalidate through the asset
/// service instead of retaining deleted originals in HTTP caches.
const SERVE_CACHE_CONTROL: &str = "private, no-store";

// ── canonical Creative Studio Canvases ─────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeCanvasListResponse {
    canvases: Vec<CreativeCanvasSummary>,
}

async fn list_creative_canvases(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CreativeCanvasListResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeCanvasListResponse {
        canvases: state.service.list_creative_canvases().await?,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeCanvasRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    agent_kickoff: Option<CreateCreativeCanvasAgentKickoff>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeCanvasAgentKickoff {
    prompt: String,
    model: CreateCreativeCanvasAgentModel,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeCanvasAgentModel {
    provider_id: String,
    model: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeCanvasResponse {
    canvas: CreativeCanvasSummary,
}

async fn create_creative_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeCanvasRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let agent_kickoff = req.agent_kickoff.map(|kickoff| CreativeProjectAgentKickoff {
        prompt: kickoff.prompt,
        provider_id: kickoff.model.provider_id,
        model: kickoff.model.model,
    });
    let canvas = state
        .service
        .create_creative_canvas_for_owner(user.id.as_str(), req.title, agent_kickoff)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeCanvasResponse { canvas })),
    ))
}

async fn get_creative_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
) -> Result<Json<ApiResponse<crate::creative_studio::CreativeCanvasDetail>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.service.get_creative_canvas(&canvas_id).await?,
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameCreativeCanvasRequest {
    title: String,
}

async fn rename_creative_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
    body: Result<Json<RenameCreativeCanvasRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeCanvasResponse>>, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let canvas = state
        .service
        .rename_creative_canvas(&canvas_id, &req.title)
        .await?;
    Ok(Json(ApiResponse::ok(CreativeCanvasResponse { canvas })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeCanvasRequest {
    expected_revision: String,
    document: CreativeCanvasDocument,
}

async fn save_creative_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
    body: Result<Json<SaveCreativeCanvasRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeCanvasResponse>>, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let canvas = state
        .service
        .save_creative_canvas(&canvas_id, &req.expected_revision, &req.document)
        .await?;
    Ok(Json(ApiResponse::ok(CreativeCanvasResponse { canvas })))
}

async fn apply_creative_canvas_agent_ops(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
    body: Result<Json<CreativeAgentOpsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeCanvasAgentOpsResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    validate_creative_agent_ops(&request.ops)?;
    let applied = state
        .service
        .apply_creative_canvas_agent_proposal(
            user.id.as_str(),
            &canvas_id,
            &request.assistant_message_id,
            &request.expected_revision,
            request.ops,
            CREATIVE_STUDIO_AGENT_SOURCE,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeCanvasAgentOpsResponse {
        canvas: applied.canvas,
        ops: applied.ops,
        replayed: applied.replayed,
        applied_revision: applied.applied_revision,
    })))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeCanvasAgentOpsResponse {
    canvas: CreativeCanvasSummary,
    ops: Vec<CreativeAgentOpResult>,
    replayed: bool,
    applied_revision: String,
}

async fn delete_creative_canvas(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_creative_canvas(&canvas_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

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
    #[serde(default)]
    agent_kickoff: Option<CreateCreativeProjectAgentKickoff>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeProjectAgentKickoff {
    prompt: String,
    model: CreateCreativeProjectAgentModel,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeProjectAgentModel {
    provider_id: String,
    model: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeProjectResponse {
    project: CreativeProjectSummary,
}

async fn create_creative_project(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeProjectRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let agent_kickoff = req.agent_kickoff.map(|kickoff| CreativeProjectAgentKickoff {
        prompt: kickoff.prompt,
        provider_id: kickoff.model.provider_id,
        model: kickoff.model.model,
    });
    let project = state
        .service
        .create_creative_project_for_owner(user.id.as_str(), req.title, agent_kickoff)
        .await?;
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

const CREATIVE_STUDIO_AGENT_SOURCE: &str = "creative-studio-agent";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeAgentOpsRequest {
    assistant_message_id: String,
    expected_revision: String,
    ops: Vec<CreativeAgentOp>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeAgentOpsResponse {
    project: CreativeProjectSummary,
    ops: Vec<CreativeAgentOpResult>,
    replayed: bool,
    applied_revision: String,
}

fn validate_creative_agent_ops(ops: &[CreativeAgentOp]) -> Result<(), AppError> {
    if ops.is_empty() || ops.len() > MAX_CREATIVE_AGENT_OPS_PER_CALL {
        return Err(AppError::BadRequest(format!(
            "Creative Studio Agent operations must contain 1 to {MAX_CREATIVE_AGENT_OPS_PER_CALL} entries"
        )));
    }
    if ops
        .iter()
        .any(|op| matches!(op, CreativeAgentOp::DeleteNode { .. }))
    {
        return Err(AppError::BadRequest(
            "Creative Studio Agent operations cannot delete nodes; deletion requires explicit user confirmation"
                .to_owned(),
        ));
    }
    Ok(())
}

async fn apply_creative_agent_ops(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
    body: Result<Json<CreativeAgentOpsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeAgentOpsResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    validate_creative_agent_ops(&request.ops)?;
    let applied = state
        .service
        .apply_creative_agent_proposal(
            user.id.as_str(),
            &project_id,
            &request.assistant_message_id,
            &request.expected_revision,
            request.ops,
            CREATIVE_STUDIO_AGENT_SOURCE,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeAgentOpsResponse {
        project: applied.project,
        ops: applied.ops,
        replayed: applied.replayed,
        applied_revision: applied.applied_revision,
    })))
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

// ── canonical Creative Studio templates ────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTemplateDraftRequest {
    prompt: String,
    model: CreateTemplateDraftModel,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTemplateDraftModel {
    provider_id: String,
    model: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateTemplateDraftResponse {
    text: String,
}

/// Run one owner-only, stateless model completion. The provider lifecycle read
/// guard spans exact capability validation and the live stream to fence
/// destructive Provider/model deletion. Ordinary updates are not covered by
/// this barrier; the app runner resolves and freezes one config snapshot.
async fn create_template_draft(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateTemplateDraftRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreateTemplateDraftResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    state
        .service
        .require_creative_studio_owner(user.id.as_str())
        .await?;
    let run_request = template_draft_run_request(
        request.prompt,
        request.model.provider_id,
        request.model.model,
    )?;

    let _provider_guard = state.service.provider_read_guard().await;
    state
        .service
        .require_template_draft_chat_model(&run_request.provider_id, &run_request.model)
        .await?;
    let text = state.template_draft_runner.run(run_request).await?;
    if text.trim().is_empty() {
        return Err(AppError::BadGateway(
            "template draft model returned an empty response".into(),
        ));
    }
    Ok(Json(ApiResponse::ok(CreateTemplateDraftResponse { text })))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeTemplateListResponse {
    templates: Vec<CreativeTemplateDefinitionV1>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeTemplateResponse {
    template: CreativeTemplateDefinitionV1,
}

async fn list_creative_templates(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CreativeTemplateListResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeTemplateListResponse {
        templates: state.service.list_creative_templates().await?,
    })))
}

async fn get_creative_template(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(template_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeTemplateResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeTemplateResponse {
        template: state.service.get_creative_template(&template_id).await?,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeTemplateRequest {
    template: CreativeTemplateDefinitionV1,
}

async fn create_creative_template(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeTemplateRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let template = state
        .service
        .create_creative_template(request.template)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeTemplateResponse { template })),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeTemplateRequest {
    expected_revision: String,
    template: CreativeTemplateDefinitionV1,
}

async fn save_creative_template(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(template_id): Path<String>,
    body: Result<Json<SaveCreativeTemplateRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeTemplateResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let template = state
        .service
        .save_creative_template(
            &template_id,
            &request.expected_revision,
            request.template,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeTemplateResponse {
        template,
    })))
}

async fn delete_creative_template(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(template_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_creative_template(&template_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── durable Creative Studio template runs ─────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeTemplateRunQuery {
    template_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeTemplateRunListResponse {
    runs: Vec<CreativeTemplateRunAggregateV1>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreativeTemplateRunResponse {
    run: CreativeTemplateRunAggregateV1,
}

async fn list_creative_template_runs(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<CreativeTemplateRunQuery>,
) -> Result<Json<ApiResponse<CreativeTemplateRunListResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeTemplateRunListResponse {
        runs: state
            .service
            .list_creative_template_runs(query.template_id.as_deref())
            .await?,
    })))
}

async fn get_creative_template_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(template_run_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeTemplateRunResponse>>, AppError> {
    Ok(Json(ApiResponse::ok(CreativeTemplateRunResponse {
        run: state
            .service
            .get_creative_template_run(&template_run_id)
            .await?,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCreativeTemplateRunRequest {
    request: CreativeTemplateRunCreateRequest,
}

async fn create_creative_template_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCreativeTemplateRunRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let run = state
        .service
        .create_creative_template_run(request.request)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeTemplateRunResponse { run })),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveCreativeTemplateRunRequest {
    expected_revision: String,
    run: CreativeTemplateRunAggregateV1,
}

async fn save_creative_template_run(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(template_run_id): Path<String>,
    body: Result<Json<SaveCreativeTemplateRunRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeTemplateRunResponse>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let run = state
        .service
        .save_creative_template_run(
            &template_run_id,
            &request.expected_revision,
            request.run,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeTemplateRunResponse { run })))
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

async fn export_creative_canvas_archive(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(canvas_id): Path<String>,
) -> Result<Response, AppError> {
    let archive = state
        .service
        .export_creative_canvas_archive(&canvas_id)
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

async fn import_creative_canvas_archive(
    State(state): State<WorkshopRouterState>,
    Extension(_user): Extension<CurrentUser>,
    bytes: Bytes,
) -> Result<impl IntoResponse, AppError> {
    let canvas = state
        .service
        .import_creative_canvas_archive(bytes.to_vec())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeCanvasResponse { canvas })),
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
) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(ApiResponse::ok(state.service.get_asset(asset_id.as_str()).await?)),
    ))
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
    origin: Option<TextAssetOrigin>,
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
#[serde(deny_unknown_fields)]
struct RemovePromptLibraryAssetsRequest {
    prompt_library_source: String,
    prompt_library_id: String,
}

#[derive(serde::Serialize)]
struct RemovePromptLibraryAssetsResponse {
    matched: u64,
}

async fn remove_prompt_library_assets(
    State(state): State<WorkshopRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<RemovePromptLibraryAssetsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<RemovePromptLibraryAssetsResponse>>, AppError> {
    state
        .service
        .require_creative_studio_owner(user.id.as_str())
        .await?;
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let matched = state
        .service
        .hide_prompt_library_assets(
            &request.prompt_library_source,
            &request.prompt_library_id,
        )
        .await?;
    Ok(Json(ApiResponse::ok(RemovePromptLibraryAssetsResponse {
        matched,
    })))
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
    state.service.delete_asset_content(asset_id.as_str()).await?;
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

enum AssetByteRange {
    Full,
    Partial(std::ops::Range<usize>),
    Unsatisfiable,
}

fn parse_asset_byte_range(value: &str, length: usize) -> AssetByteRange {
    // Media elements request a single range. Unsupported units, malformed
    // syntax, and multipart ranges can safely fall back to the full response.
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return AssetByteRange::Full;
    };
    let Some((start, end)) = spec.split_once('-') else {
        return AssetByteRange::Full;
    };
    let offset = |value: &str| -> Option<usize> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        // Oversized decimal offsets are legal syntax: clamp ends/suffixes and
        // reject starts beyond EOF without overflowing on untrusted headers.
        Some(value.bytes().fold(0usize, |number, digit| {
            number.saturating_mul(10).saturating_add((digit - b'0') as usize)
        }))
    };
    if start.is_empty() {
        let Some(suffix) = offset(end) else {
            return AssetByteRange::Full;
        };
        return if suffix == 0 || length == 0 {
            AssetByteRange::Unsatisfiable
        } else {
            AssetByteRange::Partial(length.saturating_sub(suffix)..length)
        };
    }
    let Some(start) = offset(start) else {
        return AssetByteRange::Full;
    };
    let end = if end.is_empty() {
        usize::MAX
    } else if let Some(end) = offset(end) {
        end
    } else {
        return AssetByteRange::Full;
    };
    if start >= length || end < start {
        return AssetByteRange::Unsatisfiable;
    }
    AssetByteRange::Partial(start..end.min(length - 1) + 1)
}

/// AUTH-EXEMPT (mounted on [`workshop_public_routes`]): no `CurrentUser`
/// extractor. Serves an asset's original binary (or, with `?thumb=1`, its
/// thumbnail). Supports the single byte ranges used by media elements.
/// Traversal-safe via the service; missing or deleted assets still return 404.
async fn serve_file(
    State(state): State<WorkshopRouterState>,
    Path(asset_id): Path<WorkshopAssetId>,
    Query(query): Query<FileQuery>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let thumb = query.thumb.as_deref().map(parse_bool_flag).unwrap_or(false);
    let served = state.service.serve_file(asset_id.as_str(), thumb).await?;
    let bytes = Bytes::from(served.bytes);
    let length = bytes.len();
    // Range applies only to GET. There are no representation validators on
    // this no-store route, so an If-Range request must receive the full file.
    let range = if method == Method::GET
        && !headers.contains_key(header::IF_RANGE)
        && headers.get_all(header::RANGE).iter().count() == 1
    {
        headers.get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .map(|value| parse_asset_byte_range(value, length))
            .unwrap_or(AssetByteRange::Full)
    } else {
        AssetByteRange::Full
    };
    let (status, content_range, body) = match range {
        AssetByteRange::Full => (StatusCode::OK, None, bytes),
        AssetByteRange::Partial(range) => (
            StatusCode::PARTIAL_CONTENT,
            Some(format!("bytes {}-{}/{length}", range.start, range.end - 1)),
            bytes.slice(range),
        ),
        AssetByteRange::Unsatisfiable => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            Some(format!("bytes */{length}")),
            Bytes::new(),
        ),
    };
    let response = (
        status,
        [
            (header::CONTENT_TYPE, served.mime),
            (header::CACHE_CONTROL, SERVE_CACHE_CONTROL.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_owned()),
            (header::CONTENT_LENGTH, body.len().to_string()),
        ],
        Body::from(body),
    )
        .into_response();
    Ok(match content_range {
        Some(value) => ([(header::CONTENT_RANGE, value)], response).into_response(),
        None => response,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nomifun_common::{CreativeStudioNodeId, MessageId, ProviderId, UserId};
    use nomifun_db::{IWorkshopRepository, SqliteWorkshopRepository};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::{WorkshopService, TemplateDraftRunRequest, TemplateDraftRunner};
    use crate::template::{
        CreativeTemplateMetadata, CreativeTemplateOutputPlan, CreativeTemplatePromptSource,
        CreativeTemplateStep, CreativePromptTemplate, CreativePromptTemplateSegment,
        CreativeTemplateVariable, CreativeTemplateVisibility,
    };

    struct RecordingTemplateDraftRunner {
        calls: Mutex<Vec<TemplateDraftRunRequest>>,
        response: String,
    }

    impl RecordingTemplateDraftRunner {
        fn new(response: impl Into<String>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                response: response.into(),
            }
        }

        fn calls(&self) -> Vec<TemplateDraftRunRequest> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TemplateDraftRunner for RecordingTemplateDraftRunner {
        async fn run(&self, request: TemplateDraftRunRequest) -> Result<String, AppError> {
            self.calls.lock().unwrap().push(request);
            Ok(self.response.clone())
        }
    }

    struct BlockingTemplateDraftRunner {
        calls: Mutex<Vec<TemplateDraftRunRequest>>,
        entered: tokio::sync::Semaphore,
        release: tokio::sync::Semaphore,
    }

    impl BlockingTemplateDraftRunner {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                entered: tokio::sync::Semaphore::new(0),
                release: tokio::sync::Semaphore::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl TemplateDraftRunner for BlockingTemplateDraftRunner {
        async fn run(&self, request: TemplateDraftRunRequest) -> Result<String, AppError> {
            self.calls.lock().unwrap().push(request);
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok("```json\n{\"kind\":\"nomifun.creative-studio.template-draft/v1\"}\n```".into())
        }
    }

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
        test_state_with_database_and_runner(Arc::new(RecordingTemplateDraftRunner::new(
            "unused template draft",
        )))
        .await
    }

    async fn test_state_with_database_and_runner(
        template_draft_runner: Arc<dyn TemplateDraftRunner>,
    ) -> (
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
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let user = CurrentUser {
            id: UserId::parse(owner_id).unwrap(),
            username: "owner".into(),
        };
        (
            WorkshopRouterState::new(service, template_draft_runner),
            user,
            data_dir,
            database,
        )
    }

    fn template_definition() -> CreativeTemplateDefinitionV1 {
        let variable_id = nomifun_common::generate_id();
        let template_id = nomifun_common::generate_id();
        let render_id = nomifun_common::generate_id();
        let generate_id = nomifun_common::generate_id();
        CreativeTemplateDefinitionV1 {
            id: nomifun_common::CreativeStudioTemplateId::new().into_string(),
            revision: 1,
            metadata: CreativeTemplateMetadata {
                name: "电商海报".into(),
                description: "固定结构".into(),
                category: "电商".into(),
                visibility: CreativeTemplateVisibility::Private,
                tags: Vec::new(),
                created_at: 0,
                updated_at: 0,
            },
            output: CreativeTemplateOutputPlan::SingleImage,
            variables: vec![CreativeTemplateVariable::Text {
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
            templates: vec![CreativePromptTemplate {
                id: template_id.clone(),
                name: "主提示词".into(),
                segments: vec![
                    CreativePromptTemplateSegment::Text { text: "为 ".into() },
                    CreativePromptTemplateSegment::Variable { variable_id },
                    CreativePromptTemplateSegment::Text { text: " 生成海报".into() },
                ],
            }],
            steps: vec![
                CreativeTemplateStep::RenderTemplate {
                    id: render_id.clone(),
                    name: "渲染提示词".into(),
                    depends_on: Vec::new(),
                    enabled: true,
                    template_id: template_id.clone(),
                },
                CreativeTemplateStep::GenerateImages {
                    id: generate_id,
                    name: "生成图片".into(),
                    depends_on: vec![render_id],
                    enabled: true,
                    prompt_source: CreativeTemplatePromptSource::Template { template_id },
                    reference_variable_ids: Vec::new(),
                    generation: crate::template::CreativeTemplateImageGenerationSettings {
                        model: None,
                        quality: crate::template::CreativeTemplateImageQuality::Auto,
                        width: 1024,
                        height: 1024,
                        images_per_prompt: 1,
                    },
                },
            ],
        }
    }

    fn add_text_op(text: &str, x: f64, y: f64) -> Value {
        serde_json::json!({
            "type": "add_node",
            "node_type": "text",
            "x": x,
            "y": y,
            "data": {
                "text": text,
                "format": "plain",
                "fontSize": 16,
                "textAlign": "left"
            }
        })
    }

    fn add_idle_config_op() -> Value {
        serde_json::json!({
            "type": "add_node",
            "node_type": "config",
            "x": 0,
            "y": 0,
            "data": {
                "task": "image_generation",
                "capability": "t2i",
                "providerId": null,
                "model": null,
                "prompt": "",
                "negativePrompt": "",
                "parameters": {},
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [],
                "status": "idle",
                "errorMessage": null
            }
        })
    }

    async fn post_agent_ops(app: &Router, project_id: &str, body: Value) -> Response {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/creative-studio/projects/{project_id}/agent-ops"
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
    }

    async fn post_create_project(app: &Router, body: Value) -> Response {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn post_template_draft(app: &Router, body: Value) -> Response {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/template-drafts")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TemplateDraftPersistenceCounts {
        conversations: i64,
        messages: i64,
        projects: i64,
        templates: i64,
        template_runs: i64,
        agent_sessions: i64,
        proposal_receipts: i64,
        creation_tasks: i64,
        assets: i64,
    }

    async fn template_draft_persistence_counts(
        database: &nomifun_db::Database,
    ) -> TemplateDraftPersistenceCounts {
        async fn count(database: &nomifun_db::Database, table: &str) -> i64 {
            let query = format!("SELECT COUNT(*) FROM {table}");
            nomifun_db::sqlx::query_scalar(&query)
                .fetch_one(database.pool())
                .await
                .unwrap()
        }
        TemplateDraftPersistenceCounts {
            conversations: count(database, "conversations").await,
            messages: count(database, "messages").await,
            projects: count(database, "creative_studio_projects").await,
            templates: count(database, "creative_studio_templates").await,
            template_runs: count(database, "creative_studio_template_runs").await,
            agent_sessions: count(database, "creative_studio_agent_sessions").await,
            proposal_receipts: count(database, "creative_studio_agent_proposal_receipts").await,
            creation_tasks: count(database, "creation_tasks").await,
            assets: count(database, "workshop_assets").await,
        }
    }

    async fn seed_enabled_chat_model(
        database: &nomifun_db::Database,
        provider_id: &str,
        model: &str,
    ) {
        let credentials = nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &[0x31; 32],
        )
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, \
                 enabled, created_at, updated_at) \
             VALUES (?, 'openai', 'kickoff-test', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
        )
        .bind(provider_id)
        .bind(credentials)
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, ?, 1, 0, NULL, 1, 1)",
        )
        .bind(provider_id)
        .bind(model)
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, \
                 allow_cross_origin_credentials, provider_params, created_at, updated_at) \
             VALUES (?, ?, 'chat', '[]', 'openai.chat_text', 'default', 0, '{}', 1, 1)",
        )
        .bind(provider_id)
        .bind(model)
        .execute(database.pool())
        .await
        .unwrap();
    }

    async fn seed_bound_completed_agent_messages(
        database: &nomifun_db::Database,
        project_id: &str,
        assistant_texts: &[String],
    ) -> Vec<String> {
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let session_id = nomifun_common::generate_id();
        let conversation_id = nomifun_common::ConversationId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let mut message_ids = Vec::with_capacity(assistant_texts.len() * 2);
        let mut assistant_ids = Vec::with_capacity(assistant_texts.len());
        for _ in assistant_texts {
            message_ids.push(MessageId::new().into_string());
            let assistant_id = MessageId::new().into_string();
            message_ids.push(assistant_id.clone());
            assistant_ids.push(assistant_id);
        }
        let document_json: String = nomifun_db::sqlx::query_scalar(
            "SELECT document_json FROM creative_studio_projects WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        let mut document: Value = serde_json::from_str(&document_json).unwrap();
        document["chatSessions"] = serde_json::json!([{
            "id": session_id,
            "title": "Agent",
            "messageIds": message_ids,
            "model": { "providerId": provider_id, "model": "chat-model" },
            "pendingTurn": null,
            "createdAt": 1,
            "updatedAt": 1
        }]);
        document["activeChatId"] = Value::String(session_id.clone());
        nomifun_db::sqlx::query(
            "UPDATE creative_studio_projects SET document_json = ? WHERE project_id = ?",
        )
        .bind(document.to_string())
        .bind(project_id)
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, extra, model, status, source, created_at, updated_at) \
             VALUES (?, ?, 'Creative Studio Agent', 'nomi', '{}', ?, 'finished', 'nomifun', 1, 1)",
        )
        .bind(&conversation_id)
        .bind(&owner_id)
        .bind(serde_json::json!({
            "provider_id": provider_id,
            "model": "chat-model",
            "use_model": "chat-model"
        }).to_string())
        .execute(database.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO creative_studio_agent_sessions \
                (owner_id, project_id, session_id, conversation_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 1, 1)",
        )
        .bind(&owner_id)
        .bind(project_id)
        .bind(&session_id)
        .bind(&conversation_id)
        .execute(database.pool())
        .await
        .unwrap();
        for (index, message_id) in message_ids.iter().enumerate() {
            let (position, content) = if index % 2 == 0 {
                ("right", serde_json::json!({ "content": "request" }))
            } else {
                (
                    "left",
                    serde_json::json!({
                        "content": assistant_texts[index / 2],
                        "turn_id": MessageId::new().into_string()
                    }),
                )
            };
            nomifun_db::sqlx::query(
                "INSERT INTO messages \
                    (message_id, conversation_id, msg_id, type, content, position, status, hidden, created_at) \
                 VALUES (?, ?, ?, 'text', ?, ?, 'finish', 0, ?)",
            )
            .bind(message_id)
            .bind(&conversation_id)
            .bind(message_id)
            .bind(content.to_string())
            .bind(position)
            .bind(i64::try_from(index + 1).unwrap())
            .execute(database.pool())
            .await
            .unwrap();
        }
        assistant_ids
    }

    fn canvas_artifact_text(ops: Value) -> String {
        format!(
            "```json\n{}\n```",
            serde_json::json!({
                "kind": "nomifun.creative-studio.canvas-ops/v1",
                "summary": "Apply safe canvas changes",
                "ops": ops
            })
        )
    }

    #[tokio::test]
    async fn creative_agent_ops_route_commits_one_cas_batch_and_mints_ids() {
        let (state, user, _data_dir, database) = test_state_with_database().await;
        let project = state
            .service
            .create_creative_project(Some("Agent route".into()))
            .await
            .unwrap();
        let seed_ops = serde_json::json!([
            add_text_op("first", 10.0, 20.0),
            add_text_op("second", 400.0, 20.0)
        ]);
        let applied_ops = serde_json::json!([add_text_op("server minted", 800.0, 20.0)]);
        let assistant_ids = seed_bound_completed_agent_messages(
            &database,
            &project.project_id,
            &[
                canvas_artifact_text(seed_ops.clone()),
                canvas_artifact_text(applied_ops.clone()),
            ],
        )
        .await;
        let app = workshop_routes(state.clone()).layer(Extension(user));

        let seed = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[0],
                "expectedRevision": "1",
                "ops": seed_ops
            }),
        )
        .await;
        assert_eq!(seed.status(), StatusCode::OK);
        let seed = response_json(seed).await;
        assert_eq!(seed["data"]["project"]["revision"], "2");
        let first_id = seed["data"]["ops"][0]["node_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second_id = seed["data"]["ops"][1]["node_id"]
            .as_str()
            .unwrap()
            .to_owned();
        CreativeStudioNodeId::parse(&first_id).unwrap();
        CreativeStudioNodeId::parse(&second_id).unwrap();

        let applied = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "2",
                "ops": applied_ops.clone()
            }),
        )
        .await;
        assert_eq!(applied.status(), StatusCode::OK);
        let applied = response_json(applied).await;
        let data = applied["data"].as_object().unwrap();
        assert_eq!(data.len(), 4);
        assert!(data.contains_key("project"));
        assert!(data.contains_key("ops"));
        assert_eq!(data["replayed"], false);
        assert_eq!(data["appliedRevision"], "3");
        assert_eq!(data["project"]["revision"], "3");
        assert_eq!(data["project"]["nodeCount"], 3);
        assert_eq!(data["project"]["connectionCount"], 0);
        assert_eq!(data["ops"][0]["type"], "node_added");
        let minted_id = data["ops"][0]["node_id"].as_str().unwrap();
        CreativeStudioNodeId::parse(minted_id).unwrap();
        assert_ne!(minted_id, first_id);
        assert_ne!(minted_id, second_id);

        let detail = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(detail.project.revision, "3");
        assert_eq!(detail.document.nodes.len(), 3);
        assert!(detail.document.connections.is_empty());
        assert!(detail.document.nodes.iter().any(|node| node.id == minted_id));

        // Simulate a lost HTTP response: even a now-stale revision replays the
        // original receipt and never re-runs the add/connect operations.
        let replay = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "1",
                "ops": applied_ops.clone()
            }),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay = response_json(replay).await;
        assert_eq!(replay["data"]["replayed"], true);
        assert_eq!(replay["data"]["appliedRevision"], "3");
        assert_eq!(replay["data"]["project"]["revision"], "3");
        assert_eq!(replay["data"]["project"]["nodeCount"], 3);
        assert_eq!(replay["data"]["ops"][0]["node_id"], minted_id);

        let mismatch = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "3",
                "ops": [add_text_op("different payload", 0.0, 0.0)]
            }),
        )
        .await;
        assert_eq!(mismatch.status(), StatusCode::CONFLICT);
        let mismatch = response_json(mismatch).await;
        assert_eq!(mismatch["code"], "CONFLICT");
        let final_detail = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(final_detail.project.revision, "3");
        assert_eq!(final_detail.document.nodes.len(), 3);
    }

    #[tokio::test]
    async fn creative_agent_ops_route_reports_stale_revision_without_writing() {
        let (state, user, _data_dir, database) = test_state_with_database().await;
        let project = state
            .service
            .create_creative_project(Some("Agent stale CAS".into()))
            .await
            .unwrap();
        let first_ops = serde_json::json!([add_text_op("committed", 0.0, 0.0)]);
        let stale_ops = serde_json::json!([add_text_op("must not persist", 20.0, 20.0)]);
        let assistant_ids = seed_bound_completed_agent_messages(
            &database,
            &project.project_id,
            &[
                canvas_artifact_text(first_ops.clone()),
                canvas_artifact_text(stale_ops.clone()),
            ],
        )
        .await;
        let app = workshop_routes(state.clone()).layer(Extension(user));

        let first = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[0],
                "expectedRevision": "1",
                "ops": first_ops
            }),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let before = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();

        let stale = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "1",
                "ops": stale_ops
            }),
        )
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale = response_json(stale).await;
        assert_eq!(stale["code"], "REVISION_CONFLICT");

        let after = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(after.project, before.project);
        assert_eq!(after.document, before.document);
    }

    #[tokio::test]
    async fn creative_agent_ops_route_rejects_non_owner_without_writing() {
        let (state, _owner, _data_dir, database) = test_state_with_database().await;
        let project = state
            .service
            .create_creative_project(Some("Agent owner boundary".into()))
            .await
            .unwrap();
        let rejected_ops = serde_json::json!([add_text_op("must not persist", 0.0, 0.0)]);
        let assistant_ids = seed_bound_completed_agent_messages(
            &database,
            &project.project_id,
            &[canvas_artifact_text(rejected_ops.clone())],
        )
        .await;
        let app = workshop_routes(state.clone()).layer(Extension(CurrentUser {
            id: UserId::new(),
            username: "not-owner".into(),
        }));
        let rejected = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[0],
                "expectedRevision": "1",
                "ops": rejected_ops
            }),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        let rejected = response_json(rejected).await;
        assert_eq!(rejected["code"], "FORBIDDEN");
        let current = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(current.project.revision, "1");
        assert!(current.document.nodes.is_empty());
        let receipt_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM creative_studio_agent_proposal_receipts",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(receipt_count, 0);
    }

    #[tokio::test]
    async fn creative_agent_ops_route_rejects_extra_fields_destructive_ops_and_invalid_batches_atomically()
    {
        let (state, user, _data_dir, database) = test_state_with_database().await;
        let project = state
            .service
            .create_creative_project(Some("Agent strict input".into()))
            .await
            .unwrap();
        let seeded = state
            .service
            .apply_creative_agent_ops(
                &project.project_id,
                "1",
                vec![serde_json::from_value(add_idle_config_op()).unwrap()],
                "test-fixture",
            )
            .await
            .unwrap();
        let config_id = match &seeded.ops[0] {
            CreativeAgentOpResult::NodeAdded { node_id } => node_id.clone(),
            other => panic!("unexpected seed result {other:?}"),
        };
        let safe_move = serde_json::json!([{
            "type": "move_node",
            "node_id": config_id,
            "x": 10.0,
            "y": 20.0
        }]);
        let media_ops = serde_json::json!([{
            "type": "add_node",
            "node_type": "image",
            "x": 0.0,
            "y": 0.0,
            "data": { "assetId": null, "caption": "", "alt": "", "fit": "cover", "naturalSize": null }
        }]);
        let config_ops = serde_json::json!([add_idle_config_op()]);
        let runtime_ops = serde_json::json!([{
            "type": "update_node_data",
            "node_id": config_id,
            "patch": {
                "taskId": CreativeStudioNodeId::new().into_string(),
                "status": "running"
            }
        }]);
        let duplicate_artifact = format!(
            "```json\n{{\"kind\":\"nomifun.creative-studio.canvas-ops/v1\",\"summary\":\"first\",\"\\u0073ummary\":\"second\",\"ops\":{}}}\n```",
            safe_move
        );
        let assistant_ids = seed_bound_completed_agent_messages(
            &database,
            &project.project_id,
            &[
                canvas_artifact_text(safe_move.clone()),
                "proposal".to_owned(),
                duplicate_artifact,
                canvas_artifact_text(media_ops.clone()),
                canvas_artifact_text(config_ops.clone()),
                canvas_artifact_text(runtime_ops.clone()),
                canvas_artifact_text(safe_move.clone()),
            ],
        )
        .await;
        let app = workshop_routes(state.clone()).layer(Extension(user));
        let before = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();

        let rejected_bodies = [
            serde_json::json!({
                "expectedRevision": "2",
                "ops": [add_text_op("missing assistant identity", 0.0, 0.0)]
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "2",
                "ops": safe_move.clone(),
                "source": "client-controlled"
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "2",
                "ops": []
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "2",
                "ops": [{
                    "type": "delete_node",
                    "node_id": config_id
                }]
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[1],
                "expectedRevision": "2",
                "ops": safe_move.clone()
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[2],
                "expectedRevision": "2",
                "ops": safe_move.clone()
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[3],
                "expectedRevision": "2",
                "ops": media_ops
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[4],
                "expectedRevision": "2",
                "ops": config_ops
            }),
            serde_json::json!({
                "assistantMessageId": assistant_ids[5],
                "expectedRevision": "2",
                "ops": runtime_ops
            }),
        ];

        for body in rejected_bodies {
            let rejected = post_agent_ops(&app, &project.project_id, body).await;
            assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
            let after = state
                .service
                .get_creative_project(&project.project_id)
                .await
                .unwrap();
            assert_eq!(after.project, before.project);
            assert_eq!(after.document, before.document);
        }

        let mismatch = post_agent_ops(
            &app,
            &project.project_id,
            serde_json::json!({
                "assistantMessageId": assistant_ids[6],
                "expectedRevision": "2",
                "ops": [{
                    "type": "move_node",
                    "node_id": config_id,
                    "x": 11.0,
                    "y": 20.0
                }]
            }),
        )
        .await;
        assert_eq!(mismatch.status(), StatusCode::CONFLICT);
        let after = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(after.project, before.project);
        assert_eq!(after.document, before.document);
        let receipt_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM creative_studio_agent_proposal_receipts",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(receipt_count, 0);
    }

    #[tokio::test]
    async fn template_draft_route_runs_one_exact_stateless_completion_without_persisting() {
        let artifact = r#"```json
{"kind":"nomifun.creative-studio.template-draft/v1","summary":"三张社交海报","draft":{"mode":"multi-image-series","name":"社交海报组","description":"同一主题的三张海报","category":"社交媒体","promptTemplate":"为 {{topic}} 设计 {{style}} 风格的 {{platform}} 海报"}}
```"#;
        let runner = Arc::new(RecordingTemplateDraftRunner::new(artifact));
        let (state, owner, _data_dir, database) =
            test_state_with_database_and_runner(runner.clone()).await;
        let provider_id = ProviderId::new().into_string();
        seed_enabled_chat_model(&database, &provider_id, "chat-model").await;
        let before = template_draft_persistence_counts(&database).await;
        let app = workshop_routes(state).layer(Extension(owner));

        let response = post_template_draft(
            &app,
            serde_json::json!({
                "prompt": "  设计三张统一风格的社交媒体海报  ",
                "model": {
                    "providerId": provider_id,
                    "model": "chat-model"
                }
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = response_json(response).await;
        assert_eq!(response["data"]["text"], artifact);
        assert_eq!(response["data"].as_object().unwrap().len(), 1);

        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            1,
            "one route request must invoke the product runner once"
        );
        assert_eq!(calls[0].provider_id, provider_id);
        assert_eq!(calls[0].model, "chat-model");
        assert_eq!(calls[0].user_text, "设计三张统一风格的社交媒体海报");
        assert_eq!(
            calls[0].system_prompt,
            crate::template_draft::TEMPLATE_DRAFT_SYSTEM_PROMPT
        );
        assert!(calls[0]
            .system_prompt
            .contains("nomifun.creative-studio.template-draft/v1"));
        assert!(calls[0].system_prompt.contains("{{product_name}}"));
        assert!(calls[0].system_prompt.contains("{{topic}}"));
        assert!(calls[0].system_prompt.contains("Never save or run a template"));

        let after = template_draft_persistence_counts(&database).await;
        assert_eq!(after, before, "draft completion must not persist product state");
    }

    #[tokio::test]
    async fn template_draft_route_rejects_owner_body_and_catalog_failures_before_runner() {
        let runner = Arc::new(RecordingTemplateDraftRunner::new("must not run"));
        let (state, owner, _data_dir, database) =
            test_state_with_database_and_runner(runner.clone()).await;
        let provider_id = ProviderId::new().into_string();
        seed_enabled_chat_model(&database, &provider_id, "chat-model").await;
        let before = template_draft_persistence_counts(&database).await;
        let valid = || {
            serde_json::json!({
                "prompt": "设计一张产品海报",
                "model": {
                    "providerId": provider_id,
                    "model": "chat-model"
                }
            })
        };

        let non_owner_app = workshop_routes(state.clone()).layer(Extension(CurrentUser {
            id: UserId::new(),
            username: "not-owner".into(),
        }));
        let non_owner = post_template_draft(&non_owner_app, valid()).await;
        assert_eq!(non_owner.status(), StatusCode::FORBIDDEN);

        let app = workshop_routes(state).layer(Extension(owner));
        let rejected = [
            serde_json::json!({
                "prompt": " \r\n ",
                "model": { "providerId": provider_id, "model": "chat-model" }
            }),
            serde_json::json!({
                "prompt": "😀".repeat(
                    crate::template_draft::MAX_TEMPLATE_DRAFT_PROMPT_UTF16 / 2 + 1
                ),
                "model": { "providerId": provider_id, "model": "chat-model" }
            }),
            serde_json::json!({
                "prompt": "设计海报",
                "model": { "providerId": "not-a-provider", "model": "chat-model" }
            }),
            serde_json::json!({
                "prompt": "设计海报",
                "model": { "providerId": provider_id, "model": " chat-model " }
            }),
            serde_json::json!({
                "prompt": "设计海报",
                "model": {
                    "providerId": provider_id,
                    "model": "m".repeat(
                        crate::template_draft::MAX_TEMPLATE_DRAFT_MODEL_UTF16 + 1
                    )
                }
            }),
            serde_json::json!({
                "prompt": "设计海报",
                "model": {
                    "providerId": provider_id,
                    "model": "chat-model",
                    "useModel": "client-controlled"
                }
            }),
            serde_json::json!({
                "prompt": "设计海报",
                "model": { "providerId": provider_id, "model": "chat-model" },
                "history": []
            }),
        ];
        for body in rejected {
            let response = post_template_draft(&app, body).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let missing_exact = post_template_draft(
            &app,
            serde_json::json!({
                "prompt": "设计海报",
                "model": { "providerId": provider_id, "model": "other-model" }
            }),
        )
        .await;
        assert_eq!(missing_exact.status(), StatusCode::CONFLICT);

        nomifun_db::sqlx::query(
            "UPDATE provider_models SET enabled = 0 WHERE provider_id = ? AND model = ?",
        )
        .bind(&provider_id)
        .bind("chat-model")
        .execute(database.pool())
        .await
        .unwrap();
        let disabled = post_template_draft(&app, valid()).await;
        assert_eq!(disabled.status(), StatusCode::CONFLICT);

        assert!(
            runner.calls().is_empty(),
            "authorization, input, and catalog failures must precede model invocation"
        );
        let after = template_draft_persistence_counts(&database).await;
        assert_eq!(after, before, "rejected drafts must not persist product state");
    }

    #[tokio::test]
    async fn template_draft_route_blocks_provider_deletion_guard_through_completion() {
        let database = Arc::new(nomifun_db::init_database_memory().await.unwrap());
        let repo: Arc<dyn IWorkshopRepository> = Arc::new(SqliteWorkshopRepository::new(
            database.pool().clone(),
        ));
        let data_dir = tempfile::tempdir().unwrap();
        let provider_lifecycle = Arc::new(nomifun_common::ProviderLifecycleBarrier::new());
        let service = WorkshopService::start_with_provider_lifecycle(
            data_dir.path(),
            repo,
            provider_lifecycle.clone(),
        );
        let runner = Arc::new(BlockingTemplateDraftRunner::new());
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let owner = CurrentUser {
            id: UserId::parse(owner_id).unwrap(),
            username: "owner".into(),
        };
        let provider_id = ProviderId::new().into_string();
        seed_enabled_chat_model(&database, &provider_id, "chat-model").await;
        let app = workshop_routes(WorkshopRouterState::new(service, runner.clone()))
            .layer(Extension(owner));

        let response_task = tokio::spawn(async move {
            post_template_draft(
                &app,
                serde_json::json!({
                    "prompt": "设计产品海报",
                    "model": { "providerId": provider_id, "model": "chat-model" }
                }),
            )
            .await
        });
        runner.entered.acquire().await.unwrap().forget();

        let mut writer = tokio::spawn(async move {
            let _guard = provider_lifecycle.write().await;
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut writer)
                .await
                .is_err(),
            "destructive provider deletion must wait for the in-flight completion"
        );

        runner.release.add_permits(1);
        let response = response_task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        tokio::time::timeout(std::time::Duration::from_secs(1), writer)
            .await
            .expect("provider deletion guard remained blocked after completion")
            .unwrap();
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn creative_project_agent_kickoff_creates_one_ready_revision_one_document() {
        let (state, owner, _data_dir, database) = test_state_with_database().await;
        let provider_id = ProviderId::new().into_string();
        seed_enabled_chat_model(&database, &provider_id, "chat-model").await;
        let app = workshop_routes(state.clone()).layer(Extension(owner));

        let response = post_create_project(
            &app,
            serde_json::json!({
                "title": "  首页创作  ",
                "agentKickoff": {
                    "prompt": "  规划一张简约海报  ",
                    "model": {
                        "providerId": provider_id,
                        "model": "chat-model"
                    }
                }
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let response = response_json(response).await;
        let project_id = response["data"]["project"]["projectId"]
            .as_str()
            .unwrap();
        assert_eq!(response["data"]["project"]["title"], "首页创作");
        assert_eq!(response["data"]["project"]["revision"], "1");
        assert_eq!(response["data"]["project"]["nodeCount"], 0);
        assert_eq!(response["data"]["project"]["connectionCount"], 0);

        let detail = state.service.get_creative_project(project_id).await.unwrap();
        assert!(detail.document.nodes.is_empty());
        assert!(detail.document.connections.is_empty());
        assert_eq!(detail.document.chat_sessions.len(), 1);
        let chat = &detail.document.chat_sessions[0];
        assert_eq!(detail.document.active_chat_id.as_deref(), Some(chat.id.as_str()));
        assert!(chat.message_ids.is_empty());
        assert_eq!(chat.created_at, chat.updated_at);
        assert_eq!(
            chat.model.as_ref().unwrap(),
            &crate::creative_studio::CreativeChatModel {
                provider_id,
                model: "chat-model".into(),
            }
        );
        let pending = chat.pending_turn.as_ref().unwrap();
        assert_eq!(pending.prompt, "规划一张简约海报");
        assert_eq!(pending.model_input.as_deref(), Some("规划一张简约海报"));
        assert_eq!(pending.skill_ids, ["creative-studio-canvas"]);
        assert_eq!(pending.created_at, chat.created_at);
        assert!(detail.document.panels.right.open);
        assert_eq!(detail.document.panels.right.width, 390.0);
        assert_eq!(
            detail.document.panels.right.active_view,
            crate::creative_studio::CreativeRightView::Assistant
        );
        let project_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM creative_studio_projects",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(project_count, 1);
    }

    #[tokio::test]
    async fn creative_project_agent_kickoff_fails_closed_before_insert() {
        let (state, owner, _data_dir, database) = test_state_with_database().await;
        let provider_id = ProviderId::new().into_string();
        seed_enabled_chat_model(&database, &provider_id, "chat-model").await;
        let request = |prompt: &str, model: &str| {
            serde_json::json!({
                "agentKickoff": {
                    "prompt": prompt,
                    "model": {
                        "providerId": provider_id,
                        "model": model
                    }
                }
            })
        };

        let non_owner = CurrentUser {
            id: UserId::new(),
            username: "not-owner".into(),
        };
        let response = post_create_project(
            &workshop_routes(state.clone()).layer(Extension(non_owner)),
            request("请规划", "chat-model"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let app = workshop_routes(state.clone()).layer(Extension(owner));
        let blank = post_create_project(&app, request(" \r\n ", "chat-model")).await;
        assert_eq!(blank.status(), StatusCode::BAD_REQUEST);
        let too_long_prompt = "x".repeat(65_536 + 1);
        let too_long = post_create_project(&app, request(&too_long_prompt, "chat-model")).await;
        assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);
        let untrimmed_model = post_create_project(&app, request("请规划", " chat-model ")).await;
        assert_eq!(untrimmed_model.status(), StatusCode::BAD_REQUEST);

        let invalid_provider = post_create_project(
            &app,
            serde_json::json!({
                "agentKickoff": {
                    "prompt": "请规划",
                    "model": {
                        "providerId": "not-a-provider-id",
                        "model": "chat-model"
                    }
                }
            }),
        )
        .await;
        assert_eq!(invalid_provider.status(), StatusCode::BAD_REQUEST);

        let missing_exact_model =
            post_create_project(&app, request("请规划", "other-model")).await;
        assert_eq!(missing_exact_model.status(), StatusCode::CONFLICT);

        nomifun_db::sqlx::query(
            "UPDATE provider_models SET enabled = 0 WHERE provider_id = ? AND model = ?",
        )
        .bind(&provider_id)
        .bind("chat-model")
        .execute(database.pool())
        .await
        .unwrap();
        let disabled = post_create_project(&app, request("请规划", "chat-model")).await;
        assert_eq!(disabled.status(), StatusCode::CONFLICT);
        nomifun_db::sqlx::query(
            "UPDATE provider_models SET enabled = 1 WHERE provider_id = ? AND model = ?",
        )
        .bind(&provider_id)
        .bind("chat-model")
        .execute(database.pool())
        .await
        .unwrap();

        nomifun_db::sqlx::query("UPDATE providers SET enabled = 0 WHERE provider_id = ?")
            .bind(&provider_id)
            .execute(database.pool())
            .await
            .unwrap();
        let disabled_provider = post_create_project(&app, request("请规划", "chat-model")).await;
        assert_eq!(disabled_provider.status(), StatusCode::CONFLICT);
        nomifun_db::sqlx::query("UPDATE providers SET enabled = 1 WHERE provider_id = ?")
            .bind(&provider_id)
            .execute(database.pool())
            .await
            .unwrap();

        nomifun_db::sqlx::query(
            "DELETE FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = ? AND task = 'chat'",
        )
        .bind(&provider_id)
        .bind("chat-model")
        .execute(database.pool())
        .await
        .unwrap();
        let no_chat_capability =
            post_create_project(&app, request("请规划", "chat-model")).await;
        assert_eq!(no_chat_capability.status(), StatusCode::CONFLICT);

        let unknown_nested_key = post_create_project(
            &app,
            serde_json::json!({
                "agentKickoff": {
                    "prompt": "请规划",
                    "skillIds": ["creative-studio-canvas"],
                    "model": {
                        "providerId": provider_id,
                        "model": "chat-model"
                    }
                }
            }),
        )
        .await;
        assert_eq!(unknown_nested_key.status(), StatusCode::BAD_REQUEST);

        let project_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM creative_studio_projects",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(project_count, 0);
    }

    #[tokio::test]
    async fn creative_project_handlers_cover_crud_and_revision_conflict() {
        let (state, user, _data_dir) = test_state().await;

        let created = create_creative_project(
            State(state.clone()),
            Extension(user.clone()),
            Ok::<_, JsonRejection>(Json(CreateCreativeProjectRequest {
                title: Some("路由项目".into()),
                agent_kickoff: None,
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
        assert!(document.chat_sessions.is_empty());
        assert!(document.active_chat_id.is_none());
        assert!(!document.panels.right.open);

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
        assert!(matches!(stale, AppError::RevisionConflict(_)));
        let stale_response = stale.into_response();
        assert_eq!(stale_response.status(), StatusCode::CONFLICT);
        let stale_body = axum::body::to_bytes(stale_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let stale_json: Value = serde_json::from_slice(&stale_body).unwrap();
        assert_eq!(stale_json["code"], "REVISION_CONFLICT");

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
    async fn canonical_canvas_routes_use_canvas_wire_and_keep_project_alias() {
        let (state, user, _data_dir) = test_state().await;
        let app = workshop_routes(state.clone()).layer(Extension(user));

        let created = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/canvases")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Canonical Canvas"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created_json = response_json(created).await;
        let canvas_id = created_json["data"]["canvas"]["canvasId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(created_json["data"]["canvas"]["projectId"].is_null());
        assert!(created_json["data"].get("project").is_none());

        let listed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/creative-studio/canvases")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_json = response_json(listed).await;
        assert_eq!(listed_json["data"]["canvases"].as_array().unwrap().len(), 1);
        assert!(listed_json["data"].get("projects").is_none());

        let detail = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/creative-studio/canvases/{canvas_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::OK);
        let detail_json = response_json(detail).await;
        assert_eq!(detail_json["data"]["canvas"]["canvasId"], canvas_id);
        assert_eq!(detail_json["data"]["document"]["canvasId"], canvas_id);
        assert!(detail_json["data"]["document"]["projectId"].is_null());

        let save_body = serde_json::json!({
            "expectedRevision": "1",
            "document": detail_json["data"]["document"],
        });
        let saved = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/api/creative-studio/canvases/{canvas_id}/document"
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&save_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let saved_json = response_json(saved).await;
        assert_eq!(saved_json["data"]["canvas"]["revision"], "2");

        let legacy_document = serde_json::json!({
            "expectedRevision": "2",
            "document": {
                "schema": "nomifun.creative-studio/v1",
                "projectId": canvas_id,
                "viewport": { "x": 0, "y": 0, "zoom": 1 },
                "background": "lines",
                "nodes": [],
                "connections": [],
                "chatSessions": [],
                "activeChatId": null,
                "panels": {
                    "left": { "open": true, "width": 280, "activeView": "canvas" },
                    "right": { "open": false, "width": 360, "activeView": "assistant" },
                    "bottom": { "open": false, "height": 240, "activeView": "history" }
                },
                "pendingTaskIds": []
            }
        });
        let rejected_legacy_wire = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/api/creative-studio/canvases/{canvas_id}/document"
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&legacy_document).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected_legacy_wire.status(), StatusCode::BAD_REQUEST);

        let renamed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/creative-studio/canvases/{canvas_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Renamed Canvas"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(renamed.status(), StatusCode::OK);
        let renamed_json = response_json(renamed).await;
        assert_eq!(renamed_json["data"]["canvas"]["title"], "Renamed Canvas");

        let empty_ops = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/creative-studio/canvases/{canvas_id}/agent-ops"
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"assistantMessageId":"0190f5fe-7c00-7a00-8abc-000000000301","expectedRevision":"2","ops":[]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(empty_ops.status(), StatusCode::BAD_REQUEST);

        let exported = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/creative-studio/canvases/{canvas_id}/archive"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exported.status(), StatusCode::OK);
        let archive_bytes = axum::body::to_bytes(
            exported.into_body(),
            MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES,
        )
        .await
        .unwrap();

        let imported = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/canvases/import")
                    .body(Body::from(archive_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(imported.status(), StatusCode::CREATED);
        let imported_json = response_json(imported).await;
        assert!(imported_json["data"]["canvas"]["canvasId"].is_string());
        assert!(imported_json["data"].get("project").is_none());

        let legacy_list = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/creative-studio/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(legacy_list.status(), StatusCode::OK);
        assert_eq!(
            legacy_list
                .headers()
                .get("deprecation")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        assert_eq!(
            legacy_list
                .headers()
                .get(header::LINK)
                .and_then(|value| value.to_str().ok()),
            Some("</api/creative-studio/canvases>; rel=\"successor-version\"")
        );
        let legacy_json = response_json(legacy_list).await;
        assert_eq!(legacy_json["data"]["projects"].as_array().unwrap().len(), 2);
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
    async fn creative_template_routes_cover_canonical_crud() {
        let (state, user, _data_dir) = test_state().await;
        let app = workshop_routes(state).layer(Extension(user));
        let definition = template_definition();
        let template_id = definition.id.clone();

        let create_body = serde_json::to_vec(&serde_json::json!({ "template": definition }))
            .unwrap();
        let created = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/templates")
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
                    .uri("/api/creative-studio/templates")
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
                    .uri(format!("/api/creative-studio/templates/{template_id}"))
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
        let mut replacement: CreativeTemplateDefinitionV1 = serde_json::from_value(
            detail_json["data"]["template"].clone(),
        )
        .unwrap();
        replacement.revision = 2;
        replacement.metadata.name = "高端海报".into();

        let saved = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!("/api/creative-studio/templates/{template_id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "expectedRevision": "1",
                            "template": replacement,
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
                    .uri(format!("/api/creative-studio/templates/{template_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn creative_template_run_routes_cover_idempotent_create_list_get_and_cas() {
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

        let mut definition = template_definition();
        let variable_id = match &definition.variables[0] {
            CreativeTemplateVariable::Text { id, .. } => id.clone(),
            _ => panic!("template fixture must contain a text variable"),
        };
        if let CreativeTemplateStep::GenerateImages { generation, .. } = &mut definition.steps[1]
        {
            generation.model = Some(crate::template::CreativeTemplateImageModelBinding {
                provider_id,
                model: "image-model".into(),
                task: crate::template::CreativeTemplateImageTask::ImageGeneration,
            });
        }
        let definition = state
            .service
            .create_creative_template(definition)
            .await
            .unwrap();
        let template_id = definition.id.clone();
        let template_run_id = nomifun_common::CreativeStudioTemplateRunId::new().into_string();
        let app = workshop_routes(state).layer(Extension(user));
        let create_json = serde_json::json!({
            "request": {
                "templateRunId": template_run_id,
                "templateId": template_id,
                "templateRevision": 1,
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
                    .uri("/api/creative-studio/template-runs")
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
        let mut run: CreativeTemplateRunAggregateV1 =
            serde_json::from_value(created_json["data"]["run"].clone()).unwrap();

        let replay = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/template-runs")
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
                        "/api/creative-studio/template-runs?templateId={template_id}"
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
                    .uri(format!(
                        "/api/creative-studio/template-runs/{template_run_id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::OK);

        run.revision = 2;
        run.record.status = crate::template_run::CreativeTemplateRunStatus::Queued;
        run.record.task_ids = vec![nomifun_common::generate_id()];
        run.record.queued_at = Some(run.request.requested_at + 1);
        let saved = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/api/creative-studio/template-runs/{template_run_id}"
                    ))
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
    async fn asset_delete_route_removes_content_and_preserves_explicit_history_tombstones() {
        let (state, user, data_dir) = test_state().await;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let unrelated = state
            .service
            .ingest_asset_bytes(png.get_ref().clone(), "image/png", "unrelated", true, None)
            .await
            .unwrap();
        let referenced = state
            .service
            .ingest_asset_bytes(png.into_inner(), "image/png", "history result", true, None)
            .await
            .unwrap();
        let project = state
            .service
            .create_creative_project(Some("history without its source node".into()))
            .await
            .unwrap();
        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id": CreativeStudioNodeId::new().into_string(),
                "type": "config",
                "position": { "x": 0, "y": 0 },
                "size": { "width": 320, "height": 180 },
                "groupId": null,
                "zIndex": 0,
                "locked": false,
                "data": {
                    "task": "image_generation",
                    "capability": "t2i",
                    "providerId": null,
                    "model": null,
                    "prompt": "retained generation history",
                    "negativePrompt": "",
                    "operation": {
                        "kind": "image-node-compose",
                        "sourceNodeId": CreativeStudioNodeId::new().into_string(),
                        "sourceAssetId": null
                    },
                    "parameters": {},
                    "inputAssetIds": [],
                    "taskId": null,
                    "resultAssetIds": [referenced.asset_id],
                    "status": "succeeded",
                    "errorMessage": null
                }
            }))
            .unwrap(),
        );
        let saved = state
            .service
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();
        let app = workshop_routes(state.clone()).layer(Extension(user));

        for asset in [&unrelated, &referenced] {
            assert!(asset.rel_path.is_some());
            assert!(asset.thumb_rel_path.is_some());
            let delete = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("DELETE")
                        .uri(format!("/api/creative-studio/assets/{}", asset.asset_id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(delete.status(), StatusCode::NO_CONTENT);

            let get = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/api/creative-studio/assets/{}", asset.asset_id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(get.status(), StatusCode::OK);
            assert_eq!(get.headers()[header::CACHE_CONTROL], "private, no-store");
            let metadata = response_json(get).await;
            assert!(metadata["data"]["deleted_at"].as_i64().is_some());
            assert_eq!(metadata["data"]["in_library"], false);
            assert!(matches!(state.service.serve_file(&asset.asset_id, false).await,
                Err(AppError::NotFound(_))));
            assert!(matches!(state.service.serve_file(&asset.asset_id, true).await,
                Err(AppError::NotFound(_))));
            for rel_path in [asset.rel_path.as_deref(), asset.thumb_rel_path.as_deref()]
                .into_iter()
                .flatten()
            {
                assert!(!data_dir.path().join(rel_path).exists());
            }
        }

        let after = state
            .service
            .get_creative_project(&project.project_id)
            .await
            .unwrap();
        assert_eq!(after.document, document);
        assert_eq!(after.project.revision, saved.revision);
    }

    #[tokio::test]
    async fn prompt_library_asset_remove_route_soft_hides_and_is_idempotent() {
        let (state, user, _data_dir) = test_state().await;
        let asset = state
            .service
            .create_text_asset(NewTextAsset {
                title: "saved prompt".into(),
                text_content: "body".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
                origin: Some(
                    crate::service::PromptLibraryAssetOrigin {
                        prompt_library_source: "catalog".into(),
                        prompt_library_id: "route-remove-id".into(),
                        prompt_catalog_id: Some("route-remove-id".into()),
                        source_url: None,
                        license: None,
                        license_url: None,
                    }
                    .into(),
                ),
            })
            .await
            .unwrap();
        let app = workshop_routes(state.clone()).layer(Extension(user));
        let body = r#"{"prompt_library_source":"catalog","prompt_library_id":"route-remove-id"}"#;

        for _ in 0..2 {
            let removed = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/api/creative-studio/prompt-library-assets/remove")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(removed.status(), StatusCode::OK);
            assert_eq!(response_json(removed).await["data"]["matched"], 1);
        }

        let unknown = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/prompt-library-assets/remove")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"prompt_library_source":"catalog","prompt_library_id":"missing"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::OK);
        assert_eq!(response_json(unknown).await["data"]["matched"], 0);

        for invalid_body in [
            r#"{"prompt_library_source":"asset","prompt_library_id":"route-remove-id"}"#,
            r#"{"prompt_library_source":"catalog","prompt_library_id":" "}"#,
        ] {
            let invalid = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/api/creative-studio/prompt-library-assets/remove")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(invalid_body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        }

        let forbidden = workshop_routes(state.clone())
            .layer(Extension(CurrentUser {
                id: UserId::new(),
                username: "not-owner".into(),
            }))
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/creative-studio/prompt-library-assets/remove")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        let retained = state.service.get_asset(&asset.asset_id).await.unwrap();
        assert!(!retained.in_library);
        assert_eq!(retained.asset_id, asset.asset_id);
        assert_eq!(retained.text_content.as_deref(), Some("body"));
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

    #[tokio::test]
    async fn asset_file_range_returns_exact_bytes_and_media_headers() {
        let (state, _user, _data_dir) = test_state().await;
        let asset = state.service
            .ingest_asset_bytes(b"0123456789".to_vec(), "video/mp4", "video", true, None)
            .await
            .unwrap();
        let app = workshop_public_routes(state);
        let uri = format!("/api/creative-studio/files/{}", asset.asset_id);
        for (range, status, content_range, expected) in [
            (None, StatusCode::OK, None, "0123456789"),
            (Some("bytes=0-1"), StatusCode::PARTIAL_CONTENT, Some("bytes 0-1/10"), "01"),
            (Some("bytes=7-"), StatusCode::PARTIAL_CONTENT, Some("bytes 7-9/10"), "789"),
            (Some("bytes=-3"), StatusCode::PARTIAL_CONTENT, Some("bytes 7-9/10"), "789"),
            (Some("bytes=-200"), StatusCode::PARTIAL_CONTENT, Some("bytes 0-9/10"), "0123456789"),
            (Some("bytes=8-200"), StatusCode::PARTIAL_CONTENT, Some("bytes 8-9/10"), "89"),
            (Some("bytes=8-999999999999999999999999999999"), StatusCode::PARTIAL_CONTENT, Some("bytes 8-9/10"), "89"),
            (Some("bytes=-999999999999999999999999999999"), StatusCode::PARTIAL_CONTENT, Some("bytes 0-9/10"), "0123456789"),
            (Some("bytes=invalid"), StatusCode::OK, None, "0123456789"),
            (Some("bytes=0-1,4-5"), StatusCode::OK, None, "0123456789"),
            (Some("items=0-1"), StatusCode::OK, None, "0123456789"),
        ] {
            let mut request = axum::http::Request::builder().uri(&uri);
            if let Some(range) = range {
                request = request.header(header::RANGE, range);
            }
            let response = app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status, "range: {range:?}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "video/mp4");
            assert_eq!(response.headers()[header::CACHE_CONTROL], SERVE_CACHE_CONTROL);
            assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
            assert_eq!(response.headers()[header::CONTENT_LENGTH], expected.len().to_string());
            assert_eq!(
                response.headers().get(header::CONTENT_RANGE).map(|value| value.to_str().unwrap()),
                content_range,
            );
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 10).await.unwrap().as_ref(),
                expected.as_bytes(),
                "range: {range:?}",
            );
        }
    }

    #[tokio::test]
    async fn asset_file_range_unsatisfiable_returns_416_and_actual_length() {
        let (state, _user, _data_dir) = test_state().await;
        let asset = state.service
            .ingest_asset_bytes(b"0123456789".to_vec(), "video/mp4", "video", true, None)
            .await
            .unwrap();
        let app = workshop_public_routes(state);
        for range in ["bytes=10-", "bytes=20-30", "bytes=5-2", "bytes=-0", "bytes=999999999999999999999999999999-"] {
            let response = app.clone().oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/creative-studio/files/{}", asset.asset_id))
                    .header(header::RANGE, range)
                    .body(Body::empty())
                    .unwrap(),
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE, "range: {range}");
            assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
            assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
            assert_eq!(response.headers()[header::CACHE_CONTROL], SERVE_CACHE_CONTROL);
            assert!(axum::body::to_bytes(response.into_body(), 10).await.unwrap().is_empty());
        }
        assert!(matches!(parse_asset_byte_range("bytes=0-", 0), AssetByteRange::Unsatisfiable));
        assert!(matches!(parse_asset_byte_range("bytes=-1", 0), AssetByteRange::Unsatisfiable));
    }

    #[tokio::test]
    async fn asset_file_range_head_conditional_and_multiple_headers_keep_full_representation() {
        let (state, _user, _data_dir) = test_state().await;
        let asset = state.service
            .ingest_asset_bytes(b"0123456789".to_vec(), "video/mp4", "video", true, None)
            .await
            .unwrap();
        let app = workshop_public_routes(state);
        let uri = format!("/api/creative-studio/files/{}", asset.asset_id);
        for (method, extra_header) in [
            (Method::HEAD, None),
            (Method::GET, Some((header::IF_RANGE, "\"unmatched-validator\""))),
            (Method::GET, Some((header::RANGE, "bytes=6-7"))),
        ] {
            let mut request = axum::http::Request::builder()
                .method(method.clone())
                .uri(&uri)
                .header(header::RANGE, "bytes=0-1");
            if let Some((name, value)) = extra_header {
                request = request.header(name, value);
            }
            let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
            assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
            assert!(response.headers().get(header::CONTENT_RANGE).is_none());
            let body = axum::body::to_bytes(response.into_body(), 10).await.unwrap();
            assert_eq!(body.as_ref(), if method == Method::HEAD { b"".as_slice() } else { b"0123456789".as_slice() });
        }
    }

    #[tokio::test]
    async fn asset_file_range_uses_thumbnail_representation_and_preserves_deleted_guard() {
        let (state, _user, _data_dir) = test_state().await;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 4)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let asset = state.service
            .ingest_asset_bytes(png.into_inner(), "image/png", "image", true, None)
            .await
            .unwrap();
        let thumbnail = state.service.serve_file(&asset.asset_id, true).await.unwrap();
        let app = workshop_public_routes(state.clone());
        let uri = format!("/api/creative-studio/files/{}", asset.asset_id);
        let response = app.clone().oneshot(
            axum::http::Request::builder()
                .uri(format!("{uri}?thumb=1"))
                .header(header::RANGE, "bytes=0-1")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(response.headers()[header::CONTENT_RANGE], format!("bytes 0-1/{}", thumbnail.bytes.len()));
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "2");
        assert_eq!(axum::body::to_bytes(response.into_body(), 2).await.unwrap().as_ref(), &thumbnail.bytes[..2]);

        state.service.delete_asset_content(&asset.asset_id).await.unwrap();
        for path in [uri.clone(), format!("{uri}?thumb=1"), format!("/api/creative-studio/files/{}", WorkshopAssetId::new())] {
            let response = app.clone().oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .header(header::RANGE, "bytes=0-1")
                    .body(Body::empty())
                    .unwrap(),
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(response.headers().get(header::CONTENT_RANGE).is_none());
        }
    }
}
