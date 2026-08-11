//! The claim protocol: atomic task allocation, renewable leases, and fenced
//! submission.
//!
//! Correctness rests on three things and nothing else:
//!
//! 1. `claim_next` is a single `UPDATE ... WHERE task_id = (SELECT ...)`, so
//!    SQLite's writer serialization makes double-allocation impossible.
//! 2. Every mutation after the claim carries `claim_token`. A worker that
//!    stalled past its lease and woke up later fails the token match and its
//!    result is discarded instead of clobbering the retry.
//! 3. `reap_expired` is the only liveness mechanism. There is no heartbeat and
//!    no health check, so a worker that vanishes needs no detection.

use nomifun_common::{CrawlJobId, CrawlTaskId, TimestampMs, now_ms};
use sqlx::{Row, SqlitePool};

use crate::error::CrawlError;
use crate::model::{ClaimedTask, CrawlTask, DiscoveredUrl, JobProgress, TaskOutcome, TaskStatus};

/// Attempts allowed before a task is parked as `failed`.
pub const MAX_ATTEMPTS: u32 = 3;
/// Lease granted on claim. Workers renew at half this interval.
pub const LEASE_MS: i64 = 60_000;

const TASK_COLUMNS: &str = "task_id, job_id, parent_task_id, url, url_fingerprint, host, depth, \
     priority, status, attempt_count, claim_generation, owner_node_id, claimed_at, \
     lease_expires_at, http_status, content_hash, etag, last_modified, error_code, \
     error_detail, completed_at, created_at, updated_at";

fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("system randomness unavailable");
    hex::encode(bytes)
}

