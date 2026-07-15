//! Runtime assertions for the clean ID-contract-v2 schema.
//!
//! The migration is intentionally a hard-cut baseline. These checks keep
//! accidental reintroduction of integer entity keys, foreign keys, or SQLite
//! auto-increment allocation from passing unnoticed when the schema is edited.
//! Natural keys and external/operation identifiers are deliberately outside
//! this registry. Explicit entity-or-natural-key unions are tracked separately
//! so their text storage is protected without falsely describing every value
//! as an entity reference.

use std::collections::BTreeSet;

use sqlx::{Row, SqlitePool};

use crate::error::DbError;

/// Durable entity primary-key columns. Composite-scoped entities still list
/// their entity component (`id`); the owning FK is registered separately
/// below.
const ENTITY_PRIMARY_KEYS: &[(&str, &str)] = &[
    ("agent_execution_attempts", "id"),
    ("agent_execution_events", "id"),
    ("agent_execution_participants", "id"),
    ("agent_execution_steps", "id"),
    ("agent_execution_template_participants", "id"),
    ("agent_execution_templates", "id"),
    ("agent_executions", "id"),
    ("agent_metadata", "id"),
    ("attachments", "id"),
    ("channel_plugins", "id"),
    ("channel_sessions", "id"),
    ("channel_users", "id"),
    ("connector_credentials", "id"),
    ("conversation_artifacts", "id"),
    ("conversation_execution_links", "id"),
    ("conversations", "id"),
    ("creation_tasks", "id"),
    ("cron_job_runs", "id"),
    ("cron_jobs", "id"),
    ("idmm_interventions", "id"),
    ("knowledge_bases", "id"),
    ("knowledge_bindings", "binding_id"),
    ("mcp_servers", "id"),
    ("messages", "id"),
    ("presets", "id"),
    ("preset_tags", "key"),
    ("providers", "id"),
    ("remote_agents", "id"),
    ("requirements", "id"),
    ("terminal_sessions", "id"),
    ("users", "id"),
    ("webhooks", "id"),
    ("workshop_assets", "id"),
    ("workshop_canvases", "id"),
];

