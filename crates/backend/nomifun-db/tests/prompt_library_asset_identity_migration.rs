use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const LEGACY_A: &str = "0190f5fe-7c00-7a00-8abc-000000000901";
const LEGACY_B: &str = "0190f5fe-7c00-7a00-8abc-000000000902";
const CURRENT_A: &str = "0190f5fe-7c00-7a00-8abc-000000000903";
const CURRENT_B: &str = "0190f5fe-7c00-7a00-8abc-000000000904";
const INVALID: &str = "0190f5fe-7c00-7a00-8abc-000000000905";

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

async fn insert_text_asset(
    pool: &sqlx::SqlitePool,
    asset_id: &str,
    origin: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO workshop_assets \
            (asset_id, kind, title, tags, text_content, in_library, origin, created_at, updated_at) \
         VALUES (?, 'text', 'prompt', '[]', 'body', 1, ?, 1, 1)",
    )
    .bind(asset_id)
    .bind(origin.to_string())
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn migration_052_preserves_legacy_duplicates_and_enforces_new_identity() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 51).await;

    let legacy = serde_json::json!({
        "prompt_catalog_id": "legacy-duplicate",
        "source_url": "https://example.test/source",
        "license": "MIT",
        "license_url": "https://example.test/license"
    });
    insert_text_asset(&pool, LEGACY_A, legacy.clone())
        .await
        .unwrap();
    insert_text_asset(&pool, LEGACY_B, legacy).await.unwrap();

    migrate_to(&pool, 52).await;
    nomifun_db::validate_id_schema_contract(&pool)
        .await
        .expect("v52 schema contract");

    let legacy_ids: Vec<String> = sqlx::query_scalar(
        "SELECT asset_id FROM workshop_assets \
         WHERE json_extract(origin, '$.prompt_catalog_id') = 'legacy-duplicate' \
         ORDER BY asset_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(legacy_ids, vec![LEGACY_A.to_owned(), LEGACY_B.to_owned()]);

    let current = serde_json::json!({
        "prompt_library_source": "catalog",
        "prompt_library_id": "catalog-item",
        "prompt_catalog_id": "catalog-item"
    });
    insert_text_asset(&pool, CURRENT_A, current.clone())
        .await
        .unwrap();
    let duplicate = insert_text_asset(&pool, CURRENT_B, current)
        .await
        .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("uq_workshop_assets_prompt_library_identity")
            || duplicate.to_string().contains("UNIQUE constraint failed")
    );

    let invalid = insert_text_asset(
        &pool,
        INVALID,
        serde_json::json!({ "prompt_library_source": "catalog" }),
    )
    .await
    .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("invalid prompt library asset origin identity")
    );
}
