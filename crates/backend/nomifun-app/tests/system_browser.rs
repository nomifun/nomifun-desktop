#![cfg(feature = "browser-use")]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use nomi_browser_engine::attached_browser::{AttachError, UserTabChoice, UserTabInventory};
use nomifun_ai_agent::{
    AgentRuntimeControl, AgentRuntimeHandle, AgentRuntimeRegistry, MockAgentRuntime,
};
use nomifun_app::system_browser::{
    SystemBrowserConnection, SystemBrowserConnectionFactory, SystemBrowserService,
    SystemBrowserState, SystemBrowserTab,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tower::ServiceExt;

struct RunningRuntime;
#[async_trait::async_trait]
impl AgentRuntimeControl for RunningRuntime {
    fn agent_type(&self) -> nomifun_common::AgentType {
        nomifun_common::AgentType::Nomi
    }
    fn conversation_id(&self) -> &str {
        "0190f5fe-7c00-7a00-8000-000000000099"
    }
    fn workspace(&self) -> &str {
        ""
    }
    fn status(&self) -> Option<nomifun_common::ConversationStatus> {
        Some(nomifun_common::ConversationStatus::Running)
    }
    fn is_transport_healthy(&self) -> bool {
        true
    }
    fn last_activity_at(&self) -> nomifun_common::TimestampMs {
        nomifun_common::now_ms()
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent> {
        tokio::sync::broadcast::channel(1).1
    }
    async fn send_message(
        &self,
        _: nomifun_ai_agent::types::SendMessageData,
    ) -> Result<(), nomifun_ai_agent::AgentSendError> {
        panic!("fixture must not start an Agent")
    }
    async fn cancel(&self) -> Result<(), nomifun_common::AppError> {
        panic!("browser reconfiguration must not stop the running Agent")
    }
    fn kill(
        &self,
        _: Option<nomifun_common::AgentKillReason>,
    ) -> Result<(), nomifun_common::AppError> {
        panic!("browser reconfiguration must not kill the running Agent")
    }
}
#[async_trait::async_trait]
impl MockAgentRuntime for RunningRuntime {}

struct RuntimeRegistry {
    running: AtomicBool,
    provider: Mutex<Option<Arc<dyn nomifun_ai_agent::NomiPluginToolSessionProvider>>>,
}
#[async_trait::async_trait]
impl AgentRuntimeRegistry for RuntimeRegistry {
    fn install_nomi_plugin_tool_session_provider(
        &self,
        provider: Arc<dyn nomifun_ai_agent::NomiPluginToolSessionProvider>,
    ) -> Result<(), nomifun_common::AppError> {
        *self.provider.lock().unwrap() = Some(provider);
        Ok(())
    }
    fn get_runtime(&self, id: &str) -> Option<AgentRuntimeHandle> {
        (self.running.load(Ordering::SeqCst) && id == RunningRuntime.conversation_id())
            .then(|| AgentRuntimeHandle::Mock(Arc::new(RunningRuntime)))
    }
    async fn get_or_create_runtime(
        &self,
        _: &str,
        _: nomifun_ai_agent::types::AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, nomifun_common::AppError> {
        panic!("no model needed")
    }
    fn terminate(
        &self,
        _: &str,
        _: Option<nomifun_common::AgentKillReason>,
    ) -> Result<(), nomifun_common::AppError> {
        assert!(!self.running.load(Ordering::SeqCst));
        Ok(())
    }
    fn terminate_and_wait_result(
        &self,
        _: &str,
        _: Option<nomifun_common::AgentKillReason>,
    ) -> futures_util::future::BoxFuture<'static, Result<(), nomifun_common::AppError>> {
        assert!(
            !self.running.load(Ordering::SeqCst),
            "running Agent must be rejected before retirement"
        );
        Box::pin(async { Ok(()) })
    }
    fn terminate_all(&self) {}
    fn active_runtime_count(&self) -> usize {
        usize::from(self.running.load(Ordering::SeqCst))
    }
}

struct FixtureConnection {
    connected: AtomicBool,
    grants: AtomicUsize,
    closes: AtomicUsize,
    fail_close: AtomicBool,
}
#[async_trait::async_trait]
impl SystemBrowserConnection for FixtureConnection {
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
    fn request_disconnect(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }
    async fn choices(&self) -> Result<UserTabInventory, AttachError> {
        Ok(UserTabInventory {
            tabs: vec![UserTabChoice {
                choice_id: "fixture-choice".into(),
                title: "User chooser only".into(),
                url: "https://fixture.test".into(),
            }],
        })
    }
    async fn grant(&self, choice: &str) -> Result<SystemBrowserTab, AttachError> {
        if choice != "fixture-choice" {
            return Err(AttachError::StaleSelection);
        }
        self.grants.fetch_add(1, Ordering::SeqCst);
        Ok(SystemBrowserTab {
            tab_id: "fixture-grant".into(),
            title: "Authorized tab".into(),
            url: "https://fixture.test".into(),
        })
    }
    async fn disconnect(&self) -> Result<(), AttachError> {
        self.connected.store(false, Ordering::SeqCst);
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.fail_close.swap(false, Ordering::SeqCst) {
            return Err(AttachError::ConnectionFailed);
        }
        Ok(())
    }
}
struct FixtureFactory {
    calls: AtomicUsize,
    gate: tokio::sync::Semaphore,
    connections: Mutex<Vec<Arc<FixtureConnection>>>,
}
#[async_trait::async_trait]
impl SystemBrowserConnectionFactory for FixtureFactory {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        let connection = Arc::new(FixtureConnection {
            connected: AtomicBool::new(true),
            grants: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            fail_close: AtomicBool::new(false),
        });
        self.connections.lock().unwrap().push(connection.clone());
        Ok(connection)
    }
}
async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn request(method: &str, path: &str, trusted: bool, value: Value) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if trusted {
        request = request.header("x-nomi-local-trust", "system-browser-api-fixture");
    }
    request.body(Body::from(value.to_string())).unwrap()
}

