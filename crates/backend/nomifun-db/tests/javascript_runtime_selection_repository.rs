use std::collections::BTreeSet;

use nomifun_db::{
    DbError, IJavaScriptRuntimeSelectionRepository, SaveJavaScriptRuntimeSelectionParams,
    SqliteJavaScriptRuntimeSelectionRepository, init_database,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

const RUNTIME_SELECTION_068: &str =
    include_str!("../migrations/068_javascript_runtime_selection.sql");
const RUNTIME_SELECTION_PATHS_071: &str =
    include_str!("../migrations/071_javascript_runtime_selection_paths.sql");

fn runtime(id: &str, executable_digest: &str) -> serde_json::Value {
    json!({
        "installation_id": id,
        "runtime_target": "windows-x64",
        "executable_digest": executable_digest,
        "node_version": "24.0.0"
    })
}

#[tokio::test]
async fn empty_selection_saves_with_revision_cas_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime-selection.db");
    let database = init_database(&path).await.unwrap();
    let repository =
        SqliteJavaScriptRuntimeSelectionRepository::new(database.pool().clone());
    assert_eq!(repository.load().await.unwrap().revision, 0);

    let selected = runtime("runtime-selected", &"a".repeat(64));
    let pending = runtime("runtime-candidate", &"b".repeat(64));
    let validation = json!({
        "candidate": pending.clone(),
        "participants": [],
        "decision": "commit_candidate"
    });
    let saved = repository
        .save_cas(&SaveJavaScriptRuntimeSelectionParams {
            expected_revision: 0,
            selected_runtime: Some(selected.clone()),
            selected_executable_path: Some(r"C:\node\selected.exe".into()),
            pending_candidate: Some(pending.clone()),
            pending_candidate_executable_path: Some(
                r"C:\node\candidate.exe".into(),
            ),
            validation_result: Some(validation.clone()),
            last_error_code: None,
            non_recommended_warning_acknowledged: BTreeSet::from([
                "runtime-selected".into(),
                "runtime-candidate".into(),
            ]),
            updated_at: 10,
        })
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.pending_candidate.as_ref(), Some(&pending));
    assert_eq!(saved.validation_result.as_ref(), Some(&validation));
    assert!(saved
        .non_recommended_warning_acknowledged
        .contains("runtime-candidate"));

    let stale = repository
        .save_cas(&SaveJavaScriptRuntimeSelectionParams {
            expected_revision: 0,
            selected_runtime: Some(selected),
            selected_executable_path: Some(r"C:\node\selected.exe".into()),
            pending_candidate: None,
            pending_candidate_executable_path: None,
            validation_result: None,
            last_error_code: Some("runtime_probe_failed".into()),
            non_recommended_warning_acknowledged: BTreeSet::new(),
            updated_at: 11,
        })
        .await
        .unwrap_err();
    assert!(matches!(stale, DbError::Conflict(message) if message.contains("revision CAS")));
    database.close().await;

    let reopened = init_database(&path).await.unwrap();
    let reloaded =
        SqliteJavaScriptRuntimeSelectionRepository::new(reopened.pool().clone())
            .load()
            .await
            .unwrap();
    assert_eq!(reloaded, saved);
}

#[tokio::test]
async fn validation_must_bind_exact_pending_candidate_and_singleton_rejects_second_row() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let repository =
        SqliteJavaScriptRuntimeSelectionRepository::new(database.pool().clone());
    let pending = runtime("runtime-candidate", &"b".repeat(64));
    let mismatched = repository
        .save_cas(&SaveJavaScriptRuntimeSelectionParams {
            expected_revision: 0,
            selected_runtime: None,
            selected_executable_path: None,
            pending_candidate: Some(pending),
            pending_candidate_executable_path: Some(
                r"C:\node\candidate.exe".into(),
            ),
            validation_result: Some(json!({
                "candidate": runtime("another-runtime", &"c".repeat(64))
            })),
            last_error_code: None,
            non_recommended_warning_acknowledged: BTreeSet::new(),
            updated_at: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(mismatched, DbError::Conflict(message) if message.contains("exact pending")));

    repository
        .save_cas(&SaveJavaScriptRuntimeSelectionParams {
            expected_revision: 0,
            selected_runtime: None,
            selected_executable_path: None,
            pending_candidate: None,
            pending_candidate_executable_path: None,
            validation_result: None,
            last_error_code: None,
            non_recommended_warning_acknowledged: BTreeSet::new(),
            updated_at: 2,
        })
        .await
        .unwrap();
    let second = sqlx::query(
        "INSERT INTO javascript_runtime_selection (
            singleton_key, non_recommended_warning_acknowledged_json, revision, updated_at
         ) VALUES ('another', '[]', 1, 2)",
    )
    .execute(database.pool())
    .await;
    assert!(second.is_err());

    let direct_invalid = sqlx::query(
        "UPDATE javascript_runtime_selection
         SET validation_result_json = '{\"candidate\":{}}',
             pending_candidate_json = NULL
         WHERE singleton_key = 'javascript_runtime_selection'",
    )
    .execute(database.pool())
    .await;
    assert!(direct_invalid.is_err());
}

#[tokio::test]
async fn migration_071_invalidates_unverifiable_068_runtime_fingerprints() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(RUNTIME_SELECTION_068)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO javascript_runtime_selection (
            singleton_key, selected_runtime_json, pending_candidate_json,
            validation_result_json, last_error_code,
            non_recommended_warning_acknowledged_json, revision, updated_at
         ) VALUES (?, ?, ?, ?, NULL, '[]', 7, 42)",
    )
    .bind("javascript_runtime_selection")
    .bind(runtime("runtime-selected", &"a".repeat(64)).to_string())
    .bind(runtime("runtime-candidate", &"b".repeat(64)).to_string())
    .bind(json!({
        "candidate": runtime("runtime-candidate", &"b".repeat(64))
    })
    .to_string())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(RUNTIME_SELECTION_PATHS_071)
        .execute(&pool)
        .await
        .unwrap();

    let reloaded = SqliteJavaScriptRuntimeSelectionRepository::new(pool)
        .load()
        .await
        .unwrap();
    assert!(reloaded.selected_runtime.is_none());
    assert!(reloaded.selected_executable_path.is_none());
    assert!(reloaded.pending_candidate.is_none());
    assert!(reloaded.pending_candidate_executable_path.is_none());
    assert!(reloaded.validation_result.is_none());
    assert_eq!(
        reloaded.last_error_code.as_deref(),
        Some("JAVASCRIPT_RUNTIME_RESELECTION_REQUIRED")
    );
    assert_eq!(reloaded.revision, 8);
}
