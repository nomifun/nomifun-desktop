//! Real file-backed WAL contention, with no browser, network or user dataset.
use super::*;
use nomifun_agent_contracts::{AgentPresetRevisionPayload, ResolvedSnapshotContent};
use sqlx::Connection;
use std::time::Duration;

const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PRESET: &str = "0190f5fe-7c00-7a00-8000-000000000010";

async fn remove_fixture(root: tempfile::TempDir) {
    // SQLx pool closure may precede Windows releasing its last SQLite handle.
    // Retry only sharing violations on this owned fixture directory; never
    // retry the transaction or suppress persistent cleanup failures.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match std::fs::remove_dir_all(root.path()) {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error)
                if cfg!(windows)
                    && error.raw_os_error() == Some(32)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("fixture cleanup failed: {error}"),
        }
    }
    drop(root);
}

fn preset() -> StoredPreset {
    StoredPreset {
        session_only: false,
        preset: AgentPreset {
            preset_id: PRESET.into(),
            owner_user_id: Some(OWNER.into()),
            source: AgentPresetSource::User,
            display_name: "Before".into(),
            description: None,
            current_stable_revision: None,
        },
    }
}

fn candidate(instructions: &str) -> (AgentPresetRevision, ResolvedSnapshotEnvelope) {
    let payload: AgentPresetRevisionPayload = serde_json::from_value(serde_json::json!({
        "schema_version":"1.0.0", "model_route_refs":{}, "enabled_capabilities":[],
        "skill_bindings":[], "persona":"", "instructions":instructions
    }))
    .unwrap();
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: PRESET.into(),
            revision: 1,
            revision_digest: "a".repeat(64).into(),
        },
        payload,
        contribution_locks: vec![],
        created_by: OWNER.into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let content = ResolvedSnapshotContent {
        schema_version: "1.0.0".into(),
        resolver_version: "1.0.0".into(),
        preset_revision_ref: revision.reference.clone(),
        required_runtime_protocol_version: "1.0.0".into(),
        required_runtime_profile: nomifun_agent_contracts::RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: "a".repeat(64).into(),
        required_runtime_features: Default::default(),
        compiled_runtime_profile_digest: "a".repeat(64).into(),
        model_route_refs: Default::default(),
        chat_route_identity: None,
        enabled_capabilities: vec![],
        context_order: vec![],
        middleware_order: vec![],
        required_resource_kinds: Default::default(),
        capability_allowlist: Default::default(),
        skill_locks: vec![],
        mcp_tool_locks: vec![],
        resolved_role_providers: Default::default(),
        canonical_schema_manifest_digest: "a".repeat(64).into(),
        target_contribution_manifest_digest: "a".repeat(64).into(),
    };
    let snapshot = ResolvedSnapshotEnvelope {
        snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef {
            snapshot_id: "0190f5fe-7c00-7a00-8000-000000000013".into(),
            snapshot_digest: digest_payload(&content).unwrap(),
        },
        content,
        actor: nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".into(),
            principal_id: OWNER.into(),
        },
        scene: "test".into(),
        surface: "desktop".into(),
        audience: "user".into(),
        created_at_ms: 1,
        resolver_run_id: "0190f5fe-7c00-7a00-8000-000000000014".into(),
        availability_evidence_revision: "test".into(),
    };
    revision.validate().unwrap();
    snapshot.validate().unwrap();
    (revision, snapshot)
}

#[tokio::test]
async fn sqlite_revision_reads_preserve_canonical_contribution_lock_order() {
    let database = nomifun_db::init_database_memory_with_owner(
        CommonUserId::parse(OWNER.to_owned()).unwrap(),
    ).await.unwrap();
    let store = NomiCoreControlPlaneStore::new(database.pool().clone());
    store.insert_preset(preset()).await.unwrap();
    let (mut revision, mut snapshot) = candidate("Canonical locks");
    revision.contribution_locks = [
        ("a-source", "z-contribution"), ("z-source", "a-contribution"),
    ].into_iter().map(|(source, contribution)| nomifun_agent_contracts::ContributionLock {
        source_kind: nomifun_agent_contracts::ContributionSourceKind::PlatformBuiltin,
        source_identity: source.into(), mount_id: None, mcp_binding_id: None,
        contribution_id: contribution.into(), contract_digest: "a".repeat(64).into(),
    }).collect();
    revision.contribution_locks.sort();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    snapshot.content.preset_revision_ref = revision.reference.clone();
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
    store.append_revision(None, revision.clone(), snapshot.clone(), "Canonical locks".into(), None).await.unwrap();
    let sql_order: Vec<String> = sqlx::query_scalar(
        "SELECT contribution_id FROM agent_preset_contribution_locks WHERE revision_id = ? ORDER BY contribution_id",
    ).bind(revision.reference.revision_id()).fetch_all(database.pool()).await.unwrap();
    assert_eq!(sql_order, ["a-contribution", "z-contribution"]);
    let loaded = store.get_revision(&revision.reference).await.unwrap().unwrap();
    assert_eq!(loaded.contribution_locks[0].contribution_id.as_ref(), "z-contribution");
    assert_eq!(loaded, revision);
    assert_eq!(store.get_snapshot(&loaded.reference).await.unwrap(), Some(snapshot));
    database.close().await;
}

