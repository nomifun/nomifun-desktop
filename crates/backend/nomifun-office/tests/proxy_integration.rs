use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nomifun_api_types::WebSocketMessage;
use nomifun_office::OfficeError;
use nomifun_office::proxy::{ProxyError, ProxyService};
use nomifun_office::types::DocType;
use nomifun_office::watch_manager::{OfficecliWatchManager, ProcessHandle, ProcessSpawner};
use nomifun_realtime::EventBroadcaster;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

struct MockProcessHandle {
    alive: AtomicBool,
}

impl MockProcessHandle {
    fn new() -> Self {
        Self {
            alive: AtomicBool::new(true),
        }
    }
}

impl ProcessHandle for MockProcessHandle {
    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

struct HttpMockSpawner {
    response_template: String,
}

#[async_trait::async_trait]
impl ProcessSpawner for HttpMockSpawner {
    async fn spawn_officecli(
        &self,
        _file_path: &str,
        port: u16,
        _doc_type: DocType,
    ) -> Result<Box<dyn ProcessHandle>, OfficeError> {
        let resp = self.response_template.replace("__PORT__", &port.to_string());
        tokio::spawn(async move {
            let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
            for _ in 0..10 {
                if let Ok((mut stream, _)) = listener.accept().await {
                    let resp = resp.clone();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 4096];
                        let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Ok(Box::new(MockProcessHandle::new()))
    }

    async fn install_officecli(&self) -> Result<(), OfficeError> {
        Ok(())
    }

    async fn is_officecli_installed(&self) -> bool {
        true
    }

    async fn check_update(&self, _doc_type: DocType) -> Result<(), OfficeError> {
        Ok(())
    }
}

struct TcpOnlySpawner;

#[async_trait::async_trait]
impl ProcessSpawner for TcpOnlySpawner {
    async fn spawn_officecli(
        &self,
        _file_path: &str,
        port: u16,
        _doc_type: DocType,
    ) -> Result<Box<dyn ProcessHandle>, OfficeError> {
        let listener = std::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .map_err(|e| OfficeError::StartFailed(e.to_string()))?;
        std::mem::forget(listener);
        Ok(Box::new(MockProcessHandle::new()))
    }

    async fn install_officecli(&self) -> Result<(), OfficeError> {
        Ok(())
    }

    async fn is_officecli_installed(&self) -> bool {
        true
    }

    async fn check_update(&self, _doc_type: DocType) -> Result<(), OfficeError> {
        Ok(())
    }
}

struct NoopBroadcaster;

impl EventBroadcaster for NoopBroadcaster {
    fn broadcast(&self, _event: WebSocketMessage<serde_json::Value>) {}
}

fn build_http_response(status: u16, headers: &[(&str, &str)], body: &str) -> String {
    let status_text = match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        _ => "Unknown",
    };

    let mut resp = format!("HTTP/1.1 {status} {status_text}\r\n");
    for (k, v) in headers {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    if !headers.iter().any(|(k, _)| k.to_lowercase() == "content-length") {
        resp.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    resp
}

async fn setup_proxy(doc_type: DocType, response_template: &str) -> (ProxyService, u16, tempfile::TempDir) {
    let spawner = HttpMockSpawner {
        response_template: response_template.to_owned(),
    };
    let mgr = Arc::new(OfficecliWatchManager::new(Arc::new(spawner), Arc::new(NoopBroadcaster)));

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.docx");
    std::fs::write(&file, b"test").unwrap();

    let port = mgr.start(file.to_str().unwrap(), doc_type).await.unwrap();
    let proxy = ProxyService::new(mgr);

    (proxy, port, dir)
}

async fn setup_ssrf_proxy(doc_type: DocType) -> (ProxyService, u16, tempfile::TempDir) {
    let mgr = Arc::new(OfficecliWatchManager::new(
        Arc::new(TcpOnlySpawner),
        Arc::new(NoopBroadcaster),
    ));

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.docx");
    std::fs::write(&file, b"test").unwrap();

    let port = mgr.start(file.to_str().unwrap(), doc_type).await.unwrap();
    let proxy = ProxyService::new(mgr);

    (proxy, port, dir)
}

// ---------------------------------------------------------------------------
// RP-2: PPT proxy SSRF protection — inactive port rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp2_ppt_proxy_ssrf_rejects_inactive_port() {
    let (proxy, _active_port, _dir) = setup_ssrf_proxy(DocType::Ppt).await;

    let result = proxy.forward(9999, "/index.html", DocType::Ppt, &[]).await;

    let err = result.unwrap_err();
    assert!(matches!(err, ProxyError::PortNotActive(9999)));
}

// ---------------------------------------------------------------------------
// RP-4: Office watch proxy SSRF protection — inactive port rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp4_office_watch_proxy_ssrf_rejects_inactive_port() {
    let (proxy, _active_port, _dir) = setup_ssrf_proxy(DocType::Word).await;

    let result = proxy.forward_watch(9999, "/", &[]).await;

    assert!(matches!(result.unwrap_err(), ProxyError::PortNotActive(9999)));
}

// ---------------------------------------------------------------------------
// SSRF: wrong doc_type rejected even when port is active
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ssrf_wrong_doc_type_rejected() {
    let (proxy, active_port, _dir) = setup_ssrf_proxy(DocType::Word).await;

    let result = proxy.forward(active_port, "/index.html", DocType::Ppt, &[]).await;

    assert!(matches!(result.unwrap_err(), ProxyError::PortNotActive(_)));
}

// ---------------------------------------------------------------------------
// H-1-13.8 fix: forward_watch accepts Excel session ports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forward_watch_accepts_excel_session_port() {
    let response = build_http_response(200, &[("Content-Type", "text/plain")], "Excel preview");
    let (proxy, port, _dir) = setup_proxy(DocType::Excel, &response).await;

    let result = proxy.forward_watch(port, "/", &[]).await.unwrap();

    assert_eq!(result.status, 200);
    let body = String::from_utf8(result.body).unwrap();
    assert!(body.contains("Excel preview"));
}

// ---------------------------------------------------------------------------
// forward_watch accepts Word session ports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forward_watch_accepts_word_session_port() {
    let response = build_http_response(200, &[("Content-Type", "text/plain")], "Word preview");
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response).await;

