use crate::error::DbError;
use crate::models::{
    CreativeStudioProjectRow, CreativeStudioWorkflowRow, CreativeStudioWorkflowRunRow,
    WorkshopAssetRow,
};

/// Data access for canonical Creative Studio projects, workflows, runs, and
/// the shared asset library.
///
/// Project bodies live atomically in `creative_studio_projects`. Asset binaries
/// remain service-owned files while this repository stores their indexed
/// `workshop_assets` metadata.
#[async_trait::async_trait]
pub trait IWorkshopRepository: Send + Sync {
    /// Check that a Provider business ID exists. Workshop JSON references are
    /// logical links; callers must perform this check before persisting them.
    async fn provider_exists(&self, provider_id: &str) -> Result<bool, DbError>;

    /// Check one exact Provider/model logical parent. Creative Studio config
    /// nodes bind the pair, not merely the Provider, so accepting a Provider
    /// with an unknown model would persist an invocation target that can never
    /// be resolved by the managed model catalog.
    async fn provider_model_exists(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<bool, DbError>;

    /// Check one enabled Provider/model/task capability. Workflow definitions
    /// persist exact NomiFun task bindings rather than inferring modality from
    /// a model name.
    async fn provider_model_supports_task(
        &self,
        provider_id: &str,
        model: &str,
        task: &str,
    ) -> Result<bool, DbError> {
        let _ = (provider_id, model, task);
        Err(DbError::Init(
            "task-scoped provider model validation is unavailable in this repository".into(),
        ))
    }

    // ---- canonical Creative Studio projects ----

    /// Every canonical Creative Studio project, newest-updated first.
    async fn list_creative_projects(&self) -> Result<Vec<CreativeStudioProjectRow>, DbError>;

    /// One canonical Creative Studio project by business ID, or `None`.
    async fn get_creative_project(
        &self,
        project_id: &str,
    ) -> Result<Option<CreativeStudioProjectRow>, DbError>;

    /// Insert the initial revision and canonical v1 document atomically.
    async fn create_creative_project(
        &self,
        project_id: &str,
        title: &str,
        document_json: &str,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError>;

    /// Rename project metadata. The document revision is deliberately kept so
    /// an in-flight autosave does not conflict with a title-only edit.
    async fn rename_creative_project(
        &self,
        project_id: &str,
        title: &str,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError>;

    /// Compare-and-swap the canonical document. A stale expected revision is a
    /// conflict; a successful write increments the revision exactly once.
    async fn save_creative_project(
        &self,
        project_id: &str,
        expected_revision: i64,
        document_json: &str,
        node_count: i64,
        connection_count: i64,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError>;

    /// Insert a freshly remapped canonical project and all of its imported
    /// assets in one SQLite transaction. Callers stage binary files before
    /// entering this method and remove them if the transaction fails; the DB
    /// therefore never exposes a project without every referenced asset row.
    async fn import_creative_project_with_assets(
        &self,
        project: &CreativeStudioProjectRow,
        assets: &[WorkshopAssetRow],
    ) -> Result<CreativeStudioProjectRow, DbError>;

    /// Hard-delete one canonical project row. Managed assets are not deleted:
    /// they have their own library lifecycle and may be shared by projects.
    async fn delete_creative_project(&self, project_id: &str) -> Result<(), DbError>;

    // ---- canonical Creative Studio workflows ----

    /// Every workflow definition, newest-updated first.
    async fn list_creative_workflows(&self) -> Result<Vec<CreativeStudioWorkflowRow>, DbError> {
        Err(DbError::Init(
            "creative studio workflow persistence is unavailable in this repository".into(),
        ))
    }

    /// One workflow definition by business ID, or `None`.
    async fn get_creative_workflow(
        &self,
        workflow_id: &str,
    ) -> Result<Option<CreativeStudioWorkflowRow>, DbError> {
        let _ = workflow_id;
        Err(DbError::Init(
            "creative studio workflow persistence is unavailable in this repository".into(),
        ))
    }

    /// Insert revision one of a closed workflow definition.
    async fn create_creative_workflow(
        &self,
        row: &CreativeStudioWorkflowRow,
    ) -> Result<CreativeStudioWorkflowRow, DbError> {
        let _ = row;
        Err(DbError::Init(
            "creative studio workflow persistence is unavailable in this repository".into(),
        ))
    }

    /// Compare-and-swap the definition. The replacement row must carry
    /// `expected_revision + 1`.
    async fn save_creative_workflow(
        &self,
        workflow_id: &str,
        expected_revision: i64,
        row: &CreativeStudioWorkflowRow,
    ) -> Result<CreativeStudioWorkflowRow, DbError> {
        let _ = (workflow_id, expected_revision, row);
        Err(DbError::Init(
            "creative studio workflow persistence is unavailable in this repository".into(),
        ))
    }

    /// Hard-delete one workflow definition.
    async fn delete_creative_workflow(&self, workflow_id: &str) -> Result<(), DbError> {
        let _ = workflow_id;
        Err(DbError::Init(
            "creative studio workflow persistence is unavailable in this repository".into(),
        ))
    }

    // ---- canonical Creative Studio workflow runs ----

    /// Durable runs, newest-updated first. When `workflow_id` is present the
    /// result is restricted to that exact pinned definition family.
    async fn list_creative_workflow_runs(
        &self,
        workflow_id: Option<&str>,
    ) -> Result<Vec<CreativeStudioWorkflowRunRow>, DbError> {
        let _ = workflow_id;
        Err(DbError::Init(
            "creative studio workflow run persistence is unavailable in this repository".into(),
        ))
    }

    /// One durable workflow run by business ID, or `None`.
    async fn get_creative_workflow_run(
        &self,
        workflow_run_id: &str,
    ) -> Result<Option<CreativeStudioWorkflowRunRow>, DbError> {
        let _ = workflow_run_id;
        Err(DbError::Init(
            "creative studio workflow run persistence is unavailable in this repository".into(),
        ))
    }

    /// Insert revision one of a closed workflow-run aggregate.
    async fn create_creative_workflow_run(
        &self,
        row: &CreativeStudioWorkflowRunRow,
        referenced_asset_ids: &[String],
    ) -> Result<CreativeStudioWorkflowRunRow, DbError> {
        let _ = (row, referenced_asset_ids);
        Err(DbError::Init(
            "creative studio workflow run persistence is unavailable in this repository".into(),
        ))
    }

    /// Compare-and-swap a workflow-run aggregate. The replacement must carry
    /// the same identity and `expected_revision + 1`.
    async fn save_creative_workflow_run(
        &self,
        workflow_run_id: &str,
        expected_revision: i64,
        row: &CreativeStudioWorkflowRunRow,
    ) -> Result<CreativeStudioWorkflowRunRow, DbError> {
        let _ = (workflow_run_id, expected_revision, row);
        Err(DbError::Init(
            "creative studio workflow run persistence is unavailable in this repository".into(),
        ))
    }

    // ---- assets ----

    /// Insert a fully-formed asset row.
    async fn create_asset(&self, row: &WorkshopAssetRow) -> Result<WorkshopAssetRow, DbError>;

    /// One asset by id, or `None`.
    async fn get_asset(&self, id: &str) -> Result<Option<WorkshopAssetRow>, DbError>;

    /// Every asset row (no pagination) — for GC (orphan detection + on-disk
    /// file reconciliation). The asset table is small enough to scan whole.
    async fn list_all_assets(&self) -> Result<Vec<WorkshopAssetRow>, DbError>;

    /// Filtered + paginated listing. Returns `(page_items, total_matching)`.
    async fn list_assets(&self, params: ListAssetsParams<'_>) -> Result<(Vec<WorkshopAssetRow>, i64), DbError>;

    /// Partial update (title/collection/tags/in_library). `DbError::NotFound`
    /// when the id is unknown.
    async fn update_asset(&self, id: &str, params: UpdateAssetParams<'_>, now: i64) -> Result<WorkshopAssetRow, DbError>;

    /// Set (or replace) an asset's thumbnail `rel_path` — used by lazy thumbnail
    /// generation on the serve path. `DbError::NotFound` when the id is unknown.
    async fn set_asset_thumb(&self, id: &str, thumb_rel_path: &str, now: i64) -> Result<(), DbError>;

    /// Delete an asset row (the service removes the file). `DbError::NotFound`
    /// when the id is unknown.
    async fn delete_asset(&self, id: &str) -> Result<(), DbError>;

    /// Bulk-rename a collection: every asset whose `collection` equals `from`
    /// gets `to` (or `NULL` when `to` is `None`, i.e. ungrouped). Returns the
    /// number of rows updated (0 when no asset used `from`). Append-only
    /// management operation.
    async fn rename_collection(&self, from: &str, to: Option<&str>, now: i64) -> Result<u64, DbError>;
}

/// Result ordering for [`IWorkshopRepository::list_assets`]. Append-only (the
/// asset-library management page): existing callers keep the `Default`
/// (newest-created first) via `..Default::default()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AssetSort {
    /// Newest created first (the default, matching the original hard-coded order).
    #[default]
    CreatedDesc,
    /// Oldest created first.
    CreatedAsc,
    /// Most recently updated first.
    UpdatedDesc,
    /// Title A→Z (case-insensitive).
    TitleAsc,
    /// Largest byte size first (text assets carry no `bytes` and sort last).
    SizeDesc,
}

/// Filters + pagination for [`IWorkshopRepository::list_assets`]. All filters
/// are optional; `None` means "no filter on this field".
#[derive(Debug, Default)]
pub struct ListAssetsParams<'a> {
    pub kind: Option<&'a str>,
    pub collection: Option<&'a str>,
    /// Case-insensitive substring over title.
    pub q: Option<&'a str>,
    pub in_library: Option<bool>,
    /// Append-only (M10a): when `true`, restrict to assets with no collection
    /// (`collection IS NULL OR collection = ''`). Callers keep this mutually
    /// exclusive with `collection`; if both are set here the two clauses AND
    /// together (never matching), so the caller is responsible for the split.
    pub ungrouped: bool,
    /// Append-only (asset-library page): exact-match filter on one entry of the
    /// JSON `tags` array. `None` means "no tag filter".
    pub tag: Option<&'a str>,
    /// Append-only (asset-library page): result ordering. Defaults to
    /// [`AssetSort::CreatedDesc`].
    pub sort: AssetSort,
    /// 1-based page (clamped to `>= 1` by the caller).
    pub page: i64,
    /// Rows per page (clamped by the caller).
    pub page_size: i64,
}

/// Partial-update params for [`IWorkshopRepository::update_asset`]. Each `Some`
/// replaces the field; `None` keeps the current value. Inner `Option` (for
/// nullable columns) distinguishes "set to NULL" from "keep".
#[derive(Debug, Default)]
pub struct UpdateAssetParams<'a> {
    pub title: Option<&'a str>,
    pub collection: Option<Option<&'a str>>,
    /// Replacement JSON array string of tags.
    pub tags: Option<&'a str>,
    pub in_library: Option<bool>,
}
