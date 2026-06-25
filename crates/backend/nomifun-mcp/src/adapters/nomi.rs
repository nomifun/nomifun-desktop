use std::collections::HashMap;

use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;
use crate::types::McpServerTransport;

use super::cli_helpers::{DETECT_TIMEOUT, is_cli_installed, run_cli_strict};

const CLI_NAME: &str = "nomi";

/// MCP Agent adapter for Nomi.
///
/// Nomi stores MCP configuration in a TOML config file. The config path
/// is obtained via `nomi --config-path`.
///
/// # Config Format (TOML)
///
/// ```toml
/// [mcp.servers.server-name]
/// transport = "stdio"
/// command = "npx"
/// args = ["-y", "@test/server"]
///
/// [mcp.servers.server-name.env]
/// KEY = "VALUE"
///
/// [mcp.servers.remote-server]
/// transport = "http"
/// url = "https://example.com/mcp"
///
/// [mcp.servers.remote-server.headers]
/// Authorization = "Bearer xxx"
/// ```
pub struct NomiAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for NomiAdapter {
    fn source(&self) -> McpSource {
        McpSource::Nomi
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        is_cli_installed(CLI_NAME).await
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let config_path = get_config_path().await?;
        let path = std::path::Path::new(&config_path);

        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {config_path}: {e}")))?;

        parse_toml_servers(&content)
    }

    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let config_path = get_config_path().await?;
        let path = std::path::Path::new(&config_path);

        let mut doc = if path.exists() {
            let content = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {config_path}: {e}")))?;
            content
                .parse::<toml::Value>()
                .map_err(|e| McpError::AgentOperationFailed(format!("failed to parse TOML: {e}")))?
        } else {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| McpError::AgentOperationFailed(format!("failed to create dir: {e}")))?;
            }
            toml::Value::Table(toml::map::Map::new())
        };

        // Ensure mcp.servers exists
        let root = doc
            .as_table_mut()
            .ok_or_else(|| McpError::AgentOperationFailed("TOML root is not a table".into()))?;

        let mcp = root
            .entry("mcp")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));

        let mcp_table = mcp
            .as_table_mut()
            .ok_or_else(|| McpError::AgentOperationFailed("mcp is not a table".into()))?;

        let servers = mcp_table
            .entry("servers")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));

        let servers_table = servers
            .as_table_mut()
            .ok_or_else(|| McpError::AgentOperationFailed("mcp.servers is not a table".into()))?;

        servers_table.insert(name.to_owned(), transport_to_toml(transport));

        let output = toml::to_string_pretty(&doc)
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to serialize TOML: {e}")))?;

        tokio::fs::write(path, output)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to write {config_path}: {e}")))?;

        Ok(())
    }

    async fn remove_server(&self, name: &str) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let config_path = get_config_path().await?;
        let path = std::path::Path::new(&config_path);

        if !path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {config_path}: {e}")))?;

        let mut doc: toml::Value = content
            .parse()
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to parse TOML: {e}")))?;

        let removed = doc
            .as_table_mut()
            .and_then(|root| root.get_mut("mcp"))
            .and_then(|mcp| mcp.as_table_mut())
            .and_then(|mcp| mcp.get_mut("servers"))
            .and_then(|servers| servers.as_table_mut())
            .map(|servers| servers.remove(name).is_some())
            .unwrap_or(false);

        if !removed {
            // Idempotent: not found is fine
            return Ok(());
        }

        let output = toml::to_string_pretty(&doc)
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to serialize TOML: {e}")))?;

        tokio::fs::write(path, output)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to write {config_path}: {e}")))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run `nomi --config-path` to get the TOML config file path.
async fn get_config_path() -> Result<String, McpError> {
    let stdout = run_cli_strict(CLI_NAME, &["--config-path"], DETECT_TIMEOUT).await?;
    let path = stdout.trim().to_owned();
    if path.is_empty() {
        return Err(McpError::AgentOperationFailed(
            "nomi --config-path returned empty output".into(),
        ));
    }
    Ok(path)
}