    let result = proxy.forward_watch(port, "/", &[]).await.unwrap();

    assert_eq!(result.status, 200);
    let body = String::from_utf8(result.body).unwrap();
    assert!(body.contains("Word preview"));
}

// ---------------------------------------------------------------------------
// forward_watch rejects PPT session ports
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forward_watch_rejects_ppt_session_port() {
    let (proxy, ppt_port, _dir) = setup_ssrf_proxy(DocType::Ppt).await;

    let result = proxy.forward_watch(ppt_port, "/", &[]).await;

    assert!(matches!(result.unwrap_err(), ProxyError::PortNotActive(_)));
}

// ---------------------------------------------------------------------------
// RP-1 / RP-3: Proxy forwards plain text response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp1_rp3_proxy_forwards_plain_text() {
    let response = build_http_response(200, &[("Content-Type", "text/plain")], "Hello from preview");
    let (proxy, port, _dir) = setup_proxy(DocType::Ppt, &response).await;

    let result = proxy.forward(port, "/index.html", DocType::Ppt, &[]).await.unwrap();

    assert_eq!(result.status, 200);
    let body = String::from_utf8(result.body).unwrap();
    assert!(body.contains("Hello from preview"));
}

// ---------------------------------------------------------------------------
// RP-5: HTML injection — navigation guard script injected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp5_proxy_injects_navigation_guard_in_html() {
    let html_body = "<html><head><title>Preview</title></head><body>Content</body></html>";
    let response = build_http_response(200, &[("Content-Type", "text/html")], html_body);
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response).await;

    let result = proxy.forward(port, "/", DocType::Word, &[]).await.unwrap();

    assert_eq!(result.status, 200);
    let body = String::from_utf8(result.body).unwrap();
    assert!(body.contains("<script>"), "should inject navigation guard script");
    assert!(
        body.contains(&format!("'/api/office-watch-proxy/{port}'")),
        "guard should reference correct proxy base path"
    );
    assert!(
        body.contains("<title>Preview</title>"),
        "should preserve original HTML content"
    );
}

