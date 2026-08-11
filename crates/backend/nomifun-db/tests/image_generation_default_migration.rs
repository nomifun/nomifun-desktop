use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
const LEGACY_KEY: &str = "tools.imageGenerationModel";
const CANONICAL_KEY: &str = "models.default.imageGeneration";

async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

async fn database_before_migration() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 27).await;
    sqlx::query(
        "INSERT INTO providers (\
             provider_id, platform, name, base_url, api_key_encrypted, enabled, created_at, updated_at\
         ) VALUES (?, 'openai', 'Image Provider', 'https://example.invalid', 'encrypted', 1, 1, 1)",
    )
    .bind(PROVIDER)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn valid_legacy_default_is_normalized_and_renamed() {
    let pool = database_before_migration().await;
    sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, 42)",
    )
    .bind(LEGACY_KEY)
    .bind(
        serde_json::json!({
            "provider_id": PROVIDER,
            "model": "image-model",
            "switch": true,
            "tool_owned_field": "discard me"
        })
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 28).await;

    let legacy_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key = ?")
            .bind(LEGACY_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (value, updated_at): (String, i64) = sqlx::query_as(
        "SELECT value, updated_at FROM client_preferences WHERE key = ?",
    )
    .bind(CANONICAL_KEY)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(legacy_count, 0);
    assert_eq!(updated_at, 42);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&value).unwrap(),
        serde_json::json!({"provider_id": PROVIDER, "model": "image-model"})
    );
    nomifun_db::validate_id_schema_contract(&pool).await.unwrap();
    nomifun_db::validate_id_data_contract(&pool).await.unwrap();
}

#[tokio::test]
async fn canonical_default_wins_and_legacy_key_is_always_removed() {
    let pool = database_before_migration().await;
    let canonical = serde_json::json!({
        "provider_id": PROVIDER,
        "model": "new-authority"
    })
    .to_string();
    sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, 10), (?, ?, 20)",
    )
    .bind(CANONICAL_KEY)
    .bind(&canonical)
    .bind(LEGACY_KEY)
    .bind(
        serde_json::json!({"provider_id": PROVIDER, "model": "legacy-value"})
            .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 28).await;

    let value: String =
        sqlx::query_scalar("SELECT value FROM client_preferences WHERE key = ?")
            .bind(CANONICAL_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
    let legacy_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key = ?")
            .bind(LEGACY_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&value).unwrap(),
        serde_json::from_str::<serde_json::Value>(&canonical).unwrap()
    );
    assert_eq!(legacy_count, 0);
}

#[tokio::test]
async fn malformed_or_dangling_legacy_default_is_discarded() {
    for value in [
        serde_json::json!({"provider_id": PROVIDER, "model": " "}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "\tmodel"}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "model\n"}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "\u{00a0}model"}).to_string(),
        serde_json::json!({
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000099",
            "model": "image-model"
        })
        .to_string(),
        "not-json".to_owned(),
    ] {
        let pool = database_before_migration().await;
        sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, 1)",
        )
        .bind(LEGACY_KEY)
        .bind(value)
        .execute(&pool)
        .await
        .unwrap();

        migrate_to(&pool, 28).await;

        let canonical_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key = ?")
                .bind(CANONICAL_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let legacy_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key = ?")
                .bind(LEGACY_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((canonical_count, legacy_count), (0, 0));
    }
}

#[tokio::test]
async fn malformed_canonical_is_removed_before_valid_legacy_is_migrated() {
    for malformed_canonical in [
        "not-json".to_owned(),
        serde_json::json!({"provider_id": PROVIDER, "model": " "}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "\tmodel"}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "model\n"}).to_string(),
        serde_json::json!({"provider_id": PROVIDER, "model": "\u{00a0}model"}).to_string(),
        serde_json::json!({
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000099",
            "model": "dangling"
        })
        .to_string(),
    ] {
        let pool = database_before_migration().await;
        sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, 10), (?, ?, 20)",
        )
        .bind(CANONICAL_KEY)
        .bind(malformed_canonical)
        .bind(LEGACY_KEY)
        .bind(
            serde_json::json!({
                "provider_id": PROVIDER,
                "model": "legacy-recovery",
                "switch": true
            })
            .to_string(),
        )
        .execute(&pool)
        .await
        .unwrap();

        migrate_to(&pool, 28).await;

        let value: String =
            sqlx::query_scalar("SELECT value FROM client_preferences WHERE key = ?")
                .bind(CANONICAL_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&value).unwrap(),
            serde_json::json!({"provider_id": PROVIDER, "model": "legacy-recovery"})
        );
        let legacy_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key = ?")
                .bind(LEGACY_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(legacy_count, 0);
        nomifun_db::validate_id_data_contract(&pool).await.unwrap();
    }
}

#[tokio::test]
async fn canonical_beta_fields_are_normalized_without_losing_precedence() {
    let pool = database_before_migration().await;
    sqlx::query("INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, 7)")
        .bind(CANONICAL_KEY)
        .bind(
            serde_json::json!({
                "provider_id": PROVIDER,
                "model": "canonical-model",
                "switch": false,
                "beta": "discard"
            })
            .to_string(),
        )
        .execute(&pool)
        .await
        .unwrap();

    migrate_to(&pool, 28).await;

    let (value, updated_at): (String, i64) = sqlx::query_as(
        "SELECT value, updated_at FROM client_preferences WHERE key = ?",
    )
    .bind(CANONICAL_KEY)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(updated_at, 7);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&value).unwrap(),
        serde_json::json!({"provider_id": PROVIDER, "model": "canonical-model"})
    );
}
