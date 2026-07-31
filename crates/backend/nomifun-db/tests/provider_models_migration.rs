use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-012345678901";

/// Apply migrations up to (and including) `max_version`, skipping versions
/// already recorded in `_sqlx_migrations` so repeated calls are incremental.
async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.version)
        .collect();
    for m in MIGRATOR.iter() {
        if m.version <= max_version && !applied.contains(&m.version) {
            conn.apply(m).await.unwrap();
        }
    }
}

#[tokio::test]
async fn backfill_merges_maps_and_profiles_and_drops_orphans() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 13).await;

    // Legacy-shaped provider row: 2 catalog models + per-model maps.
    sqlx::query(
        "INSERT INTO providers (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, model_context_limits, model_protocols, model_descriptions, model_enabled, model_health, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, 'openai', 'P', 'https://x.test/v1', 'enc', ?, 1, '[]', ?, ?, ?, ?, ?, 0, 0, 1, 1)",
    )
    .bind(PROVIDER)
    .bind(r#"["gpt-4o","flux-pro"]"#)
    .bind(r#"{"gpt-4o":128000}"#)
    .bind(r#"{"flux-pro":"anthropic"}"#)
    .bind(r#"{"gpt-4o":"desc"}"#)
    .bind(r#"{"flux-pro":false}"#)
    .bind(r#"{"gpt-4o":{"status":"healthy"}}"#)
    .execute(&pool)
    .await
    .unwrap();
    // Profile for gpt-4o + an ORPHAN profile (model not in catalog).
    sqlx::query(
        "INSERT INTO model_profiles (provider_id, model, tasks, traits, params, source, updated_at) VALUES \
         (?, 'gpt-4o', '[\"chat\"]', '[\"vision_input\"]', '{\"endpoint\":\"/x\"}', 'user', 42), \
         (?, 'ghost-model', '[\"chat\"]', '[]', '{}', 'user', 42)",
    )
    .bind(PROVIDER)
    .bind(PROVIDER)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 14).await;

    let rows: Vec<(String, i64, i64, String, String, Option<String>, String, Option<i64>, Option<String>, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT model, enabled, sort_order, tasks, traits, protocol, params, context_limit, description, source, health, updated_at \
         FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
    )
    .bind(PROVIDER)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "orphan profile must not migrate");
    let gpt = &rows[0];
    assert_eq!(gpt.0, "gpt-4o");
    assert_eq!(gpt.1, 1);
    assert_eq!(gpt.3, r#"["chat"]"#);
    assert_eq!(gpt.4, r#"["vision_input"]"#);
    assert_eq!(gpt.6, r#"{"endpoint":"/x"}"#);
    assert_eq!(gpt.7, Some(128000));
    assert_eq!(gpt.8.as_deref(), Some("desc"));
    assert_eq!(gpt.9, "user");
    assert!(gpt.10.as_deref().unwrap_or("").contains("healthy"));
    assert_eq!(gpt.11, 42);
    let flux = &rows[1];
    assert_eq!(flux.0, "flux-pro");
    assert_eq!(flux.1, 0, "model_enabled=false must carry over");
    assert_eq!(flux.5.as_deref(), Some("anthropic"));
    assert_eq!(flux.9, "inferred");
}

#[tokio::test]
async fn backfill_dedupes_duplicate_models_and_tolerates_corrupt_health() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 13).await;

    // Legacy catalog with a duplicated name (the legacy API never deduplicated
    // providers.models) plus a corrupt model_health value: json_each unwraps
    // the bare-string entry to non-JSON text, which raw json() would reject.
    sqlx::query(
        "INSERT INTO providers (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, model_health, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, 'openai', 'P', 'https://x.test/v1', 'enc', ?, 1, '[]', ?, 0, 0, 1, 1)",
    )
    .bind(PROVIDER)
    .bind(r#"["a","a","b"]"#)
    .bind(r#"{"a":"not-an-object"}"#)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 14).await;

    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT model, sort_order, health FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
    )
    .bind(PROVIDER)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "duplicate catalog entry must collapse to one row");
    assert_eq!(rows[0].0, "a");
    assert_eq!(rows[0].1, 0, "first array occurrence wins, matching dual-write semantics");
    assert!(rows[0].2.is_none(), "corrupt legacy health degrades to NULL, not a failed migration");
    assert_eq!(rows[1].0, "b");
}

#[tokio::test]
async fn migration_15_drops_model_profiles() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 14).await;
    // At the 014 point the superseded table must still exist (the backfill
    // above reads from it).
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'model_profiles'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "model_profiles must still exist at migration 14");

    migrate_to(&pool, 15).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE name IN ('model_profiles', 'idx_model_profiles_provider_id')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "migration 15 must drop model_profiles and its index");
}

