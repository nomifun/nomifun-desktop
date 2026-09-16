//! Authenticate the local runtime-events lineage through 095, excluding the
//! retired 027, before converging it with upstream's 095 snapshot retirement
//! and 096 product Agent selections.
//! Canonical 099 retains the exact old runtime SQL bytes. Only its ledger
//! version moves: no runtime table, event, checksum or timestamp is rewritten.
//! The move and missing canonical suffix commit together, or roll back together.

use sqlx::migrate::{Migrate, Migration, Migrator};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, Row, SqliteConnection};

use crate::error::DbError;

#[cfg(test)]
mod tests;

const DISPLACED_VERSION: i64 = 95;
const CANONICAL_VERSION: i64 = 99;
// SHA-384 of the original, LF-encoded 095_conversation_runtime_events.sql.
// Pin the historical SQL, rather than trusting arbitrary future edits to 099.
const RUNTIME_CHECKSUM: &str =
    "7b1ea27100c5240ae76a170f0a3bd155b6a568ce304600690b03fbdd148c1c61e1f4619543f250f9c1594bff267b369f";
const READ_LEDGER: &str =
    "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version";

fn canonical_runtime(migrator: &Migrator) -> Result<&Migration, DbError> {
    let canonical = migrator
        .iter()
        .find(|migration| migration.version == CANONICAL_VERSION)
        .ok_or_else(|| DbError::Init("canonical runtime migration 099 is missing".into()))?;
    if hex::encode(canonical.checksum.as_ref()) != RUNTIME_CHECKSUM || canonical.no_tx {
        return Err(DbError::Init(
            "canonical runtime migration 099 no longer matches the original transactional SQL".into(),
        ));
    }
    Ok(canonical)
}

pub(super) fn is_displaced_prefix(
    rows: &[SqliteRow],
    migrator: &Migrator,
) -> Result<bool, DbError> {
    let mut displaced = None;
    let mut target = None;
    for row in rows {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        if version == DISPLACED_VERSION {
            displaced = Some(row);
        }
        if version == CANONICAL_VERSION {
            target = Some(row);
        }
    }
    let Some(displaced) = displaced else {
        if target.is_some() {
            return Err(DbError::Init(
                "runtime migration 099 exists without canonical 095; partial relocation refused".into(),
            ));
        }
        return Ok(false);
    };
    let canonical = canonical_runtime(migrator)?;
    if let Some(target) = target {
        let checksum: Vec<u8> = target.try_get("checksum").map_err(DbError::Query)?;
        let success: bool = target.try_get("success").map_err(DbError::Query)?;
        if !success || checksum.as_slice() != canonical.checksum.as_ref() {
            return Err(DbError::Init("migration 099 conflicts with canonical runtime SQL".into()));
        }
    }
    let checksum: Vec<u8> = displaced.try_get("checksum").map_err(DbError::Query)?;
    if checksum.as_slice() != canonical.checksum.as_ref() {
        let upstream = migrator
            .iter()
            .find(|migration| migration.version == DISPLACED_VERSION)
            .ok_or_else(|| DbError::Init("upstream snapshot migration 095 is missing".into()))?;
        if checksum.as_slice() == upstream.checksum.as_ref() {
            // Ordinary upstream lineage; the regular validator/migrator owns it.
            return Ok(false);
        }
        return Err(DbError::Init("unknown migration 095 checksum; runtime relocation refused".into()));
    }
    if target.is_some() {
        return Err(DbError::Init("cannot relocate runtime 095: migration 099 is already occupied".into()));
    }

    // The merged canonical prefix has the fixed 027 retirement gap. Authenticate
    // that exact version set, not a row-count/index assumption and not whatever
    // a future embedded migrator happens to contain.
    let prefix = migrator
        .iter()
        .filter(|migration| migration.version < DISPLACED_VERSION)
        .collect::<Vec<_>>();
    let expected_versions = (1..DISPLACED_VERSION).filter(|version| *version != 27);
    if !prefix.iter().map(|migration| migration.version).eq(expected_versions) {
        return Err(DbError::Init(
            "embedded runtime prefix no longer matches authenticated 001..026,028..094".into(),
        ));
    }

    // Exactly that prefix plus the old runtime 095, all successful and canonical.
    // Reject target occupancy and all later/gapped/partially repaired lineages;
    // unpublished effect migrations are not an authenticated historical suffix.
    if rows.len() != prefix.len() + 1 {
        return Err(DbError::Init(
            "displaced runtime lineage must be exactly 001..026,028..095 with no occupied 099 or extra suffix".into(),
        ));
    }
    let expected = prefix
        .into_iter()
        .map(|migration| (migration.version, migration))
        .chain(std::iter::once((DISPLACED_VERSION, canonical)));
    for (row, (expected_version, expected)) in rows.iter().zip(expected) {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let success: bool = row.try_get("success").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        if version != expected_version
            || !success
            || checksum.as_slice() != expected.checksum.as_ref()
        {
            return Err(DbError::Init(format!(
                "displaced runtime lineage does not match authenticated migration {expected_version}"
            )));
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
    let canonical = canonical_runtime(migrator)?;
    let mut transaction = conn.begin().await.map_err(DbError::Query)?;
    let result = async {
        // Re-authenticate under the same transaction as the ledger update.
        let rows = sqlx::query(READ_LEDGER)
            .fetch_all(&mut *transaction)
            .await
            .map_err(DbError::Query)?;
        if !is_displaced_prefix(&rows, migrator)? {
            return Err(DbError::Init("runtime migration changed during reconciliation".into()));
        }
        let changed = sqlx::query(
            "UPDATE _sqlx_migrations SET version = ? \
             WHERE version = ? AND success = 1 AND checksum = ? \
             AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
        )
        .bind(CANONICAL_VERSION)
        .bind(DISPLACED_VERSION)
        .bind(canonical.checksum.as_ref())
        .bind(CANONICAL_VERSION)
        .execute(&mut *transaction)
        .await
        .map_err(DbError::Query)?;
        if changed.rows_affected() != 1 {
            return Err(DbError::Init("runtime migration relocation lost its exact ledger row".into()));
        }
        // SQLx skips authenticated 099 and applies missing 095/096 and 100+.
        // Nested migration transactions are savepoints in this outer transaction.
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
        "Converged runtime migration ledger without replaying runtime events"
    );
    Ok(true)
}
