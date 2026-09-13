use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn migrate_to(pool: &sqlx::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect::<std::collections::BTreeSet<_>>();
    for migration in MIGRATOR.iter() {
        if migration.version <= maximum_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

const OWNER_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000011";
const PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000012";
const NEUTRAL_PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000014";
const SNAPSHOT_BOUND_PRESET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000015";
const SNAPSHOT_BOUND_REMOTE_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000016";

#[tokio::test]
async fn migration_066_retires_old_resource_bound_presets_without_deleting_history() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 65).await;

    sqlx::query(
        "INSERT INTO users \
         (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-resource-boundary', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
         (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Old resource preset', 1, 1)",
    )
    .bind(PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    let old_payload = serde_json::json!({
        "schema_version": "1.0.0",
        "model_route_refs": {},
        "chat_route_records": {},
        "enabled_capabilities": [{
            "capability": {"id": "knowledge.search", "version": "1.0.0"},
            "action_allowlist": [],
            "resource_binding_refs": ["knowledge"]
        }],
                "skill_bindings": [],
        "resource_bindings": [{
            "binding_id": "knowledge",
            "resource_kind": "knowledge_base",
            "resource_id": "old-kb",
            "owner_id": OWNER_ID,
            "operations": ["read", "search"]
        }],
        "system_role_provider_overrides": {},
        "persona": "",
        "instructions": "",
        "starter_prompts": []
    });
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, payload_json, \
          revision_digest, created_by, created_at, reason, snapshot_json, \
          contribution_locks_json) \
         VALUES (?, ?, 1, '1.0.0', ?, ?, ?, 1, '', '{}', '[]')",
    )
    .bind(format!("{PRESET_ID}@1"))
    .bind(PRESET_ID)
    .bind(old_payload.to_string())
    .bind("a".repeat(64))
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_bindings \
         (target_kind, target_id, owner_user_id, agent_binding_json) \
         VALUES ('conversation', 'old-target', ?, ?)",
    )
    .bind(OWNER_ID)
    .bind(
        serde_json::json!({
            "preset_revision_ref": {"preset_id": PRESET_ID},
            "resolved_snapshot_ref": {"snapshot_id": "old", "snapshot_digest": "old"},
            "typed_resource_bindings": [],
            "binding_version": 1
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json, \
          nomi_snapshot_json, provenance_json, agent_binding_digest, \
          binding_version, created_at, updated_at) \
         VALUES ('0190f5fe-7c00-7a00-8abc-000000000013', ?, 'Old remote', ?, \
                 '{}', '{}', ?, 1, 1, 1)",
    )
    .bind(OWNER_ID)
    .bind(
        serde_json::json!({"preset_revision_ref": {"preset_id": PRESET_ID}})
            .to_string(),
    )
    .bind("b".repeat(64))
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 66).await;

    let latest: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(latest, 66);
    let retired: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(retired.is_some());
    let agent_bindings: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nomi_agent_bindings")
            .fetch_one(&pool)
            .await
            .unwrap();
    let remote_bindings: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM remote_bindings")
            .fetch_one(&pool)
            .await
            .unwrap();
    let revisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_preset_revisions WHERE preset_id = ?",
    )
    .bind(PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(agent_bindings, 0);
    assert_eq!(remote_bindings, 0);
    assert_eq!(revisions, 1);

    let columns = sqlx::query("PRAGMA table_info(nomi_agent_presets)")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(columns
        .iter()
        .any(|row| row.get::<String, _>("name") == "retired_at_ms"));
}

