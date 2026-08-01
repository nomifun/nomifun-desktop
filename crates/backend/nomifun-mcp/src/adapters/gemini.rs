use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;

use super::cli_helpers::{DETECT_TIMEOUT, is_cli_installed, parse_standard_list_output, run_cli};

const CLI_NAME: &str = "gemini";

/// MCP Agent adapter for Gemini CLI.
///
/// # CLI Commands
///
/// - **detect**: `gemini mcp list`
pub struct GeminiAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for GeminiAdapter {
    fn source(&self) -> McpSource {
        McpSource::Gemini
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
    fn source_is_gemini() {
        assert_eq!(GeminiAdapter.source(), McpSource::Gemini);
    }

    #[test]
    fn parse_gemini_list_output() {
        let output = "\
Configured MCP servers:
✓ my-server: npx -y @test/server (stdio) - Connected
✗ broken: node bad.js (stdio) - Disconnected
✓ remote: https://example.com/mcp (http) - Connected";

        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].name, "my-server");
        assert_eq!(servers[1].name, "broken");
        assert_eq!(servers[2].name, "remote");

        match &servers[0].transport {
            McpServerTransport::Stdio { command, .. } => {
                assert_eq!(command, "npx -y @test/server");
            }
            _ => panic!("expected Stdio"),
        }

        match &servers[2].transport {
            McpServerTransport::Http { url, .. } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn parse_gemini_empty() {
        let servers = parse_standard_list_output("No MCP servers configured.");
        assert!(servers.is_empty());
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(GeminiAdapter);
        assert_eq!(adapter.source(), McpSource::Gemini);
    }
}
