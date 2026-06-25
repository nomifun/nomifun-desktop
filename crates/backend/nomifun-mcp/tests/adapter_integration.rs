//! Integration tests for McpAgentAdapter trait and DetectedServer.
//!
//! Uses a mock adapter to verify the trait's public API contract:
//! object safety, install/detect/remove lifecycle, and error cases.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nomifun_common::McpSource;
use nomifun_mcp::{DetectedServer, McpAgentAdapter, McpError, McpServerTransport};

// ---------------------------------------------------------------------------
// Mock adapter (in-memory, for integration tests)
// ---------------------------------------------------------------------------

struct InMemoryAdapter {
    source: McpSource,
    installed: bool,
    servers: Mutex<Vec<DetectedServer>>,
}

impl InMemoryAdapter {
    fn new(source: McpSource, installed: bool) -> Self {
        Self {
            source,
            installed,
            servers: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl McpAgentAdapter for InMemoryAdapter {
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
        Ok(self.servers.lock().unwrap().clone())
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trait_object_safety_with_arc() {
    let adapter: Arc<dyn McpAgentAdapter> = Arc::new(InMemoryAdapter::new(McpSource::Claude, true));

    assert_eq!(adapter.source(), McpSource::Claude);
    assert!(adapter.is_installed().await.unwrap());
    assert!(adapter.detect_existing().await.unwrap().is_empty());
}

#[tokio::test]
async fn full_lifecycle_install_detect_remove() {
    let adapter = InMemoryAdapter::new(McpSource::Gemini, true);

    // Install two servers
    let t1 = McpServerTransport::Stdio {
        command: "npx".into(),
        args: vec!["-y".into(), "server-a".into()],
        env: HashMap::new(),
    };
    let t2 = McpServerTransport::Http {
        url: "https://example.com/mcp".into(),
        headers: HashMap::from([("Auth".into(), "Bearer x".into())]),
    };

    adapter.install_server("server-a", &t1).await.unwrap();
    adapter.install_server("server-b", &t2).await.unwrap();

    // Detect both
    let detected = adapter.detect_existing().await.unwrap();
    assert_eq!(detected.len(), 2);

    let names: Vec<&str> = detected.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"server-a"));
    assert!(names.contains(&"server-b"));

    // Remove one
    adapter.remove_server("server-a").await.unwrap();
    let detected = adapter.detect_existing().await.unwrap();
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0].name, "server-b");

    // Remove the other
    adapter.remove_server("server-b").await.unwrap();
    let detected = adapter.detect_existing().await.unwrap();
    assert!(detected.is_empty());
}

#[tokio::test]
async fn install_replaces_existing_by_name() {
    let adapter = InMemoryAdapter::new(McpSource::Qwen, true);

    let t1 = McpServerTransport::Stdio {
        command: "old-cmd".into(),
        args: vec![],
        env: HashMap::new(),
    };
    let t2 = McpServerTransport::Stdio {
        command: "new-cmd".into(),
        args: vec!["--flag".into()],
        env: HashMap::new(),
    };

    adapter.install_server("my-server", &t1).await.unwrap();
    adapter.install_server("my-server", &t2).await.unwrap();

    let detected = adapter.detect_existing().await.unwrap();
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0].transport, t2);
}

#[tokio::test]
async fn remove_nonexistent_is_idempotent() {
    let adapter = InMemoryAdapter::new(McpSource::Nomifun, true);
    // Should succeed without error
    adapter.remove_server("does-not-exist").await.unwrap();
}

#[tokio::test]
async fn not_installed_errors() {
    let adapter = InMemoryAdapter::new(McpSource::Codex, false);

    assert!(!adapter.is_installed().await.unwrap());

    let err = adapter.detect_existing().await.unwrap_err();
    assert!(matches!(err, McpError::AgentNotInstalled(_)));

    let transport = McpServerTransport::Stdio {
        command: "x".into(),
        args: vec![],
        env: HashMap::new(),
    };
    let err = adapter.install_server("s", &transport).await.unwrap_err();
    assert!(matches!(err, McpError::AgentNotInstalled(_)));

    let err = adapter.remove_server("s").await.unwrap_err();
    assert!(matches!(err, McpError::AgentNotInstalled(_)));
}

#[tokio::test]
async fn multiple_adapters_independent() {
    let claude: Arc<dyn McpAgentAdapter> = Arc::new(InMemoryAdapter::new(McpSource::Claude, true));
    let gemini: Arc<dyn McpAgentAdapter> = Arc::new(InMemoryAdapter::new(McpSource::Gemini, true));

    let transport = McpServerTransport::Stdio {
        command: "npx".into(),
        args: vec![],
        env: HashMap::new(),
    };

    claude.install_server("shared-server", &transport).await.unwrap();

    // Claude has the server, Gemini does not
    assert_eq!(claude.detect_existing().await.unwrap().len(), 1);
    assert!(gemini.detect_existing().await.unwrap().is_empty());
}
