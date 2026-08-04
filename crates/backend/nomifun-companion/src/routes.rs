//! `/api/companion/*` route handlers.

use axum::Router;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use serde::Deserialize;

use crate::profile::{HeadBox, CompanionProfileConfig, SharedCompanionConfig};
use crate::memory_search::MemoryStatusFilter;
use crate::learner::CompanionLearnResult;
use crate::service::{
    CompanionSkillContent, CompanionSkillViewPage, CompanionStatus, CompanionWeeklyDigest,
    MemoryListItem, MemoryListPage, MemoryMergeGroup, SourceStats,
};
use crate::state::CompanionRouterState;
use crate::store::{
    MemoryBatchAction, MemoryFilter, MemoryListSort, MemoryScope,
    CompanionMemory, CompanionSkill,
};

pub fn companion_routes(state: CompanionRouterState) -> Router {
    Router::new()
        .route("/api/companion/config", get(get_config).patch(patch_config))
        .route("/api/companion/companions", get(list_companions).post(create_companion))
        .route(
            "/api/companion/companions/{companion_id}",
            get(get_companion).patch(patch_companion).delete(delete_companion),
        )
        .route(
            "/api/companion/companions/{companion_id}/apply-preset",
            post(apply_preset),
        )
        .route("/api/companion/companions/{companion_id}/status", get(companion_status))
        .route("/api/companion/companions/{companion_id}/figure", post(upload_figure).get(get_figure))
        .route("/api/companion/matting-model", get(get_matting_model))
        .route("/api/companion/figures", get(list_figures).post(create_figure))
        .route(
            "/api/companion/figures/{figure_id}",
            axum::routing::patch(update_figure).delete(delete_figure),
        )
        .route(
            "/api/companion/companions/{companion_id}/companion/threads",
            post(create_thread),
        )
        .route("/api/companion/companions/{companion_id}/companion/active", get(get_active_thread))
        .route("/api/companion/memories", get(list_memories).post(add_memory))
        .route(
            "/api/companion/memories/{memory_id}",
            axum::routing::put(update_memory).delete(delete_memory),
        )
        .route("/api/companion/memories/batch", post(batch_memories))
        .route("/api/companion/memories/merge-suggestions", post(memory_merge_suggestions))
        .route("/api/companion/memories/merge", post(merge_memories))
        .route("/api/companion/companions/{companion_id}/skills", get(list_companion_skills))
        .route("/api/companion/companions/{companion_id}/weekly-digest", get(weekly_digest))
        .route("/api/companion/companions/{companion_id}/digests", get(list_day_digests))
        .route(
            "/api/companion/companions/{companion_id}/skills/{companion_skill_id}",
            get(get_companion_skill).put(update_companion_skill),
        )
        .route(
            "/api/companion/companions/{companion_id}/skills/{companion_skill_id}/decide",
            post(decide_companion_skill),
        )
        .route(
            "/api/companion/companions/{companion_id}/skills/from-session",
            post(draft_skill_from_session),
        )
        .route(
            "/api/companion/companions/{companion_id}/skills/{companion_skill_id}/gift",
            post(gift_companion_skill),
        )
        .route("/api/companion/learn/run", post(run_learn))
        .route("/api/companion/events/stats", get(event_stats))
        .route("/api/companion/events/storage", get(event_storage))
        .route("/api/companion/consent", post(apply_consent))
        .route("/api/companion/disable-all", post(disable_all))
        .route("/api/companion/export/memory", post(export_memory))
        .route("/api/companion/export/companions/{companion_id}", post(export_companion))
        .route("/api/companion/import", post(import_package))
        .with_state(state)
}

/// Public (auth-exempt) figure-image serving.
///
/// `<img>` / `new Image()` are browser-native subresource loads with no
/// custom-header API, so under the desktop's `TrustLocalToken` policy they
/// cannot present the `x-nomi-local-trust` header — the authenticated router
/// would 403 every figure thumbnail (broken library image + blank desktop
/// companion mesh). This GET-only route therefore lives outside auth, exactly
/// like `asset_routes` (logos) and the office proxy. Figure ids are canonical
/// bare UUIDv7 values and listing/creation/rename/delete stay authenticated, so
/// this only serves opaque-id image bytes — a capability URL, not an
/// enumeration surface.
pub fn companion_public_routes(state: CompanionRouterState) -> Router {
    Router::new()
        .route("/api/companion/figures/{figure_id}/image", get(get_figure_image))
        .with_state(state)
}

async fn get_config(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<SharedCompanionConfig>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.get_config().await)))
}

async fn patch_config(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Json<ApiResponse<SharedCompanionConfig>>, AppError> {
    let Json(patch) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.patch_config(patch).await?)))
}

