//! Integration tests for McpAgentAdapter trait and DetectedServer.
//!
//! Uses a mock adapter to verify the trait's public API contract:
//! object safety, detection lifecycle, and error cases.

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

    fn with_servers(self, servers: Vec<DetectedServer>) -> Self {
        *self.servers.lock().unwrap() = servers;
        self
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
}

fn detected(name: &str, transport: McpServerTransport) -> DetectedServer {
    DetectedServer {
        name: name.to_owned(),
        transport,
        importable: true,
        import_skip_reason: None,
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
async fn detect_returns_configured_servers() {
    let t1 = McpServerTransport::Stdio {
        command: "npx".into(),
        args: vec!["-y".into(), "server-a".into()],
        env: HashMap::new(),
    };
    let t2 = McpServerTransport::Http {
        url: "https://example.com/mcp".into(),
        headers: HashMap::from([("Auth".into(), "Bearer x".into())]),
    };
    let adapter = InMemoryAdapter::new(McpSource::Gemini, true)
        .with_servers(vec![detected("server-a", t1), detected("server-b", t2)]);

    let detected = adapter.detect_existing().await.unwrap();
    assert_eq!(detected.len(), 2);

    let names: Vec<&str> = detected.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"server-a"));
    assert!(names.contains(&"server-b"));
}

#[tokio::test]
async fn not_installed_errors() {
    let adapter = InMemoryAdapter::new(McpSource::Codex, false);

    assert!(!adapter.is_installed().await.unwrap());

    let err = adapter.detect_existing().await.unwrap_err();
    assert!(matches!(err, McpError::AgentNotInstalled(_)));
}

#[tokio::test]
async fn multiple_adapters_independent() {
    let transport = McpServerTransport::Stdio {
        command: "npx".into(),
        args: vec![],
        env: HashMap::new(),
    };
    let claude: Arc<dyn McpAgentAdapter> = Arc::new(
        InMemoryAdapter::new(McpSource::Claude, true).with_servers(vec![detected("shared-server", transport)]),
    );
    let gemini: Arc<dyn McpAgentAdapter> = Arc::new(InMemoryAdapter::new(McpSource::Gemini, true));

    // Claude has the server, Gemini does not
    assert_eq!(claude.detect_existing().await.unwrap().len(), 1);
    assert!(gemini.detect_existing().await.unwrap().is_empty());
}