// ---------------------------------------------------------------------------
// RP-5b: Non-HTML response should NOT inject script
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp5b_proxy_does_not_inject_in_json() {
    let response = build_http_response(200, &[("Content-Type", "application/json")], r#"{"ok":true}"#);
    let (proxy, port, _dir) = setup_proxy(DocType::Ppt, &response).await;

    let result = proxy.forward(port, "/api/data", DocType::Ppt, &[]).await.unwrap();

    let body = String::from_utf8(result.body).unwrap();
    assert!(!body.contains("<script>"), "should not inject script in JSON responses");
}

// ---------------------------------------------------------------------------
// RP-7: Hop-by-hop header stripping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp7_proxy_strips_hop_by_hop_headers() {
    let response = build_http_response(
        200,
        &[
            ("Content-Type", "text/plain"),
            ("Connection", "keep-alive"),
            ("Keep-Alive", "timeout=5"),
            ("X-Custom", "preserved"),
        ],
        "body",
    );
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response).await;

    let result = proxy.forward(port, "/", DocType::Word, &[]).await.unwrap();

    let header_names: Vec<&str> = result.headers.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        !header_names.contains(&"connection"),
        "connection header should be stripped"
    );
    assert!(
        !header_names.contains(&"keep-alive"),
        "keep-alive header should be stripped"
    );
    assert!(header_names.contains(&"x-custom"), "custom header should be preserved");
}

// ---------------------------------------------------------------------------
// X-Frame-Options set to SAMEORIGIN
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_sets_x_frame_options_sameorigin() {
    let response = build_http_response(200, &[("Content-Type", "text/plain")], "body");
    let (proxy, port, _dir) = setup_proxy(DocType::Ppt, &response).await;

    let result = proxy.forward(port, "/", DocType::Ppt, &[]).await.unwrap();

    let xfo = result
        .headers
        .iter()
        .find(|(k, _)| k == "x-frame-options")
        .map(|(_, v)| v.as_str());
    assert_eq!(xfo, Some("SAMEORIGIN"));
}

// ---------------------------------------------------------------------------
// HTML content-length stripped after injection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_removes_content_length_for_html() {
    let html_body = "<html><head></head><body></body></html>";
    let response = build_http_response(200, &[("Content-Type", "text/html")], html_body);
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response).await;

    let result = proxy.forward(port, "/", DocType::Word, &[]).await.unwrap();

    let has_cl = result.headers.iter().any(|(k, _)| k == "content-length");
    assert!(
        !has_cl,
        "content-length should be stripped for HTML responses (body size changed after injection)"
    );
}

// ---------------------------------------------------------------------------
// Non-HTML content-length preserved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_preserves_content_length_for_non_html() {
    let response = build_http_response(200, &[("Content-Type", "application/json")], r#"{"ok":true}"#);
    let (proxy, port, _dir) = setup_proxy(DocType::Ppt, &response).await;

    let result = proxy.forward(port, "/api/data", DocType::Ppt, &[]).await.unwrap();

    let has_cl = result.headers.iter().any(|(k, _)| k == "content-length");
    assert!(has_cl, "content-length should be preserved for non-HTML");
}

// ---------------------------------------------------------------------------
// RP-6: Location rewriting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp6_proxy_rewrites_location_header() {
    let response_template = build_http_response(
        302,
        &[
            ("Content-Type", "text/html"),
            ("Location", "http://localhost:__PORT__/new/path"),
        ],
        "",
    );
    let (proxy, port, _dir) = setup_proxy(DocType::Ppt, &response_template).await;

    let result = proxy.forward(port, "/old", DocType::Ppt, &[]).await.unwrap();

    assert_eq!(result.status, 302);
    let location = result
        .headers
        .iter()
        .find(|(k, _)| k == "location")
        .map(|(_, v)| v.as_str());
    assert_eq!(location, Some(format!("/api/ppt-proxy/{port}/new/path").as_str()));
}

// ---------------------------------------------------------------------------
// RP-6b: Location rewriting for root-relative paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rp6b_proxy_rewrites_root_relative_location() {
    let response_template = build_http_response(302, &[("Content-Type", "text/html"), ("Location", "/redirected")], "");
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response_template).await;

    let result = proxy.forward(port, "/old", DocType::Word, &[]).await.unwrap();

    let location = result
        .headers
        .iter()
        .find(|(k, _)| k == "location")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        location,
        Some(format!("/api/office-watch-proxy/{port}/redirected").as_str())
    );
}

// ---------------------------------------------------------------------------
// Proxy forwards 404 status correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_forwards_404_status() {
    let response = build_http_response(404, &[("Content-Type", "text/plain")], "Not Found");
    let (proxy, port, _dir) = setup_proxy(DocType::Word, &response).await;

    let result = proxy.forward(port, "/missing", DocType::Word, &[]).await.unwrap();

    assert_eq!(result.status, 404);
}