#[tokio::test]
async fn system_browser_http_is_local_owner_scoped_and_never_connects_on_read_or_rejected_requests()
{
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = nomifun_app::compatibility::AppServices::from_config(
        database,
        &nomifun_app::AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from("system-browser-api-fixture")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let factory = Arc::new(FixtureFactory {
        calls: AtomicUsize::new(0),
        gate: tokio::sync::Semaphore::new(0),
        connections: Mutex::new(Vec::new()),
    });
    let system = SystemBrowserService::with_factory(factory.clone());
    services.system_browser = Some(system.clone());
    let runtimes = Arc::new(RuntimeRegistry {
        running: AtomicBool::new(false),
        provider: Mutex::new(None),
    });
    services = services.with_agent_runtime_registry(runtimes.clone());
    let id = "0190f5fe-7c00-7a00-8000-000000000099";
    let foreign = "0190f5fe-7c00-7a00-8000-000000000098";
    for (conversation, owner) in [
        (id, services.authoritative_user_id.as_ref()),
        (foreign, "0190f5fe-7c00-7a00-8000-000000000097"),
    ] {
        nomifun_db::sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,status,created_at,updated_at) VALUES (?,?,'system browser fixture','nomi','{}','pending',1,1)")
            .bind(conversation).bind(owner).execute(services.database.pool()).await.unwrap();
    }
    let router = nomifun_app::compatibility::create_router(&services).await;
    let base = format!("/api/conversations/{id}/system-browser");
    let response = router
        .clone()
        .oneshot(request("GET", &base, true, json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body["data"].is_null());
    assert!(
        services
            .system_browser
            .as_ref()
            .unwrap()
            .snapshot(services.authoritative_user_id.as_ref(), id)
            .is_none()
    );

    for (method, suffix, body) in [
        ("GET", "", json!({})),
        ("POST", "", json!({})),
        ("DELETE", "", json!({"incarnation":"unknown"})),
        ("POST", "/choices", json!({"incarnation":"unknown"})),
        (
            "POST",
            "/tabs",
            json!({"incarnation":"unknown","choice_id":"unknown"}),
        ),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, &format!("{base}{suffix}"), false, body))
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/conversations/{foreign}/system-browser"),
            true,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &base,
            true,
            json!({"ws_endpoint":"ws://127.0.0.1:1/devtools/browser/untrusted"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "raw endpoints are not a product API"
    );
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &base,
            true,
            json!({"expected_incarnation":"stale"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &format!("{base}/tabs"),
            true,
            json!({"incarnation":"stale","choice_id":"https://untrusted.test"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    runtimes.running.store(true, Ordering::SeqCst);
    let response = router
        .clone()
        .oneshot(request("POST", &base, true, json!({})))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "Agent-running connect must be rejected before any browser discovery"
    );
    assert!(
        services
            .system_browser
            .as_ref()
            .unwrap()
            .snapshot(services.authoritative_user_id.as_ref(), id)
            .is_none()
    );
    assert_eq!(factory.calls.load(Ordering::SeqCst), 0);

    // Successful HTTP wiring uses only the disposable port above. A regression
    // in admission cannot accidentally connect the machine's personal browser.
    runtimes.running.store(false, Ordering::SeqCst);
    factory.gate.add_permits(1);
    let response = router
        .clone()
        .oneshot(request("POST", &base, true, json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let connected = json_body(response).await;
    let incarnation = connected["data"]["incarnation"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(connected["data"]["state"], "connected");
    assert_eq!(connected["data"]["tabs"], json!([]));
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &format!("{base}/choices"),
            true,
            json!({"incarnation":incarnation}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["data"]["tabs"][0]["choice_id"],
        "fixture-choice"
    );
    assert!(
        system
            .snapshot(services.authoritative_user_id.as_ref(), id)
            .unwrap()
            .tabs
            .is_empty()
    );
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &format!("{base}/tabs"),
            true,
            json!({"incarnation":incarnation,"choice_id":"fixture-choice"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["data"]["tabs"][0]["tab_id"],
        "fixture-grant"
    );
    let first = factory.connections.lock().unwrap()[0].clone();
    runtimes.running.store(true, Ordering::SeqCst);
    for (method, suffix, body) in [
        (
            "POST",
            "/tabs",
            json!({"incarnation":incarnation,"choice_id":"fixture-choice"}),
        ),
        ("DELETE", "", json!({"incarnation":incarnation})),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, &format!("{base}{suffix}"), true, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
    assert_eq!(first.grants.load(Ordering::SeqCst), 1);
    assert_eq!(first.closes.load(Ordering::SeqCst), 0);
    runtimes.running.store(false, Ordering::SeqCst);
    let response = router
        .clone()
        .oneshot(request(
            "DELETE",
            &base,
            true,
            json!({"incarnation":incarnation}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(first.closes.load(Ordering::SeqCst), 1);

    // Drop the actual HTTP future while the factory has not delivered a socket.
    // The outer idle-reconfiguration task must receive this cancellation too.
    let pending = {
        let router = router.clone();
        let base = base.clone();
        let incarnation = incarnation.clone();
        tokio::spawn(async move {
            router
                .oneshot(request(
                    "POST",
                    &base,
                    true,
                    json!({"expected_incarnation":incarnation}),
                ))
                .await
        })
    };
    wait_until(|| factory.calls.load(Ordering::SeqCst) == 2).await;
    pending.abort();
    let _ = pending.await;
    factory.gate.add_permits(1);
    wait_until(|| {
        system
            .snapshot(services.authoritative_user_id.as_ref(), id)
            .unwrap()
            .state
            == SystemBrowserState::Disconnected
    })
    .await;
    assert_eq!(
        factory.connections.lock().unwrap()[1]
            .closes
            .load(Ordering::SeqCst),
        1
    );

    // A separate DELETE can cancel a pending POST without waiting for Chrome's
    // connection factory to finish behind the conversation preparation gate.
    let previous = system
        .snapshot(services.authoritative_user_id.as_ref(), id)
        .unwrap()
        .incarnation;
    let connecting = {
        let router = router.clone();
        let base = base.clone();
        tokio::spawn(async move {
            router
                .oneshot(request(
                    "POST",
                    &base,
                    true,
                    json!({"expected_incarnation":previous}),
                ))
                .await
                .unwrap()
        })
    };
    wait_until(|| factory.calls.load(Ordering::SeqCst) == 3).await;
    let pending_incarnation = system
        .snapshot(services.authoritative_user_id.as_ref(), id)
        .unwrap()
        .incarnation;
    let disconnecting = {
        let router = router.clone();
        let base = base.clone();
        tokio::spawn(async move {
            router
                .oneshot(request(
                    "DELETE",
                    &base,
                    true,
                    json!({"incarnation":pending_incarnation}),
                ))
                .await
                .unwrap()
        })
    };
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
            .await
            .unwrap()
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    wait_until(|| {
        system
            .snapshot(services.authoritative_user_id.as_ref(), id)
            .unwrap()
            .state
            == SystemBrowserState::Disconnecting
    })
    .await;
    factory.gate.add_permits(1);
    assert_eq!(disconnecting.await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        factory.connections.lock().unwrap()[2]
            .closes
            .load(Ordering::SeqCst),
        1
    );
    let previous = system
        .snapshot(services.authoritative_user_id.as_ref(), id)
        .unwrap()
        .incarnation;
    factory.gate.add_permits(1);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &base,
            true,
            json!({"expected_incarnation":previous}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleting_connection = factory.connections.lock().unwrap()[3].clone();
    deleting_connection.fail_close.store(true, Ordering::SeqCst);
    let delete_path = format!("/api/conversations/{id}");
    let response = router
        .clone()
        .oneshot(request("DELETE", &delete_path, true, Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let count: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT count(*) FROM conversations WHERE conversation_id=?",
    )
    .bind(id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        count, 1,
        "unconfirmed disconnect must preserve the conversation"
    );
    let response = router
        .clone()
        .oneshot(request("DELETE", &delete_path, true, Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(deleting_connection.closes.load(Ordering::SeqCst), 2);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
