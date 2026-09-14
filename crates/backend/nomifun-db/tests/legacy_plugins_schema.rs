//! Negative contract coverage for the retired single-document `plugins` root.
//!
//! The destructive Plugin unification starts from the final `plugin_*` schema.
//! This file intentionally verifies that the old table and its indexes are not
//! recreated or registered as a compatibility surface.

use nomifun_db::init_database_memory;

#[tokio::test]
async fn clean_start_does_not_create_the_retired_plugins_table_or_indexes() {
    let database = init_database_memory().await.expect("database");

    let retired_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table' AND name = 'plugins'",
    )
    .fetch_one(database.pool())
    .await
    .expect("retired plugins table lookup");
    assert_eq!(retired_table_count, 0);

    let retired_index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'index'
           AND name IN ('idx_plugins_user_id', 'idx_plugins_source_conversation_id')",
    )
    .fetch_one(database.pool())
    .await
    .expect("retired plugins index lookup");
    assert_eq!(retired_index_count, 0);

    let final_product_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table'
           AND name IN ('plugin_library_state', 'plugin_products', 'plugin_projects')",
    )
    .fetch_one(database.pool())
    .await
    .expect("final Plugin roots lookup");
    assert_eq!(final_product_count, 3);

    nomifun_db::validate_id_schema_contract(database.pool())
        .await
        .expect("final Plugin schema remains registered");
}

#[test]
fn retired_migrations_are_explicit_clean_start_noops() {
    let migration_028 = include_str!("../migrations/028_plugins_clean_start.sql");
    let migration_029 = include_str!("../migrations/029_plugins_clean_start_followup.sql");

    for migration in [migration_028, migration_029] {
        assert!(!migration.contains("CREATE TABLE plugins"));
        assert!(!migration.contains("INSERT INTO plugins"));
        assert!(!migration.contains("ALTER TABLE plugins"));
        assert!(!migration.contains("published_at"));
    }
}
