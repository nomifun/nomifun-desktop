//! User-facing MiniApp workflow. Authoring stays in recoverable draft documents;
//! only the explicit save command commits a production release.
mod authoring;
#[cfg(test)]
mod tests;
mod transfer;

use super::plugin_runtime::{PluginRuntimeM1RouterState, application_error};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use nomifun_api_types::{
    ApiResponse, BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest, PluginRuntimeKindDto,
    PluginRuntimeWorkshopDto, PublishPluginRuntimeRequest, ReplacePluginRuntimeSourceFileRequest,
    SetPluginRuntimeEnabledRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_db::PluginProductDocuments;
use nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationService;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct PluginRuntimeProductService {
    documents: PluginProductDocuments,
    application: Arc<PluginRuntimeM1ApplicationService>,
    model: Arc<nomifun_model_invoke::ModelInvokeService>,
    root: PathBuf,
    mutations: Arc<Mutex<()>>,
    jobs: Arc<Mutex<BTreeMap<String, (CancellationToken, Option<String>)>>>,
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
    #[serde(default)]
    pub source_manifest: Option<serde_json::Value>,
    pub messages: Vec<ChatMessage>,
    pub status: String,
    pub error: Option<String>,
    #[serde(rename = "plugin_id")]
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

impl PluginRuntimeProductService {
    pub(super) async fn cancel_app_jobs(&self, owner: &str, miniapp_id: &str) {
        let prefix = format!("{owner}:");
        self.jobs.lock().await.retain(|key, (token, app)| {
            if key.starts_with(&prefix) && app.as_deref() == Some(miniapp_id) {
                token.cancel();
                false
            } else {
                true
            }
        });
    }
    pub(crate) fn new(
        documents: PluginProductDocuments,
        application: Arc<PluginRuntimeM1ApplicationService>,
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
            .ok_or_else(|| AppError::NotFound("Plugin draft".into()))
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

pub(crate) fn read_routes() -> Router<PluginRuntimeM1RouterState> {
    Router::new()
        .route("/api/plugins/workspace", get(get_workspace))
        .route("/api/plugins/drafts", get(list_drafts))
        .route("/api/plugins/drafts/{draft_id}", get(get_draft))
}

pub(crate) fn write_routes() -> Router<PluginRuntimeM1RouterState> {
    Router::new()
        .route("/api/plugins/workspace", post(update_workspace))
        .route("/api/plugins/authoring", post(authoring::generate))
        .route("/api/plugins/drafts/{draft_id}/cancel", post(cancel))
        .route("/api/plugins/drafts/{draft_id}/save", post(save))
        .route("/api/plugins/drafts/{draft_id}/discard", post(discard))
        .route("/api/plugins/runtimes/import/inspect", post(transfer::inspect))
        .route(
            "/api/plugins/runtimes/{miniapp_id}/export-file",
            post(transfer::export_file),
        )
}

fn service(state: &PluginRuntimeM1RouterState) -> Result<Arc<PluginRuntimeProductService>, AppError> {
    state
        .product
        .clone()
        .ok_or_else(|| AppError::Internal("Plugin authoring unavailable".into()))
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
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Workspace>>, AppError> {
    Ok(Json(ApiResponse::ok(
        service(&state)?.workspace(user.id.as_str()).await?,
    )))
}

async fn update_workspace(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(mut value): Json<Workspace>,
) -> Result<Json<ApiResponse<Workspace>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    if value.collections.len() > 200 || value.items.len() > 10000 {
        return Err(invalid("Too many Plugin collections or items"));
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
    let owned = service.documents.owned_plugin_ids(user.id.as_str()).await?
        .into_iter()
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
                return Err(invalid("Invalid Plugin name"));
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
    State(state): State<PluginRuntimeM1RouterState>,
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
    State(state): State<PluginRuntimeM1RouterState>,
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
    State(state): State<PluginRuntimeM1RouterState>,
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
    if let Some((token, _)) = service
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
    State(state): State<PluginRuntimeM1RouterState>,
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
    if let Some((token, _)) = service
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
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ExpectedRevision>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let mut draft = service.draft(user.id.as_str(), &id).await?;
    check_revision(&draft, request.expected_revision)?;
    if draft.status == "generating" {
        return Err(AppError::Conflict(
            "The Plugin is still being created".into(),
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
                tracing::warn!(%error,"Plugin import staging cleanup deferred");
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

impl PluginRuntimeProductService {
    async fn save_draft(
        &self,
        owner: &str,
        draft: &mut Draft,
    ) -> Result<PluginRuntimeWorkshopDto, AppError> {
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
            authoring::validate_plugin_source(&draft.html, draft.service_source.as_deref(), draft.source_manifest.as_ref())?;
            let library = self
                .application
                .library(owner)
                .await
                .map_err(application_error)?;
            let workshop = self
                .application
                .create(
                    owner,
                    CreatePluginRuntimeProjectRequest {
                        expected_library_revision: library.library_revision,
                        display_name: draft.name.clone(),
                        description: Some(draft.description.clone()),
                        kind: if draft.service_source.is_some() {
                            PluginRuntimeKindDto::Service
                        } else {
                            PluginRuntimeKindDto::UiOnly
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
                "The saved Plugin changed since this draft was opened".into(),
            ));
        }
        if draft.import.is_none() {
            if draft.base_source_digest.is_some()
                && current.source_snapshot_digest != draft.base_source_digest
            {
                return Err(AppError::RevisionConflict(
                    "The Plugin source changed since this draft was opened".into(),
                ));
            }
            let mut changed = false;
            let mut files = vec![("ui/index.html", draft.html.clone())];
            if let Some(source) = &draft.service_source {
                files.push(("service/main.mjs", source.clone()));
            }
            if let Some(manifest) = &draft.source_manifest {
                files.push(("nomifun.plugin.json", serde_json::to_string(manifest).map_err(internal)?));
            }
            for (path, content) in files {
                let existing = self
                    .application
                    .source_file(owner, &current.miniapp.miniapp_id, path)
                    .await;
                let existing = match existing {
                    Ok(file) => Some(file),
                    Err(nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationError::NotFound) if path == "nomifun.plugin.json" || (path == "ui/index.html" && content.is_empty()) => None,
                    Err(error) => return Err(application_error(error)),
                };
                if existing.as_ref().is_some_and(|file| file.content == content) {
                    continue;
                }
                if existing.is_none() && content.is_empty() { continue; }
                changed = true;
                current = self
                    .application
                    .replace_source_file(
                        owner,
                        ReplacePluginRuntimeSourceFileRequest {
                            miniapp_id: current.miniapp.miniapp_id.clone(),
                            expected_product_revision: current.miniapp.product_revision,
                            project_id: current.project_id.clone(),
                            expected_project_revision: current.project_revision,
                            expected_build_generation: current.build_generation,
                            expected_source_snapshot_digest: current.source_snapshot_digest.clone().ok_or_else(|| invalid("Source unavailable"))?,
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
                        BuildPluginRuntimeRequest {
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
        if let Some(ready) = current.ready.as_ref().filter(|ready| ready.service.is_some()) {
            current = self.application.test_ready_service(owner, nomifun_api_types::TestPluginRuntimeReleaseRequest {
                miniapp_id: current.miniapp.miniapp_id.clone(),
                expected_product_revision: current.miniapp.product_revision,
                expected_pointer_revision: current.miniapp.releases.pointer_revision,
                project_id: current.project_id.clone(), expected_project_revision: current.project_revision,
                expected_build_generation: current.build_generation,
                release_id: ready.release.release_id.clone(), expected_release_digest: ready.release.release_digest.clone(),
                expected_config_revision: current.config.config_revision,
                expected_credential_bindings_revision: current.credential_bindings_revision,
                resolved_test_input_digest: crate::cli::DEFAULT_PLUGIN_TEST_INPUT_DIGEST.into(),
            }).await.map_err(application_error)?;
            if current.ready.as_ref().is_none_or(|ready| ready.test.status != nomifun_api_types::PluginRuntimeTestStatusDto::Passed) {
                return Err(invalid("The plugin's background code did not pass validation. Its draft is saved; fix it before enabling."));
            }
        }
        if let Some(ready) = &current.ready {
            if !ready.can_publish {
                return Err(invalid("The Plugin is not ready to save"));
            }
            current = self
                .application
                .publish(
                    owner,
                    PublishPluginRuntimeRequest {
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
                        expected_service_test_receipt_id: ready.test.receipt_id.clone(),
                        acknowledge_test_warning: false,
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
            nomifun_api_types::PluginRuntimeLifecycleDto::Enabled
        ) {
            current = self
                .application
                .set_enabled(
                    owner,
                    SetPluginRuntimeEnabledRequest {
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
