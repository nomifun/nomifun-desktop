//! [`WorkshopService`] — the single handle used by `/api/creative-studio/*`
//! project routes and `/api/workshop/*` asset/legacy-canvas routes. Canonical
//! project documents live in SQLite; legacy canvas bodies and asset binaries
//! remain under the service data directory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use nomifun_common::{
    AppError, CreativeStudioProjectId, CreativeStudioWorkflowId, ProviderId,
    SharedProviderLifecycleBarrier, WorkshopAssetId, WorkshopCanvasId, now_ms,
};
use nomifun_db::{
    AssetSort, CreativeStudioProjectRow, IWorkshopRepository, ListAssetsParams,
    UpdateAssetParams, WorkshopAssetRow,
};
use serde_json::Value;

use crate::archive::{
    CREATIVE_STUDIO_ARCHIVE_MIME, CreativeArchiveAssetSnapshot,
    build_creative_project_archive, collect_document_asset_ids,
    parse_creative_project_archive, remap_creative_archive_for_import,
    sanitized_archive_origin,
};
use crate::creative_studio::{
    CreativeNodeData, CreativeProjectDocument, CreativeProjectSummary,
    MAX_CREATIVE_PROJECT_DOCUMENT_BYTES,
};
#[cfg(test)]
use crate::creative_studio::CREATIVE_STUDIO_SCHEMA;
use crate::dto::{WorkshopAsset, WorkshopCanvasMeta};
use crate::workflow::{CreativeWorkflowDefinitionV1, parse_workflow_row};
use crate::{
    DEFAULT_DOC, MAX_ASSET_BYTES, MAX_DOC_BYTES, WORKSHOP_REL_DIR, docscan, fsio, imagemeta,
    thumbnail,
};

/// A canvas plus its (opaque) doc — the `GET /canvases/{id}` payload.
pub struct CanvasWithDoc {
    pub meta: WorkshopCanvasMeta,
    pub doc: Value,
}

/// A canonical Creative Studio project and its validated v1 document.
pub struct CreativeProjectWithDocument {
    pub project: CreativeProjectSummary,
    pub document: CreativeProjectDocument,
}

/// A completed, bounded Creative Studio v1 project archive.
pub struct CreativeProjectArchive {
    pub file_name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// A paginated asset listing.
pub struct AssetListPage {
    pub items: Vec<WorkshopAsset>,
    pub total: i64,
}

/// A served asset file (bytes + resolved Content-Type).
pub struct ServedFile {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Internal descriptor for storing a binary (image/video/audio) asset — the
/// shared path behind both the HTTP upload and the programmatic
/// [`WorkshopService::ingest_asset_bytes`].
struct BinaryAsset {
    kind: String,
    ext: String,
    mime: String,
    bytes: Vec<u8>,
    title: String,
    collection: Option<String>,
    tags: Option<Vec<String>>,
    in_library: bool,
    origin: Option<Value>,
}

/// Files are published before the SQLite import transaction so committed rows
/// never point at absent media. Any early return or task cancellation removes
/// the staged final paths; a successful DB commit disarms the guard.
struct CreativeArchiveFileRollback {
    paths: Vec<PathBuf>,
    committed: bool,
}

impl CreativeArchiveFileRollback {
    fn new() -> Self {
        Self {
            paths: Vec::new(),
            committed: false,
        }
    }

    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for CreativeArchiveFileRollback {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in &self.paths {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), %error, "failed to roll back creative archive asset file");
            }
        }
    }
}

/// A multipart asset upload (binary + optional metadata).
pub struct NewAssetUpload {
    pub file_name: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
    pub title: Option<String>,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

/// A `text`-kind asset (no binary; body lives in `text_content`).
pub struct NewTextAsset {
    pub title: String,
    pub text_content: String,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

/// Filters + pagination for [`WorkshopService::list_assets`].
#[derive(Default)]
pub struct AssetQuery {
    pub kind: Option<String>,
    pub collection: Option<String>,
    pub q: Option<String>,
    pub in_library: Option<bool>,
    /// Append-only (M10a): when `true`, return only assets with no collection
    /// (`collection IS NULL OR ''`). The caller keeps this mutually exclusive
    /// with `collection`.
    pub ungrouped: bool,
    /// Append-only (asset-library page): exact-match filter on one tag.
    pub tag: Option<String>,
    /// Append-only (asset-library page): result ordering (default newest first).
    pub sort: AssetSort,
    pub page: i64,
    pub page_size: i64,
}

/// Partial asset update. A present field updates; an absent one keeps. For
/// `collection`, `Some("")` clears it to NULL.
#[derive(Default)]
pub struct AssetPatch {
    pub title: Option<String>,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

/// GC recency grace (ms). An asset row or on-disk file created/modified more
/// recently than this is never reclaimed by [`WorkshopService::gc`] or the
/// `delete_canvas` internal-asset sweep — it may still be an in-flight upload
/// (file on disk before its row is inserted) or a reference an open canvas has
/// added but not yet autosaved. A truly orphaned asset is still older than this
/// on the next pass and gets reclaimed then. 10 minutes ≫ the max
/// write+thumbnail latency and the 800ms autosave debounce.
const GC_GRACE_MS: i64 = 10 * 60 * 1000;

const DEFAULT_CREATIVE_PROJECT_TITLE: &str = "未命名画布";
const MAX_CREATIVE_PROJECT_TITLE_CHARS: usize = 1_000;

pub struct WorkshopService {
    repo: Arc<dyn IWorkshopRepository>,
    /// Backend data dir root. Asset `rel_path`s are relative to this.
    data_dir: PathBuf,
    /// 画布助手 (canvas assistant) agent-op queue — the in-memory buffer the
    /// gateway enqueues into and the REST `pending-ops` routes drain. One
    /// instance per singleton service, so the gateway and the routes share it.
    agent_ops: crate::agent_ops::AgentOpsQueue,
    /// GC recency grace (ms). Defaults to [`GC_GRACE_MS`]; tests override it to
    /// `0` to drive immediate reclamation deterministically.
    gc_grace_ms: i64,
    provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
}

impl WorkshopService {
    /// Build the service over its index repo + the data dir root.
    pub fn start(data_dir: &Path, repo: Arc<dyn IWorkshopRepository>) -> Arc<Self> {
        Self::start_with_gc_grace_and_provider_lifecycle(data_dir, repo, GC_GRACE_MS, None)
    }

    /// Build the service with the process-wide Provider lifecycle barrier.
    pub fn start_with_provider_lifecycle(
        data_dir: &Path,
        repo: Arc<dyn IWorkshopRepository>,
        provider_lifecycle: SharedProviderLifecycleBarrier,
    ) -> Arc<Self> {
        Self::start_with_gc_grace_and_provider_lifecycle(
            data_dir,
            repo,
            GC_GRACE_MS,
            Some(provider_lifecycle),
        )
    }

    /// [`Self::start`] with an explicit GC recency grace (ms). Production uses
    /// [`GC_GRACE_MS`]; tests pass `0` for immediate reclamation.
    #[cfg(test)]
    fn start_with_gc_grace(data_dir: &Path, repo: Arc<dyn IWorkshopRepository>, gc_grace_ms: i64) -> Arc<Self> {
        Self::start_with_gc_grace_and_provider_lifecycle(data_dir, repo, gc_grace_ms, None)
    }

    fn start_with_gc_grace_and_provider_lifecycle(
        data_dir: &Path,
        repo: Arc<dyn IWorkshopRepository>,
        gc_grace_ms: i64,
        provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repo,
            data_dir: data_dir.to_path_buf(),
            agent_ops: crate::agent_ops::AgentOpsQueue::new(),
            gc_grace_ms,
            provider_lifecycle,
        })
    }

    // ---- path helpers ----

    fn workshop_dir(&self) -> PathBuf {
        self.data_dir.join(WORKSHOP_REL_DIR)
    }

    fn canvas_dir(&self, id: &str) -> PathBuf {
        self.workshop_dir().join("canvases").join(id)
    }

    fn assets_dir(&self) -> PathBuf {
        self.workshop_dir().join("assets")
    }

    async fn provider_read_guard(
        &self,
    ) -> Option<tokio::sync::RwLockReadGuard<'_, ()>> {
        match &self.provider_lifecycle {
            Some(barrier) => Some(barrier.read().await),
            None => None,
        }
    }

    async fn validate_generator_providers(&self, doc: &Value) -> Result<(), AppError> {
        let provider_ids = docscan::collect_generator_provider_refs(doc)
            .map_err(|error| AppError::BadRequest(format!("invalid workshop canvas doc: {error}")))?;
        for provider_id in provider_ids {
            if !self.repo.provider_exists(&provider_id).await? {
                return Err(AppError::Conflict(format!(
                    "workshop canvas references missing provider '{provider_id}'"
                )));
            }
        }
        Ok(())
    }

    /// Validate every durable Creative Studio config-node selection against
    /// the exact managed Provider/model pair. The lifecycle read guard is held
    /// by callers across this check and the subsequent project CAS write.
    async fn validate_creative_provider_models(
        &self,
        document: &CreativeProjectDocument,
    ) -> Result<(), AppError> {
        let mut references = BTreeMap::new();
        for node in &document.nodes {
            let CreativeNodeData::Config(config) = &node.data else {
                continue;
            };
            match (config.provider_id.as_deref(), config.model.as_deref()) {
                (None, None) => {}
                (Some(provider_id), Some(model)) => {
                    ProviderId::parse(provider_id).map_err(|error| {
                        AppError::BadRequest(format!(
                            "creative config node {} providerId must be a canonical Provider UUIDv7: {error}",
                            node.id
                        ))
                    })?;
                    references
                        .entry((provider_id.to_owned(), model.to_owned()))
                        .or_insert_with(|| node.id.clone());
                }
                (Some(_), None) => {
                    return Err(AppError::BadRequest(format!(
                        "creative config node {} providerId and model must be set together",
                        node.id
                    )));
                }
                (None, Some(_)) => {
                    return Err(AppError::BadRequest(format!(
                        "creative config node {} providerId and model must be set together",
                        node.id
                    )));
                }
            }
        }
        for ((provider_id, model), node_id) in references {
            if !self
                .repo
                .provider_model_exists(&provider_id, &model)
                .await?
            {
                return Err(AppError::Conflict(format!(
                    "creative config node {node_id} references missing provider-model '{provider_id}/{model}'"
                )));
            }
        }
        Ok(())
    }

    // ---- canvases ----

    // Canonical Creative Studio project methods intentionally live beside,
    // rather than inside, the legacy canvas methods below. They share the Rust
    // service/repository infrastructure but never read or rewrite the retired
    // schema.

    pub async fn list_creative_projects(
        &self,
    ) -> Result<Vec<CreativeProjectSummary>, AppError> {
        Ok(self
            .repo
            .list_creative_projects()
            .await?
            .into_iter()
            .map(CreativeProjectSummary::from)
            .collect())
    }

    pub async fn create_creative_project(
        &self,
        title: Option<String>,
    ) -> Result<CreativeProjectSummary, AppError> {
        let project_id = CreativeStudioProjectId::new().into_string();
        let title = normalize_creative_project_title(title.as_deref(), true)?;
        let document = CreativeProjectDocument::empty(project_id.clone());
        document
            .validate_for_project(&project_id)
            .map_err(|error| AppError::Internal(format!("invalid default creative project: {error}")))?;
        let document_json = serialize_creative_project_document(&document)?;
        let row = self
            .repo
            .create_creative_project(&project_id, &title, &document_json, now_ms())
            .await?;
        Ok(row.into())
    }