/// Build an optional [`MemoryScope`] from wire parts without accepting empty-ID
/// sentinels. Both fields absent means "leave unchanged"; shared is exactly
/// `scope_kind = "user"` with no owner, and private requires a canonical owner.
fn scope_from_parts(
    scope_kind: Option<&str>,
    scope_companion_id: Option<&str>,
) -> Result<Option<MemoryScope>, AppError> {
    match (scope_kind, scope_companion_id) {
        (None, None) => Ok(None),
        (Some("user"), None) => Ok(Some(MemoryScope::Shared)),
        (Some("companion"), Some(companion_id)) => {
            nomifun_common::CompanionId::try_from(companion_id).map_err(|error| {
                AppError::BadRequest(format!("invalid scope_companion_id: {error}"))
            })?;
            Ok(Some(MemoryScope::Companion(companion_id.to_owned())))
        }
        (Some(kind), _) if kind != "user" && kind != "companion" => {
            Err(AppError::BadRequest(format!("invalid memory scope_kind {kind:?}")))
        }
        (Some("user"), Some(_)) => Err(AppError::BadRequest(
            "shared memory scope must not include scope_companion_id".into(),
        )),
        (Some("companion"), None) => Err(AppError::BadRequest(
            "private memory scope requires scope_companion_id".into(),
        )),
        (None, Some(_)) => Err(AppError::BadRequest(
            "scope_companion_id requires an explicit scope_kind".into(),
        )),
        _ => unreachable!(),
    }
}

#[derive(Deserialize)]
struct ListMemoriesQuery {
    kind: Option<String>,
    q: Option<String>,
    /// `active` (default) / `archived` / `all`.
    status: Option<String>,
    /// When set, scope the list to memories visible to this companion (shared +
    /// its own private). Absent = cross-companion "all" view.
    scope_companion_id: Option<String>,
    /// `relevance` (default with `q`) / `time` / `importance`.
    sort: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_memories(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListMemoriesQuery>,
) -> Result<Json<ApiResponse<MemoryListPage>>, AppError> {
    if let Some(companion_id) = query.scope_companion_id.as_deref() {
        nomifun_common::CompanionId::try_from(companion_id).map_err(|error| {
            AppError::BadRequest(format!("invalid scope_companion_id: {error}"))
        })?;
    }
    let status = query.status.filter(|s| !s.is_empty()).unwrap_or_else(|| "active".into());
    let kind = query.kind.filter(|k| !k.is_empty());
    let q = query.q.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    let sort = query.sort.filter(|s| !s.is_empty());
    if let Some(sort) = sort.as_deref()
        && !matches!(sort, "relevance" | "time" | "importance")
    {
        return Err(AppError::BadRequest(format!("invalid memory sort '{sort}'")));
    }
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    if let Some(q) = q {
        // Full-text path: FTS relevance by default, snippet per hit.
        let status = match status.as_str() {
            "active" => MemoryStatusFilter::Active,
            "archived" => MemoryStatusFilter::Archived,
            "all" => MemoryStatusFilter::All,
            other => return Err(AppError::BadRequest(format!("invalid memory status '{other}'"))),
        };
        let sort = sort.as_deref().unwrap_or("relevance");
        let page = state
            .service
            .search_memory_page(&q, kind, status, query.scope_companion_id, sort, limit, offset)
            .await?;
        return Ok(Json(ApiResponse::ok(page)));
    }

    let status_filter = match status.as_str() {
        "active" | "archived" => Some(status),
        "all" => None,
        other => return Err(AppError::BadRequest(format!("invalid memory status '{other}'"))),
    };
    let sort = match sort.as_deref() {
        None | Some("relevance") => MemoryListSort::Default,
        Some("time") => MemoryListSort::Time,
        Some("importance") => MemoryListSort::Importance,
        Some(_) => unreachable!("validated above"),
    };
    let filter = MemoryFilter {
        kind,
        q: None,
        status: status_filter,
        scope_companion_id: query.scope_companion_id,
        limit,
        offset,
    };
    let page = state.service.list_memory_page_sorted(&filter, sort).await?;
    Ok(Json(ApiResponse::ok(MemoryListPage {
        items: page
            .items
            .into_iter()
            .map(|memory| MemoryListItem { memory, snippet: None, rank: None })
            .collect(),
        total: page.total,
    })))
}

#[derive(Deserialize)]
struct AddMemoryRequest {
    kind: String,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    /// Owning canonical companion for a private memory; absent = shared.
    #[serde(default)]
    scope_companion_id: Option<String>,
}

async fn add_memory(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<AddMemoryRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionMemory>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let scope = match req.scope_companion_id.as_deref() {
        Some(companion_id) => scope_from_parts(Some("companion"), Some(companion_id))?
            .expect("explicit scope"),
        None => MemoryScope::Shared,
    };
    Ok(Json(ApiResponse::ok(
        state.service.add_memory(&req.kind, &req.content, &req.tags, scope).await?,
    )))
}

#[derive(Deserialize)]
struct UpdateMemoryRequest {
    content: Option<String>,
    pinned: Option<bool>,
    status: Option<String>,
    /// `'user'` (shared) or `'companion'` (private). Present together with
    /// `scope_companion_id` to re-home a memory; both absent = scope unchanged.
    scope_kind: Option<String>,
    scope_companion_id: Option<String>,
}

async fn update_memory(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(memory_id): Path<String>,
    body: Result<Json<UpdateMemoryRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let scope = scope_from_parts(req.scope_kind.as_deref(), req.scope_companion_id.as_deref())?;
    state
        .service
        .update_memory(
            &memory_id,
            req.content.as_deref(),
            req.pinned,
            req.status.as_deref(),
            scope,
        )
        .await?;
    Ok(Json(ApiResponse::ok(())))
}

async fn delete_memory(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(memory_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.delete_memory(&memory_id).await?;
    Ok(Json(ApiResponse::ok(())))
}

#[derive(Deserialize)]
struct BatchMemoriesRequest {
    ids: Vec<String>,
    /// `archive` | `restore` | `delete` | `reclassify`.
    action: String,
    /// Target kind — required for `reclassify`, ignored otherwise.
    #[serde(default)]
    kind: Option<String>,
}

/// Atomic batch memory operation (single transaction; any bad id rolls back).
async fn batch_memories(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<BatchMemoriesRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let action = match req.action.as_str() {
        "archive" => MemoryBatchAction::Archive,
        "restore" => MemoryBatchAction::Restore,
        "delete" => MemoryBatchAction::Delete,
        "reclassify" => MemoryBatchAction::Reclassify {
            kind: req.kind.filter(|kind| !kind.is_empty()).ok_or_else(|| {
                AppError::BadRequest("batch reclassify requires a target kind".into())
            })?,
        },
        other => {
            return Err(AppError::BadRequest(format!("invalid batch action '{other}'")));
        }
    };
    state.service.batch_memories(&req.ids, &action).await?;
    Ok(Json(ApiResponse::ok(())))
}

/// Merge-assistant dry run: suspected-duplicate groups (active layer only).
async fn memory_merge_suggestions(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<MemoryMergeGroup>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.memory_merge_suggestions().await?)))
}

