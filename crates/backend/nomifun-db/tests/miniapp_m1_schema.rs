use nomifun_db::{init_database_memory, installation_owner_id};
use sqlx::Row;
use sqlx::migrate::{Migrate, Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn migrate_through(pool: &nomifun_db::SqlitePool, maximum_version: i64) {
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= maximum_version)
    {
        connection.apply(migration).await.unwrap();
    }
}

#[tokio::test]
async fn migration_072_is_additive_and_starts_with_an_empty_new_root() {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool();

    let old_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapps")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(old_count, 0);

    for table in [
        "miniapp_library_state",
        "miniapp_products",
        "miniapp_projects",
        "miniapp_release_artifacts",
        "miniapp_releases",
        "miniapp_credential_bindings",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "new M1 table {table} must start empty");
    }

    let migration = include_str!("../migrations/072_miniapp_m1_data_root.sql");
    assert!(!migration.contains("FROM miniapps"));
    assert!(!migration.contains("INSERT INTO miniapps"));
    assert!(!migration.contains("UPDATE miniapps"));
}

#[tokio::test]
async fn new_root_preserves_owner_and_pointer_shape_checks() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let miniapp_id = "0190f5fe-7c00-7000-8000-000000000001";
    let invalid_miniapp_id = "not-a-uuid";

    let invalid = sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, display_name, kind,
            materialized_catalog_digest, created_at, updated_at
         ) VALUES (?, ?, 'Invalid', 'ui_only', ?, 1, 1)",
    )
    .bind(invalid_miniapp_id)
    .bind(&owner)
    .bind("a".repeat(64))
    .execute(database.pool())
    .await;
    assert!(invalid.is_err());

    sqlx::query(
        "INSERT INTO miniapp_products (
            miniapp_id, owner_user_id, display_name, kind,
            materialized_catalog_digest, created_at, updated_at
         ) VALUES (?, ?, 'Valid', 'ui_only', ?, 1, 1)",
    )
    .bind(miniapp_id)
    .bind(&owner)
    .bind("a".repeat(64))
    .execute(database.pool())
    .await
    .unwrap();

    let row = sqlx::query(
        "SELECT miniapp_id, owner_user_id, lifecycle, active_release_epoch
         FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(miniapp_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("miniapp_id"), miniapp_id);
    assert_eq!(row.get::<String, _>("owner_user_id"), owner);
    assert_eq!(row.get::<String, _>("lifecycle"), "disabled");
    assert_eq!(row.get::<i64, _>("active_release_epoch"), 0);
}

#[tokio::test]
async fn migration_072_preserves_existing_legacy_miniapp_rows_byte_for_byte() {
    let database = nomifun_db::SqlitePool::connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_through(&database, 71).await;
    let owner = "0190f5fe-7c00-7000-8000-000000000201";
    sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(owner)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    let legacy_id = "0190f5fe-7c00-7000-8000-000000000202";
    sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            created_at, updated_at
         ) VALUES (?, ?, 'legacy', 'old', '<p>old</p>', 10, 2, 3)",
    )
    .bind(legacy_id)
    .bind(owner)
    .execute(&database)
    .await
    .unwrap();
    let before: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();

    let mut connection = database.acquire().await.unwrap();
    let migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == 72)
        .unwrap();
    connection.apply(migration).await.unwrap();
    drop(connection);

    let after: (String, String, String, i64, i64) = sqlx::query_as(
        "SELECT name, description, html, created_at, updated_at
         FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_id)
    .fetch_one(&database)
    .await
    .unwrap();
    assert_eq!(after, before);
    let new_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_products")
            .fetch_one(&database)
            .await
            .unwrap();
    assert_eq!(new_count, 0);
}
