// Settings-facing result projections. Wire protocol belongs to the shared owner.

use nomifun_api_types::{
    McpAuthMethod, McpConnectionTestErrorCode, McpConnectionTestResult, McpToolResponse,
};
use serde::Deserialize;


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
    }
}

pub(super) fn spawn_error_result(command: &str, error: &std::io::Error) -> McpConnectionTestResult {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            let runtime = command_runtime(command);
            error_result(
                McpConnectionTestErrorCode::CommandNotFound,
                format!("Command not found: {command}: {error}"),
                Some(serde_json::json!({
                    "command": command,
                    "runtime": runtime,
                    "io_error": error.to_string(),
                })),
            )
        }
        std::io::ErrorKind::PermissionDenied => error_result(
            McpConnectionTestErrorCode::CommandPermissionDenied,
            format!("Permission denied: {command}: {error}"),
            Some(serde_json::json!({ "command": command, "io_error": error.to_string() })),
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

pub(super) fn auth_result(headers: &reqwest::header::HeaderMap) -> McpConnectionTestResult {
    let challenges = crate::oauth_protocol::parse_www_authenticate(headers);
    let auth_method = detect_auth_method(&challenges);

    McpConnectionTestResult {
        success: false,
        tools: None,
        error: None,
        code: None,
        details: None,
        needs_auth: Some(true),
        auth_method,
    }
}

fn detect_auth_method(challenges: &[crate::oauth_protocol::AuthenticateChallenge]) -> Option<McpAuthMethod> {
    if challenges.iter().any(|challenge| challenge.scheme == "bearer"
        && ["resource_metadata", "authorization_uri"].iter().any(|parameter|
            challenge.parameters.get(*parameter).is_some_and(|value| !value.is_empty())))
    { Some(McpAuthMethod::Oauth) }
    else if challenges.iter().any(|challenge| challenge.scheme == "basic") { Some(McpAuthMethod::Basic) }
    else { None }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner::is_package_runner;

    // -- Auth detection ---------------------------------------------------

    #[test]
    fn bearer_requires_an_explicit_discovery_hint() {
        assert!(detect_auth_method(&crate::oauth_protocol::parse_challenges("Bearer realm=\"mcp\"")).is_none());
        assert!(matches!(detect_auth_method(&crate::oauth_protocol::parse_challenges(
            "Bearer resource_metadata=\"https://server.example/meta\"")),Some(McpAuthMethod::Oauth)));
    }

    #[test]
    fn schemes_are_not_inferred_from_realm_or_substrings() {
        for header in ["OAuth realm=\"mcp\"", "Digest realm=\"oauth bearer\"", "Bearerish realm=\"mcp\""] {
            assert!(detect_auth_method(&crate::oauth_protocol::parse_challenges(header)).is_none());
        }
    }

    #[test]
    fn detect_basic_auth() {
        assert!(matches!(
            detect_auth_method(&crate::oauth_protocol::parse_challenges("Basic realm=\"mcp\"")),
            Some(McpAuthMethod::Basic)
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
    fn spawn_error_not_found() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("npx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: npx"));
        assert_eq!(result.details.as_ref().unwrap()["io_error"], "not found");
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
        assert_eq!(result.details.as_ref().unwrap()["io_error"], "not found");
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "generic");
    }

    #[test]
    fn spawn_error_for_user_owned_bun_path_uses_generic_command_guidance() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("/user/configured/bunx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: /user/configured/bunx"));
        assert_eq!(result.details.as_ref().unwrap()["io_error"], "not found");
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "bun");
    }

    #[test]
    fn spawn_error_not_found_uv_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("uvx", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: uvx"));
        assert!(error.contains("not found"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "uv");
    }

    #[test]
    fn spawn_error_not_found_python_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("python3", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: python3"));
        assert!(error.contains("not found"));
        assert_eq!(result.details.as_ref().unwrap()["runtime"], "python");
    }

    #[test]
    fn spawn_error_not_found_deno_command() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let result = spawn_error_result("deno", &err);
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("Command not found: deno"));
        assert!(error.contains("not found"));
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
    fn auth_result_with_bearer_preserves_unknown_credential_type() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("www-authenticate", "Bearer realm=\"mcp\"".parse().unwrap());
        let result = auth_result(&headers);
        assert!(!result.success);
        assert_eq!(result.needs_auth, Some(true));
        assert!(result.auth_method.is_none());
        assert!(result.code.is_none());
    }

    #[test]
    fn auth_result_reads_all_challenges_without_exposing_provider_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append("www-authenticate", "Basic realm=\"reflected-private-credential\"".parse().unwrap());
        headers.append("www-authenticate", "Bearer resource_metadata=\"https://server.example/meta?token=private-fixture-value\"".parse().unwrap());
        let result = auth_result(&headers);
        assert!(matches!(result.auth_method, Some(McpAuthMethod::Oauth)));
        let projection = serde_json::to_string(&result).unwrap();
        assert!(!projection.contains("reflected-private-credential"));
        assert!(!projection.contains("private-fixture-value"));
        assert!(!projection.contains("www_authenticate"));
    }

    #[test]
    fn auth_result_without_www_authenticate() {
        let headers = reqwest::header::HeaderMap::new();
        let result = auth_result(&headers);
        assert_eq!(result.needs_auth, Some(true));
        assert!(result.auth_method.is_none());
    }
}