#[derive(Deserialize)]
struct MergeMemoriesRequest {
    group: Vec<String>,
    merged_content: String,
    kind: String,
}

/// Merge-assistant confirm: insert the merged memory, archive the source group.
async fn merge_memories(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<MergeMemoriesRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionMemory>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .merge_memories(&req.group, &req.merged_content, &req.kind)
            .await?,
    )))
}

#[derive(Deserialize)]
struct ListSkillsQuery {
    include_shared: Option<bool>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_companion_skills(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    Query(q): Query<ListSkillsQuery>,
) -> Result<Json<ApiResponse<CompanionSkillViewPage>>, AppError> {
    let status = q.status.filter(|s| !s.is_empty());
    let page = state
        .service
        .list_companion_skill_page(
            &companion_id,
            q.include_shared.unwrap_or(true),
            status.as_deref(),
            q.limit.unwrap_or(100),
            q.offset.unwrap_or(0),
        )
        .await?;
    Ok(Json(ApiResponse::ok(page)))
}

#[derive(Deserialize)]
struct DigestQuery {
    days: Option<i64>,
}

async fn weekly_digest(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    Query(q): Query<DigestQuery>,
) -> Result<Json<ApiResponse<CompanionWeeklyDigest>>, AppError> {
    let days = q.days.unwrap_or(7).clamp(1, 90);
    let since_ms = nomifun_common::now_ms() - days * 86_400_000;
    Ok(Json(ApiResponse::ok(state.service.weekly_digest(&companion_id, since_ms).await?)))
}

#[derive(Deserialize)]
struct DayDigestsQuery {
    /// Inclusive `YYYYMMDD` lower bound (empty/absent = open).
    since: Option<String>,
    /// Inclusive `YYYYMMDD` upper bound (empty/absent = open).
    until: Option<String>,
    /// "去年今日" mode: a 4-char `MMDD`; when set, returns same-day-of-year
    /// archived digests (excluding today), ignoring `since`/`until`.
    on_day: Option<String>,
    limit: Option<i64>,
}

/// Archived session-window day-digests for a companion (伙伴会话归档回看时间线数据源).
async fn list_day_digests(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    Query(q): Query<DayDigestsQuery>,
) -> Result<Json<ApiResponse<Vec<crate::store::SessionWindow>>>, AppError> {
    let limit = q.limit.unwrap_or(60).clamp(1, 365);
    let digests = if let Some(mmdd) = q.on_day.filter(|s| s.len() == 4) {
        let today = crate::store::local_day(nomifun_common::now_ms());
        state.service.digests_on_this_day(&companion_id, &mmdd, &today, limit).await?
    } else {
        state
            .service
            .list_day_digests(
                &companion_id,
                q.since.as_deref().unwrap_or(""),
                q.until.as_deref().unwrap_or(""),
                limit,
            )
            .await?
    };
    Ok(Json(ApiResponse::ok(digests)))
}

async fn get_companion_skill(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path((companion_id, companion_skill_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<CompanionSkillContent>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state
            .service
            .get_companion_skill_content(&companion_id, &companion_skill_id)
            .await?,
    )))
}