/// Parse MCP servers from TOML config content.
///
/// Expects `[mcp.servers.<name>]` tables.
fn parse_toml_servers(content: &str) -> Result<Vec<DetectedServer>, McpError> {
    let doc: toml::Value = content
        .parse()
        .map_err(|e| McpError::AgentOperationFailed(format!("failed to parse TOML: {e}")))?;

    let servers_table = match doc
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .and_then(|s| s.as_table())
    {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let mut servers = Vec::new();
    for (name, config) in servers_table {
        if let Some(server) = parse_toml_server_entry(name, config) {
            servers.push(server);
        }
    }

    Ok(servers)
}

/// Parse a single server entry from the TOML `[mcp.servers.*]` table.
fn parse_toml_server_entry(name: &str, config: &toml::Value) -> Option<DetectedServer> {
    let table = config.as_table()?;

    let transport_type = table
        .get("transport")
        .or_else(|| table.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("stdio");

    let transport = match transport_type {
        "stdio" => {
            let command = table.get("command")?.as_str()?.to_owned();
            let args = table
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let env = table
                .get("env")
                .and_then(|v| v.as_table())
                .map(|t| {
                    t.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            McpServerTransport::Stdio { command, args, env }
        }
        "sse" => {
            let url = table.get("url")?.as_str()?.to_owned();
            let headers = parse_toml_headers(table);
            McpServerTransport::Sse { url, headers }
        }
        "http" | "streamable_http" => {
            let url = table.get("url")?.as_str()?.to_owned();
            let headers = parse_toml_headers(table);
            McpServerTransport::Http { url, headers }
        }
        _ => return None,
    };

    Some(DetectedServer {
        name: name.to_owned(),
        transport,
        importable: true,
        import_skip_reason: None,
    })
}

/// Extract headers from a TOML table's `headers` field.
fn parse_toml_headers(table: &toml::map::Map<String, toml::Value>) -> HashMap<String, String> {
    table
        .get("headers")
        .and_then(|v| v.as_table())
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Convert a `McpServerTransport` to a TOML value for writing to config.
fn transport_to_toml(transport: &McpServerTransport) -> toml::Value {
    let mut table = toml::map::Map::new();

    match transport {
        McpServerTransport::Stdio { command, args, env } => {
            table.insert("transport".into(), toml::Value::String("stdio".into()));
            table.insert("command".into(), toml::Value::String(command.clone()));
            if !args.is_empty() {
                table.insert(
                    "args".into(),
                    toml::Value::Array(args.iter().map(|a| toml::Value::String(a.clone())).collect()),
                );
            }
            if !env.is_empty() {
                let env_table: toml::map::Map<String, toml::Value> = env
                    .iter()
                    .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
                    .collect();
                table.insert("env".into(), toml::Value::Table(env_table));
            }
        }
        McpServerTransport::Sse { url, headers } => {
            table.insert("transport".into(), toml::Value::String("sse".into()));
            table.insert("url".into(), toml::Value::String(url.clone()));
            insert_toml_headers(&mut table, headers);
        }
        McpServerTransport::Http { url, headers } => {
            table.insert("transport".into(), toml::Value::String("http".into()));
            table.insert("url".into(), toml::Value::String(url.clone()));
            insert_toml_headers(&mut table, headers);
        }
    }

    toml::Value::Table(table)
}

/// Insert headers into a TOML table if non-empty.
fn insert_toml_headers(table: &mut toml::map::Map<String, toml::Value>, headers: &HashMap<String, String>) {
    if !headers.is_empty() {
        let headers_table: toml::map::Map<String, toml::Value> = headers
            .iter()
            .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
            .collect();
        table.insert("headers".into(), toml::Value::Table(headers_table));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_nomi() {
        assert_eq!(NomiAdapter.source(), McpSource::Nomi);
    }

    // -- parse_toml_servers ---------------------------------------------------

    #[test]
    fn parse_empty_config() {
        let servers = parse_toml_servers("").unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_no_mcp_section() {
        let toml = r#"
[some_other]
key = "value"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_empty_servers() {
        let toml = r#"
[mcp]
[mcp.servers]
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_stdio_server() {
        let toml = r#"
[mcp.servers.test-mcp]
type = "stdio"
command = "npx"
args = ["-y", "@test/server"]

[mcp.servers.test-mcp.env]
KEY = "VALUE"
NODE_ENV = "production"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "test-mcp");
        match &servers[0].transport {
            McpServerTransport::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "@test/server"]);
                assert_eq!(env.get("KEY").unwrap(), "VALUE");
                assert_eq!(env.get("NODE_ENV").unwrap(), "production");
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_stdio_server_with_transport_key() {
        let toml = r#"
[mcp.servers.test-mcp]
transport = "stdio"
command = "npx"
args = ["-y", "@test/server"]

[mcp.servers.test-mcp.env]
KEY = "VALUE"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "@test/server"]);
                assert_eq!(env.get("KEY").unwrap(), "VALUE");
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_http_server() {
        let toml = r#"
[mcp.servers.remote]
type = "http"
url = "https://example.com/mcp"

[mcp.servers.remote.headers]
Authorization = "Bearer tok"
"#;
        let servers = parse_toml_servers(toml).unwrap();
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
    fn parse_sse_server() {
        let toml = r#"
[mcp.servers.sse-srv]
type = "sse"
url = "https://example.com/sse"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Sse { url, .. } => {
                assert_eq!(url, "https://example.com/sse");
            }
            _ => panic!("expected Sse"),
        }
    }

    #[test]
    fn parse_streamable_http_becomes_http() {
        let toml = r#"
[mcp.servers.sh]
type = "streamable_http"
url = "https://example.com/api"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Http { .. }));
    }

    #[test]
    fn parse_unknown_transport_skipped() {
        let toml = r#"
[mcp.servers.ws]
type = "websocket"
url = "ws://localhost"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_stdio_missing_command_skipped() {
        let toml = r#"
[mcp.servers.bad]
type = "stdio"
args = []
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_multiple_servers() {
        let toml = r#"
[mcp.servers.srv-a]
type = "stdio"
command = "node"

[mcp.servers.srv-b]
type = "http"
url = "https://b.com/mcp"

[mcp.servers.srv-c]
type = "sse"
url = "https://c.com/sse"
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 3);
    }

    #[test]
    fn parse_default_type_is_stdio() {
        let toml = r#"
[mcp.servers.no-type]
command = "node"
args = ["srv.js"]
"#;
        let servers = parse_toml_servers(toml).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Stdio { .. }));
    }

    // -- transport_to_toml roundtrip ------------------------------------------

    #[test]
    fn stdio_to_toml_roundtrip() {
        let transport = McpServerTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@test/srv".into()],
            env: HashMap::from([("K".into(), "V".into())]),
        };
        let toml_val = transport_to_toml(&transport);
        let table = toml_val.as_table().unwrap();
        assert_eq!(table.get("transport").unwrap().as_str().unwrap(), "stdio");
        assert!(table.get("type").is_none());
        assert_eq!(
            table
                .get("env")
                .and_then(|v| v.as_table())
                .and_then(|env| env.get("K"))
                .and_then(|v| v.as_str())
                .unwrap(),
            "V"
        );
        let server = parse_toml_server_entry("test", &toml_val).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn http_to_toml_roundtrip() {
        let transport = McpServerTransport::Http {
            url: "https://example.com/mcp".into(),
            headers: HashMap::from([("Authorization".into(), "Bearer tok".into())]),
        };
        let toml_val = transport_to_toml(&transport);
        let server = parse_toml_server_entry("test", &toml_val).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn sse_to_toml_roundtrip() {
        let transport = McpServerTransport::Sse {
            url: "https://example.com/sse".into(),
            headers: HashMap::new(),
        };
        let toml_val = transport_to_toml(&transport);
        let server = parse_toml_server_entry("test", &toml_val).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn stdio_to_toml_omits_empty_args_and_env() {
        let transport = McpServerTransport::Stdio {
            command: "node".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let toml_val = transport_to_toml(&transport);
        let table = toml_val.as_table().unwrap();
        assert!(table.get("args").is_none());
        assert!(table.get("env").is_none());
    }

    #[test]
    fn http_to_toml_omits_empty_headers() {
        let transport = McpServerTransport::Http {
            url: "https://x.com".into(),
            headers: HashMap::new(),
        };
        let toml_val = transport_to_toml(&transport);
        let table = toml_val.as_table().unwrap();
        assert!(table.get("headers").is_none());
    }

    // -- invalid TOML ---------------------------------------------------------

    #[test]
    fn parse_invalid_toml_fails() {
        let result = parse_toml_servers("not valid toml [[[");
        assert!(result.is_err());
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(NomiAdapter);
        assert_eq!(adapter.source(), McpSource::Nomi);
    }
}