/// Columns that store a NomiFun entity reference. This includes declared
/// SQLite foreign keys plus intentionally polymorphic or cross-store
/// references whose owner cannot be expressed as one SQLite FK.
const ENTITY_REFERENCE_COLUMNS: &[(&str, &str)] = &[
    ("acp_session", "conversation_id"),
    ("acp_session", "agent_id"),
    ("agent_execution_attempts", "execution_id"),
    ("agent_execution_attempts", "step_id"),
    ("agent_execution_attempts", "participant_id"),
    ("agent_execution_events", "execution_id"),
    ("agent_execution_events", "step_id"),
    ("agent_execution_events", "attempt_id"),
    ("agent_execution_events", "actor_conversation_id"),
    ("agent_execution_events", "actor_attempt_id"),
    ("agent_execution_events", "on_behalf_of_user_id"),
    ("agent_execution_participants", "execution_id"),
    ("agent_execution_participants", "source_agent_id"),
    ("agent_execution_participants", "preset_id"),
    ("agent_execution_participants", "provider_id"),
    ("agent_execution_step_dependencies", "execution_id"),
    ("agent_execution_step_dependencies", "blocker_step_id"),
    ("agent_execution_step_dependencies", "blocked_step_id"),
    ("agent_execution_steps", "execution_id"),
    ("agent_execution_steps", "assigned_participant_id"),
    ("agent_execution_template_participants", "template_id"),
    ("agent_execution_template_participants", "source_agent_id"),
    ("agent_execution_template_participants", "preset_id"),
    ("agent_execution_template_participants", "provider_id"),
    ("agent_execution_templates", "user_id"),
    ("agent_execution_templates", "primary_participant_id"),
    ("agent_executions", "user_id"),
    ("attachments", "requirement_id"),
    ("channel_pairing_codes", "channel_id"),
    ("channel_plugins", "companion_id"),
    ("channel_plugins", "public_agent_id"),
    ("channel_sessions", "user_id"),
    ("channel_sessions", "conversation_id"),
    ("channel_sessions", "channel_id"),
    ("channel_users", "channel_id"),
    ("channel_users", "session_id"),
    ("companion_access_token", "companion_id"),
    ("conversation_artifacts", "conversation_id"),
    ("conversation_artifacts", "cron_job_id"),
    ("conversation_creation_keys", "user_id"),
    ("conversation_creation_keys", "conversation_id"),
    ("conversation_delivery_receipts", "conversation_id"),
    ("conversation_delivery_receipts", "message_id"),
    ("conversation_delivery_receipts", "user_id"),
    ("conversation_execution_links", "conversation_id"),
    ("conversation_execution_links", "execution_id"),
    ("conversation_execution_links", "step_id"),
    ("conversation_execution_links", "attempt_id"),
    ("conversation_mcp_servers", "conversation_id"),
    ("conversation_mcp_servers", "mcp_server_id"),
    ("conversations", "user_id"),
    ("conversations", "cron_job_id"),
    ("conversations", "preset_id"),
    ("conversations", "execution_template_id"),
    ("creation_tasks", "canvas_id"),
    ("creation_tasks", "provider_id"),
    ("cron_job_runs", "job_id"),
    ("cron_jobs", "user_id"),
    ("cron_jobs", "preset_id"),
    ("cron_jobs", "conversation_id"),
    ("idmm_interventions", "user_id"),
    ("idmm_interventions", "target_id"),
    ("installation_identity", "owner_user_id"),
    ("knowledge_binding_bases", "binding_id"),
    ("knowledge_binding_bases", "kb_id"),
    ("knowledge_bindings", "target_conv_id"),
    ("knowledge_bindings", "target_term_id"),
    ("knowledge_bindings", "target_companion_id"),
    ("message_correlations", "conversation_id"),
    ("message_correlations", "turn_message_id"),
    ("message_correlations", "message_id"),
    ("messages", "conversation_id"),
    ("messages", "msg_id"),
    ("model_profiles", "provider_id"),
    ("preset_agent_preferences", "preset_id"),
    ("preset_agent_preferences", "agent_id"),
    ("preset_examples", "preset_id"),
    ("preset_knowledge_bases", "preset_id"),
    ("preset_knowledge_bases", "knowledge_base_id"),
    ("preset_knowledge_policy", "preset_id"),
    ("preset_localizations", "preset_id"),
    ("preset_model_preferences", "preset_id"),
    ("preset_model_preferences", "provider_id"),
    ("preset_skill_bindings", "preset_id"),
    ("preset_tag_bindings", "preset_id"),
    ("preset_targets", "preset_id"),
    ("preset_user_state", "preset_id"),
    ("preset_user_state", "preferred_agent_id"),
    ("requirement_tags", "paused_req_id"),
    ("requirements", "owner_conversation_id"),
    ("requirements", "owner_terminal_id"),
    ("tag_settings", "webhook_id"),
    ("terminal_scrollback", "session_id"),
    ("terminal_sessions", "user_id"),
];

/// Columns whose contract is an explicit union of a durable entity ID and a
/// natural key. `preset_tag_bindings.tag_key` stores either a canonical
/// `presettag_` user-tag ID or a stable builtin manifest vocabulary key; it is
/// intentionally not a single-domain foreign key.
const ENTITY_OR_NATURAL_KEY_UNION_COLUMNS: &[(&str, &str)] =
    &[("preset_tag_bindings", "tag_key")];

/// Numeric primary keys are only permitted for true singleton discriminators,
/// never for entities. Keep this whitelist intentionally tiny and explicit.
const INTEGER_PRIMARY_KEY_ALLOWLIST: &[(&str, &str)] = &[("system_settings", "id")];

/// Validate all durable entity key columns and the SQLite allocation policy.
pub async fn validate_id_schema_contract(pool: &SqlitePool) -> Result<(), DbError> {
    for (table, column) in ENTITY_PRIMARY_KEYS {
        require_text_column(pool, table, column, "entity key").await?;
    }
    for (table, column) in ENTITY_REFERENCE_COLUMNS {
        require_text_column(pool, table, column, "entity reference").await?;
    }
    for (table, column) in ENTITY_OR_NATURAL_KEY_UNION_COLUMNS {
        require_text_column(pool, table, column, "entity-or-natural-key union").await?;
    }
    validate_no_autoincrement(pool).await?;
    validate_integer_primary_key_allowlist(pool).await?;
    Ok(())
}