#[tokio::test]
async fn migration_16_drops_legacy_provider_model_columns_preserving_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 13).await;

    // Legacy-shaped provider row: the 014 backfill materializes its
    // provider_models rows, which must survive the 016 column drop intact.
    sqlx::query(
        "INSERT INTO providers (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, model_context_limits, model_protocols, model_descriptions, model_enabled, model_health, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, 'openai', 'P', 'https://x.test/v1', 'enc', ?, 1, '[]', ?, ?, ?, ?, ?, 0, 0, 1, 1)",
    )
    .bind(PROVIDER)
    .bind(r#"["gpt-4o","flux-pro"]"#)
    .bind(r#"{"gpt-4o":128000}"#)
    .bind(r#"{"flux-pro":"anthropic"}"#)
    .bind(r#"{"gpt-4o":"desc"}"#)
    .bind(r#"{"flux-pro":false}"#)
    .bind(r#"{"gpt-4o":{"status":"healthy"}}"#)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 15).await;
    let legacy_columns = [
        "models",
        "model_context_limits",
        "model_protocols",
        "model_descriptions",
        "model_enabled",
        "model_health",
    ];
    let columns_at_15: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('providers')")
            .fetch_all(&pool)
            .await
            .unwrap();
    for column in legacy_columns {
        assert!(
            columns_at_15.iter().any(|name| name == column),
            "column {column} must still exist at migration 15"
        );
    }

    migrate_to(&pool, 16).await;
    let columns_at_16: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('providers')")
            .fetch_all(&pool)
            .await
            .unwrap();
    for column in legacy_columns {
        assert!(
            !columns_at_16.iter().any(|name| name == column),
            "migration 16 must drop providers.{column}"
        );
    }
    // The remaining provider fields and the authoritative rows are untouched.
    for column in [
        "provider_id",
        "platform",
        "name",
        "base_url",
        "api_key_encrypted",
        "enabled",
        "capabilities",
        "bedrock_config",
        "is_full_url",
        "sort_order",
    ] {
        assert!(
            columns_at_16.iter().any(|name| name == column),
            "providers.{column} must survive migration 16"
        );
    }
    let (name, enabled): (String, i64) =
        sqlx::query_as("SELECT name, enabled FROM providers WHERE provider_id = ?")
            .bind(PROVIDER)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((name.as_str(), enabled), ("P", 1));
    let rows: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT model, enabled, context_limit, protocol FROM provider_models \
         WHERE provider_id = ? ORDER BY sort_order",
    )
    .bind(PROVIDER)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("gpt-4o".to_owned(), 1, Some(128_000), None),
            ("flux-pro".to_owned(), 0, None, Some("anthropic".to_owned())),
        ],
        "the 014-backfilled rows must survive the 016 column drop verbatim"
    );
}

#[tokio::test]
async fn migration_21_drops_capabilities_preserving_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 13).await;

    // Legacy-shaped provider row (pre-016 schema, so the legacy columns are
    // still insertable here); only name/enabled/capabilities matter below.
    sqlx::query(
        "INSERT INTO providers (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, 'openai', 'P', 'https://x.test/v1', 'enc', '[\"gpt-4o\"]', 1, '[{\"type\":\"text\"}]', 0, 0, 1, 1)",
    )
    .bind(PROVIDER)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 16).await;
    let columns_at_16: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('providers')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        columns_at_16.iter().any(|name| name == "capabilities"),
        "providers.capabilities must still exist at migration 16"
    );

    migrate_to(&pool, 21).await;
    let columns_at_17: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('providers')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        !columns_at_17.iter().any(|name| name == "capabilities"),
        "migration 17 must drop providers.capabilities"
    );
    // The remaining provider fields survive.
    for column in [
        "provider_id",
        "platform",
        "name",
        "base_url",
        "api_key_encrypted",
        "enabled",
        "bedrock_config",
        "is_full_url",
        "sort_order",
    ] {
        assert!(
            columns_at_17.iter().any(|name| name == column),
            "providers.{column} must survive migration 17"
        );
    }
    // And the provider row itself is untouched.
    let (name, enabled): (String, i64) =
        sqlx::query_as("SELECT name, enabled FROM providers WHERE provider_id = ?")
            .bind(PROVIDER)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((name.as_str(), enabled), ("P", 1));
    // The 014-backfilled provider_models row survives too.
    let models: Vec<String> = sqlx::query_scalar(
        "SELECT model FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
    )
    .bind(PROVIDER)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(models, vec!["gpt-4o"]);
}

#[tokio::test]
async fn fresh_database_passes_schema_contract_with_new_tables() {
    // init_database_memory runs ALL migrations + the id schema contract.
    let db = nomifun_db::init_database_memory().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_connections")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}
