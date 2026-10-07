use nomifun_common::McpSource;

use crate::error::McpError;
use crate::types::McpServerTransport;

// ---------------------------------------------------------------------------
// DetectedServer — lightweight server info from Agent CLI detection
// ---------------------------------------------------------------------------

/// A server configuration detected from an Agent CLI.
///
/// Returned by `McpAgentAdapter::detect_existing()`. Contains only the
/// fields needed for diff comparison during sync operations (name +
/// transport). The full `McpServer` model includes DB-level metadata
/// (id, timestamps, etc.) that CLI detection cannot provide.
#[derive(Debug, Clone)]
pub struct DetectedServer {
    /// Server name as registered in the Agent CLI.
    pub name: String,
    /// Transport configuration detected from the Agent CLI.
    pub transport: McpServerTransport,
    /// Whether this detected MCP can be imported without extra intervention.
    pub importable: bool,
    /// Human-readable reason when the MCP is not currently importable.
    pub import_skip_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// McpAgentAdapter — trait for Agent CLI adapters
// ---------------------------------------------------------------------------

/// Abstraction for read-only AI Agent CLI MCP configuration discovery.
///
/// Each Agent CLI (Claude, Gemini, Qwen, etc.) implements this trait to
/// provide detection of existing MCP server configurations.
///
/// # Concurrency
///
/// The read-only sync service serializes scans within each service instance.
///
/// # Error handling
///
/// Methods return `McpError` rather than `AppError` to keep the adapter
/// layer independent of HTTP concerns.
#[async_trait::async_trait]
pub trait McpAgentAdapter: Send + Sync {
    /// Returns the agent source identifier (e.g., `McpSource::Claude`).
    fn source(&self) -> McpSource;

    /// Checks whether the Agent CLI is installed on this machine.
    ///
    /// Typically implemented via `which <cli-name>` or checking a known
    /// config directory.
    async fn is_installed(&self) -> Result<bool, McpError>;

    /// Reads the currently configured MCP servers from this Agent CLI.
    ///
    /// Returns an empty vec if the CLI is installed but has no MCP servers.
    /// Returns `McpError::AgentNotInstalled` if the CLI is not available.
    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError>;
}
