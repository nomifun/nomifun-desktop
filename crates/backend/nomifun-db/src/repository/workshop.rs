use crate::error::DbError;
use crate::models::{
    CreativeStudioAgentProposalReceiptRow, CreativeStudioProjectRow, CreativeStudioTemplateRow,
    CreativeStudioTemplateRunRow, WorkshopAssetRow,
};

/// Canonical candidate passed to the atomic proposal receipt + project CAS.
/// `expected_revision` is deliberately not part of the persisted payload
/// identity: a response-loss replay remains valid after the project advances.
#[derive(Debug)]
pub struct ApplyCreativeAgentProposalParams<'a> {
    pub owner_id: &'a str,
    pub project_id: &'a str,
    pub assistant_message_id: &'a str,
    /// Exact raw `messages.content` JSON read before artifact parsing. The
    /// atomic proof rechecks byte equality to fence concurrent message edits.
    pub assistant_message_content_json: &'a str,
    pub ops_fingerprint: &'a str,
    pub ops_json: &'a str,
    pub results_json: &'a str,
    pub expected_revision: i64,
    pub document_json: &'a str,
    pub node_count: i64,
    pub connection_count: i64,
    pub now: i64,
}

/// Result of the atomic repository operation. On replay, `project` is the
/// current authoritative row while `receipt.applied_revision` remains the
/// revision created by the first execution.
#[derive(Debug)]
pub struct CreativeAgentProposalCommit {
    pub project: CreativeStudioProjectRow,
    pub receipt: CreativeStudioAgentProposalReceiptRow,
    pub replayed: bool,
}

/// Stable, namespaced identity of a prompt-library item materialized as a text
/// asset. The database owns the uniqueness guarantee; callers must not derive
/// this identity from mutable title/body text.
#[derive(Debug, Clone, Copy)]
pub struct PromptLibraryAssetIdentity<'a> {
    pub source: &'a str,
    pub prompt_library_id: &'a str,
}

