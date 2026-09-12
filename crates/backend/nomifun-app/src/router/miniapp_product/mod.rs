//! User-facing MiniApp workflow. Authoring stays in recoverable draft documents;
//! only the explicit save command commits a production release.
mod authoring;
#[cfg(test)]
mod tests;
mod transfer;

use super::miniapp_m1::{MiniAppM1RouterState, application_error};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use nomifun_api_types::{
    ApiResponse, BuildMiniAppRequest, CreateMiniAppProjectRequest, MiniAppKindDto,
    MiniAppWorkshopDto, PublishMiniAppRequest, ReplaceMiniAppSourceFileRequest,
    SetMiniAppEnabledRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_db::MiniAppProductDocuments;
use nomifun_miniapp_platform::MiniAppM1ApplicationService;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct MiniAppProductService {
    documents: MiniAppProductDocuments,
    application: Arc<MiniAppM1ApplicationService>,
    model: Arc<nomifun_model_invoke::ModelInvokeService>,
    root: PathBuf,
    mutations: Arc<Mutex<()>>,
    jobs: Arc<Mutex<BTreeMap<String, CancellationToken>>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Collection {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Item {
    #[serde(default)]
    pub collection_id: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub last_opened: i64,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Workspace {
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub items: BTreeMap<String, Item>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Draft {
    pub id: String,
    pub revision: i64,
    pub name: String,
    pub description: String,
    pub html: String,
    #[serde(default)]
    pub service_source: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub status: String,
    pub error: Option<String>,
    pub miniapp_id: Option<String>,
    pub base_release_digest: Option<String>,
    #[serde(default)]
    pub base_source_digest: Option<String>,
    pub updated_at: i64,
    #[serde(default)]
    pub import: Option<transfer::ImportSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpectedRevision {
    pub expected_revision: i64,
}

impl MiniAppProductService {
    pub(crate) fn new(
        documents: MiniAppProductDocuments,
        application: Arc<MiniAppM1ApplicationService>,
        model: Arc<nomifun_model_invoke::ModelInvokeService>,
        root: PathBuf,
    ) -> Self {
        Self {
            documents,
            application,
            model,
            root,
            mutations: Arc::new(Mutex::new(())),
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    async fn read<T: DeserializeOwned>(
        &self,
        owner: &str,
        key: &str,
    ) -> Result<Option<T>, AppError> {
        self.documents
            .get(owner, key)
            .await?
            .map(|(_, json)| {
                serde_json::from_str(&json).map_err(|e| AppError::Internal(e.to_string()))
            })
            .transpose()
    }

    async fn draft(&self, owner: &str, id: &str) -> Result<Draft, AppError> {
        valid_id(id)?;
        self.read(owner, &format!("draft:{id}"))
            .await?
            .ok_or_else(|| AppError::NotFound("MiniApp draft".into()))
    }

    async fn put_draft(&self, owner: &str, draft: &mut Draft) -> Result<(), AppError> {
        let expected = draft.revision;
        let mut next = draft.clone();
        next.revision = expected
            .checked_add(1)
            .ok_or_else(|| invalid("Invalid draft revision"))?;
        next.updated_at = nomifun_common::now_ms();
        let json = serde_json::to_string(&next).map_err(internal)?;
        self.documents
            .put(owner, &format!("draft:{}", draft.id), expected, &json)
            .await?;
        *draft = next;
        Ok(())
    }

    async fn workspace(&self, owner: &str) -> Result<Workspace, AppError> {
        Ok(self.read(owner, "library").await?.unwrap_or_default())
    }
}

pub(crate) fn read_routes() -> Router<MiniAppM1RouterState> {
    Router::new()
        .route("/api/miniapps/workspace", get(get_workspace))
        .route("/api/miniapps/drafts", get(list_drafts))
        .route("/api/miniapps/drafts/{draft_id}", get(get_draft))
}

pub(crate) fn write_routes() -> Router<MiniAppM1RouterState> {
    Router::new()
        .route("/api/miniapps/workspace", post(update_workspace))
        .route("/api/miniapps/authoring", post(authoring::generate))
        .route("/api/miniapps/drafts/{draft_id}/cancel", post(cancel))
        .route("/api/miniapps/drafts/{draft_id}/save", post(save))
        .route("/api/miniapps/drafts/{draft_id}/discard", post(discard))
        .route("/api/miniapps/import/inspect", post(transfer::inspect))
        .route(
            "/api/miniapps/{miniapp_id}/export-file",
            post(transfer::export_file),
        )
}

fn service(state: &MiniAppM1RouterState) -> Result<Arc<MiniAppProductService>, AppError> {
    state
        .product
        .clone()
        .ok_or_else(|| AppError::Internal("MiniApp authoring unavailable".into()))
}
fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}
fn invalid(message: &str) -> AppError {
    AppError::BadRequest(message.into())
}
fn valid_id(id: &str) -> Result<(), AppError> {
    let value = uuid::Uuid::parse_str(id).map_err(|_| invalid("Invalid draft identity"))?;
    if value.get_version_num() != 7 || value.to_string() != id {
        return Err(invalid("Invalid draft identity"));
    }
    Ok(())
}
fn check_revision(draft: &Draft, expected: i64) -> Result<(), AppError> {
    if draft.revision != expected {
        return Err(AppError::RevisionConflict(
            "The draft changed; reload it before retrying".into(),
        ));
    }
    Ok(())
}

async fn get_workspace(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Workspace>>, AppError> {
    Ok(Json(ApiResponse::ok(
        service(&state)?.workspace(user.id.as_str()).await?,
    )))
}

async fn update_workspace(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(mut value): Json<Workspace>,
) -> Result<Json<ApiResponse<Workspace>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    if value.collections.len() > 200 || value.items.len() > 10000 {
        return Err(invalid("Too many MiniApp collections or items"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for collection in &mut value.collections {
        valid_id(&collection.id)?;
        collection.name = collection.name.trim().to_owned();
        if collection.name.is_empty()
            || collection.name.chars().count() > 60
            || !ids.insert(collection.id.clone())
            || !names.insert(collection.name.clone())
        {
            return Err(invalid(
                "Collection names must be unique and contain 1–60 characters",
            ));
        }
    }
    let library = service
        .application
        .library(user.id.as_str())
        .await
        .map_err(application_error)?;
    let owned = library
        .miniapps
        .iter()
        .map(|a| a.miniapp_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    // Permanently removed applications must not keep stale pins or membership.
    value.items.retain(|id, _| owned.contains(id.as_str()));
    for item in value.items.values_mut() {
        if item
            .collection_id
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
        {
            item.collection_id = None;
        }
        if let Some(name) = &mut item.name {
            *name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 120 {
                return Err(invalid("Invalid MiniApp name"));
            }
        }
        if item.last_opened < 0 {
            return Err(invalid("Invalid last-opened time"));
        }
    }
    let expected = value.revision;
    value.revision = expected
        .checked_add(1)
        .ok_or_else(|| invalid("Invalid workspace revision"))?;
    service
        .documents
        .put(
            user.id.as_str(),
            "library",
            expected,
            &serde_json::to_string(&value).map_err(internal)?,
        )
        .await?;
    Ok(Json(ApiResponse::ok(value)))
}

async fn get_draft(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let mut draft = service.draft(user.id.as_str(), &id).await?;
    if draft.status == "generating"
        && !service
            .jobs
            .lock()
            .await
            .contains_key(&format!("{}:{id}", user.id))
    {
        draft.status = "interrupted".into();
        draft.error = Some("generation_interrupted".into());
        service.put_draft(user.id.as_str(), &mut draft).await?;
    }
    Ok(Json(ApiResponse::ok(draft)))
}

async fn list_drafts(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<Draft>>>, AppError> {
    let service = service(&state)?;
    let mut drafts = Vec::new();
    for (_, _, json) in service.documents.list(user.id.as_str(), "draft:").await? {
        let mut draft: Draft = serde_json::from_str(&json).map_err(internal)?;
        if draft.status == "saved" {
            continue;
        }
        // List responses contain only the metadata needed by the library.
        draft.html.clear();
        draft.service_source = None;
        draft.messages.clear();
        draft.import = None;
        drafts.push(draft);
    }
    Ok(Json(ApiResponse::ok(drafts)))
}

async fn cancel(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ExpectedRevision>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let mut draft = service.draft(user.id.as_str(), &id).await?;
    check_revision(&draft, request.expected_revision)?;
    if draft.status == "saving" {
        return Err(AppError::Conflict("Save is in progress".into()));
    }
    if let Some(token) = service
        .jobs
        .lock()
        .await
        .remove(&format!("{}:{id}", user.id))
    {
        token.cancel();
    }
    draft.status = "stopped".into();
    draft.error = None;
    service.put_draft(user.id.as_str(), &mut draft).await?;
    Ok(Json(ApiResponse::ok(draft)))
}

async fn discard(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ExpectedRevision>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let draft = service.draft(user.id.as_str(), &id).await?;
    check_revision(&draft, request.expected_revision)?;
    if draft.status == "saving" {
        return Err(AppError::Conflict("Save is in progress".into()));
    }
    if let Some(token) = service
        .jobs
        .lock()
        .await
        .remove(&format!("{}:{id}", user.id))
    {
        token.cancel();
    }
    service.cleanup_import_stage(&draft)?;
    service
        .documents
        .delete(user.id.as_str(), &format!("draft:{id}"), draft.revision)
        .await?;
    Ok(Json(ApiResponse::ok(true)))
}

async fn save(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ExpectedRevision>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let mut draft = service.draft(user.id.as_str(), &id).await?;
    check_revision(&draft, request.expected_revision)?;
    if draft.status == "generating" {
        return Err(AppError::Conflict(
            "The MiniApp is still being created".into(),
        ));
    }
    let result = service.save_draft(user.id.as_str(), &mut draft).await;
    match result {
        Ok(workshop) => {
            draft.status = "saved".into();
            draft.error = None;
            draft.base_release_digest = workshop
                .miniapp
                .releases
                .active
                .as_ref()
                .map(|r| r.release_digest.clone());
            service.put_draft(user.id.as_str(), &mut draft).await?;
            if let Err(error) = service.cleanup_import_stage(&draft) {
                tracing::warn!(%error,"MiniApp import staging cleanup deferred");
            }
            Ok(Json(ApiResponse::ok(workshop)))
        }
        Err(error) => {
            draft.status = "ready".into();
            draft.error = Some("save_failed".into());
            service.put_draft(user.id.as_str(), &mut draft).await?;
            Err(error)
        }
    }
}

impl MiniAppProductService {
    async fn save_draft(
        &self,
        owner: &str,
        draft: &mut Draft,
    ) -> Result<MiniAppWorkshopDto, AppError> {
        let mut current = if let Some(id) = &draft.miniapp_id {
            self.application
                .workshop(owner, id)
                .await
                .map_err(application_error)?
        } else if draft.import.is_some() {
            let workshop = self.commit_import(owner, draft).await?;
            draft.base_release_digest = workshop
                .miniapp
                .releases
                .active
                .as_ref()
                .map(|r| r.release_digest.clone());
            draft.miniapp_id = Some(workshop.miniapp.miniapp_id.clone());
            self.put_draft(owner, draft).await?;
            workshop
        } else {
            authoring::validate_html(&draft.html)?;
            let library = self
                .application
                .library(owner)
                .await
                .map_err(application_error)?;
            let workshop = self
                .application
                .create(
                    owner,
                    CreateMiniAppProjectRequest {
                        expected_library_revision: library.library_revision,
                        display_name: draft.name.clone(),
                        description: Some(draft.description.clone()),
                        kind: if draft.service_source.is_some() {
                            MiniAppKindDto::Service
                        } else {
                            MiniAppKindDto::UiOnly
                        },
                    },
                )
                .await
                .map_err(application_error)?;
            draft.miniapp_id = Some(workshop.miniapp.miniapp_id.clone());
            self.put_draft(owner, draft).await?;
            workshop
        };
        let active = current
            .miniapp
            .releases
            .active
            .as_ref()
            .map(|r| r.release_digest.clone());
        if active != draft.base_release_digest {
            return Err(AppError::RevisionConflict(
                "The saved MiniApp changed since this draft was opened".into(),
            ));
        }
        if draft.import.is_none() {
            if draft.base_source_digest.is_some()
                && current.source_snapshot_digest != draft.base_source_digest
            {
                return Err(AppError::RevisionConflict(
                    "The MiniApp source changed since this draft was opened".into(),
                ));
            }
            let mut changed = false;
            let mut files = vec![("ui/index.html", draft.html.clone())];
            if let Some(source) = &draft.service_source {
                files.push(("service/main.mjs", source.clone()));
            }
            for (path, content) in files {
                let existing = self
                    .application
                    .source_file(owner, &current.miniapp.miniapp_id, path)
                    .await
                    .map_err(application_error)?;
                if existing.content == content {
                    continue;
                }
                changed = true;
                current = self
                    .application
                    .replace_source_file(
                        owner,
                        ReplaceMiniAppSourceFileRequest {
                            miniapp_id: current.miniapp.miniapp_id.clone(),
                            expected_product_revision: current.miniapp.product_revision,
                            project_id: current.project_id.clone(),
                            expected_project_revision: current.project_revision,
                            expected_build_generation: current.build_generation,
                            expected_source_snapshot_digest: existing.source_snapshot_digest,
                            path: path.into(),
                            content,
                        },
                    )
                    .await
                    .map_err(application_error)?;
                draft.base_source_digest = current.source_snapshot_digest.clone();
                self.put_draft(owner, draft).await?;
            }
            if changed || current.miniapp.releases.active.is_none() || current.ready.is_some() {
                current = self
                    .application
                    .build(
                        owner,
                        BuildMiniAppRequest {
                            miniapp_id: current.miniapp.miniapp_id.clone(),
                            expected_product_revision: current.miniapp.product_revision,
                            project_id: current.project_id.clone(),
                            expected_project_revision: current.project_revision,
                            expected_build_generation: current.build_generation,
                            expected_source_snapshot_digest: current
                                .source_snapshot_digest
                                .clone()
                                .ok_or_else(|| invalid("Source unavailable"))?,
                            expected_dependency_lock_digest: current
                                .dependency_lock_digest
                                .clone()
                                .ok_or_else(|| invalid("Dependencies unavailable"))?,
                            service_lifecycle: current.service_lifecycle,
                        },
                    )
                    .await
                    .map_err(application_error)?;
            }
        }
        if let Some(ready) = &current.ready {
            if !ready.can_publish {
                return Err(invalid("The MiniApp is not ready to save"));
            }
            current = self
                .application
                .publish(
                    owner,
                    PublishMiniAppRequest {
                        miniapp_id: current.miniapp.miniapp_id.clone(),
                        expected_product_revision: current.miniapp.product_revision,
                        expected_pointer_revision: current.miniapp.releases.pointer_revision,
                        expected_active_release_epoch: current
                            .miniapp
                            .releases
                            .active_release_epoch,
                        ready_release_id: ready.release.release_id.clone(),
                        expected_ready_release_digest: ready.release.release_digest.clone(),
                        expected_active_release_digest: current
                            .miniapp
                            .releases
                            .active
                            .as_ref()
                            .map(|r| r.release_digest.clone()),
                        expected_service_test_receipt_id: None,
                        acknowledge_test_warning: matches!(ready.kind, MiniAppKindDto::Service),
                    },
                )
                .await
                .map_err(application_error)?;
            // Persist the exact saved release before enable; retry never republishes an old draft.
            draft.base_release_digest = current
                .miniapp
                .releases
                .active
                .as_ref()
                .map(|r| r.release_digest.clone());
            self.put_draft(owner, draft).await?;
        }
        if !matches!(
            current.miniapp.lifecycle,
            nomifun_api_types::MiniAppLifecycleDto::Enabled
        ) {
            current = self
                .application
                .set_enabled(
                    owner,
                    SetMiniAppEnabledRequest {
                        miniapp_id: current.miniapp.miniapp_id.clone(),
                        expected_product_revision: current.miniapp.product_revision,
                        expected_pointer_revision: current.miniapp.releases.pointer_revision,
                        expected_active_release_digest: current
                            .miniapp
                            .releases
                            .active
                            .as_ref()
                            .map(|r| r.release_digest.clone()),
                        enabled: true,
                    },
                )
                .await
                .map_err(application_error)?;
        }
        Ok(current)
    }
}
