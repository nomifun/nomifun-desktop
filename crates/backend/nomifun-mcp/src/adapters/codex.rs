use std::collections::HashMap;

use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;
use crate::types::McpServerTransport;

use super::cli_helpers::{DETECT_TIMEOUT, MUTATE_TIMEOUT, is_cli_installed, run_cli_strict};

const CLI_NAME: &str = "codex";

/// MCP Agent adapter for Codex CLI.
///
/// # CLI Commands
///
/// - **detect**: `codex mcp list --json` (JSON output)
/// - **install (stdio)**: `codex mcp add <name> [--env K=V]... -- <cmd> [args...]`
/// - **install (http)**: `codex mcp add <name> --url <url>`
/// - **remove**: `codex mcp remove <name>` (no scope parameter)
///
/// Codex outputs structured JSON for list, unlike the text-based agents.
pub struct CodexAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for CodexAdapter {
    fn source(&self) -> McpSource {
        McpSource::Codex
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        is_cli_installed(CLI_NAME).await
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let stdout = run_cli_strict(CLI_NAME, &["mcp", "list", "--json"], DETECT_TIMEOUT).await?;

        parse_codex_list_json(&stdout)
    }

    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        match transport {
            McpServerTransport::Stdio { command, args, env } => {
                let mut cli_args = vec!["mcp".to_owned(), "add".to_owned(), name.to_owned()];

                // Env vars come before --
                for (k, v) in env {
                    cli_args.push("--env".to_owned());
                    cli_args.push(format!("{k}={v}"));
                }

                // Command and args come after --
                cli_args.push("--".to_owned());
                cli_args.push(command.clone());
                cli_args.extend(args.iter().cloned());

                let arg_refs: Vec<&str> = cli_args.iter().map(|s| s.as_str()).collect();
                run_cli_strict(CLI_NAME, &arg_refs, MUTATE_TIMEOUT).await?;
            }
            McpServerTransport::Http { url, .. } | McpServerTransport::Sse { url, .. } => {
                // Codex only supports --url for HTTP, no headers via CLI
                run_cli_strict(CLI_NAME, &["mcp", "add", name, "--url", url], MUTATE_TIMEOUT).await?;
            }
        }

        Ok(())
    }

    async fn remove_server(&self, name: &str) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        // Codex has no scope parameter; remove is simple.
        let (stdout, _stderr) = super::cli_helpers::run_cli(CLI_NAME, &["mcp", "remove", name], MUTATE_TIMEOUT).await?;

        // Idempotent: treat "not found" as success.
        let lower = stdout.to_lowercase();
        if lower.contains("not found") || lower.contains("removed") || lower.is_empty() {
            return Ok(());
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------

/// Parse the JSON output of `codex mcp list --json`.
///
/// Expected format: array of entries with transport details.
///
/// ```json
/// [
///   {
///     "name": "...",
///     "enabled": true,
///     "transport": {
///       "type": "stdio",
///       "command": "...",
///       "args": [...],
///       "env": { ... },
///       "env_vars": [{ "name": "...", "value": "..." }]
///     }
///   }
/// ]
/// ```
fn parse_codex_list_json(json_str: &str) -> Result<Vec<DetectedServer>, McpError> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(Vec::new());
    }

    let entries: Vec<serde_json::Value> = serde_json::from_str(trimmed).map_err(McpError::from)?;

    let mut servers = Vec::new();

    for entry in &entries {
        if let Some(server) = parse_codex_entry(entry) {
            servers.push(server);
        }
    }

    Ok(servers)
}

/// Parse a single Codex list entry.
fn parse_codex_entry(entry: &serde_json::Value) -> Option<DetectedServer> {
    let name = entry.get("name")?.as_str()?.to_owned();
    let enabled = entry.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let transport_obj = entry.get("transport")?;
    let transport_type = transport_obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");

    let transport = match transport_type {
        "stdio" => {
            let command = transport_obj
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let args = transport_obj
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            // Codex supports both `env` (object) and `env_vars` (array of {name, value})
            let env = parse_codex_env(transport_obj);

            McpServerTransport::Stdio { command, args, env }
        }
        "http" | "streamable_http" => {
            let url = transport_obj
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            McpServerTransport::Http {
                url,
                headers: HashMap::new(),
            }
        }
        "sse" => {
            let url = transport_obj
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            McpServerTransport::Sse {
                url,
                headers: HashMap::new(),
            }
        }
        _ => return None,
    };

    Some(DetectedServer {
        name,
        transport,
        importable: enabled,
        import_skip_reason: if enabled { None } else { Some("Disabled".into()) },
    })
}

