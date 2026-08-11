//! The worker pool for one job.
//!
//! Each worker claims, renews, executes, and submits. Nothing here tracks
//! worker liveness: a worker that dies simply stops renewing, and the reaper
//! returns its task to the queue.

use std::sync::Arc;
use std::time::Duration;

use nomifun_common::CrawlJobId;
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::claim::{self, LEASE_MS};
use crate::error::CrawlError;
use crate::events::CrawlEventSink;
use crate::executor::{CrawlExecutor, renew_interval};
use crate::model::{CrawlJob, JobStatus};
use crate::store;

/// How long a worker waits before re-checking a queue that had nothing for it.
/// Short enough to pick up newly discovered URLs, long enough not to spin.
const IDLE_POLL: Duration = Duration::from_millis(250);
/// How often expired leases are swept back into the queue.
const REAP_INTERVAL: Duration = Duration::from_secs(15);

pub struct RunnerConfig {
    pub node_id: String,
    pub lease_ms: i64,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self { node_id: "local".into(), lease_ms: LEASE_MS }
    }
}

/// A running job. Dropping this does not stop the workers; call [`Self::cancel`].
pub struct JobHandle {
    job_id: CrawlJobId,
    cancel: CancellationToken,
}

impl JobHandle {
    pub fn job_id(&self) -> &CrawlJobId {
        &self.job_id
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// Start the pool. Returns once the workers are spawned; the job finishes in
/// the background and the returned handle can cancel it.
pub fn spawn_job(
    pool: SqlitePool,
    job: CrawlJob,
    executor: Arc<dyn CrawlExecutor>,
    events: Arc<dyn CrawlEventSink>,
    config: RunnerConfig,
    cancel: CancellationToken,
) -> JobHandle {
    let handle = JobHandle { job_id: job.job_id.clone(), cancel: cancel.clone() };
    tokio::spawn(async move {
        if let Err(err) = run_job(pool, job.clone(), executor, events.clone(), config, cancel).await
        {
            warn!(job_id = %job.job_id, error = %err, "crawl job ended with an error");
        }
    });
    handle
}

async fn run_job(
    pool: SqlitePool,
    job: CrawlJob,
    executor: Arc<dyn CrawlExecutor>,
    events: Arc<dyn CrawlEventSink>,
    config: RunnerConfig,
    cancel: CancellationToken,
) -> Result<(), CrawlError> {
    let mut workers = JoinSet::new();
    let config = Arc::new(config);

    for index in 0..job.concurrency.max(1) {
        let pool = pool.clone();
        let job = job.clone();
        let executor = executor.clone();
        let events = events.clone();
        let config = config.clone();
        let cancel = cancel.clone();
        workers.spawn(async move {
            worker_loop(pool, job, executor, events, config, cancel, index).await
        });
    }

    // The reaper runs alongside the workers: a task whose owner died mid-job
    // has to come back while the job is still running, not after it drains.
    let reaper = {
        let pool = pool.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(REAP_INTERVAL) => {
                        if let Err(err) = claim::reap_expired(&pool).await {
                            warn!(error = %err, "lease reap failed");
                        }
                    }
                }
            }
        })
    };

    while workers.join_next().await.is_some() {}
    reaper.abort();

    let progress = claim::progress(&pool, &job.job_id).await?;
    let status = if cancel.is_cancelled() {
        JobStatus::Cancelled
    } else if progress.failed > 0 && progress.done == 0 {
        JobStatus::Failed
    } else {
        JobStatus::Done
    };
    store::finish_job(&pool, &job.job_id, status, None).await?;
    events.job_finished(&job.job_id, status, &progress);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn worker_loop(
    pool: SqlitePool,
    job: CrawlJob,
    executor: Arc<dyn CrawlExecutor>,
    events: Arc<dyn CrawlEventSink>,
    config: Arc<RunnerConfig>,
    cancel: CancellationToken,
    index: u32,
) {
    let node_id = format!("{}#{index}", config.node_id);
    loop {
        if cancel.is_cancelled() {
            return;
        }

        let claimed = match claim::claim_next(
            &pool,
            &job.job_id,
            &node_id,
            job.per_host_concurrency.max(1),
            config.lease_ms,
        )
        .await
        {
            Ok(Some(claimed)) => claimed,
            Ok(None) => {
                // Nothing claimable: either the queue is drained (stop) or
                // every remaining URL is behind a per-host gate (wait).
                match claim::progress(&pool, &job.job_id).await {
                    Ok(p) if p.is_drained() => return,
                    Ok(_) => {}
                    Err(err) => {
                        warn!(error = %err, "progress query failed");
                        return;
                    }
                }
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(IDLE_POLL) => continue,
                }
            }
            Err(err) => {
                warn!(error = %err, "claim failed");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(IDLE_POLL) => continue,
                }
            }
        };

        let task_id = claimed.task.task_id.clone();
        let token = claimed.claim_token.clone();

        // Three-way race: finish the work, lose the claim, or get cancelled.
        // Losing the claim must not submit — someone else owns the task now.
        let outcome = tokio::select! {
            outcome = executor.execute(&claimed.task, &job) => Some(outcome),
            _ = keep_lease_alive(&pool, &claimed, config.lease_ms) => {
                debug!(task_id = %task_id, "lost the claim mid-flight; discarding result");
                None
            }
            _ = cancel.cancelled() => None,
        };

        let Some(outcome) = outcome else {
            continue;
        };

        events.task_settled(&job.job_id, &claimed.task, &outcome);
        match claim::submit(&pool, &task_id, &token, outcome, job.max_urls).await {
            Ok(()) => {}
            // A stale claim here means the lease expired between the last
            // renewal and the submit. The reaper already requeued it.
            Err(CrawlError::StaleClaim(_)) => {
                debug!(task_id = %task_id, "submit rejected as stale");
            }
            Err(err) => warn!(task_id = %task_id, error = %err, "submit failed"),
        }

        if let Ok(progress) = claim::progress(&pool, &job.job_id).await {
            events.job_progress(&job.job_id, &progress);
        }
    }
}

