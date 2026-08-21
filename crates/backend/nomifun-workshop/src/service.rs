//! [`WorkshopService`] — the canonical Creative Studio project, asset,
//! workflow, archive, and generation-support service. Project documents live
//! in SQLite; asset binaries live under the service data directory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use nomifun_common::{
    AppError, CreativeStudioProjectId, CreativeStudioWorkflowId, CreativeStudioWorkflowRunId,
    ProviderId, SharedProviderLifecycleBarrier, WorkshopAssetId, now_ms,
};
use nomifun_db::{
    AssetSort, CreativeStudioProjectRow, CreativeStudioWorkflowRunRow, DbError,
    IWorkshopRepository, ListAssetsParams, UpdateAssetParams, WorkshopAssetRow,
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
use crate::creative_agent_ops::{CreativeAgentOp, CreativeAgentOpResult};
use crate::dto::WorkshopAsset;
use crate::prompt_catalog::{CreativePromptCatalogPage, PromptCatalogService};
use crate::workflow::{CreativeWorkflowDefinitionV1, parse_workflow_row};
use crate::workflow_run::{
    CreativeWorkflowRunAggregateV1, CreativeWorkflowRunCreateRequest, parse_workflow_run_row,
};
use crate::{MAX_ASSET_BYTES, WORKSHOP_REL_DIR, fsio, imagemeta, thumbnail};

/// A canonical Creative Studio project and its validated v1 document.
pub struct CreativeProjectWithDocument {
    pub project: CreativeProjectSummary,
    pub document: CreativeProjectDocument,
}

/// One CAS-committed Agent graph mutation batch.
#[derive(Debug)]
pub struct CreativeAgentApplyResult {
    pub project: CreativeProjectSummary,
    pub ops: Vec<CreativeAgentOpResult>,
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

/// Auditable provenance stored with a prompt-catalog text asset.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromptCatalogAssetOrigin {
    pub prompt_catalog_id: String,
    pub source_url: String,
    pub license: String,
    pub license_url: String,
}

/// A `text`-kind asset (no binary; body lives in `text_content`).
pub struct NewTextAsset {
    pub title: String,
    pub text_content: String,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
    /// Optional bounded provenance for a prompt-catalog item. The text itself
    /// remains authoritative in `text_content`; this metadata preserves source
    /// attribution when the user adds a catalog prompt to My Assets.
    pub origin: Option<PromptCatalogAssetOrigin>,
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

const DEFAULT_CREATIVE_PROJECT_TITLE: &str = "未命名画布";
const MAX_CREATIVE_PROJECT_TITLE_CHARS: usize = 1_000;

pub struct WorkshopService {
    repo: Arc<dyn IWorkshopRepository>,
    /// Backend data dir root. Asset `rel_path`s are relative to this.
    data_dir: PathBuf,
    provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    prompt_catalog: PromptCatalogService,
}

impl WorkshopService {
    /// Build the service over its index repo + the data dir root.
    pub fn start(data_dir: &Path, repo: Arc<dyn IWorkshopRepository>) -> Arc<Self> {
        Self::start_with_optional_provider_lifecycle(data_dir, repo, None)
    }

    /// Build the service with the process-wide Provider lifecycle barrier.
    pub fn start_with_provider_lifecycle(
        data_dir: &Path,
        repo: Arc<dyn IWorkshopRepository>,
        provider_lifecycle: SharedProviderLifecycleBarrier,
    ) -> Arc<Self> {
        Self::start_with_optional_provider_lifecycle(data_dir, repo, Some(provider_lifecycle))
    }

    fn start_with_optional_provider_lifecycle(
        data_dir: &Path,
        repo: Arc<dyn IWorkshopRepository>,
        provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repo,
            data_dir: data_dir.to_path_buf(),
            provider_lifecycle,
            prompt_catalog: PromptCatalogService::start(data_dir),
        })
    }

    /// Read the last valid prompt-catalog snapshot without touching the
    /// network. A fresh installation returns an empty, stale page so the
    /// client can explicitly request the owner-only synchronization route.
    pub async fn list_prompt_catalog(&self) -> Result<CreativePromptCatalogPage, AppError> {
        self.prompt_catalog.list().await
    }

    /// Refresh the fixed, attributed upstream prompt sources. Per-source
    /// failures retain the last valid cached entries; a completely empty first
    /// sync fails instead of publishing an empty catalog as success.
    pub async fn sync_prompt_catalog(
        &self,
        force: bool,
    ) -> Result<CreativePromptCatalogPage, AppError> {
        self.prompt_catalog.sync(force).await
    }

    // ---- path helpers ----

    fn workshop_dir(&self) -> PathBuf {
        self.data_dir.join(WORKSHOP_REL_DIR)
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

    /// Validate every durable Creative Studio config-node selection against
    /// the exact managed Provider/model pair. The lifecycle read guard is held
    /// by callers across this check and the subsequent project CAS write.
    async fn validate_creative_provider_models(
        &self,
        document: &CreativeProjectDocument,
    ) -> Result<(), AppError> {
        let mut references = BTreeMap::new();
        for node in &document.nodes {
            match &node.data {
                CreativeNodeData::Config(config) => {
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
                                .or_insert_with(|| format!("config node {}", node.id));
                        }
                        (Some(_), None) | (None, Some(_)) => {
                            return Err(AppError::BadRequest(format!(
                                "creative config node {} providerId and model must be set together",
                                node.id
                            )));
                        }
                    }
                }
                CreativeNodeData::Image(image) => {
                    let Some(model) = image
                        .composer
                        .as_ref()
                        .and_then(|composer| composer.model.as_ref())
                    else {
                        continue;
                    };
                    ProviderId::parse(&model.provider_id).map_err(|error| {
                        AppError::BadRequest(format!(
                            "creative image node {} composer providerId must be a canonical Provider UUIDv7: {error}",
                            node.id
                        ))
                    })?;
                    references
                        .entry((model.provider_id.clone(), model.model.clone()))
                        .or_insert_with(|| format!("image node {} composer", node.id));
                }
                _ => {}
            }
        }
        for ((provider_id, model), owner) in references {
            if !self
                .repo
                .provider_model_exists(&provider_id, &model)
                .await?
            {
                return Err(AppError::Conflict(format!(
                    "creative {owner} references missing provider-model '{provider_id}/{model}'"
                )));
            }
        }
        Ok(())
    }

    // ---- projects ----

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
        let saved = self
            .repo
            .save_creative_project(
                project_id,
                expected_revision,
                &document_json,
                node_count,
                connection_count,
                now_ms(),
            )
            .await
            .map_err(|error| match error {
                DbError::Conflict(message) => AppError::RevisionConflict(message),
                other => other.into(),
            })?;
        Ok(saved.into())
    }

    /// Apply Agent graph operations through the canonical project revision CAS.
    /// The product editor and Agent therefore share one conflict model: neither
    /// can overwrite a newer document snapshot silently.
    pub async fn apply_creative_agent_ops(
        &self,
        project_id: &str,
        expected_revision: &str,
        ops: Vec<CreativeAgentOp>,
        source: &str,
    ) -> Result<CreativeAgentApplyResult, AppError> {
        let expected_revision = parse_creative_project_revision(expected_revision)?;
        let current = self.get_creative_project(project_id).await?;
        if current.project.revision != expected_revision.to_string() {
            return Err(AppError::RevisionConflict(format!(
                "creative studio project {project_id} revision is {}, expected {expected_revision}",
                current.project.revision
            )));
        }
        let (document, results) = crate::creative_agent_ops::apply_creative_agent_ops(
            &current.document,
            ops,
        )
        .map_err(|error| AppError::BadRequest(format!("invalid Creative Studio operations: {error}")))?;
        let project = self
            .save_creative_project(project_id, &expected_revision.to_string(), &document)
            .await?;
        tracing::info!(
            project_id,
            source,
            revision = project.revision,
            ops = results.len(),
            "Creative Studio Agent operations committed"
        );
        Ok(CreativeAgentApplyResult {
            project,
            ops: results,
        })
    }

    pub async fn delete_creative_project(&self, project_id: &str) -> Result<(), AppError> {
        validate_creative_project_id(project_id)?;
        let project = self.get_creative_project(project_id).await?;
        let referenced_asset_ids = collect_document_asset_ids(&project.document)?;
        self.repo.delete_creative_project(project_id).await?;
        for asset_id in referenced_asset_ids {
            let Some(asset) = self.repo.get_asset(&asset_id).await? else {
                continue;
            };
            if asset.in_library {
                continue;
            }
            if let Err(error) = self.delete_asset(&asset_id).await {
                tracing::warn!(
                    project_id,
                    asset_id,
                    %error,
                    "Creative Studio project deleted but an internal asset remained referenced"
                );
            }
        }
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

    // ---- durable Creative Studio workflow runs ----

    pub async fn list_creative_workflow_runs(
        &self,
        workflow_id: Option<&str>,
    ) -> Result<Vec<CreativeWorkflowRunAggregateV1>, AppError> {
        if let Some(workflow_id) = workflow_id {
            validate_creative_workflow_id(workflow_id)?;
        }
        self.repo
            .list_creative_workflow_runs(workflow_id)
            .await?
            .into_iter()
            .map(|row| parse_stored_workflow_run(&row))
            .collect()
    }

    pub async fn get_creative_workflow_run(
        &self,
        workflow_run_id: &str,
    ) -> Result<CreativeWorkflowRunAggregateV1, AppError> {
        validate_creative_workflow_run_id(workflow_run_id)?;
        let row = self
            .repo
            .get_creative_workflow_run(workflow_run_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio workflow run {workflow_run_id} not found"
                ))
            })?;
        parse_stored_workflow_run(&row)
    }

    pub async fn create_creative_workflow_run(
        &self,
        request: CreativeWorkflowRunCreateRequest,
    ) -> Result<CreativeWorkflowRunAggregateV1, AppError> {
        validate_creative_workflow_run_id(&request.run_id)?;
        validate_creative_workflow_id(&request.workflow_id)?;
        if request.workflow_revision < 1 {
            return Err(AppError::BadRequest(
                "workflowRevision must be a positive integer".into(),
            ));
        }

        if let Some(existing) = self
            .repo
            .get_creative_workflow_run(&request.run_id)
            .await?
        {
            let existing = parse_stored_workflow_run(&existing)?;
            if existing.matches_create_request(&request) {
                return Ok(existing);
            }
            return Err(AppError::Conflict(format!(
                "workflow run {} idempotency key is already bound to another request",
                request.run_id
            )));
        }

        let definition_row = self
            .repo
            .get_creative_workflow(&request.workflow_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio workflow {} not found",
                    request.workflow_id
                ))
            })?;
        let definition = parse_workflow_row(&definition_row).map_err(|error| {
            AppError::Internal(format!(
                "stored creative studio workflow {} is corrupt: {error}",
                request.workflow_id
            ))
        })?;
        if definition.revision != request.workflow_revision {
            return Err(AppError::Conflict(format!(
                "creative studio workflow {} revision changed from {} to {}",
                request.workflow_id, request.workflow_revision, definition.revision
            )));
        }

        let now = now_ms();
        let aggregate = CreativeWorkflowRunAggregateV1::requested(
            definition,
            request.run_id.clone(),
            request.inputs.clone(),
            request.reference_asset_ids.clone(),
            now,
        )
        .map_err(|error| AppError::BadRequest(format!("invalid workflow run request: {error}")))?;
        self.validate_creative_workflow_assets(&aggregate.workflow_snapshot)
            .await?;
        self.validate_workflow_run_input_assets(&aggregate).await?;
        let _provider_guard = self.provider_read_guard().await;
        self.validate_creative_workflow_models_under_guard(&aggregate.workflow_snapshot)
            .await?;

        let row = aggregate
            .to_row(now, now)
            .map_err(|error| AppError::BadRequest(format!("invalid workflow run request: {error}")))?;
        let referenced_asset_ids = aggregate
            .referenced_input_asset_ids()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let persisted = self
            .repo
            .create_creative_workflow_run(&row, &referenced_asset_ids)
            .await?;
        let persisted = parse_stored_workflow_run(&persisted)?;
        if !persisted.matches_create_request(&request) {
            return Err(AppError::Conflict(format!(
                "workflow run {} idempotency key is already bound to another request",
                request.run_id
            )));
        }
        Ok(persisted)
    }

    pub async fn save_creative_workflow_run(
        &self,
        workflow_run_id: &str,
        expected_revision: &str,
        replacement: CreativeWorkflowRunAggregateV1,
    ) -> Result<CreativeWorkflowRunAggregateV1, AppError> {
        validate_creative_workflow_run_id(workflow_run_id)?;
        let expected_revision = parse_creative_workflow_revision(expected_revision)?;
        if replacement.request.id != workflow_run_id {
            return Err(AppError::BadRequest(
                "workflow run aggregate id must match its route id".into(),
            ));
        }
        let current_row = self
            .repo
            .get_creative_workflow_run(workflow_run_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "creative studio workflow run {workflow_run_id} not found"
                ))
            })?;
        let current = parse_stored_workflow_run(&current_row)?;
        if current.revision != expected_revision {
            return Err(AppError::Conflict(format!(
                "creative studio workflow run {workflow_run_id} revision conflict"
            )));
        }
        current.validate_transition(&replacement).map_err(|error| {
            AppError::BadRequest(format!("invalid workflow run transition: {error}"))
        })?;
        self.validate_workflow_run_results(&replacement).await?;
        let now = now_ms().max(current.request.requested_at);
        let replacement_row = replacement
            .to_row(current.request.requested_at, now)
            .map_err(|error| AppError::BadRequest(format!("invalid workflow run: {error}")))?;
        let saved = self
            .repo
            .save_creative_workflow_run(workflow_run_id, expected_revision, &replacement_row)
            .await?;
        parse_stored_workflow_run(&saved)
    }

    async fn validate_workflow_run_input_assets(
        &self,
        aggregate: &CreativeWorkflowRunAggregateV1,
    ) -> Result<(), AppError> {
        for asset_id in aggregate.referenced_input_asset_ids() {
            let asset = self.repo.get_asset(asset_id).await?.ok_or_else(|| {
                AppError::Conflict(format!("workflow run references missing asset {asset_id}"))
            })?;
            if asset.kind != "image" {
                return Err(AppError::Conflict(format!(
                    "workflow run reference {asset_id} must identify an image asset"
                )));
            }
        }
        Ok(())
    }

    async fn validate_workflow_run_results(
        &self,
        aggregate: &CreativeWorkflowRunAggregateV1,
    ) -> Result<(), AppError> {
        let executable_steps = aggregate
            .executable_task_step_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let task_ids = aggregate
            .record
            .task_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        for asset_id in &aggregate.record.result_asset_ids {
            let asset = self.repo.get_asset(asset_id).await?.ok_or_else(|| {
                AppError::Conflict(format!(
                    "workflow run result references missing asset {asset_id}"
                ))
            })?;
            if asset.kind != "image" {
                return Err(AppError::Conflict(format!(
                    "workflow run result {asset_id} must identify an image asset"
                )));
            }
            validate_workflow_result_origin(&asset, aggregate, &executable_steps, &task_ids)?;
        }
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

    /// Read-only startup audit for canonical Creative Studio projects and the
    /// shared asset store. It validates every managed reference without
    /// rewriting or deleting user content.
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
    /// provenance blob (`{prompt,model,provider_id,params,project_id,…}`).
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
    /// instead of overwriting a newer revision. The overall scan is idempotent.
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
                match &mut node.data {
                    CreativeNodeData::Config(config)
                        if config.provider_id.as_deref() == Some(provider_id.as_str()) =>
                    {
                        config.provider_id = None;
                        config.model = None;
                        changed = true;
                    }
                    CreativeNodeData::Image(image) => {
                        let clears_target = image
                            .composer
                            .as_ref()
                            .and_then(|composer| composer.model.as_ref())
                            .is_some_and(|model| model.provider_id == provider_id);
                        if clears_target {
                            if let Some(composer) = image.composer.as_mut() {
                                composer.model = None;
                            }
                            changed = true;
                        }
                    }
                    _ => {}
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
            origin: serialize_text_asset_origin(input.origin)?,
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

    /// Resolve one canonical asset record for authenticated product clients.
    pub async fn get_asset(&self, id: &str) -> Result<WorkshopAsset, AppError> {
        let row = self
            .repo
            .get_asset(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {id} not found")))?;
        WorkshopAsset::try_from(row)
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
        for project_row in self.repo.list_creative_projects().await? {
            let document = parse_stored_creative_project_row(&project_row)?;
            if collect_document_asset_ids(&document)?.contains(id) {
                return Err(AppError::Conflict(format!(
                    "asset {id} is referenced by Creative Studio project {}",
                    project_row.project_id
                )));
            }
        }
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
        for run_row in self.repo.list_creative_workflow_runs(None).await? {
            let run = parse_stored_workflow_run(&run_row)?;
            if run.referenced_input_asset_ids().contains(&id)
                || run.record.result_asset_ids.iter().any(|asset_id| asset_id == id)
            {
                return Err(AppError::Conflict(format!(
                    "asset {id} is referenced by workflow run {}",
                    run.request.id
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

fn serialize_text_asset_origin(
    origin: Option<PromptCatalogAssetOrigin>,
) -> Result<Option<String>, AppError> {
    let Some(origin) = origin else {
        return Ok(None);
    };
    let required = |key: &str, value: &str, max: usize| -> Result<(), AppError> {
        let value = Some(value)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::BadRequest(format!("text asset origin.{key} is required")))?;
        if value.len() > max || value.chars().any(char::is_control) {
            return Err(AppError::BadRequest(format!(
                "text asset origin.{key} is invalid"
            )));
        }
        Ok(())
    };
    required("prompt_catalog_id", &origin.prompt_catalog_id, 255)?;
    required("source_url", &origin.source_url, 4_096)?;
    required("license", &origin.license, 120)?;
    required("license_url", &origin.license_url, 4_096)?;
    let valid_https_url = |value: &str| {
        reqwest::Url::parse(value)
            .is_ok_and(|url| url.scheme() == "https" && url.host().is_some())
    };
    if !valid_https_url(&origin.source_url) || !valid_https_url(&origin.license_url) {
        return Err(AppError::BadRequest(
            "text asset origin URLs must use HTTPS".into(),
        ));
    }
    serde_json::to_string(&origin)
        .map(Some)
        .map_err(|error| AppError::BadRequest(format!("serialize text asset origin: {error}")))
}

fn validate_creative_workflow_id(workflow_id: &str) -> Result<(), AppError> {
    CreativeStudioWorkflowId::parse(workflow_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid workflow id: {error}")))
}

fn validate_creative_workflow_run_id(workflow_run_id: &str) -> Result<(), AppError> {
    CreativeStudioWorkflowRunId::parse(workflow_run_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid workflow run id: {error}")))
}

fn parse_stored_workflow_run(
    row: &CreativeStudioWorkflowRunRow,
) -> Result<CreativeWorkflowRunAggregateV1, AppError> {
    parse_workflow_run_row(row).map_err(|error| {
        AppError::Internal(format!(
            "stored creative studio workflow run {} is corrupt: {error}",
            row.workflow_run_id
        ))
    })
}

fn validate_workflow_result_origin(
    asset: &WorkshopAssetRow,
    aggregate: &CreativeWorkflowRunAggregateV1,
    executable_steps: &BTreeSet<String>,
    task_ids: &BTreeSet<String>,
) -> Result<(), AppError> {
    let origin = asset.origin.as_deref().ok_or_else(|| {
        AppError::Conflict(format!(
            "workflow run result {} has no durable provenance",
            asset.asset_id
        ))
    })?;
    let origin = serde_json::from_str::<Value>(origin).map_err(|error| {
        AppError::Conflict(format!(
            "workflow run result {} has invalid provenance: {error}",
            asset.asset_id
        ))
    })?;
    let origin = origin.as_object().ok_or_else(|| {
        AppError::Conflict(format!(
            "workflow run result {} provenance must be an object",
            asset.asset_id
        ))
    })?;
    let string = |key: &str| origin.get(key).and_then(Value::as_str);
    let workflow_step_id = string("workflow_step_id");
    let creation_task_id = string("creation_task_id");
    if string("workflow_id") != Some(aggregate.request.workflow_id.as_str())
        || string("workflow_run_id") != Some(aggregate.request.id.as_str())
        || workflow_step_id.is_none_or(|id| !executable_steps.contains(id))
        || creation_task_id.is_none_or(|id| !task_ids.contains(id))
        || origin.contains_key("project_id")
        || origin.contains_key("canvas_id")
        || origin.contains_key("node_id")
    {
        return Err(AppError::Conflict(format!(
            "workflow run result {} provenance does not match run {}",
            asset.asset_id, aggregate.request.id
        )));
    }
    Ok(())
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
    use crate::workflow_run::{
        CreativeWorkflowInputValue, CreativeWorkflowRunCreateRequest,
        CreativeWorkflowRunStatus,
    };
    use nomifun_common::{CreativeStudioNodeId, ProviderLifecycleBarrier};
    use nomifun_db::{IProviderRepository, SqliteProviderRepository, SqliteWorkshopRepository};

    async fn service() -> (Arc<WorkshopService>, tempfile::TempDir) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn IWorkshopRepository> = Arc::new(SqliteWorkshopRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        let dir = tempfile::tempdir().unwrap();
        (WorkshopService::start(dir.path(), repo), dir)
    }

    async fn service_with_database_and_lifecycle(
        provider_lifecycle: Option<SharedProviderLifecycleBarrier>,
    ) -> (Arc<WorkshopService>, tempfile::TempDir, Arc<nomifun_db::Database>) {
        let db = Arc::new(nomifun_db::init_database_memory().await.unwrap());
        let repo: Arc<dyn IWorkshopRepository> =
            Arc::new(SqliteWorkshopRepository::new(db.pool().clone()));
        let dir = tempfile::tempdir().unwrap();
        let service = WorkshopService::start_with_optional_provider_lifecycle(
            dir.path(),
            repo,
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

    #[tokio::test]
    async fn creative_workflow_run_is_idempotent_cas_backed_and_checks_result_provenance() {
        let (service, _dir, database) = service_with_database_and_lifecycle(None).await;
        let provider_id = ProviderId::new().into_string();
        insert_provider(&database, &provider_id).await;
        insert_provider_model(&database, &provider_id, "image-model").await;
        insert_provider_model_capability(
            &database,
            &provider_id,
            "image-model",
            "image_generation",
        )
        .await;

        let mut definition = workflow_definition();
        let CreativeWorkflowVariable::Text { id: variable_id, .. } = &definition.variables[0]
        else {
            panic!("workflow fixture must start with a text variable")
        };
        let variable_id = variable_id.clone();
        if let CreativeWorkflowStep::GenerateImages { generation, .. } = &mut definition.steps[1]
        {
            generation.model = Some(crate::workflow::CreativeWorkflowImageModelBinding {
                provider_id: provider_id.clone(),
                model: "image-model".into(),
                task: crate::workflow::CreativeWorkflowImageTask::ImageGeneration,
            });
        }
        let definition = service
            .create_creative_workflow(definition)
            .await
            .unwrap();
        let reference_asset = service
            .ingest_asset_bytes(
                png_1x1(),
                "image/png",
                "reference",
                false,
                None,
            )
            .await
            .unwrap();
        let run_id = CreativeStudioWorkflowRunId::new().into_string();
        let request = CreativeWorkflowRunCreateRequest {
            run_id: run_id.clone(),
            workflow_id: definition.id.clone(),
            workflow_revision: definition.revision,
            inputs: vec![CreativeWorkflowInputValue::Text {
                variable_id,
                value: "NomiFun".into(),
            }],
            reference_asset_ids: vec![reference_asset.asset_id.clone()],
        };
        let created = service
            .create_creative_workflow_run(request.clone())
            .await
            .unwrap();
        assert_eq!(
            service
                .create_creative_workflow_run(request.clone())
                .await
                .unwrap(),
            created
        );
        let mut mismatched = request;
        if let CreativeWorkflowInputValue::Text { value, .. } = &mut mismatched.inputs[0] {
            *value = "another request".into();
        }
        assert!(matches!(
            service.create_creative_workflow_run(mismatched).await,
            Err(AppError::Conflict(_))
        ));

        let task_id = nomifun_common::generate_id();
        let step_id = created.executable_task_step_ids()[0].clone();
        let mut queued = created.clone();
        queued.revision = 2;
        queued.record.status = CreativeWorkflowRunStatus::Queued;
        queued.record.task_ids = vec![task_id.clone()];
        queued.record.queued_at = Some(created.request.requested_at + 1);
        let queued = service
            .save_creative_workflow_run(&run_id, "1", queued)
            .await
            .unwrap();
        assert!(matches!(
            service
                .save_creative_workflow_run(&run_id, "1", queued.clone())
                .await,
            Err(AppError::Conflict(_))
        ));

        let mut running = queued.clone();
        running.revision = 3;
        running.record.status = CreativeWorkflowRunStatus::Running;
        running.record.started_at = Some(created.request.requested_at + 2);
        let running = service
            .save_creative_workflow_run(&run_id, "2", running)
            .await
            .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, workflow_id, workflow_run_id, workflow_step_id, \
                 provider_id, model, capability, params, status, error, result_asset_ids, \
                 remote_task_id, attempt, submitted_at, started_at, finished_at, request_fingerprint) \
             VALUES (?, ?, ?, ?, ?, 'image-model', 'image_generation', '{}', 'running', \
                     NULL, '[]', NULL, 1, ?, ?, NULL, '{}')",
        )
        .bind(&task_id)
        .bind(&definition.id)
        .bind(&run_id)
        .bind(&step_id)
        .bind(&provider_id)
        .bind(created.request.requested_at + 1)
        .bind(created.request.requested_at + 2)
        .execute(database.pool())
        .await
        .unwrap();
        let asset = service
            .ingest_asset_bytes(
                png_1x1(),
                "image/png",
                "result",
                false,
                Some(serde_json::json!({
                    "workflow_id": definition.id,
                    "workflow_run_id": run_id,
                    "workflow_step_id": step_id,
                    "creation_task_id": task_id
                })),
            )
            .await
            .unwrap();
        let mut succeeded = running;
        succeeded.revision = 4;
        succeeded.record.status = CreativeWorkflowRunStatus::Succeeded;
        succeeded.record.result_asset_ids = vec![asset.asset_id];
        succeeded.record.completed_at = Some(created.request.requested_at + 3);
        let succeeded = service
            .save_creative_workflow_run(&run_id, "3", succeeded)
            .await
            .unwrap();
        assert_eq!(succeeded.record.status, CreativeWorkflowRunStatus::Succeeded);
        assert_eq!(
            service
                .list_creative_workflow_runs(Some(&succeeded.request.workflow_id))
                .await
                .unwrap(),
            vec![succeeded]
        );
        assert!(matches!(
            service.delete_asset(&reference_asset.asset_id).await,
            Err(AppError::Conflict(message)) if message.contains("workflow run")
        ));
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

    fn creative_image_node(
        id: &str,
        provider_id: Option<&str>,
        model: Option<&str>,
    ) -> crate::creative_studio::CreativeNode {
        let composer_model = match (provider_id, model) {
            (Some(provider_id), Some(model)) => serde_json::json!({
                "providerId": provider_id,
                "model": model
            }),
            (None, None) => Value::Null,
            _ => panic!("image composer model identity must be complete"),
        };
        serde_json::from_value(serde_json::json!({
            "id": id,
            "type": "image",
            "position": { "x": 0, "y": 0 },
            "size": { "width": 320, "height": 240 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": {
                "assetId": null,
                "caption": "",
                "alt": "",
                "fit": "contain",
                "naturalSize": null,
                "composer": {
                    "prompt": "draft",
                    "model": composer_model,
                    "interfaceMode": "images",
                    "quality": "auto",
                    "width": 1024,
                    "height": 1024,
                    "aspectRatio": "1:1",
                    "count": 1
                }
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
    async fn creative_project_crud_uses_revision_cas() {
        let (svc, _dir) = service().await;
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
        assert!(matches!(stale, AppError::RevisionConflict(_)));

        svc.delete_creative_project(&created.project_id)
            .await
            .unwrap();
        assert!(matches!(
            svc.get_creative_project(&created.project_id).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn creative_agent_ops_share_project_revision_cas() {
        let (svc, _dir) = service().await;
        let created = svc.create_creative_project(Some("Agent CAS".into())).await.unwrap();
        let applied = svc
            .apply_creative_agent_ops(
                &created.project_id,
                "1",
                vec![crate::creative_agent_ops::CreativeAgentOp::AddNode {
                    node_type: crate::creative_studio::CreativeNodeType::Text,
                    x: 10.0,
                    y: 20.0,
                    width: None,
                    height: None,
                    group_id: None,
                    data: serde_json::json!({
                        "text": "Agent-created",
                        "format": "plain",
                        "fontSize": 16,
                        "textAlign": "left"
                    }),
                }],
                "conversation:test",
            )
            .await
            .unwrap();
        assert_eq!(applied.project.revision, "2");
        assert_eq!(applied.project.node_count, 1);
        assert_eq!(applied.ops.len(), 1);

        let stale = svc
            .apply_creative_agent_ops(
                &created.project_id,
                "1",
                vec![crate::creative_agent_ops::CreativeAgentOp::DeleteNode {
                    node_id: match &applied.ops[0] {
                        crate::creative_agent_ops::CreativeAgentOpResult::NodeAdded { node_id } => {
                            node_id.clone()
                        }
                        other => panic!("unexpected result {other:?}"),
                    },
                }],
                "conversation:test",
            )
            .await
            .unwrap_err();
        assert!(matches!(stale, AppError::RevisionConflict(_)));
        let current = svc.get_creative_project(&created.project_id).await.unwrap();
        assert_eq!(current.project.revision, "2");
        assert_eq!(current.document.nodes.len(), 1);
    }

    #[tokio::test]
    async fn project_asset_references_block_deletion_until_project_is_removed() {
        let (svc, _dir) = service().await;
        let asset = svc
            .upload_asset(NewAssetUpload {
                file_name: "reference.png".into(),
                content_type: Some("image/png".into()),
                bytes: png_1x1(),
                title: None,
                collection: None,
                tags: None,
                in_library: Some(false),
            })
            .await
            .unwrap();
        let project = svc
            .create_creative_project(Some("asset owner".into()))
            .await
            .unwrap();
        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id": CreativeStudioNodeId::new().into_string(),
                "type": "image",
                "position": { "x": 0, "y": 0 },
                "size": { "width": 320, "height": 240 },
                "groupId": null,
                "zIndex": 0,
                "locked": false,
                "data": {
                    "assetId": asset.asset_id,
                    "caption": "",
                    "alt": "",
                    "fit": "cover",
                    "naturalSize": { "width": 1, "height": 1 }
                }
            }))
            .unwrap(),
        );
        svc.save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        assert!(matches!(
            svc.delete_asset(&asset.asset_id).await,
            Err(AppError::Conflict(message)) if message.contains("Creative Studio project")
        ));
        svc.delete_creative_project(&project.project_id)
            .await
            .unwrap();
        assert!(matches!(
            svc.serve_file(&asset.asset_id, false).await,
            Err(AppError::NotFound(_))
        ));
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
                origin: None,
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
                .filter(|result| matches!(result, Err(AppError::RevisionConflict(_))))
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
    async fn canonical_save_validates_image_composer_provider_model_pair() {
        let barrier = Arc::new(ProviderLifecycleBarrier::new());
        let (svc, _dir, db) = service_with_database_and_lifecycle(Some(barrier)).await;
        let project = svc.create_creative_project(None).await.unwrap();
        let provider_id = "0190f5fe-7c00-7a00-8000-000000000083";
        insert_provider(&db, provider_id).await;

        let mut document = CreativeProjectDocument::empty(project.project_id.clone());
        document.nodes.push(creative_image_node(
            "image-composer",
            Some(provider_id),
            Some("missing-model"),
        ));
        let missing_model = svc
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap_err();
        assert!(matches!(
            missing_model,
            AppError::Conflict(ref message)
                if message.contains("image node image-composer composer")
                    && message.contains("missing-model")
        ));

        insert_provider_model(&db, provider_id, "image-model").await;
        document.nodes[0] = creative_image_node(
            "image-composer",
            Some(provider_id),
            Some("image-model"),
        );
        let saved = svc
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();
        assert_eq!(saved.revision, "2");
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
        document.nodes.push(creative_image_node(
            "target-image",
            Some(target_provider_id),
            Some("delete-me"),
        ));
        document.nodes.push(creative_image_node(
            "surviving-image",
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
        let CreativeNodeData::Image(target_image) = &cleaned.document.nodes[2].data else {
            panic!("expected target image node")
        };
        let target_composer = target_image.composer.as_ref().unwrap();
        assert_eq!(target_composer.model, None);
        assert_eq!(target_composer.prompt, "draft");
        assert_eq!(target_composer.aspect_ratio, "1:1");
        assert_eq!(target_composer.count, 1);
        let CreativeNodeData::Image(surviving_image) = &cleaned.document.nodes[3].data else {
            panic!("expected surviving image node")
        };
        let surviving_image_model = surviving_image
            .composer
            .as_ref()
            .and_then(|composer| composer.model.as_ref())
            .expect("unrelated image composer model must survive provider cleanup");
        assert_eq!(surviving_image_model.provider_id, other_provider_id);
        assert_eq!(surviving_image_model.model, "keep-me");

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
                origin: None,
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

    #[tokio::test]
    async fn text_asset_prompt_catalog_origin_is_bounded_and_roundtrips() {
        let (svc, _dir) = service().await;
        let asset = svc
            .create_text_asset(NewTextAsset {
                title: "有来源的提示词".into(),
                text_content: "Create a paper poster".into(),
                collection: Some("提示词".into()),
                tags: Some(vec!["poster".into()]),
                in_library: Some(true),
                origin: Some(PromptCatalogAssetOrigin {
                    prompt_catalog_id: "awesome-gpt-image-001".into(),
                    source_url: "https://github.com/ZeroLu/awesome-gpt-image".into(),
                    license: "MIT".into(),
                    license_url:
                        "https://github.com/ZeroLu/awesome-gpt-image/blob/main/LICENSE".into(),
                }),
            })
            .await
            .unwrap();
        assert_eq!(
            asset.origin.as_ref().unwrap()["prompt_catalog_id"],
            "awesome-gpt-image-001"
        );

        let error = svc
            .create_text_asset(NewTextAsset {
                title: "不安全来源".into(),
                text_content: "prompt".into(),
                collection: None,
                tags: None,
                in_library: Some(true),
                origin: Some(PromptCatalogAssetOrigin {
                    prompt_catalog_id: "prompt-1".into(),
                    source_url: "http://example.test/source".into(),
                    license: "MIT".into(),
                    license_url: "https://example.test/license".into(),
                }),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)));
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
    async fn list_assets_ungrouped_filters_serverside() {
        let (svc, _dir) = service().await;
        // Two ungrouped text assets (no collection) + one in a named collection.
        svc.create_text_asset(NewTextAsset {
            title: "散图".into(),
            text_content: "x".into(),
            collection: None,
            tags: None,
            in_library: Some(true),
            origin: None,
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
            origin: None,
        })
        .await
        .unwrap();
        svc.create_text_asset(NewTextAsset {
            title: "角色图".into(),
            text_content: "z".into(),
            collection: Some("角色".into()),
            tags: None,
            in_library: Some(true),
            origin: None,
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

}
