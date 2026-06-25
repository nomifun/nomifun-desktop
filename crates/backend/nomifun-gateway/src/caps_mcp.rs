//! MCP-servers, Extensions, Skills, and Hub management capabilities.
//!
//! Lets the LLM agent manage the desktop's MCP server registry, enable/disable
//! extensions, import/delete skills, and install extensions from the Hub.
//!
//! ## Assumed GatewayDeps fields (parent must wire):
//!
//! - `mcp_config_service: McpConfigService`
//!    Clone of `states.mcp.config_service` (from `McpRouterState`).
//!    Crate: `nomifun-mcp`, type: `nomifun_mcp::McpConfigService`.
//!
//! - `extension_registry: ExtensionRegistry`
//!    Clone of `states.extension.registry` (from `ExtensionRouterState`).
//!    Crate: `nomifun-extension`, type: `nomifun_extension::ExtensionRegistry`.
//!
//! - `hub_installer: HubInstaller`
//!    Clone of `states.hub.installer` (from `HubRouterState`).
//!    Crate: `nomifun-extension`, type: `nomifun_extension::hub::installer::HubInstaller`.
//!
//! - `hub_index_manager: HubIndexManager`
//!    Clone of `states.hub.index_manager` (from `HubRouterState`).
//!    Crate: `nomifun-extension`, type: `nomifun_extension::hub::index_manager::HubIndexManager`.
//!
//! - `skill_paths: SkillPaths`
//!    Clone of `states.skill.skill_paths` (from `SkillRouterState`).
//!    Crate: `nomifun-extension`, type: `nomifun_extension::skill_service::SkillPaths`.
//!
//! ## SKIPPED tools (listed at the bottom of this file):
//!
//! - `nomi_mcp_test_connection` — requires building a `McpServerTransport`
//!   from the API `McpTransport` enum (tagged union with three variants), which
//!   is awkward to expose in a flat JSON schema for an LLM. The route handler
//!   also persists test results back to the config service by server id. Skipped
//!   until a clear agent use case emerges.
//!
//! - `nomi_skill_set_tags` — needs `skill_tag_repo` + `builtin_skill_tags`;
//!   low agent utility (user-facing tagging).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

// ══════════════════════════════════════════════════════════════════════════════
// MCP Server param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct McpListServersParams {}

#[derive(Deserialize, JsonSchema)]
struct McpAddServerParams {
    /// Human-readable name for the MCP server (must be unique; existing name = upsert).
    name: String,
    /// Optional description of the server's purpose.
    #[serde(default)]
    description: Option<String>,
    /// Transport type: "stdio", "sse", or "http".
    transport_type: String,
    /// For stdio: the command to launch (e.g. "npx").
    #[serde(default)]
    command: Option<String>,
    /// For stdio: arguments to the command.
    #[serde(default)]
    args: Option<Vec<String>>,
    /// For stdio: environment variables as key-value pairs.
    #[serde(default)]
    env: Option<std::collections::HashMap<String, String>>,
    /// For sse/http: the endpoint URL.
    #[serde(default)]
    url: Option<String>,
    /// For sse/http: extra headers as key-value pairs.
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize, JsonSchema)]
struct McpEditServerParams {
    /// The MCP server id (from nomi_mcp_list_servers).
    id: String,
    /// New description (pass null to clear, omit to keep).
    #[serde(default)]
    description: Option<Option<String>>,
    /// New transport type (omit to keep). Must provide matching fields.
    #[serde(default)]
    transport_type: Option<String>,
    /// For stdio: the command (omit to keep).
    #[serde(default)]
    command: Option<String>,
    /// For stdio: arguments (omit to keep).
    #[serde(default)]
    args: Option<Vec<String>>,
    /// For stdio: environment variables (omit to keep).
    #[serde(default)]
    env: Option<std::collections::HashMap<String, String>>,
    /// For sse/http: the endpoint URL (omit to keep).
    #[serde(default)]
    url: Option<String>,
    /// For sse/http: extra headers (omit to keep).
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize, JsonSchema)]
struct McpDeleteServerParams {
    /// The MCP server id to permanently delete.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct McpToggleServerParams {
    /// The MCP server id to toggle enabled/disabled.
    id: String,
}

