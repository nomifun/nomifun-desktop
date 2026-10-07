mod protocol;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nomifun_api_types::{McpConnectionTestErrorCode, McpConnectionTestResult};
use tracing::{debug, warn};

use crate::types::McpServerTransport;
use protocol::{error_result, spawn_error_result, success_result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const PACKAGE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// McpConnectionTestService
// ---------------------------------------------------------------------------

/// Service for testing MCP server connectivity.
///
/// Creates a temporary MCP client, performs the protocol handshake
/// (initialize -> initialized -> tools/list), and returns the tool list
/// or an error.  Supports stdio, HTTP (Streamable HTTP), and SSE transports.
#[derive(Clone)]
pub struct McpConnectionTestService {
    http_client: HttpClientFactory,
    timeout: Duration,
    package_bootstrap_timeout: Duration,
}

type HttpClientFactory =
    Arc<dyn Fn() -> Result<reqwest::Client, McpConnectionTestResult> + Send + Sync>;

impl McpConnectionTestService {
    /// Injected clients must disable redirects, just like the execution owner.
    pub fn new(http_client: reqwest::Client) -> Self {
        Self {
            http_client: Arc::new(move || Ok(http_client.clone())),
            timeout: CONNECTION_TIMEOUT,
            package_bootstrap_timeout: PACKAGE_BOOTSTRAP_TIMEOUT,
        }
    }

    pub fn new_dynamic() -> Self {
        Self {
            http_client: Arc::new(|| {
                nomifun_net::http_client_no_redirect().map_err(|_| {
                    error_result(
                        McpConnectionTestErrorCode::ConnectionFailed,
                        "MCP discovery client could not be initialized with redirects disabled"
                            .into(),
                        None,
                    )
                })
            }),
            timeout: CONNECTION_TIMEOUT,
            package_bootstrap_timeout: PACKAGE_BOOTSTRAP_TIMEOUT,
        }
    }

    fn http_client(&self) -> Result<reqwest::Client, McpConnectionTestResult> {
        (self.http_client)()
    }

    /// Override the protocol timeout (default: 30s). Known sessions/processes
    /// still receive a separate bounded cleanup phase after protocol timeout.
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self {
            timeout,
            package_bootstrap_timeout: timeout,
            ..self
        }
    }

    /// Test connectivity to an MCP server.
    ///
    /// Dispatches to the appropriate transport handler.  Always returns
    /// a result (never errors) -- failures are encoded in the struct.
    pub async fn test_connection(
        &self,
        name: &str,
        transport: &McpServerTransport,
    ) -> McpConnectionTestResult {
        debug!(
            name,
            transport = transport.transport_type(),
            "starting MCP connection test"
        );
        let result = match transport {
            McpServerTransport::Stdio { command, args, env } => {
                self.test_stdio(command, args, env).await
            }
            McpServerTransport::Http { url, headers } => {
                self.test_network(url, headers, false).await
            }
            McpServerTransport::Sse { url, headers } => self.test_network(url, headers, true).await,
        };
        if !result.success && result.needs_auth != Some(true) {
            let code = result.code.map(McpConnectionTestErrorCode::as_str);
            let failure_kind = result
                .details
                .as_ref()
                .and_then(|value| value.get("failure_kind"))
                .and_then(serde_json::Value::as_str);
            let endpoint_scope = result
                .details
                .as_ref()
                .and_then(|value| value.get("endpoint_scope"))
                .and_then(serde_json::Value::as_str);
            warn!(
                name,
                transport = transport.transport_type(),
                code,
                failure_kind,
                endpoint_scope,
                "MCP connection test failed"
            );
        }
        result
    }

    // -- Stdio transport --------------------------------------------------

    async fn test_stdio(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> McpConnectionTestResult {
        let is_package_runner = protocol::is_package_runner(command, args);
        let timeout = if is_package_runner {
            self.package_bootstrap_timeout
        } else {
            self.timeout
        };
        match crate::owner::discover_stdio(command, args, env, timeout).await {
            Ok(catalog) => success_result(Some(catalog)),
            Err(error) => {
                let kind = match error.code() {
                    "MCP_COMMAND_NOT_FOUND" => Some(std::io::ErrorKind::NotFound),
                    "MCP_COMMAND_PERMISSION_DENIED" => Some(std::io::ErrorKind::PermissionDenied),
                    "MCP_PROCESS_START_FAILED" => Some(std::io::ErrorKind::Other),
                    _ => None,
                };
                if let Some(kind) = kind {
                    return spawn_error_result(
                        command,
                        &std::io::Error::new(kind, error.message().to_owned()),
                    );
                }
                let (code, failure_kind) = match error.code() {
                    "MCP_TIMEOUT" => (
                        McpConnectionTestErrorCode::Timeout,
                        Some(if is_package_runner {
                            "package_bootstrap_timeout"
                        } else {
                            "handshake_timeout"
                        }),
                    ),
                    "MCP_PACKAGE_NOT_FOUND" => (
                        McpConnectionTestErrorCode::CommandStartFailed,
                        Some("package_not_found"),
                    ),
                    "MCP_PACKAGE_DOWNLOAD_FAILED" => (
                        McpConnectionTestErrorCode::CommandStartFailed,
                        Some("package_download_failed"),
                    ),
                    "MCP_PROCESS_DEPENDENCY_MISSING" => (
                        McpConnectionTestErrorCode::CommandStartFailed,
                        Some("dependency_missing"),
                    ),
                    "MCP_PROCESS_CONFIGURATION_REQUIRED" => (
                        McpConnectionTestErrorCode::CommandStartFailed,
                        Some("configuration_required"),
                    ),
                    "MCP_PROCESS_EXITED" | "MCP_CONNECTION_FAILED" => (
                        McpConnectionTestErrorCode::CommandStartFailed,
                        Some("process_exited"),
                    ),
                    _ => (McpConnectionTestErrorCode::ProtocolError, None),
                };
                let mut details = serde_json::json!({
                    "transport": "stdio",
                    "owner_code": error.code(),
                    "command": command,
                    "runtime": protocol::command_runtime(command),
                });
                if let Some(failure_kind) = failure_kind {
                    details["failure_kind"] = serde_json::json!(failure_kind);
                }
                if code == McpConnectionTestErrorCode::Timeout {
                    details["timeout_seconds"] = serde_json::json!(timeout.as_secs());
                    details["phase"] = serde_json::json!(if is_package_runner {
                        "package_bootstrap"
                    } else {
                        "handshake"
                    });
                }
                error_result(
                    code,
                    error.message().to_owned(),
                    Some(details),
                )
            }
        }
    }

    // HTTP and legacy SSE use the same bounded owner as actual execution.
    async fn test_network(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        legacy: bool,
    ) -> McpConnectionTestResult {
        let client = match self.http_client() {
            Ok(client) => client,
            Err(result) => return result,
        };
        match crate::owner::discover_network(client, url, headers, legacy, self.timeout).await {
            Ok(catalog) => success_result(Some(catalog)),
            Err(error) => {
                if error.code() == "MCP_CREDENTIAL_REQUIRED" {
                    let mut challenge_headers = reqwest::header::HeaderMap::new();
                    if let Some(challenge) = error.authentication_challenge() {
                        if let Ok(value) = reqwest::header::HeaderValue::from_str(challenge) {
                            challenge_headers.insert("www-authenticate", value);
                        }
                    }
                    return protocol::auth_result(&challenge_headers);
                }
                if error.code() == "MCP_TIMEOUT" {
                    let mut result = protocol::timeout_result(self.timeout);
                    add_endpoint_details(&mut result, url);
                    return result;
                }
                let code = match error.code() {
                    "MCP_CONNECTION_FAILED" | "MCP_HTTP_CLIENT_UNAVAILABLE" => {
                        McpConnectionTestErrorCode::ConnectionFailed
                    }
                    "MCP_HTTP_ERROR" => McpConnectionTestErrorCode::HttpError,
                    "MCP_RPC_ERROR" => McpConnectionTestErrorCode::RpcError,
                    _ => McpConnectionTestErrorCode::ProtocolError,
                };
                let mut details = serde_json::json!({
                    "transport": if legacy { "sse" } else { "http" },
                    "owner_code": error.code()
                });
                if let Some(status) = error.http_status() {
                    details["status"] = serde_json::json!(status);
                }
                add_endpoint_detail_fields(&mut details, url);
                error_result(code, error.message().to_owned(), Some(details))
            }
        }
    }
}

fn add_endpoint_details(result: &mut McpConnectionTestResult, url: &str) {
    if let Some(details) = result.details.as_mut() {
        add_endpoint_detail_fields(details, url);
    }
}

fn add_endpoint_detail_fields(details: &mut serde_json::Value, url: &str) {
    let Ok(endpoint) = reqwest::Url::parse(url) else {
        return;
    };
    let Some(host) = endpoint.host_str() else {
        return;
    };
    let local = host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    details["endpoint_scope"] = serde_json::json!(if local { "local" } else { "remote" });
    if local {
        details["host"] = serde_json::json!(host);
        if let Some(port) = endpoint.port_or_known_default() {
            details["port"] = serde_json::json!(port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_clone() {
        let svc = McpConnectionTestService::new(reqwest::Client::new());
        let _cloned = svc.clone();
    }

    #[test]
    fn service_with_timeout() {
        let svc = McpConnectionTestService::new(reqwest::Client::new())
            .with_timeout(Duration::from_secs(5));
        assert_eq!(svc.timeout, Duration::from_secs(5));
        assert_eq!(svc.package_bootstrap_timeout, Duration::from_secs(5));
    }

    #[test]
    fn package_runner_gets_a_first_run_bootstrap_budget() {
        let svc = McpConnectionTestService::new(reqwest::Client::new());
        assert_eq!(svc.timeout, Duration::from_secs(30));
        assert_eq!(svc.package_bootstrap_timeout, Duration::from_secs(120));
    }

    #[test]
    fn endpoint_details_only_expose_local_host_and_port() {
        let mut local = serde_json::json!({});
        add_endpoint_detail_fields(&mut local, "http://localhost:18060/mcp?token=secret");
        assert_eq!(local["endpoint_scope"], "local");
        assert_eq!(local["host"], "localhost");
        assert_eq!(local["port"], 18060);
        assert!(local.get("url").is_none());

        let mut remote = serde_json::json!({});
        add_endpoint_detail_fields(&mut remote, "https://secret.example.test/mcp?token=secret");
        assert_eq!(remote, serde_json::json!({ "endpoint_scope": "remote" }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_timeout_cleans_up_process_group() {
        let marker_path = std::env::temp_dir().join(format!(
            "nomifun-mcp-timeout-pid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let transport = McpServerTransport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                "printf '%s\n' \"$$\" > \"$1\"; sleep 30".into(),
                "mcp-timeout-child".into(),
                marker_path.to_string_lossy().into_owned(),
            ],
            env: HashMap::new(),
        };
        let svc = McpConnectionTestService::new(reqwest::Client::new())
            .with_timeout(Duration::from_millis(100));

        let result = svc.test_connection("timeout-cleanup", &transport).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("timed out"),
            "expected timeout result, got {result:?}"
        );

        let pid: i32 = std::fs::read_to_string(&marker_path)
            .expect("stdio child should write its pid")
            .trim()
            .parse()
            .expect("pid marker should be numeric");

        let group_alive = wait_for_process_group_exit(pid, Duration::from_secs(1)).await;
        if group_alive {
            let _ = kill_process_group(pid, libc_sigkill());
        }
        let _ = std::fs::remove_file(marker_path);

        assert!(
            !group_alive,
            "stdio timeout should terminate the spawned process group for pid={pid}"
        );
    }

    #[cfg(unix)]
    async fn wait_for_process_group_exit(pid: i32, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if !is_process_group_alive(pid) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        is_process_group_alive(pid)
    }

    #[cfg(unix)]
    fn is_process_group_alive(pid: i32) -> bool {
        kill_process_group(pid, 0)
    }

    #[cfg(unix)]
    fn kill_process_group(pid: i32, signal: i32) -> bool {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe { kill(-pid, signal) == 0 }
    }

    #[cfg(unix)]
    fn libc_sigkill() -> i32 {
        9
    }
}
