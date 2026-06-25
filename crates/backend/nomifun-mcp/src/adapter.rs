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

/// Abstraction for AI Agent CLI MCP configuration management.
///
/// Each Agent CLI (Claude, Gemini, Qwen, etc.) implements this trait to
/// provide detection, installation, and removal of MCP server configurations.
///
/// # Concurrency
///
/// Implementations do **not** need to handle concurrency internally.
/// The sync service applies per-agent serialization locks before calling
/// adapter methods.
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

    /// Installs (or updates) an MCP server configuration in this Agent CLI.
    ///
    /// The `name` and `transport` fields from `server` are used to configure
    /// the Agent CLI. If a server with the same name already exists in the
    /// CLI, it should be replaced.
    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError>;

    /// Removes an MCP server configuration from this Agent CLI by name.
    ///
    /// Should be idempotent: removing a non-existent server is not an error.
    async fn remove_server(&self, name: &str) -> Result<(), McpError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// In-memory mock adapter for testing the trait interface.
    struct MockAdapter {
        source: McpSource,
        installed: bool,
        servers: Arc<Mutex<Vec<DetectedServer>>>,
    }

    impl MockAdapter {
        fn new(source: McpSource, installed: bool) -> Self {
            Self {
                source,
                installed,
                servers: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl McpAgentAdapter for MockAdapter {
        fn source(&self) -> McpSource {
            self.source
        }

        async fn is_installed(&self) -> Result<bool, McpError> {
            Ok(self.installed)
        }

        async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
            if !self.installed {
                return Err(McpError::AgentNotInstalled(format!("{:?}", self.source)));
            }
            let servers = self.servers.lock().unwrap();
            Ok(servers.clone())
        }

        async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
            if !self.installed {
                return Err(McpError::AgentNotInstalled(format!("{:?}", self.source)));
            }
            let mut servers = self.servers.lock().unwrap();
            servers.retain(|s| s.name != name);
            servers.push(DetectedServer {
                name: name.to_owned(),
                transport: transport.clone(),
                importable: true,
                import_skip_reason: None,
            });
            Ok(())
        }

        async fn remove_server(&self, name: &str) -> Result<(), McpError> {
            if !self.installed {
                return Err(McpError::AgentNotInstalled(format!("{:?}", self.source)));
            }
            let mut servers = self.servers.lock().unwrap();
            servers.retain(|s| s.name != name);
            Ok(())
        }
    }

    #[tokio::test]
    async fn mock_adapter_source() {
        let adapter = MockAdapter::new(McpSource::Claude, true);
        assert_eq!(adapter.source(), McpSource::Claude);
    }

    #[tokio::test]
    async fn mock_adapter_is_installed() {
        let installed = MockAdapter::new(McpSource::Gemini, true);
        assert!(installed.is_installed().await.unwrap());

        let not_installed = MockAdapter::new(McpSource::Gemini, false);
        assert!(!not_installed.is_installed().await.unwrap());
    }

    #[tokio::test]
    async fn mock_adapter_install_and_detect() {
        let adapter = MockAdapter::new(McpSource::Claude, true);
        let transport = McpServerTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@test/server".into()],
            env: HashMap::new(),
        };

        adapter.install_server("test-mcp", &transport).await.unwrap();

        let detected = adapter.detect_existing().await.unwrap();
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].name, "test-mcp");
        assert_eq!(detected[0].transport, transport);
    }

    #[tokio::test]
    async fn mock_adapter_install_replaces_existing() {
        let adapter = MockAdapter::new(McpSource::Claude, true);
        let t1 = McpServerTransport::Stdio {
            command: "old".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let t2 = McpServerTransport::Stdio {
            command: "new".into(),
            args: vec![],
            env: HashMap::new(),
        };

        adapter.install_server("test-mcp", &t1).await.unwrap();
        adapter.install_server("test-mcp", &t2).await.unwrap();

        let detected = adapter.detect_existing().await.unwrap();
        assert_eq!(detected.len(), 1);
        match &detected[0].transport {
            McpServerTransport::Stdio { command, .. } => assert_eq!(command, "new"),
            _ => panic!("expected Stdio"),
        }
    }

    #[tokio::test]
    async fn mock_adapter_remove() {
        let adapter = MockAdapter::new(McpSource::Claude, true);
        let transport = McpServerTransport::Http {
            url: "http://x".into(),
            headers: HashMap::new(),
        };
        adapter.install_server("srv", &transport).await.unwrap();
        adapter.remove_server("srv").await.unwrap();

        let detected = adapter.detect_existing().await.unwrap();
        assert!(detected.is_empty());
    }

    #[tokio::test]
    async fn mock_adapter_remove_nonexistent_is_idempotent() {
        let adapter = MockAdapter::new(McpSource::Claude, true);
        // Should not error
        adapter.remove_server("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn not_installed_detect_fails() {
        let adapter = MockAdapter::new(McpSource::Qwen, false);
        let result = adapter.detect_existing().await;
        assert!(matches!(result, Err(McpError::AgentNotInstalled(_))));
    }

    #[tokio::test]
    async fn not_installed_install_fails() {
        let adapter = MockAdapter::new(McpSource::Qwen, false);
        let transport = McpServerTransport::Stdio {
            command: "x".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let result = adapter.install_server("srv", &transport).await;
        assert!(matches!(result, Err(McpError::AgentNotInstalled(_))));
    }

    #[tokio::test]
    async fn trait_is_object_safe() {
        let adapter: Arc<dyn McpAgentAdapter> = Arc::new(MockAdapter::new(McpSource::Nomifun, true));
        assert_eq!(adapter.source(), McpSource::Nomifun);
        assert!(adapter.is_installed().await.unwrap());
    }
}
