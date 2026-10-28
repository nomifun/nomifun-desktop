use std::{borrow::Cow, path::Path};

use nomifun_agent_contracts::{
    AGENT_STORE_DATA_GENERATION, AGENT_STORE_MIGRATION_HEAD, AGENT_STORE_PROJECTION_SCHEMA_VERSION,
    agent_store_schema_manifest_payload, digest_payload, official_preset_seed_manifest_payload,
};
use nomifun_db::{
    MigrationLineage, init_database, inspect_migration_lineage, validate_current_migration_lineage,
    validate_known_migration_lineage_prefix,
};
use serde_json::json;
use sqlx::{SqlitePool, migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

static MIGRATOR: Migrator = sqlx::migrate!();
const NOMI: &str = "0190f5fe-7c00-7a00-8000-000000000114";

async fn previous_database(path: &Path) -> SqlitePool {
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(false),
    ).await.unwrap();
    let previous = Migrator { migrations: Cow::Owned(MIGRATOR.iter().take(4).cloned().collect()), ..Migrator::DEFAULT };
    previous.run(&pool).await.unwrap();
    // This fixture's schema predates migration 010, so its declared
    // lineage is the pre-010 manifest digest, not the current manifest.
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD))
        .bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref())
        .bind("181fba4223793ec7cfd298a8a3d026925c54b622172d1c962d5bf70816e1a019")
        .bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&pool).await.unwrap();
    assert_eq!(inspect_migration_lineage(&pool).await.unwrap(), MigrationLineage::KnownPrefix);
    assert!(!validate_known_migration_lineage_prefix(&pool).await.unwrap());
    assert!(validate_current_migration_lineage(&pool).await.is_err());
    pool
}

async fn insert_policy(pool: &SqlitePool, policy: Option<&str>) -> String {
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO agent_metadata(agent_id,name,agent_type,agent_source,behavior_policy,created_at,updated_at) VALUES(?,'Saved policy fixture','nomi','internal',?,10,10)")
        .bind(&id).bind(policy).execute(pool).await.unwrap();
    id
}