/// Validate persisted ID values at an offline trust boundary.
///
/// Schema affinity and CHECK expressions are defense in depth, but a crafted
/// SQLite bundle can rewrite both data and checksums. Backup/open and restore
/// therefore scan every declared durable PK/FK value and require canonical
/// `{prefix}_{lowercase UUIDv7}` syntax before accepting the dataset.
pub(crate) async fn validate_id_value_contract(pool: &SqlitePool) -> Result<(), DbError> {
    let natural_agent_keys: BTreeSet<String> = sqlx::query_scalar(
        "SELECT id FROM agent_metadata WHERE id LIKE 'agent_builtin_%'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    let natural_preset_keys: BTreeSet<String> = sqlx::query_scalar(
        "SELECT id FROM presets WHERE source_kind IN ('builtin', 'extension')",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    for value in natural_agent_keys.iter().chain(&natural_preset_keys) {
        validate_natural_installation_key(value).map_err(|reason| {
            DbError::Init(format!(
                "ID contract natural installation key {value:?} is invalid: {reason}"
            ))
        })?;
    }

    let mut columns = BTreeSet::new();
    columns.extend(ENTITY_PRIMARY_KEYS.iter().copied());
    columns.extend(ENTITY_REFERENCE_COLUMNS.iter().copied());
    for (table, column) in columns {
        let sql = format!(
            "SELECT {} AS entity_id FROM {} WHERE {} IS NOT NULL",
            quote_sqlite_identifier(column),
            quote_sqlite_identifier(table),
            quote_sqlite_identifier(column)
        );
        let values: Vec<String> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
        for value in values {
            if let Err(reason) = validate_generic_entity_id(&value) {
                let allowed_natural_key = (is_agent_identity_column(table, column)
                    && natural_agent_keys.contains(&value))
                    || (is_preset_identity_column(table, column)
                        && natural_preset_keys.contains(&value));
                if !allowed_natural_key {
                    return Err(DbError::Init(format!(
                        "ID contract value {table}.{column}={value:?} is not canonical: {reason}"
                    )));
                }
            }
        }
    }

    for (table, column) in ENTITY_OR_NATURAL_KEY_UNION_COLUMNS {
        let sql = format!(
            "SELECT {} AS union_id FROM {} WHERE {} IS NOT NULL",
            quote_sqlite_identifier(column),
            quote_sqlite_identifier(table),
            quote_sqlite_identifier(column)
        );
        let values: Vec<String> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
        for value in values {
            // Builtin preset tag vocabulary keys are natural keys. Only the
            // entity arm of this explicit union carries the presettag_ prefix.
            if value.starts_with("presettag_") {
                nomifun_common::validate_prefixed_id(&value, "presettag").map_err(|error| {
                    DbError::Init(format!(
                        "ID contract union value {table}.{column}={value:?} is not canonical: {error}"
                    ))
                })?;
            }
        }
    }
    Ok(())
}

fn is_agent_identity_column(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        ("agent_metadata", "id")
            | ("acp_session", "agent_id")
            | ("agent_execution_participants", "source_agent_id")
            | (
                "agent_execution_template_participants",
                "source_agent_id"
            )
            | ("preset_agent_preferences", "agent_id")
            | ("preset_user_state", "preferred_agent_id")
    )
}

fn is_preset_identity_column(table: &str, column: &str) -> bool {
    (table == "presets" && column == "id") || column == "preset_id"
}

fn validate_natural_installation_key(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 255 {
        return Err("must contain between 1 and 255 bytes");
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'-' | b'.' | b':')
    }) {
        return Err("contains characters outside the stable natural-key alphabet");
    }
    Ok(())
}

fn validate_generic_entity_id(value: &str) -> Result<(), String> {
    let Some((prefix, uuid_body)) = value.split_once('_') else {
        return Err("missing prefix separator".into());
    };
    nomifun_common::validate_id_prefix(prefix).map_err(|error| error.to_string())?;
    nomifun_common::validate_uuidv7(uuid_body).map_err(|error| error.to_string())?;
    Ok(())
}

async fn require_text_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    role: &str,
) -> Result<(), DbError> {
    let sql = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let Some(row) = rows
        .iter()
        .find(|row| row.try_get::<String, _>("name").ok().as_deref() == Some(column))
    else {
        return Err(DbError::Init(format!(
            "ID contract table {table} is missing {role} column {column}"
        )));
    };
    let data_type = row
        .try_get::<String, _>("type")
        .map_err(DbError::Query)?
        .to_ascii_uppercase();
    if data_type != "TEXT" {
        return Err(DbError::Init(format!(
            "ID contract {role} column {table}.{column} must be TEXT, found {data_type}"
        )));
    }
    Ok(())
}

