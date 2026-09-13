use std::borrow::Cow;
use std::path::Path;

use nomifun_db::{MigrationLineageStatus, init_database, inspect_supported_migration_lineage};
use sqlx::migrate::{Migrate, Migration, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn open(path: &Path, read_only: bool) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(!read_only)
                .read_only(read_only),
        )
        .await
        .unwrap()
}

async fn create_displaced_database(path: &Path) -> (Vec<u8>, String) {
    let pool = open(path, false).await;
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR.iter().filter(|migration| migration.version <= 87) {
        connection.apply(migration).await.unwrap();
    }
    let canonical = MIGRATOR
        .iter()
        .find(|migration| migration.version == 89)
        .unwrap();
    let displaced = Migration::new(
        88,
        canonical.description.clone(),
        canonical.migration_type,
        Cow::Owned(canonical.sql.replace("v089", "v088")),
        canonical.no_tx,
    );
    connection.apply(&displaced).await.unwrap();
    drop(connection);
    let installed_on: String = sqlx::query_scalar(
        "SELECT CAST(installed_on AS TEXT) FROM _sqlx_migrations WHERE version = 88",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let checksum = displaced.checksum.into_owned();
    pool.close().await;
    (checksum, installed_on)
}

#[tokio::test]
async fn displaced_agent_preset_migration_converges_without_replacing_the_database() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("displaced-agent-preset.db");
    let (displaced_checksum, original_installed_on) = create_displaced_database(&path).await;

    let probe = open(&path, true).await;
    assert_eq!(
        inspect_supported_migration_lineage(&probe).await.unwrap(),
        MigrationLineageStatus::UpgradeRequired
    );
    let before: (i64, Vec<u8>, String) = sqlx::query_as(
        "SELECT version, checksum, CAST(installed_on AS TEXT) \
         FROM _sqlx_migrations WHERE version = 88",
    )
    .fetch_one(&probe)
    .await
    .unwrap();
    assert_eq!(before, (88, displaced_checksum, original_installed_on.clone()));
    probe.close().await;

    let db = init_database(&path).await.unwrap();
    assert_eq!(
        inspect_supported_migration_lineage(db.pool()).await.unwrap(),
        MigrationLineageStatus::Current
    );
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(
        versions,
        MIGRATOR.iter().map(|migration| migration.version).collect::<Vec<_>>()
    );
    let canonical_89 = MIGRATOR
        .iter()
        .find(|migration| migration.version == 89)
        .unwrap();
    let adopted: (Vec<u8>, String) = sqlx::query_as(
        "SELECT checksum, CAST(installed_on AS TEXT) FROM _sqlx_migrations WHERE version = 89",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(adopted.0.as_slice(), canonical_89.checksum.as_ref());
    assert_eq!(adopted.1, original_installed_on);
    db.close().await;

    let reopened = init_database(&path).await.unwrap();
    assert_eq!(
        inspect_supported_migration_lineage(reopened.pool()).await.unwrap(),
        MigrationLineageStatus::Current
    );
    reopened.close().await;
}

#[tokio::test]
async fn unknown_migration_88_checksum_still_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unknown-agent-preset.db");
    create_displaced_database(&path).await;
    let pool = open(&path, false).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 88")
        .execute(&pool)
        .await
        .unwrap();
    assert!(inspect_supported_migration_lineage(&pool).await.is_err());
    pool.close().await;
    assert!(init_database(&path).await.is_err());
}