/// Renew until the claim is lost. Returns only on failure — while renewals
/// succeed this future never resolves, which is what makes it a usable
/// `select!` arm against the work itself.
async fn keep_lease_alive(
    pool: &SqlitePool,
    claimed: &crate::model::ClaimedTask,
    lease_ms: i64,
) {
    let interval = renew_interval(lease_ms);
    loop {
        tokio::time::sleep(interval).await;
        if claim::renew_lease(pool, &claimed.task.task_id, &claimed.claim_token, lease_ms)
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoopEvents;
    use crate::model::{CrawlScope, CrawlSink, DiscoveredUrl, RenderMode, TaskOutcome};
    use crate::{frontier, store};
    use nomifun_common::UserId;
    use nomifun_db::init_database_memory;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingExecutor {
        seen: AtomicU32,
        /// Links each page hands back, as (depth-1 children) URLs.
        children: Vec<String>,
    }

    #[async_trait::async_trait]
    impl CrawlExecutor for CountingExecutor {
        async fn execute(
            &self,
            task: &crate::model::CrawlTask,
            _job: &CrawlJob,
        ) -> TaskOutcome {
            self.seen.fetch_add(1, Ordering::SeqCst);
            let discovered = if task.depth == 0 {
                self.children
                    .iter()
                    .filter_map(|u| {
                        let url = frontier::normalize(u).ok()?;
                        Some(DiscoveredUrl {
                            fingerprint: frontier::fingerprint(&url),
                            host: url.host_str()?.to_string(),
                            url: url.to_string(),
                            depth: 1,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            TaskOutcome::Fetched {
                http_status: 200,
                content_hash: "a".repeat(64),
                etag: None,
                last_modified: None,
                discovered,
            }
        }
    }

    struct AlwaysFails;

    #[async_trait::async_trait]
    impl CrawlExecutor for AlwaysFails {
        async fn execute(&self, _t: &crate::model::CrawlTask, _j: &CrawlJob) -> TaskOutcome {
            TaskOutcome::Failed {
                error_code: "boom".into(),
                error_detail: "always".into(),
                retryable: false,
            }
        }
    }

    /// Insert the owning user so `crawl_jobs.user_id` has a real parent.
    async fn seed_user(pool: &SqlitePool, user_id: &UserId) {
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO users (user_id, username, password_hash, created_at, updated_at) \
             VALUES (?, ?, 'x', ?, ?)",
        )
        .bind(user_id.as_str())
        .bind(format!("u{}", &user_id.as_str()[..8]))
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("user");
    }

    fn job(concurrency: u32) -> CrawlJob {
        CrawlJob {
            job_id: nomifun_common::CrawlJobId::new(),
            user_id: UserId::new(),
            name: "runner".into(),
            seeds: vec!["https://example.com/".into()],
            scope: CrawlScope::default(),
            max_depth: 2,
            max_urls: 50,
            render_mode: RenderMode::Http,
            concurrency,
            per_host_concurrency: 4,
            delay_ms: 0,
            respect_robots: false,
            user_agent: None,
            sink: CrawlSink::default(),
            status: JobStatus::Running,
            error_detail: None,
            started_at: None,
            finished_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    async fn seed(pool: &SqlitePool, job: &CrawlJob, urls: &[&str]) {
        for u in urls {
            let url = frontier::normalize(u).unwrap();
            let d = DiscoveredUrl {
                fingerprint: frontier::fingerprint(&url),
                host: url.host_str().unwrap().to_string(),
                url: url.to_string(),
                depth: 0,
            };
            claim::enqueue(pool, &job.job_id, None, &d, 0).await.unwrap();
        }
    }

    #[tokio::test]
    async fn drains_the_queue_and_marks_the_job_done() {
        let db = init_database_memory().await.unwrap();
        let j = job(2);
        seed_user(db.pool(), &j.user_id).await;
        store::create_job(db.pool(), &j).await.unwrap();
        seed(db.pool(), &j, &["https://example.com/a", "https://example.com/b"]).await;

        let exec = Arc::new(CountingExecutor {
            seen: AtomicU32::new(0),
            children: vec!["https://example.com/c".into()],
        });
        let cancel = CancellationToken::new();
        run_job(
            db.pool().clone(),
            j.clone(),
            exec.clone(),
            Arc::new(NoopEvents),
            RunnerConfig::default(),
            cancel,
        )
        .await
        .unwrap();

        // 2 seeds + 1 discovered child (deduped across both seeds).
        assert_eq!(exec.seen.load(Ordering::SeqCst), 3);
        let progress = claim::progress(db.pool(), &j.job_id).await.unwrap();
        assert_eq!(progress.done, 3);
        assert!(progress.is_drained());
        let stored = store::get_job(db.pool(), &j.job_id).await.unwrap();
        assert_eq!(stored.status, JobStatus::Done);
    }

    #[tokio::test]
    async fn a_job_whose_every_task_fails_is_marked_failed() {
        let db = init_database_memory().await.unwrap();
        let j = job(1);
        seed_user(db.pool(), &j.user_id).await;
        store::create_job(db.pool(), &j).await.unwrap();
        seed(db.pool(), &j, &["https://example.com/a"]).await;

        run_job(
            db.pool().clone(),
            j.clone(),
            Arc::new(AlwaysFails),
            Arc::new(NoopEvents),
            RunnerConfig::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(store::get_job(db.pool(), &j.job_id).await.unwrap().status, JobStatus::Failed);
    }

    #[tokio::test]
    async fn cancelling_stops_the_pool_and_marks_the_job_cancelled() {
        let db = init_database_memory().await.unwrap();
        let j = job(1);
        seed_user(db.pool(), &j.user_id).await;
        store::create_job(db.pool(), &j).await.unwrap();
        let many: Vec<String> = (0..200).map(|i| format!("https://example.com/p{i}")).collect();
        seed(db.pool(), &j, &many.iter().map(String::as_str).collect::<Vec<_>>()).await;

        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });

        run_job(
            db.pool().clone(),
            j.clone(),
            Arc::new(CountingExecutor { seen: AtomicU32::new(0), children: vec![] }),
            Arc::new(NoopEvents),
            RunnerConfig::default(),
            cancel,
        )
        .await
        .unwrap();

        let stored = store::get_job(db.pool(), &j.job_id).await.unwrap();
        assert_eq!(stored.status, JobStatus::Cancelled);
        let progress = claim::progress(db.pool(), &j.job_id).await.unwrap();
        assert!(progress.pending > 0, "cancellation should leave work unclaimed");
    }
}
