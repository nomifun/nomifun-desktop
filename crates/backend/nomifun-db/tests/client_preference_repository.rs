//! Black-box integration tests for IClientPreferenceRepository.
//!
//! Tests exercise the public trait interface against an in-memory SQLite database.

use std::sync::Arc;

use nomifun_db::{IClientPreferenceRepository, SqliteClientPreferenceRepository, init_database_memory};

async fn repo() -> Arc<dyn IClientPreferenceRepository> {
    let db = init_database_memory().await.unwrap();
    Arc::new(SqliteClientPreferenceRepository::new(db.pool().clone()))
}

// -- Empty state --

#[tokio::test]
async fn get_all_returns_empty_when_no_preferences() {
    let r = repo().await;
    assert!(r.get_all().await.unwrap().is_empty());
}

// -- Upsert and retrieval --

#[tokio::test]
async fn upsert_then_get_all_returns_inserted_entries() {
    let r = repo().await;
    r.upsert_batch(&[("theme", "\"dark\""), ("companion.size", "360")])
        .await
        .unwrap();

    let prefs = r.get_all().await.unwrap();
    assert_eq!(prefs.len(), 2);

    let keys: Vec<&str> = prefs.iter().map(|p| p.key.as_str()).collect();
    assert!(keys.contains(&"theme"));
    assert!(keys.contains(&"companion.size"));
}

#[tokio::test]
async fn upsert_overwrites_existing_key() {
    let r = repo().await;
    r.upsert_batch(&[("k", "v1")]).await.unwrap();
    r.upsert_batch(&[("k", "v2")]).await.unwrap();

    let prefs = r.get_all().await.unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].value, "v2");
}

#[tokio::test]
async fn upsert_empty_batch_is_noop() {
    let r = repo().await;
    r.upsert_batch(&[]).await.unwrap();
    assert!(r.get_all().await.unwrap().is_empty());
}

// -- Filtered retrieval --

#[tokio::test]
async fn get_by_keys_returns_only_matching() {
    let r = repo().await;
    r.upsert_batch(&[("a", "1"), ("b", "2"), ("c", "3")]).await.unwrap();

    let prefs = r.get_by_keys(&["a", "c"]).await.unwrap();
    assert_eq!(prefs.len(), 2);

    let keys: Vec<&str> = prefs.iter().map(|p| p.key.as_str()).collect();
    assert!(keys.contains(&"a"));
    assert!(keys.contains(&"c"));
}

#[tokio::test]
async fn get_by_keys_omits_nonexistent() {
    let r = repo().await;
    r.upsert_batch(&[("x", "1")]).await.unwrap();

    let prefs = r.get_by_keys(&["x", "ghost"]).await.unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].key, "x");
}

#[tokio::test]
async fn get_by_keys_empty_input_returns_empty() {
    let r = repo().await;
    r.upsert_batch(&[("x", "1")]).await.unwrap();

    let prefs = r.get_by_keys(&[]).await.unwrap();
    assert!(prefs.is_empty());
}

// -- Deletion --

#[tokio::test]
async fn delete_keys_removes_specified_entries() {
    let r = repo().await;
    r.upsert_batch(&[("a", "1"), ("b", "2"), ("c", "3")]).await.unwrap();

    r.delete_keys(&["a", "c"]).await.unwrap();

    let prefs = r.get_all().await.unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].key, "b");
}

#[tokio::test]
async fn delete_nonexistent_keys_is_noop() {
    let r = repo().await;
    r.upsert_batch(&[("x", "1")]).await.unwrap();
    r.delete_keys(&["ghost"]).await.unwrap();

    assert_eq!(r.get_all().await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_empty_keys_is_noop() {
    let r = repo().await;
    r.upsert_batch(&[("x", "1")]).await.unwrap();
    r.delete_keys(&[]).await.unwrap();

    assert_eq!(r.get_all().await.unwrap().len(), 1);
}

// -- Value types --

#[tokio::test]
async fn stores_boolean_number_string_json_values() {
    let r = repo().await;
    r.upsert_batch(&[
        ("bool_key", "true"),
        ("num_key", "42"),
        ("str_key", "\"hello\""),
        ("null_key", "null"),
    ])
    .await
    .unwrap();

    let prefs = r.get_all().await.unwrap();
    assert_eq!(prefs.len(), 4);

    let find = |k: &str| prefs.iter().find(|p| p.key == k).unwrap().value.as_str();
    assert_eq!(find("bool_key"), "true");
    assert_eq!(find("num_key"), "42");
    assert_eq!(find("str_key"), "\"hello\"");
    assert_eq!(find("null_key"), "null");
}
