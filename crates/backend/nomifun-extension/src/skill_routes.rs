use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, Path as AxumPath, State};
use axum::routing::{delete, get, post, put};

use nomifun_api_types::{
    AddExternalPathRequest, ApiResponse, BuiltinAutoSkillResponse, ExportSkillRequest, ExternalSkillSourceResponse,
    ImportSkillRequest, ImportSkillResponse, MaterializeSkillsRequest, MaterializeSkillsResponse, MaterializedSkillRef,
    NamedPathResponse, ReadPresetRuleRequest, ReadBuiltinResourceRequest, ReadSkillInfoRequest,
    ReadSkillInfoResponse, RemoveExternalPathRequest, ScanForSkillsRequest, ScanForSkillsResponse,
    ScannedSkillResponse, SetSkillTagsRequest, SkillListItemResponse, SkillMarketItemResponse,
    SkillMarketMcpConfigRequest, SkillMarketMcpConfigResponse, SkillMarketPackageRequest,
    SkillMarketPackageInstallError, SkillMarketPackageInstallResponse, SkillMarketPackageResponse,
    SkillMarketSyncRequest, SkillMarketSyncResponse, SkillPathsResponse, SkillSourceResponse,
    WritePresetRuleRequest,
};
use nomifun_common::AppError;
use nomifun_db::ISkillTagRepository;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue};

use crate::classifier::PresetRuleDispatcher;
use crate::external_paths::ExternalPathsManager;
use crate::skill_service::{self, SkillPaths, SkillSource};

