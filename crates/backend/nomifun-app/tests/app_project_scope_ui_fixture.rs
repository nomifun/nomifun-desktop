//! Isolated production API and desktop WebUI acceptance for application Session ownership.
//! No model input is submitted to Runtime; history uses canonical accepted/cancelled inputs.
//! The optional GUI fixture serves the existing `ui/dist` and requires explicit finish.
//! Run with `cargo test --locked -p nomifun-app --test app_project_scope_ui_fixture
//! --no-default-features --features browser-use -- --test-threads=1`.

use std::{panic::AssertUnwindSafe, path::Path, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
    routing::post,
};
use nomifun_api_types::ConversationResponse;
use futures_util::FutureExt;
use nomifun_app::{
    AuthPolicy,
    bootstrap::{NomiCoreApplication, ServerEnvironment},
};
use serde_json::{Value, json};
use sqlx::{SqlitePool, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};
use tempfile::TempDir;
use tokio::sync::watch;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

// Public disposable test values, only used with a newly created TempDir.
const LOCAL_TRUST: &str = "app-project-scope-isolated-fixture";
const WEB_USERNAME: &str = "app-project-scope-qa";
const WEB_PASSWORD: &str = "NomiFunScopeQA2026!";
const MODEL: &str = "app-scope-qa-never-generated";
const SEARCH_MARKER: &str = "SCOPEQA";

struct Fixture {
    environment: ServerEnvironment,
    application: NomiCoreApplication,
    seed_pool: SqlitePool,
    temporary: TempDir,
}

impl Fixture {
    async fn build() -> Self {
        let temporary = tempfile::Builder::new().prefix("nomifun-app-scope-qa-").tempdir().unwrap();
        for relative in ["data", "work", "logs"] {
            std::fs::create_dir(temporary.path().join(relative)).unwrap();
        }
        let cli = nomifun_app::cli::Cli {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: temporary.path().join("data"),
            work_dir: Some(temporary.path().join("work")),
            app_version: env!("CARGO_PKG_VERSION").into(),
            local: true,
            log_dir: Some(temporary.path().join("logs")),
            log_level: Some("off".into()),
            command: None,
        };
        let mut environment = nomifun_app::bootstrap::init_nomi_core_environment(&cli, "").await.unwrap();
        environment.config.auth_policy = AuthPolicy::TrustLocalToken;
        environment.config.local_trust_secret = Some(Arc::from(LOCAL_TRUST));
        let application = NomiCoreApplication::compose(&environment).await.unwrap();
        let seed_pool = SqlitePoolOptions::new().max_connections(1).connect_with(
            SqliteConnectOptions::new().filename(environment.config.database_path())
                .create_if_missing(false).foreign_keys(true),
        ).await.unwrap();
        Self { environment, application, seed_pool, temporary }
    }

    async fn close(self) {
        self.seed_pool.close().await;
        self.application.close().await.expect("production Host close must succeed");
        drop(self.environment);
        self.temporary.close().expect("only the isolated fixture directory is reclaimed");
    }
}

async fn request(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", LOCAL_TRUST).header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let response = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"non_json": String::from_utf8_lossy(&bytes)}));
    (status, response)
}

async fn data(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let (status, response) = request(router, method, path, body).await;
    assert!(status.is_success(), "{method} {path}: {status}: {response}");
    assert_eq!(response["success"], true, "{method} {path}: {response}");
    response["data"].clone()
}

#[derive(Clone)]
struct AppSeed {
    id: String,
    directory: String,
    sessions: Vec<String>,
}

struct Seed {
    apps: Vec<AppSeed>,
    ordinary: String,
    model: Value,
}

async fn cancelled_history(fixture: &Fixture, owner: &str, session: &str, content: &str) {
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(fixture.seed_pool.clone()).await.unwrap();
    let owner = nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: owner.into() };
    let session = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    canonical.start_turn(&owner, &session, "scope-qa-accepted-input", json!({"content": content})).await.unwrap();
    canonical.cancel(&owner, &session, "scope-qa-cancelled-before-runtime").await.unwrap();
}

