use super::*;
use crate::database::DB_MIGRATOR;

#[tokio::test]
async fn embedded_prefix_version_drift_is_not_authenticated_as_history() {
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in DB_MIGRATOR.iter().filter(|migration| migration.version < 88) {
        connection.apply(migration).await.unwrap();
    }
    connection.apply(&displaced_migration(&DB_MIGRATOR).unwrap()).await.unwrap();
    let rows = sqlx::query(READ_LEDGER).fetch_all(&mut connection).await.unwrap();
    assert!(is_displaced_prefix(&rows, &DB_MIGRATOR).unwrap());

    for add_retired_version in [false, true] {
        let mut migrations = DB_MIGRATOR.iter().cloned().collect::<Vec<_>>();
        if add_retired_version {
            let mut extra = migrations.iter().find(|migration| migration.version == 26)
                .unwrap().clone();
            extra.version = 27;
            migrations.push(extra);
            migrations.sort_by_key(|migration| migration.version);
        } else {
            migrations.retain(|migration| migration.version != 28);
        }
        let migrator = Migrator {
            migrations: Cow::Owned(migrations),
            ..Migrator::DEFAULT
        };
        let error = is_displaced_prefix(&rows, &migrator).unwrap_err();
        assert!(error.to_string().contains("embedded Agent prefix no longer matches"));
    }
}
