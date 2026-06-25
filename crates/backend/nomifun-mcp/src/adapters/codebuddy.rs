use std::collections::HashMap;

use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;
use crate::types::McpServerTransport;

use super::cli_helpers::{MUTATE_TIMEOUT, is_cli_installed, run_cli};

const CLI_NAME: &str = "codebuddy";

/// Scopes to try when removing a server.
const REMOVE_SCOPES: &[&str] = &["user", "local", "project"];

/// MCP Agent adapter for CodeBuddy CLI.
///
/// # Detection
///
/// Detection reads `~/.codebuddy/mcp.json` directly (JSON) rather than
/// parsing CLI text output. The file format:
///
/// ```json
/// { "mcpServers": { "name": { "command": "...", "args": [...], ... } } }
/// ```
///
/// # CLI Commands
///
/// - **install (stdio)**: `codebuddy mcp add -s user <name> <cmd> [-- args...] [-e K=V...]`
/// - **install (http)**: `codebuddy mcp add-json -s user <name> <json>`
/// - **remove**: `codebuddy mcp remove -s <scope> <name>` (tries user → local → project)
pub struct CodeBuddyAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for CodeBuddyAdapter {
    fn source(&self) -> McpSource {
        McpSource::CodeBuddy
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        is_cli_installed(CLI_NAME).await
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let config_path = config_file_path()?;
        if !config_path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&config_path)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("read codebuddy config: {e}")))?;

        parse_codebuddy_config(&content)
    }

    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        match transport {
            McpServerTransport::Stdio { command, args, env } => {
                let mut cli_args = vec![
                    "mcp".to_owned(),
                    "add".to_owned(),
                    "-s".to_owned(),
                    "user".to_owned(),
                    name.to_owned(),
                    command.clone(),
                ];

                // Separate args from env with --
                if !args.is_empty() {
                    cli_args.push("--".to_owned());
                    cli_args.extend(args.iter().cloned());
                }

                // Env vars as -e KEY=VALUE
                for (k, v) in env {
                    cli_args.push("-e".to_owned());
                    cli_args.push(format!("{k}={v}"));
                }

                let arg_refs: Vec<&str> = cli_args.iter().map(|s| s.as_str()).collect();
                run_cli(CLI_NAME, &arg_refs, MUTATE_TIMEOUT).await?;
            }
            McpServerTransport::Sse { url, headers } => {
                let config = build_http_json("sse", url, headers);
                install_via_add_json(name, &config).await?;
            }
            McpServerTransport::Http { url, headers } => {
                let config = build_http_json("streamable-http", url, headers);
                install_via_add_json(name, &config).await?;
            }
        }

        Ok(())
    }

    async fn remove_server(&self, name: &str) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        for scope in REMOVE_SCOPES {
            let (stdout, _stderr) = run_cli(CLI_NAME, &["mcp", "remove", "-s", scope, name], MUTATE_TIMEOUT).await?;
            let lower = stdout.to_lowercase();
            if lower.contains("removed") || lower.contains("not found") {
                return Ok(());
            }
        }

        Ok(())
    }
}

/// Install via `codebuddy mcp add-json -s user <name> <json>`.
async fn install_via_add_json(name: &str, config: &serde_json::Value) -> Result<(), McpError> {
    let config_str = serde_json::to_string(config).map_err(|e| McpError::AgentOperationFailed(e.to_string()))?;
    run_cli(
        CLI_NAME,
        &["mcp", "add-json", "-s", "user", name, &config_str],
        MUTATE_TIMEOUT,
    )
    .await?;
    Ok(())
}

/// Build JSON config for HTTP-like servers.
fn build_http_json(transport_type: &str, url: &str, headers: &HashMap<String, String>) -> serde_json::Value {
    let mut config = serde_json::json!({
        "url": url,
        "transportType": transport_type,
    });
    if !headers.is_empty() {
        config["headers"] = serde_json::json!(headers);
    }
    config
}

/// Get the CodeBuddy config file path: `~/.codebuddy/mcp.json`.
fn config_file_path() -> Result<std::path::PathBuf, McpError> {
    let home =
        dirs::home_dir().ok_or_else(|| McpError::AgentOperationFailed("cannot determine home directory".into()))?;
    Ok(home.join(".codebuddy").join("mcp.json"))
}

