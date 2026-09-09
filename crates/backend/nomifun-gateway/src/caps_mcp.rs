//! MCP server and Skill library capabilities.
//!
//! These tools operate only on services with an explicit owner in the current
//! Gateway composition: the MCP server registry and the local Skill library.
//!
//! ## Assumed CompatibilityCapabilityHost fields (parent must wire):
//!
//! - `mcp_config_service: McpConfigService`
//!    Clone of `states.mcp.config_service` (from `McpRouterState`).
//!    Crate: `nomifun-mcp`, type: `nomifun_mcp::McpConfigService`.
//!
//! - `skill_paths: SkillPaths`
//!    Clone of `states.skill.skill_paths` (from `SkillRouterState`).
//!    Crate: `nomifun-skill-library`, type: `nomifun_skill_library::SkillPaths`.
//!
//! ## Skipped tools:
//!
//! - `nomi_mcp_test_connection` requires an additional connection-test service
//!   and persists test results; the desktop UI remains its owner.
//! - `nomi_skill_set_tags` is a user-facing classification operation with no
//!   agent-owned workflow.

use std::{collections::HashMap, future::Future, sync::Arc};

use crate::deps::{CallerCtx, CompatibilityCapabilityHost};
use crate::registry::{Capability, CapabilityMeta, EffectClass};
use crate::server::ok;
use nomifun_api_types::McpServerId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