#[tokio::test]
async fn migration_066_preserves_resource_neutral_presets_and_bindings() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 65).await;

    sqlx::query(
        "INSERT INTO users \
         (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-resource-neutral', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
         (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Resource neutral preset', 1, 1)",
    )
    .bind(NEUTRAL_PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    let neutral_payload = serde_json::json!({
        "schema_version": "1.0.0",
        "model_route_refs": {},
        "chat_route_records": {},
        "enabled_capabilities": [{
            "capability": {"id": "knowledge.search", "version": "1.0.0"},
            "action_allowlist": []
        }],
                "skill_bindings": [],
        "system_role_provider_overrides": {},
        "persona": "",
        "instructions": "",
        "starter_prompts": []
    });
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, payload_json, \
          revision_digest, created_by, created_at, reason, snapshot_json, \
          contribution_locks_json) \
         VALUES (?, ?, 1, '1.0.0', ?, ?, ?, 1, '', '{}', '[]')",
    )
    .bind(format!("{NEUTRAL_PRESET_ID}@1"))
    .bind(NEUTRAL_PRESET_ID)
    .bind(neutral_payload.to_string())
    .bind("c".repeat(64))
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_bindings \
         (target_kind, target_id, owner_user_id, agent_binding_json) \
         VALUES ('conversation', 'neutral-target', ?, ?)",
    )
    .bind(OWNER_ID)
    .bind(
        serde_json::json!({
            "preset_revision_ref": {"preset_id": NEUTRAL_PRESET_ID},
            "resolved_snapshot_ref": {"snapshot_id": "neutral", "snapshot_digest": "neutral"},
            "typed_resource_bindings": [{
                "binding_id": "conversation-knowledge",
                "resource_kind": "knowledge_base",
                "resource_id": "selected-in-conversation",
                "owner_id": OWNER_ID,
                "operations": ["read", "search"]
            }],
            "binding_version": 1
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 66).await;

    let retired: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(NEUTRAL_PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retired, None);
    let agent_bindings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_bindings \
         WHERE json_extract(agent_binding_json, '$.preset_revision_ref.preset_id') = ?",
    )
    .bind(NEUTRAL_PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(agent_bindings, 1);
}

#[tokio::test]
async fn migration_066_retires_presets_with_legacy_snapshot_resources() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 65).await;

    sqlx::query(
        "INSERT INTO users \
         (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, 'agent-snapshot-boundary', 'hash', 1, 1)",
    )
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
         (preset_id, owner_user_id, source_kind, display_name, current_revision, created_at) \
         VALUES (?, ?, 'user', 'Snapshot resource preset', 1, 1)",
    )
    .bind(SNAPSHOT_BOUND_PRESET_ID)
    .bind(OWNER_ID)
    .execute(&pool)
    .await
    .unwrap();

    let payload = serde_json::json!({
        "schema_version": "1.0.0",
        "model_route_refs": {},
        "chat_route_records": {},
        "enabled_capabilities": [],
                "skill_bindings": [],
        "system_role_provider_overrides": {},
        "persona": "",
        "instructions": "",
        "starter_prompts": []
    });
    let legacy_snapshot = serde_json::json!({
        "snapshot_ref": {
            "snapshot_id": "legacy-snapshot",
            "snapshot_digest": "d".repeat(64)
        },
        "content": {
            "typed_resource_bindings": [{
                "binding_id": "old-workspace",
                "resource_kind": "workspace",
                "resource_id": "old-workspace-id"
            }]
        }
    });
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, payload_json, \
          revision_digest, created_by, created_at, reason, snapshot_json, \
          contribution_locks_json) \
         VALUES (?, ?, 1, '1.0.0', ?, ?, ?, 1, '', ?, '[]')",
    )
    .bind(format!("{SNAPSHOT_BOUND_PRESET_ID}@1"))
    .bind(SNAPSHOT_BOUND_PRESET_ID)
    .bind(payload.to_string())
    .bind("a".repeat(64))
    .bind(OWNER_ID)
    .bind(legacy_snapshot.to_string())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO nomi_agent_bindings \
         (target_kind, target_id, owner_user_id, agent_binding_json) \
         VALUES ('conversation', 'snapshot-target', ?, ?)",
    )
    .bind(OWNER_ID)
    .bind(
        serde_json::json!({
            "preset_revision_ref": {"preset_id": SNAPSHOT_BOUND_PRESET_ID}
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json, \
          nomi_snapshot_json, provenance_json, agent_binding_digest, \
          binding_version, created_at, updated_at) \
         VALUES (?, ?, 'Snapshot remote', ?, '{}', '{}', ?, 1, 1, 1)",
    )
    .bind(SNAPSHOT_BOUND_REMOTE_ID)
    .bind(OWNER_ID)
    .bind(
        serde_json::json!({
            "preset_revision_ref": {"preset_id": SNAPSHOT_BOUND_PRESET_ID}
        })
        .to_string(),
    )
    .bind("b".repeat(64))
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 66).await;

    let retired: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(SNAPSHOT_BOUND_PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(retired.is_some());
    let agent_bindings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_bindings \
         WHERE json_extract(agent_binding_json, '$.preset_revision_ref.preset_id') = ?",
    )
    .bind(SNAPSHOT_BOUND_PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    let remote_bindings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM remote_bindings \
         WHERE remote_binding_id = ?",
    )
    .bind(SNAPSHOT_BOUND_REMOTE_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    let history: String = sqlx::query_scalar(
        "SELECT snapshot_json FROM nomi_agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = 1",
    )
    .bind(SNAPSHOT_BOUND_PRESET_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(agent_bindings, 0);
    assert_eq!(remote_bindings, 0);
    assert_eq!(history, legacy_snapshot.to_string());
}
