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
    "agent_deletion_audits",
    "agent_sessions",
    "agent_bindings",
    "agent_runtime_snapshots",
    "agent_preset_contribution_locks",
    "agent_preset_revisions",
    "agent_presets",
    "agent_preset_templates",
    "remote_bindings",
];

/// Product-owned facts whose lifetime is bounded by an Agent configuration,
/// Session or Execution but whose schema remains owned by another domain.
const SESSION_BOUND_RESET_ORDER: &[&str] = &[
    "conversation_execution_links",
    "agent_execution_events",
    "agent_execution_attempts",
    "agent_execution_step_dependencies",
    "agent_execution_steps",
    "agent_execution_participants",
    "agent_executions",
    "agent_execution_template_participants",
    "agent_execution_templates",
    "nomi_remote_events",
    "nomi_remote_sessions",
    "nomi_wave1_memory_action_receipts",
    "nomi_wave4_action_receipts",
    "plugin_surface_sessions",
    "creative_studio_agent_proposal_receipts",
    "creative_studio_agent_sessions",
    "creation_tasks",
    "channel_pending_prompts",
    "cron_run_reservations",
    "cron_job_runs",
    "product_agent_selections",
];

const RESET_SUSPENDED_TRIGGERS: &[&str] = &[
    "trg_nomi_remote_events_append_only_delete",
    "trg_requirements_active_identity_exit_guard",
    "trg_requirements_pre_effect_abandon_guard_delete_guard",
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
    let mut suspended_trigger_sql = Vec::new();
    for trigger in RESET_SUSPENDED_TRIGGERS {
        let sql: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?",
        )
        .bind(trigger)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(sql) = sql {
            sqlx::query(&format!("DROP TRIGGER {trigger}"))
                .execute(&mut *tx)
                .await?;
            suspended_trigger_sql.push(sql);
        }
    }
    let preserved_tables = preserved_tables
        .into_iter()
        .filter(|table| existing_tables.contains(table))
        .collect::<Vec<_>>();
    let preserved_before = table_counts(&mut tx, &preserved_tables).await?;
    if existing_tables.contains("requirement_pre_effect_abandon_guards") {
        sqlx::query("DELETE FROM requirement_pre_effect_abandon_guards")
            .execute(&mut *tx)
            .await?;
    }
    let requirements_released = if existing_tables.contains("requirements") {
        sqlx::query(
            "UPDATE requirements SET status = 'needs_review', \
                completion_note = 'Agent history was cleared; review before retrying.', \
                owner_conversation_id = NULL, active_turn_started_at = NULL, \
                lease_expires_at = NULL, claim_token = NULL \
             WHERE owner_conversation_id IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };
    let channel_sessions_detached = if existing_tables.contains("channel_sessions") {
        sqlx::query(
            "UPDATE channel_sessions SET conversation_id = NULL WHERE conversation_id IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };
    let cron_jobs_detached = if existing_tables.contains("cron_jobs") {
        sqlx::query(
            "UPDATE cron_jobs SET conversation_id = NULL, conversation_title = NULL, \
                last_run_at = NULL, last_status = NULL, last_error = NULL, \
                run_count = 0, retry_count = 0 \
             WHERE conversation_id IS NOT NULL OR last_run_at IS NOT NULL \
                OR last_status IS NOT NULL OR last_error IS NOT NULL \
                OR run_count <> 0 OR retry_count <> 0",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };
    let knowledge_session_bindings = if existing_tables.contains("knowledge_bindings") {
        sqlx::query("DELETE FROM knowledge_bindings WHERE target_kind = 'conversation'")
            .execute(&mut *tx)
            .await?
            .rows_affected()
    } else {
        0
    };

    let mut deleted_rows = BTreeMap::new();
    deleted_rows.insert(
        "requirements.agent_claims_released".to_owned(),
        requirements_released,
    );
    deleted_rows.insert(
        "channel_sessions.agent_links_cleared".to_owned(),
        channel_sessions_detached,
    );
    deleted_rows.insert(
        "cron_jobs.agent_runtime_cleared".to_owned(),
        cron_jobs_detached,
    );
    deleted_rows.insert(
        "knowledge_bindings.session_rows".to_owned(),
        knowledge_session_bindings,
    );
    for table in SESSION_BOUND_RESET_ORDER {
        if !existing_tables.contains(*table) {
            continue;
        }
        let result = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
        deleted_rows.insert((*table).to_owned(), result.rows_affected());
    }
    sqlx::query("UPDATE agent_events SET causation_event_id = NULL")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE agent_sessions SET parent_agent_session_id = NULL")
        .execute(&mut *tx)
        .await?;

    for table in RESET_ORDER {
        let result = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
        deleted_rows.insert((*table).to_owned(), result.rows_affected());
    }
    let removed_custom_agents = if existing_tables.contains("agent_metadata") {
        sqlx::query(
            "DELETE FROM agent_metadata WHERE source_key <> 'agent_builtin_nomi' \
                OR agent_type <> 'nomi' OR agent_source <> 'internal'",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };
    deleted_rows.insert("agent_metadata.custom_rows".to_owned(), removed_custom_agents);
    for trigger_sql in suspended_trigger_sql {
        sqlx::query(&trigger_sql).execute(&mut *tx).await?;
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
