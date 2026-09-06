use std::collections::BTreeSet;

use nomifun_db::{
    DbError, IJavaScriptRuntimeSelectionRepository, SaveJavaScriptRuntimeSelectionParams,
    SqliteJavaScriptRuntimeSelectionRepository, init_database,
};
use serde_json::json;

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
            pending_candidate: Some(pending.clone()),
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
            pending_candidate: None,
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
            pending_candidate: Some(pending),
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
            pending_candidate: None,
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
