use tracing::warn;

use crate::types::{ExtMcpServer, ResolvedMcpServer};

/// Resolve a single MCP server contribution.
///
/// MCP server config is passed through as-is (opaque JSON).
pub fn resolve_mcp_server(server: &ExtMcpServer, extension_name: &str) -> ResolvedMcpServer {
    ResolvedMcpServer {
        extension_name: extension_name.to_owned(),
        id: server.id.clone(),
        name: server.name.clone(),
        description: server.description.clone(),
        config: server.config.clone(),
    }
}

/// Resolve all MCP server contributions from an extension.
pub fn resolve_mcp_servers(servers: &[ExtMcpServer], extension_name: &str) -> Vec<ResolvedMcpServer> {
    if servers.is_empty() {
        return Vec::new();
    }
    tracing::debug!(
        extension = extension_name,
        count = servers.len(),
        "Resolving MCP servers"
    );
    servers
        .iter()
        .inspect(|s| {
            if s.id.is_empty() || s.name.is_empty() {
                warn!(
                    extension = extension_name,
                    server_id = s.id,
                    "MCP server has empty id or name"
                );
            }
        })
        .map(|s| resolve_mcp_server(s, extension_name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server() -> ExtMcpServer {
        ExtMcpServer {
            id: "test-mcp".into(),
            name: "Test MCP".into(),
            description: Some("A test MCP server".into()),
            config: serde_json::json!({
                "command": "npx",
                "args": ["-y", "test-server"]
            }),
        }
    }

    #[test]
    fn test_resolve_basic_mcp_server() {
        let server = make_server();
        let result = resolve_mcp_server(&server, "my-ext");

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "test-mcp");
        assert_eq!(result.name, "Test MCP");
        assert_eq!(result.config["command"], "npx");
    }

    #[test]
    fn test_resolve_mcp_servers_empty() {
        let result = resolve_mcp_servers(&[], "my-ext");
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_mcp_servers_multiple() {
        let servers = vec![make_server(), make_server()];
        let result = resolve_mcp_servers(&servers, "my-ext");
        assert_eq!(result.len(), 2);
    }
}