fn to_source_response(source: SkillSource) -> SkillSourceResponse {
    match source {
        SkillSource::Builtin => SkillSourceResponse::Builtin,
        SkillSource::Custom => SkillSourceResponse::Custom,
        SkillSource::Extension => SkillSourceResponse::Extension,
    }
}

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for skill/rule route handlers.
#[derive(Clone)]
pub struct SkillRouterState {
    pub skill_paths: SkillPaths,
    pub external_paths_manager: Arc<ExternalPathsManager>,
    /// Optional dispatcher that routes preset-rule / preset-skill
    /// read/write/delete by source (builtin / extension / user). When
    /// `None`, the legacy user-directory-only behavior is preserved.
    #[allow(clippy::type_complexity)]
    pub preset_dispatcher: Option<Arc<dyn PresetRuleDispatcher>>,
    /// Per-skill tag assignment repo (user assignments/overrides).
    pub skill_tag_repo: Arc<dyn ISkillTagRepository>,
    /// Built-in skill tag seed: skill name → (audience_tags, scenario_tags).
    pub builtin_skill_tags: Arc<HashMap<String, (Vec<String>, Vec<String>)>>,
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the skill router with all `/api/skills/*` routes.
///
/// All routes require authentication (applied by the caller).
pub fn skill_routes(state: SkillRouterState) -> Router {
    Router::new()
        // Skill listing & info
        .route("/api/skills", get(list_skills))
        .route("/api/skills/builtin-auto", get(list_builtin_auto_skills))
        .route("/api/skills/{name}/tags", put(set_skill_tags))
        .route("/api/skills/info", post(read_skill_info))
        .route("/api/skills/paths", get(get_skill_paths))
        // Import / export / delete
        .route("/api/skills/import", post(import_skill))
        .route("/api/skills/import-symlink", post(import_skill_symlink))
        .route("/api/skills/export-symlink", post(export_skill_symlink))
        .route("/api/skills/{name}", delete(delete_skill))
        // Scanning & discovery
        .route("/api/skills/scan", post(scan_for_skills))
        .route("/api/skills/detect-paths", get(detect_paths))
        .route("/api/skills/detect-external", get(detect_external))
        // Built-in resources
        .route("/api/skills/builtin-rule", post(read_builtin_rule))
        .route("/api/skills/builtin-skill", post(read_builtin_skill))
        // Per-agent skill resolution (for agent CLI symlink layout).
        .route("/api/skills/materialize-for-agent", post(materialize_for_agent))
        // Preset rules CRUD
        .route("/api/skills/preset-rule/read", post(read_preset_rule))
        .route("/api/skills/preset-rule/write", post(write_preset_rule))
        .route("/api/skills/preset-rule/{id}", delete(delete_preset_rule))
        // Preset skills CRUD
        .route("/api/skills/preset-skill/read", post(read_preset_skill))
        .route("/api/skills/preset-skill/write", post(write_preset_skill))
        .route("/api/skills/preset-skill/{id}", delete(delete_preset_skill))
        // External path management
        .route(
            "/api/skills/external-paths",
            get(get_external_paths)
                .post(add_external_path)
                .delete(remove_external_path),
        )
        // Skills market
        .route("/api/skills/market/enable", post(enable_skills_market))
        .route("/api/skills/market/disable", post(disable_skills_market))
        .route(
            "/api/skills/market/rankings/sync",
            post(sync_skill_market_rankings),
        )
        .route(
            "/api/skills/market/mcp/config",
            post(resolve_skill_market_mcp_config),
        )
        .route(
            "/api/skills/market/package",
            post(resolve_skill_market_package),
        )
        .route(
            "/api/skills/market/package/install",
            post(install_skill_market_package),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Skill listing & info
// ---------------------------------------------------------------------------

/// `GET /api/skills` — list all available skills.
async fn list_skills(
    State(state): State<SkillRouterState>,
) -> Result<Json<ApiResponse<Vec<SkillListItemResponse>>>, AppError> {
    let items = skill_service::list_available_skills(&state.skill_paths).await?;
    let builtin_display = skill_service::load_builtin_skill_display_metadata();
    // user sidecar assignments (decode JSON arrays), keyed by skill name
    let user_rows = state.skill_tag_repo.get_all().await.map_err(AppError::from)?;
    let mut user_map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    for r in user_rows {
        let aud = decode_tags(r.audience_tags.as_deref());
        let scn = decode_tags(r.scenario_tags.as_deref());
        user_map.insert(r.skill_name, (aud, scn));
    }
    let resp: Vec<SkillListItemResponse> = items
        .into_iter()
        .map(|s| {
            let display = if s.source == skill_service::SkillSource::Builtin {
                builtin_display.get(&s.name).cloned().unwrap_or_default()
            } else {
                Default::default()
            };
            let (audience_tags, scenario_tags) = user_map
                .get(&s.name)
                .cloned()
                .or_else(|| state.builtin_skill_tags.get(&s.name).cloned())
                .unwrap_or_default();
            SkillListItemResponse {
                name: s.name,
                description: s.description,
                name_i18n: display.name_i18n,
                description_i18n: display.description_i18n,
                location: s.location,
                relative_location: s.relative_location,
                is_custom: s.is_custom,
                source: to_source_response(s.source),
                audience_tags,
                scenario_tags,
            }
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

/// Decode a JSON-array TEXT column into a `Vec<String>`. Fail-soft on purpose
/// (intentionally unlike `nomifun-preset`'s `decode_str_list`, which 500s on
/// bad JSON): this is the read path for the skill list, so one corrupted sidecar
/// row must not break the whole listing — it degrades to no tags for that skill.
fn decode_tags(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `PUT /api/skills/{name}/tags` — set a skill's tag assignment (user sidecar).
async fn set_skill_tags(
    State(state): State<SkillRouterState>,
    AxumPath(name): AxumPath<String>,
    body: Result<Json<SetSkillTagsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let aud = serde_json::to_string(&req.audience_tags).map_err(|e| AppError::Internal(e.to_string()))?;
    let scn = serde_json::to_string(&req.scenario_tags).map_err(|e| AppError::Internal(e.to_string()))?;
    state
        .skill_tag_repo
        .upsert(&nomifun_db::UpsertSkillTagParams {
            skill_name: &name,
            audience_tags: Some(&aud),
            scenario_tags: Some(&scn),
        })
        .await
        .map_err(AppError::from)?;
    Ok(Json(ApiResponse::success()))
}

/// `GET /api/skills/builtin-auto` — list auto-injected built-in skills.
async fn list_builtin_auto_skills(
    State(state): State<SkillRouterState>,
) -> Result<Json<ApiResponse<Vec<BuiltinAutoSkillResponse>>>, AppError> {
    let items = skill_service::list_builtin_auto_skills(&state.skill_paths).await?;
    let builtin_display = skill_service::load_builtin_skill_display_metadata();
    let resp: Vec<BuiltinAutoSkillResponse> = items
        .into_iter()
        .map(|s| {
            let display = builtin_display.get(&s.name).cloned().unwrap_or_default();
            BuiltinAutoSkillResponse {
                name: s.name,
                description: s.description,
                name_i18n: display.name_i18n,
                description_i18n: display.description_i18n,
                location: s.location,
            }
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

/// `POST /api/skills/info` — read skill info without importing.
async fn read_skill_info(
    body: Result<Json<ReadSkillInfoRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ReadSkillInfoResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let (name, description) = skill_service::read_skill_info(Path::new(&req.skill_path)).await?;
    Ok(Json(ApiResponse::ok(ReadSkillInfoResponse { name, description })))
}

/// `GET /api/skills/paths` — get user and built-in skill directories.
async fn get_skill_paths(
    State(state): State<SkillRouterState>,
) -> Result<Json<ApiResponse<SkillPathsResponse>>, AppError> {
    let (user_dir, builtin_dir) = skill_service::get_skill_paths(&state.skill_paths);
    Ok(Json(ApiResponse::ok(SkillPathsResponse {
        user_skills_dir: user_dir,
        builtin_skills_dir: builtin_dir,
    })))
}

// ---------------------------------------------------------------------------
// Import / export / delete
// ---------------------------------------------------------------------------

/// `POST /api/skills/import` — import a skill by copying.
async fn import_skill(
    State(state): State<SkillRouterState>,
    body: Result<Json<ImportSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ImportSkillResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let name = skill_service::import_skill(&state.skill_paths, Path::new(&req.skill_path)).await?;
    Ok(Json(ApiResponse::ok(ImportSkillResponse {
        skill_name: name.clone(),
        skill_names: vec![name],
    })))
}

/// `POST /api/skills/import-symlink` — import a skill by symlink.
async fn import_skill_symlink(
    State(state): State<SkillRouterState>,
    body: Result<Json<ImportSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ImportSkillResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let names = skill_service::import_skills_with_symlink(&state.skill_paths, Path::new(&req.skill_path)).await?;
    let first_name = names.first().cloned().unwrap_or_default();
    Ok(Json(ApiResponse::ok(ImportSkillResponse {
        skill_name: first_name,
        skill_names: names,
    })))
}

/// `POST /api/skills/export-symlink` — export a skill symlink.
async fn export_skill_symlink(
    body: Result<Json<ExportSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    skill_service::export_skill_with_symlink(Path::new(&req.skill_path), Path::new(&req.target_dir)).await?;
    Ok(Json(ApiResponse::success()))
}

/// `DELETE /api/skills/:name` — delete a user-custom skill.
async fn delete_skill(
    State(state): State<SkillRouterState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    skill_service::delete_skill(&state.skill_paths, &name).await?;
    Ok(Json(ApiResponse::success()))
}

// ---------------------------------------------------------------------------
// Scanning & discovery
// ---------------------------------------------------------------------------

/// `POST /api/skills/scan` — scan a directory for skills.
async fn scan_for_skills(
    body: Result<Json<ScanForSkillsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ScanForSkillsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let skills = skill_service::scan_for_skills(Path::new(&req.folder_path)).await?;
    let resp = ScanForSkillsResponse {
        skills: skills
            .into_iter()
            .map(|s| ScannedSkillResponse {
                name: s.name,
                description: s.description,
                path: s.path,
            })
            .collect(),
    };
    Ok(Json(ApiResponse::ok(resp)))
}

/// `GET /api/skills/detect-paths` — detect common skill paths.
async fn detect_paths() -> Result<Json<ApiResponse<Vec<NamedPathResponse>>>, AppError> {
    let paths = skill_service::detect_common_skill_paths().await;
    let resp: Vec<NamedPathResponse> = paths
        .into_iter()
        .map(|p| NamedPathResponse {
            name: p.name,
            path: p.path,
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

/// `GET /api/skills/detect-external` — discover external skills from all sources.
async fn detect_external(
    State(state): State<SkillRouterState>,
) -> Result<Json<ApiResponse<Vec<ExternalSkillSourceResponse>>>, AppError> {
    let custom = state.external_paths_manager.get_custom_external_paths().await;
    let sources = skill_service::detect_and_count_external_skills(&custom).await;
    let resp: Vec<ExternalSkillSourceResponse> = sources
        .into_iter()
        .map(|s| ExternalSkillSourceResponse {
            name: s.name,
            path: s.path,
            source: s.source,
            skill_count: s.skill_count,
            skills: s
                .skills
                .into_iter()
                .map(|sk| ScannedSkillResponse {
                    name: sk.name,
                    description: sk.description,
                    path: sk.path,
                })
                .collect(),
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

// ---------------------------------------------------------------------------
// Built-in resources
// ---------------------------------------------------------------------------

/// `POST /api/skills/builtin-rule` — read a built-in rule file.
async fn read_builtin_rule(
    State(state): State<SkillRouterState>,
    body: Result<Json<ReadBuiltinResourceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let content = skill_service::read_builtin_rule(&state.skill_paths, &req.file_name).await?;
    Ok(Json(ApiResponse::ok(content)))
}

/// `POST /api/skills/builtin-skill` — read a built-in skill file.
async fn read_builtin_skill(
    State(state): State<SkillRouterState>,
    body: Result<Json<ReadBuiltinResourceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let content = skill_service::read_builtin_skill(&state.skill_paths, &req.file_name).await?;
    Ok(Json(ApiResponse::ok(content)))
}

/// `POST /api/skills/materialize-for-agent` — resolve each requested skill
/// name to its on-disk source directory. The frontend symlinks each
/// returned `source_path` into the agent CLI's native skills dir. The
/// backend no longer copies any files per-conversation.
async fn materialize_for_agent(
    State(state): State<SkillRouterState>,
    body: Result<Json<MaterializeSkillsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<MaterializeSkillsResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let conversation_id = req.conversation_id.into_string();
    let resolved =
        skill_service::materialize_skills_for_agent(&state.skill_paths, &conversation_id, &req.skills).await?;
    let skills: Vec<MaterializedSkillRef> = resolved
        .into_iter()
        .map(|s| MaterializedSkillRef {
            name: s.name,
            source_path: s.source_path.to_string_lossy().into_owned(),
        })
        .collect();
    Ok(Json(ApiResponse::ok(MaterializeSkillsResponse { skills })))
}

// ---------------------------------------------------------------------------
// Preset rules CRUD
// ---------------------------------------------------------------------------

/// `POST /api/skills/preset-rule/read` — read an preset rule.
///
/// Dispatches by source via [`PresetRuleDispatcher`] when wired; falls
/// back to user-directory-only legacy behavior otherwise.
async fn read_preset_rule(
    State(state): State<SkillRouterState>,
    body: Result<Json<ReadPresetRuleRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(dispatcher) = &state.preset_dispatcher {
        let content = dispatcher.read_rule(&req.preset_id, req.locale.as_deref()).await?;
        return Ok(Json(ApiResponse::ok(content)));
    }
    let content =
        skill_service::read_preset_rule(&state.skill_paths, &req.preset_id, req.locale.as_deref()).await?;
    Ok(Json(ApiResponse::ok(content)))
}

/// `POST /api/skills/preset-rule/write` — write an preset rule.
///
/// Dispatches by source: builtin / extension ids reject with 400.
async fn write_preset_rule(
    State(state): State<SkillRouterState>,
    body: Result<Json<WritePresetRuleRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(dispatcher) = &state.preset_dispatcher {
        dispatcher
            .write_rule(&req.preset_id, req.locale.as_deref(), &req.content)
            .await?;
        return Ok(Json(ApiResponse::ok(true)));
    }
    let ok = skill_service::write_preset_rule(
        &state.skill_paths,
        &req.preset_id,
        &req.content,
        req.locale.as_deref(),
    )
    .await?;
    Ok(Json(ApiResponse::ok(ok)))
}

/// `DELETE /api/skills/preset-rule/:id` — delete all locale versions.
async fn delete_preset_rule(
    State(state): State<SkillRouterState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    if let Some(dispatcher) = &state.preset_dispatcher {
        let ok = dispatcher.delete_rule(&id).await?;
        return Ok(Json(ApiResponse::ok(ok)));
    }
    let ok = skill_service::delete_preset_rule(&state.skill_paths, &id).await?;
    Ok(Json(ApiResponse::ok(ok)))
}

// ---------------------------------------------------------------------------
// Preset skills CRUD
// ---------------------------------------------------------------------------

/// `POST /api/skills/preset-skill/read` — read an preset skill.
///
/// Dispatches by source via [`PresetRuleDispatcher`] when wired.
async fn read_preset_skill(
    State(state): State<SkillRouterState>,
    body: Result<Json<ReadPresetRuleRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(dispatcher) = &state.preset_dispatcher {
        let content = dispatcher.read_skill(&req.preset_id, req.locale.as_deref()).await?;
        return Ok(Json(ApiResponse::ok(content)));
    }
    let content =
        skill_service::read_preset_skill(&state.skill_paths, &req.preset_id, req.locale.as_deref()).await?;
    Ok(Json(ApiResponse::ok(content)))
}

/// `POST /api/skills/preset-skill/write` — write an preset skill.
///
/// Dispatches by source: builtin / extension ids reject with 400.
async fn write_preset_skill(
    State(state): State<SkillRouterState>,
    body: Result<Json<WritePresetRuleRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(dispatcher) = &state.preset_dispatcher {
        dispatcher
            .write_skill(&req.preset_id, req.locale.as_deref(), &req.content)
            .await?;
        return Ok(Json(ApiResponse::ok(true)));
    }
    let ok = skill_service::write_preset_skill(
        &state.skill_paths,
        &req.preset_id,
        &req.content,
        req.locale.as_deref(),
    )
    .await?;
    Ok(Json(ApiResponse::ok(ok)))
}

/// `DELETE /api/skills/preset-skill/:id` — delete all locale versions.
async fn delete_preset_skill(
    State(state): State<SkillRouterState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    if let Some(dispatcher) = &state.preset_dispatcher {
        let ok = dispatcher.delete_skill(&id).await?;
        return Ok(Json(ApiResponse::ok(ok)));
    }
    let ok = skill_service::delete_preset_skill(&state.skill_paths, &id).await?;
    Ok(Json(ApiResponse::ok(ok)))
}

// ---------------------------------------------------------------------------
// External path management
// ---------------------------------------------------------------------------

/// `GET /api/skills/external-paths` — list custom external paths.
async fn get_external_paths(
    State(state): State<SkillRouterState>,
) -> Result<Json<ApiResponse<Vec<NamedPathResponse>>>, AppError> {
    let paths = state.external_paths_manager.get_custom_external_paths().await;
    let resp: Vec<NamedPathResponse> = paths
        .into_iter()
        .map(|p| NamedPathResponse {
            name: p.name,
            path: p.path,
        })
        .collect();
    Ok(Json(ApiResponse::ok(resp)))
}

/// `POST /api/skills/external-paths` — add a custom external path.
async fn add_external_path(
    State(state): State<SkillRouterState>,
    body: Result<Json<AddExternalPathRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .external_paths_manager
        .add_custom_external_path(&req.name, &req.path)
        .await?;
    Ok(Json(ApiResponse::success()))
}

/// `DELETE /api/skills/external-paths` — remove a custom external path.
async fn remove_external_path(
    State(state): State<SkillRouterState>,
    body: Result<Json<RemoveExternalPathRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .external_paths_manager
        .remove_custom_external_path(&req.path)
        .await?;
    Ok(Json(ApiResponse::success()))
}

// ---------------------------------------------------------------------------
// Skills market
// ---------------------------------------------------------------------------

/// `POST /api/skills/market/enable` — enable the nomifun skills market.
async fn enable_skills_market(State(state): State<SkillRouterState>) -> Result<Json<ApiResponse<()>>, AppError> {
    state.external_paths_manager.enable_skills_market().await?;
    Ok(Json(ApiResponse::success()))
}

/// `POST /api/skills/market/disable` — disable the nomifun skills market.
async fn disable_skills_market(State(state): State<SkillRouterState>) -> Result<Json<ApiResponse<()>>, AppError> {
    state.external_paths_manager.disable_skills_market().await?;
    Ok(Json(ApiResponse::success()))
}

async fn sync_skill_market_rankings(
    body: Result<Json<SkillMarketSyncRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SkillMarketSyncResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let resp = fetch_skill_market_rankings(req.sources).await?;
    Ok(Json(ApiResponse::ok(resp)))
}

async fn resolve_skill_market_mcp_config(
    body: Result<Json<SkillMarketMcpConfigRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SkillMarketMcpConfigResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let config_json = resolve_market_mcp_config(req).await?;
    Ok(Json(ApiResponse::ok(SkillMarketMcpConfigResponse { config_json })))
}

async fn resolve_skill_market_package(
    body: Result<Json<SkillMarketPackageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SkillMarketPackageResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let package = resolve_market_package(req).await?;
    Ok(Json(ApiResponse::ok(package)))
}

async fn install_skill_market_package(
    State(state): State<SkillRouterState>,
    body: Result<Json<SkillMarketPackageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SkillMarketPackageInstallResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let package = resolve_market_package(req).await?;
    let install_result = install_skillhub_package_skills(&state.skill_paths, &package.skill_slugs).await?;
    Ok(Json(ApiResponse::ok(SkillMarketPackageInstallResponse {
        package,
        installed_skill_names: install_result.installed_skill_names,
        errors: install_result.errors,
    })))
}

const CLAWHUB_SOURCE: &str = "clawhub";
const SKILLHUB_SOURCE: &str = "skillhub";
const LOOPHUB_SOURCE: &str = "loophub";
const SKILLHUB_MCP_SOURCE: &str = "skillhub_mcp";
const MCPWORLD_SOURCE: &str = "mcpworld";
const CLAWHUB_PLUGINS_SOURCE: &str = "clawhub_plugins";
const SKILLHUB_PACKAGES_SOURCE: &str = "skillhub_packages";
const CLAWHUB_RANKING_URL: &str = "https://clawhub.ai/skills?tab=new";
const CLAWHUB_CONVEX_QUERY_URL: &str = "https://wry-manatee-359.convex.cloud/api/query";
const SKILLHUB_RANKING_URL: &str = "https://api.skillhub.cn/api/skills?page=1&pageSize=100&sortBy=score&order=desc";
const SKILLHUB_HTML_FALLBACK_URL: &str = "https://www.skills.sh/trending/";
const LOOPHUB_RANKING_URL: &str =
    "https://api.cocoloop.cn/api/v1/store/skills?page=1&page_size=100&sort=downloads&tab=overall";
const SKILLHUB_MCP_RANKING_URL: &str =
    "https://api.skillhub.cn/api/v1/mcp/servers?page=1&pageSize=100&sortBy=updated_at&order=desc";
const MCPWORLD_RANKING_URL: &str =
    "https://www.mcpworld.com/api/mcp-market/servers?wd=most_popular&type=tag&pn=0&lg=zh&pl=100";
const CLAWHUB_PLUGINS_API_URL: &str = "https://clawhub.ai/api/v1/plugins?limit=100&sort=recommended";
const CLAWHUB_PLUGINS_URL: &str = "https://clawhub.ai/plugins";
const SKILLHUB_PACKAGES_URL: &str = "https://api.skillhub.cn/api/v1/skillsets?page=1&pageSize=200";
const SKILLHUB_SKILL_DOWNLOAD_URL: &str = "https://api.skillhub.cn/api/v1/download";
const SKILLHUB_SKILL_SEARCH_URL: &str = "https://api.skillhub.cn/api/v1/search";
const MAX_MARKET_BODY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SKILLHUB_SKILL_ZIP_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MARKET_ITEMS_PER_SOURCE: usize = 200;
const MARKET_SOURCE_TIMEOUT: Duration = Duration::from_secs(18);

async fn fetch_skill_market_rankings(sources: Vec<String>) -> Result<SkillMarketSyncResponse, AppError> {
    let selected = normalize_market_sources(sources)?;
    let client = build_market_client()?;

    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut tasks = tokio::task::JoinSet::new();
    for source in selected {
        let client = client.clone();
        tasks.spawn(async move { fetch_market_source_with_timeout(&client, source).await });
    }

    while let Some(joined) = tasks.join_next().await {
        let (source, result) = joined.map_err(|e| AppError::Internal(format!("skill market task failed: {e}")))?;
        match result {
            Ok(mut source_items) => items.append(&mut source_items),
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }

    Ok(SkillMarketSyncResponse {
        fetched_at: now_epoch_ms(),
        items,
        errors,
    })
}

async fn fetch_market_source_with_timeout(
    client: &reqwest::Client,
    source: &'static str,
) -> (&'static str, Result<Vec<SkillMarketItemResponse>, AppError>) {
    let result = match tokio::time::timeout(MARKET_SOURCE_TIMEOUT, fetch_market_source(client, source)).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Timeout(format!(
            "skill market source timed out after {}s",
            MARKET_SOURCE_TIMEOUT.as_secs()
        ))),
    };
    (source, result)
}

fn normalize_market_sources(sources: Vec<String>) -> Result<Vec<&'static str>, AppError> {
    if sources.is_empty() {
        return Ok(vec![
            CLAWHUB_SOURCE,
            LOOPHUB_SOURCE,
            SKILLHUB_SOURCE,
            SKILLHUB_MCP_SOURCE,
            MCPWORLD_SOURCE,
            CLAWHUB_PLUGINS_SOURCE,
            SKILLHUB_PACKAGES_SOURCE,
        ]);
    }

    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for source in sources {
        let normalized = source.trim().to_ascii_lowercase();
        let source = match normalized.as_str() {
            CLAWHUB_SOURCE => CLAWHUB_SOURCE,
            SKILLHUB_SOURCE => SKILLHUB_SOURCE,
            LOOPHUB_SOURCE => LOOPHUB_SOURCE,
            SKILLHUB_MCP_SOURCE => SKILLHUB_MCP_SOURCE,
            MCPWORLD_SOURCE => MCPWORLD_SOURCE,
            CLAWHUB_PLUGINS_SOURCE => CLAWHUB_PLUGINS_SOURCE,
            SKILLHUB_PACKAGES_SOURCE => SKILLHUB_PACKAGES_SOURCE,
            other => return Err(AppError::BadRequest(format!("unsupported skill market source: {other}"))),
        };
        if seen.insert(source) {
            selected.push(source);
        }
    }
    Ok(selected)
}

async fn fetch_market_source(
    client: &reqwest::Client,
    source: &'static str,
) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    let url = match source {
        CLAWHUB_SOURCE => CLAWHUB_RANKING_URL,
        SKILLHUB_SOURCE => SKILLHUB_RANKING_URL,
        LOOPHUB_SOURCE => LOOPHUB_RANKING_URL,
        SKILLHUB_MCP_SOURCE => SKILLHUB_MCP_RANKING_URL,
        MCPWORLD_SOURCE => MCPWORLD_RANKING_URL,
        CLAWHUB_PLUGINS_SOURCE => CLAWHUB_PLUGINS_URL,
        SKILLHUB_PACKAGES_SOURCE => SKILLHUB_PACKAGES_URL,
        _ => return Err(AppError::BadRequest("unsupported skill market source".into())),
    };

    match source {
        CLAWHUB_SOURCE => fetch_clawhub_rankings(client).await,
        SKILLHUB_SOURCE => fetch_skillhub_rankings(client).await,
        CLAWHUB_PLUGINS_SOURCE => fetch_clawhub_plugins(client).await,
        _ => {
            let body = read_market_body(client, url).await?;
            Ok(match source {
                LOOPHUB_SOURCE => parse_loophub_rankings(&body),
                SKILLHUB_MCP_SOURCE => parse_skillhub_mcp_rankings(&body),
                MCPWORLD_SOURCE => parse_mcpworld_rankings(&body),
                SKILLHUB_PACKAGES_SOURCE => parse_skillhub_packages(&body),
                _ => Vec::new(),
            })
        }
    }
}

async fn fetch_clawhub_rankings(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    let primary = read_market_json_post(
        client,
        CLAWHUB_CONVEX_QUERY_URL,
        serde_json::json!({
            "path": "skills:listPublicPageV4",
            "format": "convex_encoded_json",
            "args": [{
                "dir": "desc",
                "numItems": 100,
                "sort": "newest"
            }]
        }),
    )
    .await
    .map(|body| parse_clawhub_rankings(&body));
    if let Ok(items) = &primary {
        if !items.is_empty() {
            return primary;
        }
    }

    let fallback = read_market_body(client, CLAWHUB_RANKING_URL)
        .await
        .map(|body| parse_clawhub_rankings(&body));
    match (primary, fallback) {
        (_, Ok(items)) if !items.is_empty() => Ok(items),
        (Ok(items), _) => Ok(items),
        (Err(primary_error), Ok(_)) => Err(primary_error),
        (Err(_), Err(fallback_error)) => Err(fallback_error),
    }
}

async fn fetch_skillhub_rankings(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    let primary = read_market_body(client, SKILLHUB_RANKING_URL)
        .await
        .map(|body| parse_skillhub_rankings(&body));
    if let Ok(items) = &primary {
        if !items.is_empty() {
            return primary;
        }
    }

    let fallback = read_market_body(client, SKILLHUB_HTML_FALLBACK_URL)
        .await
        .map(|body| parse_skillhub_rankings(&body));
    match (primary, fallback) {
        (_, Ok(items)) if !items.is_empty() => Ok(items),
        (Ok(items), _) => Ok(items),
        (Err(primary_error), Ok(_)) => Err(primary_error),
        (Err(_), Err(fallback_error)) => Err(fallback_error),
    }
}

async fn fetch_clawhub_plugins(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    let primary = read_market_body(client, CLAWHUB_PLUGINS_API_URL)
        .await
        .map(|body| parse_clawhub_plugins(&body));
    if let Ok(items) = &primary {
        if !items.is_empty() {
            return primary;
        }
    }

    let fallback = read_market_body(client, CLAWHUB_PLUGINS_URL)
        .await
        .map(|body| parse_clawhub_plugins(&body));
    match (primary, fallback) {
        (_, Ok(items)) if !items.is_empty() => Ok(items),
        (Ok(items), _) => Ok(items),
        (Err(primary_error), Ok(_)) => Err(primary_error),
        (Err(_), Err(fallback_error)) => Err(fallback_error),
    }
}

fn build_market_client() -> Result<reqwest::Client, AppError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8"),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));

    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(12))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 NomiFun-SkillMarket/1.0",
        )
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn read_market_body(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let mut response = client.get(url).send().await.map_err(map_market_fetch_error)?;
    read_market_response(&mut response).await
}

async fn read_market_json_post(
    client: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> Result<String, AppError> {
    let mut response = client.post(url).json(&body).send().await.map_err(map_market_fetch_error)?;
    read_market_response(&mut response).await
}

async fn read_market_response(response: &mut reqwest::Response) -> Result<String, AppError> {
    if !response.status().is_success() {
        return Err(AppError::BadGateway(format!("market page returned {}", response.status())));
    }
    if response.content_length().unwrap_or(0) > MAX_MARKET_BODY_BYTES {
        return Err(AppError::BadGateway("market response is too large".into()));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_market_fetch_error)? {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_MARKET_BODY_BYTES {
            return Err(AppError::BadGateway("market response is too large".into()));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn read_market_bytes(
    response: &mut reqwest::Response,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!("{label} not found")));
    }
    if !status.is_success() {
        return Err(AppError::BadGateway(format!("{label} returned {status}")));
    }
    if response.content_length().unwrap_or(0) > max_bytes {
        return Err(AppError::BadGateway(format!("{label} is too large")));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_market_fetch_error)? {
        if bytes.len().saturating_add(chunk.len()) as u64 > max_bytes {
            return Err(AppError::BadGateway(format!("{label} is too large")));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}

async fn resolve_market_mcp_config(req: SkillMarketMcpConfigRequest) -> Result<serde_json::Value, AppError> {
    let client = build_market_client()?;
    match req.source.as_str() {
        SKILLHUB_MCP_SOURCE => {
            let slug = market_ref_suffix(&req.id, SKILLHUB_MCP_SOURCE)
                .or_else(|| last_url_segment(&req.url))
                .ok_or_else(|| AppError::BadRequest("invalid SkillHub MCP market id".into()))?;
            if !is_market_slug(&slug) {
                return Err(AppError::BadRequest("invalid SkillHub MCP slug".into()));
            }
            let body = read_market_body(
                &client,
                &format!("https://api.skillhub.cn/api/v1/mcp/servers/{slug}/readme"),
            )
            .await?;
            extract_mcp_config_from_markdown(&body)
                .ok_or_else(|| AppError::BadGateway("MCP config block not found".into()))
        }
        MCPWORLD_SOURCE => {
            let id = market_ref_suffix(&req.id, MCPWORLD_SOURCE)
                .or_else(|| last_url_segment(&req.url))
                .ok_or_else(|| AppError::BadRequest("invalid MCPWorld market id".into()))?;
            if !is_market_slug(&id) {
                return Err(AppError::BadRequest("invalid MCPWorld id".into()));
            }
            let body = read_market_body(
                &client,
                &format!("https://www.mcpworld.com/api/mcp-market/server/detail?id={id}&lg=zh"),
            )
            .await?;
            let value = serde_json::from_str::<serde_json::Value>(&body)
                .map_err(|e| AppError::BadGateway(format!("MCPWorld detail JSON parse failed: {e}")))?;
            let detail_text = value
                .pointer("/data/detail/abstract")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.get("value").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            extract_mcp_config_from_markdown(&detail_text)
                .ok_or_else(|| AppError::BadGateway("MCP config block not found".into()))
        }
        other => Err(AppError::BadRequest(format!("unsupported MCP market source: {other}"))),
    }
}

async fn resolve_market_package(req: SkillMarketPackageRequest) -> Result<SkillMarketPackageResponse, AppError> {
    if req.source != SKILLHUB_PACKAGES_SOURCE {
        return Err(AppError::BadRequest(format!(
            "unsupported package market source: {}",
            req.source
        )));
    }
    let slug = market_ref_suffix(&req.id, SKILLHUB_PACKAGES_SOURCE)
        .or_else(|| last_url_segment(&req.url))
        .ok_or_else(|| AppError::BadRequest("invalid SkillHub package id".into()))?;
    if !is_market_slug(&slug) {
        return Err(AppError::BadRequest("invalid SkillHub package slug".into()));
    }

    let client = build_market_client()?;
    let body = read_market_body(&client, SKILLHUB_PACKAGES_URL).await?;
    let root = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| AppError::BadGateway(format!("SkillHub package JSON parse failed: {e}")))?;
    let packages = root
        .get("skillSets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AppError::BadGateway("SkillHub package list missing skillSets".into()))?;
    let package = packages
        .iter()
        .find(|item| json_text(item, "slug", 96).as_deref() == Some(slug.as_str()))
        .ok_or_else(|| AppError::NotFound(format!("SkillHub package '{slug}' not found")))?;

    build_skillhub_package_response(package, &slug)
}

fn build_skillhub_package_response(
    package: &serde_json::Value,
    slug: &str,
) -> Result<SkillMarketPackageResponse, AppError> {
    let name = json_text(package, "displayName", 96)
        .or_else(|| json_text(package, "displayNameEn", 96))
        .unwrap_or_else(|| title_from_slug(slug));
    let description = json_text(package, "summary", 500)
        .or_else(|| json_text(package, "summaryEn", 500))
        .unwrap_or_default();
    let instructions = json_text_preserve(package, "content", 120_000)
        .or_else(|| json_text_preserve(package, "contentEn", 120_000))
        .ok_or_else(|| AppError::BadGateway("SkillHub package content missing".into()))?;
    let skill_slugs = package_skill_slugs(package, &instructions);
    let avatar = json_text(package, "iconUrl", 260);

    Ok(SkillMarketPackageResponse {
        name,
        description,
        instructions,
        skill_slugs,
        avatar,
    })
}

async fn install_skillhub_package_skills(
    paths: &SkillPaths,
    skill_slugs: &[String],
) -> Result<SkillMarketPackageSkillInstallOutcome, AppError> {
    let slugs = normalize_package_skill_install_slugs(skill_slugs.to_vec());
    if slugs.is_empty() {
        return Ok(SkillMarketPackageSkillInstallOutcome::default());
    }

    let available = skill_service::list_available_skills(paths).await?;
    let mut available_names = available
        .into_iter()
        .map(|skill| (skill.name.to_ascii_lowercase(), skill.name))
        .collect::<HashMap<_, _>>();
    let client = build_market_client()?;
    let mut installed_skill_names = Vec::new();
    let mut errors = Vec::new();

    for slug in slugs {
        if let Some(name) = available_names.get(&slug.to_ascii_lowercase()) {
            installed_skill_names.push(name.clone());
            continue;
        }

        let child_result = async {
            let (download_slug, archive) = download_skillhub_skill_zip(&client, &slug).await?;
            import_skillhub_skill_archive(paths, &download_slug, &archive).await
        }
        .await;

        match child_result {
            Ok(imported) => {
                for name in imported {
                    available_names.insert(name.to_ascii_lowercase(), name.clone());
                    installed_skill_names.push(name);
                }
            }
            Err(error) => errors.push(SkillMarketPackageInstallError {
                skill_slug: slug,
                error: error.to_string(),
            }),
        }
    }

    dedup_strings(&mut installed_skill_names);
    Ok(SkillMarketPackageSkillInstallOutcome {
        installed_skill_names,
        errors,
    })
}

#[derive(Default)]
struct SkillMarketPackageSkillInstallOutcome {
    installed_skill_names: Vec<String>,
    errors: Vec<SkillMarketPackageInstallError>,
}

async fn download_skillhub_skill_zip(
    client: &reqwest::Client,
    skill_slug: &str,
) -> Result<(String, Vec<u8>), AppError> {
    if !is_market_slug(skill_slug) {
        return Err(AppError::BadRequest("invalid SkillHub skill slug".into()));
    }

    match request_skillhub_skill_zip(client, skill_slug).await {
        Ok(bytes) => Ok((skill_slug.to_string(), bytes)),
        Err(AppError::NotFound(_)) => {
            let found_slug = search_skillhub_skill_slug(client, skill_slug).await?;
            let bytes = request_skillhub_skill_zip(client, &found_slug).await?;
            Ok((found_slug, bytes))
        }
        Err(error) => Err(error),
    }
}

async fn request_skillhub_skill_zip(client: &reqwest::Client, skill_slug: &str) -> Result<Vec<u8>, AppError> {
    let url = skillhub_skill_download_url(skill_slug)?;
    let mut response = client
        .get(url)
        .header(ACCEPT, "application/zip,application/octet-stream,*/*")
        .send()
        .await
        .map_err(map_market_fetch_error)?;
    read_market_bytes(&mut response, MAX_SKILLHUB_SKILL_ZIP_BYTES, "SkillHub skill archive").await
}

fn skillhub_skill_download_url(skill_slug: &str) -> Result<reqwest::Url, AppError> {
    if !is_market_slug(skill_slug) {
        return Err(AppError::BadRequest("invalid SkillHub skill slug".into()));
    }
    reqwest::Url::parse_with_params(SKILLHUB_SKILL_DOWNLOAD_URL, &[("slug", skill_slug)])
        .map_err(|e| AppError::Internal(format!("invalid SkillHub download URL: {e}")))
}

async fn search_skillhub_skill_slug(client: &reqwest::Client, skill_slug: &str) -> Result<String, AppError> {
    let url = reqwest::Url::parse_with_params(
        SKILLHUB_SKILL_SEARCH_URL,
        &[("q", skill_slug), ("limit", "20")],
    )
    .map_err(|e| AppError::Internal(format!("invalid SkillHub search URL: {e}")))?;
    let body = read_market_body(client, url.as_str()).await?;
    let root = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| AppError::BadGateway(format!("SkillHub search JSON parse failed: {e}")))?;
    select_skillhub_search_slug(&root, skill_slug)
        .ok_or_else(|| AppError::NotFound(format!("SkillHub skill '{skill_slug}' not found")))
}

fn select_skillhub_search_slug(root: &serde_json::Value, requested_slug: &str) -> Option<String> {
    let results = root
        .get("results")
        .and_then(serde_json::Value::as_array)
        .or_else(|| root.as_array())?;

    results.iter().find_map(|item| {
        let slug = json_text(item, "slug", 96)
            .or_else(|| item.get("skill").and_then(|skill| json_text(skill, "slug", 96)))?;
        if is_market_slug(&slug) && slug.eq_ignore_ascii_case(requested_slug) {
            Some(slug)
        } else {
            None
        }
    })
}

async fn import_skillhub_skill_archive(
    paths: &SkillPaths,
    skill_slug: &str,
    archive: &[u8],
) -> Result<Vec<String>, AppError> {
    let temp_dir = paths.user_skills_dir.join(".market-import");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let archive_path = temp_dir.join(format!("skillhub-{skill_slug}-{nonce}.zip"));
    tokio::fs::write(&archive_path, archive)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let result = skill_service::import_skills_with_symlink(paths, &archive_path)
        .await
        .map_err(AppError::from);
    let _ = tokio::fs::remove_file(&archive_path).await;
    let _ = tokio::fs::remove_dir(&temp_dir).await;
    result
}

fn map_market_fetch_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::Timeout(format!("skill market fetch timed out: {error}"))
    } else {
        AppError::BadGateway(format!("skill market fetch failed: {error}"))
    }
}

fn parse_clawhub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root
            .pointer("/value/items")
            .or_else(|| root.pointer("/value/page"))
            .and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = items
                .iter()
                .filter_map(parse_clawhub_api_item)
                .take(MAX_MARKET_ITEMS_PER_SOURCE)
                .enumerate()
                .map(|(index, mut item)| {
                    item.rank = index + 1;
                    item
                })
                .collect::<Vec<_>>();
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_clawhub_html_rankings(body)
}

fn parse_clawhub_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let skill = item.get("skill")?;
    if skill
        .get("isSuspicious")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let owner = json_text(item, "ownerHandle", 96)
        .or_else(|| item.get("owner").and_then(|owner| json_text(owner, "handle", 96)))?;
    let slug = json_text(skill, "slug", 96)?;
    if valid_owner_slug(&owner, &slug).is_none() {
        return None;
    }
    let name = json_text(skill, "displayName", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(skill, "summary", 220).unwrap_or_default();
    let mut tags = json_string_array(skill.get("topics"), 40);
    tags.extend(json_string_array(skill.get("categories"), 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        skill.get("stats"),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{CLAWHUB_SOURCE}:{owner}/{slug}"),
        source: CLAWHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://clawhub.ai/{owner}/skills/{slug}"),
        install_command: format!("openclaw skills install @{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_clawhub_html_rankings(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(CLAWHUB_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = clawhub_owner_slug(&url) else {
            continue;
        };
        let id = format!("{CLAWHUB_SOURCE}:{owner}/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_clawhub_name(&text, &owner, &slug);
        let description = extract_clawhub_description(&text, &owner, &name);
        let stats = extract_stats(&text);
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: CLAWHUB_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://clawhub.ai/{owner}/skills/{slug}"),
            install_command: format!("openclaw skills install @{owner}/{slug}"),
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

fn parse_skillhub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root.pointer("/data/skills").and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = items
                .iter()
                .filter_map(parse_skillhub_api_item)
                .take(MAX_MARKET_ITEMS_PER_SOURCE)
                .enumerate()
                .map(|(index, mut item)| {
                    item.rank = index + 1;
                    item
                })
                .collect::<Vec<_>>();
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_skillhub_html_rankings(body)
}

fn parse_skillhub_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let namespace = item.get("namespace")?;
    let canonical = json_text(namespace, "canonicalName", 160);
    let (owner, slug) = canonical
        .as_deref()
        .and_then(skillhub_canonical_owner_slug)
        .or_else(|| {
            let owner = json_text(namespace, "handle", 96)?;
            let slug = json_text(namespace, "publicSlug", 96).or_else(|| json_text(item, "slug", 96))?;
            valid_owner_slug(&owner, &slug)
        })?;
    let name = json_text(item, "name", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "description_zh", 220)
        .or_else(|| json_text(item, "description", 220))
        .unwrap_or_default();
    let mut tags = Vec::new();
    if item
        .pointer("/labels/requires_api_key")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        tags.push("requires_api_key".into());
    } else {
        tags.push("no_api_key".into());
    }
    tags.extend(
        item.get("subCategories")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|category| json_text(category, "key", 40)),
    );
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        Some(item),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_SOURCE}:{owner}/skills/{slug}"),
        source: SKILLHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/skills/{owner}/{slug}"),
        install_command: format!("npx skills add @{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_skillhub_html_rankings(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(SKILLHUB_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = skillhub_owner_slug(&url) else {
            continue;
        };
        let id = format!("{SKILLHUB_SOURCE}:{owner}/skills/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_skillhub_name(&text, &owner, &slug);
        let stats = extract_stats(&text);
        let description = extract_skillhub_description(&text, &owner, &name, stats.as_deref());
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let install_command = if owner.contains('.') {
            format!("npx skills add https://www.skills.sh/{owner}/skills/{slug}")
        } else {
            format!("npx skills add https://github.com/{owner}/skills --skill {slug}")
        };
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: SKILLHUB_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://www.skills.sh/{owner}/skills/{slug}"),
            install_command,
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

fn parse_loophub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.pointer("/data/items").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(parse_loophub_item)
        .take(MAX_MARKET_ITEMS_PER_SOURCE)
        .enumerate()
        .map(|(index, mut item)| {
            item.rank = index + 1;
            item
        })
        .collect()
}

fn parse_loophub_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let id = item.get("id")?.as_i64()?;
    let download_url = json_text(item, "download_url", 260)?;
    if !download_url.starts_with("https://dl.cocoloop.cn/bss/skills/") {
        return None;
    }
    let name = json_text(item, "name", 96).unwrap_or_else(|| format!("LoopHub Skill {id}"));
    let subtitle = json_text(item, "subtitle", 160).unwrap_or_default();
    let brief = json_text(item, "brief", 220).unwrap_or_default();
    let description = if !brief.is_empty() { brief } else { subtitle };
    let stats = json_text(item, "downloads", 60).map(|downloads| format!("{downloads} downloads"));
    let mut tags = json_text(item, "category", 40).into_iter().collect::<Vec<_>>();
    tags.extend(json_text(item, "security_level", 20).map(|value| format!("security-{value}")));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    Some(SkillMarketItemResponse {
        id: format!("{LOOPHUB_SOURCE}:{id}"),
        source: LOOPHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://hub.cocoloop.cn/skills/{id}"),
        install_command: format!("loophub skill download {download_url}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_skillhub_mcp_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.get("items").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(parse_skillhub_mcp_item)
        .take(MAX_MARKET_ITEMS_PER_SOURCE)
        .enumerate()
        .map(|(index, mut item)| {
            item.rank = index + 1;
            item
        })
        .collect()
}

fn parse_skillhub_mcp_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let slug = json_text(item, "slug", 96)?;
    if !is_market_slug(&slug) {
        return None;
    }
    let name = json_text(item, "name", 96)
        .or_else(|| json_text(item, "nameEn", 96))
        .unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summaryZh", 220)
        .or_else(|| json_text(item, "summary", 220))
        .unwrap_or_else(|| "SkillHub MCP server.".into());
    let mut tags = json_text(item, "category", 40).into_iter().collect::<Vec<_>>();
    tags.extend(json_string_array(item.get("tags"), 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = item.get("stats").map(|stats| {
        let downloads = stats.get("downloads").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let installs = stats.get("installs").and_then(serde_json::Value::as_u64).unwrap_or(0);
        format!("{downloads} downloads / {installs} installs")
    });
    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_MCP_SOURCE}:{slug}"),
        source: SKILLHUB_MCP_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/mcp/{slug}"),
        install_command: format!("mcp market add skillhub:{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_mcpworld_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(lists) = root.pointer("/data/mcpList").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    lists
        .iter()
        .flat_map(|list| list.get("servers").and_then(serde_json::Value::as_array).into_iter().flatten())
        .filter_map(parse_mcpworld_item)
        .take(MAX_MARKET_ITEMS_PER_SOURCE)
        .enumerate()
        .map(|(index, mut item)| {
            item.rank = index + 1;
            item
        })
        .collect()
}

fn parse_mcpworld_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let id = json_text(item, "id", 120)?;
    if !is_market_slug(&id) {
        return None;
    }
    let name = json_text(item, "serverName", 96).unwrap_or_else(|| format!("MCP {id}"));
    let description = json_text(item, "description", 220).unwrap_or_else(|| "MCP World server.".into());
    let mut tags = json_string_array(item.get("labels"), 40);
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = item.get("star").and_then(serde_json::Value::as_u64).map(|stars| format!("{stars} stars"));
    Some(SkillMarketItemResponse {
        id: format!("{MCPWORLD_SOURCE}:{id}"),
        source: MCPWORLD_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://www.mcpworld.com/zh/detail/{id}"),
        install_command: format!("mcp market add mcpworld:{id}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_clawhub_plugins(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root.get("items").and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = items
                .iter()
                .filter_map(parse_clawhub_plugin_api_item)
                .take(MAX_MARKET_ITEMS_PER_SOURCE)
                .enumerate()
                .map(|(index, mut item)| {
                    item.rank = index + 1;
                    item
                })
                .collect::<Vec<_>>();
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_clawhub_plugins_html(body)
}

fn parse_clawhub_plugin_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let canonical_name = json_text(item, "name", 160)?;
    let (owner, slug) = skillhub_canonical_owner_slug(&canonical_name).or_else(|| {
        let owner = json_text(item, "ownerHandle", 96)?;
        let slug = json_text(item, "runtimeId", 96)?;
        valid_owner_slug(&owner, &slug)
    })?;
    let name = json_text(item, "displayName", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summary", 220).unwrap_or_default();
    let mut tags = json_string_array(item.get("topics"), 40);
    tags.extend(json_string_array(item.get("categories"), 40));
    tags.extend(json_text(item, "family", 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        item.get("stats"),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{CLAWHUB_PLUGINS_SOURCE}:{owner}/{slug}"),
        source: CLAWHUB_PLUGINS_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://clawhub.ai/{owner}/plugins/{slug}"),
        install_command: format!("openclaw plugins install clawhub:@{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_clawhub_plugins_html(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(CLAWHUB_PLUGINS_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = clawhub_plugin_owner_slug(&url) else {
            continue;
        };
        let id = format!("{CLAWHUB_PLUGINS_SOURCE}:{owner}/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_clawhub_name(&text, &owner, &slug);
        let description = extract_clawhub_description(&text, &owner, &name);
        let stats = extract_stats(&text);
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: CLAWHUB_PLUGINS_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://clawhub.ai/{owner}/plugins/{slug}"),
            install_command: format!("openclaw plugins install clawhub:@{owner}/{slug}"),
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

fn parse_skillhub_packages(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.get("skillSets").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(parse_skillhub_package_item)
        .take(MAX_MARKET_ITEMS_PER_SOURCE)
        .enumerate()
        .map(|(index, mut item)| {
            item.rank = index + 1;
            item
        })
        .collect()
}

fn parse_skillhub_package_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let slug = json_text(item, "slug", 96)?;
    if !is_market_slug(&slug) {
        return None;
    }
    let name = json_text(item, "displayName", 96)
        .or_else(|| json_text(item, "displayNameEn", 96))
        .unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summary", 220)
        .or_else(|| json_text(item, "summaryEn", 220))
        .unwrap_or_else(|| "SkillHub expert package.".into());
    let skill_slugs = json_string_array(item.get("skillSlugs"), 40);
    let mut tags = skill_slugs.clone();
    tags.extend(json_text(item, "scene", 40));
    tags.extend(json_text(item, "subScene", 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let skill_count = item
        .get("skillCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(skill_slugs.len() as u64);
    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_PACKAGES_SOURCE}:{slug}"),
        source: SKILLHUB_PACKAGES_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/skillspackage/{slug}"),
        install_command: format!("skillhub package add {slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats: Some(format!("{skill_count} skills")),
    })
}

fn market_anchors(html: &str) -> Vec<(String, String)> {
    let anchor_re = regex::Regex::new(r#"(?is)<a\b[^>]*\bhref=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    anchor_re
        .captures_iter(html)
        .filter_map(|cap| {
            let href = cap.get(1)?.as_str().trim();
            let inner = cap.get(2)?.as_str();
            let text = clean_market_text(&strip_html_tags(inner), 360);
            if href.is_empty() || text.is_empty() {
                return None;
            }
            Some((href.to_string(), text))
        })
        .collect()
}

fn market_url(source: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with('#') || href.starts_with("mailto:") || href.starts_with("javascript:") {
        return None;
    }
    let url = if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        match source {
            CLAWHUB_SOURCE | CLAWHUB_PLUGINS_SOURCE => format!("https://clawhub.ai{href}"),
            SKILLHUB_SOURCE => format!("https://www.skills.sh{href}"),
            LOOPHUB_SOURCE => format!("https://hub.cocoloop.cn{href}"),
            SKILLHUB_MCP_SOURCE | SKILLHUB_PACKAGES_SOURCE => format!("https://skillhub.cn{href}"),
            MCPWORLD_SOURCE => format!("https://www.mcpworld.com{href}"),
            _ => return None,
        }
    } else {
        return None;
    };

    match source {
        CLAWHUB_SOURCE if url.starts_with("https://clawhub.ai/") => Some(url),
        CLAWHUB_PLUGINS_SOURCE if url.starts_with("https://clawhub.ai/") => Some(url),
        SKILLHUB_SOURCE if url.starts_with("https://www.skills.sh/") || url.starts_with("https://skills.sh/") => {
            Some(url.replacen("https://skills.sh/", "https://www.skills.sh/", 1))
        }
        LOOPHUB_SOURCE if url.starts_with("https://hub.cocoloop.cn/") => Some(url),
        SKILLHUB_MCP_SOURCE | SKILLHUB_PACKAGES_SOURCE if url.starts_with("https://skillhub.cn/") => Some(url),
        MCPWORLD_SOURCE if url.starts_with("https://www.mcpworld.com/") => Some(url),
        _ => None,
    }
}

fn clawhub_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://clawhub.ai")?;
    let reserved = ["skills", "plugins", "docs", "about", "login", "sign-in", "search"];
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "skills") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    if segments.len() == 2 && !reserved.contains(&segments[0].as_str()) && !reserved.contains(&segments[1].as_str()) {
        return valid_owner_slug(&segments[0], &segments[1]);
    }
    None
}

fn skillhub_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://www.skills.sh")?;
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "skills") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    None
}

fn clawhub_plugin_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://clawhub.ai")?;
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "plugins") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    None
}

fn market_path_segments(url: &str, origin: &str) -> Option<Vec<String>> {
    let rest = url.strip_prefix(origin)?;
    let path = rest
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(path.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

fn valid_owner_slug(owner: &str, slug: &str) -> Option<(String, String)> {
    if is_market_slug(owner) && is_market_slug(slug) {
        Some((owner.to_string(), slug.to_string()))
    } else {
        None
    }
}

fn skillhub_canonical_owner_slug(canonical_name: &str) -> Option<(String, String)> {
    let value = canonical_name.trim().trim_start_matches('@');
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let slug = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    valid_owner_slug(owner, slug)
}

fn package_skill_slugs(package: &serde_json::Value, instructions: &str) -> Vec<String> {
    let mut slugs = json_string_array(package.get("skillSlugs"), 80);
    slugs.extend(frontmatter_orchestration_child_slugs(instructions));
    normalize_package_skill_slugs(slugs)
}

fn normalize_package_skill_slugs(slugs: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    slugs
        .into_iter()
        .map(|value| clean_market_text(&value, 80))
        .filter(|value| is_market_slug(value) && !is_package_metadata_field(value))
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn normalize_package_skill_install_slugs(slugs: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    slugs
        .into_iter()
        .map(|value| clean_market_text(&value, 80))
        .filter(|value| !value.is_empty() && !is_package_metadata_field(value))
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn is_package_metadata_field(value: &str) -> bool {
    const FIELDS: &[&str] = &[
        "aliases",
        "author",
        "children",
        "compatibility",
        "description",
        "display_name",
        "metadata",
        "name",
        "orchestration",
        "package_type",
        "version",
    ];
    FIELDS.iter().any(|field| value.eq_ignore_ascii_case(field))
}

fn frontmatter_orchestration_child_slugs(markdown: &str) -> Vec<String> {
    let Some(frontmatter) = markdown_frontmatter(markdown) else {
        return Vec::new();
    };
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) else {
        return Vec::new();
    };
    let Some(children) =
        yaml_mapping_get(&root, "orchestration").and_then(|value| yaml_mapping_get(value, "children"))
    else {
        return Vec::new();
    };

    children
        .as_sequence()
        .into_iter()
        .flatten()
        .filter_map(serde_yaml::Value::as_str)
        .map(|value| clean_market_text(value, 80))
        .collect()
}

fn yaml_mapping_get<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a serde_yaml::Value> {
    let serde_yaml::Value::Mapping(map) = value else {
        return None;
    };
    map.get(&serde_yaml::Value::String(key.to_string()))
}

fn markdown_frontmatter(markdown: &str) -> Option<&str> {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let rest = markdown
        .strip_prefix("---\r\n")
        .or_else(|| markdown.strip_prefix("---\n"))?;

    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
        if trimmed == "---" || trimmed == "..." {
            return Some(rest[..offset].trim());
        }
        offset += line.len();
    }

    None
}

fn is_market_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn json_text(item: &serde_json::Value, key: &str, max_chars: usize) -> Option<String> {
    item.get(key)
        .and_then(serde_json::Value::as_str)
        .map(|value| clean_market_text(value, max_chars))
        .filter(|value| !value.is_empty())
}

fn json_text_preserve(item: &serde_json::Value, key: &str, max_chars: usize) -> Option<String> {
    let value = item.get(key)?.as_str()?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(max_chars).collect())
}

fn json_string_array(value: Option<&serde_json::Value>, max_chars: usize) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(|value| clean_market_text(value, max_chars))
        .filter(|value| !value.is_empty())
        .collect()
}

fn market_count_stats(value: Option<&serde_json::Value>, fields: &[(&str, &str)]) -> Option<String> {
    let value = value?;
    let stats = fields
        .iter()
        .filter_map(|(key, label)| {
            let count = value
                .get(*key)
                .and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64))
                        .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
                })?;
            Some(format!("{count} {label}"))
        })
        .collect::<Vec<_>>();
    if stats.is_empty() {
        None
    } else {
        Some(stats.join(" · "))
    }
}

fn market_ref_suffix(id: &str, source: &str) -> Option<String> {
    id.strip_prefix(&format!("{source}:"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn last_url_segment(url: &str) -> Option<String> {
    url.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/')
        .rsplit('/')
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_mcp_config_from_markdown(markdown: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(markdown.trim()) {
        if value.get("mcpServers").is_some() {
            return Some(value);
        }
    }

    let fence_re = regex::Regex::new(r"(?is)```(?:json|javascript|js)?\s*(.*?)```").unwrap();
    for cap in fence_re.captures_iter(markdown) {
        let Some(block) = cap.get(1).map(|m| m.as_str().trim()) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(block) {
            if value.get("mcpServers").is_some() {
                return Some(value);
            }
        }
    }
    None
}

fn extract_clawhub_name(text: &str, owner: &str, slug: &str) -> String {
    let owner_marker = format!("@ {owner}");
    let before_owner = text
        .split(&owner_marker)
        .next()
        .unwrap_or(text)
        .split('@')
        .next()
        .unwrap_or(text);
    let candidate = clean_market_text(before_owner.trim_matches(|c: char| c == '#' || c.is_ascii_digit()), 80);
    if candidate.len() >= 2 {
        candidate
    } else {
        title_from_slug(slug)
    }
}

fn extract_clawhub_description(text: &str, owner: &str, name: &str) -> String {
    let owner_marker = format!("@ {owner}");
    let tail = text.split(&owner_marker).nth(1).unwrap_or(text);
    let cleaned = strip_known_stats(tail);
    let cleaned = clean_market_text(&cleaned.replace(name, ""), 180);
    if cleaned.len() >= 12 {
        cleaned
    } else {
        "Trending ClawHub skill package.".into()
    }
}

fn extract_skillhub_name(text: &str, owner: &str, slug: &str) -> String {
    let repo_marker = format!("{owner}/skills");
    let before_repo = text.split(&repo_marker).next().unwrap_or(text);
    let candidate = clean_market_text(
        before_repo.trim_matches(|c: char| c == '#' || c.is_ascii_digit() || c == '.'),
        80,
    );
    if candidate.len() >= 2 && !candidate.eq_ignore_ascii_case("skill") {
        candidate
    } else {
        title_from_slug(slug)
    }
}

fn extract_skillhub_description(text: &str, owner: &str, name: &str, stats: Option<&str>) -> String {
    let without_stats = stats.map_or_else(|| text.to_string(), |s| text.replace(s, ""));
    let without_repo = without_stats.replace(&format!("{owner}/skills"), "");
    let cleaned = clean_market_text(&without_repo.replace(name, ""), 180);
    if cleaned.len() >= 18 {
        cleaned
    } else {
        format!("Ranked SkillHub skill from {owner}/skills.")
    }
}

fn extract_stats(text: &str) -> Option<String> {
    let stats_re =
        regex::Regex::new(r"(?i)(\d+(?:\.\d+)?\s*[km]?\+?\s*(?:installs?|downloads?|uses?|stars?)?)").unwrap();
    let mut matches = stats_re
        .captures_iter(text)
        .filter_map(|cap| cap.get(1).map(|m| clean_market_text(m.as_str(), 40)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    matches.dedup();
    matches.last().cloned()
}

fn strip_known_stats(text: &str) -> String {
    let stats_re = regex::Regex::new(r"(?i)\d+(?:\.\d+)?\s*[km]?\+?\s*(?:installs?|downloads?|uses?|stars?)?").unwrap();
    stats_re.replace_all(text, " ").to_string()
}

fn strip_html_tags(html: &str) -> String {
    let tag_re = regex::Regex::new(r"(?is)<[^>]+>").unwrap();
    tag_re.replace_all(html, " ").to_string()
}

fn clean_market_text(text: &str, max_chars: usize) -> String {
    let decoded = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let mut out = String::new();
    let mut last_was_space = false;
    for ch in decoded.chars() {
        let is_space = ch.is_whitespace() || ch.is_control();
        if is_space {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
        if out.chars().count() >= max_chars {
            break;
        }
    }
    out.trim().to_string()
}

fn title_from_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn infer_market_tags(text: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let lower = text.to_ascii_lowercase();
    let mut audience = Vec::new();
    let mut scenario = Vec::new();

    if contains_any(&lower, &["code", "github", "git", "api", "cli", "npm", "python", "typescript", "developer"]) {
        audience.push("developer".to_string());
        scenario.push("coding".to_string());
    }
    if contains_any(&lower, &["doc", "pdf", "word", "office", "excel", "sheet", "ppt", "slide"]) {
        audience.push("office".to_string());
        if contains_any(&lower, &["excel", "sheet", "spreadsheet"]) {
            scenario.push("spreadsheet".to_string());
        }
        if contains_any(&lower, &["ppt", "slide", "presentation"]) {
            scenario.push("presentation".to_string());
        }
        if contains_any(&lower, &["doc", "pdf", "word"]) {
            scenario.push("document".to_string());
        }
    }
    if contains_any(&lower, &["design", "image", "figma", "ui", "ux"]) {
        audience.push("designer".to_string());
        scenario.push("design".to_string());
    }
    if contains_any(&lower, &["research", "paper", "academic", "web search"]) {
        audience.push("student".to_string());
        scenario.push("research".to_string());
    }
    if contains_any(&lower, &["write", "blog", "copy", "content"]) {
        scenario.push("writing".to_string());
    }
    if contains_any(&lower, &["plan", "project", "task", "calendar"]) {
        scenario.push("planning".to_string());
    }
    if contains_any(&lower, &["social", "tweet", "x.com", "marketing"]) {
        audience.push("marketing".to_string());
        scenario.push("social".to_string());
    }
    if contains_any(&lower, &["setup", "install", "configure", "config"]) {
        scenario.push("setup".to_string());
    }

    dedup_strings(&mut audience);
    dedup_strings(&mut scenario);
    let mut tags = audience.clone();
    tags.extend(scenario.clone());
    dedup_strings(&mut tags);
    (tags, audience, scenario)
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InMemorySkillTagRepo {
        rows: std::sync::Mutex<Vec<nomifun_db::SkillTagRow>>,
    }
    #[async_trait::async_trait]
    impl nomifun_db::ISkillTagRepository for InMemorySkillTagRepo {
        async fn get_all(&self) -> Result<Vec<nomifun_db::SkillTagRow>, nomifun_db::DbError> {
            Ok(self.rows.lock().unwrap().clone())
        }
        async fn upsert(
            &self,
            p: &nomifun_db::UpsertSkillTagParams<'_>,
        ) -> Result<nomifun_db::SkillTagRow, nomifun_db::DbError> {
            let row = nomifun_db::SkillTagRow {
                id: 0,
                skill_name: p.skill_name.into(),
                audience_tags: p.audience_tags.map(String::from),
                scenario_tags: p.scenario_tags.map(String::from),
                updated_at: 0,
            };
            let mut g = self.rows.lock().unwrap();
            g.retain(|r| r.skill_name != row.skill_name);
            g.push(row.clone());
            Ok(row)
        }
        async fn delete(&self, name: &str) -> Result<bool, nomifun_db::DbError> {
            let mut g = self.rows.lock().unwrap();
            let before = g.len();
            g.retain(|r| r.skill_name != name);
            Ok(g.len() != before)
        }
    }

    async fn make_state() -> SkillRouterState {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = SkillPaths {
            data_dir: tmp.path().to_path_buf(),
            user_skills_dir: tmp.path().join("skills"),
            cron_skills_dir: tmp.path().join("cron").join("skills"),
            builtin_skills_dir: tmp.path().join("builtin-skills"),
            builtin_rules_dir: tmp.path().join("builtin-rules"),
            preset_rules_dir: tmp.path().join("preset-rules"),
            preset_skills_dir: tmp.path().join("preset-skills"),
        };
        let ext_mgr = Arc::new(ExternalPathsManager::with_file(tmp.path().join("paths.json")).await);
        std::mem::forget(tmp);
        SkillRouterState {
            skill_paths: paths,
            external_paths_manager: ext_mgr,
            preset_dispatcher: None,
            skill_tag_repo: std::sync::Arc::new(InMemorySkillTagRepo::default()),
            builtin_skill_tags: std::sync::Arc::new(std::collections::HashMap::new()),
        }
    }

    #[tokio::test]
    async fn skill_routes_builds_router() {
        let state = make_state().await;
        let _router = skill_routes(state);
    }

    #[test]
    fn normalize_market_sources_rejects_unknown_source() {
        let err = normalize_market_sources(vec!["unknown".into()]).unwrap_err();
        assert!(err.to_string().contains("unsupported skill market source"));
    }

    #[test]
    fn normalize_market_sources_accepts_new_market_sources() {
        let sources = normalize_market_sources(vec![
            LOOPHUB_SOURCE.into(),
            SKILLHUB_MCP_SOURCE.into(),
            MCPWORLD_SOURCE.into(),
            CLAWHUB_PLUGINS_SOURCE.into(),
            SKILLHUB_PACKAGES_SOURCE.into(),
        ])
        .unwrap();
        assert_eq!(
            sources,
            vec![
                LOOPHUB_SOURCE,
                SKILLHUB_MCP_SOURCE,
                MCPWORLD_SOURCE,
                CLAWHUB_PLUGINS_SOURCE,
                SKILLHUB_PACKAGES_SOURCE,
            ]
        );
    }

    #[test]
    fn clawhub_market_uses_skills_ranking_page() {
        assert!(CLAWHUB_RANKING_URL.ends_with("/skills?tab=new"));
    }

    #[test]
    fn parse_clawhub_rankings_extracts_safe_install_command() {
        let html = r#"
          <a href="/pskoett/skills/self-improving-agent">
            <span>self-improving agent</span>
            <span>@<!-- -->pskoett</span>
            <p>Captures discoveries from agent sessions into reusable skills.</p>
            <span>468k installs</span>
          </a>
        "#;
        let items = parse_clawhub_rankings(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_SOURCE);
        assert_eq!(items[0].install_command, "openclaw skills install @pskoett/self-improving-agent");
        assert!(items[0].url.starts_with("https://clawhub.ai/"));
    }

    #[test]
    fn parse_clawhub_rankings_extracts_convex_api_items() {
        let body = r#"{
          "status": "success",
          "value": {
            "page": [{
              "ownerHandle": "steipete",
              "skill": {
                "displayName": "Github",
                "slug": "github",
                "summary": "Interact with GitHub using the gh CLI.",
                "stats": { "downloads": 194199, "installs": 7620, "stars": 659 },
                "topics": ["GitHub"],
                "categories": ["integrations"]
              }
            }]
          }
        }"#;
        let items = parse_clawhub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_SOURCE);
        assert_eq!(items[0].url, "https://clawhub.ai/steipete/skills/github");
        assert_eq!(items[0].install_command, "openclaw skills install @steipete/github");
        assert_eq!(items[0].stats.as_deref(), Some("194199 downloads · 7620 installs · 659 stars"));
    }

    #[test]
    fn parse_skillhub_rankings_extracts_skills_command() {
        let html = r#"
          <a href="/vercel-labs/skills/find-skills">
            <span>find-skills</span>
            <span>vercel-labs/skills</span>
            <span>2.5M installs</span>
          </a>
        "#;
        let items = parse_skillhub_rankings(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_SOURCE);
        assert_eq!(
            items[0].install_command,
            "npx skills add https://github.com/vercel-labs/skills --skill find-skills"
        );
    }

    #[test]
    fn parse_skillhub_rankings_extracts_api_items() {
        let body = r#"{
          "code": 0,
          "data": {
            "skills": [{
              "name": "web-tools-guide",
              "slug": "web-tools-guide",
              "description_zh": "上网检索工具指南",
              "downloads": 196303,
              "installs": 3459,
              "stars": 168,
              "namespace": {
                "canonicalName": "@user_ec205dbb/web-tools-guide",
                "handle": "user_ec205dbb",
                "publicSlug": "web-tools-guide"
              },
              "labels": { "requires_api_key": "false" },
              "subCategories": [{ "key": "knowledge-retrieval", "name": "信息检索" }]
            }]
          }
        }"#;
        let items = parse_skillhub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_SOURCE);
        assert_eq!(items[0].url, "https://skillhub.cn/skills/user_ec205dbb/web-tools-guide");
        assert_eq!(items[0].install_command, "npx skills add @user_ec205dbb/web-tools-guide");
        assert!(items[0].tags.contains(&"no_api_key".into()));
    }

    #[test]
    fn parse_loophub_rankings_extracts_download_package() {
        let body = r#"{
          "code": 0,
          "data": {
            "items": [{
              "id": 12277,
              "author": "pskoett",
              "name": "Self-Improving Agent",
              "subtitle": "Keeps lessons",
              "brief": "Records fixes and best practices.",
              "downloads": "419.4k",
              "category": "productivity",
              "security_level": "A",
              "download_url": "https://dl.cocoloop.cn/bss/skills/pskoett-self-improving-agent.zip"
            }]
          }
        }"#;
        let items = parse_loophub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, LOOPHUB_SOURCE);
        assert_eq!(items[0].id, "loophub:12277");
        assert_eq!(
            items[0].install_command,
            "loophub skill download https://dl.cocoloop.cn/bss/skills/pskoett-self-improving-agent.zip"
        );
    }

    #[test]
    fn parse_skillhub_mcp_rankings_extracts_market_add_handle() {
        let body = r#"{
          "items": [{
            "slug": "playwright",
            "name": "Playwright MCP",
            "summary": "Browser automation server",
            "category": "browser",
            "tags": ["automation"],
            "stats": { "downloads": 12, "installs": 8 }
          }]
        }"#;
        let items = parse_skillhub_mcp_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_MCP_SOURCE);
        assert_eq!(items[0].url, "https://skillhub.cn/mcp/playwright");
        assert_eq!(items[0].install_command, "mcp market add skillhub:playwright");
    }

    #[test]
    fn parse_mcpworld_rankings_extracts_detail_url() {
        let body = r#"{
          "code": 0,
          "data": {
            "mcpList": [{
              "servers": [{
                "id": "c7897f8abf0350fbbf5a7fccc3e79bb8",
                "serverName": "Playwright MCP",
                "description": "Browser automation",
                "star": 68302,
                "labels": ["local", "browser"]
              }]
            }]
          }
        }"#;
        let items = parse_mcpworld_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, MCPWORLD_SOURCE);
        assert_eq!(
            items[0].url,
            "https://www.mcpworld.com/zh/detail/c7897f8abf0350fbbf5a7fccc3e79bb8"
        );
    }

    #[test]
    fn parse_clawhub_plugins_extracts_openclaw_plugin_command() {
        let html = r#"
          <a href="/openclaw/plugins/whatsapp">
            <span>WhatsApp MCP Plugin</span>
            <span>@<!-- -->openclaw</span>
            <p>WhatsApp chat integration.</p>
            <code>openclaw plugins install clawhub:@openclaw/whatsapp</code>
          </a>
        "#;
        let items = parse_clawhub_plugins(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_PLUGINS_SOURCE);
        assert_eq!(
            items[0].install_command,
            "openclaw plugins install clawhub:@openclaw/whatsapp"
        );
    }

    #[test]
    fn parse_clawhub_plugins_extracts_api_items() {
        let body = r#"{
          "items": [{
            "categories": ["channels"],
            "displayName": "WhatsApp",
            "family": "code-plugin",
            "name": "@openclaw/whatsapp",
            "ownerHandle": "openclaw",
            "runtimeId": "whatsapp",
            "stats": { "downloads": 160061, "installs": 597, "stars": 0 },
            "summary": "OpenClaw WhatsApp channel plugin for WhatsApp Web chats.",
            "topics": ["WhatsApp"]
          }],
          "totalCount": 1609
        }"#;
        let items = parse_clawhub_plugins(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_PLUGINS_SOURCE);
        assert_eq!(items[0].url, "https://clawhub.ai/openclaw/plugins/whatsapp");
        assert_eq!(
            items[0].install_command,
            "openclaw plugins install clawhub:@openclaw/whatsapp"
        );
        assert!(items[0].stats.as_deref().unwrap_or_default().contains("downloads"));
    }

    #[test]
    fn parse_skillhub_packages_extracts_expert_package() {
        let body = r#"{
          "skillSets": [{
            "slug": "tech-test-automation",
            "displayName": "Test Automation",
            "summary": "End-to-end automated testing workflow.",
            "skillCount": 6,
            "skillSlugs": ["superpowers-tdd", "test-case-generator"]
          }],
          "total": 1
        }"#;
        let items = parse_skillhub_packages(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_PACKAGES_SOURCE);
        assert_eq!(items[0].install_command, "skillhub package add tech-test-automation");
        assert!(items[0].stats.as_deref().unwrap_or_default().contains("6 skills"));
    }

    #[test]
    fn build_skillhub_package_response_uses_real_child_skills() {
        let package = serde_json::json!({
            "slug": "tech-test-automation",
            "displayName": "Test Automation",
            "summary": "End-to-end automated testing workflow.",
            "skillSlugs": ["name", "superpowers-tdd", "description", "superpowers-tdd"],
            "content": "---\nname: tech-test-automation\ndescription: Test package\nmetadata:\n  author: SkillHub\norchestration:\n  children:\n    - test-case-generator\n    - metadata\n---\n# Test Automation\nUse this package."
        });

        let response = build_skillhub_package_response(&package, "tech-test-automation").unwrap();

        assert_eq!(response.skill_slugs, vec!["superpowers-tdd", "test-case-generator"]);
        assert!(response.instructions.starts_with("---\nname: tech-test-automation"));
        assert!(response.instructions.contains("metadata:"));
        assert!(response.instructions.contains("# Test Automation"));
    }

    #[test]
    fn skillhub_skill_download_url_rejects_unsafe_slug() {
        assert!(skillhub_skill_download_url("superpowers-tdd").is_ok());
        assert!(skillhub_skill_download_url("../superpowers-tdd").is_err());
        assert!(skillhub_skill_download_url("owner/skill").is_err());
    }

    #[test]
    fn select_skillhub_search_slug_requires_exact_safe_slug() {
        let root = serde_json::json!({
            "results": [
                { "slug": "superpowers-tdd-extra", "displayName": "Superpowers TDD Extra" },
                { "skill": { "slug": "superpowers-tdd" }, "displayName": "Superpowers TDD" },
                { "slug": "../bad", "displayName": "Bad" }
            ]
        });

        assert_eq!(
            select_skillhub_search_slug(&root, "superpowers-tdd"),
            Some("superpowers-tdd".into())
        );
        assert_eq!(select_skillhub_search_slug(&root, "missing"), None);
    }

    #[tokio::test]
    async fn install_skillhub_package_skills_uses_existing_available_skill() {
        let state = make_state().await;
        let skill_dir = state.skill_paths.user_skills_dir.join("superpowers-tdd");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: superpowers-tdd\ndescription: TDD workflow\n---\n# Superpowers TDD",
        )
        .await
        .unwrap();

        let installed = install_skillhub_package_skills(&state.skill_paths, &["superpowers-tdd".into()])
            .await
            .unwrap();

        assert_eq!(installed.installed_skill_names, vec!["superpowers-tdd"]);
        assert!(installed.errors.is_empty());
    }

    #[tokio::test]
    async fn install_skillhub_package_skills_keeps_successes_when_one_child_fails() {
        let state = make_state().await;
        let skill_dir = state.skill_paths.user_skills_dir.join("superpowers-tdd");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: superpowers-tdd\ndescription: TDD workflow\n---\n# Superpowers TDD",
        )
        .await
        .unwrap();

        let installed = install_skillhub_package_skills(
            &state.skill_paths,
            &["../missing-child".into(), "superpowers-tdd".into()],
        )
        .await
        .unwrap();

        assert_eq!(installed.installed_skill_names, vec!["superpowers-tdd"]);
        assert_eq!(installed.errors.len(), 1);
        assert_eq!(installed.errors[0].skill_slug, "../missing-child");
        assert!(installed.errors[0].error.contains("invalid SkillHub skill slug"));
    }

    #[test]
    fn extract_mcp_config_from_markdown_finds_mcpservers_block() {
        let markdown = r#"
```json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["@playwright/mcp@latest"]
    }
  }
}
```
"#;
        let config = extract_mcp_config_from_markdown(markdown).unwrap();
        assert!(config.get("mcpServers").is_some());
    }

    /// Manual contract smoke test for the two third-party pages. Kept ignored
    /// in normal CI because it requires public network access and those sites
    /// are outside NomiFun's availability control.
    #[tokio::test]
    #[ignore = "requires public ClawHub and SkillHub access"]
    async fn live_market_pages_still_match_the_ranking_contract() {
        let response = fetch_skill_market_rankings(vec![CLAWHUB_SOURCE.into(), SKILLHUB_SOURCE.into()])
            .await
            .unwrap();

        assert!(response.errors.is_empty(), "live fetch errors: {:?}", response.errors);
        assert!(response.items.iter().any(|item| item.source == CLAWHUB_SOURCE));
        assert!(response.items.iter().any(|item| item.source == SKILLHUB_SOURCE));
        assert!(response.items.iter().all(|item| {
            item.url.starts_with("https://")
                && (item.install_command.starts_with("openclaw skills install @")
                    || item.install_command.starts_with("npx skills add "))
        }));
    }
}
