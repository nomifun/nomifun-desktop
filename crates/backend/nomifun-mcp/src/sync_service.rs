use std::sync::Arc;

use nomifun_api_types::{DetectedMcpServerEntry, DetectedMcpServerResponse};
use tokio::sync::Mutex;
use tracing::warn;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::adapters::cli_helpers::normalize_detection_status;
use crate::error::McpError;

/// Discovers MCP configuration currently installed in external Agent CLIs.
///
/// This service is intentionally read-only. It serializes detection work to
/// avoid concurrent CLI scans from spawning overlapping child processes.
#[derive(Clone)]
pub struct McpSyncService {
    adapters: Arc<Vec<Arc<dyn McpAgentAdapter>>>,
    service_lock: Arc<Mutex<()>>,
}

impl McpSyncService {
    pub fn new(adapters: Vec<Arc<dyn McpAgentAdapter>>) -> Self {
        Self {
            adapters: Arc::new(adapters),
            service_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Scan all installed Agent CLIs and return each one's current MCP
    /// server configurations.
    ///
    /// Agents that are not installed are silently skipped.
    pub async fn get_agent_configs(&self) -> Result<Vec<DetectedMcpServerResponse>, McpError> {
        let _guard = self.service_lock.lock().await;

        let mut results = Vec::new();
        for adapter in self.adapters.iter() {
            let installed = match adapter.is_installed().await {
                Ok(installed) => installed,
                Err(error) => {
                    results.push(DetectedMcpServerResponse {
                        source: adapter.source(), servers: vec![],
                        detection_error: Some(error.to_string()),
                    });
                    continue;
                }
            };
            if !installed {
                continue;
            }

            match adapter.detect_existing().await {
                Ok(detected) => {
                    let servers = detected.into_iter().map(detected_to_response).collect();
                    results.push(DetectedMcpServerResponse {
                        source: adapter.source(),
                        servers,
                        detection_error: None,
                    });
                }
                Err(e) => {
                    warn!(
                        agent = ?adapter.source(),
                        error = %e,
                        "failed to detect existing MCP servers"
                    );
                    results.push(DetectedMcpServerResponse {
                        source: adapter.source(), servers: vec![],
                        detection_error: Some(e.to_string()),
                    });
                }
            }
        }

        Ok(results)
    }
}

fn detected_to_response(detected: DetectedServer) -> DetectedMcpServerEntry {
    let normalized_skip_reason = detected.import_skip_reason.as_deref().map(normalize_detection_status);
    let importable = detected.importable || normalized_skip_reason.as_deref() == Some("Connected");

    DetectedMcpServerEntry {
        name: detected.name,
        description: None,
        transport: detected.transport.into(),
        original_json: None,
        importable,
        import_skip_reason: if importable { None } else { normalized_skip_reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::McpServerTransport;
    use nomifun_common::McpSource;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    struct MockAdapter {
        source: McpSource,
        installed: bool,
        servers: Arc<StdMutex<Vec<DetectedServer>>>,
        failed: bool,
    }

    impl MockAdapter {
        fn new(source: McpSource, installed: bool) -> Self {
            Self {
                source,
                installed,
                servers: Arc::new(StdMutex::new(Vec::new())),
                failed: false,
            }
        }

        fn with_existing(mut self, servers: Vec<DetectedServer>) -> Self {
            self.servers = Arc::new(StdMutex::new(servers));
            self
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
            if self.failed {
                return Err(McpError::AgentOperationFailed("unsupported CLI capability".into()));
            }
            if !self.installed {
                return Err(McpError::AgentNotInstalled(format!("{:?}", self.source)));
            }
            Ok(self.servers.lock().unwrap().clone())
        }
    }

    fn stdio_transport() -> McpServerTransport {
        McpServerTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@test/server".into()],
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn failed_scan_remains_visible_alongside_successful_empty_catalog() {
        let mut failed = MockAdapter::new(McpSource::Claude, true);
        failed.failed = true;
        let service = McpSyncService::new(vec![Arc::new(failed), Arc::new(MockAdapter::new(McpSource::Qwen, true))]);
        let results = service.get_agent_configs().await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].detection_error.is_some());
        assert!(results[1].detection_error.is_none());
    }

    #[tokio::test]
    async fn get_agent_configs_returns_installed_only() {
        let adapter_a = Arc::new(
            MockAdapter::new(McpSource::Claude, true).with_existing(vec![DetectedServer {
                name: "srv1".into(),
                transport: stdio_transport(),
                importable: true,
                import_skip_reason: None,
            }]),
        );
        let adapter_b = Arc::new(MockAdapter::new(McpSource::Gemini, false));
        let adapter_c = Arc::new(MockAdapter::new(McpSource::Qwen, true).with_existing(vec![]));

        let svc = McpSyncService::new(vec![adapter_a, adapter_b, adapter_c]);
        let configs = svc.get_agent_configs().await.unwrap();

        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].source, McpSource::Claude);
        assert_eq!(configs[0].servers.len(), 1);
        assert_eq!(configs[0].servers[0].name, "srv1");
        assert_eq!(configs[1].source, McpSource::Qwen);
        assert!(configs[1].servers.is_empty());
    }

    #[test]
    fn detected_to_response_normalizes_connected_skip_reason() {
        let resp = detected_to_response(DetectedServer {
            name: "sentry".into(),
            transport: stdio_transport(),
            importable: false,
            import_skip_reason: Some("✓ Connected".into()),
        });

        assert!(resp.importable);
        assert_eq!(resp.import_skip_reason, None);
    }

    #[tokio::test]
    async fn get_agent_configs_no_adapters() {
        let svc = McpSyncService::new(vec![]);
        let configs = svc.get_agent_configs().await.unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn detected_to_response_fields() {
        let detected = DetectedServer {
            name: "my-srv".into(),
            transport: stdio_transport(),
            importable: false,
            import_skip_reason: Some("Needs authentication".into()),
        };
        let resp = detected_to_response(detected);
        assert_eq!(resp.name, "my-srv");
        assert!(!resp.importable);
        assert_eq!(resp.import_skip_reason.as_deref(), Some("Needs authentication"));
    }
}