async fn seed(fixture: &Fixture) -> Seed {
    let router = fixture.application.router();
    let (status, user) = request(&router, "GET", "/api/auth/user", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let owner = user["user"]["user_id"].as_str().unwrap();
    let (status, setup) = request(&router, "POST", "/api/auth/setup", json!({
        "username": WEB_USERNAME, "password": WEB_PASSWORD,
    })).await;
    assert_eq!(status, StatusCode::OK, "{setup}");
    assert_eq!(setup["user"]["user_id"], owner, "login setup preserves the installation owner");

    // .invalid is reserved and contains no real provider key. Neither seeding
    // nor acceptance submits a generation request; this model only compiles bindings.
    let provider = data(&router, "POST", "/api/providers", json!({
        "platform": "openai", "name": "Isolated application scope QA",
        "base_url": "https://app-scope-qa.invalid/v1", "auth_scheme": "bearer",
        "credentials": {"api_keys": ["public-disposable-fixture-key"]},
        "enabled": true, "sort_order": 0,
        "initial_model": {"model": MODEL, "enabled": true, "sort_order": 0,
            "capabilities": [{"task": "chat", "traits": [],
                "protocol": "openai.responses", "connection_role": "default", "endpoint": "/responses",
                "provider_params": {}, "context_limit": 128000}]},
        "connections": [],
    })).await;
    let model = json!({"provider_id": provider["provider_id"], "model": MODEL});
    let mut apps = Vec::new();
    for (label, name) in [("A", "验收应用甲"), ("B", "验收应用乙")] {
        let app = data(&router, "POST", "/api/apps", json!({"name": name})).await;
        let id = app["id"].as_str().unwrap().to_owned();
        let directory = app["directory"].as_str().unwrap().to_owned();
        assert!(Path::new(&directory).canonicalize().unwrap().starts_with(
            fixture.temporary.path().canonicalize().unwrap()), "app source stays inside the fixture");
        let mut sessions = Vec::new();
        for index in 1..=2 {
            let body = if index == 1 { json!({"model": model}) } else {
                json!({"model": model, "new_session": true, "creation_key": uuid::Uuid::now_v7().to_string()})
            };
            let opened = data(&router, "POST", &format!("/api/apps/{id}/development-session"), body).await;
            let projection: ConversationResponse = serde_json::from_value(opened["conversation"].clone()).unwrap();
            assert_eq!(projection.app_project_id.as_deref(), Some(id.as_str()), "real DTO has exact application ownership");
            let session = projection.conversation_id.to_string();
            cancelled_history(fixture, owner, &session, &format!("{SEARCH_MARKER} 应用{label}专属历史{index}")).await;
            data(&router, "PATCH", &format!("/api/agent-sessions/{session}"), json!({
                "name": format!("应用{label}项目会话{index}"), "archived": label == "A" && index == 1,
            })).await;
            sessions.push(session);
        }
        apps.push(AppSeed { id, directory, sessions });
    }

    // Exact same directory as App A, with ordinary creation provenance. Paths
    // alone do not grant application ownership or application deletion authority.
    let preset = data(&router, "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "model": model, "reuse_existing": true, "display_name": "Ordinary scope QA",
    })).await;
    let ordinary = data(&router, "POST", "/api/agent-sessions", json!({
        "preset_id": preset["preset"]["preset_id"], "model": model,
        "title": "普通会话保留-同目录", "workspace": apps[0].directory, "resource_selections": [],
    })).await;
    let ordinary = ordinary["agent_session_id"].as_str().unwrap().to_owned();
    cancelled_history(fixture, owner, &ordinary, &format!("{SEARCH_MARKER} 普通会话历史保留")).await;
    let projection = data(&router, "GET", &format!("/api/agent-sessions/{ordinary}/projection"), Value::Null).await;
    let projection: ConversationResponse = serde_json::from_value(projection).unwrap();
    assert!(projection.app_project_id.is_none(), "same-directory ordinary Session is still ordinary");
    Seed { apps, ordinary, model }
}