/// Parse environment variables from Codex transport.
///
/// Handles both formats:
/// - `"env": { "KEY": "VALUE" }` (object)
/// - `"env_vars": [{ "name": "KEY", "value": "VALUE" }]` (array)
fn parse_codex_env(transport: &serde_json::Value) -> HashMap<String, String> {
    // Try object format first
    if let Some(obj) = transport.get("env").and_then(|v| v.as_object()) {
        return obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect();
    }

    // Try array format
    if let Some(arr) = transport.get("env_vars").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|entry| {
                let name = entry.get("name")?.as_str()?;
                let value = entry.get("value")?.as_str()?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect();
    }

    HashMap::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_codex() {
        assert_eq!(CodexAdapter.source(), McpSource::Codex);
    }

    #[test]
    fn parse_empty_json() {
        let servers = parse_codex_list_json("[]").unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_empty_string() {
        let servers = parse_codex_list_json("").unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_stdio_server_with_env_object() {
        let json = r#"[
            {
                "name": "test-mcp",
                "enabled": true,
                "transport": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "@test/server"],
                    "env": { "NODE_ENV": "production" }
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "test-mcp");
        match &servers[0].transport {
            McpServerTransport::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "@test/server"]);
                assert_eq!(env.get("NODE_ENV").unwrap(), "production");
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_stdio_server_with_env_vars_array() {
        let json = r#"[
            {
                "name": "test-mcp",
                "enabled": true,
                "transport": {
                    "type": "stdio",
                    "command": "node",
                    "args": ["index.js"],
                    "env_vars": [
                        { "name": "KEY1", "value": "VAL1" },
                        { "name": "KEY2", "value": "VAL2" }
                    ]
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Stdio { env, .. } => {
                assert_eq!(env.get("KEY1").unwrap(), "VAL1");
                assert_eq!(env.get("KEY2").unwrap(), "VAL2");
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_http_server() {
        let json = r#"[
            {
                "name": "remote",
                "enabled": true,
                "transport": {
                    "type": "http",
                    "url": "https://example.com/mcp"
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Http { url, .. } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn parse_streamable_http_becomes_http() {
        let json = r#"[
            {
                "name": "streamable",
                "enabled": true,
                "transport": {
                    "type": "streamable_http",
                    "url": "https://example.com/api"
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Http { .. }));
    }

    #[test]
    fn parse_sse_server() {
        let json = r#"[
            {
                "name": "sse-srv",
                "enabled": true,
                "transport": {
                    "type": "sse",
                    "url": "https://example.com/sse"
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Sse { url, .. } => {
                assert_eq!(url, "https://example.com/sse");
            }
            _ => panic!("expected Sse"),
        }
    }

    #[test]
    fn parse_multiple_servers() {
        let json = r#"[
            {
                "name": "stdio-srv",
                "enabled": true,
                "transport": { "type": "stdio", "command": "node", "args": [] }
            },
            {
                "name": "http-srv",
                "enabled": true,
                "transport": { "type": "http", "url": "https://a.com/mcp" }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "stdio-srv");
        assert_eq!(servers[1].name, "http-srv");
    }

    #[test]
    fn parse_disabled_server_skipped_from_import_only() {
        let json = r#"[
            {
                "name": "disabled-srv",
                "enabled": false,
                "transport": { "type": "stdio", "command": "node", "args": [] }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(!servers[0].importable);
        assert_eq!(servers[0].import_skip_reason.as_deref(), Some("Disabled"));
    }

    #[test]
    fn parse_unknown_transport_skipped() {
        let json = r#"[
            {
                "name": "unknown",
                "enabled": true,
                "transport": { "type": "websocket", "url": "ws://localhost" }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_missing_name_skipped() {
        let json = r#"[
            {
                "enabled": true,
                "transport": { "type": "stdio", "command": "node" }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_default_type_is_stdio() {
        let json = r#"[
            {
                "name": "no-type",
                "enabled": true,
                "transport": { "command": "node", "args": ["srv.js"] }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Stdio { .. }));
    }

    #[test]
    fn parse_env_object_takes_precedence() {
        let json = r#"[
            {
                "name": "both-env",
                "enabled": true,
                "transport": {
                    "type": "stdio",
                    "command": "node",
                    "env": { "FROM_OBJ": "yes" },
                    "env_vars": [{ "name": "FROM_ARR", "value": "yes" }]
                }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        match &servers[0].transport {
            McpServerTransport::Stdio { env, .. } => {
                // Object format takes precedence
                assert_eq!(env.get("FROM_OBJ").unwrap(), "yes");
                assert!(env.get("FROM_ARR").is_none());
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_disabled_server_skipped() {
        let json = r#"[
            {
                "name": "disabled-mcp",
                "enabled": false,
                "transport": { "type": "stdio", "command": "node", "args": ["srv.js"] }
            }
        ]"#;
        let servers = parse_codex_list_json(json).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "disabled-mcp");
        assert!(!servers[0].importable);
        assert_eq!(servers[0].import_skip_reason.as_deref(), Some("Disabled"));
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(CodexAdapter);
        assert_eq!(adapter.source(), McpSource::Codex);
    }
}
