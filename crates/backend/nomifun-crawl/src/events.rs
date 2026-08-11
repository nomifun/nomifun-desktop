//! Progress events. The runner depends on the trait, not on the realtime hub,
//! so tests run the pool without a WebSocket.

use nomifun_common::CrawlJobId;
use serde::Serialize;

use crate::model::{CrawlTask, JobProgress, JobStatus, TaskOutcome};

pub trait CrawlEventSink: Send + Sync {
    fn job_progress(&self, job_id: &CrawlJobId, progress: &JobProgress);
    fn task_settled(&self, job_id: &CrawlJobId, task: &CrawlTask, outcome: &TaskOutcome);
    fn job_finished(&self, job_id: &CrawlJobId, status: JobStatus, progress: &JobProgress);
}

pub struct NoopEvents;

impl CrawlEventSink for NoopEvents {
    fn job_progress(&self, _job_id: &CrawlJobId, _progress: &JobProgress) {}
    fn task_settled(&self, _job_id: &CrawlJobId, _task: &CrawlTask, _outcome: &TaskOutcome) {}
    fn job_finished(&self, _job_id: &CrawlJobId, _status: JobStatus, _progress: &JobProgress) {}
}

/// Payload shapes broadcast over `/ws`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CrawlEvent {
    Progress {
        job_id: String,
        progress: JobProgress,
    },
    Task {
        job_id: String,
        task_id: String,
        url: String,
        status: String,
        http_status: Option<u16>,
        detail: Option<String>,
    },
    Finished {
        job_id: String,
        status: String,
        progress: JobProgress,
    },
}

impl CrawlEvent {
    pub fn from_outcome(job_id: &CrawlJobId, task: &CrawlTask, outcome: &TaskOutcome) -> Self {
        let (status, http_status, detail) = match outcome {
            TaskOutcome::Fetched { http_status, .. } => ("done", Some(*http_status), None),
            TaskOutcome::Unchanged { http_status } => ("unchanged", Some(*http_status), None),
            TaskOutcome::Skipped { reason } => ("skipped", None, Some(reason.clone())),
            TaskOutcome::Failed { error_code, error_detail, .. } => (
                "failed",
                None,
                Some(format!("{error_code}: {error_detail}")),
            ),
        };
        Self::Task {
            job_id: job_id.to_string(),
            task_id: task.task_id.to_string(),
            url: task.url.clone(),
            status: status.to_string(),
            http_status,
            detail,
        }
    }
}
