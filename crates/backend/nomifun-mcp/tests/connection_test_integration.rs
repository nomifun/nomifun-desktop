//! Integration tests for McpConnectionTestService.
//!
//! Tests from test-plan §2 (Connection Test):
//! - CT-3: Command not found (ENOENT)
//! - CT-4: URL not reachable
//! - CT-5: Needs OAuth authentication (401)
//! - CT-6: Timeout
//! - SSE auth probe (M-33 coverage)

use std::collections::HashMap;
use std::time::Duration;

use nomifun_mcp::McpConnectionTestService;
use nomifun_mcp::McpServerTransport;

fn make_service() -> McpConnectionTestService {
    McpConnectionTestService::new(reqwest::Client::new())
}

fn make_service_with_timeout(timeout: Duration) -> McpConnectionTestService {
    McpConnectionTestService::new(reqwest::Client::new()).with_timeout(timeout)
}

// ---------------------------------------------------------------------------
// CT-3: Command not found (ENOENT)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stdio_nonexistent_command_returns_not_found_error() {
    let svc = make_service();
    let transport = McpServerTransport::Stdio {
        command: "nonexistent-mcp-cmd-xyz-12345".into(),
        args: vec![],
        env: HashMap::new(),
    };

    let result = svc.test_connection("test-server", &transport).await;

    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    assert!(
        error.contains("Command not found"),
        "expected 'Command not found' in: {error}"
    );
    assert!(result.tools.is_none());
    assert!(result.needs_auth.is_none());
}

// ---------------------------------------------------------------------------
// CT-4: URL not reachable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_unreachable_url_returns_connection_error() {
    let svc = make_service_with_timeout(Duration::from_secs(5));
    let transport = McpServerTransport::Http {
        url: "http://127.0.0.1:1/mcp-unreachable".into(),
        headers: HashMap::new(),
    };

    let result = svc.test_connection("test-http", &transport).await;

    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    assert!(
        error.contains("Connection failed"),
        "expected connection failure in: {error}"
    );
}

#[tokio::test]
async fn sse_unreachable_url_returns_connection_error() {
    let svc = make_service_with_timeout(Duration::from_secs(5));
    let transport = McpServerTransport::Sse {
        url: "http://127.0.0.1:1/sse-unreachable".into(),
        headers: HashMap::new(),
    };

    let result = svc.test_connection("test-sse", &transport).await;

    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    assert!(
        error.contains("Connection failed"),
        "expected connection failure in: {error}"
    );
}

// ---------------------------------------------------------------------------
// CT-5: HTTP 401 Unauthorized -> needsAuth
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_401_returns_needs_auth() {
    // Spin up a mock server that returns 401 with WWW-Authenticate
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/mcp",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    [(axum::http::header::WWW_AUTHENTICATE, "Bearer realm=\"mcp-server\"")],
                    "",
                )
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let svc = make_service();
    let transport = McpServerTransport::Http {
        url: format!("http://{}/mcp", addr),
        headers: HashMap::new(),
    };

    let result = svc.test_connection("auth-server", &transport).await;

    assert!(!result.success);
    assert_eq!(result.needs_auth, Some(true));
    assert!(result.auth_method.is_some());
    assert!(result.www_authenticate.is_some());
    assert!(result.error.is_none());

    server_handle.abort();
}

#[tokio::test]
async fn sse_401_returns_needs_auth() {
    // Spin up a mock server that returns 401 for GET
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/sse",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    [(axum::http::header::WWW_AUTHENTICATE, "Bearer realm=\"mcp-sse\"")],
                    "",
                )
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let svc = make_service();
    let transport = McpServerTransport::Sse {
        url: format!("http://{}/sse", addr),
        headers: HashMap::new(),
    };

    let result = svc.test_connection("sse-auth", &transport).await;

    assert!(!result.success);
    assert_eq!(result.needs_auth, Some(true));
    assert!(result.www_authenticate.is_some());

    server_handle.abort();
}

// ---------------------------------------------------------------------------
// CT-6: Timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stdio_timeout_returns_timeout_error() {
    // Use `sleep` which produces no stdout — our protocol read will block
    let svc = make_service_with_timeout(Duration::from_secs(1));
    let transport = McpServerTransport::Stdio {
        command: "sleep".into(),
        args: vec!["60".into()],
        env: HashMap::new(),
    };

    let result = svc.test_connection("timeout-server", &transport).await;

    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    assert!(error.contains("timed out"), "expected timeout in: {error}");
}

// ---------------------------------------------------------------------------
// HTTP non-success status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_500_returns_error_with_status() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/mcp",
            axum::routing::post(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let svc = make_service();
    let transport = McpServerTransport::Http {
        url: format!("http://{}/mcp", addr),
        headers: HashMap::new(),
    };

    let result = svc.test_connection("error-server", &transport).await;

    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    assert!(error.contains("500"), "expected HTTP 500 in: {error}");

    server_handle.abort();
}

// ---------------------------------------------------------------------------
// HTTP transport with custom headers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_custom_headers_are_sent() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/mcp",
            axum::routing::post(|headers: axum::http::HeaderMap| async move {
                // Verify the custom header was received
                if headers.get("x-api-key").and_then(|v| v.to_str().ok()) == Some("secret") {
                    // Return a valid initialize response
                    axum::Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "serverInfo": { "name": "test", "version": "1.0" }
                        }
                    }))
                } else {
                    // Return error if header missing
                    axum::Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "error": { "code": -1, "message": "Missing API key" }
                    }))
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let svc = make_service();
    let mut headers = HashMap::new();
    headers.insert("X-Api-Key".into(), "secret".into());
    let transport = McpServerTransport::Http {
        url: format!("http://{}/mcp", addr),
        headers,
    };

    let result = svc.test_connection("header-server", &transport).await;

    // The server returns a valid initialize response for request id=1,
    // but the subsequent tools/list (id=2) will also hit the same handler.
    // Either way, the first request should succeed (no initialize error).
    // The tools/list might succeed or fail depending on how the mock handles id=2.
    // For this test, we just verify the custom header was sent (no "Missing API key" error).
    if let Some(ref error) = result.error {
        assert!(
            !error.contains("Missing API key"),
            "Custom header should have been sent"
        );
    }

    server_handle.abort();
}

// ---------------------------------------------------------------------------
// Stdio with args and env
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stdio_with_args_spawns_correctly() {
    // Use echo as a simple command that exits immediately
    // Since echo doesn't speak MCP, we expect a protocol error (not a spawn error)
    let svc = make_service_with_timeout(Duration::from_secs(3));
    let transport = McpServerTransport::Stdio {
        command: "echo".into(),
        args: vec!["hello".into()],
        env: HashMap::new(),
    };

    let result = svc.test_connection("echo-server", &transport).await;

    // echo outputs "hello\n" then exits — not valid JSON-RPC
    assert!(!result.success);
    let error = result.error.as_deref().unwrap();
    // Should be a protocol error, not a spawn error
    assert!(!error.contains("Command not found"), "echo should be found");
}
