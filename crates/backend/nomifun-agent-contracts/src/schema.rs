use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const AGENT_STORE_DATA_GENERATION: u32 = 6;
pub const AGENT_STORE_MIGRATION_HEAD: u32 = 6;
pub const AGENT_STORE_PROJECTION_SCHEMA_VERSION: u32 = 1;
pub const AGENT_STORE_BASELINE_SQL: &str = include_str!("../schema/0001_agent_store.sql");
pub const CHAT_ROUTE_RECORD_JSON_SCHEMA: &str =
    include_str!("../schema/chat-route-record.v1.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaTableContract {
    pub table_name: String,
    pub owner: String,
    pub fact_class: String,
    pub reset_scope: SchemaResetScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaResetScope {
    Preserve,
    AgentData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentStoreSchemaManifestPayload {
    pub schema_version: String,
    pub data_generation: u32,
    pub migration_head: u32,
    pub projection_schema_version: u32,
    pub baseline_logical_path: String,
    pub tables: Vec<SchemaTableContract>,
    pub forbidden_table_names: Vec<String>,
}

pub fn agent_store_schema_manifest_payload() -> AgentStoreSchemaManifestPayload {
    AgentStoreSchemaManifestPayload {
        schema_version: "1.0.0".to_owned(),
        data_generation: AGENT_STORE_DATA_GENERATION,
        migration_head: AGENT_STORE_MIGRATION_HEAD,
        projection_schema_version: AGENT_STORE_PROJECTION_SCHEMA_VERSION,
        baseline_logical_path:
            "crates/backend/nomifun-agent-contracts/schema/0001_agent_store.sql".to_owned(),
        tables: TABLES
            .iter()
            .map(|(table_name, owner, fact_class, reset_scope)| SchemaTableContract {
                table_name: (*table_name).to_owned(),
                owner: (*owner).to_owned(),
                fact_class: (*fact_class).to_owned(),
                reset_scope: *reset_scope,
            })
            .collect(),
        forbidden_table_names: FORBIDDEN_TABLE_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    }
}
const TABLES: &[(&str, &str, &str, SchemaResetScope)] = &[
    ("schema_metadata", "platform.schema", "fact", SchemaResetScope::Preserve),
    ("plugin_packages", "platform.plugin-manager", "fact", SchemaResetScope::Preserve),
    ("plugin_mounts", "platform.plugin-manager", "fact", SchemaResetScope::Preserve),
    ("plugin_configs", "platform.plugin-manager", "fact", SchemaResetScope::Preserve),
    ("plugin_states", "platform.plugin-manager", "fact", SchemaResetScope::Preserve),
    ("installation_role_bindings", "platform.capability-registry", "fact", SchemaResetScope::Preserve),
    ("capability_definitions", "platform.capability-registry", "fact", SchemaResetScope::Preserve),
    ("capability_catalog_entries", "platform.capability-registry", "fact", SchemaResetScope::Preserve),
    ("skill_instructions", "platform.skill-catalog", "fact", SchemaResetScope::Preserve),
    ("mcp_servers", "plugin.mcp-connectors", "fact", SchemaResetScope::Preserve),
    ("mcp_tool_materializations", "plugin.mcp-connectors", "fact", SchemaResetScope::Preserve),
    ("agent_preset_templates", "platform.agent-preset", "configuration", SchemaResetScope::AgentData),
    ("agent_presets", "platform.agent-preset", "configuration", SchemaResetScope::AgentData),
    ("agent_preset_revisions", "platform.agent-preset", "configuration", SchemaResetScope::AgentData),
    ("agent_preset_contribution_locks", "platform.agent-preset", "configuration", SchemaResetScope::AgentData),
    ("agent_bindings", "platform.agent-preset", "configuration", SchemaResetScope::AgentData),
    ("remote_bindings", "plugin.remote-ingress", "configuration", SchemaResetScope::AgentData),
    ("installation_auth", "plugin.remote-ingress", "fact", SchemaResetScope::Preserve),
    ("providers", "platform.chat-model-broker", "configuration", SchemaResetScope::Preserve),
    ("provider_models", "platform.chat-model-broker", "configuration", SchemaResetScope::Preserve),
    ("provider_connections", "platform.chat-model-broker", "configuration", SchemaResetScope::Preserve),
    ("provider_model_capabilities", "platform.chat-model-broker", "configuration", SchemaResetScope::Preserve),
    ("client_preferences", "platform.host-configuration", "configuration", SchemaResetScope::Preserve),
    ("system_settings", "platform.host-configuration", "configuration", SchemaResetScope::Preserve),
    ("agent_runtime_snapshots", "platform.agent-preset-compiler", "fact", SchemaResetScope::AgentData),
    ("agent_sessions", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_deletion_audits", "platform.agent-session", "audit", SchemaResetScope::AgentData),
    ("agent_turns", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_events", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_payloads", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_effects", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_session_resources", "platform.agent-session", "fact", SchemaResetScope::AgentData),
    ("agent_session_heads", "platform.agent-session", "projection", SchemaResetScope::AgentData),
    ("agent_messages", "platform.agent-session", "projection", SchemaResetScope::AgentData),
];

const FORBIDDEN_TABLE_NAMES: &[&str] = &[
    "conversations",
    "messages",
    "conversation_delivery_receipts",
    "conversation_runtime_events",
    "conversation_mcp_effects",
    "conversation_hosted_effects",
    "conversation_git_effects",
    "conversation_sessions",
    "session_events",
    "session_payloads",
    "session_heads",
    "message_projection",
    "runtime_contributions",
    "service_catalog",
    "remote_agents",
    "remote_sessions",
    "session_retention",
    "session_restore",
    "test_sessions",
    "test_revisions",
    "effect_coordinator",
    "runtime_event_store",
    "legacy_imports",
    "migration_reports",
    "capability_packs",
    "capability_pack_items",
    "agent_preset_model_routes",
    "preset_initial_capabilities",
    "preset_on_demand_capabilities",
    "preset_skill_bindings",
    "preset_resource_bindings",
    "agent_runtime_snapshot_capabilities",
    "agent_runtime_profiles",
    "agent_preset_audit_events",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rusqlite::Connection;

    use super::*;

    #[test]
    fn agent_store_baseline_builds_from_an_empty_database() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");

        let actual = database
            .prepare(
                "SELECT name FROM sqlite_schema \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .expect("table query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("table rows")
            .collect::<Result<BTreeSet<_>, _>>()
            .expect("table names");
        let expected = TABLES
            .iter()
            .map(|(name, _, _, _)| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn capability_tables_use_stable_ids_and_package_provenance() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");
        for (table, expected) in [
            (
                "capability_definitions",
                vec![
                    "capability_id",
                    "package_id",
                    "package_version",
                    "manifest_json",
                    "manifest_digest",
                ],
            ),
            (
                "capability_catalog_entries",
                vec![
                    "capability_id",
                    "contribution_id",
                    "entry_json",
                    "entry_digest",
                ],
            ),
            (
                "mcp_tool_materializations",
                vec![
                    "server_id",
                    "canonical_tool_key",
                    "schema_hash",
                    "capability_id",
                    "materialization_revision",
                    "package_id",
                    "package_version",
                ],
            ),
        ] {
            let sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
            let actual = database
                .prepare(&sql)
                .expect("table-info query")
                .query_map([], |row| row.get::<_, String>(0))
                .expect("table-info rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("column names");
            assert_eq!(actual, expected, "{table}");
        }
    }

    #[test]
    fn every_table_stays_within_the_physical_index_budget() {
        const MAX_INDEXES_PER_TABLE: usize = 5;

        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");

        for (table, _, _, _) in TABLES {
            let quoted_table = table.replace('"', "\"\"");
            let sql = format!("PRAGMA index_list(\"{quoted_table}\")");
            let names = database
                .prepare(&sql)
                .expect("index-list query")
                .query_map([], |row| row.get::<_, String>(1))
                .expect("index-list rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("index names");
            assert!(
                names.len() <= MAX_INDEXES_PER_TABLE,
                "{table} exceeds the {MAX_INDEXES_PER_TABLE}-index budget: {names:?}"
            );
        }
    }

    #[test]
    fn schema_contract_contains_no_legacy_table() {
        let lowercase = AGENT_STORE_BASELINE_SQL.to_ascii_lowercase();
        for table in FORBIDDEN_TABLE_NAMES {
            assert!(
                !lowercase.contains(&format!("create table {table}")),
                "Agent Store baseline must not create {table}"
            );
        }
    }

    #[test]
    fn every_table_has_one_owner_and_class() {
        let names = TABLES
            .iter()
            .map(|(name, _, _, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), TABLES.len());
        assert!(
            TABLES
                .iter()
                .all(|(_, owner, class, _)| !owner.is_empty() && !class.is_empty())
        );
    }

    #[test]
    fn reset_scope_is_total_and_preserves_only_non_agent_configuration() {
        let agent_tables = TABLES
            .iter()
            .filter(|(_, _, _, scope)| *scope == SchemaResetScope::AgentData)
            .map(|(name, _, _, _)| *name)
            .collect::<BTreeSet<_>>();
        for required in [
            "agent_sessions",
            "agent_turns",
            "agent_events",
            "agent_messages",
            "agent_payloads",
            "agent_effects",
            "agent_session_resources",
            "agent_presets",
            "agent_preset_revisions",
            "agent_runtime_snapshots",
        ] {
            assert!(agent_tables.contains(required), "{required} must reset");
        }
        for preserved in [
            "plugin_packages",
            "mcp_servers",
            "providers",
            "provider_models",
            "client_preferences",
            "system_settings",
        ] {
            assert!(TABLES.iter().any(|(name, _, _, scope)| {
                *name == preserved && *scope == SchemaResetScope::Preserve
            }));
        }
    }

    #[test]
    fn runtime_snapshot_stores_only_canonical_content_and_envelope() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");

        let columns = database
            .prepare("PRAGMA table_info(agent_runtime_snapshots)")
            .expect("snapshot table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("snapshot columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("snapshot column names");
        assert_eq!(
            columns,
            [
                "snapshot_id",
                "snapshot_digest",
                "content_json",
                "envelope_json"
            ]
        );
    }

    #[test]
    fn payload_schema_supports_only_inline_or_content_addressed_objects() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");
        database
            .execute(
                "INSERT INTO agent_sessions (agent_session_id, owner_ref_json, state, deleted_at) \
                 VALUES ('session-deleted', '{}', 'deleted', 1)",
                [],
            )
            .unwrap();
        let digest = "a".repeat(64);
        let object_ref = format!("objects/{digest}");
        database
            .execute(
                "INSERT INTO agent_payloads (\
                    payload_id, session_id, media_type, byte_len, digest, \
                    storage_kind, body, object_ref\
                 ) VALUES (?1, 'session-deleted', 'application/octet-stream', 3, ?2, \
                    'object', NULL, ?3)",
                rusqlite::params!["payload-object", digest, object_ref],
            )
            .unwrap();
        assert!(database
            .execute(
                "INSERT INTO agent_payloads (\
                    payload_id, session_id, media_type, byte_len, digest, \
                    storage_kind, body, object_ref\
                 ) VALUES ('payload-invalid', 'session-deleted', 'text/plain', 1, ?1, \
                    'object', NULL, 'objects/wrong')",
                rusqlite::params!["b".repeat(64)],
            )
            .is_err());
    }

    #[test]
    fn installation_role_binding_is_one_exact_selection_per_role() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(AGENT_STORE_BASELINE_SQL)
            .expect("Agent Store baseline");

        let columns = database
            .prepare("PRAGMA table_info(installation_role_bindings)")
            .expect("role binding table info")
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
            })
            .expect("role binding columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("role binding column names");
        assert_eq!(
            columns,
            [
                ("role_id".to_owned(), 1),
                ("role_contract_ref_json".to_owned(), 0),
                ("provider_mount_id".to_owned(), 0),
                ("binding_version".to_owned(), 0),
                ("updated_at".to_owned(), 0),
            ]
        );
        let row_count: i64 = database
            .query_row("SELECT COUNT(*) FROM installation_role_bindings", [], |row| {
                row.get(0)
            })
            .expect("role binding row count");
        assert_eq!(row_count, 0);
    }

    #[test]
    fn chat_route_record_schema_is_strict_and_compilable() {
        let schema: serde_json::Value =
            serde_json::from_str(CHAT_ROUTE_RECORD_JSON_SCHEMA).expect("route JSON schema");
        let validator = jsonschema::options()
            .build(&schema)
            .expect("route JSON schema must compile");
        let record = serde_json::json!({
            "schema": "nomifun.chat-route-record.v1",
            "task": "agent_chat",
            "primary": {
                "model_route_id": "route-1",
                "model_route_revision": 1,
                "provider_id": "provider-1",
                "model": "model-1",
                "protocol": "openai_chat",
                "connection_config_ref": "connection-1",
                "config_revision_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "credential_ref": "credential-1",
                "features": ["text_input", "text_output"]
            },
            "failovers": []
        });
        assert!(validator.is_valid(&record));
        let mut with_web_search = record.clone();
        with_web_search["primary"]["features"]
            .as_array_mut()
            .expect("features array")
            .push(serde_json::json!(crate::ChatRouteFeature::WebSearch));
        assert!(validator.is_valid(&with_web_search));
        assert!(!validator.is_valid(&serde_json::json!("route-1")));
        assert!(!validator.is_valid(&serde_json::json!({
            "schema": "nomifun.chat-route-record.v1",
            "task": "agent_chat",
            "primary": record["primary"],
            "failovers": [],
            "unexpected": true
        })));
    }
}
