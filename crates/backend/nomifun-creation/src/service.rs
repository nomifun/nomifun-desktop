//! [`CreationService`] — the generation task queue + state machine (contract §6
//! `service.rs`).
//!
//! M0 is the skeleton: `create_task` enqueues `queued` then immediately
//! transitions to `failed(adapter_unavailable)` (no adapter is wired yet); the
//! query/cancel transitions are fully live; [`CreationService::reconcile_on_boot`]
//! settles any stray live task. The `providers` registry, per-provider
//! concurrency gate, poll loop, cancellation propagation, and [`AssetSink`]
//! plumbing arrive in M2.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::{AppError, generate_prefixed_id, now_ms};
use nomifun_db::{
    CreateCreationTaskParams, ICreationTaskRepository, ListCreationTasksParams, UpdateCreationTaskParams,
};
use serde_json::Value;

use crate::dto::CreationTask;
use crate::provider::{MediaProvider, ProducedAsset};
use crate::types::{CreationError, CreationInput, MediaCapability, TaskStatus};

/// A generation request accepted by [`CreationService::create_task`].
pub struct NewCreationTask {
    pub canvas_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    /// Wire capability code (`t2i|i2i|…`).
    pub capability: String,
    /// Opaque parameter map (prompt/size/quality/…).
    pub params: Value,
    pub inputs: Vec<CreationInput>,
}

/// Where produced artifacts are persisted. Implemented by the app over
/// `nomifun-workshop` (registers each result as a `wsa_` asset), so this crate
/// never depends on `nomifun-workshop` — no dependency cycle. Unused in M0
/// (nothing produces artifacts yet); wired in M2.
#[async_trait]
pub trait AssetSink: Send + Sync {
    /// Persist one produced artifact and return its new `wsa_` asset id.
    async fn persist(
        &self,
        canvas_id: Option<&str>,
        node_id: Option<&str>,
        produced: ProducedAsset,
        origin: Value,
    ) -> Result<String, CreationError>;
}

pub struct CreationService {
    repo: Arc<dyn ICreationTaskRepository>,
    /// Registered media adapters (empty in M0; populated in M2).
    providers: Vec<Arc<dyn MediaProvider>>,
}

impl CreationService {
    /// Build the service over its task repo (no adapters wired — M0).
    pub fn new(repo: Arc<dyn ICreationTaskRepository>) -> Arc<Self> {
        Arc::new(Self { repo, providers: Vec::new() })
    }

    /// The first registered adapter that serves `cap`, if any. Always `None` in
    /// M0 (no adapters). Drives adapter selection once M2 registers providers.
    pub fn provider_for(&self, cap: MediaCapability) -> Option<Arc<dyn MediaProvider>> {
        self.providers.iter().find(|p| p.supports(cap)).cloned()
    }

