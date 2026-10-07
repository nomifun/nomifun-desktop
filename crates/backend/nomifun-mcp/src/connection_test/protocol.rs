// Settings-facing result projections. Wire protocol belongs to the shared owner.

use nomifun_api_types::{
    McpAuthMethod, McpConnectionTestErrorCode, McpConnectionTestResult, McpToolResponse,
};
use serde::Deserialize;

use std::time::Duration;

#[derive(Debug, Deserialize)]
struct ToolsListResult {
    tools: Vec<McpToolInfo>,
}

#[derive(Debug, Deserialize)]
struct McpToolInfo {
    name: String,
    description: Option<String>,
    #[serde(rename = "inputSchema")]
    input_schema: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Result builders
// ---------------------------------------------------------------------------

pub(super) fn success_result(tools_value: Option<serde_json::Value>) -> McpConnectionTestResult {
    let Some(result) = tools_value
        .filter(|value| {
            value
                .get("tools")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tools| tools.iter().all(serde_json::Value::is_object))
        })
        .and_then(|v| serde_json::from_value::<ToolsListResult>(v).ok())
    else {
        return error_result(
            McpConnectionTestErrorCode::ProtocolError,
            "tools/list response has a missing or invalid result".into(),
            Some(serde_json::json!({ "stage": "tools_list_response" })),
        );
    };
    let tools = result
        .tools
        .into_iter()
        .map(|t| McpToolResponse {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        })
        .collect();

    McpConnectionTestResult {
        success: true,
        tools: Some(tools),
        error: None,
        code: None,
        details: None,
        needs_auth: None,
        auth_method: None,
        www_authenticate: None,
    }
}

pub(super) fn error_result(
    code: McpConnectionTestErrorCode,
    msg: String,
    details: Option<serde_json::Value>,
) -> McpConnectionTestResult {
    McpConnectionTestResult {
        success: false,
        tools: None,
        error: Some(msg),
        code: Some(code),
        details,
        needs_auth: None,
        auth_method: None,
        www_authenticate: None,
    }
}

pub(super) fn timeout_result(duration: Duration) -> McpConnectionTestResult {
    error_result(
        McpConnectionTestErrorCode::Timeout,
        format!("Connection test timed out after {}s", duration.as_secs()),
        Some(serde_json::json!({ "timeout_seconds": duration.as_secs() })),
    )
}

pub(super) fn spawn_error_result(command: &str, error: &std::io::Error) -> McpConnectionTestResult {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            let runtime = command_runtime(command);
            error_result(
                McpConnectionTestErrorCode::CommandNotFound,
                command_not_found_message(command),
                Some(serde_json::json!({
                    "command": command,
                    "runtime": runtime,
                })),
            )
        }
        std::io::ErrorKind::PermissionDenied => error_result(
            McpConnectionTestErrorCode::CommandPermissionDenied,
            format!("Permission denied: {command}"),
            Some(serde_json::json!({ "command": command })),
        ),
        _ => error_result(
            McpConnectionTestErrorCode::CommandStartFailed,
            format!("Failed to start '{command}': {error}"),
            Some(serde_json::json!({
                "command": command,
                "io_error": error.to_string(),
            })),
        ),
    }
}

fn command_basename(command: &str) -> String {
    let mut command_name = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat"] {
        if let Some(stripped) = command_name.strip_suffix(suffix) {
            command_name = stripped.to_owned();
            break;
        }
    }
    command_name
}

pub(super) fn command_runtime(command: &str) -> &'static str {
    let command_name = command_basename(command);
    match command_name.as_str() {
        "npx" | "npm" | "node" | "pnpx" => "node",
        "bun" | "bunx" => "bun",
        "uv" | "uvx" => "uv",
        "python" | "python3" => "python",
        "deno" => "deno",
        _ => "generic",
    }
}

