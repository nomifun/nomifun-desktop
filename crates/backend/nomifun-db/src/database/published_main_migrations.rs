//! One-time convergence of the two branches after their common migration 058.
//! Main shipped asset deletion as 059/060 while the refactor used 059..072 for
//! its control plane. Their SQL bytes are retained as canonical 073/074.
//! Authenticate the complete published prefix before touching its ledger, then
//! relocate only those two rows and apply the refactor suffix in one transaction.
//! Checksums, descriptions and installation timestamps are never rewritten.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, Row, SqliteConnection};

use crate::error::DbError;

const RELOCATIONS: [(i64, i64); 2] = [(59, 73), (60, 74)];
const READ_LEDGER: &str =
    "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version";

pub(super) fn is_published_main_prefix(
    rows: &[SqliteRow],
    migrator: &Migrator,
) -> Result<bool, DbError> {
    let prefix = migrator.iter().filter(|migration| migration.version <= 58)
        .collect::<Vec<_>>();
    let mut published_tail = Vec::new();
    for (published, canonical) in RELOCATIONS {
        let Some(expected) = migrator.iter().find(|migration| migration.version == canonical) else {
            return Ok(false);
        };
        published_tail.push((published, expected));
    }
    // Only the exact published asset SQL identifies this branch. Once found,
    // malformed lineages must fail before SQLx can apply an earlier missing DDL.
    let mut recognized = false;
    for row in rows {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        recognized |= published_tail.iter().any(|(published, expected)|
            version == *published && checksum.as_slice() == expected.checksum.as_ref());
    }
    if !recognized {
        return Ok(false);
    }
    if rows.len() != prefix.len() + 1 && rows.len() != prefix.len() + 2 {
        return Err(DbError::Init("published main lineage must contain the complete common prefix and only 059 or 059/060".into()));
    }
    // The common prefix has the canonical 027 gap; version is not row offset.
    let expected = prefix.into_iter().map(|migration| (migration.version, migration))
        .chain(published_tail);
    for (row, (expected_version, expected)) in rows.iter().zip(expected) {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let success: bool = row.try_get("success").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        if version != expected_version || !success || checksum.as_slice() != expected.checksum.as_ref() {
            return Err(DbError::Init(format!(
                "published main lineage does not match authenticated migration {expected_version}"
            )));
        }
    }
    Ok(true)
}

pub(super) async fn adopt_and_migrate(
    conn: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<bool, DbError> {
    conn.ensure_migrations_table().await.map_err(DbError::Migration)?;
    let rows = sqlx::query(READ_LEDGER).fetch_all(&mut *conn).await.map_err(DbError::Query)?;
    if !is_published_main_prefix(&rows, migrator)? {
        return Ok(false);
    }

    let mut transaction = conn.begin().await.map_err(DbError::Query)?;
    let rows = sqlx::query(READ_LEDGER).fetch_all(&mut *transaction).await.map_err(DbError::Query)?;
    // Recheck under the same transaction as the writes, including when another
    // process completed the upgrade after the initial read.
    if !is_published_main_prefix(&rows, migrator)? {
        transaction.rollback().await.map_err(DbError::Query)?;
        return Ok(false);
    }
    let published_rows = rows.iter().map(|row| {
        Ok((row.try_get::<i64, _>("version").map_err(DbError::Query)?,
            row.try_get::<Vec<u8>, _>("checksum").map_err(DbError::Query)?))
    }).collect::<Result<Vec<_>, DbError>>()?;
    let published_head = published_rows.last().map(|row| row.0);
    let result = async {
        for (published, canonical) in RELOCATIONS {
            let Some((_, checksum)) = published_rows.iter().find(|row| row.0 == published) else {
                continue;
            };
            let changed = sqlx::query(
                "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND success = 1 AND checksum = ?",
            )
            .bind(canonical).bind(published).bind(checksum)
            .execute(&mut *transaction).await.map_err(DbError::Query)?;
            if changed.rows_affected() != 1 {
                return Err(DbError::Init("published main migration changed during reconciliation".into()));
            }
        }
        // SQLx applies every missing version, including 059..072 below the
        // relocated rows. Its per-migration transactions become savepoints in
        // this outer transaction; a failed upgrade rolls back the whole move.
        migrator.run_direct(&mut *transaction).await.map_err(DbError::Migration)
    }.await;
    if let Err(error) = result {
        transaction.rollback().await.map_err(DbError::Query)?;
        return Err(error);
    }
    transaction.commit().await.map_err(DbError::Query)?;
    tracing::info!(
        published_head,
        "Converged published main asset migrations 059/060 to canonical 073/074 with unchanged checksums"
    );
    Ok(true)
}