// ══════════════════════════════════════════════════════════════════════════════
// MCP Server param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpListServersParams {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpAddServerParams {
    /// Human-readable name for the MCP server (must be unique; existing name = upsert).
    name: String,
    /// Optional description of the server's purpose.
    #[serde(default)]
    description: Option<String>,
    /// Fixed transport payload. Variant-specific fields are rejected on every
    /// other transport variant.
    transport: McpTransportParam,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum McpTransportParam {
    Stdio {
        /// Command to launch (for example, `npx`).
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

impl From<McpTransportParam> for nomifun_api_types::McpTransport {
    fn from(value: McpTransportParam) -> Self {
        match value {
            McpTransportParam::Stdio { command, args, env } => Self::Stdio {
                command,
                args,
                env,
            },
            McpTransportParam::Sse { url, headers } => Self::Sse { url, headers },
            McpTransportParam::Http { url, headers } => Self::Http { url, headers },
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpEditServerParams {
    /// Stable MCP server business id (from nomi_mcp_list_servers).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    mcp_server_id: McpServerId,
    /// New description (pass null to clear, omit to keep).
    #[serde(default)]
    description: Option<Option<String>>,
    /// New fixed transport payload (omit to keep).
    #[serde(default)]
    transport: Option<McpTransportParam>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpDeleteServerParams {
    /// Stable MCP server business id to permanently delete.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    mcp_server_id: McpServerId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpToggleServerParams {
    /// Stable MCP server business id to toggle enabled/disabled.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    mcp_server_id: McpServerId,
}

// ══════════════════════════════════════════════════════════════════════════════
// Skill param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SkillListParams {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SkillImportParams {
    /// Absolute path to the skill directory to import (by copy).
    skill_path: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SkillDeleteParams {
    /// Skill name to permanently delete (user-custom only).
    name: String,
}

// ══════════════════════════════════════════════════════════════════════════════
// MCP Server handlers
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct McpSkillCapabilityDeps {
    config: nomifun_mcp::McpConfigService,
    skill_paths: nomifun_skill_library::SkillPaths,
}

fn adapt<P, F, Fut>(
    handler: F,
) -> impl Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, P) -> Fut + Send + Sync + 'static
where
    P: Send + 'static,
    F: Fn(Arc<McpSkillCapabilityDeps>, CallerCtx, P) -> Fut
        + Send
        + Sync
        + Clone
        + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    move |deps, ctx, params| {
        handler(
            Arc::new(McpSkillCapabilityDeps {
                config: deps.mcp_config_service.clone(),
                skill_paths: deps.skill_paths.clone(),
            }),
            ctx,
            params,
        )
    }
}

async fn mcp_list_servers(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    _p: McpListServersParams,
) -> Value {
    match deps.config.list_servers().await {
        Ok(servers) => ok(servers),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_add_server(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: McpAddServerParams,
) -> Value {
    let req = nomifun_api_types::CreateMcpServerRequest {
        name: p.name,
        description: p.description,
        transport: p.transport.into(),
        original_json: None,
        builtin: false,
    };
    match deps.config.add_server(req).await {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_edit_server(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: McpEditServerParams,
) -> Value {
    let req = nomifun_api_types::UpdateMcpServerRequest {
        name: None,
        description: p.description,
        transport: p.transport.map(Into::into),
        original_json: None,
        builtin: None,
    };
    match deps
        .config
        .edit_server(&p.mcp_server_id, req)
        .await
    {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_delete_server(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: McpDeleteServerParams,
) -> Value {
    match deps
        .config
        .delete_server(&p.mcp_server_id)
        .await
    {
        Ok(was_enabled) => ok(json!({
            "deleted": true,
            "was_enabled": was_enabled,
        })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_toggle_server(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: McpToggleServerParams,
) -> Value {
    match deps
        .config
        .toggle_server(&p.mcp_server_id)
        .await
    {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Skill handlers
// ══════════════════════════════════════════════════════════════════════════════

async fn skill_list(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    _p: SkillListParams,
) -> Value {
    match nomifun_skill_library::skill_service::list_available_skills(&deps.skill_paths).await {
        Ok(items) => {
            let resp: Vec<Value> = items
                .into_iter()
                .map(|s| {
                    json!({
                        "name": s.name,
                        "description": s.description,
                        "is_custom": s.is_custom,
                    })
                })
                .collect();
            ok(resp)
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn skill_import(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: SkillImportParams,
) -> Value {
    let path = std::path::Path::new(&p.skill_path);
    match nomifun_skill_library::skill_service::import_skill(&deps.skill_paths, path).await {
        Ok(name) => ok(json!({ "imported": true, "skill_name": name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn skill_delete(
    deps: Arc<McpSkillCapabilityDeps>,
    _ctx: CallerCtx,
    p: SkillDeleteParams,
) -> Value {
    match nomifun_skill_library::skill_service::delete_skill(&deps.skill_paths, &p.name).await {
        Ok(()) => ok(json!({ "deleted": true, "name": p.name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Registration
// ══════════════════════════════════════════════════════════════════════════════

/// Register the MCP server and Skill library capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // ── MCP Servers ──────────────────────────────────────────────────────

    out.push(Capability::new::<McpListServersParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_list_servers",
            "mcp",
            "List all configured MCP servers (name, transport, enabled state, connection status).",
            EffectClass::Read,
        ),
        adapt(mcp_list_servers),
    ));

    out.push(Capability::new::<McpAddServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_add_server",
            "mcp",
            "Add a new MCP server (stdio/sse/http). Upserts by name if one already exists. Headers may contain auth tokens.",
            EffectClass::Sensitive,
        ),
        adapt(mcp_add_server),
    ));

    out.push(Capability::new::<McpEditServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_edit_server",
            "mcp",
            "Edit an existing MCP server's transport or description (by mcp_server_id).",
            EffectClass::Write,
        ),
        adapt(mcp_edit_server),
    ));

    out.push(Capability::new::<McpDeleteServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_delete_server",
            "mcp",
            "Permanently delete an MCP server configuration (by mcp_server_id).",
            EffectClass::Destructive,
        ),
        adapt(mcp_delete_server),
    ));

    out.push(Capability::new::<McpToggleServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_toggle_server",
            "mcp",
            "Toggle the enabled/disabled state of an MCP server (by mcp_server_id).",
            EffectClass::Write,
        ),
        adapt(mcp_toggle_server),
    ));

    // ── Skills ───────────────────────────────────────────────────────────

    out.push(Capability::new::<SkillListParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_list",
            "skill",
            "List all available skills (built-in and user-custom).",
            EffectClass::Read,
        ),
        adapt(skill_list),
    ));

    out.push(Capability::new::<SkillImportParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_import",
            "skill",
            "Import a skill from a local directory (by absolute path). Copies the skill into the user skills folder.",
            EffectClass::Write,
        ),
        adapt(skill_import),
    ));

    out.push(Capability::new::<SkillDeleteParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_delete",
            "skill",
            "Permanently delete a user-custom skill by name.",
            EffectClass::Destructive,
        ),
        adapt(skill_delete),
    ));
}

// ══════════════════════════════════════════════════════════════════════════════
// SKIPPED tools
// ══════════════════════════════════════════════════════════════════════════════
//
// 1. `nomi_mcp_test_connection` (Read/Write)
//    Service: `McpConnectionTestService::test_connection(&self, name: &str, transport: &McpServerTransport)`
//    Issue: The `McpServerTransport` is a domain enum built from the tagged
//    `McpTransport` API type. Exposing a tagged-union transport in the flat
//    JSON schema would be confusing for an LLM (requires `type` + variant-
//    specific fields). The route handler also persists test results back.
//    Agent use case unclear — the user can trigger a test from the UI.
//
// 2. `nomi_skill_set_tags` (Write)
//    Service: `ISkillTagRepository::upsert(...)` + `builtin_skill_tags` map.
//    Issue: Tags are audience/scenario classifications for UI filtering, not
//    something an agent typically needs to set. Low priority.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Registry, Surface};

    const MCP_SERVER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000123";

    #[test]
    fn registration_contains_only_owned_mcp_and_skill_tools() {
        let mut capabilities = Vec::new();
        register(&mut capabilities);

        let names: Vec<_> = capabilities
            .iter()
            .map(|capability| capability.meta.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "nomi_mcp_list_servers",
                "nomi_mcp_add_server",
                "nomi_mcp_edit_server",
                "nomi_mcp_delete_server",
                "nomi_mcp_toggle_server",
                "nomi_skill_list",
                "nomi_skill_import",
                "nomi_skill_delete",
            ]
        );
        assert!(
            capabilities
                .iter()
                .all(|capability| capability.meta.domain == "mcp"
                    || capability.meta.domain == "skill")
        );
    }

    #[test]
    fn mcp_server_mutation_params_accept_only_canonical_uuid_v7_business_ids() {
        let edit: McpEditServerParams = serde_json::from_value(json!({
            "mcp_server_id": MCP_SERVER_ID,
            "description": "updated"
        }))
        .unwrap();
        assert_eq!(edit.mcp_server_id.as_str(), MCP_SERVER_ID);

        let delete: McpDeleteServerParams =
            serde_json::from_value(json!({"mcp_server_id": MCP_SERVER_ID})).unwrap();
        assert_eq!(delete.mcp_server_id.as_str(), MCP_SERVER_ID);

        let toggle: McpToggleServerParams =
            serde_json::from_value(json!({"mcp_server_id": MCP_SERVER_ID})).unwrap();
        assert_eq!(toggle.mcp_server_id.as_str(), MCP_SERVER_ID);
    }

    #[test]
    fn mcp_server_mutation_params_reject_legacy_and_noncanonical_ids() {
        let invalid_values = [
            json!(42),
            json!("42"),
            json!("mcp_0190f5fe-7c00-7a00-8000-000000000123"),
            json!("550e8400-e29b-41d4-a716-446655440000"),
            json!(MCP_SERVER_ID.to_ascii_uppercase()),
        ];

        for invalid in invalid_values {
            assert!(
                serde_json::from_value::<McpEditServerParams>(json!({
                    "mcp_server_id": invalid.clone(),
                    "description": "updated"
                }))
                .is_err(),
                "edit accepted invalid MCP server id: {invalid}"
            );
            assert!(
                serde_json::from_value::<McpDeleteServerParams>(json!({
                    "mcp_server_id": invalid.clone()
                }))
                .is_err(),
                "delete accepted invalid MCP server id: {invalid}"
            );
            assert!(
                serde_json::from_value::<McpToggleServerParams>(json!({
                    "mcp_server_id": invalid.clone()
                }))
                .is_err(),
                "toggle accepted invalid MCP server id: {invalid}"
            );
        }

        assert!(serde_json::from_value::<McpEditServerParams>(json!({
            "id": MCP_SERVER_ID,
            "description": "legacy field must be rejected"
        }))
        .is_err());
        assert!(
            serde_json::from_value::<McpDeleteServerParams>(json!({"id": MCP_SERVER_ID}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<McpToggleServerParams>(json!({"id": MCP_SERVER_ID}))
                .is_err()
        );
    }

    #[test]
    fn mcp_transport_params_have_a_fixed_variant_shape() {
        let stdio: McpAddServerParams = serde_json::from_value(json!({
            "name": "filesystem",
            "transport": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem"]
            }
        }))
        .unwrap();
        assert!(matches!(stdio.transport, McpTransportParam::Stdio { .. }));

        assert!(
            serde_json::from_value::<McpAddServerParams>(json!({
                "name": "invalid mixed transport",
                "transport": {
                    "type": "stdio",
                    "command": "npx",
                    "url": "https://example.invalid/mcp"
                }
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<McpAddServerParams>(json!({
                "name": "legacy flat transport",
                "transport_type": "stdio",
                "command": "npx"
            }))
            .is_err()
        );
    }

    #[test]
    fn mcp_server_mutation_tool_schemas_require_named_uuid_v7_business_id() {
        let specs = Registry::global().tool_specs(Surface::Desktop);
        for name in [
            "nomi_mcp_edit_server",
            "nomi_mcp_delete_server",
            "nomi_mcp_toggle_server",
        ] {
            let spec = specs
                .iter()
                .find(|spec| spec.name == name)
                .expect("tool registered");
            let properties = spec
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .expect("tool properties");
            assert!(properties.contains_key("mcp_server_id"), "{name}");
            assert!(!properties.contains_key("id"), "{name}");
            let id_schema = properties
                .get("mcp_server_id")
                .and_then(Value::as_object)
                .expect("mcp_server_id schema");
            assert_eq!(id_schema.get("type"), Some(&json!("string")), "{name}");
            assert_eq!(
                id_schema.get("pattern"),
                Some(&json!(
                    "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
                )),
                "{name}"
            );
        }
    }
}