    pub async fn get_creative_project(
        &self,
        project_id: &str,
    ) -> Result<CreativeProjectWithDocument, AppError> {
        validate_creative_project_id(project_id)?;
        let row = self
            .repo
            .get_creative_project(project_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio project {project_id} not found"
                ))
            })?;
        let document = parse_stored_creative_project_row(&row)?;
        Ok(CreativeProjectWithDocument {
            project: row.into(),
            document,
        })
    }

    pub async fn rename_creative_project(
        &self,
        project_id: &str,
        title: &str,
    ) -> Result<CreativeProjectSummary, AppError> {
        validate_creative_project_id(project_id)?;
        let title = normalize_creative_project_title(Some(title), false)?;
        Ok(self
            .repo
            .rename_creative_project(project_id, &title, now_ms())
            .await?
            .into())
    }

    pub async fn save_creative_project(
        &self,
        project_id: &str,
        expected_revision: &str,
        document: &CreativeProjectDocument,
    ) -> Result<CreativeProjectSummary, AppError> {
        validate_creative_project_id(project_id)?;
        let expected_revision = parse_creative_project_revision(expected_revision)?;
        document
            .validate_for_project(project_id)
            .map_err(|error| AppError::BadRequest(format!("invalid creative project document: {error}")))?;
        let document_json = serialize_creative_project_document(document)?;
        let node_count = i64::try_from(document.nodes.len())
            .map_err(|_| AppError::BadRequest("creative project has too many nodes".into()))?;
        let connection_count = i64::try_from(document.connections.len()).map_err(|_| {
            AppError::BadRequest("creative project has too many connections".into())
        })?;
        let _provider_guard = self.provider_read_guard().await;
        self.validate_creative_provider_models(document).await?;
        Ok(self
            .repo
            .save_creative_project(
                project_id,
                expected_revision,
                &document_json,
                node_count,
                connection_count,
                now_ms(),
            )
            .await?
            .into())
    }

    pub async fn delete_creative_project(&self, project_id: &str) -> Result<(), AppError> {
        validate_creative_project_id(project_id)?;
        self.repo.delete_creative_project(project_id).await?;
        Ok(())
    }

    // ---- canonical Creative Studio workflows ----

    pub async fn list_creative_workflows(
        &self,
    ) -> Result<Vec<CreativeWorkflowDefinitionV1>, AppError> {
        self.repo
            .list_creative_workflows()
            .await?
            .into_iter()
            .map(|row| {
                parse_workflow_row(&row).map_err(|error| {
                    AppError::Internal(format!(
                        "stored creative studio workflow {} is corrupt: {error}",
                        row.workflow_id
                    ))
                })
            })
            .collect()
    }

    pub async fn get_creative_workflow(
        &self,
        workflow_id: &str,
    ) -> Result<CreativeWorkflowDefinitionV1, AppError> {
        validate_creative_workflow_id(workflow_id)?;
        let row = self
            .repo
            .get_creative_workflow(workflow_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio workflow {workflow_id} not found"
                ))
            })?;
        parse_workflow_row(&row).map_err(|error| {
            AppError::Internal(format!(
                "stored creative studio workflow {workflow_id} is corrupt: {error}"
            ))
        })
    }

    pub async fn create_creative_workflow(
        &self,
        mut definition: CreativeWorkflowDefinitionV1,
    ) -> Result<CreativeWorkflowDefinitionV1, AppError> {
        validate_creative_workflow_id(&definition.id)?;
        if definition.revision != 1 {
            return Err(AppError::BadRequest(
                "a creative studio workflow must start at revision 1".into(),
            ));
        }
        if self.repo.get_creative_workflow(&definition.id).await?.is_some() {
            return Err(AppError::Conflict(format!(
                "creative studio workflow {} already exists",
                definition.id
            )));
        }
        let now = now_ms();
        definition.metadata.created_at = now;
        definition.metadata.updated_at = now;
        definition
            .validate()
            .map_err(|error| AppError::BadRequest(format!("invalid workflow definition: {error}")))?;
        self.validate_creative_workflow_assets(&definition).await?;
        self.validate_creative_workflow_models(&definition).await?;
        let row = definition
            .to_row()
            .map_err(|error| AppError::BadRequest(format!("invalid workflow definition: {error}")))?;
        let saved = self.repo.create_creative_workflow(&row).await?;
        parse_workflow_row(&saved).map_err(|error| {
            AppError::Internal(format!(
                "created creative studio workflow {} is corrupt: {error}",
                saved.workflow_id
            ))
        })
    }

    pub async fn save_creative_workflow(
        &self,
        workflow_id: &str,
        expected_revision: &str,
        mut definition: CreativeWorkflowDefinitionV1,
    ) -> Result<CreativeWorkflowDefinitionV1, AppError> {
        validate_creative_workflow_id(workflow_id)?;
        let expected_revision = parse_creative_workflow_revision(expected_revision)?;
        let current_row = self
            .repo
            .get_creative_workflow(workflow_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio workflow {workflow_id} not found"
                ))
            })?;
        let current = parse_workflow_row(&current_row).map_err(|error| {
            AppError::Internal(format!(
                "stored creative studio workflow {workflow_id} is corrupt: {error}"
            ))
        })?;
        if definition.id != workflow_id {
            return Err(AppError::BadRequest(
                "workflow definition id must match its route id".into(),
            ));
        }
        if definition.revision != expected_revision + 1 {
            return Err(AppError::BadRequest(
                "workflow definition revision must increment expectedRevision exactly once".into(),
            ));
        }
        definition.metadata.created_at = current.metadata.created_at;
        definition.metadata.updated_at = now_ms();
        definition
            .validate()
            .map_err(|error| AppError::BadRequest(format!("invalid workflow definition: {error}")))?;
        self.validate_creative_workflow_assets(&definition).await?;
        self.validate_creative_workflow_models(&definition).await?;
        let replacement = definition
            .to_row()
            .map_err(|error| AppError::BadRequest(format!("invalid workflow definition: {error}")))?;
        let saved = self
            .repo
            .save_creative_workflow(workflow_id, expected_revision, &replacement)
            .await?;
        parse_workflow_row(&saved).map_err(|error| {
            AppError::Internal(format!(
                "saved creative studio workflow {workflow_id} is corrupt: {error}"
            ))
        })
    }

    pub async fn delete_creative_workflow(&self, workflow_id: &str) -> Result<(), AppError> {
        validate_creative_workflow_id(workflow_id)?;
        self.repo.delete_creative_workflow(workflow_id).await?;
        Ok(())
    }

    async fn validate_creative_workflow_assets(
        &self,
        definition: &CreativeWorkflowDefinitionV1,
    ) -> Result<(), AppError> {
        for asset_id in definition.collect_asset_ids() {
            WorkshopAssetId::parse(asset_id).map_err(|error| {
                AppError::BadRequest(format!(
                    "workflow references invalid asset id {asset_id:?}: {error}"
                ))
            })?;
            let asset = self.repo.get_asset(asset_id).await?.ok_or_else(|| {
                AppError::Conflict(format!("workflow references missing asset {asset_id}"))
            })?;
            if asset.kind != "image" {
                return Err(AppError::Conflict(format!(
                    "workflow reference {asset_id} must identify an image asset"
                )));
            }
        }
        Ok(())
    }

    async fn validate_creative_workflow_models(
        &self,
        definition: &CreativeWorkflowDefinitionV1,
    ) -> Result<(), AppError> {
        let _provider_guard = self.provider_read_guard().await;
        self.validate_creative_workflow_models_under_guard(definition)
            .await
    }

    async fn validate_creative_workflow_models_under_guard(
        &self,
        definition: &CreativeWorkflowDefinitionV1,
    ) -> Result<(), AppError> {
        for binding in definition.image_model_bindings() {
            ProviderId::parse(&binding.provider_id).map_err(|error| {
                AppError::BadRequest(format!(
                    "workflow references invalid provider id {:?}: {error}",
                    binding.provider_id
                ))
            })?;
            if !self
                .repo
                .provider_model_supports_task(
                    &binding.provider_id,
                    &binding.model,
                    binding.task.as_str(),
                )
                .await?
            {
                return Err(AppError::Conflict(format!(
                    "workflow model binding {}/{} does not provide enabled task {}",
                    binding.provider_id,
                    binding.model,
                    binding.task.as_str()
                )));
            }
        }
        for binding in definition.text_model_bindings() {
            ProviderId::parse(&binding.provider_id).map_err(|error| {
                AppError::BadRequest(format!(
                    "workflow references invalid provider id {:?}: {error}",
                    binding.provider_id
                ))
            })?;
            if !self
                .repo
                .provider_model_supports_task(
                    &binding.provider_id,
                    &binding.model,
                    binding.task.as_str(),
                )
                .await?
            {
                return Err(AppError::Conflict(format!(
                    "workflow model binding {}/{} does not provide enabled task {}",
                    binding.provider_id,
                    binding.model,
                    binding.task.as_str()
                )));
            }
        }
        Ok(())
    }

    pub async fn export_creative_project_archive(
        &self,
        project_id: &str,
    ) -> Result<CreativeProjectArchive, AppError> {
        let detail = self.get_creative_project(project_id).await?;
        let asset_ids = collect_document_asset_ids(&detail.document)?;
        let mut assets = Vec::with_capacity(asset_ids.len());
        for asset_id in asset_ids {
            let row = self
                .repo
                .get_asset(&asset_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(format!(
                        "creative project {project_id} references missing asset {asset_id}"
                    ))
                })?;
            let bytes = self
                .read_original(&row)
                .await
                .map_err(|error| match error {
                    AppError::NotFound(message) => AppError::Conflict(format!(
                        "creative project {project_id} asset cannot be exported: {message}"
                    )),
                    other => other,
                })?
                .0;
            assets.push(CreativeArchiveAssetSnapshot { row, bytes });
        }
        let title = detail.project.title;
        let document = detail.document;
        let archive_bytes = tokio::task::spawn_blocking(move || {
            build_creative_project_archive(&title, &document, assets, now_ms())
        })
        .await
        .map_err(|error| {
            AppError::Internal(format!("creative project archive worker failed: {error}"))
        })??;
        Ok(CreativeProjectArchive {
            file_name: format!("creative-studio-{project_id}.nomifun-canvas.zip"),
            mime: CREATIVE_STUDIO_ARCHIVE_MIME,
            bytes: archive_bytes,
        })
    }

    pub async fn import_creative_project_archive(
        &self,
        archive_bytes: Vec<u8>,
    ) -> Result<CreativeProjectSummary, AppError> {
        let project_id = CreativeStudioProjectId::new().into_string();
        let remap_project_id = project_id.clone();
        let archive = tokio::task::spawn_blocking(move || {
            let parsed = parse_creative_project_archive(&archive_bytes)?;
            remap_creative_archive_for_import(parsed, &remap_project_id)
        })
        .await
        .map_err(|error| {
            AppError::Internal(format!("creative project archive worker failed: {error}"))
        })??;

        let title = normalize_creative_project_title(Some(&archive.title), false)?;
        let document_json = serialize_creative_project_document(&archive.document)?;
        let node_count = i64::try_from(archive.document.nodes.len())
            .map_err(|_| AppError::BadRequest("creative project has too many nodes".into()))?;
        let connection_count = i64::try_from(archive.document.connections.len()).map_err(|_| {
            AppError::BadRequest("creative project has too many connections".into())
        })?;
        let now = now_ms();
        let mut rollback = CreativeArchiveFileRollback::new();
        let mut asset_rows = Vec::with_capacity(archive.assets.len());
        for asset in archive.assets {
            let metadata = asset.metadata;
            let tags = serde_json::to_string(&metadata.tags).map_err(|error| {
                AppError::Internal(format!("encode imported creative asset tags: {error}"))
            })?;
            let origin = sanitized_archive_origin(metadata.origin)?;
            let (rel_path, mime, bytes, text_content) = if metadata.kind == "text" {
                let text = String::from_utf8(asset.bytes).map_err(|_| {
                    AppError::BadRequest(format!(
                        "creative archive text asset {} is not valid UTF-8",
                        metadata.asset_id
                    ))
                })?;
                (None, None, None, Some(text))
            } else {
                let (classified_kind, ext) = classify_mime(&metadata.mime)?;
                if classified_kind != metadata.kind {
                    return Err(AppError::BadRequest(format!(
                        "creative archive asset {} kind does not match MIME type",
                        metadata.asset_id
                    )));
                }
                let disk_name = format!("{}.{}", metadata.asset_id, ext);
                let rel_path = format!("{WORKSHOP_REL_DIR}/assets/{disk_name}");
                fsio::save_bytes_atomic(&self.assets_dir(), &disk_name, &asset.bytes)
                    .await
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "write imported creative asset {}: {error}",
                            metadata.asset_id
                        ))
                    })?;
                rollback.track(self.data_dir.join(&rel_path));
                (
                    Some(rel_path),
                    Some(metadata.mime),
                    Some(asset.bytes.len() as i64),
                    None,
                )
            };
            asset_rows.push(WorkshopAssetRow {
                id: 0,
                asset_id: metadata.asset_id,
                kind: metadata.kind,
                title: metadata.title,
                collection: metadata.collection,
                tags,
                rel_path,
                thumb_rel_path: None,
                mime,
                width: metadata.width,
                height: metadata.height,
                bytes,
                text_content,
                in_library: metadata.in_library,
                origin,
                created_at: now,
                updated_at: now,
            });
        }
        let project_row = CreativeStudioProjectRow {
            id: 0,
            project_id,
            title,
            revision: 1,
            node_count,
            connection_count,
            document_json,
            created_at: now,
            updated_at: now,
        };
        let imported = self
            .repo
            .import_creative_project_with_assets(&project_row, &asset_rows)
            .await?;
        rollback.commit();
        Ok(imported.into())
    }

    pub async fn list_canvases(&self) -> Result<Vec<WorkshopCanvasMeta>, AppError> {
        Ok(self.repo.list_canvases().await?.into_iter().map(WorkshopCanvasMeta::from).collect())
    }

    /// Read-only startup audit for canonical Creative Studio projects and the
    /// shared asset store. Retired legacy canvas rows/files are intentionally
    /// inert: corrupt or missing `canvas.json` data must never prevent the new
    /// product from starting, and this audit never rewrites or deletes it.
    pub async fn audit_managed_data_on_boot(&self) -> Result<(), AppError> {
        let mut referenced_assets = BTreeSet::new();
        for project in self.repo.list_creative_projects().await? {
            let document = parse_stored_creative_project_row(&project)?;
            self.validate_creative_provider_models(&document).await?;
            referenced_assets.extend(collect_document_asset_ids(&document)?);
        }

        let assets = self.repo.list_all_assets().await?;
        let indexed_assets = assets
            .iter()
            .map(|asset| asset.asset_id.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(asset_id) = referenced_assets
            .iter()
            .find(|asset_id| !indexed_assets.contains(asset_id.as_str()))
        {
            return Err(AppError::Internal(format!(
                "managed creative studio project references missing asset {asset_id}"
            )));
        }

        for asset in assets {
            let tags = serde_json::from_str::<Value>(&asset.tags).map_err(|error| {
                AppError::Internal(format!(
                    "managed workshop asset {} has invalid tags JSON: {error}",
                    asset.asset_id
                ))
            })?;
            if !tags.is_array() {
                return Err(AppError::Internal(format!(
                    "managed workshop asset {} tags must be a JSON array",
                    asset.asset_id
                )));
            }

            match asset.rel_path.as_deref() {
                Some(rel_path) => {
                    let path = self.resolve_within_workshop(rel_path)?;
                    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
                        AppError::Internal(format!(
                            "managed workshop asset {} payload is unavailable: {error}",
                            asset.asset_id
                        ))
                    })?;
                    if !metadata.is_file() {
                        return Err(AppError::Internal(format!(
                            "managed workshop asset {} payload is not a regular file",
                            asset.asset_id
                        )));
                    }
                }
                None if asset.kind != "text" => {
                    return Err(AppError::Internal(format!(
                        "managed binary workshop asset {} has no payload path",
                        asset.asset_id
                    )));
                }
                None => {}
            }

            if let Some(thumb_rel_path) = asset.thumb_rel_path.as_deref() {
                let path = self.resolve_within_workshop(thumb_rel_path)?;
                let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
                    AppError::Internal(format!(
                        "managed workshop asset {} thumbnail is unavailable: {error}",
                        asset.asset_id
                    ))
                })?;
                if !metadata.is_file() {
                    return Err(AppError::Internal(format!(
                        "managed workshop asset {} thumbnail is not a regular file",
                        asset.asset_id
                    )));
                }
            }
        }
        Ok(())
    }

    pub async fn create_canvas(&self, title: Option<String>) -> Result<WorkshopCanvasMeta, AppError> {
        let id = WorkshopCanvasId::new().into_string();
        let title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "未命名画布".to_string());
        let now = now_ms();
        // Write the empty doc first so a crash between INSERT and write cannot
        // leave an indexed canvas without its managed document.
        fsio::save_bytes_atomic(&self.canvas_dir(&id), "canvas.json", DEFAULT_DOC.as_bytes())
            .await
            .map_err(|e| AppError::Internal(format!("write canvas doc: {e}")))?;
        let row = self.repo.create_canvas(&id, &title, now).await?;
        Ok(row.into())
    }

    pub async fn get_canvas(&self, id: &str) -> Result<CanvasWithDoc, AppError> {
        let row = self
            .repo
            .get_canvas(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop canvas {id} not found")))?;
        let doc = self.read_doc(id).await?;
        Ok(CanvasWithDoc { meta: row.into(), doc })
    }

    /// The document payload remains frontend-owned, but its durable identity
    /// envelope is a backend invariant: every node/edge ID and every declared
    /// node reference must be canonical before data is served back to clients.
    async fn read_doc(&self, id: &str) -> Result<Value, AppError> {
        let path = self.canvas_dir(id).join("canvas.json");
        let bytes = fsio::read_bytes_opt(&path)
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "read managed workshop canvas {id} document: {error}"
                ))
            })?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "managed workshop canvas {id} document is missing"
                ))
            })?;
        let doc: Value = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "managed workshop canvas {id} document is invalid JSON: {error}"
            ))
        })?;
        docscan::validate_canvas_doc_ids(&doc).map_err(|error| {
            AppError::Internal(format!(
                "managed workshop canvas {id} has invalid durable IDs: {error}"
            ))
        })?;
        docscan::collect_generator_provider_refs(&doc).map_err(|error| {
            AppError::Internal(format!(
                "managed workshop canvas {id} has invalid provider references: {error}"
            ))
        })?;
        Ok(doc)
    }

    /// Persist a frontend-owned doc (≤ [`MAX_DOC_BYTES`]), sync `node_count`
    /// from `doc.nodes`, and return the new `updated_at`.
    ///
    /// Although node payloads remain opaque, durable IDs are validated deeply:
    /// `nodes[].id`/`groupId`, `edges[].id`/`from`/`to`, and `node:<id>` mention
    /// references must all be canonical and internally resolvable.
    pub async fn save_doc(&self, id: &str, doc: &Value) -> Result<i64, AppError> {
        let _provider_guard = self.provider_read_guard().await;
        self.save_doc_inner(id, doc, true).await
    }

    async fn save_doc_inner(
        &self,
        id: &str,
        doc: &Value,
        validate_providers: bool,
    ) -> Result<i64, AppError> {
        // Ensure the canvas exists before touching disk.
        if self.repo.get_canvas(id).await?.is_none() {
            return Err(AppError::NotFound(format!("workshop canvas {id} not found")));
        }
        let node_count = docscan::validate_canvas_doc_ids(doc)
            .map_err(|error| AppError::BadRequest(format!("invalid workshop canvas doc: {error}")))?
            as i64;
        if validate_providers {
            self.validate_generator_providers(doc).await?;
        }
        let bytes = serde_json::to_vec(doc).map_err(|e| AppError::BadRequest(format!("invalid doc json: {e}")))?;
        if bytes.len() > MAX_DOC_BYTES {
            return Err(AppError::BadRequest(format!(
                "canvas doc is too large: {} bytes (max {MAX_DOC_BYTES})",
                bytes.len()
            )));
        }
        fsio::save_bytes_atomic(&self.canvas_dir(id), "canvas.json", &bytes)
            .await
            .map_err(|e| AppError::Internal(format!("write canvas doc: {e}")))?;
        let row = self.repo.touch_canvas(id, node_count, now_ms()).await?;
        Ok(row.updated_at)
    }

    pub async fn rename_canvas(&self, id: &str, title: &str) -> Result<WorkshopCanvasMeta, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        Ok(self.repo.rename_canvas(id, title, now_ms()).await?.into())
    }

    /// PATCH a canvas: optionally rename and/or set its gallery thumbnail from an
    /// asset (append-only over `rename_canvas`). Returns the latest meta. A
    /// request with no fields is a no-op that returns the current meta.
    pub async fn patch_canvas(
        &self,
        id: &str,
        title: Option<String>,
        thumbnail_asset_id: Option<String>,
    ) -> Result<WorkshopCanvasMeta, AppError> {
        let mut latest: Option<WorkshopCanvasMeta> = None;
        if let Some(title) = title {
            latest = Some(self.rename_canvas(id, &title).await?);
        }
        if let Some(asset_id) = thumbnail_asset_id {
            latest = Some(self.set_canvas_thumbnail(id, &asset_id).await?);
        }
        match latest {
            Some(meta) => Ok(meta),
            None => {
                let row = self
                    .repo
                    .get_canvas(id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("workshop canvas {id} not found")))?;
                Ok(row.into())
            }
        }
    }

    /// Point a canvas's gallery thumbnail at an asset's thumbnail. The asset
    /// must be a decodable image (its JPEG thumbnail — generated on demand — is
    /// copied to `{canvas_dir}/thumb.jpg`).
    pub async fn set_canvas_thumbnail(&self, canvas_id: &str, asset_id: &str) -> Result<WorkshopCanvasMeta, AppError> {
        if self.repo.get_canvas(canvas_id).await?.is_none() {
            return Err(AppError::NotFound(format!("workshop canvas {canvas_id} not found")));
        }
        let row = self
            .repo
            .get_asset(asset_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {asset_id} not found")))?;
        let bytes = self
            .thumb_bytes(&row)
            .await
            .ok_or_else(|| AppError::BadRequest("thumbnail asset must be a decodable image".into()))?;
        fsio::save_bytes_atomic(&self.canvas_dir(canvas_id), "thumb.jpg", &bytes)
            .await
            .map_err(|e| AppError::Internal(format!("write canvas thumbnail: {e}")))?;
        let rel = format!("{WORKSHOP_REL_DIR}/canvases/{canvas_id}/thumb.jpg");
        Ok(self.repo.set_canvas_thumbnail(canvas_id, &rel, now_ms()).await?.into())
    }

    /// Serve a canvas's gallery thumbnail bytes (JPEG). NotFound when the canvas
    /// has no thumbnail set.
    pub async fn serve_canvas_thumbnail(&self, canvas_id: &str) -> Result<ServedFile, AppError> {
        let row = self
            .repo
            .get_canvas(canvas_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop canvas {canvas_id} not found")))?;
        let rel = row
            .thumbnail_rel_path
            .as_deref()
            .ok_or_else(|| AppError::NotFound(format!("canvas {canvas_id} has no thumbnail")))?;
        let abs = self.resolve_within_workshop(rel)?;
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|_| AppError::NotFound(format!("canvas {canvas_id} thumbnail is missing")))?;
        Ok(ServedFile { mime: thumbnail::THUMB_MIME.to_string(), bytes })
    }

    pub async fn delete_canvas(&self, id: &str) -> Result<(), AppError> {
        // Snapshot this canvas's asset references before its doc disappears, so
        // we can GC canvas-internal assets it alone kept alive.
        let doc = self.read_doc(id).await?;
        let own_refs = docscan::collect_asset_refs(&doc);

        self.repo.delete_canvas(id).await?;
        // Best-effort remove the on-disk body dir (row is the source of truth).
        if let Err(e) = tokio::fs::remove_dir_all(self.canvas_dir(id)).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(id, error = %e, "workshop canvas dir remove failed (row deleted)");
        }

        // GC: for each asset this canvas referenced, if it's canvas-internal
        // (`in_library = 0`) and no *other* canvas still references it, drop it.
        // A recency grace protects a freshly-created asset that another OPEN
        // canvas may reference but hasn't autosaved yet (its ref isn't on disk).
        if !own_refs.is_empty() {
            let now = now_ms();
            let still_referenced = self.collect_all_referenced_asset_ids().await.unwrap_or_default();
            for asset_id in own_refs {
                if still_referenced.contains(&asset_id) {
                    continue;
                }
                if let Ok(Some(row)) = self.repo.get_asset(&asset_id).await
                    && !row.in_library
                    && now.saturating_sub(row.created_at.max(row.updated_at)) >= self.gc_grace_ms
                    && let Err(e) = self.delete_asset(&asset_id).await
                {
                    tracing::warn!(asset_id, error = %e, "workshop GC: internal asset delete failed");
                }
            }
        }
        Ok(())
    }

    /// Every asset id referenced by *any* canvas doc (scans all canvases; the
    /// canvas count is small by design).
    async fn collect_all_referenced_asset_ids(&self) -> Result<BTreeSet<String>, AppError> {
        let mut out = BTreeSet::new();
        for canvas in self.repo.list_canvases().await? {
            let doc = self.read_doc(&canvas.canvas_id).await?;
            out.extend(docscan::collect_asset_refs(&doc));
        }
        Ok(out)
    }

    // ---- agent ops (画布助手) ----

    /// Enqueue or directly apply a batch of 画布助手 agent ops. See
    /// [`crate::agent_ops`] for the open-frontend-authority rule.
    ///
    /// All ops are validated up front (a single bad op fails the whole call so
    /// the agent can self-correct). Then, per op:
    /// - an OPEN canvas (a frontend is polling) queues EVERY op for the live
    ///   frontend to apply (preserving its write authority);
    /// - a CLOSED canvas applies `add_node` / `connect` straight to `canvas.json`
    ///   and queues the data-mutating ops (`update_node_data` / `delete_node`)
    ///   for whenever a frontend next opens.
    ///
    /// Returns a per-op disposition (`queued` | `applied` | `skipped`).
    pub async fn apply_agent_ops(
        &self,
        canvas_id: &str,
        ops: Vec<crate::agent_ops::AgentOp>,
        source: &str,
    ) -> Result<Vec<crate::agent_ops::AppliedOp>, AppError> {
        use crate::agent_ops::{self, AgentOp, AppliedOp, OpDisposition, PendingOp};
        let _provider_guard = self.provider_read_guard().await;

        if ops.is_empty() {
            return Err(AppError::BadRequest("no ops provided".into()));
        }
        if ops.len() > agent_ops::MAX_OPS_PER_CALL {
            return Err(AppError::BadRequest(format!(
                "too many ops in one call: {} (max {})",
                ops.len(),
                agent_ops::MAX_OPS_PER_CALL
            )));
        }
        if self.repo.get_canvas(canvas_id).await?.is_none() {
            return Err(AppError::NotFound(format!("workshop canvas {canvas_id} not found")));
        }
        for (i, op) in ops.iter().enumerate() {
            op.validate().map_err(|e| AppError::BadRequest(format!("ops[{i}]: {e}")))?;
        }

        let open = self.agent_ops.is_open(canvas_id);
        let mut results: Vec<AppliedOp> = Vec::with_capacity(ops.len());
        let mut to_queue: Vec<PendingOp> = Vec::new();
        // Direct-apply path (closed canvas) mutates one doc snapshot, saved once.
        let mut doc: Option<Value> = None;
        let mut dirty = false;

        for op in ops {
            let op_id = agent_ops::new_op_id();
            if !open && op.direct_applicable() {
                if doc.is_none() {
                    doc = Some(self.read_doc(canvas_id).await?);
                }
                let d = doc.as_mut().expect("doc loaded above");
                match op {
                    AgentOp::AddNode { node } => {
                        let node_id = agent_ops::apply_add_node(d, &node);
                        dirty = true;
                        results.push(AppliedOp {
                            op_id,
                            disposition: OpDisposition::Applied,
                            node_id: Some(node_id),
                            note: None,
                        });
                    }
                    AgentOp::Connect { from_node_id, to_node_id } => match agent_ops::apply_connect(d, &from_node_id, &to_node_id) {
                        Ok(Some(_edge)) => {
                            dirty = true;
                            results.push(AppliedOp { op_id, disposition: OpDisposition::Applied, node_id: None, note: None });
                        }
                        Ok(None) => results.push(AppliedOp {
                            op_id,
                            disposition: OpDisposition::Applied,
                            node_id: None,
                            note: Some("edge already existed".into()),
                        }),
                        Err(reason) => results.push(AppliedOp {
                            op_id,
                            disposition: OpDisposition::Skipped,
                            node_id: None,
                            note: Some(reason),
                        }),
                    },
                    // direct_applicable() only matches AddNode/Connect.
                    other => to_queue.push(PendingOp::new(op_id, other)),
                }
            } else {
                results.push(AppliedOp {
                    op_id: op_id.clone(),
                    disposition: OpDisposition::Queued,
                    node_id: None,
                    note: None,
                });
                to_queue.push(PendingOp::new(op_id, op));
            }
        }

        if dirty
            && let Some(d) = &doc
        {
            self.save_doc_inner(canvas_id, d, true).await?;
        }
        if !to_queue.is_empty() {
            self.agent_ops.enqueue(canvas_id, to_queue);
        }
        tracing::info!(canvas_id, source, open, ops = results.len(), "workshop agent ops processed");
        Ok(results)
    }

    /// Drain (idempotently — ops stay until acked) the pending 画布助手 ops for a
    /// canvas, recording the poll so the canvas registers as "open".
    pub async fn take_pending_ops(&self, canvas_id: &str) -> Result<Vec<crate::agent_ops::PendingOp>, AppError> {
        if self.repo.get_canvas(canvas_id).await?.is_none() {
            return Err(AppError::NotFound(format!("workshop canvas {canvas_id} not found")));
        }
        Ok(self.agent_ops.take_pending(canvas_id))
    }

    /// Acknowledge (remove) applied 画布助手 ops by id.
    pub fn ack_agent_ops(&self, canvas_id: &str, op_ids: &[String]) {
        self.agent_ops.ack(canvas_id, op_ids);
    }

    /// Register that an editor just opened this canvas (its doc was loaded via
    /// the REST canvas-doc GET). Marks the canvas "open" immediately so a
    /// concurrent agent `apply_ops` in the gap before the first pending-ops poll
    /// is queued for the live editor rather than direct-written and then
    /// clobbered by the editor's first autosave. See
    /// [`crate::agent_ops::AgentOpsQueue::mark_open`].
    pub fn mark_canvas_open(&self, canvas_id: &str) {
        self.agent_ops.mark_open(canvas_id);
    }

    // ---- assets ----

    pub async fn upload_asset(&self, input: NewAssetUpload) -> Result<WorkshopAsset, AppError> {
        let (ext, mime, kind) = classify_upload(&input.file_name, input.content_type.as_deref())?;
        let title = input
            .title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| input.file_name.clone());
        let row = self
            .store_binary_asset(BinaryAsset {
                kind: kind.to_string(),
                ext,
                mime,
                bytes: input.bytes,
                title,
                collection: input.collection,
                tags: input.tags,
                in_library: input.in_library.unwrap_or(true),
                origin: None,
            })
            .await?;
        WorkshopAsset::try_from(row)
    }

    /// Programmatic asset ingest: store raw `bytes` of a given `mime` as a new
    /// asset row and return it. The shared entry point for other modules (e.g.
    /// the generation engine writing produced media). `origin` is the JSON
    /// provenance blob (`{prompt,model,provider_id,params,canvas_id,…}`).
    pub async fn ingest_asset_bytes(
        &self,
        bytes: Vec<u8>,
        mime: &str,
        title: &str,
        in_library: bool,
        origin: Option<Value>,
    ) -> Result<WorkshopAssetRow, AppError> {
        let (kind, ext) = classify_mime(mime)?;
        let title = title.trim();
        let title = if title.is_empty() { format!("{kind} asset") } else { title.to_string() };
        self.store_binary_asset(BinaryAsset {
            kind: kind.to_string(),
            ext,
            mime: mime.trim().to_string(),
            bytes,
            title,
            collection: None,
            tags: None,
            in_library,
            origin,
        })
        .await
    }

    /// Remove one Provider/model selection from every canonical Creative
    /// Studio config node and workflow generation step. The Provider deletion
    /// coordinator invokes this while holding the process-wide lifecycle write
    /// guard, so compliant saves cannot race the scan. Each changed project or
    /// workflow is replaced with its repository CAS: a conflict fails closed
    /// instead of overwriting a newer revision. The overall scan is idempotent;
    /// retired legacy canvas files remain inert and are neither read nor
    /// rewritten.
    pub async fn clear_provider_references_under_lifecycle_write_guard(
        &self,
        provider_id: &str,
    ) -> Result<(), AppError> {
        let provider_id = ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?
            .into_string();
        for project in self.repo.list_creative_projects().await? {
            let mut document = parse_stored_creative_project_row(&project)?;
            let mut changed = false;
            for node in &mut document.nodes {
                let CreativeNodeData::Config(config) = &mut node.data else {
                    continue;
                };
                if config.provider_id.as_deref() == Some(provider_id.as_str()) {
                    config.provider_id = None;
                    config.model = None;
                    changed = true;
                }
            }
            if !changed {
                continue;
            }

            document.validate_for_project(&project.project_id).map_err(|error| {
                AppError::Conflict(format!(
                    "creative studio project {} is invalid after provider cleanup: {error}",
                    project.project_id
                ))
            })?;
            self.validate_creative_provider_models(&document).await?;
            let document_json = serialize_creative_project_document(&document)?;
            let node_count = i64::try_from(document.nodes.len()).map_err(|_| {
                AppError::Conflict(format!(
                    "creative studio project {} has too many nodes",
                    project.project_id
                ))
            })?;
            let connection_count = i64::try_from(document.connections.len()).map_err(|_| {
                AppError::Conflict(format!(
                    "creative studio project {} has too many connections",
                    project.project_id
                ))
            })?;
            self.repo
                .save_creative_project(
                    &project.project_id,
                    project.revision,
                    &document_json,
                    node_count,
                    connection_count,
                    now_ms(),
                )
                .await?;
        }
        for workflow_row in self.repo.list_creative_workflows().await? {
            let mut workflow = parse_workflow_row(&workflow_row).map_err(|error| {
                AppError::Conflict(format!(
                    "creative studio workflow {} is corrupt during provider cleanup: {error}",
                    workflow_row.workflow_id
                ))
            })?;
            let mut changed = false;
            for step in &mut workflow.steps {
                match step {
                    crate::workflow::CreativeWorkflowStep::GenerateImages {
                        generation,
                        ..
                    } if generation
                        .model
                        .as_ref()
                        .is_some_and(|binding| binding.provider_id == provider_id) =>
                    {
                        generation.model = None;
                        changed = true;
                    }
                    crate::workflow::CreativeWorkflowStep::DraftPrompts {
                        planning,
                        ..
                    } if planning
                        .model
                        .as_ref()
                        .is_some_and(|binding| binding.provider_id == provider_id) =>
                    {
                        planning.model = None;
                        changed = true;
                    }
                    _ => {}
                }
            }
            if !changed {
                continue;
            }
            workflow.revision = workflow_row.revision + 1;
            workflow.metadata.updated_at = now_ms();
            workflow.validate().map_err(|error| {
                AppError::Conflict(format!(
                    "creative studio workflow {} is invalid after provider cleanup: {error}",
                    workflow.id
                ))
            })?;
            self.validate_creative_workflow_models_under_guard(&workflow)
                .await?;
            let replacement = workflow.to_row().map_err(|error| {
                AppError::Conflict(format!(
                    "creative studio workflow {} cannot be saved after provider cleanup: {error}",
                    workflow.id
                ))
            })?;
            self.repo
                .save_creative_workflow(&workflow.id, workflow_row.revision, &replacement)
                .await?;
        }
        Ok(())
    }

    /// Read an asset's original binary + its resolved mime. Errors when the
    /// asset is unknown, is a text asset (no file), or its file is missing. The
    /// programmatic counterpart to [`Self::serve_file`] (no thumbnail path).
    pub async fn read_asset_bytes(&self, asset_id: &str) -> Result<(Vec<u8>, String), AppError> {
        let row = self
            .repo
            .get_asset(asset_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {asset_id} not found")))?;
        self.read_original(&row).await
    }

    /// The shared store path: validate size, extract image dimensions, persist
    /// the binary, best-effort generate a thumbnail (images only), then insert
    /// the row (rolling the file back if the insert fails).
    async fn store_binary_asset(&self, input: BinaryAsset) -> Result<WorkshopAssetRow, AppError> {
        if input.bytes.is_empty() {
            return Err(AppError::BadRequest("asset payload is empty".into()));
        }
        if input.bytes.len() > MAX_ASSET_BYTES {
            return Err(AppError::BadRequest(format!(
                "asset is too large: {} bytes (max {MAX_ASSET_BYTES})",
                input.bytes.len()
            )));
        }
        let is_image = input.kind == "image";
        let (width, height) = if is_image {
            match imagemeta::image_dimensions(&input.bytes) {
                Some((w, h)) => (Some(w as i64), Some(h as i64)),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        let id = WorkshopAssetId::new().into_string();
        let disk_name = format!("{id}.{}", input.ext);
        let rel_path = format!("{WORKSHOP_REL_DIR}/assets/{disk_name}");
        fsio::save_bytes_atomic(&self.assets_dir(), &disk_name, &input.bytes)
            .await
            .map_err(|e| AppError::Internal(format!("write asset file: {e}")))?;

        let thumb_rel_path = if is_image {
            self.generate_and_store_thumb(&id, &input.bytes).await
        } else {
            None
        };

        let now = now_ms();
        let row = WorkshopAssetRow {
            id: 0,
            asset_id: id,
            kind: input.kind,
            title: input.title,
            collection: normalize_opt(input.collection),
            tags: tags_json(input.tags),
            rel_path: Some(rel_path),
            thumb_rel_path,
            mime: Some(input.mime),
            width,
            height,
            bytes: Some(input.bytes.len() as i64),
            text_content: None,
            in_library: input.in_library,
            origin: input.origin.map(|v| v.to_string()),
            created_at: now,
            updated_at: now,
        };
        // Roll the files back if the row insert fails.
        match self.repo.create_asset(&row).await {
            Ok(saved) => Ok(saved),
            Err(e) => {
                for rel in [row.rel_path.as_deref(), row.thumb_rel_path.as_deref()].into_iter().flatten() {
                    let _ = tokio::fs::remove_file(self.data_dir.join(rel)).await;
                }
                Err(e.into())
            }
        }
    }

    /// Generate a JPEG thumbnail from `bytes` and persist it under
    /// `assets/thumbs/{id}.jpg`. Returns its data-dir-relative path, or `None`
    /// when the bytes aren't decodable / the write fails (thumbnails are
    /// best-effort — the asset is still fully usable without one).
    async fn generate_and_store_thumb(&self, id: &str, bytes: &[u8]) -> Option<String> {
        let owned = bytes.to_vec();
        let thumb = tokio::task::spawn_blocking(move || {
            thumbnail::encode_thumbnail_jpeg(&owned, thumbnail::THUMB_MAX_EDGE)
        })
        .await
        .ok()??;
        let disk_name = format!("{id}.{}", thumbnail::THUMB_EXT);
        let dir = self.assets_dir().join("thumbs");
        if let Err(e) = fsio::save_bytes_atomic(&dir, &disk_name, &thumb).await {
            tracing::warn!(id, error = %e, "workshop thumbnail write failed");
            return None;
        }
        Some(format!("{WORKSHOP_REL_DIR}/assets/thumbs/{disk_name}"))
    }

    /// Best-effort thumbnail bytes for an asset: an existing thumbnail file if
    /// present, else (for images) one generated + persisted on the fly. `None`
    /// for non-images or when generation fails.
    async fn thumb_bytes(&self, row: &WorkshopAssetRow) -> Option<Vec<u8>> {
        if let Some(rel) = row.thumb_rel_path.as_deref()
            && let Ok(abs) = self.resolve_within_workshop(rel)
            && let Ok(bytes) = tokio::fs::read(&abs).await
        {
            return Some(bytes);
        }
        if row.kind != "image" {
            return None;
        }
        let rel = row.rel_path.as_deref()?;
        let abs = self.resolve_within_workshop(rel).ok()?;
        let original = tokio::fs::read(&abs).await.ok()?;
        let thumb_rel = self.generate_and_store_thumb(&row.asset_id, &original).await?;
        // Persist the freshly minted thumb path (best-effort).
        let _ = self
            .repo
            .set_asset_thumb(&row.asset_id, &thumb_rel, now_ms())
            .await;
        let thumb_abs = self.resolve_within_workshop(&thumb_rel).ok()?;
        tokio::fs::read(&thumb_abs).await.ok()
    }

    /// Read an asset's original bytes + mime (used by serve + programmatic read).
    async fn read_original(&self, row: &WorkshopAssetRow) -> Result<(Vec<u8>, String), AppError> {
        let Some(rel) = row.rel_path.as_deref() else {
            // Text assets keep their body inline in the row instead of on disk.
            if let Some(text) = row.text_content.as_deref() {
                return Ok((text.as_bytes().to_vec(), "text/plain; charset=utf-8".to_string()));
            }
            return Err(AppError::NotFound(format!(
                "asset {} has no file",
                row.asset_id
            )));
        };
        let abs = self.resolve_within_workshop(rel)?;
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|_| AppError::NotFound(format!("asset {} file is missing", row.asset_id)))?;
        let mime = row.mime.clone().unwrap_or_else(|| "application/octet-stream".to_string());
        Ok((bytes, mime))
    }

    pub async fn create_text_asset(&self, input: NewTextAsset) -> Result<WorkshopAsset, AppError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        let now = now_ms();
        let row = WorkshopAssetRow {
            id: 0,
            asset_id: WorkshopAssetId::new().into_string(),
            kind: "text".to_string(),
            title: title.to_string(),
            collection: normalize_opt(input.collection),
            tags: tags_json(input.tags),
            rel_path: None,
            thumb_rel_path: None,
            mime: None,
            width: None,
            height: None,
            bytes: None,
            text_content: Some(input.text_content),
            in_library: input.in_library.unwrap_or(true),
            origin: None,
            created_at: now,
            updated_at: now,
        };
        WorkshopAsset::try_from(self.repo.create_asset(&row).await?)
    }

    pub async fn list_assets(&self, query: AssetQuery) -> Result<AssetListPage, AppError> {
        let (rows, total) = self
            .repo
            .list_assets(ListAssetsParams {
                kind: query.kind.as_deref(),
                collection: query.collection.as_deref(),
                q: query.q.as_deref().filter(|s| !s.trim().is_empty()),
                in_library: query.in_library,
                ungrouped: query.ungrouped,
                tag: query.tag.as_deref().filter(|s| !s.trim().is_empty()),
                sort: query.sort,
                page: query.page,
                page_size: query.page_size,
            })
            .await?;
        let items = rows
            .into_iter()
            .map(WorkshopAsset::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AssetListPage { items, total })
    }

    pub async fn patch_asset(&self, id: &str, patch: AssetPatch) -> Result<WorkshopAsset, AppError> {
        // Own the JSON string so the borrowed params can reference it.
        let tags_owned = patch.tags.map(|t| serde_json::to_string(&t).unwrap_or_else(|_| "[]".to_string()));
        let collection = patch
            .collection
            .as_ref()
            .map(|c| if c.trim().is_empty() { None } else { Some(c.trim()) });
        let params = UpdateAssetParams {
            title: patch.title.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            collection,
            tags: tags_owned.as_deref(),
            in_library: patch.in_library,
        };
        WorkshopAsset::try_from(self.repo.update_asset(id, params, now_ms()).await?)
    }

    /// Bulk-rename a collection across every asset that used it (asset-library
    /// management). `from` must be non-empty; a whitespace-only `to` ungroups
    /// those assets (sets `collection` to NULL). Returns rows updated.
    pub async fn rename_collection(&self, from: &str, to: &str) -> Result<u64, AppError> {
        let from = from.trim();
        if from.is_empty() {
            return Err(AppError::BadRequest("collection name must not be empty".into()));
        }
        let to = to.trim();
        let to_opt = if to.is_empty() { None } else { Some(to) };
        Ok(self.repo.rename_collection(from, to_opt, now_ms()).await?)
    }

    pub async fn delete_asset(&self, id: &str) -> Result<(), AppError> {
        let row = self
            .repo
            .get_asset(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {id} not found")))?;
        for workflow_row in self.repo.list_creative_workflows().await? {
            let workflow = parse_workflow_row(&workflow_row).map_err(|error| {
                AppError::Internal(format!(
                    "stored creative studio workflow {} is corrupt: {error}",
                    workflow_row.workflow_id
                ))
            })?;
            if workflow.collect_asset_ids().contains(id) {
                return Err(AppError::Conflict(format!(
                    "asset {id} is referenced by workflow {}",
                    workflow.id
                )));
            }
        }
        self.repo.delete_asset(id).await?;
        for rel in [row.rel_path.as_deref(), row.thumb_rel_path.as_deref()].into_iter().flatten() {
            let abs = self.data_dir.join(rel);
            if let Err(e) = tokio::fs::remove_file(&abs).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(id, path = %abs.display(), error = %e, "workshop asset file remove failed (row deleted)");
            }
        }
        Ok(())
    }

    /// Serve an asset's original (or, when `thumb`, its thumbnail — generated on
    /// demand for images that lack one, else falling back to the original per
    /// contract §3.2). Traversal-safe. Missing file → NotFound.
    pub async fn serve_file(&self, asset_id: &str, thumb: bool) -> Result<ServedFile, AppError> {
        let row = self
            .repo
            .get_asset(asset_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {asset_id} not found")))?;

        if thumb
            && let Some(bytes) = self.thumb_bytes(&row).await
        {
            return Ok(ServedFile { mime: thumbnail::THUMB_MIME.to_string(), bytes });
        }
        let (bytes, mime) = self.read_original(&row).await?;
        Ok(ServedFile { mime, bytes })
    }

    /// Resolve a data-dir-relative path and guarantee it stays inside the
    /// workshop dir (defense-in-depth; `rel_path`s are minted by us).
    fn resolve_within_workshop(&self, rel: &str) -> Result<PathBuf, AppError> {
        if rel.contains('\0') || Path::new(rel).components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(AppError::Forbidden("asset path contains invalid traversal".into()));
        }
        let abs = self.data_dir.join(rel);
        let canonical = std::fs::canonicalize(&abs)
            .map_err(|_| AppError::NotFound("asset file is missing".into()))?;
        let root = std::fs::canonicalize(self.workshop_dir())
            .map_err(|e| AppError::Internal(format!("resolve workshop dir: {e}")))?;
        if !canonical.starts_with(&root) {
            return Err(AppError::Forbidden("asset path escapes the workshop sandbox".into()));
        }
        Ok(canonical)
    }
}

