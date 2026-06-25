use std::collections::HashMap;
use std::path::PathBuf;

use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;
use crate::types::McpServerTransport;

/// MCP Agent adapter for Opencode.
///
/// Opencode stores configuration in `~/.config/opencode/opencode.json`.
/// The `mcp` field is a map of server names to transport configs.
///
/// # Config Format (JSONC)
///
/// ```jsonc
/// {
///   // other opencode config...
///   "mcp": {
///     "server-name": {
///       "type": "stdio",
///       "command": "npx",
///       "args": ["-y", "@test/server"],
///       "env": { "KEY": "VALUE" }
///     },
///     "remote-server": {
///       "type": "http",
///       "url": "https://example.com/mcp",
///       "headers": { "Authorization": "Bearer xxx" }
///     }
///   }
/// }
/// ```
///
/// Opencode config files may contain JSON comments (JSONC), so we
/// strip comments before parsing and preserve the original structure
/// when writing back.
pub struct OpencodeAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for OpencodeAdapter {
    fn source(&self) -> McpSource {
        McpSource::OpenCode
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        Ok(config_dir().is_some_and(|d| d.exists()))
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        let path = config_file_path().ok_or_else(|| McpError::AgentNotInstalled("opencode".into()))?;

        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {}: {e}", path.display())))?;

        let root = parse_jsonc(&content)?;
        parse_mcp_field(&root)
    }

    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
        let path = config_file_path().ok_or_else(|| McpError::AgentNotInstalled("opencode".into()))?;

        let mut root = if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {}: {e}", path.display())))?;
            parse_jsonc(&content)?
        } else {
            // Ensure directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| McpError::AgentOperationFailed(format!("failed to create dir: {e}")))?;
            }
            serde_json::json!({})
        };

        let mcp = root
            .as_object_mut()
            .ok_or_else(|| McpError::AgentOperationFailed("config root is not an object".into()))?
            .entry("mcp")
            .or_insert_with(|| serde_json::json!({}));

        let mcp_obj = mcp
            .as_object_mut()
            .ok_or_else(|| McpError::AgentOperationFailed("mcp field is not an object".into()))?;

        mcp_obj.insert(name.to_owned(), transport_to_json(transport));

        let output = serde_json::to_string_pretty(&root)
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to serialize config: {e}")))?;

        tokio::fs::write(&path, output)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to write {}: {e}", path.display())))?;

        Ok(())
    }

    async fn remove_server(&self, name: &str) -> Result<(), McpError> {
        let path = config_file_path().ok_or_else(|| McpError::AgentNotInstalled("opencode".into()))?;

        if !path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to read {}: {e}", path.display())))?;

        let mut root = parse_jsonc(&content)?;

        let removed = root
            .as_object_mut()
            .and_then(|obj| obj.get_mut("mcp"))
            .and_then(|mcp| mcp.as_object_mut())
            .map(|mcp_obj| mcp_obj.remove(name).is_some())
            .unwrap_or(false);

        if !removed {
            // Idempotent: not found is fine
            return Ok(());
        }

        let output = serde_json::to_string_pretty(&root)
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to serialize config: {e}")))?;

        tokio::fs::write(&path, output)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("failed to write {}: {e}", path.display())))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `~/.config/opencode/` if HOME is available.
fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("opencode"))
}

/// Returns `~/.config/opencode/opencode.json` if HOME is available.
fn config_file_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("opencode.json"))
}