/// Data access for canonical Creative Studio projects, templates, runs, and
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

    /// Check one enabled Provider/model/task capability. Template definitions
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

    /// Read an existing durable Canvas Agent proposal receipt.
    async fn get_creative_agent_proposal_receipt(
        &self,
        owner_id: &str,
        project_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<CreativeStudioAgentProposalReceiptRow>, DbError> {
        let _ = (owner_id, project_id, assistant_message_id);
        Err(DbError::Init(
            "creative studio Agent proposal receipts are unavailable in this repository".into(),
        ))
    }

    /// Read the exact persisted `messages.content` JSON for a completed,
    /// visible assistant message in the owner-bound project chat session.
    async fn get_creative_agent_proposal_message_content(
        &self,
        owner_id: &str,
        project_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<String>, DbError> {
        let _ = (owner_id, project_id, assistant_message_id);
        Err(DbError::Init(
            "creative studio Agent proposal provenance is unavailable in this repository".into(),
        ))
    }

    /// Whether this canonical user is the installation owner authorized to
    /// operate the private Creative Studio surface.
    async fn is_creative_studio_owner(&self, owner_id: &str) -> Result<bool, DbError> {
        let _ = owner_id;
        Err(DbError::Init(
            "creative studio owner validation is unavailable in this repository".into(),
        ))
    }

    /// Atomically claim one assistant proposal, compare-and-swap the project,
    /// and publish its durable result receipt. A concurrent identical claim
    /// replays the winner; reusing the assistant ID for different canonical
    /// operations is a conflict.
    async fn apply_creative_agent_proposal(
        &self,
        params: ApplyCreativeAgentProposalParams<'_>,
    ) -> Result<CreativeAgentProposalCommit, DbError> {
        let _ = params;
        Err(DbError::Init(
            "creative studio Agent proposal receipts are unavailable in this repository".into(),
        ))
    }

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

    // ---- canonical Creative Studio templates ----

    /// Every template definition, newest-updated first.
    async fn list_creative_templates(&self) -> Result<Vec<CreativeStudioTemplateRow>, DbError> {
        Err(DbError::Init(
            "creative studio template persistence is unavailable in this repository".into(),
        ))
    }

    /// One template definition by business ID, or `None`.
    async fn get_creative_template(
        &self,
        template_id: &str,
    ) -> Result<Option<CreativeStudioTemplateRow>, DbError> {
        let _ = template_id;
        Err(DbError::Init(
            "creative studio template persistence is unavailable in this repository".into(),
        ))
    }

    /// Insert revision one of a closed template definition.
    async fn create_creative_template(
        &self,
        row: &CreativeStudioTemplateRow,
    ) -> Result<CreativeStudioTemplateRow, DbError> {
        let _ = row;
        Err(DbError::Init(
            "creative studio template persistence is unavailable in this repository".into(),
        ))
    }

    /// Compare-and-swap the definition. The replacement row must carry
    /// `expected_revision + 1`.
    async fn save_creative_template(
        &self,
        template_id: &str,
        expected_revision: i64,
        row: &CreativeStudioTemplateRow,
    ) -> Result<CreativeStudioTemplateRow, DbError> {
        let _ = (template_id, expected_revision, row);
        Err(DbError::Init(
            "creative studio template persistence is unavailable in this repository".into(),
        ))
    }

    /// Hard-delete one template definition.
    async fn delete_creative_template(&self, template_id: &str) -> Result<(), DbError> {
        let _ = template_id;
        Err(DbError::Init(
            "creative studio template persistence is unavailable in this repository".into(),
        ))
    }

    // ---- canonical Creative Studio template runs ----

    /// Durable runs, newest-updated first. When `template_id` is present the
    /// result is restricted to that exact pinned definition family.
    async fn list_creative_template_runs(
        &self,
        template_id: Option<&str>,
    ) -> Result<Vec<CreativeStudioTemplateRunRow>, DbError> {
        let _ = template_id;
        Err(DbError::Init(
            "creative studio template run persistence is unavailable in this repository".into(),
        ))
    }

    /// One durable template run by business ID, or `None`.
    async fn get_creative_template_run(
        &self,
        template_run_id: &str,
    ) -> Result<Option<CreativeStudioTemplateRunRow>, DbError> {
        let _ = template_run_id;
        Err(DbError::Init(
            "creative studio template run persistence is unavailable in this repository".into(),
        ))
    }

    /// Insert revision one of a closed template-run aggregate.
    async fn create_creative_template_run(
        &self,
        row: &CreativeStudioTemplateRunRow,
        referenced_asset_ids: &[String],
    ) -> Result<CreativeStudioTemplateRunRow, DbError> {
        let _ = (row, referenced_asset_ids);
        Err(DbError::Init(
            "creative studio template run persistence is unavailable in this repository".into(),
        ))
    }

    /// Compare-and-swap a template-run aggregate. The replacement must carry
    /// the same identity and `expected_revision + 1`.
    async fn save_creative_template_run(
        &self,
        template_run_id: &str,
        expected_revision: i64,
        row: &CreativeStudioTemplateRunRow,
    ) -> Result<CreativeStudioTemplateRunRow, DbError> {
        let _ = (template_run_id, expected_revision, row);
        Err(DbError::Init(
            "creative studio template run persistence is unavailable in this repository".into(),
        ))
    }

    // ---- assets ----

    /// Insert a fully-formed asset row.
    async fn create_asset(&self, row: &WorkshopAssetRow) -> Result<WorkshopAssetRow, DbError>;

    /// Atomically materialize one prompt-library item or return the already
    /// materialized row. Implementations must make concurrent calls for the
    /// same `(source, prompt_library_id)` converge on one asset. Catalog lookup
    /// also recognizes the legacy `origin.prompt_catalog_id` representation.
    async fn create_prompt_library_asset(
        &self,
        row: &WorkshopAssetRow,
        identity: PromptLibraryAssetIdentity<'_>,
    ) -> Result<WorkshopAssetRow, DbError>;

    /// Soft-remove every materialization of one prompt-library identity from
    /// My Assets. Catalog matching includes the legacy `prompt_catalog_id`
    /// representation so historical duplicates cannot keep membership alive.
    /// Rows and files remain intact for project/task references.
    async fn hide_prompt_library_assets(
        &self,
        identity: PromptLibraryAssetIdentity<'_>,
        now: i64,
    ) -> Result<u64, DbError>;

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
