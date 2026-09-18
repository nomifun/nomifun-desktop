//! One-time authenticated cutover from the last historical migration lineage.
//!
//! The old SQL files are deliberately not executable in this build. An
//! existing database is admitted only when every historical ledger checksum
//! and the normalized SQLite schema match the final pre-UARC lineage exactly.
//! The cutover then clears Agent-owned facts, removes retired tables, preserves
//! non-Agent configuration in place, and adopts the single canonical baseline.

use nomifun_agent_contracts::{agent_store_schema_manifest_payload, digest_payload};
use sha2::{Digest, Sha256};
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqliteRow;
use sqlx::{Connection, Row, SqliteConnection};

use crate::error::DbError;

const LEGACY_MIGRATION_COUNT: usize = 109;
const LEGACY_MIGRATION_HEAD: i64 = 112;
const LEGACY_LINEAGE_DIGEST: &str =
    "f4e999a6b7b792096b13887a360b9afdb684bfd9a807fff1e676b283254e8926";
const LEGACY_SCHEMA_DIGEST: &str =
    "537e5ba2cb420783f68a24c487fc2e4d4c6b742cfe166204ec19608bd58d9f50";

const READ_LEDGER: &str =
    "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version";

const RETIRED_AGENT_TABLES: &[&str] = &[
    "conversation_artifacts",
    "conversation_creation_keys",
    "conversation_creation_tasks",
    "conversation_delivery_notify",
    "conversation_delivery_receipts",
    "conversation_git_effects",
    "conversation_hosted_effects",
    "conversation_mcp_effects",
    "conversation_mcp_servers",
    "conversation_runtime_events",
    "conversations",
    "idmm_action_reservations",
    "idmm_interventions",
    "message_correlations",
    "messages",
    "nomi_agent_bindings",
    "nomi_agent_preset_revisions",
    "nomi_agent_presets",
    "schema_migrations",
];

const DELETE_AGENT_FACTS: &[&str] = &[
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

const REQUIREMENT_ACTIVE_TO_PENDING_TRIGGER: &str = r#"
CREATE TRIGGER trg_requirements_active_to_pending_pre_effect_guard
BEFORE UPDATE ON requirements
FOR EACH ROW
WHEN OLD.status = 'in_progress'
 AND NEW.status = 'pending'
 AND (
     NOT EXISTS (
         SELECT 1
           FROM requirement_pre_effect_abandon_guards AS guard
          WHERE guard.requirement_id = OLD.requirement_id
            AND guard.claim_generation = OLD.claim_generation
            AND guard.claim_token = OLD.claim_token
            AND guard.owner_conversation_id IS OLD.owner_conversation_id
            AND guard.owner_terminal_id IS OLD.owner_terminal_id
     )
     OR EXISTS (
         SELECT 1
           FROM agent_executions AS execution
          WHERE json_extract(execution.initial_plan_input, '$.mode') = 'automation'
            AND json_extract(execution.initial_plan_input, '$.source.requirement_id') = OLD.requirement_id
            AND json_extract(execution.initial_plan_input, '$.source.claim_generation') = OLD.claim_generation
     )
     OR EXISTS (
         SELECT 1
           FROM terminal_turn_admissions AS admission
          WHERE admission.requirement_id = OLD.requirement_id
            AND admission.claim_generation = OLD.claim_generation
     )
     OR NEW.claim_generation IS NOT OLD.claim_generation
     OR NEW.claim_token IS NOT NULL
     OR NEW.completion_note IS NOT NULL
     OR NEW.owner_conversation_id IS NOT NULL
     OR NEW.owner_terminal_id IS NOT NULL
     OR NEW.active_turn_started_at IS NOT NULL
     OR NEW.lease_expires_at IS NOT NULL
     OR NEW.started_at IS NOT OLD.started_at
     OR NEW.attempt_count IS NOT MAX(OLD.attempt_count - 1, 0)
 )
BEGIN
    SELECT RAISE(ABORT, 'active Requirement may become pending only through exact pre-effect abandon');
END
"#;

const REQUIREMENT_GUARD_INSERT_TRIGGER: &str = r#"
CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_insert
BEFORE INSERT ON requirement_pre_effect_abandon_guards
FOR EACH ROW
WHEN NOT EXISTS (
    SELECT 1
      FROM requirements AS requirement
     WHERE requirement.requirement_id = NEW.requirement_id
       AND requirement.status = 'in_progress'
       AND requirement.claim_generation = NEW.claim_generation
       AND requirement.claim_token = NEW.claim_token
       AND requirement.owner_conversation_id IS NEW.owner_conversation_id
       AND requirement.owner_terminal_id IS NEW.owner_terminal_id
       AND NOT EXISTS (
           SELECT 1
             FROM agent_executions AS execution
            WHERE json_extract(execution.initial_plan_input, '$.mode') = 'automation'
              AND json_extract(execution.initial_plan_input, '$.source.requirement_id') = requirement.requirement_id
              AND json_extract(execution.initial_plan_input, '$.source.claim_generation') = requirement.claim_generation
       )
       AND NOT EXISTS (
           SELECT 1
             FROM terminal_turn_admissions AS admission
            WHERE admission.requirement_id = requirement.requirement_id
              AND admission.claim_generation = requirement.claim_generation
       )
)
BEGIN
    SELECT RAISE(ABORT, 'Requirement pre-effect abandon guard requires exact authority and receiver-admission absence');
END
"#;

pub(super) fn is_authenticated_lineage(rows: &[SqliteRow]) -> Result<bool, DbError> {
    if rows.len() != LEGACY_MIGRATION_COUNT {
        return Ok(false);
    }
    let mut digest = Sha256::new();
    let mut head = None;
    for row in rows {
        let version: i64 = row.try_get("version").map_err(DbError::Query)?;
        let success: bool = row.try_get("success").map_err(DbError::Query)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(DbError::Query)?;
        if !success || checksum.len() != 48 {
            return Ok(false);
        }
        digest.update(version.to_be_bytes());
        digest.update([1]);
        digest.update(checksum);
        head = Some(version);
    }
    Ok(head == Some(LEGACY_MIGRATION_HEAD)
        && hex::encode(digest.finalize()) == LEGACY_LINEAGE_DIGEST)
}

pub(super) async fn validate_schema_on_pool(
    pool: &sqlx::SqlitePool,
) -> Result<(), DbError> {
    let rows = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
           AND name <> '_sqlx_migrations' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::Query)?;
    require_schema_digest(rows)
}

