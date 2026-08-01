use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;

use super::cli_helpers::{DETECT_TIMEOUT, is_cli_installed, parse_standard_list_output, run_cli};

const CLI_NAME: &str = "qwen";

/// MCP Agent adapter for Qwen CLI.
///
/// # CLI Commands
///
/// - **detect**: `qwen mcp list`
pub struct QwenAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for QwenAdapter {
    fn source(&self) -> McpSource {
        McpSource::Qwen
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        is_cli_installed(CLI_NAME).await
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let (stdout, _stderr) = run_cli(CLI_NAME, &["mcp", "list"], DETECT_TIMEOUT).await?;
        Ok(parse_standard_list_output(&stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::McpServerTransport;

    #[test]
    fn source_is_qwen() {
        assert_eq!(QwenAdapter.source(), McpSource::Qwen);
    }

    #[test]
    fn parse_qwen_list_output() {
        let output = "\
✓ my-server: npx -y @test/server (stdio) - Connected
✗ broken: node bad.js (stdio) - Disconnected";

        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "my-server");
        assert_eq!(servers[1].name, "broken");
    }

    #[test]
    fn parse_qwen_http_server() {
        let output = "✓ remote: https://example.com/mcp (http) - Connected";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Http { url, .. } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(QwenAdapter);
        assert_eq!(adapter.source(), McpSource::Qwen);
    }
}