#[derive(Deserialize)]
struct UpdateSkillRequest {
    content: String,
}

async fn update_companion_skill(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path((companion_id, companion_skill_id)): Path<(String, String)>,
    body: Result<Json<UpdateSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .service
        .write_companion_skill_content(&companion_id, &companion_skill_id, &req.content)
        .await?;
    Ok(Json(ApiResponse::ok(())))
}

#[derive(Deserialize)]
struct DecideSkillRequest {
    accept: bool,
    reason: Option<String>,
}

async fn decide_companion_skill(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path((companion_id, companion_skill_id)): Path<(String, String)>,
    body: Result<Json<DecideSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionSkill>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .decide_companion_skill(
                &companion_id,
                &companion_skill_id,
                req.accept,
                req.reason.as_deref(),
            )
            .await?,
    )))
}

#[derive(Deserialize)]
struct FromSessionRequest {
    conversation_id: String,
}

async fn draft_skill_from_session(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<FromSessionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Option<String>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state.service.draft_skill_from_session(&companion_id, &req.conversation_id).await?,
    )))
}

#[derive(Deserialize)]
struct GiftSkillRequest {
    to_companion_id: String,
}

async fn gift_companion_skill(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path((companion_id, companion_skill_id)): Path<(String, String)>,
    body: Result<Json<GiftSkillRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionSkill>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .gift_companion_skill(
                &companion_id,
                &companion_skill_id,
                &req.to_companion_id,
            )
            .await?,
    )))
}

async fn run_learn(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<CompanionLearnResult>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.run_learn_now().await?)))
}

async fn event_stats(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<SourceStats>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.event_stats().await?)))
}

async fn event_storage(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<crate::collector::EventStorageStatus>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.event_storage().await?)))
}

async fn apply_consent(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<SharedCompanionConfig>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.apply_default_on_consent().await?)))
}

async fn disable_all(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<SharedCompanionConfig>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.disable_all().await?)))
}

// ----- companions -----

/// One companion card: profile fields flattened at the top level plus that companion's
/// live status — list/detail fetch everything for a card in one round trip.
#[derive(serde::Serialize)]
struct CompanionWithStatus {
    #[serde(flatten)]
    profile: CompanionProfileConfig,
    status: CompanionStatus,
}

async fn list_companions(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<CompanionWithStatus>>>, AppError> {
    let mut companions = Vec::new();
    for profile in state.service.list_companions().await {
        match state.service.companion_status(&profile.companion_id).await {
            Ok(status) => companions.push(CompanionWithStatus { profile, status }),
            // The companion vanished between list and status (concurrent delete):
            // drop the card rather than failing the whole list.
            Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Json(ApiResponse::ok(companions)))
}

#[derive(Deserialize)]
struct CreateCompanionRequest {
    name: String,
    /// Empty/missing falls back to the default roster character.
    #[serde(default)]
    character: String,
}

async fn create_companion(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCompanionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let profile = state.service.create_companion(&req.name, &req.character).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(profile))))
}

async fn get_companion(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<CompanionWithStatus>>, AppError> {
    let profile = state.service.get_companion(&companion_id).await?;
    let status = state.service.companion_status(&companion_id).await?;
    Ok(Json(ApiResponse::ok(CompanionWithStatus { profile, status })))
}

/// RFC 7396 merge patch over one companion's profile.
async fn patch_companion(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionProfileConfig>>, AppError> {
    let Json(patch) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.patch_companion(&companion_id, patch).await?)))
}

#[derive(Deserialize)]
struct ApplyPresetRequest {
    preset_id: String,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    overrides: nomifun_api_types::PresetOverrides,
}

