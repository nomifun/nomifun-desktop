use nomifun_agent_contracts::{agent_store_schema_manifest_payload, digest_payload};
use nomifun_db::{
    init_database_memory, validate_id_data_contract, validate_id_schema_contract,
};

const BASELINE: &str = include_str!("../migrations/001_canonical_baseline.sql");

const RETIRED_AGENT_TABLES: &[&str] = &[
    "conversations",
    "messages",
    "conversation_delivery_receipts",
    "conversation_runtime_events",
    "conversation_mcp_effects",
    "conversation_hosted_effects",
    "conversation_artifacts",
    "conversation_mcp_servers",
    "message_correlations",
    "idmm_action_reservations",
    "idmm_interventions",
    "nomi_agent_presets",
    "nomi_agent_preset_revisions",
    "nomi_agent_bindings",
];

#[test]
fn canonical_baseline_never_creates_then_deletes_retired_agent_schema() {
    let normalized = BASELINE.to_ascii_lowercase();
    assert!(!normalized.contains("drop table"));
    for table in RETIRED_AGENT_TABLES {
        assert!(
            !normalized.contains(&format!("create table {table}")),
            "canonical baseline must not create retired table {table}"
        );
    }
    for required in [
        "agent_presets",
        "agent_preset_revisions",
        "agent_runtime_snapshots",
        "agent_sessions",
        "agent_turns",
        "agent_events",
        "agent_effects",
        "agent_session_resources",
        "agent_messages",
    ] {
        assert!(
            normalized.contains(&format!("create table {required}")),
            "canonical baseline is missing {required}"
        );
    }
}

#[test]
fn migration_directory_contains_only_the_canonical_baseline() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(files, ["001_canonical_baseline.sql"]);
}

#[tokio::test]
async fn initialized_database_satisfies_the_canonical_id_contract() {
    let database = init_database_memory().await.unwrap();
    validate_id_schema_contract(database.pool()).await.unwrap();
    validate_id_data_contract(database.pool()).await.unwrap();
}

#[tokio::test]
async fn initialized_database_has_no_retired_agent_table() {
    let database = init_database_memory().await.unwrap();
    for table in RETIRED_AGENT_TABLES {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(count, 0, "retired table {table} must be absent");
    }
}

#[tokio::test]
async fn schema_metadata_matches_the_agent_store_manifest() {
    let database = init_database_memory().await.unwrap();
    let actual: (i64, i64, i64, String) = sqlx::query_as(
        "SELECT data_generation, migration_head, projection_schema_version, \
                canonical_schema_manifest_digest \
         FROM schema_metadata WHERE singleton_key = 'canonical'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    let expected = agent_store_schema_manifest_payload();
    assert_eq!(actual.0, i64::from(expected.data_generation));
    assert_eq!(actual.1, i64::from(expected.migration_head));
    assert_eq!(actual.2, i64::from(expected.projection_schema_version));
    assert_eq!(actual.3, digest_payload(&expected).unwrap().as_ref());
}

#[tokio::test]
async fn canonical_preset_indexes_cover_owner_ui_revision_and_snapshot_lookups() {
    let database = init_database_memory().await.unwrap();
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'index' AND name IN (\
            'idx_agent_presets_owner_active', 'idx_agent_presets_ui_plugin', \
            'idx_agent_preset_revisions_created_by', 'idx_agent_bindings_preset', \
            'idx_agent_runtime_snapshots_revision') ORDER BY name",
    )
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert_eq!(indexes.len(), 5);
}