async fn verify_scope(router: &Router, seed: &Seed) {
    let ordinary = data(router, "GET", "/api/agent-sessions?limit=1", Value::Null).await;
    assert_eq!(ordinary["total"], 1, "application scope is excluded before pagination");
    assert_eq!(ordinary["items"][0]["conversation_id"], seed.ordinary);
    assert_eq!(ordinary["has_more"], false);
    let search = data(router, "GET", &format!("/api/agent-session-messages/search?keyword={SEARCH_MARKER}&page_size=10"), Value::Null).await;
    assert_eq!(search["total"], 5, "global search retains current ordinary and application history");
    assert_eq!(search["items"].as_array().unwrap().len(), 5);
    assert_eq!(search["has_more"], false);
    for item in search["items"].as_array().unwrap() {
        let projection: ConversationResponse = serde_json::from_value(item["conversation"].clone()).unwrap();
        if projection.conversation_id == seed.ordinary {
            assert!(projection.app_project_id.is_none());
        } else {
            let app = seed.apps.iter().find(|app| app.sessions.contains(&projection.conversation_id)).unwrap();
            assert_eq!(projection.app_project_id.as_deref(), Some(app.id.as_str()), "global search can route application hits to their owner");
        }
    }
    for app in &seed.apps {
        let history = data(router, "GET", &format!("/api/apps/{}/development-sessions", app.id), Value::Null).await;
        assert_eq!(history["total"], 2);
        assert_eq!(history["owned_session_count"], 2, "deletion preview covers archived and unarchived scoped Sessions");
        for item in history["items"].as_array().unwrap() {
            let projection: ConversationResponse = serde_json::from_value(item["conversation"].clone()).unwrap();
            assert_eq!(projection.app_project_id.as_deref(), Some(app.id.as_str()));
            assert!(app.sessions.contains(&projection.conversation_id));
        }
    }
}

#[tokio::test]
async fn production_scope_dtos_filter_history_and_application_deletion_preserves_same_directory_ordinary_session() {
    let fixture = Fixture::build().await;
    let outcome = AssertUnwindSafe(async {
    let seed = seed(&fixture).await;
    let router = fixture.application.router();
    verify_scope(&router, &seed).await;
    let ordinary_before = data(&router, "GET", &format!("/api/agent-sessions/{}/events", seed.ordinary), Value::Null).await;
    for session in &seed.apps[0].sessions {
        let (status, response) = request(&router, "DELETE", &format!("/api/agent-sessions/{session}"), Value::Null).await;
        assert_eq!(status, StatusCode::CONFLICT, "{response}");
        assert_eq!(response["code"], "APP_PROJECT_SESSION_MANAGED_BY_APP");
    }
    data(&router, "DELETE", &format!("/api/apps/{}", seed.apps[0].id), Value::Null).await;
    for session in &seed.apps[0].sessions {
        let (status, _) = request(&router, "GET", &format!("/api/agent-sessions/{session}/projection"), Value::Null).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "application deletion retires its archived and live Sessions");
    }
    let ordinary_history = data(&router, "GET", &format!("/api/agent-sessions/{}/events", seed.ordinary), Value::Null).await;
    assert_eq!(ordinary_history, ordinary_before, "application deletion does not rewrite ordinary canonical history");
    assert!(ordinary_history.to_string().contains("普通会话历史保留"));
    assert_eq!(data(&router, "GET", &format!("/api/apps/{}/development-sessions", seed.apps[1].id), Value::Null).await["total"], 2);
    let search = data(&router, "GET", &format!("/api/agent-session-messages/search?keyword={SEARCH_MARKER}&page_size=10"), Value::Null).await;
    assert_eq!(search["total"], 3, "deleted application history leaves search while unrelated history stays");
    assert!(search["items"].as_array().unwrap().iter().all(|item|
        item["conversation"]["app_project_id"] != seed.apps[0].id));
    let ordinary = data(&router, "GET", "/api/agent-sessions?limit=1", Value::Null).await;
    assert_eq!(ordinary["total"], 1);
    assert_eq!(ordinary["items"][0]["conversation_id"], seed.ordinary);
    drop(router);
    }).catch_unwind().await;
    fixture.close().await;
    if let Err(error) = outcome { std::panic::resume_unwind(error); }
}

async fn finish(State(finished): State<watch::Sender<bool>>, axum::Json(body): axum::Json<Value>) -> StatusCode {
    if body["finished"] != true { return StatusCode::BAD_REQUEST; }
    match finished.send(true) { Ok(()) => StatusCode::NO_CONTENT, Err(_) => StatusCode::GONE }
}