fn command_not_found_message(command: &str) -> String {
    match command_runtime(command) {
        "node" => format!(
            "Command not found: {command}. Install Node.js (which includes npm/npx), then restart Nomi or configure this MCP server to use an absolute command path."
        ),
        "bun" => format!(
            "Command not found: {command}. Install Bun (which includes bun/bunx), then restart Nomi or configure this MCP server to use an absolute command path."
        ),
        "uv" => format!(
            "Command not found: {command}. Install uv, then restart Nomi or configure this MCP server to use an absolute command path."
        ),
        "python" => format!(
            "Command not found: {command}. Install Python, then restart Nomi or configure this MCP server to use an absolute command path."
        ),
        "deno" => format!(
            "Command not found: {command}. Install Deno, then restart Nomi or configure this MCP server to use an absolute command path."
        ),
        _ => format!(
            "Command not found: {command}. Install the command or configure this MCP server to use an absolute command path."
        ),
    }
}

pub(super) fn is_package_runner(command: &str, args: &[String]) -> bool {
    let command_name = command_basename(command);
    match command_name.as_str() {
        "npx" | "pnpx" | "bunx" | "uvx" | "pipx" => true,
        "npm" => matches!(args.first().map(String::as_str), Some("exec") | Some("x")),
        "pnpm" => matches!(args.first().map(String::as_str), Some("dlx")),
        "bun" => matches!(args.first().map(String::as_str), Some("x")),
        "uv" => matches!(
            args.first().map(String::as_str),
            Some("run") | Some("tool")
        ),
        _ => false,
    }
}

pub(super) fn auth_result(headers: &reqwest::header::HeaderMap) -> McpConnectionTestResult {
    let www_authenticate = headers
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let auth_method = www_authenticate.as_deref().map(detect_auth_method);

    McpConnectionTestResult {
        success: false,
        tools: None,
        error: None,
        code: None,
        details: None,
        needs_auth: Some(true),
        auth_method,
        www_authenticate,
    }
}

