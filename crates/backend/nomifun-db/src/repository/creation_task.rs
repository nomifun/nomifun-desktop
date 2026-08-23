use crate::error::DbError;
use crate::models::CreationTaskRow;

/// Data access for the `creation_tasks` table (生成引擎 任务队列 状态机).
///
/// The `nomifun-creation` service owns the state machine; this repo is the
/// persistence seam. `params` / `error` / `result_asset_ids` are pre-serialized
/// JSON strings the caller builds.
#[async_trait::async_trait]
pub trait ICreationTaskRepository: Send + Sync {
    /// Atomically insert or recover one canonical Creative Studio task.
    ///
    /// `creation_task_id` is the caller's UUIDv7 Idempotency-Key. An existing
    /// row is returned only when its persisted canonical request fingerprint
    /// is byte-for-byte identical; reusing a key for another request is a
    /// conflict. `inserted` is the sole authority for spawning a worker.
    async fn get_or_create_creative_task(
        &self,
        params: CreateCreativeTaskParams<'_>,
    ) -> Result<IdempotentCreationTask, DbError> {
        let _ = params;
        Err(DbError::Init(
            "canonical creative task idempotency is unavailable in this repository".into(),
        ))
    }

    /// One task by stable business id, or `None`.
    async fn get_task(
        &self,
        creation_task_id: &str,
    ) -> Result<Option<CreationTaskRow>, DbError>;

    /// Newest-first keyset page for one exact standalone-workbench aggregate.
    /// Implementations fetch `limit + 1` rows so the service can derive an
    /// opaque continuation cursor without a count query.
    async fn list_standalone_workbench_tasks_page(
        &self,
        params: ListStandaloneWorkbenchTasksParams<'_>,
    ) -> Result<Vec<CreationTaskRow>, DbError> {
        let _ = params;
        Err(DbError::Init(
            "standalone workbench task paging is unavailable in this repository".into(),
        ))
    }

    /// Atomically tombstone one exact terminal standalone-owner batch. Existing
    /// tombstones are idempotent. Implementations return rows in request order.
    async fn retire_standalone_workbench_tasks(
        &self,
        params: RetireStandaloneWorkbenchTasksParams<'_>,
    ) -> Result<Vec<CreationTaskRow>, DbError> {
        let _ = params;
        Err(DbError::Init(
            "standalone workbench task retirement is unavailable in this repository".into(),
        ))
    }

    /// Complete task inventory for boot-time artifact reconciliation. Unlike
    /// the paginated API listing, this intentionally has no 500-row cap.
    async fn list_all_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError>;

    /// Partial state-machine update. `DbError::NotFound` when the business id is unknown.
    async fn update_task(
        &self,
        creation_task_id: &str,
        params: UpdateCreationTaskParams<'_>,
    ) -> Result<CreationTaskRow, DbError>;

    /// Conditional terminal-state write: apply `params` ONLY if the task is
    /// still live (`status IN ('queued','running')`). Returns `Ok(true)` when
    /// the row was updated, `Ok(false)` when the task was no longer live (e.g.
    /// already `canceled`) or unknown. Unlike [`Self::update_task`] this never
    /// overwrites a terminal status — the worker's finalize routes through it so
    /// a `cancel` that lands mid-finalize is not silently flipped to
    /// `succeeded`/`failed` (compare-and-set on `status`, not the token).
    async fn update_task_if_live(
        &self,
        creation_task_id: &str,
        params: UpdateCreationTaskParams<'_>,
    ) -> Result<bool, DbError>;

    /// Patch only the remote provider handle while the task is live. This is a
    /// single-statement CAS and must never rewrite status from a stale row
    /// snapshot when cancel races async submission.
    async fn set_remote_task_id_if_live(
        &self,
        creation_task_id: &str,
        remote_task_id: &str,
    ) -> Result<bool, DbError>;

    /// Every task currently in a live (`queued`/`running`) state — the boot
    /// reconciliation input.
    async fn list_live_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError>;
}

#[derive(Debug, Clone)]
pub struct IdempotentCreationTask {
    pub row: CreationTaskRow,
    pub inserted: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CreationTaskPageCursorRef<'a> {
    pub submitted_at: i64,
    pub creation_task_id: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct ListStandaloneWorkbenchTasksParams<'a> {
    pub workbench_kind: &'a str,
    pub active_only: bool,
    pub before: Option<CreationTaskPageCursorRef<'a>>,
    /// Requested visible page size. The repository reads one additional row.
    pub limit: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct RetireStandaloneWorkbenchTasksParams<'a> {
    pub workbench_kind: &'a str,
    pub task_ids: &'a [String],
    pub deleted_at: i64,
}

/// Strict canonical Creative Studio task owner. No field is shared between
/// the two branches, so callers cannot accidentally reinterpret a template
/// step as a canvas node.
#[derive(Debug, Clone, Copy)]
pub enum CreativeTaskOwnerRef<'a> {
    CanvasNode {
        project_id: &'a str,
        node_id: &'a str,
    },
    StandaloneWorkbench {
        workbench_kind: &'a str,
    },
    TemplateStep {
        template_id: &'a str,
        template_run_id: &'a str,
        template_step_id: &'a str,
    },
}

/// Canonical Creative Studio create parameters. The exact tagged owner and
/// request are persisted for durable idempotency comparison.
#[derive(Debug)]
pub struct CreateCreativeTaskParams<'a> {
    pub creation_task_id: &'a str,
    pub owner: CreativeTaskOwnerRef<'a>,
    pub provider_id: &'a str,
    pub model: &'a str,
    pub capability: &'a str,
    pub params: &'a str,
    /// Canonical ordered JSON array of `{asset_id,kind,role}` objects. New
    /// writes must always supply it, including `[]` for no inputs.
    pub input_bindings: &'a str,
    pub request_fingerprint: &'a str,
    pub status: &'a str,
    pub submitted_at: i64,
}

/// Partial-update params for [`ICreationTaskRepository::update_task`]. Each
/// `Some` replaces the field; `None` keeps the current value. Inner `Option`
/// (for nullable columns) distinguishes "set to NULL" from "keep".
#[derive(Debug, Default)]
pub struct UpdateCreationTaskParams<'a> {
    pub status: Option<&'a str>,
    pub error: Option<Option<&'a str>>,
    /// Replacement JSON array string of result asset ids.
    pub result_asset_ids: Option<&'a str>,
    pub remote_task_id: Option<Option<&'a str>>,
    pub attempt: Option<i64>,
    pub started_at: Option<Option<i64>>,
    pub finished_at: Option<Option<i64>>,
}
