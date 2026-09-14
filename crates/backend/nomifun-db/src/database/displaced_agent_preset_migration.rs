//! One-time convergence for a pre-merge development lineage where the Agent
//! preset materialization migration shipped locally as 088. The Director
//! retirement later became canonical 088 and the Agent migration moved to 089.
//!
//! The displaced SQL is byte-identical to canonical 089 except for its private
//! temporary-table suffix (`v088` versus `v089`). Authenticate that exact SQL
//! checksum and the complete canonical 001..087 prefix before changing the
//! ledger. Then move the row to 089, adopt the canonical checksum, and let SQLx
//! apply the missing canonical 088 plus the remaining suffix in one transaction.

use std::borrow::Cow;

use sqlx::migrate::{Migrate, Migration, Migrator};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, Row, SqliteConnection};

use crate::error::DbError;

const DISPLACED_VERSION: i64 = 88;
const CANONICAL_VERSION: i64 = 89;
const READ_LEDGER: &str =
    "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version";

fn displaced_migration(migrator: &Migrator) -> Result<Migration, DbError> {
    let canonical = migrator
        .iter()
        .find(|migration| migration.version == CANONICAL_VERSION)
        .ok_or_else(|| {
            DbError::Init(format!(
                "embedded migration {CANONICAL_VERSION} is missing from the displaced Agent repair"
            ))
        })?;
    let displaced_sql = canonical.sql.replace("v089", "v088");
    if displaced_sql == canonical.sql {
        return Err(DbError::Init(format!(
            "embedded migration {CANONICAL_VERSION} no longer contains the authenticated v089 marker"
        )));
    }
    Ok(Migration::new(
        DISPLACED_VERSION,
        canonical.description.clone(),
        canonical.migration_type,
        Cow::Owned(displaced_sql),
        canonical.no_tx,
    ))
}

pub(super) fn is_displaced_prefix(
    rows: &[SqliteRow],
    migrator: &Migrator,
) -> Result<bool, DbError> {
    if rows.len() != DISPLACED_VERSION as usize {
        return Ok(false);
    }
    let expected = migrator.iter().collect::<Vec<_>>();
    if expected.len() < CANONICAL_VERSION as usize {
        return Ok(false);
    }
    let displaced = displaced_migration(migrator)?;
    for (index, row) in rows.iter().enumerate() {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let success: bool = row.try_get("success").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        let expected_version = index as i64 + 1;
        if version != expected_version || !success {
            return Ok(false);
        }
        let expected_checksum = if version == DISPLACED_VERSION {
            displaced.checksum.as_ref()
        } else {
            expected[index].checksum.as_ref()
        };
        if checksum.as_slice() != expected_checksum {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) async fn adopt_and_migrate(
    conn: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<bool, DbError> {
    conn.ensure_migrations_table()
        .await
        .map_err(DbError::Migration)?;
    let rows = sqlx::query(READ_LEDGER)
        .fetch_all(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    if !is_displaced_prefix(&rows, migrator)? {
        return Ok(false);
    }

    let displaced = displaced_migration(migrator)?;
    let canonical = migrator
        .iter()
        .find(|migration| migration.version == CANONICAL_VERSION)
        .ok_or_else(|| DbError::Init("canonical Agent migration disappeared during repair".into()))?;
    let mut transaction = conn.begin().await.map_err(DbError::Query)?;
    let rows = sqlx::query(READ_LEDGER)
        .fetch_all(&mut *transaction)
        .await
        .map_err(DbError::Query)?;
    if !is_displaced_prefix(&rows, migrator)? {
        transaction.rollback().await.map_err(DbError::Query)?;
        return Ok(false);
    }

    let result = async {
        let changed = sqlx::query(
            "UPDATE _sqlx_migrations \
             SET version = ?, checksum = ? \
             WHERE version = ? AND success = 1 AND checksum = ?",
        )
        .bind(CANONICAL_VERSION)
        .bind(canonical.checksum.as_ref())
        .bind(DISPLACED_VERSION)
        .bind(displaced.checksum.as_ref())
        .execute(&mut *transaction)
        .await
        .map_err(DbError::Query)?;
        if changed.rows_affected() != 1 {
            return Err(DbError::Init(
                "displaced Agent migration changed during reconciliation".into(),
            ));
        }

        migrator
            .run_direct(&mut *transaction)
            .await
            .map_err(DbError::Migration)
    }
    .await;
    if let Err(error) = result {
        transaction.rollback().await.map_err(DbError::Query)?;
        return Err(error);
    }
    transaction.commit().await.map_err(DbError::Query)?;
    tracing::info!(
        displaced_version = DISPLACED_VERSION,
        canonical_version = CANONICAL_VERSION,
        "Converged displaced Agent preset migration and applied the canonical suffix"
    );
    Ok(true)
}
