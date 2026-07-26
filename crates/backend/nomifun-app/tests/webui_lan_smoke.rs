//! Smoke test that drives the REAL desktop WebUI/LAN serving path end-to-end
//! (`DesktopServer::start` → `start_lan`) against a throwaway data dir, so the
//! actual failure cause of "enable WebUI" surfaces deterministically instead of
//! being guessed at. Prints the resolved status (port, LAN IP, URL, error).

use std::path::Path;

use clap::Parser as _;

fn local_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("local reqwest client should build")
}

/// Keep each backend's conversation workspace independent from the process
/// global `NOMIFUN_WORK_DIR` that another concurrently running test may set.
fn isolated_cli(data_dir: &Path) -> nomifun_app::cli::Cli {
    let work_dir = data_dir.join("work");
    nomifun_app::cli::Cli::parse_from([
        "nomifun-desktop-test".to_owned(),
        "--data-dir".to_owned(),
        data_dir.to_string_lossy().into_owned(),
        "--work-dir".to_owned(),
        work_dir.to_string_lossy().into_owned(),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_lan_start_smoke() {
    // Isolated data dir so we never touch a running instance's state / lock.
    let tmp = tempfile::TempDir::new().unwrap();
    // Isolate the data dir via --data-dir below (NOT a process-global env var):
    // these tests run in parallel, so a shared set_var("NOMIFUN_DATA_DIR") would
    // race and two backends could resolve to the SAME dir → "data directory
    // already in use by another running NomiFun backend".
    let spa_dir = tmp.path().join("spa");
    std::fs::create_dir_all(&spa_dir).unwrap();
    std::fs::write(
        spa_dir.join("index.html"),
        "<!doctype html><title>Nomi</title>",
    )
    .unwrap();

    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();

    let started =
        nomifun_app::DesktopServer::start(&cli, &merged_path, Some(spa_dir), None, None).await;
    let (server, _keep) = match started {
        Ok(pair) => pair,
        Err(e) => panic!("DesktopServer::start failed: {e:#}"),
    };

    eprintln!("== loopback_port = {}", server.loopback_port());

    let status = server.start_lan().await;
    eprintln!("== start_lan status = {status:?}");

    server.stop_lan().await;

    assert!(
        status.running,
        "start_lan did NOT run — error = {:?}",
        status.error
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_lan_spa_deep_link_serves_app_shell() {
    let tmp = tempfile::TempDir::new().unwrap();
    let spa_dir = tmp.path().join("spa");
    std::fs::create_dir_all(&spa_dir).unwrap();
    std::fs::write(
        spa_dir.join("index.html"),
        "<!doctype html><title>Nomi deep link</title>",
    )
    .unwrap();

    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();

    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, Some(spa_dir), None, None)
            .await
            .expect("DesktopServer::start failed");

    let status = server.start_lan().await;
    assert!(status.running, "start_lan failed: {:?}", status.error);

    let response = local_http_client()
        .get(format!(
            "http://127.0.0.1:{}/open-capabilities",
            status.port
        ))
        .send()
        .await
        .expect("request to LAN listener failed");
    let response_status = response.status();
    let body = response.text().await.unwrap_or_default();

    server.stop_lan().await;

    assert_eq!(response_status, reqwest::StatusCode::OK);
    assert!(
        body.contains("Nomi deep link"),
        "SPA deep link should serve index.html, got status={response_status}; body={body}"
    );
}

/// HTTP-layer regression coverage for custom-protocol builds, which can have an
/// embedded frontend source but no external webui-dist directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_lan_embedded_asset_source_needs_no_filesystem_shell() {
    const INDEX: &str = "<!doctype html><title>EMBEDDED_WEBUI_MARKER</title>";
    const SCRIPT: &str = "window.__embeddedWebUi = true;";

    let tmp = tempfile::TempDir::new().unwrap();
    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();
    let assets = nomifun_app::WebUiAssetSource::new([
        (
            "index.html",
            nomifun_app::WebUiAsset::new(INDEX.as_bytes().to_vec(), "text/html")
                .with_csp_header(Some("default-src 'self'".to_string())),
        ),
        (
            "assets/app.js",
            nomifun_app::WebUiAsset::new(SCRIPT.as_bytes().to_vec(), "text/javascript"),
        ),
    ]);

    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, None, None, Some(assets))
            .await
            .expect("DesktopServer::start failed");
    let status = server.start_lan().await;
    assert!(status.running, "start_lan failed: {:?}", status.error);

    let client = local_http_client();
    let base = format!("http://127.0.0.1:{}", status.port);
    let route = client
        .get(format!("{base}/open-capabilities"))
        .send()
        .await
        .expect("request embedded SPA route");
    assert_eq!(route.status(), reqwest::StatusCode::OK);
    assert_eq!(
        route
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html")
    );
    assert_eq!(
        route
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
    assert_eq!(
        route
            .headers()
            .get(reqwest::header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok()),
        Some("default-src 'self'")
    );
    assert_eq!(
        route
            .headers()
            .get(reqwest::header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert!(
        route
            .text()
            .await
            .unwrap_or_default()
            .contains("EMBEDDED_WEBUI_MARKER")
    );

    let script = client
        .get(format!("{base}/assets/app.js"))
        .send()
        .await
        .expect("request embedded JS");
    assert_eq!(script.status(), reqwest::StatusCode::OK);
    assert_eq!(
        script
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/javascript")
    );
    assert_eq!(
        script
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(script.text().await.unwrap_or_default(), SCRIPT);

    let head = client
        .head(format!("{base}/open-capabilities"))
        .send()
        .await
        .expect("HEAD embedded SPA route");
    assert_eq!(head.status(), reqwest::StatusCode::OK);
    let head_length = head
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    assert_eq!(head_length, Some(INDEX.len()));
    assert!(head.bytes().await.unwrap_or_default().is_empty());

    let method = client
        .post(format!("{base}/open-capabilities"))
        .send()
        .await
        .expect("POST embedded SPA route");
    assert_eq!(method.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        method
            .headers()
            .get(reqwest::header::ALLOW)
            .and_then(|value| value.to_str().ok()),
        Some("GET, HEAD")
    );

    server.stop_lan().await;
}

/// A LAN listener with only `/qr-login` + `/api/auth/qr-login` but no SPA shell
/// reproduces the phone symptom: the QR page can say "Login successful", then
/// the browser navigates to `/` and receives an HTTP failure. Refuse that
/// partial state at start-up so the desktop UI reports a real WebUI start error
/// instead of handing users a broken QR flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_lan_without_app_shell_fails_instead_of_serving_partial_qr_flow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();

    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, None, None, None)
            .await
            .expect("DesktopServer::start failed");

    let status = server.start_lan().await;

    assert!(
        !status.running,
        "LAN WebUI must not start without an app shell"
    );
    assert!(
        status
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("WebUI app shell"),
        "missing app shell error should be actionable, got {:?}",
        status.error
    );
}

/// Regression guard for the dev bug "saved figure image is broken + desktop
/// companion renders blank". Native `<img>` / `new Image()` loads (figure
/// thumbnails, the companion mesh texture) cannot present the local-trust
/// header, so under `TrustLocalToken` the figure-image GET MUST be auth-exempt —
/// while listing/creation stay authenticated. Boots the real desktop backend and
/// hits its loopback port with NO trust header, exactly like a native image load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn figure_image_get_is_public_but_listing_stays_authenticated() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Isolate the data dir via --data-dir below (NOT a process-global env var):
    // these tests run in parallel, so a shared set_var("NOMIFUN_DATA_DIR") would
    // race and two backends could resolve to the SAME dir → "data directory
    // already in use by another running NomiFun backend".
    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();
    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, None, None, None)
            .await
            .expect("DesktopServer::start failed");

    let base = format!("http://127.0.0.1:{}", server.loopback_port());
    let client = local_http_client();

    // Figure-image GET with NO trust header (what a native <img> sends): must NOT
    // be auth-rejected, AND must not 500. Under `TrustLocalToken` an untrusted
    // request gets no injected `CurrentUser`, so the handler must not depend on
    // that extension — an unknown id yields 404, a real one 200, never 401/403/500.
    let img = client
        .get(format!(
            "{base}/api/companion/figures/figure_nonexistent/image"
        ))
        .send()
        .await
        .expect("figure image request failed");
    assert_eq!(
        img.status(),
        reqwest::StatusCode::NOT_FOUND,
        "figure-image GET for an unknown id must be a clean 404 (auth-exempt, no \
         CurrentUser dependency); got {} — a 401/403 means the route is still \
         authenticated, a 500 means the handler still extracts Extension<CurrentUser>",
        img.status()
    );

    // The figures listing must STILL require auth — no trust header → rejected.
    let list = client
        .get(format!("{base}/api/companion/figures"))
        .send()
        .await
        .expect("figures listing request failed");
    assert!(
        list.status() == reqwest::StatusCode::UNAUTHORIZED
            || list.status() == reqwest::StatusCode::FORBIDDEN,
        "figures listing must stay authenticated, got {}",
        list.status()
    );
}

/// In DEV the LAN listener must serve the SAME live frontend the desktop webview
/// loads — proxied to the vite dev server — NOT a stale bundled `ui/dist`. This
/// stands up a mock "vite" server, points `DesktopServer` at it, enables LAN,
/// and asserts a request to the LAN port is proxied through to the live content.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_lan_dev_proxy_serves_live_frontend() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Isolate the data dir via --data-dir below (NOT a process-global env var):
    // these tests run in parallel, so a shared set_var("NOMIFUN_DATA_DIR") would
    // race and two backends could resolve to the SAME dir → "data directory
    // already in use by another running NomiFun backend".
    // Mock vite dev server: returns a recognizable marker for any path.
    let mock = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mock_port = mock.local_addr().unwrap().port();
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(|| async { "LIVE_VITE_INDEX_MARKER" });
        let _ = axum::serve(mock, app).await;
    });

    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();
    let dev_url = format!("http://127.0.0.1:{mock_port}");
    let stale_embedded = nomifun_app::WebUiAssetSource::new([(
        "index.html",
        nomifun_app::WebUiAsset::new(b"STALE_EMBEDDED_MARKER".to_vec(), "text/html"),
    )]);

    let (server, _keep) = nomifun_app::DesktopServer::start(
        &cli,
        &merged_path,
        None,
        Some(dev_url),
        Some(stale_embedded),
    )
    .await
    .expect("DesktopServer::start failed");

    let status = server.start_lan().await;
    assert!(status.running, "start_lan failed: {:?}", status.error);

    // A request to the LAN listener's SPA path must be proxied to the mock vite.
    let url = format!("http://127.0.0.1:{}/some/spa/route", status.port);
    let response = local_http_client()
        .get(&url)
        .send()
        .await
        .expect("request to LAN listener failed");
    let response_status = response.status();
    let body = response.text().await.unwrap_or_default();
    eprintln!("== proxied status = {response_status}; body = {body}");
    assert!(
        body.contains("LIVE_VITE_INDEX_MARKER"),
        "LAN listener did not proxy to the dev frontend; status={response_status}; got: {body}"
    );
    assert!(
        !body.contains("STALE_EMBEDDED_MARKER"),
        "dev proxy must take precedence over stale embedded assets"
    );

    server.stop_lan().await;
}

