//! Integration tests for read-only Agent MCP config discovery.

use std::collections::HashMap;
use std::sync::Arc;

use nomifun_common::McpSource;
use nomifun_db::SqliteMcpServerRepository;
use nomifun_mcp::{DetectedServer, McpAgentAdapter, McpError, McpServerTransport, McpSyncService};

struct MockAdapter {
    source: McpSource,
    installed: bool,
    servers: std::sync::Mutex<Vec<DetectedServer>>,
}

impl MockAdapter {
    fn new(source: McpSource, installed: bool) -> Self {
        Self {
            source,
            installed,
            servers: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_servers(source: McpSource, servers: Vec<DetectedServer>) -> Self {
        Self {
            source,
            installed: true,
            servers: std::sync::Mutex::new(servers),
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
        Ok(self.servers.lock().unwrap().clone())
    }

    async fn install_server(&self, _name: &str, _transport: &McpServerTransport) -> Result<(), McpError> {
        unreachable!("write-to-CLI is no longer supported")
    }

    async fn remove_server(&self, _name: &str) -> Result<(), McpError> {
        unreachable!("write-to-CLI is no longer supported")
    }
}

async fn make_service(adapters: Vec<Arc<dyn McpAgentAdapter>>) -> McpSyncService {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let repo: Arc<dyn nomifun_db::IMcpServerRepository> = Arc::new(SqliteMcpServerRepository::new(db.pool().clone()));
    McpSyncService::new(repo, adapters)
}

fn stdio_transport() -> McpServerTransport {
    McpServerTransport::Stdio {
        command: "npx".into(),
        args: vec!["-y".into(), "@test/server".into()],
        env: HashMap::new(),
    }
}

#[tokio::test]
async fn get_agent_configs_returns_installed_agents() {
    let adapter_claude = Arc::new(MockAdapter::with_servers(
        McpSource::Claude,
        vec![DetectedServer {
            name: "existing-srv".into(),
            transport: stdio_transport(),
            importable: true,
            import_skip_reason: None,
        }],
    ));
    let adapter_gemini = Arc::new(MockAdapter::new(McpSource::Gemini, false));
    let adapter_qwen = Arc::new(MockAdapter::new(McpSource::Qwen, true));

    let sync_svc = make_service(vec![
        adapter_claude as Arc<dyn McpAgentAdapter>,
        adapter_gemini,
        adapter_qwen,
    ])
    .await;
    let configs = sync_svc.get_agent_configs().await.unwrap();

    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].source, McpSource::Claude);
    assert_eq!(configs[0].servers.len(), 1);
    assert_eq!(configs[0].servers[0].name, "existing-srv");
    assert_eq!(configs[1].source, McpSource::Qwen);
    assert!(configs[1].servers.is_empty());
}

#[tokio::test]
async fn get_agent_configs_empty_when_none_installed() {
    let adapter = Arc::new(MockAdapter::new(McpSource::Claude, false));
    let sync_svc = make_service(vec![adapter as Arc<dyn McpAgentAdapter>]).await;

    let configs = sync_svc.get_agent_configs().await.unwrap();
    assert!(configs.is_empty());
}
