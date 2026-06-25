pub use nomifun_common::constants::TEAM_CAPABLE_BACKENDS;
use nomifun_common::constants::is_team_capable;

/// Determine if a backend supports team mode.
///
/// Hard whitelist always passes. For non-whitelisted backends, checks the
/// persisted `agent_capabilities` JSON for MCP transport declarations.
pub fn is_team_capable_backend(backend: &str, agent_capabilities: Option<&serde_json::Value>) -> bool {
    is_team_capable(backend, agent_capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whitelist_backend_is_capable_regardless_of_capabilities() {
        assert!(is_team_capable_backend("claude", None));
        assert!(is_team_capable_backend("claude", Some(&json!({}))));
        assert!(is_team_capable_backend("codex", None));
        assert!(is_team_capable_backend("gemini", None));
        assert!(is_team_capable_backend("nomi", None));
        assert!(is_team_capable_backend("codebuddy", None));
    }

    #[test]
    fn non_whitelist_backend_with_mcp_capabilities_is_capable() {
        let caps_stdio = json!({"mcp_capabilities": {"stdio": true}});
        assert!(is_team_capable_backend("qwen", Some(&caps_stdio)));

        let caps_http = json!({"mcpCapabilities": {"http": true, "sse": true}});
        assert!(is_team_capable_backend("droid", Some(&caps_http)));

        let caps_mcp = json!({"mcp": {"stdio": true}});
        assert!(is_team_capable_backend("goose", Some(&caps_mcp)));
    }

    #[test]
    fn non_whitelist_backend_without_mcp_capabilities_is_not_capable() {
        assert!(!is_team_capable_backend("custom", None));
        assert!(!is_team_capable_backend("custom", Some(&json!({}))));
        assert!(!is_team_capable_backend("", None));
        assert!(!is_team_capable_backend("Claude", None));
    }
}