/// Readiness proves the fixture's production setup, not UI acceptance. An
/// independent browser driver checks actual rendered controls, then explicitly finishes.
#[tokio::test]
#[ignore = "isolated production desktop WebUI; requires ui/dist and explicit finish"]
async fn app_project_scope_desktop_webui_fixture() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ui/dist");
    assert!(dist.join("index.html").is_file(), "build the current renderer with bun run build:ui first");
    let fixture = Fixture::build().await;
    let seeded = AssertUnwindSafe(async {
        let seed = seed(&fixture).await;
        verify_scope(&fixture.application.router(), &seed).await;
        seed
    }).catch_unwind().await;
    let seed = match seeded {
        Ok(seed) => seed,
        Err(error) => { fixture.close().await; std::panic::resume_unwind(error); }
    };
    fixture.seed_pool.close().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    fixture.application.publish_app_hosting_endpoint(address.port(), "127.0.0.1").await;
    let (finished, mut finished_rx) = watch::channel(false);
    let (stopping, mut stopping_rx) = watch::channel(false);
    let served = fixture.application.router().merge(Router::new()
        .route("/__app-project-scope-fixture/finish", post(finish)).with_state(finished))
        .fallback_service(ServeDir::new(&dist).append_index_html_on_directories(true)
            .fallback(ServeFile::new(dist.join("index.html"))));
    let server = tokio::spawn(async move {
        axum::serve(listener, served).with_graceful_shutdown(async move {
            let _ = stopping_rx.changed().await;
        }).await
    });
    let host_url = format!("http://{address}");
    let ready = json!({
        "host_url": host_url, "finish_url": format!("{host_url}/__app-project-scope-fixture/finish"),
        "username": WEB_USERNAME, "password": WEB_PASSWORD, "search_marker": SEARCH_MARKER,
        "apps": seed.apps.iter().map(|app| json!({"id": app.id, "directory": app.directory, "sessions": app.sessions})).collect::<Vec<_>>(),
        "ordinary_session_id": seed.ordinary, "model": seed.model,
        "source": "production-router-and-ui-dist", "provider_requests": "none; no Runtime input submitted", "validation": null,
    });
    eprintln!("NOMIFUN_APP_PROJECT_SCOPE_UI_READY {ready}");
    use std::io::Write as _;
    std::io::stderr().flush().unwrap();
    let explicit_finish = tokio::time::timeout(Duration::from_secs(1800), finished_rx.changed()).await;
    fixture.application.quiesce_for_shutdown().await.unwrap();
    stopping.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(30), server).await.unwrap().unwrap().unwrap();
    fixture.close().await;
    explicit_finish.expect("independent UI acceptance must explicitly finish").unwrap();
    eprintln!("NOMIFUN_APP_PROJECT_SCOPE_UI_CLOSED validation=null production_close=true");
}

/// Recover only this fixture's isolated installation after the external test
/// runner has interrupted its process before the explicit finish callback.
#[tokio::test]
#[ignore = "requires an exact interrupted isolated fixture root"]
async fn app_project_scope_close_interrupted_fixture() {
    let requested = std::path::PathBuf::from(std::env::var_os("NOMIFUN_APP_PROJECT_SCOPE_RECOVERY_ROOT")
        .expect("supply the exact interrupted fixture root"));
    assert!(requested.is_absolute());
    let root = requested.canonicalize().unwrap();
    assert_eq!(root.parent().unwrap(), std::env::temp_dir().canonicalize().unwrap());
    assert!(root.file_name().unwrap().to_str().unwrap().starts_with("nomifun-app-scope-qa-"));
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new()
        .filename(root.join("data/nomifun-backend.db")).create_if_missing(false).read_only(true)).await.unwrap();
    let usernames: Vec<String> = sqlx::query_scalar("SELECT username FROM users").fetch_all(&pool).await.unwrap();
    assert_eq!(usernames, vec![WEB_USERNAME], "never recover a real user installation");
    let providers: Vec<String> = sqlx::query_scalar("SELECT base_url FROM providers").fetch_all(&pool).await.unwrap();
    assert_eq!(providers, vec!["https://app-scope-qa.invalid/v1"], "fixture has no real provider route");
    pool.close().await;
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".into(), port: 0, data_dir: root.join("data"), work_dir: Some(root.join("work")),
        app_version: env!("CARGO_PKG_VERSION").into(), local: true, log_dir: Some(root.join("logs")),
        log_level: Some("off".into()), command: None,
    };
    let mut environment = nomifun_app::bootstrap::init_nomi_core_environment(&cli, "").await.unwrap();
    environment.config.auth_policy = AuthPolicy::TrustLocalToken;
    environment.config.local_trust_secret = Some(Arc::from(LOCAL_TRUST));
    let application = NomiCoreApplication::compose(&environment).await.unwrap();
    application.close().await.expect("recovered production Host close must succeed");
    drop(environment);
    // The canonical target was checked against the exact temporary parent,
    // fixture prefix, disposable account and non-provider route above.
    std::fs::remove_dir_all(&root).expect("reclaim only this interrupted fixture root");
    assert!(!root.exists());
    eprintln!("NOMIFUN_APP_PROJECT_SCOPE_UI_RECOVERED_CLOSED production_close=true temp_root_reclaimed=true");
}