#[tokio::test]
async fn write_admission_precedes_validation_reads_and_still_allows_readers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("db.sqlite");
    let database = nomifun_db::init_database(&path).await.unwrap();
    let store = NomiCoreControlPlaneStore::new(database.pool().clone());
    store.insert_preset(preset()).await.unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .busy_timeout(Duration::ZERO);
    let mut other = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let transaction = store.begin_write_transaction().await.unwrap();
    // No SELECT or INSERT has run in this transaction yet. A deferred BEGIN
    // would allow the competing writer, exposing the read->write upgrade race.
    let writer_reserved = {
        let competing = other.begin_with("BEGIN IMMEDIATE").await;
        let reserved = matches!(&competing, Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("5"));
        if let Ok(competing) = competing {
            competing.rollback().await.unwrap();
        }
        reserved
    };
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets")
        .fetch_one(&mut other)
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    other.close().await.unwrap();
    drop(store);
    database.close().await;
    remove_fixture(root).await;
    assert!(
        writer_reserved,
        "write admission must already own SQLite's writer slot"
    );
    assert_eq!(count, 1, "WAL readers must remain available");
}

#[tokio::test]
async fn concurrent_revision_saves_have_one_winner_and_one_version_conflict() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database(&root.path().join("db.sqlite"))
        .await
        .unwrap();
    let store = NomiCoreControlPlaneStore::new(database.pool().clone());
    store.insert_preset(preset()).await.unwrap();
    let (first, first_snapshot) = candidate("First editor");
    let (second, second_snapshot) = candidate("Second editor");
    let results = tokio::join!(
        store.append_revision(None, first, first_snapshot, "First".into(), None),
        store.append_revision(None, second, second_snapshot, "Second".into(), None),
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions WHERE preset_id = ?")
            .bind(PRESET)
            .fetch_one(database.pool())
            .await
            .unwrap();
    drop(store);
    database.close().await;
    remove_fixture(root).await;
    let outcomes = [results.0, results.1];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    let error = outcomes.into_iter().find_map(Result::err).unwrap();
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn revision_save_waits_for_an_existing_writer_then_keeps_compare_and_swap() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("db.sqlite");
    let database = nomifun_db::init_database(&path).await.unwrap();
    let store = NomiCoreControlPlaneStore::new(database.pool().clone());
    store.insert_preset(preset()).await.unwrap();
    let mut writer = database.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "UPDATE agent_presets \
         SET display_json = json_set(display_json, '$.display_name', 'Other writer') \
         WHERE preset_id = ?",
    )
        .bind(PRESET)
        .execute(&mut *writer)
        .await
        .unwrap();
    let (revision, snapshot) = candidate("Saved content");
    let mut save =
        Box::pin(store.append_revision(None, revision.clone(), snapshot, "Saved".into(), None));
    // This is a bounded observation of the actual store future, not a sleep
    // added to production and not a retry of a possibly committed mutation.
    let early = tokio::time::timeout(Duration::from_millis(150), &mut save).await;
    writer.commit().await.unwrap();
    let waited = early.is_err();
    let saved = match early {
        Ok(result) => {
            drop(save);
            result
        }
        Err(_) => tokio::time::timeout(Duration::from_secs(5), save)
            .await
            .unwrap(),
    };
    if !waited || saved.is_err() {
        drop(store);
        database.close().await;
        remove_fixture(root).await;
        panic!(
            "save did not wait and complete after writer release: waited={waited}, result={saved:?}"
        );
    }
    let saved = saved.unwrap();
    let (stale_revision, stale_snapshot) = candidate("Must not overwrite");
    let stale = store
        .append_revision(None, stale_revision, stale_snapshot, "Stale".into(), None)
        .await;
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions WHERE preset_id = ?")
            .bind(PRESET)
            .fetch_one(database.pool())
            .await
            .unwrap();
    let name = store
        .get_preset(&PRESET.into())
        .await
        .unwrap()
        .unwrap()
        .preset
        .display_name;
    drop(store);
    database.close().await;
    remove_fixture(root).await;
    assert_eq!(
        saved.preset.current_stable_revision,
        Some(revision.reference)
    );
    assert_eq!(saved.preset.display_name, "Saved");
    assert_eq!(stale.unwrap_err().status(), StatusCode::CONFLICT);
    assert_eq!(rows, 1);
    assert_eq!(name, "Saved");
}