fn row_to_task(row: &sqlx::sqlite::SqliteRow) -> Result<CrawlTask, CrawlError> {
    let status_raw: String = row.try_get("status")?;
    let status = TaskStatus::parse(&status_raw)
        .ok_or_else(|| CrawlError::UrlRejected(format!("unknown task status {status_raw}")))?;
    let parse_task_id = |value: String| {
        CrawlTaskId::try_from(value.as_str())
            .map_err(|e| CrawlError::TaskNotFound(format!("malformed task id: {e}")))
    };
    Ok(CrawlTask {
        task_id: parse_task_id(row.try_get("task_id")?)?,
        job_id: CrawlJobId::try_from(row.try_get::<String, _>("job_id")?.as_str())
            .map_err(|e| CrawlError::JobNotFound(format!("malformed job id: {e}")))?,
        parent_task_id: row
            .try_get::<Option<String>, _>("parent_task_id")?
            .map(parse_task_id)
            .transpose()?,
        url: row.try_get("url")?,
        url_fingerprint: row.try_get("url_fingerprint")?,
        host: row.try_get("host")?,
        depth: row.try_get::<i64, _>("depth")? as u32,
        priority: row.try_get("priority")?,
        status,
        attempt_count: row.try_get::<i64, _>("attempt_count")? as u32,
        claim_generation: row.try_get("claim_generation")?,
        owner_node_id: row.try_get("owner_node_id")?,
        claimed_at: row.try_get("claimed_at")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        http_status: row.try_get::<Option<i64>, _>("http_status")?.map(|v| v as u16),
        content_hash: row.try_get("content_hash")?,
        etag: row.try_get("etag")?,
        last_modified: row.try_get("last_modified")?,
        error_code: row.try_get("error_code")?,
        error_detail: row.try_get("error_detail")?,
        completed_at: row.try_get("completed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Enqueue a URL. Returns `false` when the job already knows this fingerprint —
/// the `UNIQUE(job_id, url_fingerprint)` index is the dedup authority, so a
/// racing insert loses here rather than creating a duplicate.
pub async fn enqueue(
    pool: &SqlitePool,
    job_id: &CrawlJobId,
    parent: Option<&CrawlTaskId>,
    url: &DiscoveredUrl,
    priority: i64,
) -> Result<bool, CrawlError> {
    let now = now_ms();
    let task_id = CrawlTaskId::new();
    let result = sqlx::query(
        "INSERT OR IGNORE INTO crawl_tasks \
         (task_id, job_id, parent_task_id, url, url_fingerprint, host, depth, priority, \
          status, attempt_count, claim_generation, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, 0, ?, ?)",
    )
    .bind(task_id.as_str())
    .bind(job_id.as_str())
    .bind(parent.map(|p| p.as_str()))
    .bind(&url.url)
    .bind(&url.fingerprint)
    .bind(&url.host)
    .bind(url.depth as i64)
    .bind(priority)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Atomically take the next eligible pending task for `job_id`.
///
/// `per_host_concurrency` is enforced inside the selection subquery, not by the
/// caller, so two workers cannot each independently conclude a host has spare
/// capacity.
pub async fn claim_next(
    pool: &SqlitePool,
    job_id: &CrawlJobId,
    node_id: &str,
    per_host_concurrency: u32,
    lease_ms: i64,
) -> Result<Option<ClaimedTask>, CrawlError> {
    let now = now_ms();
    let token = mint_token();
    let sql = format!(
        "UPDATE crawl_tasks \
         SET status = 'in_progress', \
             claim_generation = claim_generation + 1, \
             attempt_count = attempt_count + 1, \
             claim_token = ?1, owner_node_id = ?2, claimed_at = ?3, \
             lease_expires_at = ?3 + ?4, updated_at = ?3 \
         WHERE task_id = ( \
             SELECT t.task_id FROM crawl_tasks t \
             WHERE t.job_id = ?5 AND t.status = 'pending' \
               AND ( \
                   SELECT COUNT(*) FROM crawl_tasks a \
                   WHERE a.job_id = ?5 AND a.host = t.host AND a.status = 'in_progress' \
               ) < ?6 \
             ORDER BY t.priority DESC, t.depth ASC, t.created_at ASC \
             LIMIT 1 \
         ) \
         RETURNING {TASK_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(&token)
        .bind(node_id)
        .bind(now)
        .bind(lease_ms)
        .bind(job_id.as_str())
        .bind(per_host_concurrency as i64)
        .fetch_optional(pool)
        .await?;

    match row {
        Some(row) => Ok(Some(ClaimedTask {
            task: row_to_task(&row)?,
            claim_token: token,
        })),
        None => Ok(None),
    }
}

/// Extend the lease. Fails when the token no longer matches, which is how a
/// worker learns it was reaped and should abandon the task.
pub async fn renew_lease(
    pool: &SqlitePool,
    task_id: &CrawlTaskId,
    claim_token: &str,
    lease_ms: i64,
) -> Result<(), CrawlError> {
    let now = now_ms();
    let affected = sqlx::query(
        "UPDATE crawl_tasks SET lease_expires_at = ?1 + ?2, updated_at = ?1 \
         WHERE task_id = ?3 AND claim_token = ?4 AND status = 'in_progress'",
    )
    .bind(now)
    .bind(lease_ms)
    .bind(task_id.as_str())
    .bind(claim_token)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(CrawlError::StaleClaim(task_id.to_string()));
    }
    Ok(())
}

/// Settle a claimed task and enqueue whatever it discovered, in one
/// transaction. A token mismatch aborts the whole thing: a zombie worker
/// neither settles the task nor injects URLs.
pub async fn submit(
    pool: &SqlitePool,
    task_id: &CrawlTaskId,
    claim_token: &str,
    outcome: TaskOutcome,
    max_urls: u32,
) -> Result<(), CrawlError> {
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let current = sqlx::query(
        "SELECT job_id, attempt_count FROM crawl_tasks \
         WHERE task_id = ?1 AND claim_token = ?2 AND status = 'in_progress'",
    )
    .bind(task_id.as_str())
    .bind(claim_token)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| CrawlError::StaleClaim(task_id.to_string()))?;

    let job_id: String = current.try_get("job_id")?;
    let attempt_count: i64 = current.try_get("attempt_count")?;

    match outcome {
        TaskOutcome::Fetched {
            http_status,
            content_hash,
            etag,
            last_modified,
            discovered,
        } => {
            settle(
                &mut tx,
                task_id,
                claim_token,
                "done",
                now,
                Some(http_status),
                Some(&content_hash),
                etag.as_deref(),
                last_modified.as_deref(),
                None,
                None,
            )
            .await?;

            let remaining = remaining_budget(&mut tx, &job_id, max_urls).await?;
            for url in discovered.iter().take(remaining) {
                let child_id = CrawlTaskId::new();
                sqlx::query(
                    "INSERT OR IGNORE INTO crawl_tasks \
                     (task_id, job_id, parent_task_id, url, url_fingerprint, host, depth, \
                      priority, status, attempt_count, claim_generation, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 0, 'pending', 0, 0, ?, ?)",
                )
                .bind(child_id.as_str())
                .bind(&job_id)
                .bind(task_id.as_str())
                .bind(&url.url)
                .bind(&url.fingerprint)
                .bind(&url.host)
                .bind(url.depth as i64)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
        }
        TaskOutcome::Unchanged { http_status } => {
            settle(
                &mut tx,
                task_id,
                claim_token,
                "done",
                now,
                Some(http_status),
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        }
        TaskOutcome::Skipped { reason } => {
            settle(
                &mut tx,
                task_id,
                claim_token,
                "skipped",
                now,
                None,
                None,
                None,
                None,
                Some("skipped"),
                Some(&reason),
            )
            .await?;
        }
        TaskOutcome::Failed {
            error_code,
            error_detail,
            retryable,
        } => {
            let exhausted = !retryable || attempt_count >= MAX_ATTEMPTS as i64;
            if exhausted {
                settle(
                    &mut tx,
                    task_id,
                    claim_token,
                    "failed",
                    now,
                    None,
                    None,
                    None,
                    None,
                    Some(&error_code),
                    Some(&error_detail),
                )
                .await?;
            } else {
                requeue(&mut tx, task_id, claim_token, now, &error_code, &error_detail).await?;
            }
        }
    }

    tx.commit().await?;
    Ok(())
}

/// How many more URLs this job may enqueue before hitting `max_urls`.
async fn remaining_budget(
    tx: &mut sqlx::SqliteConnection,
    job_id: &str,
    max_urls: u32,
) -> Result<usize, CrawlError> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM crawl_tasks WHERE job_id = ?")
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
    Ok((max_urls as i64 - total).max(0) as usize)
}

#[allow(clippy::too_many_arguments)]
async fn settle(
    tx: &mut sqlx::SqliteConnection,
    task_id: &CrawlTaskId,
    claim_token: &str,
    status: &str,
    now: TimestampMs,
    http_status: Option<u16>,
    content_hash: Option<&str>,
    etag: Option<&str>,
    last_modified: Option<&str>,
    error_code: Option<&str>,
    error_detail: Option<&str>,
) -> Result<(), CrawlError> {
    // Clearing token/owner/lease is mandatory: `trg_crawl_tasks_settled_release_guard`
    // aborts a settle that keeps the capability alive.
    let affected = sqlx::query(
        "UPDATE crawl_tasks \
         SET status = ?1, claim_token = NULL, owner_node_id = NULL, lease_expires_at = NULL, \
             http_status = ?2, content_hash = COALESCE(?3, content_hash), \
             etag = COALESCE(?4, etag), last_modified = COALESCE(?5, last_modified), \
             error_code = ?6, error_detail = ?7, completed_at = ?8, updated_at = ?8 \
         WHERE task_id = ?9 AND claim_token = ?10 AND status = 'in_progress'",
    )
    .bind(status)
    .bind(http_status.map(|v| v as i64))
    .bind(content_hash)
    .bind(etag)
    .bind(last_modified)
    .bind(error_code)
    .bind(error_detail)
    .bind(now)
    .bind(task_id.as_str())
    .bind(claim_token)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(CrawlError::StaleClaim(task_id.to_string()));
    }
    Ok(())
}

async fn requeue(
    tx: &mut sqlx::SqliteConnection,
    task_id: &CrawlTaskId,
    claim_token: &str,
    now: TimestampMs,
    error_code: &str,
    error_detail: &str,
) -> Result<(), CrawlError> {
    let affected = sqlx::query(
        "UPDATE crawl_tasks \
         SET status = 'pending', claim_token = NULL, owner_node_id = NULL, \
             claimed_at = NULL, lease_expires_at = NULL, \
             error_code = ?1, error_detail = ?2, updated_at = ?3 \
         WHERE task_id = ?4 AND claim_token = ?5 AND status = 'in_progress'",
    )
    .bind(error_code)
    .bind(error_detail)
    .bind(now)
    .bind(task_id.as_str())
    .bind(claim_token)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(CrawlError::StaleClaim(task_id.to_string()));
    }
    Ok(())
}

/// Return expired leases to the queue. This is the whole failure-detection
/// story: a worker that died, hung, or lost the network simply stops renewing.
/// Returns how many tasks were recovered.
pub async fn reap_expired(pool: &SqlitePool) -> Result<u64, CrawlError> {
    reap_expired_at(pool, now_ms()).await
}

/// [`reap_expired`] with an explicit clock, so tests can advance time instead
/// of sleeping or hand-editing a lease (which the in-progress guard forbids).
pub async fn reap_expired_at(pool: &SqlitePool, now: TimestampMs) -> Result<u64, CrawlError> {
    // Poison URLs are parked instead of looping forever.
    let failed = sqlx::query(
        "UPDATE crawl_tasks \
         SET status = 'failed', claim_token = NULL, owner_node_id = NULL, \
             lease_expires_at = NULL, error_code = 'lease_expired', \
             error_detail = 'worker lease expired and the attempt budget is exhausted', \
             completed_at = ?1, updated_at = ?1 \
         WHERE status = 'in_progress' AND lease_expires_at < ?1 AND attempt_count >= ?2",
    )
    .bind(now)
    .bind(MAX_ATTEMPTS as i64)
    .execute(pool)
    .await?
    .rows_affected();

    let requeued = sqlx::query(
        "UPDATE crawl_tasks \
         SET status = 'pending', claim_token = NULL, owner_node_id = NULL, \
             claimed_at = NULL, lease_expires_at = NULL, \
             error_code = 'lease_expired', updated_at = ?1 \
         WHERE status = 'in_progress' AND lease_expires_at < ?1 AND attempt_count < ?2",
    )
    .bind(now)
    .bind(MAX_ATTEMPTS as i64)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(failed + requeued)
}

pub async fn progress(pool: &SqlitePool, job_id: &CrawlJobId) -> Result<JobProgress, CrawlError> {
    let rows =
        sqlx::query("SELECT status, COUNT(*) AS n FROM crawl_tasks WHERE job_id = ? GROUP BY status")
            .bind(job_id.as_str())
            .fetch_all(pool)
            .await?;
    let mut out = JobProgress::default();
    for row in &rows {
        let status: String = row.try_get("status")?;
        let n = row.try_get::<i64, _>("n")? as u64;
        match status.as_str() {
            "pending" => out.pending = n,
            "in_progress" => out.in_progress = n,
            "done" => out.done = n,
            "failed" => out.failed = n,
            "skipped" => out.skipped = n,
            _ => {}
        }
    }
    Ok(out)
}

pub async fn list_tasks(
    pool: &SqlitePool,
    job_id: &CrawlJobId,
    status: Option<TaskStatus>,
    limit: u32,
) -> Result<Vec<CrawlTask>, CrawlError> {
    let filter = if status.is_some() { " AND status = ?2" } else { "" };
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM crawl_tasks WHERE job_id = ?1{filter} \
         ORDER BY updated_at DESC LIMIT {limit}"
    );
    let mut query = sqlx::query(&sql).bind(job_id.as_str());
    if let Some(status) = status {
        query = query.bind(status.as_str());
    }
    let rows = query.fetch_all(pool).await?;
    rows.iter().map(row_to_task).collect()
}

/// Return every parked task to the queue, clearing the attempt budget so a
/// fixed scope or restored credential gets a genuine retry rather than one
/// last attempt.
pub async fn requeue_failed(pool: &SqlitePool, job_id: &CrawlJobId) -> Result<u64, CrawlError> {
    let affected = sqlx::query(
        "UPDATE crawl_tasks \
         SET status = 'pending', attempt_count = 0, claim_token = NULL, owner_node_id = NULL, \
             claimed_at = NULL, lease_expires_at = NULL, error_code = NULL, error_detail = NULL, \
             updated_at = ?1 \
         WHERE job_id = ?2 AND status = 'failed'",
    )
    .bind(now_ms())
    .bind(job_id.as_str())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

pub async fn get_task(pool: &SqlitePool, task_id: &CrawlTaskId) -> Result<CrawlTask, CrawlError> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM crawl_tasks WHERE task_id = ?");
    let row = sqlx::query(&sql)
        .bind(task_id.as_str())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CrawlError::TaskNotFound(task_id.to_string()))?;
    row_to_task(&row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::UserId;
    use nomifun_db::init_database_memory;

    async fn seed_job(pool: &SqlitePool, max_urls: u32) -> CrawlJobId {
        let job_id = CrawlJobId::new();
        let user_id = UserId::new();
        let now = now_ms();
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
        sqlx::query(
            "INSERT INTO crawl_jobs \
             (job_id, user_id, name, seeds, max_urls, status, created_at, updated_at) \
             VALUES (?, ?, 'test', '[]', ?, 'running', ?, ?)",
        )
        .bind(job_id.as_str())
        .bind(user_id.as_str())
        .bind(max_urls as i64)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("job");
        job_id
    }

    /// Shortest lease the in-progress guard accepts (`lease > claimed_at`).
    /// Combined with [`reap_expired_at`] this expires a claim deterministically.
    const EXPIRING_LEASE_MS: i64 = 1;
    const LONG_LEASE_MS: i64 = 600_000;

    fn url(n: u32, host: &str) -> DiscoveredUrl {
        DiscoveredUrl {
            url: format!("https://{host}/p{n}"),
            fingerprint: format!("{:064x}", u128::from(n) + host.len() as u128 * 1_000_000),
            host: host.to_string(),
            depth: 0,
        }
    }

    #[tokio::test]
    async fn enqueue_dedups_on_fingerprint() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        assert!(enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap());
        assert!(!enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap());
        assert_eq!(progress(db.pool(), &job).await.unwrap().pending, 1);
    }

    #[tokio::test]
    async fn claim_is_exclusive() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();

        let first = claim_next(db.pool(), &job, "node-a", 4, LEASE_MS).await.unwrap();
        let second = claim_next(db.pool(), &job, "node-b", 4, LEASE_MS).await.unwrap();
        assert!(first.is_some(), "first worker should win the only task");
        assert!(second.is_none(), "second worker must not get the same task");

        let claimed = first.unwrap();
        assert_eq!(claimed.task.status, TaskStatus::InProgress);
        assert_eq!(claimed.task.attempt_count, 1);
        assert_eq!(claimed.task.claim_generation, 1);
        assert_eq!(claimed.claim_token.len(), 64);
    }

    #[tokio::test]
    async fn per_host_concurrency_is_enforced_by_the_allocator() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        for n in 0..4 {
            enqueue(db.pool(), &job, None, &url(n, "a.com"), 0).await.unwrap();
        }
        enqueue(db.pool(), &job, None, &url(9, "b.com"), 0).await.unwrap();

        let a = claim_next(db.pool(), &job, "n1", 2, LEASE_MS).await.unwrap().unwrap();
        let b = claim_next(db.pool(), &job, "n2", 2, LEASE_MS).await.unwrap().unwrap();
        assert_eq!(a.task.host, "a.com");
        assert_eq!(b.task.host, "a.com");

        // a.com is now at its cap of 2, so the only claimable task is on b.com.
        let c = claim_next(db.pool(), &job, "n3", 2, LEASE_MS).await.unwrap().unwrap();
        assert_eq!(c.task.host, "b.com");
        assert!(claim_next(db.pool(), &job, "n4", 2, LEASE_MS).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn higher_priority_and_shallower_depth_win() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        let mut deep = url(1, "a.com");
        deep.depth = 5;
        enqueue(db.pool(), &job, None, &deep, 0).await.unwrap();
        enqueue(db.pool(), &job, None, &url(2, "b.com"), 0).await.unwrap();
        enqueue(db.pool(), &job, None, &url(3, "c.com"), 10).await.unwrap();

        let first = claim_next(db.pool(), &job, "n", 4, LEASE_MS).await.unwrap().unwrap();
        assert_eq!(first.task.priority, 10);
        let second = claim_next(db.pool(), &job, "n", 4, LEASE_MS).await.unwrap().unwrap();
        assert_eq!(second.task.depth, 0);
    }

    #[tokio::test]
    async fn stale_token_cannot_settle_or_renew() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        let claimed = claim_next(db.pool(), &job, "n1", 4, EXPIRING_LEASE_MS)
            .await
            .unwrap()
            .unwrap();

        reap_expired_at(db.pool(), now_ms() + 1_000).await.unwrap();
        let retaken = claim_next(db.pool(), &job, "n2", 4, LEASE_MS).await.unwrap().unwrap();
        assert_eq!(retaken.task.task_id, claimed.task.task_id);
        assert_eq!(retaken.task.attempt_count, 2);
        assert_ne!(retaken.claim_token, claimed.claim_token);

        // The zombie wakes up holding generation 1's token.
        let renew =
            renew_lease(db.pool(), &claimed.task.task_id, &claimed.claim_token, LEASE_MS).await;
        assert!(matches!(renew, Err(CrawlError::StaleClaim(_))));

        let submit_result = submit(
            db.pool(),
            &claimed.task.task_id,
            &claimed.claim_token,
            TaskOutcome::Fetched {
                http_status: 200,
                content_hash: "a".repeat(64),
                etag: None,
                last_modified: None,
                discovered: vec![url(50, "a.com")],
            },
            100,
        )
        .await;
        assert!(matches!(submit_result, Err(CrawlError::StaleClaim(_))));

        // The zombie neither settled the task nor injected its discovered URL.
        let task = get_task(db.pool(), &claimed.task.task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(progress(db.pool(), &job).await.unwrap().total(), 1);
    }

    #[tokio::test]
    async fn successful_submit_settles_and_enqueues_children() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        let claimed = claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().unwrap();

        submit(
            db.pool(),
            &claimed.task.task_id,
            &claimed.claim_token,
            TaskOutcome::Fetched {
                http_status: 200,
                content_hash: "b".repeat(64),
                etag: Some("\"v1\"".into()),
                last_modified: None,
                discovered: vec![url(2, "a.com"), url(3, "a.com")],
            },
            100,
        )
        .await
        .unwrap();

        let task = get_task(db.pool(), &claimed.task.task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert_eq!(task.http_status, Some(200));
        assert_eq!(task.etag.as_deref(), Some("\"v1\""));
        assert!(task.owner_node_id.is_none(), "settled task must release its owner");
        assert!(task.lease_expires_at.is_none(), "settled task must release its lease");

        let p = progress(db.pool(), &job).await.unwrap();
        assert_eq!((p.done, p.pending), (1, 2));
    }

    #[tokio::test]
    async fn max_urls_caps_discovery() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 3).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        let claimed = claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().unwrap();

        submit(
            db.pool(),
            &claimed.task.task_id,
            &claimed.claim_token,
            TaskOutcome::Fetched {
                http_status: 200,
                content_hash: "c".repeat(64),
                etag: None,
                last_modified: None,
                discovered: (10..20).map(|n| url(n, "a.com")).collect(),
            },
            3,
        )
        .await
        .unwrap();

        assert_eq!(progress(db.pool(), &job).await.unwrap().total(), 3);
    }

    #[tokio::test]
    async fn retryable_failure_requeues_until_the_budget_runs_out() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();

        for attempt in 1..=MAX_ATTEMPTS {
            let claimed = claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().unwrap();
            assert_eq!(claimed.task.attempt_count, attempt);
            submit(
                db.pool(),
                &claimed.task.task_id,
                &claimed.claim_token,
                TaskOutcome::Failed {
                    error_code: "timeout".into(),
                    error_detail: "slow host".into(),
                    retryable: true,
                },
                100,
            )
            .await
            .unwrap();
        }

        let p = progress(db.pool(), &job).await.unwrap();
        assert_eq!(p.failed, 1, "attempt budget exhausted parks the task");
        assert!(claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_retryable_failure_parks_immediately() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        let claimed = claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().unwrap();

        submit(
            db.pool(),
            &claimed.task.task_id,
            &claimed.claim_token,
            TaskOutcome::Failed {
                error_code: "http_404".into(),
                error_detail: "not found".into(),
                retryable: false,
            },
            100,
        )
        .await
        .unwrap();

        assert_eq!(progress(db.pool(), &job).await.unwrap().failed, 1);
    }

    #[tokio::test]
    async fn reap_returns_only_expired_leases() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        enqueue(db.pool(), &job, None, &url(2, "b.com"), 0).await.unwrap();
        let live = claim_next(db.pool(), &job, "n1", 4, LONG_LEASE_MS).await.unwrap().unwrap();
        let dead = claim_next(db.pool(), &job, "n2", 4, EXPIRING_LEASE_MS)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(reap_expired_at(db.pool(), now_ms() + 1_000).await.unwrap(), 1);

        assert_eq!(
            get_task(db.pool(), &live.task.task_id).await.unwrap().status,
            TaskStatus::InProgress
        );
        assert_eq!(
            get_task(db.pool(), &dead.task.task_id).await.unwrap().status,
            TaskStatus::Pending
        );
    }

    #[tokio::test]
    async fn terminal_status_is_immutable() {
        let db = init_database_memory().await.unwrap();
        let job = seed_job(db.pool(), 100).await;
        enqueue(db.pool(), &job, None, &url(1, "a.com"), 0).await.unwrap();
        let claimed = claim_next(db.pool(), &job, "n1", 4, LEASE_MS).await.unwrap().unwrap();
        submit(
            db.pool(),
            &claimed.task.task_id,
            &claimed.claim_token,
            TaskOutcome::Unchanged { http_status: 304 },
            100,
        )
        .await
        .unwrap();

        let reopen = sqlx::query("UPDATE crawl_tasks SET status = 'pending' WHERE task_id = ?")
            .bind(claimed.task.task_id.as_str())
            .execute(db.pool())
            .await;
        assert!(reopen.is_err(), "the DB itself must refuse to reopen a done task");
    }
}