/// Regression for the credential-persistence bug: a username the user set while
/// WebUI was OFF (password still empty) must NOT be reset to "admin" when the
/// LAN server is first enabled, and `status_snapshot` must report the persisted
/// username + `password_set` even before/without the LAN listener running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_enable_preserves_user_set_username_and_reports_persisted_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let spa_dir = tmp.path().join("spa");
    std::fs::create_dir_all(&spa_dir).unwrap();
    std::fs::write(spa_dir.join("index.html"), "<!doctype html><title>Nomi</title>").unwrap();

    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();

    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, Some(spa_dir), None, None)
            .await
            .expect("DesktopServer::start failed");

    // Simulate the user renaming the admin from the panel while WebUI is OFF:
    // this leaves `password_hash` empty. Open the SAME db file the server uses
    // (WAL allows a second connection) and rewrite the username directly.
    let db_path = tmp.path().join("nomifun-backend.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db_path.display()))
        .await
        .expect("open backend db");
    let affected = sqlx::query(
        "UPDATE users SET username = 'custom_admin' \
         WHERE user_id = (SELECT owner_user_id FROM installation_identity \
                          WHERE singleton_key = 'installation')",
    )
        .execute(&pool)
        .await
        .expect("rename system admin")
        .rows_affected();
    assert_eq!(affected, 1, "should have renamed the installation owner");
    pool.close().await;

    // Before enabling: snapshot reflects the persisted username and "no password yet".
    let before = server.status_snapshot().await;
    assert_eq!(before.admin_username, "custom_admin", "persisted username must surface while stopped");
    assert!(!before.password_set, "no password provisioned yet");

    // Enable LAN: this provisions a password but MUST NOT clobber the username.
    let started = server.start_lan().await;
    assert!(started.running, "start_lan failed: {:?}", started.error);
    assert_eq!(started.admin_username, "custom_admin", "username must be preserved, not reset to 'admin'");
    assert!(started.password_set, "a password must exist once LAN is exposed");
    assert!(
        started.initial_password.is_some(),
        "first enable should surface the one-time initial password"
    );

    // "Restart" equivalent: a fresh snapshot still shows the custom username and
    // that a password is set (the credential is durable, not perceived as lost).
    server.stop_lan().await;
    let after = server.status_snapshot().await;
    assert_eq!(after.admin_username, "custom_admin");
    assert!(after.password_set, "password must remain set after stopping the LAN listener");
}