async fn apply_preset(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<ApplyPresetRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CompanionProfileConfig>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let presets = state
        .preset_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("preset service is not wired".into()))?;
    let snapshot = presets
        .resolve(
            &req.preset_id,
            nomifun_api_types::PresetTarget::Companion,
            req.locale.as_deref(),
            req.overrides,
        )
        .await?;
    if let Some(knowledge) = state.knowledge_service.as_ref() {
        let mode = if snapshot.knowledge_policy.mode == "direct" { "direct" } else { "staged" };
        knowledge
            .set_binding(
                "companion",
                &companion_id,
                nomifun_knowledge::KnowledgeBinding {
                    enabled: snapshot.knowledge_policy.enabled,
                    writeback: snapshot.knowledge_policy.writeback,
                    writeback_mode: mode.to_owned(),
                    writeback_eagerness: snapshot
                        .knowledge_policy
                        .eagerness
                        .clone()
                        .unwrap_or_else(|| "conservative".to_owned()),
                    channel_write_enabled: false,
                    kb_ids: snapshot.knowledge_base_ids.clone(),
                },
            )
            .await?;
    }
    Ok(Json(ApiResponse::ok(
        state.service.apply_preset_snapshot(&companion_id, snapshot).await?,
    )))
}

async fn delete_companion(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_companion(&companion_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn companion_status(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<CompanionStatus>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.companion_status(&companion_id).await?)))
}

// ----- DIY custom figure (spec §3 存储与回显) -----

#[derive(Deserialize)]
struct UploadFigureRequest {
    /// Temp path returned by `POST /api/fs/upload` (two-phase upload).
    source_path: String,
}

async fn upload_figure(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<UploadFigureRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.ingest_figure(&companion_id, &req.source_path).await?;
    Ok(Json(ApiResponse::ok(())))
}

/// Binary serve of one companion's figure (the nomifun-assets Response template,
/// disk-backed). `Cache-Control: no-cache` + a `"{mtime}-{len}"` ETag: the
/// browser revalidates every time and gets a cheap 304 until re-upload.
async fn get_figure(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (bytes, mtime) = state.service.read_figure(&companion_id).await?;
    let etag = format!("\"{}-{}\"", mtime, bytes.len());

    let if_none_match_hits = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').map(str::trim).any(|c| c == etag || c == "*"));
    if if_none_match_hits {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::ETAG, etag)
            .body(Body::empty())
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, crate::figure::content_type_of(&bytes))
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, etag)
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Binary serve of the cached MODNet matting model, downloading it from a
/// mirror on first use (see [`crate::matting_model`]). The renderer fetches
/// this from `127.0.0.1` and mirrors it into Cache Storage, so the matting
/// Web Worker reads a local copy instead of hitting huggingface behind a 30 s
/// timeout. Immutable + long-lived: the filename is versioned, so the browser
/// may cache it forever.
async fn get_matting_model(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Response, AppError> {
    let bytes = state.service.matting_model_bytes().await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ----- custom-figure library (decoupled from companions) -----

async fn list_figures(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<crate::figures::FigureMeta>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list_figures().await?)))
}

#[derive(Deserialize)]
struct CreateFigureRequest {
    /// Temp path returned by `POST /api/fs/upload` (two-phase upload).
    source_path: String,
    #[serde(default)]
    name: String,
    aspect: f32,
    head_box: HeadBox,
    #[serde(default)]
    size_tier: String,
}

async fn create_figure(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateFigureRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let figure = state
        .service
        .create_figure(&req.source_path, &req.name, req.aspect, req.head_box, &req.size_tier)
        .await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(figure))))
}

#[derive(Deserialize)]
struct UpdateFigureRequest {
    name: Option<String>,
    head_box: Option<HeadBox>,
    size_tier: Option<String>,
}

async fn update_figure(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(figure_id): Path<String>,
    body: Result<Json<UpdateFigureRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<crate::figures::FigureMeta>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.update_figure(
        &figure_id,
        crate::figures::FigureUpdate { name: req.name, head_box: req.head_box, size_tier: req.size_tier },
    ).await?)))
}

