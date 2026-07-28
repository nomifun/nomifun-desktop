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
async fn fresh_database_passes_schema_contract_with_new_tables() {
    // init_database_memory runs ALL migrations + the id schema contract.
    let db = nomifun_db::init_database_memory().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_connections")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}