/// Browser login contract regression: the desktop-generated credentials must
/// authenticate through the real LAN listener with the bundled WebUI schema.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webui_browser_login_contract_accepts_generated_credentials() {
    let tmp = tempfile::TempDir::new().unwrap();
    let spa_dir = tmp.path().join("spa");
    std::fs::create_dir_all(&spa_dir).unwrap();
    std::fs::write(spa_dir.join("index.html"), "<!doctype html><title>Nomi</title>").unwrap();

    let cli = isolated_cli(tmp.path());
    let merged_path = std::env::var("PATH").unwrap_or_default();
    let (server, _keep) =
        nomifun_app::DesktopServer::start(&cli, &merged_path, Some(spa_dir), None, None)
            .await
            .expect("DesktopServer::start failed");

    let status = server.start_lan().await;
    assert!(status.running, "start_lan failed: {:?}", status.error);
    let password = status
        .initial_password
        .as_deref()
        .expect("first LAN enable should expose the generated password once");
    let login_url = format!("http://127.0.0.1:{}/login", status.port);

    let response = local_http_client()
        .post(login_url)
        .json(&serde_json::json!({
            "username": status.admin_username,
            "password": password,
        }))
        .send()
        .await
        .expect("browser login request failed");
    let response_status = response.status();
    let response_body: serde_json::Value = response
        .json()
        .await
        .expect("login response should be JSON");

    server.stop_lan().await;

    assert_eq!(
        response_status,
        reqwest::StatusCode::OK,
        "generated credentials should authenticate through LAN: {response_body}"
    );
    assert_eq!(response_body["success"], true);
    assert_eq!(response_body["user"]["username"], status.admin_username);
    assert!(
        response_body["token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "successful login should return a session token"
    );
}
