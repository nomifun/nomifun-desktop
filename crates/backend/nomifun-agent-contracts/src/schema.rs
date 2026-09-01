use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const FRESH_V4_DATA_GENERATION: u32 = 4;
pub const FRESH_V4_MIGRATION_HEAD: u32 = 1;
pub const FRESH_V4_PROJECTION_SCHEMA_VERSION: u32 = 1;
pub const FRESH_V4_BASELINE_SQL: &str = include_str!("../schema/0001_fresh_v4.sql");
pub const CHAT_ROUTE_RECORD_JSON_SCHEMA: &str =
    include_str!("../schema/chat-route-record.v1.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaTableContract {
    pub table_name: String,
    pub owner: String,
    pub fact_class: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreshV4SchemaManifestPayload {
    pub schema_version: String,
    pub data_generation: u32,
    pub migration_head: u32,
    pub projection_schema_version: u32,
    pub baseline_logical_path: String,
    pub tables: Vec<SchemaTableContract>,
    pub forbidden_table_names: Vec<String>,
}

pub fn fresh_v4_schema_manifest_payload() -> FreshV4SchemaManifestPayload {
    FreshV4SchemaManifestPayload {
        schema_version: "1.0.0".to_owned(),
        data_generation: FRESH_V4_DATA_GENERATION,
        migration_head: FRESH_V4_MIGRATION_HEAD,
        projection_schema_version: FRESH_V4_PROJECTION_SCHEMA_VERSION,
        baseline_logical_path:
            "crates/backend/nomifun-agent-contracts/schema/0001_fresh_v4.sql".to_owned(),
        tables: TABLES
            .iter()
            .map(|(table_name, owner, fact_class)| SchemaTableContract {
                table_name: (*table_name).to_owned(),
                owner: (*owner).to_owned(),
                fact_class: (*fact_class).to_owned(),
            })
            .collect(),
        forbidden_table_names: FORBIDDEN_TABLE_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    }
}
const TABLES: &[(&str, &str, &str)] = &[
    ("schema_metadata", "platform.schema", "fact"),
    ("schema_migrations", "platform.schema", "fact"),
    ("plugin_packages", "platform.plugin-manager", "fact"),
    ("plugin_mounts", "platform.plugin-manager", "fact"),
    ("plugin_configs", "platform.plugin-manager", "fact"),
    ("plugin_states", "platform.plugin-manager", "fact"),
    (
        "capability_definitions",
        "platform.capability-registry",
        "fact",
    ),
    ("capability_packs", "platform.capability-registry", "fact"),
    (
        "capability_pack_items",
        "platform.capability-registry",
        "fact",
    ),
    ("skill_instructions", "platform.skill-catalog", "fact"),
    ("mcp_servers", "plugin.mcp-connectors", "fact"),
    (
        "mcp_tool_materializations",
        "plugin.mcp-connectors",
        "fact",
    ),
    (
        "agent_preset_templates",
        "platform.agent-preset",
        "fact",
    ),
    ("agent_presets", "platform.agent-preset", "fact"),
    (
        "agent_preset_revisions",
        "platform.agent-preset",
        "fact",
    ),
    (
        "agent_preset_model_routes",
        "platform.agent-preset",
        "fact",
    ),
    (
        "preset_initial_capabilities",
        "platform.agent-preset",
        "fact",
    ),
    (
        "preset_on_demand_capabilities",
        "platform.agent-preset",
        "fact",
    ),
    (
        "preset_skill_bindings",
        "platform.agent-preset",
        "fact",
    ),
    (
        "preset_resource_bindings",
        "platform.agent-preset",
        "fact",
    ),
    ("agent_bindings", "platform.agent-preset", "fact"),
    ("remote_bindings", "plugin.remote-ingress", "fact"),
    ("installation_auth", "plugin.remote-ingress", "fact"),
    ("providers", "platform.chat-model-broker", "fact"),
    ("provider_models", "platform.chat-model-broker", "fact"),
    (
        "provider_connections",
        "platform.chat-model-broker",
        "fact",
    ),
    (
        "provider_model_capabilities",
        "platform.chat-model-broker",
        "fact",
    ),
    ("client_preferences", "platform.host-configuration", "fact"),
    ("system_settings", "platform.host-configuration", "fact"),
    (
        "agent_runtime_snapshots",
        "platform.agent-preset-compiler",
        "fact",
    ),
    (
        "agent_runtime_snapshot_capabilities",
        "platform.agent-preset-compiler",
        "fact",
    ),
    (
        "agent_runtime_profiles",
        "platform.agent-preset-compiler",
        "fact",
    ),
    (
        "agent_preset_audit_events",
        "platform.agent-preset",
        "fact",
    ),
    ("agent_sessions", "platform.agent-session", "fact"),
    ("session_events", "platform.agent-session", "fact"),
    ("session_payloads", "platform.agent-session", "fact"),
    ("session_heads", "platform.agent-session", "projection"),
    (
        "message_projection",
        "platform.agent-session",
        "projection",
    ),
];

const FORBIDDEN_TABLE_NAMES: &[&str] = &[
    "conversations",
    "conversation_sessions",
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
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rusqlite::Connection;

    use super::*;

    #[test]
    fn fresh_v4_baseline_builds_from_an_empty_database() {
        let database = Connection::open_in_memory().expect("in-memory SQLite");
        database
            .execute_batch(FRESH_V4_BASELINE_SQL)
            .expect("fresh-v4 baseline");

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
            .map(|(name, _, _)| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn schema_contract_contains_no_legacy_table() {
        let lowercase = FRESH_V4_BASELINE_SQL.to_ascii_lowercase();
        for table in FORBIDDEN_TABLE_NAMES {
            assert!(
                !lowercase.contains(&format!("create table {table}")),
                "fresh-v4 baseline must not create {table}"
            );
        }
    }

    #[test]
    fn every_table_has_one_owner_and_class() {
        let names = TABLES
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), TABLES.len());
        assert!(
            TABLES
                .iter()
                .all(|(_, owner, class)| !owner.is_empty() && !class.is_empty())
        );
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