fn validate_creative_project_id(project_id: &str) -> Result<(), AppError> {
    nomifun_common::validate_uuidv7(project_id)
        .map(|_| ())
        .map_err(|error| {
            AppError::BadRequest(format!(
                "creative studio project id must be a canonical UUIDv7: {error}"
            ))
        })
}

fn validate_creative_workflow_id(workflow_id: &str) -> Result<(), AppError> {
    CreativeStudioWorkflowId::parse(workflow_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid workflow id: {error}")))
}

fn normalize_creative_project_title(
    title: Option<&str>,
    allow_default: bool,
) -> Result<String, AppError> {
    let title = title.map(str::trim).filter(|value| !value.is_empty());
    let title = match title {
        Some(title) => title,
        None if allow_default => DEFAULT_CREATIVE_PROJECT_TITLE,
        None => return Err(AppError::BadRequest("title must not be empty".into())),
    };
    if title.encode_utf16().count() > MAX_CREATIVE_PROJECT_TITLE_CHARS {
        return Err(AppError::BadRequest(format!(
            "title is too long (max {MAX_CREATIVE_PROJECT_TITLE_CHARS} UTF-16 code units)"
        )));
    }
    Ok(title.to_owned())
}

fn parse_creative_project_revision(revision: &str) -> Result<i64, AppError> {
    if revision.is_empty() || revision.starts_with('0') || !revision.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "expectedRevision must be a canonical positive decimal string".into(),
        ));
    }
    let parsed = revision.parse::<i64>().map_err(|_| {
        AppError::BadRequest(
            "expectedRevision must be a canonical positive decimal string".into(),
        )
    })?;
    if parsed < 1 || parsed.to_string() != revision {
        return Err(AppError::BadRequest(
            "expectedRevision must be a canonical positive decimal string".into(),
        ));
    }
    Ok(parsed)
}