async fn validate_schema_on_connection(conn: &mut SqliteConnection) -> Result<(), DbError> {
    let rows = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
           AND name <> '_sqlx_migrations' ORDER BY type, name",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(DbError::Query)?;
    require_schema_digest(rows)
}

fn require_schema_digest(rows: Vec<SqliteRow>) -> Result<(), DbError> {
    let mut digest = Sha256::new();
    for row in rows {
        let kind: String = row.try_get("type").map_err(DbError::Query)?;
        let name: String = row.try_get("name").map_err(DbError::Query)?;
        let table: String = row.try_get("tbl_name").map_err(DbError::Query)?;
        let sql: String = row.try_get("sql").map_err(DbError::Query)?;
        let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        digest.update(kind.as_bytes());
        digest.update([0]);
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(table.as_bytes());
        digest.update([0]);
        digest.update(normalized.as_bytes());
        digest.update(b"\n");
    }
    let actual = hex::encode(digest.finalize());
    if actual != LEGACY_SCHEMA_DIGEST {
        return Err(DbError::Init(format!(
            "historical Agent cutover schema is not the authenticated pre-UARC shape: {actual}"
        )));
    }
    Ok(())
}

pub(super) async fn adopt_and_cut_over(
    conn: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<bool, DbError> {
    conn.ensure_migrations_table().await.map_err(DbError::Migration)?;
    let rows = sqlx::query(READ_LEDGER)
        .fetch_all(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    if !is_authenticated_lineage(&rows)? {
        return Ok(false);
    }
    validate_schema_on_connection(conn).await?;

    let migrations = migrator.iter().collect::<Vec<_>>();
    let [baseline] = migrations.as_slice() else {
        return Err(DbError::Init(
            "canonical database build must embed exactly one baseline migration".into(),
        ));
    };
    if baseline.version != 1 {
        return Err(DbError::Init(
            "canonical database baseline must use migration version 1".into(),
        ));
    }
    let schema_manifest_digest = digest_payload(&agent_store_schema_manifest_payload())
        .map_err(|error| DbError::Init(format!("digest Agent Store manifest: {error}")))?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .map_err(DbError::Query)?;
    let result = async {
        let mut tx = conn.begin().await.map_err(DbError::Query)?;
        let rows = sqlx::query(READ_LEDGER)
            .fetch_all(&mut *tx)
            .await
            .map_err(DbError::Query)?;
        if !is_authenticated_lineage(&rows)? {
            tx.rollback().await.map_err(DbError::Query)?;
            return Ok(false);
        }

        let mut suspended_trigger_sql = Vec::new();
        for trigger in [
            "trg_nomi_remote_events_append_only_delete",
            "trg_requirements_active_identity_exit_guard",
            "trg_requirements_pre_effect_abandon_guard_delete_guard",
        ] {
            let sql: String = sqlx::query_scalar(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?",
            )
            .bind(trigger)
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::Query)?;
            sqlx::query(&format!("DROP TRIGGER {trigger}"))
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
            suspended_trigger_sql.push(sql);
        }
        for trigger in [
            "trg_requirements_active_to_pending_pre_effect_guard",
            "trg_requirements_pre_effect_abandon_guard_insert",
        ] {
            sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        }
        sqlx::query("DELETE FROM requirement_pre_effect_abandon_guards")
            .execute(&mut *tx)
            .await
            .map_err(DbError::Query)?;
        sqlx::query(
            "UPDATE requirements SET status = 'needs_review', \
                completion_note = 'Agent history was cleared during the unified runtime cutover; review before retrying.', \
                owner_conversation_id = NULL, active_turn_started_at = NULL, \
                lease_expires_at = NULL, claim_token = NULL \
             WHERE owner_conversation_id IS NOT NULL",
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        sqlx::query("UPDATE channel_sessions SET conversation_id = NULL")
            .execute(&mut *tx)
            .await
            .map_err(DbError::Query)?;
        sqlx::query(
            "UPDATE cron_jobs SET conversation_id = NULL, conversation_title = NULL, \
                last_run_at = NULL, last_status = NULL, last_error = NULL, \
                run_count = 0, retry_count = 0",
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        sqlx::query("DELETE FROM knowledge_bindings WHERE target_kind = 'conversation'")
            .execute(&mut *tx)
            .await
            .map_err(DbError::Query)?;

        for table in DELETE_AGENT_FACTS {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        }
        sqlx::query(
            "DELETE FROM agent_metadata WHERE source_key <> 'agent_builtin_nomi' \
                OR agent_type <> 'nomi' OR agent_source <> 'internal'",
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;

        for table in RETIRED_AGENT_TABLES {
            sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        }

        for trigger_sql in suspended_trigger_sql {
            sqlx::query(&trigger_sql)
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        }
        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_agent_presets_owner_active \
             ON agent_presets(json_extract(owner_ref_json, '$.user_id'), preset_id) \
             WHERE retired_at_ms IS NULL",
            "CREATE INDEX IF NOT EXISTS idx_agent_presets_ui_plugin \
             ON agent_presets(json_extract(display_json, '$.ui_binding.selection.plugin_id'))",
            "CREATE INDEX IF NOT EXISTS idx_agent_preset_revisions_created_by \
             ON agent_preset_revisions(created_by)",
            "CREATE INDEX IF NOT EXISTS idx_agent_bindings_preset \
             ON agent_bindings(json_extract(agent_binding_json, '$.preset_revision_ref.preset_id'))",
            "CREATE INDEX IF NOT EXISTS idx_agent_runtime_snapshots_revision \
             ON agent_runtime_snapshots( \
                json_extract(content_json, '$.preset_revision_ref.preset_id'), \
                json_extract(content_json, '$.preset_revision_ref.revision'), \
                json_extract(content_json, '$.preset_revision_ref.revision_digest'))",
            REQUIREMENT_ACTIVE_TO_PENDING_TRIGGER,
            REQUIREMENT_GUARD_INSERT_TRIGGER,
        ] {
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        }
        sqlx::query(
            "UPDATE schema_metadata SET root_instance_id = 'main-sqlite-agent-store', \
                data_generation = 5, migration_head = 1, \
                canonical_schema_manifest_digest = ?, projection_schema_version = 1 \
             WHERE singleton_key = 'canonical'",
        )
        .bind(schema_manifest_digest.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;

        sqlx::query("DELETE FROM _sqlx_migrations")
            .execute(&mut *tx)
            .await
            .map_err(DbError::Query)?;
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
                (version, description, success, checksum, execution_time) \
             VALUES (?, ?, 1, ?, 0)",
        )
        .bind(baseline.version)
        .bind(baseline.description.as_ref())
        .bind(baseline.checksum.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await.map_err(DbError::Query)?;
        Ok(true)
    }
    .await;
    let foreign_keys = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .map_err(DbError::Query);
    match (result, foreign_keys) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
