//! `crawl_jobs` persistence.

use nomifun_common::{CrawlJobId, UserId, now_ms};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::error::CrawlError;
use crate::model::{CrawlJob, CrawlScope, CrawlSink, JobStatus, RenderMode};

const JOB_COLUMNS: &str = "job_id, user_id, name, seeds, scope, max_depth, max_urls, \
     render_mode, concurrency, per_host_concurrency, delay_ms, respect_robots, user_agent, \
     sink, status, error_detail, started_at, finished_at, created_at, updated_at";

pub async fn create_job(pool: &SqlitePool, job: &CrawlJob) -> Result<(), CrawlError> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO crawl_jobs \
         (job_id, user_id, name, seeds, scope, max_depth, max_urls, render_mode, concurrency, \
          per_host_concurrency, delay_ms, respect_robots, user_agent, sink, status, \
          created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job.job_id.as_str())
    .bind(job.user_id.as_str())
    .bind(&job.name)
    .bind(serde_json::to_string(&job.seeds).unwrap_or_else(|_| "[]".into()))
    .bind(serde_json::to_string(&job.scope).unwrap_or_else(|_| "{}".into()))
    .bind(job.max_depth as i64)
    .bind(job.max_urls as i64)
    .bind(job.render_mode.as_str())
    .bind(job.concurrency as i64)
    .bind(job.per_host_concurrency as i64)
    .bind(job.delay_ms as i64)
    .bind(i64::from(job.respect_robots))
    .bind(job.user_agent.as_deref())
    .bind(serde_json::to_string(&job.sink).unwrap_or_else(|_| "{}".into()))
    .bind(job.status.as_str())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_job(pool: &SqlitePool, job_id: &CrawlJobId) -> Result<CrawlJob, CrawlError> {
    let sql = format!("SELECT {JOB_COLUMNS} FROM crawl_jobs WHERE job_id = ?");
    sqlx::query(&sql)
        .bind(job_id.as_str())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CrawlError::JobNotFound(job_id.to_string()))
        .and_then(|row| row_to_job(&row))
}

pub async fn list_jobs(pool: &SqlitePool, user_id: &UserId) -> Result<Vec<CrawlJob>, CrawlError> {
    let sql = format!(
        "SELECT {JOB_COLUMNS} FROM crawl_jobs WHERE user_id = ? ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql).bind(user_id.as_str()).fetch_all(pool).await?;
    rows.iter().map(row_to_job).collect()
}

pub async fn start_job(pool: &SqlitePool, job_id: &CrawlJobId) -> Result<(), CrawlError> {
    let now = now_ms();
    sqlx::query(
        "UPDATE crawl_jobs SET status = 'running', started_at = COALESCE(started_at, ?1), \
         finished_at = NULL, error_detail = NULL, updated_at = ?1 WHERE job_id = ?2",
    )
    .bind(now)
    .bind(job_id.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_job(
    pool: &SqlitePool,
    job_id: &CrawlJobId,
    status: JobStatus,
    error_detail: Option<&str>,
) -> Result<(), CrawlError> {
    let now = now_ms();
    sqlx::query(
        "UPDATE crawl_jobs SET status = ?1, finished_at = ?2, error_detail = ?3, updated_at = ?2 \
         WHERE job_id = ?4",
    )
    .bind(status.as_str())
    .bind(now)
    .bind(error_detail)
    .bind(job_id.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_status(
    pool: &SqlitePool,
    job_id: &CrawlJobId,
    status: JobStatus,
) -> Result<(), CrawlError> {
    sqlx::query("UPDATE crawl_jobs SET status = ?1, updated_at = ?2 WHERE job_id = ?3")
        .bind(status.as_str())
        .bind(now_ms())
        .bind(job_id.as_str())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_job(pool: &SqlitePool, job_id: &CrawlJobId) -> Result<(), CrawlError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM crawl_tasks WHERE job_id = ?")
        .bind(job_id.as_str())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM crawl_jobs WHERE job_id = ?")
        .bind(job_id.as_str())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn row_to_job(row: &SqliteRow) -> Result<CrawlJob, CrawlError> {
    let job_id: String = row.try_get("job_id")?;
    let user_id: String = row.try_get("user_id")?;
    let seeds: String = row.try_get("seeds")?;
    let scope: String = row.try_get("scope")?;
    let sink: String = row.try_get("sink")?;
    let render_mode: String = row.try_get("render_mode")?;
    let status: String = row.try_get("status")?;
    let respect_robots: i64 = row.try_get("respect_robots")?;

    Ok(CrawlJob {
        job_id: CrawlJobId::parse(job_id).map_err(|e| CrawlError::JobNotFound(e.to_string()))?,
        user_id: UserId::parse(user_id).map_err(|e| CrawlError::JobNotFound(e.to_string()))?,
        name: row.try_get("name")?,
        seeds: serde_json::from_str(&seeds).unwrap_or_default(),
        scope: serde_json::from_str::<CrawlScope>(&scope).unwrap_or_default(),
        max_depth: row.try_get::<i64, _>("max_depth")? as u32,
        max_urls: row.try_get::<i64, _>("max_urls")? as u32,
        render_mode: RenderMode::parse(&render_mode).unwrap_or_default(),
        concurrency: row.try_get::<i64, _>("concurrency")? as u32,
        per_host_concurrency: row.try_get::<i64, _>("per_host_concurrency")? as u32,
        delay_ms: row.try_get::<i64, _>("delay_ms")? as u64,
        respect_robots: respect_robots != 0,
        user_agent: row.try_get("user_agent")?,
        sink: serde_json::from_str::<CrawlSink>(&sink).unwrap_or_default(),
        status: JobStatus::parse(&status)
            .ok_or_else(|| CrawlError::JobNotFound(format!("unknown status {status}")))?,
        error_detail: row.try_get("error_detail")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}
