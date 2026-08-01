// ---------------------------------------------------------------------------
// AcpMcpCapabilities
// ---------------------------------------------------------------------------

/// ACP backend MCP capability declaration.
///
/// Describes which transport types the ACP backend supports for
/// spawning MCP servers during a session.
#[derive(Debug, Clone, PartialEq)]
pub struct AcpMcpCapabilities {
    pub stdio: bool,
    pub http: bool,
    pub sse: bool,
}

impl AcpMcpCapabilities {
    /// Returns true if no transport type is supported.
    pub fn is_empty(&self) -> bool {
        !self.stdio && !self.http && !self.sse
    }
}

impl Default for AcpMcpCapabilities {
    fn default() -> Self {
        Self {
            stdio: true,
            http: false,
            sse: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse ACP MCP capabilities from an ACP backend response.
///
/// Looks for capabilities under `mcp_capabilities`, `mcpCapabilities`,
/// or `mcp` keys. Returns default capabilities (stdio only) when the
/// field is missing or not an object.
pub fn parse_acp_mcp_capabilities(response: &serde_json::Value) -> AcpMcpCapabilities {
    let caps = response
        .get("mcp_capabilities")
        .or_else(|| response.get("mcpCapabilities"))
        .or_else(|| response.get("mcp"));

    let Some(caps) = caps else {
        return AcpMcpCapabilities::default();
    };

    let http = bool_field(caps, "http");
    let sse = bool_field(caps, "sse");
    let stdio = bool_field(caps, "stdio") || http || sse;

    AcpMcpCapabilities { stdio, http, sse }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract a boolean field from a JSON value, defaulting to false.
fn bool_field(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn all_caps() -> AcpMcpCapabilities {
        AcpMcpCapabilities {
            stdio: true,
            http: true,
            sse: true,
        }
    }

    // -- AcpMcpCapabilities ---------------------------------------------------

    #[test]
    fn capabilities_default_is_stdio_only() {
        let caps = AcpMcpCapabilities::default();
        assert!(caps.stdio);
        assert!(!caps.http);
        assert!(!caps.sse);
    }

    #[test]
    fn capabilities_is_empty() {
        let empty = AcpMcpCapabilities {
            stdio: false,
            http: false,
            sse: false,
        };
        assert!(empty.is_empty());
        assert!(!AcpMcpCapabilities::default().is_empty());
    }

    // -- parse_acp_mcp_capabilities -------------------------------------------

    #[test]
    fn parse_full_capabilities() {
        let resp = serde_json::json!({
            "mcp_capabilities": { "stdio": true, "http": true, "sse": true }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert_eq!(caps, all_caps());
    }

    #[test]
    fn parse_camel_case_key() {
        let resp = serde_json::json!({
            "mcpCapabilities": { "stdio": true, "http": false, "sse": true }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert!(caps.stdio);
        assert!(!caps.http);
        assert!(caps.sse);
    }

    #[test]
    fn parse_mcp_shorthand_key() {
        let resp = serde_json::json!({
            "mcp": { "stdio": false, "http": true, "sse": false }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert!(caps.stdio);
        assert!(caps.http);
        assert!(!caps.sse);
    }

    #[test]
    fn parse_missing_capabilities_returns_default() {
        let resp = serde_json::json!({ "other": "data" });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert_eq!(caps, AcpMcpCapabilities::default());
    }

    #[test]
    fn parse_partial_capabilities_defaults_missing_to_false() {
        let resp = serde_json::json!({
            "mcp_capabilities": { "stdio": true }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert!(caps.stdio);
        assert!(!caps.http);
        assert!(!caps.sse);
    }

    #[test]
    fn parse_http_support_implies_stdio() {
        let resp = serde_json::json!({
            "mcp_capabilities": { "http": true, "sse": false }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert!(caps.stdio);
        assert!(caps.http);
        assert!(!caps.sse);
    }

    #[test]
    fn parse_priority_mcp_capabilities_over_mcp() {
        let resp = serde_json::json!({
            "mcp_capabilities": { "stdio": true, "http": true, "sse": true },
            "mcp": { "stdio": false, "http": false, "sse": false }
        });
        let caps = parse_acp_mcp_capabilities(&resp);
        assert_eq!(caps, all_caps());
    }
}
