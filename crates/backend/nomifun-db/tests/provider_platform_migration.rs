use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            conn.apply(migration).await.unwrap();
        }
    }
}

#[tokio::test]
async fn exact_legacy_preset_roots_gain_provider_identity_and_retired_roots_move() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 29).await;

    let fixtures = [
        ("019f0000-0000-7000-8000-000000000001", "custom", "https://api.openai.com/v1/"),
        ("019f0000-0000-7000-8000-000000000002", "custom", "https://api.x.ai/v1"),
        ("019f0000-0000-7000-8000-000000000003", "custom", "https://api.ppinfra.com/v3/openai"),
        ("019f0000-0000-7000-8000-000000000004", "custom", "https://wishub-x1.ctyun.cn/v1"),
        ("019f0000-0000-7000-8000-000000000005", "custom", "https://gateway.example.com/v1"),
        ("019f0000-0000-7000-8000-000000000006", "new-api", "https://api.x.ai/v1"),
    ];
    for (provider_id, platform, base_url) in fixtures {
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, api_key_encrypted, created_at, updated_at) \
             VALUES (?, ?, 'P', ?, 'encrypted', 1, 1)",
        )
        .bind(provider_id)
        .bind(platform)
        .bind(base_url)
        .execute(&pool)
        .await
        .unwrap();
    }

    migrate_to(&pool, 30).await;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT platform, base_url FROM providers ORDER BY provider_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("openai".into(), "https://api.openai.com/v1/".into()),
            ("xai".into(), "https://api.x.ai/v1".into()),
            ("ppio".into(), "https://api.ppio.com/openai/v1".into()),
            ("ctyun".into(), "https://wishub-x6.ctyun.cn/v1".into()),
            ("custom".into(), "https://gateway.example.com/v1".into()),
            ("new-api".into(), "https://api.x.ai/v1".into()),
        ]
    );
}
