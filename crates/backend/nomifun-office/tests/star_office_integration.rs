use std::net::TcpListener;
use std::sync::Arc;

use nomifun_office::StarOfficeDetector;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn allocate_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn mock_star_office_server(
    port: u16,
    health_ok: bool,
    status_body: &'static str,
    index_body: &'static str,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();

    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };
            let status_body = status_body.to_string();
            let index_body = index_body.to_string();

            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = match stream.read(&mut buf).await {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let request = String::from_utf8_lossy(&buf[..n]);

                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");

                let (status_line, body) = match path {
                    "/health" => {
                        if health_ok {
                            ("HTTP/1.1 200 OK", "ok".to_string())
                        } else {
                            ("HTTP/1.1 503 Service Unavailable", "down".to_string())
                        }
                    }
                    "/status" => ("HTTP/1.1 200 OK", status_body),
                    "/" => ("HTTP/1.1 200 OK", index_body),
                    _ => ("HTTP/1.1 404 Not Found", "not found".to_string()),
                };

                let response = format!(
                    "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    })
}

// ---------------------------------------------------------------------------
// SO-1: No available service → returns None
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so1_no_service_returns_none() {
    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let port = allocate_port();
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(50)).await;
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// SO-2: With preferred URL, no service → returns None
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so2_preferred_url_no_service() {
    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let result = detector
        .detect_exact(Some("http://localhost:59990"), true, Some(50))
        .await;
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// SO-3: Cache behavior — second call hits cache
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so3_cache_hit_returns_cached() {
    let port = allocate_port();
    let url = format!("http://localhost:{port}");
    let detector = Arc::new(StarOfficeDetector::new(reqwest::Client::new()));

    let _ = detector.detect_exact(Some(&url), false, Some(50)).await;

    let t0 = tokio::time::Instant::now();
    let result = detector.detect_exact(Some(&url), false, Some(50)).await;
    let elapsed = t0.elapsed();

    assert!(result.is_none());
    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "cached call should be fast, took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// SO-4: Force ignores cache
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so4_force_bypasses_cache() {
    let port = allocate_port();
    let url = format!("http://localhost:{port}");
    let detector = StarOfficeDetector::new(reqwest::Client::new());

    let _ = detector.detect_exact(Some(&url), false, Some(50)).await;
    let result = detector.detect_exact(Some(&url), true, Some(50)).await;

    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// SO-5: Detect available service (three-step health check)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so5_detect_available_service() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        true,
        r#"{"status": "idle"}"#,
        "<html><head></head><body>Star Office dashboard with decorate room</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert_eq!(result, Some(format!("http://localhost:{port}")));

    handle.abort();
}

// ---------------------------------------------------------------------------
// SO-6: Exclude OpenClaw misidentification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn so6_exclude_openclaw() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        true,
        r#"{"status": "idle"}"#,
        "<html><head></head><body>Star Office with openclaw control panel</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert!(result.is_none());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Health check step 1 fail: /health returns non-200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_step1_fail_returns_none() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        false,
        r#"{"status": "idle"}"#,
        "<html><body>Star Office</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert!(result.is_none());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Health check step 2 fail: /status has no status markers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_step2_no_status_markers() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        true,
        "just some random text",
        "<html><body>Star Office</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert!(result.is_none());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Health check step 3 fail: / has no feature keywords
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_step3_no_feature_keywords() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        true,
        r#"{"status": "idle"}"#,
        "<html><body>Some other application</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert!(result.is_none());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Detect with different status markers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn detect_with_writing_status() {
    let port = allocate_port();
    let handle = mock_star_office_server(
        port,
        true,
        r#"{"status": "writing"}"#,
        "<html><body>decorate room and asset sidebar</body></html>",
    )
    .await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;

    assert_eq!(result, Some(format!("http://localhost:{port}")));

    handle.abort();
}

// ---------------------------------------------------------------------------
// Cache stores hit correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_stores_hit_after_success() {
    let port = allocate_port();
    let handle =
        mock_star_office_server(port, true, r#"idle"#, "<html><body>star office dashboard</body></html>").await;

    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let url = format!("http://localhost:{port}");
    let result = detector.detect_exact(Some(&url), true, Some(2000)).await;
    assert!(result.is_some());

    handle.abort();

    let cached = detector.detect_exact(Some(&url), false, Some(50)).await;
    assert_eq!(cached, result);
}

// ---------------------------------------------------------------------------
// Cache miss TTL expires quickly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_miss_ttl_expires() {
    let detector = StarOfficeDetector::new(reqwest::Client::new());
    let port = allocate_port();
    let url = format!("http://localhost:{port}");

    let _ = detector.detect_exact(Some(&url), false, Some(50)).await;

    tokio::time::sleep(std::time::Duration::from_millis(1600)).await;

    let port2 = allocate_port();
    let handle = mock_star_office_server(port2, true, "idle", "<html><body>star office</body></html>").await;

    let url2 = format!("http://localhost:{port2}");
    let result = detector.detect_exact(Some(&url2), false, Some(2000)).await;
    assert_eq!(result, Some(format!("http://localhost:{port2}")));

    handle.abort();
}
