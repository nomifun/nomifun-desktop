use super::*;
use crate::DbError;
use nomifun_common::AppError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::time::Duration;

#[tokio::test]
async fn contention_is_typed_but_invalid_queries_and_closed_pools_are_not_retryable() {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("availability.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_millis(10));
    let pool = SqlitePoolOptions::new().max_connections(2).connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE work (id INTEGER)").execute(&pool).await.unwrap();
    let mut writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("INSERT INTO work VALUES (1)").execute(&mut *writer).await.unwrap();
    let busy = sqlx::query("INSERT INTO work VALUES (2)").execute(&pool).await.unwrap_err();
    assert_eq!(database_unavailable_reason(&busy), Some(DatabaseUnavailableReason::Busy));
    let converted: AppError = DbError::Query(busy).into();
    assert!(matches!(converted, AppError::DatabaseUnavailable { reason: DatabaseUnavailableReason::Busy }));
    assert_eq!(converted.status_code().as_u16(), 503);
    assert_eq!(converted.error_code(), "DATABASE_UNAVAILABLE");
    assert_eq!(converted.error_details(), Some(serde_json::json!({"database_unavailable":"busy"})));
    writer.rollback().await.unwrap();
    let invalid = sqlx::query("SELECT definitely_missing FROM work").execute(&pool).await.unwrap_err();
    assert_eq!(database_unavailable_reason(&invalid), None);
    assert!(matches!(AppError::from(DbError::Query(invalid)), AppError::Internal(_)));
    assert_eq!(database_unavailable_reason(&sqlx::Error::PoolClosed), None);
    assert_eq!(database_unavailable_reason(&sqlx::Error::Protocol("pool timed out while waiting for an open connection".into())), None);
    pool.close().await;
}

#[tokio::test]
async fn pool_exhaustion_preserves_its_reason_without_authorizing_write_replay() {
    let pool = SqlitePoolOptions::new().max_connections(1)
        .acquire_timeout(Duration::from_millis(10))
        .connect("sqlite::memory:").await.unwrap();
    let held = pool.acquire().await.unwrap();
    let error = pool.acquire().await.unwrap_err();
    assert_eq!(database_unavailable_reason(&error), Some(DatabaseUnavailableReason::PoolTimedOut));
    assert!(matches!(AppError::from(DbError::Query(error)), AppError::DatabaseUnavailable { reason: DatabaseUnavailableReason::PoolTimedOut }));
    drop(held);
    pool.close().await;
}