// ══════════════════════════════════════════════════════════════════════════════
// Extension param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct ExtensionListParams {}

#[derive(Deserialize, JsonSchema)]
struct ExtensionEnableParams {
    /// Extension name (from nomi_extension_list).
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct ExtensionDisableParams {
    /// Extension name (from nomi_extension_list).
    name: String,
    /// Optional reason for disabling.
    #[serde(default)]
    reason: Option<String>,
}

// ══════════════════════════════════════════════════════════════════════════════
// Skill param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct SkillListParams {}

#[derive(Deserialize, JsonSchema)]
struct SkillImportParams {
    /// Absolute path to the skill directory to import (by copy).
    skill_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct SkillDeleteParams {
    /// Skill name to permanently delete (user-custom only).
    name: String,
}

// ══════════════════════════════════════════════════════════════════════════════
// Hub param structs
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct HubListExtensionsParams {}

#[derive(Deserialize, JsonSchema)]
struct HubInstallExtensionParams {
    /// Extension name from the Hub index (from nomi_hub_list_extensions).
    name: String,
}

// ══════════════════════════════════════════════════════════════════════════════
// MCP Server handlers
// ══════════════════════════════════════════════════════════════════════════════

async fn mcp_list_servers(deps: Arc<GatewayDeps>, _ctx: CallerCtx, _p: McpListServersParams) -> Value {
    match deps.mcp_config_service.list_servers().await {
        Ok(servers) => ok(servers),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_add_server(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: McpAddServerParams) -> Value {
    use nomifun_api_types::McpTransport;

    let transport = match p.transport_type.as_str() {
        "stdio" => {
            let Some(command) = p.command else {
                return json!({ "error": "field 'command' is required for stdio transport" });
            };
            McpTransport::Stdio {
                command,
                args: p.args.unwrap_or_default(),
                env: p.env.unwrap_or_default(),
            }
        }
        "sse" => {
            let Some(url) = p.url else {
                return json!({ "error": "field 'url' is required for sse transport" });
            };
            McpTransport::Sse {
                url,
                headers: p.headers.unwrap_or_default(),
            }
        }
        "http" => {
            let Some(url) = p.url else {
                return json!({ "error": "field 'url' is required for http transport" });
            };
            McpTransport::Http {
                url,
                headers: p.headers.unwrap_or_default(),
            }
        }
        other => {
            return json!({ "error": format!("unsupported transport_type: {other:?}; use \"stdio\", \"sse\", or \"http\"") });
        }
    };

    let req = nomifun_api_types::CreateMcpServerRequest {
        name: p.name,
        description: p.description,
        transport,
        original_json: None,
        builtin: false,
    };
    match deps.mcp_config_service.add_server(req).await {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_edit_server(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: McpEditServerParams) -> Value {
    use nomifun_api_types::McpTransport;

    let transport = match p.transport_type.as_deref() {
        Some("stdio") => {
            let Some(command) = p.command else {
                return json!({ "error": "field 'command' is required for stdio transport" });
            };
            Some(McpTransport::Stdio {
                command,
                args: p.args.unwrap_or_default(),
                env: p.env.unwrap_or_default(),
            })
        }
        Some("sse") => {
            let Some(url) = p.url else {
                return json!({ "error": "field 'url' is required for sse transport" });
            };
            Some(McpTransport::Sse {
                url,
                headers: p.headers.unwrap_or_default(),
            })
        }
        Some("http") => {
            let Some(url) = p.url else {
                return json!({ "error": "field 'url' is required for http transport" });
            };
            Some(McpTransport::Http {
                url,
                headers: p.headers.unwrap_or_default(),
            })
        }
        Some(other) => {
            return json!({ "error": format!("unsupported transport_type: {other:?}; use \"stdio\", \"sse\", or \"http\"") });
        }
        None => None,
    };

    let req = nomifun_api_types::UpdateMcpServerRequest {
        name: None,
        description: p.description,
        transport,
        original_json: None,
        builtin: None,
    };
    match deps.mcp_config_service.edit_server(&p.id, req).await {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_delete_server(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: McpDeleteServerParams) -> Value {
    match deps.mcp_config_service.delete_server(&p.id).await {
        Ok(was_enabled) => ok(json!({
            "deleted": true,
            "was_enabled": was_enabled,
        })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn mcp_toggle_server(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: McpToggleServerParams) -> Value {
    match deps.mcp_config_service.toggle_server(&p.id).await {
        Ok(server) => ok(server),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Extension handlers
// ══════════════════════════════════════════════════════════════════════════════

async fn extension_list(deps: Arc<GatewayDeps>, _ctx: CallerCtx, _p: ExtensionListParams) -> Value {
    let summaries = deps.extension_registry.get_loaded_extensions().await;
    let items: Vec<Value> = summaries
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "version": s.version,
                "display_name": s.display_name,
                "description": s.description,
                "enabled": s.enabled,
            })
        })
        .collect();
    ok(items)
}

async fn extension_enable(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: ExtensionEnableParams) -> Value {
    match deps.extension_registry.enable_extension(&p.name).await {
        Ok(()) => ok(json!({ "enabled": true, "name": p.name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn extension_disable(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: ExtensionDisableParams) -> Value {
    match deps.extension_registry.disable_extension(&p.name, p.reason.as_deref()).await {
        Ok(()) => ok(json!({ "disabled": true, "name": p.name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Skill handlers
// ══════════════════════════════════════════════════════════════════════════════

async fn skill_list(deps: Arc<GatewayDeps>, _ctx: CallerCtx, _p: SkillListParams) -> Value {
    match nomifun_extension::skill_service::list_available_skills(&deps.skill_paths).await {
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

async fn skill_import(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: SkillImportParams) -> Value {
    let path = std::path::Path::new(&p.skill_path);
    match nomifun_extension::skill_service::import_skill(&deps.skill_paths, path).await {
        Ok(name) => ok(json!({ "imported": true, "skill_name": name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn skill_delete(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: SkillDeleteParams) -> Value {
    match nomifun_extension::skill_service::delete_skill(&deps.skill_paths, &p.name).await {
        Ok(()) => ok(json!({ "deleted": true, "name": p.name })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Hub handlers
// ══════════════════════════════════════════════════════════════════════════════

async fn hub_list_extensions(deps: Arc<GatewayDeps>, _ctx: CallerCtx, _p: HubListExtensionsParams) -> Value {
    let entries = deps.hub_index_manager.load_index().await;
    let items: Vec<Value> = entries
        .into_iter()
        .map(|e| {
            let status_str = serde_json::to_value(&e.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "notInstalled".to_string());
            json!({
                "name": e.name,
                "version": e.version,
                "display_name": e.display_name,
                "description": e.description,
                "author": e.author,
                "status": status_str,
            })
        })
        .collect();
    ok(items)
}

async fn hub_install_extension(deps: Arc<GatewayDeps>, _ctx: CallerCtx, p: HubInstallExtensionParams) -> Value {
    let result = deps.hub_installer.install(&p.name).await;
    ok(json!({
        "success": result.success,
        "msg": result.msg,
    }))
}

// ══════════════════════════════════════════════════════════════════════════════
// Registration
// ══════════════════════════════════════════════════════════════════════════════

/// Register the MCP/Extension/Skill/Hub domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // ── MCP Servers ──────────────────────────────────────────────────────

    out.push(Capability::new::<McpListServersParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_list_servers",
            "mcp",
            "List all configured MCP servers (name, transport, enabled state, connection status).",
            DangerTier::Read,
        ),
        |deps, ctx, p| mcp_list_servers(deps, ctx, p),
    ));

    out.push(Capability::new::<McpAddServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_add_server",
            "mcp",
            "Add a new MCP server (stdio/sse/http). Upserts by name if one already exists. Headers may contain auth tokens.",
            DangerTier::Sensitive,
        ),
        |deps, ctx, p| mcp_add_server(deps, ctx, p),
    ));

    out.push(Capability::new::<McpEditServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_edit_server",
            "mcp",
            "Edit an existing MCP server's transport or description (by id).",
            DangerTier::Write,
        ),
        |deps, ctx, p| mcp_edit_server(deps, ctx, p),
    ));

    out.push(Capability::new::<McpDeleteServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_delete_server",
            "mcp",
            "Permanently delete an MCP server configuration (by id).",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| mcp_delete_server(deps, ctx, p),
    ));

    out.push(Capability::new::<McpToggleServerParams, _, _>(
        CapabilityMeta::new(
            "nomi_mcp_toggle_server",
            "mcp",
            "Toggle the enabled/disabled state of an MCP server (by id).",
            DangerTier::Write,
        ),
        |deps, ctx, p| mcp_toggle_server(deps, ctx, p),
    ));

    // ── Extensions ───────────────────────────────────────────────────────

    out.push(Capability::new::<ExtensionListParams, _, _>(
        CapabilityMeta::new(
            "nomi_extension_list",
            "extension",
            "List all loaded extensions (name, version, enabled state).",
            DangerTier::Read,
        ),
        |deps, ctx, p| extension_list(deps, ctx, p),
    ));

    out.push(Capability::new::<ExtensionEnableParams, _, _>(
        CapabilityMeta::new(
            "nomi_extension_enable",
            "extension",
            "Enable a disabled extension by name.",
            DangerTier::Write,
        ),
        |deps, ctx, p| extension_enable(deps, ctx, p),
    ));

    out.push(Capability::new::<ExtensionDisableParams, _, _>(
        CapabilityMeta::new(
            "nomi_extension_disable",
            "extension",
            "Disable an enabled extension by name (with optional reason).",
            DangerTier::Write,
        ),
        |deps, ctx, p| extension_disable(deps, ctx, p),
    ));

    // ── Skills ───────────────────────────────────────────────────────────

    out.push(Capability::new::<SkillListParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_list",
            "skill",
            "List all available skills (built-in and user-custom).",
            DangerTier::Read,
        ),
        |deps, ctx, p| skill_list(deps, ctx, p),
    ));

    out.push(Capability::new::<SkillImportParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_import",
            "skill",
            "Import a skill from a local directory (by absolute path). Copies the skill into the user skills folder.",
            DangerTier::Write,
        ),
        |deps, ctx, p| skill_import(deps, ctx, p),
    ));

    out.push(Capability::new::<SkillDeleteParams, _, _>(
        CapabilityMeta::new(
            "nomi_skill_delete",
            "skill",
            "Permanently delete a user-custom skill by name.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| skill_delete(deps, ctx, p),
    ));

    // ── Hub ──────────────────────────────────────────────────────────────

    out.push(Capability::new::<HubListExtensionsParams, _, _>(
        CapabilityMeta::new(
            "nomi_hub_list_extensions",
            "hub",
            "List extensions available in the Hub marketplace (name, version, install status).",
            DangerTier::Read,
        ),
        |deps, ctx, p| hub_list_extensions(deps, ctx, p),
    ));

    out.push(Capability::new::<HubInstallExtensionParams, _, _>(
        CapabilityMeta::new(
            "nomi_hub_install_extension",
            "hub",
            "Install an extension from the Hub by name. Downloads and registers it locally.",
            DangerTier::Write,
        ),
        |deps, ctx, p| hub_install_extension(deps, ctx, p),
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