async fn delete_figure(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(figure_id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.service.delete_figure(&figure_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Binary serve of one library figure's image (same ETag/no-cache template as
/// the per-companion `get_figure`).
///
/// AUTH-EXEMPT route (see `companion_public_routes`): native `<img>` loads carry
/// no trust header, so `trust_resolve_middleware` injects NO `CurrentUser` for
/// them. This handler therefore MUST NOT extract `Extension<CurrentUser>` — that
/// extractor would 500 on the very (untrusted-header) requests this route exists
/// to serve. The figure id is the opaque capability; no user identity is needed.
async fn get_figure_image(
    State(state): State<CompanionRouterState>,
    Path(figure_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (bytes, mtime) = state.service.read_figure_image(&figure_id).await?;
    let etag = format!("\"{}-{}\"", mtime, bytes.len());

    let if_none_match_hits = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').map(str::trim).any(|c| c == etag || c == "*"));
    if if_none_match_hits {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::ETAG, etag)
            .body(Body::empty())
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, crate::figure::content_type_of(&bytes))
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, etag)
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ----- companion thread (per companion, single session) -----

#[derive(Deserialize)]
struct CreateThreadRequest {
    #[serde(default)]
    title: Option<String>,
}

/// Idempotent ensure of the companion's single companion session: returns the
/// existing one, or creates it (requires the companion's model to be configured).
async fn create_thread(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<CreateThreadRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<crate::store::CompanionThread>>, AppError> {
    let title = body.map(|Json(b)| b.title).unwrap_or_default();
    Ok(Json(ApiResponse::ok(
        state.service.create_companion_thread(&companion_id, title).await?,
    )))
}

#[derive(serde::Serialize)]
struct ActiveThreadResponse {
    conversation_id: Option<String>,
}

/// The companion's single companion session id (or null when none exists yet).
async fn get_active_thread(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
) -> Result<Json<ApiResponse<ActiveThreadResponse>>, AppError> {
    // Existence gate: an unknown companion must 404, not read as "no active thread".
    state.service.get_companion(&companion_id).await?;
    Ok(Json(ApiResponse::ok(ActiveThreadResponse {
        conversation_id: state.service.companion_active_thread(&companion_id).await?,
    })))
}

// ----- cross-machine bundle export / import (§4.8) -----

/// The live shared store + shared dir for export/import. `CompanionService` keeps
/// its store private, so the boot-time registration in `crate::store` is the
/// only crate-visible handle. `None` means the production store was not opened,
/// so the endpoints refuse to operate.
fn live_store() -> Result<(&'static std::path::Path, &'static crate::store::CompanionStore), AppError> {
    crate::store::live_store()
        .ok_or_else(|| AppError::Internal("伙伴持久化存储尚未初始化，无法导入导出".into()))
}

#[derive(Deserialize)]
struct ExportMemoryRequest {
    dest_path: String,
    #[serde(default)]
    include_events: bool,
}

async fn export_memory(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ExportMemoryRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<crate::export::ExportSummary>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let summary = state
        .service
        .export_memory_bundle(
        std::path::Path::new(&req.dest_path),
        req.include_events,
    )
    .await?;
    Ok(Json(ApiResponse::ok(summary)))
}

#[derive(Deserialize)]
struct ExportCompanionRequest {
    dest_path: String,
    /// Names of the knowledge bases bound to this companion, collected by the
    /// frontend (the companion crate never reaches into the knowledge domain).
    #[serde(default)]
    knowledge_names: Vec<String>,
}

async fn export_companion(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(companion_id): Path<String>,
    body: Result<Json<ExportCompanionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<crate::export::ExportSummary>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    // Existence gate: an unknown companion must 404 before any file is written.
    let profile = state.service.get_companion(&companion_id).await?;
    let (_, store) = live_store()?;
    let summary = crate::export::export_companion_bundle(
        store,
        &profile,
        std::path::Path::new(&req.dest_path),
        &req.knowledge_names,
    )
    .await?;
    Ok(Json(ApiResponse::ok(summary)))
}

#[derive(Deserialize)]
struct ImportPackageRequest {
    src_path: String,
}

async fn import_package(
    State(state): State<CompanionRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ImportPackageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<crate::export::ImportOutcome>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let outcome = state.service.import_bundle(std::path::Path::new(&req.src_path)).await?;
    Ok(Json(ApiResponse::ok(outcome)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use nomifun_realtime::BroadcastEventBus;
    use tower::ServiceExt;

    use super::*;
    use crate::learner::CompanionCompleter;
    use crate::service::CompanionService;

    struct NoopCompleter;

    #[async_trait::async_trait]
    impl CompanionCompleter for NoopCompleter {
        async fn complete(&self, _provider_id: &str, _model: &str, _system: &str, _user: &str, _max_tokens: u32) -> Result<String, AppError> {
            Ok(String::new())
        }
    }

    async fn test_app(data_dir: &std::path::Path) -> (Router, Arc<CompanionService>) {
        let service = CompanionService::start(
            data_dir,
            Arc::new(BroadcastEventBus::new(16)),
            "owner-a",
            Arc::new(NoopCompleter),
            Arc::new(nomifun_extension::skill_service::resolve_skill_paths(data_dir, data_dir)),
        )
        .await
        .unwrap();
        let app = companion_routes(CompanionRouterState::new(service.clone())).layer(
            Extension(CurrentUser {
                id: nomifun_common::UserId::new(),
                username: "u1".into(),
            }),
        );
        (app, service)
    }

    async fn json_body(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn learn_endpoint_returns_only_a_transient_result_and_history_route_is_retired() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = test_app(dir.path()).await;

        let response = app
            .clone()
            .oneshot(
                Request::post("/api/companion/learn/run")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let data = body["data"].as_object().unwrap();
        let actual_keys: std::collections::BTreeSet<&str> =
            data.keys().map(String::as_str).collect();
        let expected_keys: std::collections::BTreeSet<&str> = [
            "error",
            "events_processed",
            "memories_added",
            "status",
            "summary",
        ]
        .into_iter()
        .collect();
        assert_eq!(actual_keys, expected_keys);
        assert_eq!(data["status"], "model_unconfigured");

        let retired = app
            .oneshot(
                Request::get("/api/companion/learn/runs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retired.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn event_storage_reports_policy_and_raw_viewer_clear_routes_are_retired() {
        let dir = tempfile::tempdir().unwrap();
        let (app, service) = test_app(dir.path()).await;
        let events_dir = dir
            .path()
            .join(crate::COMPANION_SHARED_REL_DIR)
            .join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        std::fs::write(events_dir.join("20260804.jsonl"), b"newest\n").unwrap();
        std::fs::write(events_dir.join("20260802.jsonl"), b"old\n").unwrap();

        let response = app
            .clone()
            .oneshot(Request::get("/api/companion/events/storage").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(
            body["data"],
            serde_json::json!({
                "total_bytes": 11,
                "max_bytes": 64 * 1024 * 1024,
                "file_count": 2,
                "oldest_day": "2026-08-02",
                "newest_day": "2026-08-04",
                "retention_days": 30,
                "max_storage_mb": 64
            })
        );

        let valid_patch = app
            .clone()
            .oneshot(
                Request::patch("/api/companion/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "collect": {
                                "event_retention_days": 7,
                                "event_max_storage_mb": 16
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(valid_patch.status(), StatusCode::OK);
        let status = service.event_storage().await.unwrap();
        assert_eq!(status.retention_days, 7);
        assert_eq!(status.max_storage_mb, 16);

        let invalid_patch = app
            .clone()
            .oneshot(
                Request::patch("/api/companion/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"collect": {"event_max_storage_mb": 15}}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_patch.status(), StatusCode::BAD_REQUEST);
        assert_eq!(service.get_config().await.collect.event_max_storage_mb, 16);

        for request in [
            Request::get("/api/companion/events/recent").body(Body::empty()).unwrap(),
            Request::delete("/api/companion/events").body(Body::empty()).unwrap(),
        ] {
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn batch_endpoint_applies_all_actions_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let (app, service) = test_app(dir.path()).await;
        let a = service.add_memory("episode", "上周试了埃塞俄比亚豆", &[], MemoryScope::Shared).await.unwrap();
        let b = service.add_memory("episode", "昨天喝了危地马拉豆", &[], MemoryScope::Shared).await.unwrap();

        // archive both
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str(), b.memory_id.as_str()], "action": "archive" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(service.store.count_memories("archived").await.unwrap(), 2);

        // restore both
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str(), b.memory_id.as_str()], "action": "restore" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(service.store.count_memories("active").await.unwrap(), 2);

        // reclassify one; an invalid kind is a 400 and changes nothing
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str()], "action": "reclassify", "kind": "knowledge" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(service.store.get_memory(&a.memory_id).await.unwrap().unwrap().kind, "knowledge");
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str()], "action": "reclassify", "kind": "bogus" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // atomic: one bad id rolls the whole delete back
        let missing = nomifun_common::CompanionMemoryId::new().into_string();
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str(), missing.as_str()], "action": "delete" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(service.store.get_memory(&a.memory_id).await.unwrap().is_some(), "failed batch must roll back");

        // delete both
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/batch",
                serde_json::json!({ "ids": [a.memory_id.as_str(), b.memory_id.as_str()], "action": "delete" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(service.store.count_memories("active").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn merge_flow_groups_then_archives_sources_with_audit_tag() {
        let dir = tempfile::tempdir().unwrap();
        let (app, service) = test_app(dir.path()).await;
        // Two normalized-similar actives of one kind (dedup guard skips exact
        // duplicates, so use containment ≥0.6 variants) + one unrelated.
        let a = service.store.insert_memory("preference", "主人喜欢深烘焙咖啡", &[], 0.8, "manual").await.unwrap();
        let b = service.store.insert_memory("preference", "主人喜欢深烘焙咖啡豆手冲", &[], 0.6, "manual").await.unwrap();
        let other = service.store.insert_memory("task", "帮主人整理周报", &[], 0.8, "manual").await.unwrap();

        let resp = app
            .clone()
            .oneshot(post_json("/api/companion/memories/merge-suggestions", serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = json_body(resp).await;
        let groups = v["data"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "exactly one duplicate group: {v}");
        let ids: Vec<&str> = groups[0]["memories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["memory_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&a.memory_id.as_str()) && ids.contains(&b.memory_id.as_str()));
        assert!(!ids.contains(&other.memory_id.as_str()));

        // Confirm merge: merged row active, sources archived + audit-tagged.
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/merge",
                serde_json::json!({
                    "group": [a.memory_id.as_str(), b.memory_id.as_str()],
                    "merged_content": "主人喜欢深烘焙咖啡豆，常用手冲",
                    "kind": "preference",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = json_body(resp).await;
        let merged_id = v["data"]["memory_id"].as_str().unwrap().to_owned();
        assert_eq!(v["data"]["status"], "active");
        assert_eq!(v["data"]["source"], "merge");
        for source in [&a.memory_id, &b.memory_id] {
            let row = service.store.get_memory(source).await.unwrap().unwrap();
            assert_eq!(row.status, "archived");
            assert!(
                row.tags.iter().any(|t| t == &format!("superseded_by:{merged_id}")),
                "source must carry the audit tag: {:?}",
                row.tags
            );
        }

        // Invalid kind → 400.
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/companion/memories/merge",
                serde_json::json!({ "group": [a.memory_id.as_str(), b.memory_id.as_str()], "merged_content": "x", "kind": "bogus" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_q_walks_fts_with_snippets_and_sort_and_status_all() {
        let dir = tempfile::tempdir().unwrap();
        let (app, service) = test_app(dir.path()).await;
        // Controlled timestamps/status via the raw insert path so the sort
        // assertions are deterministic (the public paths stamp now_ms()).
        let raw = |content: &str, importance: f64, status: &str, updated_at: i64| CompanionMemory {
            memory_id: nomifun_common::CompanionMemoryId::new().into_string(),
            kind: "episode".into(),
            content: content.into(),
            tags: vec![],
            importance,
            strength: importance,
            pinned: false,
            source: "manual".into(),
            status: status.into(),
            created_at: updated_at,
            updated_at,
            last_reinforced_at: updated_at,
            scope_kind: "user".into(),
            scope_companion_id: None,
        };
        let old_archived = raw("主人上月研究了咖啡烘焙曲线", 0.9, "archived", 1_000);
        let new_active = raw("主人今天又聊起咖啡豆产区", 0.2, "active", 2_000);
        service.store.insert_memory_raw(&old_archived).await.unwrap();
        service.store.insert_memory_raw(&new_active).await.unwrap();

        // q + status=all: both layers found (咖啡 is a 2-char LIKE-fallback term).
        let req = Request::get("/api/companion/memories?q=%E5%92%96%E5%95%A1&status=all")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = json_body(resp).await;
        assert_eq!(v["data"]["total"], 2, "{v}");
        // Default status stays active-only.
        let req = Request::get("/api/companion/memories?q=%E5%92%96%E5%95%A1")
            .body(Body::empty())
            .unwrap();
        let v = json_body(app.clone().oneshot(req).await.unwrap()).await;
        assert_eq!(v["data"]["total"], 1, "{v}");

        // A 3+ char query walks the trigram index and carries a highlight snippet.
        let req = Request::get("/api/companion/memories?q=%E5%92%96%E5%95%A1%E7%83%98%E7%84%99&status=all")
            .body(Body::empty())
            .unwrap();
        let v = json_body(app.clone().oneshot(req).await.unwrap()).await;
        assert_eq!(v["data"]["total"], 1, "{v}");
        assert!(
            v["data"]["items"][0]["snippet"].as_str().unwrap().contains("<b>"),
            "FTS hits carry a highlight snippet: {v}"
        );

        // sort=time puts the newest first even though it ranks lower.
        let req = Request::get("/api/companion/memories?q=%E5%92%96%E5%95%A1&status=all&sort=time")
            .body(Body::empty())
            .unwrap();
        let v = json_body(app.clone().oneshot(req).await.unwrap()).await;
        assert!(
            v["data"]["items"][0]["content"].as_str().unwrap().contains("产区"),
            "time sort puts the newest first: {v}"
        );

        // An unknown sort value is a 400.
        let req = Request::get("/api/companion/memories?q=x&sort=bogus").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::BAD_REQUEST);

        // Non-q path still lists (sort honored, no snippet key).
        let req = Request::get("/api/companion/memories?status=all&sort=importance")
            .body(Body::empty())
            .unwrap();
        let v = json_body(app.oneshot(req).await.unwrap()).await;
        assert_eq!(v["data"]["total"], 2);
        assert!(v["data"]["items"][0].get("snippet").is_none());
        assert!(
            v["data"]["items"][0]["content"].as_str().unwrap().contains("烘焙曲线"),
            "importance sort puts the 0.9-importance row first: {v}"
        );
    }
}
