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
/// strip comments before parsing.
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

/// Parse JSONC while preserving string bytes and comment token boundaries.
fn parse_jsonc(input: &str) -> Result<serde_json::Value, McpError> {
    let mut bytes = input.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    let byte = bytes[i];
                    i += 1;
                    if byte == b'\\' {
                        i += 1;
                    } else if byte == b'"' {
                        break;
                    }
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                    bytes[i] = b' ';
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let start = i;
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err(McpError::AgentOperationFailed("unterminated JSONC comment".into()));
                }
                i += 2;
                for byte in &mut bytes[start..i] {
                    if !matches!(*byte, b'\n' | b'\r') {
                        *byte = b' ';
                    }
                }
            }
            _ => i += 1,
        }
    }
    serde_json::from_slice(&bytes).map_err(McpError::from)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;


    // -- JSONC comments -------------------------------------------------------

    #[test]
    fn strip_single_line_comments() {
        let input = r#"{
  // This is a comment
  "key": "value" // inline comment
}"#;
        let parsed = parse_jsonc(input).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn strip_multi_line_comments() {
        let input = r#"{
  /* multi-line
     comment */
  "key": "value"
}"#;
        let parsed = parse_jsonc(input).unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn preserve_comments_inside_strings() {
        let input = r#"{
  "key": "value with // comment inside",
  "key2": "value with /* block */ inside"
}"#;
        let parsed = parse_jsonc(input).unwrap();
        assert_eq!(parsed["key"], "value with // comment inside");
        assert_eq!(parsed["key2"], "value with /* block */ inside");
    }

    #[test]
    fn strip_comments_preserves_escaped_quotes() {
        let input = r#"{"key": "val\"ue // not a comment"}"#;
        let parsed = parse_jsonc(input).unwrap();
        assert_eq!(parsed["key"], "val\"ue // not a comment");
    }

    #[test]
    fn jsonc_preserves_strings_and_comment_line_endings() {
        let expected = serde_json::json!({ "key": "中文🙂\\\"// /*" });
        let input = expected.to_string();
        for suffix in ["", "// 尾注释", "// 注释\r", "/* 中\r\n文 */"] {
            assert_eq!(parse_jsonc(&(input.clone() + suffix)).unwrap(), expected);
        }
        for separator in ["\r", "\n", "\r\n"] {
            let input = format!("{{// 注释{separator}\"key\": 1}}");
            assert_eq!(parse_jsonc(&input).unwrap(), serde_json::json!({ "key": 1 }));
        }
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

    // -- parse_jsonc ----------------------------------------------------------

    #[test]
    fn parse_jsonc_with_comments() {
        let input = r#"{
  // comment
  "mcp": {
    /* block comment */
    "中文🙂": {
      "type": "stdio",
      "command": "工具",
      "args": ["你好", "// 原样保留", "/* 注释文本 */"]
    }
  }
}"#;
        let root = parse_jsonc(input).unwrap();
        let servers = parse_mcp_field(&root).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "中文🙂");
        match &servers[0].transport {
            McpServerTransport::Stdio { command, args, .. } => {
                assert_eq!(command, "工具");
                assert_eq!(args, &["你好", "// 原样保留", "/* 注释文本 */"]);
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_jsonc_invalid_json_fails() {
        for input in ["not json at all", "1/**/2", "tr/**/ue", "{}/*", "{}/* ", "{}/*x", "{}/*x*"] {
            assert!(parse_jsonc(input).is_err(), "must reject {input:?}");
        }
    }

}