async fn validate_no_autoincrement(pool: &SqlitePool) -> Result<(), DbError> {
    let rows = sqlx::query(
        "SELECT name, sql FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND sql IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        let table: String = row.try_get("name").map_err(DbError::Query)?;
        let create_sql: String = row.try_get("sql").map_err(DbError::Query)?;
        if create_sql.to_ascii_uppercase().contains("AUTOINCREMENT") {
            return Err(DbError::Init(format!(
                "ID contract forbids AUTOINCREMENT; found it in table {table}"
            )));
        }
    }
    Ok(())
}

async fn validate_integer_primary_key_allowlist(pool: &SqlitePool) -> Result<(), DbError> {
    let allowlist: BTreeSet<(String, String)> = INTEGER_PRIMARY_KEY_ALLOWLIST
        .iter()
        .map(|(table, column)| ((*table).to_owned(), (*column).to_owned()))
        .collect();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    let mut seen_allowlisted = BTreeSet::new();
    for table in tables {
        let sql = format!("PRAGMA table_info({})", quote_sqlite_identifier(&table));
        let rows = sqlx::query(&sql).fetch_all(pool).await?;
        let primary_key_columns: Vec<(i64, String, String)> = rows
            .iter()
            .filter_map(|row| {
                let position = row.try_get::<i64, _>("pk").ok()?;
                if position == 0 {
                    return None;
                }
                let name = row.try_get::<String, _>("name").ok()?;
                let data_type = row.try_get::<String, _>("type").ok()?.to_ascii_uppercase();
                Some((position, name, data_type))
            })
            .collect();
        // Numeric columns are perfectly valid components of a composite
        // natural/relational key (e.g. revision, rank, sequence). The
        // forbidden form is specifically a *single-column* INTEGER PRIMARY
        // KEY, which aliases SQLite's mutable rowid allocator.
        if primary_key_columns.len() == 1
            && primary_key_columns[0].2 == "INTEGER"
        {
            let column = &primary_key_columns[0].1;
            let key = (table.clone(), column.clone());
            if !allowlist.contains(&key) {
                return Err(DbError::Init(format!(
                    "ID contract forbids INTEGER primary key {table}.{column}"
                )));
            }
            seen_allowlisted.insert(key);
        }
    }
    if seen_allowlisted != allowlist {
        let missing = allowlist
            .difference(&seen_allowlisted)
            .map(|(table, column)| format!("{table}.{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DbError::Init(format!(
            "ID contract integer-primary-key allowlist is stale; missing {missing}"
        )));
    }
    Ok(())
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    #[tokio::test]
    async fn clean_baseline_satisfies_entity_id_contract() {
        let database = init_database_memory().await.expect("database");
        validate_id_schema_contract(database.pool())
            .await
            .expect("all entity keys and references remain TEXT");
    }

    #[tokio::test]
    async fn contract_rejects_autoincrement_and_unapproved_integer_primary_keys() {
        let pool = SqlitePool::connect("sqlite::memory:").await.expect("pool");
        sqlx::raw_sql(
            "CREATE TABLE system_settings (id INTEGER PRIMARY KEY CHECK(id = 1)); \
             CREATE TABLE bad_entity (id INTEGER PRIMARY KEY AUTOINCREMENT)",
        )
            .execute(&pool)
            .await
            .expect("schema");
        let error = validate_no_autoincrement(&pool)
            .await
            .expect_err("AUTOINCREMENT entity must fail");
        assert!(error.to_string().contains("AUTOINCREMENT"));
    }

    #[tokio::test]
    async fn contract_rejects_non_allowlisted_integer_primary_keys() {
        let pool = SqlitePool::connect("sqlite::memory:").await.expect("pool");
        sqlx::raw_sql(
            "CREATE TABLE system_settings (id INTEGER PRIMARY KEY CHECK(id = 1)); \
             CREATE TABLE bad_entity (id INTEGER PRIMARY KEY)",
        )
        .execute(&pool)
        .await
        .expect("schema");
        let error = validate_integer_primary_key_allowlist(&pool)
            .await
            .expect_err("integer entity key must fail");
        assert!(error.to_string().contains("bad_entity.id"));
    }
}