/// Parse the CodeBuddy `mcp.json` config file.
///
/// Format:
/// ```json
/// {
///   "mcpServers": {
///     "name": {
///       "command": "...",
///       "args": [...],
///       "env": { ... },
///       "disabled": false,
///       "url": "...",
///       "transportType": "streamable-http",
///       "headers": { ... }
///     }
///   }
/// }
/// ```
fn parse_codebuddy_config(content: &str) -> Result<Vec<DetectedServer>, McpError> {
    let config: serde_json::Value = serde_json::from_str(content).map_err(McpError::from)?;

    let servers_obj = match config.get("mcpServers").and_then(|v| v.as_object()) {
        Some(obj) => obj,
        None => return Ok(Vec::new()),
    };

    let mut servers = Vec::new();

    for (name, entry) in servers_obj {
        if let Some(transport) = parse_codebuddy_entry(entry) {
            let disabled = entry.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            servers.push(DetectedServer {
                name: name.clone(),
                transport,
                importable: !disabled,
                import_skip_reason: if disabled { Some("Disabled".into()) } else { None },
            });
        }
    }

    Ok(servers)
}

/// Parse a single CodeBuddy config entry into a transport.
fn parse_codebuddy_entry(entry: &serde_json::Value) -> Option<McpServerTransport> {
    let has_command = entry.get("command").and_then(|v| v.as_str()).is_some();
    let has_url = entry.get("url").and_then(|v| v.as_str()).is_some();

    if has_command {
        let command = entry["command"].as_str()?.to_owned();
        let args = entry
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let env = parse_string_map(entry.get("env"));
        Some(McpServerTransport::Stdio { command, args, env })
    } else if has_url {
        let url = entry["url"].as_str()?.to_owned();
        let headers = parse_string_map(entry.get("headers"));
        let transport_type = entry
            .get("transportType")
            .and_then(|v| v.as_str())
            .unwrap_or("streamable-http");

        // Normalize transport type
        match transport_type {
            "sse" => Some(McpServerTransport::Sse { url, headers }),
            _ => Some(McpServerTransport::Http { url, headers }),
        }
    } else {
        None
    }
}

/// Parse a JSON object as `HashMap<String, String>`.
fn parse_string_map(value: Option<&serde_json::Value>) -> HashMap<String, String> {
    value
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_codebuddy() {
        assert_eq!(CodeBuddyAdapter.source(), McpSource::CodeBuddy);
    }

    #[test]
    fn parse_config_stdio_server() {
        let config = r#"{
            "mcpServers": {
                "test-server": {
                    "command": "npx",
                    "args": ["-y", "@test/server"],
                    "env": { "NODE_ENV": "production" }
                }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "test-server");
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
    fn parse_config_http_server() {
        let config = r#"{
            "mcpServers": {
                "remote": {
                    "url": "https://example.com/mcp",
                    "transportType": "streamable-http",
                    "headers": { "Authorization": "Bearer tok" }
                }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "remote");
        match &servers[0].transport {
            McpServerTransport::Http { url, headers } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(headers.get("Authorization").unwrap(), "Bearer tok");
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn parse_config_sse_server() {
        let config = r#"{
            "mcpServers": {
                "sse-srv": {
                    "url": "https://example.com/sse",
                    "transportType": "sse"
                }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Sse { url, .. } => {
                assert_eq!(url, "https://example.com/sse");
            }
            _ => panic!("expected Sse"),
        }
    }

    #[test]
    fn parse_config_skips_disabled() {
        let config = r#"{
            "mcpServers": {
                "active": { "command": "npx", "args": [] },
                "disabled": { "command": "npx", "args": [], "disabled": true }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "active");
        assert!(servers[0].importable);
        assert_eq!(servers[1].name, "disabled");
        assert!(!servers[1].importable);
        assert_eq!(servers[1].import_skip_reason.as_deref(), Some("Disabled"));
    }

    #[test]
    fn parse_config_empty_mcp_servers() {
        let config = r#"{ "mcpServers": {} }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_config_no_mcp_servers_key() {
        let config = r#"{ "otherKey": 42 }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_config_multiple_servers() {
        let config = r#"{
            "mcpServers": {
                "stdio-srv": { "command": "node", "args": ["index.js"] },
                "http-srv": { "url": "https://a.com/mcp" },
                "sse-srv": { "url": "https://b.com/sse", "transportType": "sse" }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 3);
    }

    #[test]
    fn parse_config_url_without_transport_type_defaults_to_http() {
        let config = r#"{
            "mcpServers": {
                "no-type": { "url": "https://example.com/api" }
            }
        }"#;
        let servers = parse_codebuddy_config(config).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Http { .. }));
    }

    #[test]
    fn build_http_json_without_headers() {
        let json = build_http_json("streamable-http", "https://example.com", &HashMap::new());
        assert_eq!(json["url"], "https://example.com");
        assert_eq!(json["transportType"], "streamable-http");
        assert!(json.get("headers").is_none());
    }

    #[test]
    fn build_http_json_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer tok".into());
        let json = build_http_json("sse", "https://example.com/sse", &headers);
        assert_eq!(json["transportType"], "sse");
        assert_eq!(json["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(CodeBuddyAdapter);
        assert_eq!(adapter.source(), McpSource::CodeBuddy);
    }
}