fn parse_creative_workflow_revision(revision: &str) -> Result<i64, AppError> {
    let parsed = revision.parse::<i64>().map_err(|_| {
        AppError::BadRequest("expectedRevision must be a positive decimal integer".into())
    })?;
    if parsed < 1 || parsed.to_string() != revision {
        return Err(AppError::BadRequest(
            "expectedRevision must be a positive canonical decimal integer".into(),
        ));
    }
    Ok(parsed)
}

fn serialize_creative_project_document(
    document: &CreativeProjectDocument,
) -> Result<String, AppError> {
    let json = serde_json::to_string(document)
        .map_err(|error| AppError::BadRequest(format!("invalid creative project document: {error}")))?;
    if json.len() > MAX_CREATIVE_PROJECT_DOCUMENT_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative project document is too large: {} bytes (max {MAX_CREATIVE_PROJECT_DOCUMENT_BYTES})",
            json.len()
        )));
    }
    Ok(json)
}

fn parse_stored_creative_project_document(
    document_json: &str,
    project_id: &str,
) -> Result<CreativeProjectDocument, AppError> {
    let document = serde_json::from_str::<CreativeProjectDocument>(document_json).map_err(|error| {
        AppError::Internal(format!(
            "managed creative studio project {project_id} has an invalid v1 document: {error}"
        ))
    })?;
    document.validate_for_project(project_id).map_err(|error| {
        AppError::Internal(format!(
            "managed creative studio project {project_id} violates the v1 contract: {error}"
        ))
    })?;
    Ok(document)
}