/// Strip single-line (`//`) and multi-line (`/* ... */`) JSON comments.
///
/// Preserves string contents (comments inside strings are left alone).
fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for string literal
        if bytes[i] == b'"' {
            result.push('"');
            i += 1;
            // Consume until closing quote, respecting escapes
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    result.push(bytes[i] as char);
                    result.push(bytes[i + 1] as char);
                    i += 2;
                } else if bytes[i] == b'"' {
                    result.push('"');
                    i += 1;
                    break;
                } else {
                    result.push(bytes[i] as char);
                    i += 1;
                }
            }
        } else if bytes[i] == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' {
                // Single-line comment: skip until newline
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            } else if bytes[i + 1] == b'*' {
                // Multi-line comment: skip until */
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                // Handle unterminated block comment
                if i >= len {
                    break;
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Parse JSONC (JSON with comments) into a `serde_json::Value`.
fn parse_jsonc(input: &str) -> Result<serde_json::Value, McpError> {
    let stripped = strip_json_comments(input);
    serde_json::from_str(&stripped).map_err(McpError::from)
}

/// Extract MCP servers from the parsed config root.
fn parse_mcp_field(root: &serde_json::Value) -> Result<Vec<DetectedServer>, McpError> {
    let mcp = match root.get("mcp") {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let mcp_obj = mcp
        .as_object()
        .ok_or_else(|| McpError::AgentOperationFailed("mcp field is not an object".into()))?;

    let mut servers = Vec::new();

    for (name, config) in mcp_obj {
        if let Some(server) = parse_server_entry(name, config) {
            servers.push(server);
        }
    }

    Ok(servers)
}

/// Parse a single server entry from the `mcp` object.
fn parse_server_entry(name: &str, config: &serde_json::Value) -> Option<DetectedServer> {
    let transport_type = config.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");

    let transport = match transport_type {
        "stdio" => {
            let command = config.get("command")?.as_str()?.to_owned();
            let args = config
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let env = config
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            McpServerTransport::Stdio { command, args, env }
        }
        "sse" => {
            let url = config.get("url")?.as_str()?.to_owned();
            let headers = parse_headers(config);
            McpServerTransport::Sse { url, headers }
        }
        "http" | "streamable_http" => {
            let url = config.get("url")?.as_str()?.to_owned();
            let headers = parse_headers(config);
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

/// Extract headers from a config object's `headers` field.
fn parse_headers(config: &serde_json::Value) -> HashMap<String, String> {
    config
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Convert a `McpServerTransport` to a JSON value for writing to config.
fn transport_to_json(transport: &McpServerTransport) -> serde_json::Value {
    match transport {
        McpServerTransport::Stdio { command, args, env } => {
            let mut obj = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
            });
            if !env.is_empty() {
                obj["env"] = serde_json::json!(env);
            }
            obj
        }
        McpServerTransport::Sse { url, headers } => {
            let mut obj = serde_json::json!({
                "type": "sse",
                "url": url,
            });
            if !headers.is_empty() {
                obj["headers"] = serde_json::json!(headers);
            }
            obj
        }
        McpServerTransport::Http { url, headers } => {
            let mut obj = serde_json::json!({
                "type": "http",
                "url": url,
            });
            if !headers.is_empty() {
                obj["headers"] = serde_json::json!(headers);
            }
            obj
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_opencode() {
        assert_eq!(OpencodeAdapter.source(), McpSource::OpenCode);
    }

    // -- strip_json_comments --------------------------------------------------

    #[test]
    fn strip_single_line_comments() {
        let input = r#"{
  // This is a comment
  "key": "value" // inline comment
}"#;
        let stripped = strip_json_comments(input);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn strip_multi_line_comments() {
        let input = r#"{
  /* multi-line
     comment */
  "key": "value"
}"#;
        let stripped = strip_json_comments(input);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn preserve_comments_inside_strings() {
        let input = r#"{
  "key": "value with // comment inside",
  "key2": "value with /* block */ inside"
}"#;
        let stripped = strip_json_comments(input);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["key"], "value with // comment inside");
        assert_eq!(parsed["key2"], "value with /* block */ inside");
    }

    #[test]
    fn strip_comments_preserves_escaped_quotes() {
        let input = r#"{"key": "val\"ue // not a comment"}"#;
        let stripped = strip_json_comments(input);
        let parsed: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["key"], "val\"ue // not a comment");
    }

    #[test]
    fn strip_no_comments() {
        let input = r#"{"key": "value"}"#;
        assert_eq!(strip_json_comments(input), input);
    }

    // -- parse_mcp_field ------------------------------------------------------

    #[test]
    fn parse_empty_mcp() {
        let root = serde_json::json!({ "mcp": {} });
        let servers = parse_mcp_field(&root).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_no_mcp_field() {
        let root = serde_json::json!({ "other": "stuff" });
        let servers = parse_mcp_field(&root).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_stdio_server() {
        let root = serde_json::json!({
            "mcp": {
                "test-mcp": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "@test/server"],
                    "env": { "KEY": "VALUE" }
                }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "test-mcp");
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
        let root = serde_json::json!({
            "mcp": {
                "remote": {
                    "type": "http",
                    "url": "https://example.com/mcp",
                    "headers": { "Authorization": "Bearer tok" }
                }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
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
        let root = serde_json::json!({
            "mcp": {
                "sse-srv": {
                    "type": "sse",
                    "url": "https://example.com/sse"
                }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
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
        let root = serde_json::json!({
            "mcp": {
                "sh": {
                    "type": "streamable_http",
                    "url": "https://example.com/api"
                }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Http { .. }));
    }

    #[test]
    fn parse_unknown_transport_skipped() {
        let root = serde_json::json!({
            "mcp": {
                "ws": { "type": "websocket", "url": "ws://localhost" }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_stdio_missing_command_skipped() {
        let root = serde_json::json!({
            "mcp": {
                "bad": { "type": "stdio", "args": [] }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_multiple_servers() {
        let root = serde_json::json!({
            "mcp": {
                "srv-a": { "type": "stdio", "command": "node" },
                "srv-b": { "type": "http", "url": "https://b.com/mcp" }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn parse_default_type_is_stdio() {
        let root = serde_json::json!({
            "mcp": {
                "no-type": { "command": "node", "args": ["srv.js"] }
            }
        });
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpServerTransport::Stdio { .. }));
    }

    // -- transport_to_json ----------------------------------------------------

    #[test]
    fn stdio_to_json_roundtrip() {
        let transport = McpServerTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@test/srv".into()],
            env: HashMap::from([("K".into(), "V".into())]),
        };
        let json = transport_to_json(&transport);
        let server = parse_server_entry("test", &json).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn http_to_json_roundtrip() {
        let transport = McpServerTransport::Http {
            url: "https://example.com/mcp".into(),
            headers: HashMap::from([("Authorization".into(), "Bearer tok".into())]),
        };
        let json = transport_to_json(&transport);
        let server = parse_server_entry("test", &json).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn sse_to_json_roundtrip() {
        let transport = McpServerTransport::Sse {
            url: "https://example.com/sse".into(),
            headers: HashMap::new(),
        };
        let json = transport_to_json(&transport);
        let server = parse_server_entry("test", &json).unwrap();
        assert_eq!(server.transport, transport);
    }

    #[test]
    fn stdio_to_json_omits_empty_env() {
        let transport = McpServerTransport::Stdio {
            command: "node".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let json = transport_to_json(&transport);
        assert!(json.get("env").is_none());
    }

    #[test]
    fn http_to_json_omits_empty_headers() {
        let transport = McpServerTransport::Http {
            url: "https://x.com".into(),
            headers: HashMap::new(),
        };
        let json = transport_to_json(&transport);
        assert!(json.get("headers").is_none());
    }

    // -- parse_jsonc ----------------------------------------------------------

    #[test]
    fn parse_jsonc_with_comments() {
        let input = r#"{
  // comment
  "mcp": {
    /* block comment */
    "srv": {
      "type": "stdio",
      "command": "npx"
    }
  }
}"#;
        let root = parse_jsonc(input).unwrap();
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "srv");
    }

    #[test]
    fn parse_jsonc_invalid_json_fails() {
        let result = parse_jsonc("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(OpencodeAdapter);
        assert_eq!(adapter.source(), McpSource::OpenCode);
    }
}