    /// Enqueue a task. M0: persist `queued`, then immediately fail with
    /// `adapter_unavailable` (M2 will spawn the run loop instead).
    pub async fn create_task(&self, req: NewCreationTask) -> Result<CreationTask, AppError> {
        let capability = MediaCapability::parse(&req.capability).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unknown capability '{}' (expected t2i|i2i|inpaint|t2v|i2v|v2v|tts|text)",
                req.capability
            ))
        })?;
        if req.provider_id.trim().is_empty() {
            return Err(AppError::BadRequest("provider_id must not be empty".into()));
        }
        if req.model.trim().is_empty() {
            return Err(AppError::BadRequest("model must not be empty".into()));
        }
        // Validate input references up front (cheap contract check).
        if let Some(bad) = req.inputs.iter().find(|i| i.asset_id.trim().is_empty()) {
            return Err(AppError::BadRequest(format!("input has an empty asset_id (role '{}')", bad.role)));
        }

        let params_json = serde_json::to_string(&req.params)
            .map_err(|e| AppError::BadRequest(format!("invalid params json: {e}")))?;
        let id = generate_prefixed_id("wst");
        let now = now_ms();
        self.repo
            .create_task(CreateCreationTaskParams {
                id: &id,
                canvas_id: req.canvas_id.as_deref(),
                node_id: req.node_id.as_deref(),
                provider_id: &req.provider_id,
                model: &req.model,
                capability: capability.as_str(),
                params: &params_json,
                status: TaskStatus::Queued.as_str(),
                submitted_at: now,
            })
            .await?;

        // M0: no adapter → settle to failed immediately so the caller gets a
        // definitive terminal task instead of a stuck `queued`.
        let error_json = serde_json::to_string(&CreationError::adapter_unavailable())
            .unwrap_or_else(|_| r#"{"kind":"adapter_unavailable","message":"unavailable"}"#.to_string());
        let row = self
            .repo
            .update_task(
                &id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Failed.as_str()),
                    error: Some(Some(&error_json)),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        Ok(row.into())
    }

    pub async fn get_task(&self, id: &str) -> Result<CreationTask, AppError> {
        let row = self
            .repo
            .get_task(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("creation task {id} not found")))?;
        Ok(row.into())
    }

    pub async fn list_tasks(
        &self,
        canvas_id: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CreationTask>, AppError> {
        let rows = self
            .repo
            .list_tasks(ListCreationTasksParams {
                canvas_id: canvas_id.filter(|s| !s.trim().is_empty()),
                status: status.filter(|s| !s.trim().is_empty()),
                limit,
            })
            .await?;
        Ok(rows.into_iter().map(CreationTask::from).collect())
    }

    /// Cancel a task. Terminal tasks are returned unchanged (idempotent);
    /// live tasks move to `canceled`.
    pub async fn cancel_task(&self, id: &str) -> Result<CreationTask, AppError> {
        let row = self
            .repo
            .get_task(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("creation task {id} not found")))?;
        let status = TaskStatus::parse_str(&row.status);
        if matches!(status, Some(s) if s.is_terminal()) {
            return Ok(row.into());
        }
        // M2 will also signal the live run loop / abort the HTTP call here.
        let updated = self
            .repo
            .update_task(
                id,
                UpdateCreationTaskParams {
                    status: Some(TaskStatus::Canceled.as_str()),
                    finished_at: Some(Some(now_ms())),
                    ..Default::default()
                },
            )
            .await?;
        Ok(updated.into())
    }

    /// Boot reconciliation: settle any task left `queued`/`running` by a
    /// previous process. M0 has no live loop to resume, so a stray live task is
    /// converged to `failed(interrupted)` — the "running ⟺ live worker"
    /// invariant the orchestrator uses. Best-effort; returns the count settled.
    pub async fn reconcile_on_boot(&self) -> usize {
        let live = match self.repo.list_live_tasks().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "creation boot reconcile: list live tasks failed");
                return 0;
            }
        };
        let error_json = serde_json::to_string(&CreationError::new(
            "interrupted",
            "task did not survive a restart (no live worker); settled at boot",
        ))
        .unwrap_or_else(|_| r#"{"kind":"interrupted","message":"interrupted"}"#.to_string());
        let mut settled = 0;
        for row in live {
            let r = self
                .repo
                .update_task(
                    &row.id,
                    UpdateCreationTaskParams {
                        status: Some(TaskStatus::Failed.as_str()),
                        error: Some(Some(&error_json)),
                        finished_at: Some(Some(now_ms())),
                        ..Default::default()
                    },
                )
                .await;
            match r {
                Ok(_) => settled += 1,
                Err(e) => tracing::warn!(id = %row.id, error = %e, "creation boot reconcile: settle failed"),
            }
        }
        if settled > 0 {
            tracing::info!(settled, "creation boot reconcile settled interrupted tasks");
        }
        settled
    }
}

impl TaskStatus {
    fn parse_str(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::SqliteCreationTaskRepository;

    async fn service() -> Arc<CreationService> {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn ICreationTaskRepository> = Arc::new(SqliteCreationTaskRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        CreationService::new(repo)
    }

    fn new_task() -> NewCreationTask {
        NewCreationTask {
            canvas_id: Some("wsc_1".into()),
            node_id: None,
            provider_id: "prov_x".into(),
            model: "m".into(),
            capability: "t2i".into(),
            params: serde_json::json!({"prompt": "cat"}),
            inputs: vec![],
        }
    }

    #[tokio::test]
    async fn create_task_fails_with_adapter_unavailable_in_m0() {
        let svc = service().await;
        let task = svc.create_task(new_task()).await.unwrap();
        assert!(task.id.starts_with("wst_"));
        assert_eq!(task.status, "failed");
        assert_eq!(task.error.as_ref().unwrap()["kind"], "adapter_unavailable");
        assert!(task.finished_at.is_some());
        assert_eq!(task.params["prompt"], "cat");

        // fetchable + listable
        let got = svc.get_task(&task.id).await.unwrap();
        assert_eq!(got.id, task.id);
        let list = svc.list_tasks(Some("wsc_1"), None, 50).await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn create_task_rejects_bad_capability_and_empty_provider() {
        let svc = service().await;
        let mut bad = new_task();
        bad.capability = "nope".into();
        assert!(matches!(svc.create_task(bad).await.unwrap_err(), AppError::BadRequest(_)));

        let mut bad2 = new_task();
        bad2.provider_id = "  ".into();
        assert!(matches!(svc.create_task(bad2).await.unwrap_err(), AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn cancel_is_idempotent_on_terminal() {
        let svc = service().await;
        let task = svc.create_task(new_task()).await.unwrap();
        // already failed (terminal) → cancel returns unchanged
        let canceled = svc.cancel_task(&task.id).await.unwrap();
        assert_eq!(canceled.status, "failed");
        assert!(matches!(svc.cancel_task("wst_missing").await.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn no_adapter_registered_in_m0() {
        let svc = service().await;
        assert!(svc.provider_for(MediaCapability::T2i).is_none());
    }
}