fn parse_stored_creative_project_row(
    row: &CreativeStudioProjectRow,
) -> Result<CreativeProjectDocument, AppError> {
    let document =
        parse_stored_creative_project_document(&row.document_json, &row.project_id)?;
    let node_count = i64::try_from(document.nodes.len()).map_err(|_| {
        AppError::Internal(format!(
            "managed creative studio project {} has too many nodes",
            row.project_id
        ))
    })?;
    let connection_count = i64::try_from(document.connections.len()).map_err(|_| {
        AppError::Internal(format!(
            "managed creative studio project {} has too many connections",
            row.project_id
        ))
    })?;
    if row.node_count != node_count || row.connection_count != connection_count {
        return Err(AppError::Internal(format!(
            "managed creative studio project {} summary counts do not match its document",
            row.project_id
        )));
    }
    Ok(document)
}

#[cfg(test)]
fn default_doc_value() -> Value {
    serde_json::from_str(DEFAULT_DOC).expect("DEFAULT_DOC is valid json")
}

fn normalize_opt(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn tags_json(tags: Option<Vec<String>>) -> String {
    serde_json::to_string(&tags.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string())
}

/// Resolve `(ext, mime, kind)` for an upload. Only image/* and video/* are
/// accepted; anything else is a bad request.
fn classify_upload(file_name: &str, content_type: Option<&str>) -> Result<(String, String, &'static str), AppError> {
    let ext_from_name = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty());
    let guessed_raw = mime_guess::from_path(file_name).first_raw();
    let mime = content_type
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "application/octet-stream")
        .map(str::to_string)
        .or_else(|| guessed_raw.map(str::to_string))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let kind = if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("video/") {
        "video"
    } else {
        return Err(AppError::BadRequest(format!(
            "unsupported media type '{mime}': only image/* and video/* uploads are accepted"
        )));
    };

    let ext = ext_from_name
        .or_else(|| {
            mime_guess::get_mime_extensions_str(&mime).and_then(|exts| exts.first().map(|e| e.to_string()))
        })
        .unwrap_or_else(|| "bin".to_string());
    Ok((ext, mime, kind))
}