#[tokio::test]
async fn forward_005_retires_only_side_question_policy_and_preserves_schema_and_prior_receipts() {
    let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("policy.sqlite");
    let old = previous_database(&path).await;
    let false_policy = r#"{"supports_side_question":false,"self_identity_sticky":true,"session_load_via_meta_field":true}"#;
    let true_policy = r#"{"self_identity_sticky":false,"supports_side_question":true,"session_load_via_meta_field":true}"#;
    sqlx::query("UPDATE agent_metadata SET behavior_policy=?,name='Saved owner name',enabled=0 WHERE agent_id=?")
        .bind(false_policy).bind(NOMI).execute(&old).await.unwrap();
    let true_id = insert_policy(&old, Some(true_policy)).await;
    let null_id = insert_policy(&old, None).await;
    let empty_id = insert_policy(&old, Some("{}")).await;
    let absent_policy = "{ \"self_identity_sticky\": true }";
    let absent_id = insert_policy(&old, Some(absent_policy)).await;
    // Later product migrations may retire settings columns independently of
    // this Agent metadata migration. All other schema remains identical.
    let before_schema: Vec<(String, String)> = sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'system_settings' ORDER BY name")
        .fetch_all(&old).await.unwrap();
    let before_receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&old).await.unwrap();
    let before_manifest: (i64, String) = sqlx::query_as("SELECT migration_head,canonical_schema_manifest_digest FROM schema_metadata")
        .fetch_one(&old).await.unwrap();
    // Migration 005 changes only Agent metadata rows; every schema object must
    // remain identical across just that migration (later product migrations
    // legitimately change schema independently).
    Migrator { migrations: Cow::Owned(MIGRATOR.iter().take(5).cloned().collect()), ..Migrator::DEFAULT }
        .run(&old).await.unwrap();
    assert_eq!(sqlx::query_as::<_, (String, String)>("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'system_settings' ORDER BY name")
        .fetch_all(&old).await.unwrap(), before_schema, "metadata data retirement must not change the schema");
    old.close().await;

    for _ in 0..2 {
        let current = init_database(&path).await.unwrap();
        assert_eq!(inspect_migration_lineage(current.pool()).await.unwrap(), MigrationLineage::Current);
        validate_current_migration_lineage(current.pool()).await.unwrap();
        for (id, expected) in [
            (NOMI, json!({"self_identity_sticky":true,"session_load_via_meta_field":true})),
            (true_id.as_str(), json!({"self_identity_sticky":false,"session_load_via_meta_field":true})),
        ] {
            let policy: String = sqlx::query_scalar("SELECT behavior_policy FROM agent_metadata WHERE agent_id=?")
                .bind(id).fetch_one(current.pool()).await.unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&policy).unwrap(), expected);
        }
        for (id, expected) in [(&null_id, None), (&empty_id, Some("{}")), (&absent_id, Some(absent_policy))] {
            let policy: Option<String> = sqlx::query_scalar("SELECT behavior_policy FROM agent_metadata WHERE agent_id=?")
                .bind(id).fetch_one(current.pool()).await.unwrap();
            assert_eq!(policy.as_deref(), expected, "unaffected policy text must remain exact");
        }
        let saved: (String, bool) = sqlx::query_as("SELECT name,enabled FROM agent_metadata WHERE agent_id=?")
            .bind(NOMI).fetch_one(current.pool()).await.unwrap();
        assert_eq!(saved, ("Saved owner name".into(), false));
        let manifest: (i64, String) = sqlx::query_as("SELECT migration_head,canonical_schema_manifest_digest FROM schema_metadata")
            .fetch_one(current.pool()).await.unwrap();
        assert_eq!(manifest.0, before_manifest.0);
        assert_eq!(manifest.1, digest_payload(&agent_store_schema_manifest_payload()).unwrap().as_ref().to_string());
        assert_eq!(manifest.0, 1, "product metadata migration does not advance the canonical Agent schema head");
        let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(current.pool()).await.unwrap();
        assert_eq!(&receipts[..4], before_receipts.as_slice());
        assert_eq!(receipts, MIGRATOR.iter().map(|migration| (migration.version, migration.checksum.to_vec())).collect::<Vec<_>>());
        current.close().await;
    }
}

#[tokio::test]
async fn forward_005_rejects_invalid_policy_without_overwriting_any_metadata_or_receipt() {
    for invalid in [
        "{invalid json", "[]", "null",
        r#"{"supports_side_question":null}"#,
        r#"{"supports_side_question":"false"}"#,
        r#"{"supports_side_question":[]}"#,
        r#"{"supports_side_question":false,"self_identity_sticky":"true"}"#,
        r#"{"supports_side_question":false,"session_load_via_meta_field":null}"#,
        r#"{"supports_side_question":false,"unknown_policy":true}"#,
        r#"{"supports_side_question":false,"self_identity_sticky":true,"self_identity_sticky":false}"#,
        r#"{"supports_side_question":false,"supports_side_question":true}"#,
    ] {
        let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("invalid-policy.sqlite");
        let old = previous_database(&path).await;
        let valid = r#"{"supports_side_question":false,"self_identity_sticky":true}"#;
        sqlx::query("UPDATE agent_metadata SET behavior_policy=? WHERE agent_id=?")
            .bind(valid).bind(NOMI).execute(&old).await.unwrap();
        let invalid_id = insert_policy(&old, Some(invalid)).await;
        let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&old).await.unwrap();
        let error = init_database(&path).await.unwrap_err();
        assert!(error.to_string().contains("CHECK constraint failed"), "invalid policy must fail migration explicitly: {error}");
        for (id, expected) in [(NOMI, valid), (invalid_id.as_str(), invalid)] {
            let policy: String = sqlx::query_scalar("SELECT behavior_policy FROM agent_metadata WHERE agent_id=?")
                .bind(id).fetch_one(&old).await.unwrap();
            assert_eq!(policy, expected, "failed migration must preserve all source policy text");
        }
        assert_eq!(sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&old).await.unwrap(), receipts);
        assert_eq!(inspect_migration_lineage(&old).await.unwrap(), MigrationLineage::KnownPrefix);
        old.close().await;
    }
}