fn detect_auth_method(www_authenticate: &str) -> McpAuthMethod {
    let lower = www_authenticate.to_lowercase();
    if lower.contains("bearer") || lower.contains("oauth") {
        McpAuthMethod::Oauth
    } else {
        McpAuthMethod::Basic
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Auth detection ---------------------------------------------------

    #[test]
    fn detect_bearer_as_oauth() {
        assert!(matches!(
            detect_auth_method("Bearer realm=\"mcp\""),
            McpAuthMethod::Oauth
        ));
    }

    #[test]
    fn detect_oauth_keyword() {
        assert!(matches!(
            detect_auth_method("OAuth realm=\"mcp\""),
            McpAuthMethod::Oauth
        ));
    }

    #[test]
    fn detect_basic_auth() {
        assert!(matches!(
            detect_auth_method("Basic realm=\"mcp\""),
            McpAuthMethod::Basic
        ));
    }

    // -- Result builders --------------------------------------------------

    #[test]
    fn success_result_with_tools() {
        let tools_json = serde_json::json!({
            "tools": [
                { "name": "read_file", "description": "Read a file" },
                { "name": "write_file" }
            ]
        });
        let result = success_result(Some(tools_json));
        assert!(result.success);
        let tools = result.tools.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[0].description.as_deref(), Some("Read a file"));
        assert!(tools[1].description.is_none());
    }

    #[test]
    fn success_result_empty_tools() {
        let tools_json = serde_json::json!({ "tools": [] });
        let result = success_result(Some(tools_json));
        assert!(result.success);
        assert!(result.tools.unwrap().is_empty());
    }

    #[test]
    fn success_result_none_is_protocol_error() {
        let result = success_result(None);
        assert!(!result.success);
        assert_eq!(result.code, Some(McpConnectionTestErrorCode::ProtocolError));
        assert!(result.tools.is_none());
        assert_eq!(result.details.unwrap()["stage"], "tools_list_response");
    }

    #[test]
    fn success_result_malformed_is_protocol_error() {
        for value in [
            serde_json::json!([[]]),
            serde_json::json!({ "tools": [["tool", null, {}]] }),
            serde_json::json!(null),
            serde_json::json!("not an object"),
            serde_json::json!({}),
            serde_json::json!({ "tools": null }),
            serde_json::json!({ "tools": "not an array" }),
            serde_json::json!({ "tools": [{}] }),
            serde_json::json!({ "tools": [{ "name": 1 }] }),
            serde_json::json!({ "tools": [{ "name": "valid" }, null] }),
        ] {
            let result = success_result(Some(value));
            assert!(!result.success);
            assert_eq!(result.code, Some(McpConnectionTestErrorCode::ProtocolError));
            assert!(result.tools.is_none());
            assert_eq!(
                result.error.as_deref(),
                Some("tools/list response has a missing or invalid result")
            );
        }
    }

    #[test]
    fn error_result_fields() {
        let result = error_result(
            McpConnectionTestErrorCode::ProtocolError,
            "something broke".into(),
            Some(serde_json::json!({ "stage": "initialize" })),
        );
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("something broke"));
        assert_eq!(result.code, Some(McpConnectionTestErrorCode::ProtocolError));
        assert_eq!(result.details.unwrap()["stage"], "initialize");
        assert!(result.tools.is_none());
        assert!(result.needs_auth.is_none());
    }

    #[test]
    fn timeout_result_message() {
        let result = timeout_result(Duration::from_secs(30));
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("30s"));
        assert_eq!(result.code, Some(McpConnectionTestErrorCode::Timeout));
    }

    #[test]
    fn spawn_error_not_found() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("npx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: npx"));
        assert!(error.contains("Install Node.js"));
        assert!(error.contains("absolute command path"));
        assert_eq!(
            result.code,
            Some(McpConnectionTestErrorCode::CommandNotFound)
        );
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "node");
    }

    #[test]
    fn package_runner_detection_covers_portable_market_launchers() {
        for (command, args) in [
            ("npx", vec!["-y".into(), "pkg".into()]),
            ("uvx.exe", vec!["pkg".into()]),
            ("bunx", vec!["pkg".into()]),
            ("npm.cmd", vec!["exec".into(), "pkg".into()]),
            ("pnpm", vec!["dlx".into(), "pkg".into()]),
        ] {
            assert!(is_package_runner(command, &args), "{command} {args:?}");
        }
        assert!(!is_package_runner("node", &["server.js".into()]));
        assert!(!is_package_runner("npm", &["start".into()]));
    }

    #[test]
    fn spawn_error_not_found_generic_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("missing-mcp", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: missing-mcp"));
        assert!(error.contains("Install the command"));
        assert!(error.contains("absolute command path"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "generic");
    }

    #[test]
    fn spawn_error_not_found_bun_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("bunx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: bunx"));
        assert!(error.contains("Install Bun"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "bun");
    }

    #[test]
    fn spawn_error_not_found_uv_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("uvx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: uvx"));
        assert!(error.contains("Install uv"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "uv");
    }

    #[test]
    fn spawn_error_not_found_python_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("python3", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: python3"));
        assert!(error.contains("Install Python"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "python");
    }

    #[test]
    fn spawn_error_not_found_deno_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("deno", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: deno"));
        assert!(error.contains("Install Deno"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "deno");
    }

    #[test]
    fn spawn_error_permission_denied() {
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let result = spawn_error_result("./script.sh", &err);
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("Permission denied")
        );
        assert_eq!(
            result.code,
            Some(McpConnectionTestErrorCode::CommandPermissionDenied)
        );
    }

    #[test]
    fn spawn_error_other() {
        let err = std::io::Error::other("broken pipe");
        let result = spawn_error_result("cmd", &err);
        assert!(result.error.as_deref().unwrap().contains("Failed to start"));
        assert_eq!(
            result.code,
            Some(McpConnectionTestErrorCode::CommandStartFailed)
        );
    }

    #[test]
    fn auth_result_with_bearer() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("www-authenticate", "Bearer realm=\"mcp\"".parse().unwrap());
        let result = auth_result(&headers);
        assert!(!result.success);
        assert_eq!(result.needs_auth, Some(true));
        assert!(matches!(result.auth_method, Some(McpAuthMethod::Oauth)));
        assert!(result.www_authenticate.is_some());
        assert!(result.code.is_none());
    }

    #[test]
    fn auth_result_without_www_authenticate() {
        let headers = reqwest::header::HeaderMap::new();
        let result = auth_result(&headers);
        assert_eq!(result.needs_auth, Some(true));
        assert!(result.auth_method.is_none());
        assert!(result.www_authenticate.is_none());
    }
}