/// Resolve `(kind, ext)` from a bare mime type (programmatic ingest — no
/// filename). image/* → image, video/* → video, audio/* → audio; else a bad
/// request.
fn classify_mime(mime: &str) -> Result<(&'static str, String), AppError> {
    let m = mime.trim().to_ascii_lowercase();
    let kind = if m.starts_with("image/") {
        "image"
    } else if m.starts_with("video/") {
        "video"
    } else if m.starts_with("audio/") {
        "audio"
    } else {
        return Err(AppError::BadRequest(format!(
            "unsupported media type '{mime}': only image/*, video/*, audio/* are ingestible"
        )));
    };
    let ext = mime_guess::get_mime_extensions_str(&m)
        .and_then(|exts| exts.first().map(|e| e.to_string()))
        .unwrap_or_else(|| "bin".to_string());
    Ok((kind, ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{
        CreativeWorkflowMetadata, CreativeWorkflowOutputPlan, CreativeWorkflowPromptSource,
        CreativeWorkflowStep, CreativeWorkflowTemplate, CreativeWorkflowTemplateSegment,
        CreativeWorkflowVariable, CreativeWorkflowVisibility,
    };
    use nomifun_common::{
        ProviderLifecycleBarrier, WorkshopCanvasId, WorkshopEdgeId, WorkshopNodeId,
    };
    use nomifun_db::{IProviderRepository, SqliteProviderRepository, SqliteWorkshopRepository};

    async fn service() -> (Arc<WorkshopService>, tempfile::TempDir) {
        // Default test harness reclaims immediately (grace 0) so GC/delete tests
        // stay deterministic; the grace behavior is covered by dedicated tests.
        service_with_gc_grace(0).await
    }

    async fn service_with_gc_grace(grace_ms: i64) -> (Arc<WorkshopService>, tempfile::TempDir) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn IWorkshopRepository> = Arc::new(SqliteWorkshopRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        let dir = tempfile::tempdir().unwrap();
        (WorkshopService::start_with_gc_grace(dir.path(), repo, grace_ms), dir)
    }

    async fn service_with_database_and_lifecycle(
        provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    ) -> (Arc<WorkshopService>, tempfile::TempDir, Arc<nomifun_db::Database>) {
        let db = Arc::new(nomifun_db::init_database_memory().await.unwrap());
        let repo: Arc<dyn IWorkshopRepository> =
            Arc::new(SqliteWorkshopRepository::new(db.pool().clone()));
        let dir = tempfile::tempdir().unwrap();
        let service = WorkshopService::start_with_gc_grace_and_provider_lifecycle(
            dir.path(),
            repo,
            0,
            provider_lifecycle,
        );
        (service, dir, db)
    }

    async fn insert_provider(db: &nomifun_db::Database, provider_id: &str) {
        let credentials_encrypted = nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &[0x42; 32],
        )
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO providers (\
                provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                created_at, updated_at\
             ) VALUES (?, 'openai', ?, 'https://example.invalid', 'bearer', ?, \
                        1, 1, 1)",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(&credentials_encrypted)
        .execute(db.pool())
        .await
        .unwrap();
    }

    fn workflow_definition() -> CreativeWorkflowDefinitionV1 {
        let workflow_id = CreativeStudioWorkflowId::new().into_string();
        let variable_id = nomifun_common::generate_id();
        let template_id = nomifun_common::generate_id();
        let render_id = nomifun_common::generate_id();
        let generate_id = nomifun_common::generate_id();
        CreativeWorkflowDefinitionV1 {
            id: workflow_id,
            revision: 1,
            metadata: CreativeWorkflowMetadata {
                name: "电商海报".into(),
                description: "固定结构".into(),
                category: "电商".into(),
                visibility: CreativeWorkflowVisibility::Private,
                tags: vec!["海报".into()],
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

    fn series_workflow_definition(
        provider_id: &str,
        model: &str,
    ) -> CreativeWorkflowDefinitionV1 {
        let mut definition = workflow_definition();
        let template_id = definition.templates[0].id.clone();
        let draft_id = nomifun_common::generate_id();
        let generate_id = nomifun_common::generate_id();
        definition.output = CreativeWorkflowOutputPlan::MultiImageSeries {
            target_count: 2,
            concurrency: 2,
            review_required: true,
        };
        definition.steps = vec![
            CreativeWorkflowStep::DraftPrompts {
                id: draft_id.clone(),
                name: "规划提示词".into(),
                depends_on: Vec::new(),
                enabled: true,
                template_id,
                planning: crate::workflow::CreativeWorkflowPromptPlanningSettings {
                    model: Some(crate::workflow::CreativeWorkflowTextModelBinding {
                        provider_id: provider_id.into(),
                        model: model.into(),
                        task: crate::workflow::CreativeWorkflowTextTask::Chat,
                    }),
                    instruction: "保持系列连贯".into(),
                    max_tokens: 4096,
                },
            },
            CreativeWorkflowStep::GenerateImages {
                id: generate_id,
                name: "生成图片".into(),
                depends_on: vec![draft_id.clone()],
                enabled: true,
                prompt_source: CreativeWorkflowPromptSource::PromptDrafts {
                    step_id: draft_id,
                },
                reference_variable_ids: Vec::new(),
                generation: crate::workflow::CreativeWorkflowImageGenerationSettings {
                    model: None,
                    quality: crate::workflow::CreativeWorkflowImageQuality::Auto,
                    width: 1024,
                    height: 1024,
                    images_per_prompt: 1,
                },
            },
        ];
        definition
    }

    #[tokio::test]
    async fn creative_workflow_service_persists_closed_definitions_with_cas() {
        let (service, _dir) = service().await;
        let created = service
            .create_creative_workflow(workflow_definition())
            .await
            .unwrap();
        assert_eq!(created.revision, 1);
        assert!(created.metadata.created_at > 0);
        assert_eq!(
            service.list_creative_workflows().await.unwrap(),
            vec![created.clone()]
        );

        let mut replacement = created.clone();
        replacement.revision = 2;
        replacement.metadata.name = "高端电商海报".into();
        let saved = service
            .save_creative_workflow(&created.id, "1", replacement.clone())
            .await
            .unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.metadata.name, "高端电商海报");
        assert_eq!(saved.metadata.created_at, created.metadata.created_at);

        let stale = service
            .save_creative_workflow(&created.id, "1", replacement)
            .await
            .unwrap_err();
        assert!(matches!(stale, AppError::Conflict(_)));

        service.delete_creative_workflow(&created.id).await.unwrap();
        assert!(matches!(
            service.get_creative_workflow(&created.id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn creative_workflow_model_binding_requires_one_enabled_exact_task() {
        let (service, _dir, database) = service_with_database_and_lifecycle(None).await;
        let provider_id = ProviderId::new().into_string();
        let mut definition = workflow_definition();
        if let CreativeWorkflowStep::GenerateImages { generation, .. } = &mut definition.steps[1] {
            generation.model = Some(crate::workflow::CreativeWorkflowImageModelBinding {
                provider_id: provider_id.clone(),
                model: "image-model".into(),
                task: crate::workflow::CreativeWorkflowImageTask::ImageGeneration,
            });
        }
        assert!(matches!(
            service.create_creative_workflow(definition.clone()).await,
            Err(AppError::Conflict(_))
        ));

        insert_provider(&database, &provider_id).await;
        insert_provider_model(&database, &provider_id, "image-model").await;
        assert!(matches!(
            service.create_creative_workflow(definition.clone()).await,
            Err(AppError::Conflict(_))
        ));

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
        let created = service.create_creative_workflow(definition).await.unwrap();
        assert_eq!(created.image_model_bindings().count(), 1);

        let text_provider_id = ProviderId::new().into_string();
        insert_provider(&database, &text_provider_id).await;
        insert_provider_model(&database, &text_provider_id, "chat-model").await;
        let text_definition = series_workflow_definition(&text_provider_id, "chat-model");
        assert!(matches!(
            service
                .create_creative_workflow(text_definition.clone())
                .await,
            Err(AppError::Conflict(_))
        ));
        insert_provider_model_capability(
            &database,
            &text_provider_id,
            "chat-model",
            "chat",
        )
        .await;
        let created = service
            .create_creative_workflow(text_definition)
            .await
            .unwrap();
        assert_eq!(created.text_model_bindings().count(), 1);
    }

    async fn insert_provider_model(
        db: &nomifun_db::Database,
        provider_id: &str,
        model: &str,
    ) {
        nomifun_db::sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, ?, 1, 0, NULL, 1, 1)",
        )
        .bind(provider_id)
        .bind(model)
        .execute(db.pool())
        .await
        .unwrap();
    }

    async fn insert_provider_model_capability(
        db: &nomifun_db::Database,
        provider_id: &str,
        model: &str,
        task: &str,
    ) {
        nomifun_db::sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, \
                 allow_cross_origin_credentials, provider_params, created_at, updated_at) \
             VALUES (?, ?, ?, '[]', 'openai.images', 'default', 0, '{}', 1, 1)",
        )
        .bind(provider_id)
        .bind(model)
        .bind(task)
        .execute(db.pool())
        .await
        .unwrap();
    }

    fn creative_config_node(
        id: &str,
        provider_id: Option<&str>,
        model: Option<&str>,
    ) -> crate::creative_studio::CreativeNode {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "type": "config",
            "position": { "x": 0, "y": 0 },
            "size": { "width": 320, "height": 240 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": {
                "task": "image_generation",
                "capability": "t2i",
                "providerId": provider_id,
                "model": model,
                "prompt": "",
                "negativePrompt": "",
                "parameters": {},
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [],
                "status": "idle",
                "errorMessage": null
            }
        }))
        .unwrap()
    }

    // A 1x1 PNG.
    fn png_1x1() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&[0, 0, 0, 13]);
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b
    }

    fn document_with_one_text_node(project_id: &str, label: &str) -> CreativeProjectDocument {
        serde_json::from_value(serde_json::json!({
            "schema": CREATIVE_STUDIO_SCHEMA,
            "projectId": project_id,
            "viewport": { "x": 0, "y": 0, "zoom": 1 },
            "background": "dots",
            "nodes": [{
                "id": "node-a",
                "type": "text",
                "position": { "x": 10, "y": 20 },
                "size": { "width": 320, "height": 180 },
                "groupId": null,
                "zIndex": 1,
                "locked": false,
                "data": {
                    "text": label,
                    "format": "plain",
                    "fontSize": 16,
                    "textAlign": "left"
                }
            }],
            "connections": [],
            "chatSessions": [],
            "activeChatId": null,
            "panels": {
                "left": { "open": true, "width": 320, "activeView": "canvas" },
                "right": { "open": true, "width": 360, "activeView": "assistant" },
                "bottom": { "open": false, "height": 240, "activeView": "timeline" }
            },
            "pendingTaskIds": []
        }))
        .unwrap()
    }

    #[test]
    fn creative_archive_file_guard_removes_uncommitted_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let rolled_back = dir.path().join("rolled-back.bin");
        std::fs::write(&rolled_back, b"asset").unwrap();
        {
            let mut guard = CreativeArchiveFileRollback::new();
            guard.track(rolled_back.clone());
        }
        assert!(!rolled_back.exists());

        let committed = dir.path().join("committed.bin");
        std::fs::write(&committed, b"asset").unwrap();
        {
            let mut guard = CreativeArchiveFileRollback::new();
            guard.track(committed.clone());
            guard.commit();
        }
        assert!(committed.exists());
    }

    #[tokio::test]
    async fn creative_project_crud_isolated_from_legacy_canvases() {
        let (svc, _dir) = service().await;
        let legacy = svc.create_canvas(Some("旧画布".into())).await.unwrap();
        assert!(svc.list_creative_projects().await.unwrap().is_empty());

        let created = svc
            .create_creative_project(Some("  新项目  ".into()))
            .await
            .unwrap();
        assert_eq!(created.title, "新项目");
        assert_eq!(created.revision, "1");
        assert_eq!(created.node_count, 0);
        assert_eq!(created.connection_count, 0);
        assert!(CreativeStudioProjectId::parse(&created.project_id).is_ok());

        let detail = svc.get_creative_project(&created.project_id).await.unwrap();
        assert_eq!(detail.document.schema, CREATIVE_STUDIO_SCHEMA);
        assert_eq!(detail.document.project_id, created.project_id);
        assert_eq!(svc.list_canvases().await.unwrap()[0].canvas_id, legacy.canvas_id);

        let renamed = svc
            .rename_creative_project(&created.project_id, "  重命名  ")
            .await
            .unwrap();
        assert_eq!(renamed.title, "重命名");
        assert_eq!(renamed.revision, "1", "rename must not invalidate autosave");

        let mut document = document_with_one_text_node(&created.project_id, "first");
        document.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id": "node-b",
                "type": "image",
                "position": { "x": 400, "y": 20 },
                "size": { "width": 320, "height": 180 },
                "groupId": null,
                "zIndex": 2,
                "locked": false,
                "data": {
                    "assetId": null,
                    "caption": "",
                    "alt": "",
                    "fit": "cover",
                    "naturalSize": null
                }
            }))
            .unwrap(),
        );
        document
            .connections
            .push(crate::creative_studio::CreativeConnection {
                id: "connection-a".into(),
                source_node_id: "node-a".into(),
                target_node_id: "node-b".into(),
                source_handle: None,
                target_handle: None,
            });
        let saved = svc
            .save_creative_project(&created.project_id, "1", &document)
            .await
            .unwrap();
        assert_eq!(saved.revision, "2");
        assert_eq!(saved.node_count, 2);
        assert_eq!(saved.connection_count, 1);

        let saved_detail = svc.get_creative_project(&created.project_id).await.unwrap();
        assert_eq!(
            saved_detail.project.node_count as usize,
            saved_detail.document.nodes.len()
        );
        assert_eq!(
            saved_detail.project.connection_count as usize,
            saved_detail.document.connections.len()
        );

        let stale = svc
            .save_creative_project(&created.project_id, "1", &document)
            .await
            .unwrap_err();
        assert!(matches!(stale, AppError::Conflict(_)));

        svc.delete_creative_project(&created.project_id)
            .await
            .unwrap();
        assert!(matches!(
            svc.get_creative_project(&created.project_id).await,
            Err(AppError::NotFound(_))
        ));
        assert!(svc.get_canvas(&legacy.canvas_id).await.is_ok());
    }

    #[tokio::test]
    async fn creative_project_archive_round_trip_copies_assets_and_remaps_graph() {
        let (svc, _dir) = service().await;
        let image_bytes = png_1x1();
        let image = svc
            .ingest_asset_bytes(
                image_bytes.clone(),
                "image/png",
                "归档图片",
                true,
                None,
            )
            .await
            .unwrap();
        let text = svc
            .create_text_asset(NewTextAsset {
                title: "归档文本".into(),
                text_content: "portable prompt".into(),
                collection: Some("归档".into()),
                tags: Some(vec!["prompt".into()]),
                in_library: Some(false),
            })
            .await
            .unwrap();
        let project = svc
            .create_creative_project(Some("可移植项目".into()))
            .await
            .unwrap();
        let document: CreativeProjectDocument = serde_json::from_value(serde_json::json!({
            "schema": CREATIVE_STUDIO_SCHEMA,
            "projectId": project.project_id,
            "viewport": { "x": 0, "y": 0, "zoom": 1 },
            "background": "lines",
            "nodes": [
                {
                    "id": "image-node",
                    "type": "image",
                    "position": { "x": 0, "y": 0 },
                    "size": { "width": 320, "height": 180 },
                    "groupId": null,
                    "zIndex": 1,
                    "locked": false,
                    "data": {
                        "assetId": image.asset_id,
                        "caption": "",
                        "alt": "asset",
                        "fit": "contain",
                        "naturalSize": null
                    }
                },
                {
                    "id": "config-node",
                    "type": "config",
                    "position": { "x": 400, "y": 0 },
                    "size": { "width": 320, "height": 180 },
                    "groupId": null,
                    "zIndex": 2,
                    "locked": false,
                    "data": {
                        "task": "image_generation",
                        "capability": "text-to-image",
                        "providerId": null,
                        "model": null,
                        "prompt": "portable",
                        "negativePrompt": "",
                        "parameters": {},
                        "inputAssetIds": [text.asset_id],
                        "taskId": null,
                        "resultAssetIds": [],
                        "status": "idle",
                        "errorMessage": null
                    }
                }
            ],
            "connections": [{
                "id": "edge-a",
                "sourceNodeId": "image-node",
                "targetNodeId": "config-node",
                "sourceHandle": null,
                "targetHandle": null
            }],
            "chatSessions": [],
            "activeChatId": null,
            "panels": {
                "left": { "open": true, "width": 320, "activeView": "canvas" },
                "right": { "open": true, "width": 360, "activeView": "assistant" },
                "bottom": { "open": false, "height": 240, "activeView": "history" }
            },
            "pendingTaskIds": []
        }))
        .unwrap();
        svc.save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let exported = svc
            .export_creative_project_archive(&project.project_id)
            .await
            .unwrap();
        assert_eq!(exported.mime, CREATIVE_STUDIO_ARCHIVE_MIME);
        assert!(exported.file_name.ends_with(".nomifun-canvas.zip"));
        let imported = svc
            .import_creative_project_archive(exported.bytes)
            .await
            .unwrap();
        assert_ne!(imported.project_id, project.project_id);
        assert_eq!(imported.title, "可移植项目");
        assert_eq!(imported.revision, "1");

        let imported_detail = svc
            .get_creative_project(&imported.project_id)
            .await
            .unwrap();
        assert_ne!(imported_detail.document.nodes[0].id, "image-node");
        assert_ne!(imported_detail.document.connections[0].id, "edge-a");
        assert_eq!(
            imported_detail.document.connections[0].source_node_id,
            imported_detail.document.nodes[0].id
        );
        let crate::creative_studio::CreativeNodeData::Image(imported_image) =
            &imported_detail.document.nodes[0].data
        else {
            panic!("expected imported image node")
        };
        let imported_image_id = imported_image.asset_id.as_deref().unwrap();
        assert_ne!(imported_image_id, image.asset_id);
        assert_eq!(
            svc.read_asset_bytes(imported_image_id).await.unwrap().0,
            image_bytes
        );
        let crate::creative_studio::CreativeNodeData::Config(imported_config) =
            &imported_detail.document.nodes[1].data
        else {
            panic!("expected imported config node")
        };
        assert_ne!(imported_config.input_asset_ids[0], text.asset_id);
        assert_eq!(
            svc.read_asset_bytes(&imported_config.input_asset_ids[0])
                .await
                .unwrap()
                .0,
            b"portable prompt"
        );
    }

    #[tokio::test]
    async fn creative_project_save_rejects_wrong_contract_and_oversize() {
        let (svc, _dir) = service().await;
        let created = svc.create_creative_project(None).await.unwrap();
        assert_eq!(created.title, "未命名画布");

        let mut wrong_schema = document_with_one_text_node(&created.project_id, "bad");
        wrong_schema.schema = "1".into();
        assert!(matches!(
            svc.save_creative_project(&created.project_id, "1", &wrong_schema)
                .await,
            Err(AppError::BadRequest(_))
        ));

        let mut wrong_project = document_with_one_text_node(&created.project_id, "bad");
        wrong_project.project_id = CreativeStudioProjectId::new().into_string();
        assert!(matches!(
            svc.save_creative_project(&created.project_id, "1", &wrong_project)
                .await,
            Err(AppError::BadRequest(_))
        ));

        let mut oversize = document_with_one_text_node(&created.project_id, "large");
        let crate::creative_studio::CreativeNodeData::Text(text) = &mut oversize.nodes[0].data
        else {
            panic!("fixture must contain a text node");
        };
        text.text = "x".repeat(MAX_CREATIVE_PROJECT_DOCUMENT_BYTES);
        assert!(matches!(
            svc.save_creative_project(&created.project_id, "1", &oversize)
                .await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(
            svc.get_creative_project(&created.project_id)
                .await
                .unwrap()
                .project
                .revision,
            "1"
        );
    }

    #[tokio::test]
    async fn creative_project_concurrent_save_allows_exactly_one_winner() {
        let (svc, _dir) = service().await;
        let created = svc.create_creative_project(None).await.unwrap();
        let first = document_with_one_text_node(&created.project_id, "first");
        let second = document_with_one_text_node(&created.project_id, "second");
        let (left, right) = tokio::join!(
            svc.save_creative_project(&created.project_id, "1", &first),
            svc.save_creative_project(&created.project_id, "1", &second),
        );
        let results = [left, right];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AppError::Conflict(_))))
                .count(),
            1
        );
        assert_eq!(
            svc.get_creative_project(&created.project_id)
                .await
                .unwrap()
                .project
                .revision,
            "2"
        );
    }

    #[tokio::test]
    async fn canvas_create_read_save_delete() {
        let (svc, dir) = service().await;
        let meta = svc.create_canvas(None).await.unwrap();
        assert_eq!(meta.title, "未命名画布");
        assert!(WorkshopCanvasId::parse(&meta.canvas_id).is_ok());
        assert!(dir.path().join("workshop/canvases").join(&meta.canvas_id).join("canvas.json").exists());

        // default doc parses; save a doc with 2 nodes → node_count syncs.
        let read = svc.get_canvas(&meta.canvas_id).await.unwrap();
        assert_eq!(read.doc["schema"], 1);
        let doc = serde_json::json!({
            "schema": 1,
            "nodes": [
                {"id": WorkshopNodeId::new().into_string()},
                {"id": WorkshopNodeId::new().into_string()}
            ],
            "edges": []
        });
        let updated_at = svc.save_doc(&meta.canvas_id, &doc).await.unwrap();
        assert!(updated_at >= meta.created_at);
        let all = svc.list_canvases().await.unwrap();
        assert_eq!(all[0].node_count, 2);

        // rename
        let renamed = svc.rename_canvas(&meta.canvas_id, "  我的画布  ").await.unwrap();
        assert_eq!(renamed.title, "我的画布");
        assert!(svc.rename_canvas(&meta.canvas_id, "   ").await.is_err());

        // delete removes row + dir
        svc.delete_canvas(&meta.canvas_id).await.unwrap();
        assert!(!dir.path().join("workshop/canvases").join(&meta.canvas_id).exists());
        assert!(svc.get_canvas(&meta.canvas_id).await.is_err());
    }

    #[tokio::test]
    async fn managed_data_audit_ignores_corrupt_and_missing_retired_canvas_files() {
        let (svc, dir) = service().await;
        let corrupt_canvas = svc.create_canvas(Some("corrupt retired canvas".into())).await.unwrap();
        let corrupt_path = dir
            .path()
            .join("workshop/canvases")
            .join(&corrupt_canvas.canvas_id)
            .join("canvas.json");
        let corrupt = br#"{"schema":1,"nodes":[{"id":"node_legacy"}],"edges":[]}"#;
        tokio::fs::write(&corrupt_path, corrupt).await.unwrap();

        let missing_canvas = svc.create_canvas(Some("missing retired canvas".into())).await.unwrap();
        let missing_path = dir
            .path()
            .join("workshop/canvases")
            .join(&missing_canvas.canvas_id)
            .join("canvas.json");
        tokio::fs::remove_file(&missing_path).await.unwrap();

        svc.audit_managed_data_on_boot().await.unwrap();
        assert_eq!(tokio::fs::read(&corrupt_path).await.unwrap(), corrupt);
        assert!(!missing_path.exists());
        assert_eq!(svc.list_canvases().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn managed_data_audit_still_rejects_missing_canonical_project_assets() {
        let (svc, _dir) = service().await;
        let project = svc.create_creative_project(None).await.unwrap();
        let missing_asset_id = WorkshopAssetId::new().into_string();
        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id": "missing-image",
                "type": "image",
                "position": { "x": 0, "y": 0 },
                "size": { "width": 320, "height": 240 },
                "groupId": null,
                "zIndex": 1,
                "locked": false,
                "data": {
                    "assetId": missing_asset_id,
                    "caption": "",
                    "alt": "",
                    "fit": "contain",
                    "naturalSize": null
                }
            }))
            .unwrap(),
        );
        svc.save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let error = svc.audit_managed_data_on_boot().await.unwrap_err();
        assert!(matches!(
            error,
            AppError::Internal(ref message)
                if message.contains("creative studio project references missing asset")
                    && message.contains(&missing_asset_id)
        ));
    }

    #[tokio::test]
    async fn save_doc_rejects_oversize_and_unknown_canvas() {
        let (svc, _dir) = service().await;
        assert!(
            svc.save_doc(
                "0190f5fe-7c00-7a00-8000-000000000099",
                &serde_json::json!({})
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn canonical_save_requires_one_existing_provider_model_pair() {
        let barrier = Arc::new(ProviderLifecycleBarrier::new());
        let (svc, _dir, db) =
            service_with_database_and_lifecycle(Some(barrier)).await;
        let project = svc.create_creative_project(None).await.unwrap();
        let missing_provider_id = "0190f5fe-7c00-7a00-8000-000000000081";
        let existing_provider_id = "0190f5fe-7c00-7a00-8000-000000000082";
        insert_provider(&db, existing_provider_id).await;

        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(creative_config_node(
            "missing-provider",
            Some(missing_provider_id),
            Some("image-model"),
        ));
        let missing_provider = svc
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap_err();
        assert!(matches!(
            missing_provider,
            AppError::Conflict(ref message)
                if message.contains("missing provider-model")
                    && message.contains(missing_provider_id)
        ));

        document.nodes[0] = creative_config_node(
            "missing-model",
            Some(existing_provider_id),
            Some("unknown-model"),
        );
        let missing_model = svc
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap_err();
        assert!(matches!(
            missing_model,
            AppError::Conflict(ref message)
                if message.contains("missing provider-model")
                    && message.contains("unknown-model")
        ));

        document.nodes[0] = creative_config_node(
            "partial-pair",
            Some(existing_provider_id),
            None,
        );
        assert!(matches!(
            svc.save_creative_project(&project.project_id, "1", &document)
                .await,
            Err(AppError::BadRequest(message)) if message.contains("must be set together")
        ));

        insert_provider_model(&db, existing_provider_id, "image-model").await;
        document.nodes[0] = creative_config_node(
            "valid-pair",
            Some(existing_provider_id),
            Some("image-model"),
        );
        let saved = svc
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();
        assert_eq!(saved.revision, "2");
    }

    #[tokio::test]
    async fn save_doc_enforces_generator_provider_logical_references() {
        let (svc, _dir, _db) = service_with_database_and_lifecycle(None).await;
        let canvas = svc.create_canvas(Some("provider contract".into())).await.unwrap();
        let missing_provider_id = "0190f5fe-7c00-7a00-8000-000000000085";
        let base = serde_json::json!({
            "schema": 1,
            "nodes": [{
                "id": WorkshopNodeId::new().into_string(),
                "kind": "generator",
                "data": {
                    "providerId": missing_provider_id,
                    "model": "image-model"
                }
            }],
            "edges": []
        });

        let error = svc.save_doc(&canvas.canvas_id, &base).await.unwrap_err();
        assert!(
            matches!(
                error,
                AppError::Conflict(ref message)
                    if message.contains("references missing provider")
            ),
            "missing logical parent must be rejected; got {error:?}"
        );

        let mut noncanonical = base.clone();
        noncanonical["nodes"][0]["data"]["providerId"] =
            serde_json::json!(format!("provider_{missing_provider_id}"));
        let mut missing_model = base.clone();
        missing_model["nodes"][0]["data"]
            .as_object_mut()
            .unwrap()
            .remove("model");
        let mut missing_provider = base;
        missing_provider["nodes"][0]["data"]
            .as_object_mut()
            .unwrap()
            .remove("providerId");

        for (case, invalid) in [
            ("non-canonical provider", noncanonical),
            ("provider without model", missing_model),
            ("model without provider", missing_provider),
        ] {
            let error = svc.save_doc(&canvas.canvas_id, &invalid).await.unwrap_err();
            assert!(
                matches!(error, AppError::BadRequest(_)),
                "{case} must be rejected as an invalid fixed pair; got {error:?}"
            );
        }

        assert_eq!(
            svc.get_canvas(&canvas.canvas_id).await.unwrap().doc,
            default_doc_value(),
            "rejected logical references must not replace the persisted document"
        );
    }

    #[tokio::test]
    async fn provider_cleanup_cas_clears_only_target_canonical_pairs_and_is_idempotent() {
        let barrier = Arc::new(ProviderLifecycleBarrier::new());
        let (svc, _dir, db) =
            service_with_database_and_lifecycle(Some(barrier.clone())).await;
        let target_provider_id = "0190f5fe-7c00-7a00-8000-000000000086";
        let other_provider_id = "0190f5fe-7c00-7a00-8000-000000000087";
        insert_provider(&db, target_provider_id).await;
        insert_provider(&db, other_provider_id).await;
        insert_provider_model(&db, target_provider_id, "delete-me").await;
        insert_provider_model(&db, other_provider_id, "keep-me").await;
        insert_provider_model_capability(
            &db,
            target_provider_id,
            "delete-me",
            "image_generation",
        )
        .await;
        insert_provider_model_capability(
            &db,
            other_provider_id,
            "keep-me",
            "image_generation",
        )
        .await;
        insert_provider_model_capability(&db, target_provider_id, "delete-me", "chat").await;
        insert_provider_model_capability(&db, other_provider_id, "keep-me", "chat").await;
        let project = svc.create_creative_project(Some("provider cleanup".into())).await.unwrap();
        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(creative_config_node(
            "target-config",
            Some(target_provider_id),
            Some("delete-me"),
        ));
        document.nodes.push(creative_config_node(
            "surviving-config",
            Some(other_provider_id),
            Some("keep-me"),
        ));
        svc.save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let mut target_workflow = workflow_definition();
        if let CreativeWorkflowStep::GenerateImages { generation, .. } =
            &mut target_workflow.steps[1]
        {
            generation.model = Some(crate::workflow::CreativeWorkflowImageModelBinding {
                provider_id: target_provider_id.into(),
                model: "delete-me".into(),
                task: crate::workflow::CreativeWorkflowImageTask::ImageGeneration,
            });
        }
        let target_workflow = svc
            .create_creative_workflow(target_workflow)
            .await
            .unwrap();
        let mut surviving_workflow = workflow_definition();
        if let CreativeWorkflowStep::GenerateImages { generation, .. } =
            &mut surviving_workflow.steps[1]
        {
            generation.model = Some(crate::workflow::CreativeWorkflowImageModelBinding {
                provider_id: other_provider_id.into(),
                model: "keep-me".into(),
                task: crate::workflow::CreativeWorkflowImageTask::ImageGeneration,
            });
        }
        let surviving_workflow = svc
            .create_creative_workflow(surviving_workflow)
            .await
            .unwrap();
        let target_planning_workflow = svc
            .create_creative_workflow(series_workflow_definition(
                target_provider_id,
                "delete-me",
            ))
            .await
            .unwrap();
        let surviving_planning_workflow = svc
            .create_creative_workflow(series_workflow_definition(
                other_provider_id,
                "keep-me",
            ))
            .await
            .unwrap();

        let _write_guard = barrier.write().await;
        svc.clear_provider_references_under_lifecycle_write_guard(target_provider_id)
            .await
            .unwrap();
        let cleaned_once = svc
            .get_creative_workflow(&target_workflow.id)
            .await
            .unwrap();
        assert_eq!(cleaned_once.revision, 2);
        assert_eq!(cleaned_once.image_model_bindings().count(), 0);
        let cleaned_planning = svc
            .get_creative_workflow(&target_planning_workflow.id)
            .await
            .unwrap();
        assert_eq!(cleaned_planning.revision, 2);
        assert_eq!(cleaned_planning.text_model_bindings().count(), 0);
        svc.clear_provider_references_under_lifecycle_write_guard(target_provider_id)
            .await
            .unwrap();

        let cleaned = svc.get_creative_project(&project.project_id).await.unwrap();
        assert_eq!(cleaned.project.revision, "3");
        let CreativeNodeData::Config(target) = &cleaned.document.nodes[0].data else {
            panic!("expected target config node")
        };
        assert_eq!(target.provider_id, None);
        assert_eq!(target.model, None);
        let CreativeNodeData::Config(surviving) = &cleaned.document.nodes[1].data else {
            panic!("expected surviving config node")
        };
        assert_eq!(surviving.provider_id.as_deref(), Some(other_provider_id));
        assert_eq!(surviving.model.as_deref(), Some("keep-me"));

        let cleaned_twice = svc
            .get_creative_workflow(&target_workflow.id)
            .await
            .unwrap();
        assert_eq!(cleaned_twice.revision, 2);
        assert_eq!(cleaned_twice.image_model_bindings().count(), 0);
        let surviving_workflow = svc
            .get_creative_workflow(&surviving_workflow.id)
            .await
            .unwrap();
        assert_eq!(surviving_workflow.revision, 1);
        let surviving_binding = surviving_workflow
            .image_model_bindings()
            .next()
            .expect("unrelated workflow binding must survive provider cleanup");
        assert_eq!(surviving_binding.provider_id, other_provider_id);
        assert_eq!(surviving_binding.model, "keep-me");
        let surviving_planning_workflow = svc
            .get_creative_workflow(&surviving_planning_workflow.id)
            .await
            .unwrap();
        assert_eq!(surviving_planning_workflow.revision, 1);
        let surviving_planning_binding = surviving_planning_workflow
            .text_model_bindings()
            .next()
            .expect("unrelated workflow planning binding must survive provider cleanup");
        assert_eq!(surviving_planning_binding.provider_id, other_provider_id);
        assert_eq!(surviving_planning_binding.model, "keep-me");
    }

    #[tokio::test]
    async fn provider_lifecycle_write_guard_blocks_workshop_doc_writes() {
        let barrier = Arc::new(ProviderLifecycleBarrier::new());
        let (svc, _dir, db) =
            service_with_database_and_lifecycle(Some(barrier.clone())).await;
        let provider_id = "0190f5fe-7c00-7a00-8000-000000000088";
        insert_provider(&db, provider_id).await;
        let canvas = svc.create_canvas(Some("barrier".into())).await.unwrap();
        let doc = serde_json::json!({
            "schema": 1,
            "nodes": [{
                "id": WorkshopNodeId::new().into_string(),
                "kind": "generator",
                "data": {"providerId": provider_id, "model": "image-model"}
            }],
            "edges": []
        });

        let write_guard = barrier.write().await;
        let service = svc.clone();
        let canvas_id = canvas.canvas_id.clone();
        let mut save = tokio::spawn(async move { service.save_doc(&canvas_id, &doc).await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut save)
                .await
                .is_err(),
            "a workshop write must wait while Provider deletion holds the lifecycle write guard"
        );
        drop(write_guard);
        save.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn canonical_save_cannot_resurrect_provider_during_deletion() {
        let barrier = Arc::new(ProviderLifecycleBarrier::new());
        let (svc, _dir, db) =
            service_with_database_and_lifecycle(Some(barrier.clone())).await;
        let provider_id = "0190f5fe-7c00-7a00-8000-000000000089";
        insert_provider(&db, provider_id).await;
        insert_provider_model(&db, provider_id, "image-model").await;
        let project = svc.create_creative_project(Some("delete race".into())).await.unwrap();
        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(creative_config_node(
            "config-node",
            Some(provider_id),
            Some("image-model"),
        ));
        svc.save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let write_guard = barrier.write().await;
        let service = svc.clone();
        let project_id = project.project_id.clone();
        let blocked_document = document.clone();
        let mut save = tokio::spawn(async move {
            service
                .save_creative_project(&project_id, "2", &blocked_document)
                .await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut save)
                .await
                .is_err(),
            "canonical save must wait while Provider deletion owns the lifecycle write guard"
        );

        svc.clear_provider_references_under_lifecycle_write_guard(provider_id)
            .await
            .unwrap();
        SqliteProviderRepository::new(db.pool().clone())
            .delete(provider_id)
            .await
            .unwrap();
        drop(write_guard);

        let error = save.await.unwrap().unwrap_err();
        assert!(
            matches!(
                error,
                AppError::Conflict(ref message) if message.contains("missing provider-model")
            ),
            "save resumed with an unexpected error after Provider deletion: {error:?}"
        );
        let cleaned = svc.get_creative_project(&project.project_id).await.unwrap();
        assert_eq!(cleaned.project.revision, "3");
        let CreativeNodeData::Config(config) = &cleaned.document.nodes[0].data else {
            panic!("expected config node")
        };
        assert_eq!(config.provider_id, None);
        assert_eq!(config.model, None);
    }

    #[tokio::test]
    async fn canvas_doc_save_enforces_canonical_ids_and_deep_references() {
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("identity contract".into())).await.unwrap();
        let group_id = WorkshopNodeId::new().into_string();
        let member_id = WorkshopNodeId::new().into_string();
        let edge_id = WorkshopEdgeId::new().into_string();
        let valid = serde_json::json!({
            "schema": 1,
            "nodes": [
                {"id": group_id, "kind": "group", "data": {}},
                {
                    "id": member_id,
                    "kind": "generator",
                    "groupId": group_id,
                    "data": {"mentions": [format!("node:{group_id}")]}
                }
            ],
            "edges": [{"id": edge_id, "from": group_id, "to": member_id}]
        });
        svc.save_doc(&canvas.canvas_id, &valid).await.unwrap();

        let mut non_v7_node_id = valid.clone();
        non_v7_node_id["nodes"][0]["id"] =
            serde_json::json!("550e8400-e29b-41d4-a716-446655440000");
        let mut duplicate_node = valid.clone();
        let duplicated_id = duplicate_node["nodes"][0]["id"].clone();
        duplicate_node["nodes"][1]["id"] = duplicated_id;
        let mut missing_group = valid.clone();
        missing_group["nodes"][1]["groupId"] =
            serde_json::json!(WorkshopNodeId::new().into_string());
        let mut legacy_mention = valid.clone();
        legacy_mention["nodes"][1]["data"]["mentions"] = serde_json::json!(["node:legacy-node"]);
        let mut non_v7_edge_id = valid.clone();
        non_v7_edge_id["edges"][0]["id"] =
            serde_json::json!("550e8400-e29b-41d4-a716-446655440001");
        let mut missing_endpoint = valid.clone();
        missing_endpoint["edges"][0]["to"] =
            serde_json::json!(WorkshopNodeId::new().into_string());

        for (case, invalid) in [
            ("non-v7 node id", non_v7_node_id),
            ("duplicate node id", duplicate_node),
            ("missing group", missing_group),
            ("legacy mention", legacy_mention),
            ("non-v7 edge id", non_v7_edge_id),
            ("missing endpoint", missing_endpoint),
        ] {
            let error = svc
                .save_doc(&canvas.canvas_id, &invalid)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::BadRequest(_)), "{case}: {error}");
        }

        // A rejected write must not replace the last valid document.
        assert_eq!(svc.get_canvas(&canvas.canvas_id).await.unwrap().doc, valid);
    }

    #[tokio::test]
    async fn canvas_doc_read_fails_closed_when_disk_ids_are_not_canonical() {
        let (svc, dir) = service().await;
        let canvas = svc.create_canvas(Some("corrupt identity".into())).await.unwrap();
        let path = dir
            .path()
            .join("workshop/canvases")
            .join(&canvas.canvas_id)
            .join("canvas.json");
        tokio::fs::write(
            path,
            br#"{"schema":1,"nodes":[{"id":"legacy-node"}],"edges":[]}"#,
        )
        .await
        .unwrap();

        let Err(error) = svc.get_canvas(&canvas.canvas_id).await else {
            panic!("corrupt managed canvas must not be served");
        };
        assert!(
            matches!(error, AppError::Internal(message) if message.contains("invalid durable IDs"))
        );
    }

    #[tokio::test]
    async fn upload_image_extracts_dimensions_and_serves() {
        let (svc, _dir) = service().await;
        let asset = svc
            .upload_asset(NewAssetUpload {
                file_name: "shot.png".into(),
                content_type: Some("image/png".into()),
                bytes: png_1x1(),
                title: None,
                collection: Some("角色".into()),
                tags: Some(vec!["a".into()]),
                in_library: None,
            })
            .await
            .unwrap();
        assert_eq!(asset.kind, "image");
        assert_eq!(asset.width, Some(1));
        assert_eq!(asset.height, Some(1));
        assert!(asset.in_library);
        assert_eq!(
            asset.url,
            format!("/api/creative-studio/files/{}", asset.asset_id)
        );

        // serve returns the bytes + mime
        let served = svc.serve_file(&asset.asset_id, false).await.unwrap();
        assert_eq!(served.mime, "image/png");
        assert_eq!(served.bytes, png_1x1());
        // thumb=1 falls back to original when no thumb exists
        let served_thumb = svc.serve_file(&asset.asset_id, true).await.unwrap();
        assert_eq!(served_thumb.bytes, png_1x1());
    }

    #[tokio::test]
    async fn upload_rejects_non_media() {
        let (svc, _dir) = service().await;
        let err = svc
            .upload_asset(NewAssetUpload {
                file_name: "notes.txt".into(),
                content_type: Some("text/plain".into()),
                bytes: b"hi".to_vec(),
                title: None,
                collection: None,
                tags: None,
                in_library: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn text_asset_list_patch_delete() {
        let (svc, _dir) = service().await;
        let a = svc
            .create_text_asset(NewTextAsset {
                title: "描述".into(),
                text_content: "武松打虎".into(),
                collection: None,
                tags: None,
                in_library: Some(false),
            })
            .await
            .unwrap();
        assert_eq!(a.kind, "text");
        assert!(!a.in_library);
        assert_eq!(a.text_content.as_deref(), Some("武松打虎"));

        let patched = svc
            .patch_asset(
                &a.asset_id,
                AssetPatch {
                    title: Some("新标题".into()),
                    collection: Some("场景".into()),
                    in_library: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(patched.title, "新标题");
        assert_eq!(patched.collection.as_deref(), Some("场景"));
        assert!(patched.in_library);

        let page = svc
            .list_assets(AssetQuery { page: 1, page_size: 20, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(page.total, 1);

        // text assets serve their inline body as text/plain (no file on disk)
        let served = svc.serve_file(&a.asset_id, false).await.unwrap();
        assert_eq!(served.mime, "text/plain; charset=utf-8");
        assert_eq!(String::from_utf8(served.bytes).unwrap(), a.text_content.clone().unwrap());
        svc.delete_asset(&a.asset_id).await.unwrap();
        assert!(svc.serve_file(&a.asset_id, false).await.is_err());
    }

    /// A real, decodable PNG (unlike the header-only `png_1x1`).
    fn real_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 20, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    async fn upload_png(svc: &WorkshopService, in_library: bool) -> WorkshopAsset {
        svc.upload_asset(NewAssetUpload {
            file_name: "pic.png".into(),
            content_type: Some("image/png".into()),
            bytes: real_png(800, 600),
            title: Some("pic".into()),
            collection: None,
            tags: None,
            in_library: Some(in_library),
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn thumbnail_generated_on_upload_and_served_as_jpeg() {
        let (svc, dir) = service().await;
        let asset = upload_png(&svc, true).await;
        assert!(asset.thumb_url.is_some(), "thumb_url should be advertised");
        assert!(
            dir.path().join(format!("workshop/assets/thumbs/{}.jpg", asset.asset_id)).exists(),
            "thumb file should exist on disk"
        );
        let served = svc.serve_file(&asset.asset_id, true).await.unwrap();
        assert_eq!(served.mime, "image/jpeg");
        assert_eq!(&served.bytes[0..2], &[0xFF, 0xD8], "served thumb is JPEG");
        // original still served untouched
        let orig = svc.serve_file(&asset.asset_id, false).await.unwrap();
        assert_eq!(orig.mime, "image/png");
    }

    #[tokio::test]
    async fn ingest_and_read_asset_bytes_roundtrip() {
        let (svc, _dir) = service().await;
        let png = real_png(300, 200);
        let origin = serde_json::json!({ "prompt": "a cat", "model": "x" });
        let row = svc
            .ingest_asset_bytes(png.clone(), "image/png", "generated", false, Some(origin.clone()))
            .await
            .unwrap();
        assert_eq!(row.kind, "image");
        assert!(!row.in_library);
        assert_eq!(row.width, Some(300));
        assert!(row.thumb_rel_path.is_some());
        assert_eq!(row.origin.as_deref().map(|s| s.contains("a cat")), Some(true));

        let (bytes, mime) = svc.read_asset_bytes(&row.asset_id).await.unwrap();
        assert_eq!(bytes, png);
        assert_eq!(mime, "image/png");

        // unsupported mime rejected
        assert!(svc.ingest_asset_bytes(vec![1], "application/pdf", "x", true, None).await.is_err());
    }

    #[tokio::test]
    async fn canvas_thumbnail_set_and_served() {
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("画布".into())).await.unwrap();
        assert!(canvas.thumbnail_url.is_none());
        let asset = upload_png(&svc, true).await;

        let meta = svc.patch_canvas(&canvas.canvas_id, None, Some(asset.asset_id.clone())).await.unwrap();
        assert_eq!(meta.thumbnail_url.as_deref(), Some(&*format!("/api/workshop/canvas-thumbs/{}", canvas.canvas_id)));
        let served = svc.serve_canvas_thumbnail(&canvas.canvas_id).await.unwrap();
        assert_eq!(served.mime, "image/jpeg");
        assert_eq!(&served.bytes[0..2], &[0xFF, 0xD8]);

        // a text asset cannot be a thumbnail source
        let text = svc
            .create_text_asset(NewTextAsset {
                title: "t".into(),
                text_content: "x".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
            })
            .await
            .unwrap();
        assert!(svc.set_canvas_thumbnail(&canvas.canvas_id, &text.asset_id).await.is_err());
    }

    #[tokio::test]
    async fn delete_canvas_gcs_internal_asset_unless_shared() {
        let (svc, _dir) = service().await;
        // Asset referenced by two canvases; internal (in_library=0).
        let asset = upload_png(&svc, false).await;
        let node_id = WorkshopNodeId::new().into_string();
        let doc = serde_json::json!({
            "schema": 1, "nodes": [{ "id": node_id, "kind": "image", "data": { "assetId": asset.asset_id } }], "edges": []
        });
        let c1 = svc.create_canvas(Some("c1".into())).await.unwrap();
        let c2 = svc.create_canvas(Some("c2".into())).await.unwrap();
        svc.save_doc(&c1.canvas_id, &doc).await.unwrap();
        svc.save_doc(&c2.canvas_id, &doc).await.unwrap();

        // Deleting c1 keeps the asset (c2 still references it).
        svc.delete_canvas(&c1.canvas_id).await.unwrap();
        assert!(svc.serve_file(&asset.asset_id, false).await.is_ok());

        // Deleting c2 (the last referencer) GCs the internal asset + its file.
        svc.delete_canvas(&c2.canvas_id).await.unwrap();
        assert!(svc.serve_file(&asset.asset_id, false).await.is_err());
    }

    #[tokio::test]
    async fn delete_canvas_keeps_library_asset() {
        let (svc, _dir) = service().await;
        let asset = upload_png(&svc, true).await; // in_library=1
        let node_id = WorkshopNodeId::new().into_string();
        let doc = serde_json::json!({
            "schema": 1, "nodes": [{ "id": node_id, "kind": "image", "data": { "assetId": asset.asset_id } }], "edges": []
        });
        let c = svc.create_canvas(Some("c".into())).await.unwrap();
        svc.save_doc(&c.canvas_id, &doc).await.unwrap();
        svc.delete_canvas(&c.canvas_id).await.unwrap();
        // Library assets are never GC'd on canvas delete.
        assert!(svc.serve_file(&asset.asset_id, false).await.is_ok());
    }

    #[tokio::test]
    async fn delete_canvas_grace_protects_recent_internal_asset() {
        // A large grace: an internal asset referenced only by the deleted canvas
        // is NOT reaped while still recent (another open canvas may reference it
        // but not have autosaved yet); a later full GC reclaims it once aged.
        let (svc, _dir) = service_with_gc_grace(GC_GRACE_MS).await;
        let asset = upload_png(&svc, false).await; // canvas-internal
        let node_id = WorkshopNodeId::new().into_string();
        let doc = serde_json::json!({
            "schema": 1, "nodes": [{ "id": node_id, "kind": "image", "data": { "assetId": asset.asset_id } }], "edges": []
        });
        let c = svc.create_canvas(Some("c".into())).await.unwrap();
        svc.save_doc(&c.canvas_id, &doc).await.unwrap();
        svc.delete_canvas(&c.canvas_id).await.unwrap();
        assert!(svc.serve_file(&asset.asset_id, false).await.is_ok(), "recent internal asset survives delete_canvas grace");
    }

    #[tokio::test]
    async fn mark_canvas_open_routes_agent_ops_to_queue() {
        use crate::agent_ops::{AddNodeSpec, AgentOp, OpDisposition};
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("c".into())).await.unwrap();

        // Simulate the editor's REST doc-load registering the canvas as open.
        svc.mark_canvas_open(&canvas.canvas_id);

        // An agent add_node now QUEUES (frontend authority) instead of writing
        // straight to canvas.json — closing the cold-open clobber window.
        let applied = svc
            .apply_agent_ops(
                &canvas.canvas_id,
                vec![AgentOp::AddNode {
                    node: AddNodeSpec { kind: "image".into(), x: None, y: None, w: None, h: None, data: None },
                }],
                "test",
            )
            .await
            .unwrap();
        assert_eq!(applied[0].disposition, OpDisposition::Queued);
        // The doc was NOT touched.
        assert_eq!(svc.get_canvas(&canvas.canvas_id).await.unwrap().meta.node_count, 0);
    }

    #[tokio::test]
    async fn list_assets_ungrouped_filters_serverside() {
        let (svc, _dir) = service().await;
        // Two ungrouped text assets (no collection) + one in a named collection.
        svc.create_text_asset(NewTextAsset {
            title: "散图".into(),
            text_content: "x".into(),
            collection: None,
            tags: None,
            in_library: Some(true),
        })
        .await
        .unwrap();
        svc.create_text_asset(NewTextAsset {
            title: "散图2".into(),
            text_content: "y".into(),
            // A whitespace-only collection normalizes to NULL → still ungrouped.
            collection: Some("   ".into()),
            tags: None,
            in_library: Some(true),
        })
        .await
        .unwrap();
        svc.create_text_asset(NewTextAsset {
            title: "角色图".into(),
            text_content: "z".into(),
            collection: Some("角色".into()),
            tags: None,
            in_library: Some(true),
        })
        .await
        .unwrap();

        // ungrouped=true → only the two collection-less assets.
        let page = svc
            .list_assets(AssetQuery { ungrouped: true, page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        assert!(page.items.iter().all(|a| a.collection.is_none()));

        // Named collection filter is unaffected.
        let grouped = svc
            .list_assets(AssetQuery {
                collection: Some("角色".into()),
                page: 1,
                page_size: 50,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(grouped.total, 1);
        assert_eq!(grouped.items[0].collection.as_deref(), Some("角色"));
    }

    #[tokio::test]
    async fn agent_ops_direct_apply_to_closed_canvas() {
        use crate::agent_ops::{AddNodeSpec, AgentOp, OpDisposition};
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("c".into())).await.unwrap();

        // No frontend has polled → canvas is CLOSED → add_node applies to the doc.
        let ops = vec![
            AgentOp::AddNode {
                node: AddNodeSpec {
                    kind: "generator".into(),
                    x: None,
                    y: None,
                    w: None,
                    h: None,
                    data: Some(serde_json::json!({ "prompt": "a wolf" })),
                },
            },
        ];
        let applied = svc.apply_agent_ops(&canvas.canvas_id, ops, "test").await.unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].disposition, OpDisposition::Applied);
        let node_id = applied[0].node_id.clone().unwrap();

        // The node is persisted in canvas.json and node_count synced.
        let read = svc.get_canvas(&canvas.canvas_id).await.unwrap();
        assert_eq!(read.meta.node_count, 1);
        assert_eq!(read.doc["nodes"][0]["id"], serde_json::json!(node_id));
        assert_eq!(read.doc["nodes"][0]["data"]["prompt"], "a wolf");

        // A connect to that node also applies directly.
        let connect = vec![AgentOp::AddNode {
            node: AddNodeSpec { kind: "image".into(), x: None, y: None, w: None, h: None, data: None },
        }];
        let more = svc.apply_agent_ops(&canvas.canvas_id, connect, "test").await.unwrap();
        let img_id = more[0].node_id.clone().unwrap();
        let edge = svc
            .apply_agent_ops(
                &canvas.canvas_id,
                vec![AgentOp::Connect { from_node_id: node_id, to_node_id: img_id }],
                "test",
            )
            .await
            .unwrap();
        assert_eq!(edge[0].disposition, OpDisposition::Applied);
        let read2 = svc.get_canvas(&canvas.canvas_id).await.unwrap();
        assert_eq!(read2.doc["edges"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn agent_ops_queue_when_open_and_ack_removes() {
        use crate::agent_ops::{AddNodeSpec, AgentOp, OpDisposition};
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("c".into())).await.unwrap();

        // A poll marks the canvas OPEN → even add_node is queued (frontend owns writes).
        assert!(svc.take_pending_ops(&canvas.canvas_id).await.unwrap().is_empty());
        let applied = svc
            .apply_agent_ops(
                &canvas.canvas_id,
                vec![AgentOp::AddNode {
                    node: AddNodeSpec { kind: "image".into(), x: None, y: None, w: None, h: None, data: None },
                }],
                "test",
            )
            .await
            .unwrap();
        assert_eq!(applied[0].disposition, OpDisposition::Queued);
        // The doc was NOT touched (frontend authority preserved).
        assert_eq!(svc.get_canvas(&canvas.canvas_id).await.unwrap().meta.node_count, 0);

        // The op is pullable and stays until acked.
        let pending = svc.take_pending_ops(&canvas.canvas_id).await.unwrap();
        assert_eq!(pending.len(), 1);
        svc.ack_agent_ops(&canvas.canvas_id, &[pending[0].op_id.clone()]);
        assert!(svc.take_pending_ops(&canvas.canvas_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn agent_ops_data_mutations_always_queue_and_bad_ops_rejected() {
        use crate::agent_ops::{AgentOp, OpDisposition};
        let (svc, _dir) = service().await;
        let canvas = svc.create_canvas(Some("c".into())).await.unwrap();

        // delete_node is a data-mutating op → queued even on a closed canvas.
        let applied = svc
            .apply_agent_ops(
                &canvas.canvas_id,
                vec![AgentOp::DeleteNode { node_id: WorkshopNodeId::new().into_string() }],
                "test",
            )
            .await
            .unwrap();
        assert_eq!(applied[0].disposition, OpDisposition::Queued);

        // An invalid op fails the whole batch (BadRequest).
        let node_id = WorkshopNodeId::new().into_string();
        let bad = svc
            .apply_agent_ops(
                &canvas.canvas_id,
                vec![AgentOp::Connect { from_node_id: node_id.clone(), to_node_id: node_id }],
                "test",
            )
            .await;
        assert!(matches!(bad, Err(AppError::BadRequest(_))));

        // Unknown canvas → NotFound.
        assert!(
            svc.take_pending_ops("0190f5fe-7c00-7a00-8000-000000000099")
                .await
                .is_err()
        );
    }
}
