//! Black-box integration tests for ISettingsRepository.
//!
//! Tests exercise the public trait interface against an in-memory SQLite database.

use std::sync::Arc;

use nomifun_db::{ISettingsRepository, SqliteSettingsRepository, init_database_memory};

async fn repo() -> Arc<dyn ISettingsRepository> {
    let db = init_database_memory().await.unwrap();
    Arc::new(SqliteSettingsRepository::new(db.pool().clone()))
}

// -- Get baseline default state --

#[tokio::test]
async fn get_settings_returns_v3_baseline_defaults() {
    let r = repo().await;
    let settings = r
        .get_settings()
        .await
        .unwrap()
        .expect("v3 baseline seeds the settings singleton");
    assert_eq!(settings.singleton_key, "system");
    assert_eq!(settings.language, "en-US");
    assert!(settings.notification_enabled);
    assert!(!settings.cron_notification_enabled);
    assert!(!settings.command_queue_enabled);
    assert!(!settings.save_upload_to_workspace);
}

// -- Upsert creates a row --

#[tokio::test]
async fn upsert_creates_settings_with_given_values() {
    let db = init_database_memory().await.unwrap();
    sqlx::query("DELETE FROM system_settings").execute(db.pool()).await.unwrap();
    let r = SqliteSettingsRepository::new(db.pool().clone());
    let s = r.upsert_settings(None, None, Some(true), Some(true), None).await.unwrap();

    assert_eq!(s.singleton_key, "system");
    assert!(s.id > 0);
    assert_eq!(s.language, "en-US");
    assert!(s.notification_enabled);
    assert!(s.cron_notification_enabled);
    assert!(s.command_queue_enabled);
    assert!(!s.save_upload_to_workspace);
    assert!(s.updated_at > 0);
}

// -- Upsert then get round-trip --

#[tokio::test]
async fn upsert_then_get_returns_consistent_data() {
    let r = repo().await;
    r.upsert_settings(Some("en-US"), Some(true), Some(false), Some(false), Some(true)).await.unwrap();

    let s = r.get_settings().await.unwrap().unwrap();
    assert_eq!(s.language, "en-US");
    assert!(s.notification_enabled);
    assert!(!s.cron_notification_enabled);
    assert!(!s.command_queue_enabled);
    assert!(s.save_upload_to_workspace);
}

// -- Upsert overwrites --

#[tokio::test]
async fn upsert_overwrites_previous_settings() {
    let r = repo().await;
    r.upsert_settings(Some("en-US"), Some(true), Some(false), Some(false), Some(false)).await.unwrap();
    r.upsert_settings(Some("zh-CN"), Some(false), Some(true), Some(true), Some(true)).await.unwrap();

    let s = r.get_settings().await.unwrap().unwrap();
    assert_eq!(s.language, "zh-CN");
    assert!(!s.notification_enabled);
    assert!(s.cron_notification_enabled);
    assert!(s.command_queue_enabled);
    assert!(s.save_upload_to_workspace);
}

#[tokio::test]
async fn partial_and_empty_updates_preserve_unspecified_fields() {
    let r = repo().await;
    let original = r.upsert_settings(Some("zh-CN"), Some(false), Some(true), Some(true), Some(true)).await.unwrap();
    let patched = r.upsert_settings(None, Some(true), None, Some(false), None).await.unwrap();
    assert_eq!(patched.id, original.id);
    assert_eq!(patched.language, "zh-CN");
    assert!(patched.notification_enabled);
    assert!(patched.cron_notification_enabled);
    assert!(!patched.command_queue_enabled);
    assert!(patched.save_upload_to_workspace);
    let unchanged = r.upsert_settings(None, None, None, None, None).await.unwrap();
    let mut expected = serde_json::to_value(patched).unwrap();
    expected["updated_at"] = serde_json::json!(unchanged.updated_at);
    assert_eq!(serde_json::to_value(unchanged).unwrap(), expected);
    assert_eq!(serde_json::to_value(r.get_settings().await.unwrap().unwrap()).unwrap(), expected);
}

// -- updated_at advances on each upsert --

#[tokio::test]
async fn upsert_advances_updated_at() {
    let r = repo().await;
    let first = r.upsert_settings(Some("en-US"), Some(true), Some(false), Some(false), Some(false)).await.unwrap();
    let second = r.upsert_settings(Some("en-US"), Some(true), Some(false), Some(false), Some(false)).await.unwrap();

    assert!(second.updated_at >= first.updated_at);
}
