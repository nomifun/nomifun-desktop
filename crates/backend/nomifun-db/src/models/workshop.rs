use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the canonical Creative Studio project index + document.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreativeStudioProjectRow {
    pub id: i64,
    pub project_id: String,
    pub title: String,
    /// Monotonic document revision. Metadata-only renames do not change it.
    pub revision: i64,
    pub node_count: i64,
    pub connection_count: i64,
    pub document_json: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Durable exactly-once receipt for one completed Canvas Agent assistant
/// proposal. The operation/result JSON is server-canonical and the applied
/// revision identifies the project snapshot created by the first execution.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreativeStudioAgentProposalReceiptRow {
    pub id: i64,
    pub project_id: String,
    pub assistant_message_id: String,
    pub ops_fingerprint: String,
    pub ops_json: String,
    pub results_json: String,
    pub applied_revision: i64,
    pub created_at: TimestampMs,
}

/// Row mapping for a canonical Creative Studio workflow definition.
///
/// The JSON body is validated by `nomifun-workshop`; indexed metadata is kept
/// beside it for deterministic list/search views without accepting a second
/// source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreativeStudioWorkflowRow {
    pub id: i64,
    pub workflow_id: String,
    pub revision: i64,
    pub name: String,
    pub description: String,
    pub category: String,
    pub visibility: String,
    pub definition_json: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for one durable canonical Creative Studio workflow run.
///
/// `aggregate_json` is a closed v1 contract validated by `nomifun-workshop`;
/// the duplicated workflow/status fields provide indexed ownership and
/// recovery queries without creating a second source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreativeStudioWorkflowRunRow {
    pub id: i64,
    pub workflow_run_id: String,
    pub workflow_id: String,
    pub workflow_revision: i64,
    pub revision: i64,
    pub status: String,
    pub step_ids_json: String,
    pub aggregate_json: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `workshop_assets` table (创意工坊 资产库).
///
/// Metadata is indexed here; the binary lives under the data dir at `rel_path`
/// (`workshop/assets/{asset_id}.{ext}`). `text` assets carry their body in
/// `text_content` and have no file. `tags` / `origin` are stored as JSON TEXT
/// and parsed by the service layer.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkshopAssetRow {
    pub id: i64,
    pub asset_id: String,
    /// `image | video | text`.
    pub kind: String,
    pub title: String,
    pub collection: Option<String>,
    /// JSON array of tag strings.
    pub tags: String,
    /// Relative to the data dir; `None` for text assets.
    pub rel_path: Option<String>,
    pub thumb_rel_path: Option<String>,
    pub mime: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: Option<i64>,
    pub text_content: Option<String>,
    /// `1` = appears in the asset library; `0` = project-internal material.
    pub in_library: bool,
    /// Canonical provenance object. Durable ownership is either
    /// `{project_id,node_id}` or `{workflow_id,workflow_run_id,workflow_step_id}`.
    pub origin: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `creation_tasks` table (生成引擎 任务队列).
///
/// `params` / `input_bindings` / `error` / `result_asset_ids` are JSON TEXT
/// parsed by the service layer. `provider_id` is a provider business-ID logical
/// reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationTaskRow {
    pub creation_task_id: String,
    /// Canonical project owner. A node or standalone workbench discriminator
    /// completes the branch; both are mutually exclusive with workflow ownership.
    pub project_id: Option<String>,
    pub workbench_kind: Option<String>,
    /// Canonical workflow-step owner. All three workflow columns are present together.
    pub workflow_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub workflow_step_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    /// `t2i|i2i|inpaint|t2v|i2v|v2v|tts|text`.
    pub capability: String,
    /// JSON parameter snapshot.
    pub params: String,
    /// Canonical ordered `{asset_id,kind,role}` array. `None` is reserved for a
    /// pre-044 row whose complete bindings could not be proven during migration.
    pub input_bindings: Option<String>,
    /// `queued|running|succeeded|failed|canceled`.
    pub status: String,
    /// JSON `{kind,message,http_status?}`; `None`.
    pub error: Option<String>,
    /// JSON array of bare canonical lowercase-hyphenated UUIDv7 asset IDs.
    pub result_asset_ids: String,
    /// Remote task id for async submit→poll protocols (boot resume).
    pub remote_task_id: Option<String>,
    pub attempt: i64,
    pub submitted_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    /// History retirement tombstone. The task and all manifests remain live
    /// canonical records; only owner-scoped list surfaces hide this row.
    pub deleted_at: Option<TimestampMs>,
}
