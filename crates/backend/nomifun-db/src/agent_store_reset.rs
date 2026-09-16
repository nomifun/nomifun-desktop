use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{SchemaResetScope, agent_store_schema_manifest_payload};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::DbError;

const RESET_ORDER: &[&str] = &[
    "agent_effects",
    "agent_turns",
    "agent_session_resources",
    "agent_messages",
    "agent_session_heads",
    "agent_events",
    "agent_payloads",
    "agent_sessions",
    "agent_bindings",
    "agent_runtime_snapshots",
    "agent_preset_contribution_locks",
    "agent_preset_revisions",
    "agent_presets",
    "agent_preset_templates",
    "remote_bindings",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDataResetReport {
    pub deleted_rows: BTreeMap<String, u64>,
    pub preserved_rows: BTreeMap<String, u64>,
}

/// Delete only the clean-cut Agent data generation in one SQLite transaction.
///
/// Provider, model, Plugin, MCP, Skill, capability catalog and application
/// configuration tables are counted before and after the reset. Any schema
/// drift or preservation mismatch aborts the transaction.
pub async fn reset_agent_data(pool: &SqlitePool) -> Result<AgentDataResetReport, DbError> {
    let manifest = agent_store_schema_manifest_payload();
    let reset_tables = manifest
        .tables
        .iter()
        .filter(|table| table.reset_scope == SchemaResetScope::AgentData)
        .map(|table| table.table_name.clone())
        .collect::<BTreeSet<_>>();
    let reset_order = RESET_ORDER
        .iter()
        .map(|table| (*table).to_owned())
        .collect::<BTreeSet<_>>();
    if reset_tables != reset_order {
        return Err(DbError::Init(format!(
            "Agent reset order does not cover the schema contract: expected {reset_tables:?}, found {reset_order:?}"
        )));
    }
    let preserved_tables = manifest
        .tables
        .iter()
        .filter(|table| table.reset_scope == SchemaResetScope::Preserve)
        .map(|table| table.table_name.clone())
        .collect::<Vec<_>>();

    let mut tx = pool.begin().await?;
    let existing_tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect::<BTreeSet<_>>();
    let missing_reset = reset_tables
        .difference(&existing_tables)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_reset.is_empty() {
        return Err(DbError::Init(format!(
            "Agent-only reset requires the canonical Agent Store tables: {missing_reset:?}"
        )));
    }
    let preserved_tables = preserved_tables
        .into_iter()
        .filter(|table| existing_tables.contains(table))
        .collect::<Vec<_>>();
    let preserved_before = table_counts(&mut tx, &preserved_tables).await?;
    sqlx::query("UPDATE agent_events SET causation_event_id = NULL")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE agent_sessions SET parent_agent_session_id = NULL")
        .execute(&mut *tx)
        .await?;

    let mut deleted_rows = BTreeMap::new();
    for table in RESET_ORDER {
        let result = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
        deleted_rows.insert((*table).to_owned(), result.rows_affected());
    }
    let preserved_after = table_counts(&mut tx, &preserved_tables).await?;
    if preserved_before != preserved_after {
        return Err(DbError::Init(
            "Agent-only reset changed preserved non-Agent configuration".to_owned(),
        ));
    }
    tx.commit().await?;
    Ok(AgentDataResetReport {
        deleted_rows,
        preserved_rows: preserved_after,
    })
}

async fn table_counts(
    tx: &mut Transaction<'_, Sqlite>,
    tables: &[String],
) -> Result<BTreeMap<String, u64>, DbError> {
    let mut counts = BTreeMap::new();
    for table in tables {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut **tx)
            .await?;
        let count = u64::try_from(count)
            .map_err(|_| DbError::Init(format!("negative row count for {table}")))?;
        counts.insert(table.clone(), count);
    }
    Ok(counts)
}
