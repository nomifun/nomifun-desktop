//! Black-box integration tests for auth REST API routes.
//!
//! Covers test-plan items T4 (login), T5 (logout), T6 (auth status),
//! T7 (current user), T8 (change password), T9 (refresh token),
//! T10 (ws token), T11 (QR login).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use nomifun_auth::{
    AuthPolicy, AuthRouterState, CookieConfig, DeviceStore, JwtService, QrTokenStore, TrustState, auth_routes,
    hash_password, trust_resolve_middleware,
};
use nomifun_db::{IUserRepository, SqliteUserRepository, init_database_memory};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Create a test app with an in-memory database.
async fn test_app() -> (Router, TestContext) {
    test_app_with_local(false).await
}

async fn test_app_with_local(local: bool) -> (Router, TestContext) {
    test_app_with_policy(if local { AuthPolicy::NoAuth } else { AuthPolicy::Required }).await
}

const LOCAL_SETUP_SECRET: &str = "isolated-first-run-local-trust";

async fn test_app_with_policy(policy: AuthPolicy) -> (Router, TestContext) {
    let db = init_database_memory().await.unwrap();
    let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let user_repo = Arc::new(SqliteUserRepository::new(db.pool().clone())) as Arc<dyn IUserRepository>;
    let devices = Arc::new(DeviceStore::default());
    let jwt_service = Arc::new(JwtService::with_devices("test_secret_for_routes".into(), devices.clone()));
    let cookie_config = Arc::new(CookieConfig {
        secure: false,
        same_site: "Lax",
    });
    let qr_token_store = Arc::new(QrTokenStore::new());

    let state = AuthRouterState {
        auth_policy: policy,
        jwt_service: jwt_service.clone(),
        user_repo: user_repo.clone(),
        cookie_config,
        qr_token_store: qr_token_store.clone(),
    };

    // Mirror `create_router`: the global trust middleware resolves local trust
    // (and injects the installation owner / `LocalTrusted` marker) before the per-route
    // auth + local-only gates run. State and middleware use the same policy.
    let trust_state = TrustState {
        policy,
        local_trust_secret: (policy == AuthPolicy::TrustLocalToken).then(|| Arc::from(LOCAL_SETUP_SECRET)),
        authoritative_user_id: Arc::from(installation_owner.as_str()),
    };
    let app = auth_routes(state).layer(axum::middleware::from_fn_with_state(trust_state, trust_resolve_middleware));
    let ctx = TestContext {
        jwt_service,
        user_repo,
        qr_token_store,
        devices,
        _db: db,
    };
    (app, ctx)
}

/// Holds references needed by test assertions.
struct TestContext {
    jwt_service: Arc<JwtService>,
    user_repo: Arc<dyn IUserRepository>,
    qr_token_store: Arc<QrTokenStore>,
    devices: Arc<DeviceStore>,
    _db: nomifun_db::Database,
}

/// Helper: create a test user with known credentials.
///
/// The installation owner already uses `username = "admin"` with an empty
/// password hash. If the test asks for that username, update the owner
/// row in place instead of trying to INSERT a duplicate. Any other username
/// takes the normal create_user path.
async fn create_test_user(ctx: &TestContext, username: &str, password: &str) {
    let hash = hash_password(password).unwrap();
    if username == "admin" {
        ctx.user_repo
            .set_system_user_credentials(username, &hash)
            .await
            .unwrap();
    } else {
        ctx.user_repo.create_user(username, &hash).await.unwrap();
    }
}

/// Helper: perform a JSON POST request.
fn json_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn setup_request(path: &str, username: &str) -> Request<Body> {
    let mut body = serde_json::json!({"username":username,"password":"SecurePass123!"});
    if path.ends_with("device-setup") {
        body["device_name"] = serde_json::json!("Setup phone");
        body["protocol_version"] = serde_json::json!(1);
    }
    json_post(path, &body.to_string())
}

/// Helper: perform a JSON POST request with auth token.
fn json_post_with_token(uri: &str, body: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// Helper: perform a GET request with auth token.
fn get_with_token(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

/// Helper: perform a GET request without auth.
fn get_anonymous(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

/// Helper: extract response body as JSON.
async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Helper: login and return (token, user_id).
async fn login(app: &mut Router, username: &str, password: &str) -> (String, String) {
    let req = json_post(
        "/login",
        &format!(r#"{{"username":"{username}","password":"{password}"}}"#),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let token = json["token"].as_str().unwrap().to_owned();
    let user_id = json["user"]["user_id"].as_str().unwrap().to_owned();
    (token, user_id)
}

// ===========================================================================
// T4. Login (POST /login)

#[tokio::test]
async fn mobile_pairing_is_distinct_one_time_and_individually_revocable() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let (owner, _) = login(&mut app, "admin", "secure_pass123").await;
    let info = app.clone().oneshot(get_anonymous("/api/connections/info")).await.unwrap();
    assert_eq!(info.status(), StatusCode::OK);
    let info = body_json(info).await;
    assert_eq!(info["data"]["protocol_version"], 1);
    assert_eq!(info["data"]["desktop_id"], ctx.user_repo.get_system_user().await.unwrap().unwrap().user_id.as_str());

    let browser_qr = ctx.qr_token_store.generate();
    let response = app.clone().oneshot(json_post("/api/auth/device-pair", &serde_json::json!({"qr_token":browser_qr,"device_name":"Phone","protocol_version":1}).to_string())).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(response).await["code"], "DEVICE_PAIRING_INVALID");

    let mut paired = Vec::new();
    for name in ["Phone A", "Phone B"] {
        let response = app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ticket = body_json(response).await["data"].clone();
        let qr = ticket["qr_token"].as_str().unwrap();
        let body = serde_json::json!({"qr_token":qr,"device_name":name,"protocol_version":1}).to_string();
        let response = app.clone().oneshot(json_post("/api/auth/device-pair", &body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none(), "pairing must not create a browser session");
        let device = body_json(response).await;
        let token = device["token"].as_str().unwrap().to_owned();
        let id = device["device"]["device_id"].as_str().unwrap().to_owned();
        assert_eq!(ctx.jwt_service.verify(&token).unwrap().device_id.as_deref(), Some(id.as_str()));
        let status = app.clone().oneshot(get_with_token(&format!("/api/connections/pairing/{}", ticket["ticket_id"].as_str().unwrap()), &owner)).await.unwrap();
        let status = body_json(status).await;
        assert_eq!(status["data"]["state"], "used");
        assert_eq!(status["data"]["device_id"], id);
        let again = app.clone().oneshot(json_post("/api/auth/device-pair", &body)).await.unwrap();
        assert_eq!(body_json(again).await["code"], "DEVICE_PAIRING_USED");
        paired.push((id, token));
    }
    let (a_id, a_token) = &paired[0];
    let (_, b_token) = &paired[1];
    let response = app.clone().oneshot(get_with_token("/api/connections/devices", a_token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN, "paired phones cannot manage each other");
    let response = app.clone().oneshot(json_post("/api/auth/refresh", &serde_json::json!({"token":a_token}).to_string())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed = body_json(response).await["token"].as_str().unwrap().to_owned();
    assert_eq!(ctx.jwt_service.verify(&refreshed).unwrap().device_id.as_deref(), Some(a_id.as_str()));
    let response = app.clone().oneshot(Request::builder().method("DELETE").uri(format!("/api/connections/devices/{a_id}")).header("authorization", format!("Bearer {owner}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    for token in [a_token, &refreshed] {
        assert!(ctx.jwt_service.verify(token).is_err());
        let response = app.clone().oneshot(get_with_token("/api/auth/user", token)).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_json(response).await["code"], "DEVICE_REVOKED");
        let response = app.clone().oneshot(json_post("/api/auth/refresh", &serde_json::json!({"token":token}).to_string())).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_json(response).await["code"], "DEVICE_REVOKED");
    }
    assert!(ctx.jwt_service.verify(b_token).is_ok());
    assert!(ctx.jwt_service.verify(&owner).is_ok());
    let self_remove = app.clone().oneshot(Request::builder().method("DELETE").uri("/api/connections/device").header("authorization", format!("Bearer {b_token}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(self_remove.status(), StatusCode::OK);
    assert!(ctx.jwt_service.verify(b_token).is_err());
}

#[tokio::test]
async fn pair_device_failure_at_the_device_cap_reports_failed_not_used() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let (owner, owner_id) = login(&mut app, "admin", "secure_pass123").await;
    for i in 0..128 {
        ctx.devices.register(&owner_id, &format!("Phone {i}")).await.unwrap();
    }
    let response = app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ticket = body_json(response).await["data"].clone();
    let body = serde_json::json!({"qr_token":ticket["qr_token"],"device_name":"Phone 129","protocol_version":1}).to_string();
    let response = app.clone().oneshot(json_post("/api/auth/device-pair", &body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let status = app.clone().oneshot(get_with_token(&format!("/api/connections/pairing/{}", ticket["ticket_id"].as_str().unwrap()), &owner)).await.unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status = body_json(status).await;
    assert_eq!(status["data"]["state"], "failed");
    assert_eq!(status["data"]["failure_code"], "DEVICE_LIMIT_REACHED");
    assert!(status["data"]["device_id"].is_null());
}

#[tokio::test]
async fn a_fresh_pairing_code_supersedes_an_unused_earlier_code() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let (owner, _) = login(&mut app, "admin", "secure_pass123").await;
    let ticket_a = body_json(app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap()).await["data"].clone();
    let ticket_b = body_json(app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap()).await["data"].clone();
    let pair = |qr: &str| app.clone().oneshot(json_post("/api/auth/device-pair", &serde_json::json!({"qr_token":qr,"device_name":"Phone","protocol_version":1}).to_string()));
    let response = pair(ticket_a["qr_token"].as_str().unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(response).await["code"], "DEVICE_PAIRING_INVALID");
    let status = app.clone().oneshot(get_with_token(&format!("/api/connections/pairing/{}", ticket_a["ticket_id"].as_str().unwrap()), &owner)).await.unwrap();
    assert_eq!(status.status(), StatusCode::NOT_FOUND);
    let response = pair(ticket_b["qr_token"].as_str().unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn browser_password_change_signs_out_every_paired_phone() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let (owner, _) = login(&mut app, "admin", "secure_pass123").await;
    let ticket = body_json(app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap()).await["data"].clone();
    let pair = serde_json::json!({"qr_token":ticket["qr_token"],"device_name":"Phone","protocol_version":1}).to_string();
    let response = app.clone().oneshot(json_post("/api/auth/device-pair", &pair)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let device = body_json(response).await;
    let device_token = device["token"].as_str().unwrap().to_owned();
    let device_id = device["device"]["device_id"].as_str().unwrap().to_owned();

    let request = json_post_with_token("/api/auth/change-password", r#"{"current_password":"secure_pass123","new_password":"NewP@ssword2"}"#, &owner);
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The rotated secret plus the matching revoked record kill the credential.
    let response = app.clone().oneshot(get_with_token("/api/auth/user", &device_token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let (owner, _) = login(&mut app, "admin", "NewP@ssword2").await;
    let response = app.clone().oneshot(get_with_token("/api/connections/devices", &owner)).await.unwrap();
    let devices = body_json(response).await["data"].clone();
    let row = devices.as_array().unwrap().iter().find(|d| d["device_id"] == device_id).unwrap();
    assert!(row["revoked_at_ms"].is_number(), "registry must match the dead token: {row}");

    let ticket = body_json(app.clone().oneshot(json_post_with_token("/api/connections/pairing", "{}", &owner)).await.unwrap()).await["data"].clone();
    let pair = serde_json::json!({"qr_token":ticket["qr_token"],"device_name":"Phone again","protocol_version":1}).to_string();
    let response = app.clone().oneshot(json_post("/api/auth/device-pair", &pair)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn desktop_local_password_change_keeps_paired_phones() {
    let (app, ctx) = test_app_with_local(true).await;
    let owner = ctx.user_repo.get_system_user().await.unwrap().unwrap().user_id;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let ticket = body_json(app.clone().oneshot(json_post("/api/connections/pairing", "{}")).await.unwrap()).await["data"].clone();
    let pair = serde_json::json!({"qr_token":ticket["qr_token"],"device_name":"Phone","protocol_version":1}).to_string();
    let response = app.clone().oneshot(json_post("/api/auth/device-pair", &pair)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let device = body_json(response).await;
    let device_token = device["token"].as_str().unwrap().to_owned();
    assert!(ctx.jwt_service.verify(&device_token).is_ok());

    let response = app.clone().oneshot(json_post("/api/webui/change-password", r#"{"new_password":"NewP@ssword2"}"#)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(ctx.jwt_service.verify(&device_token).is_ok(), "Desktop-local password change must not sign out paired phones");
    assert!(ctx.devices.is_active(device["device"]["device_id"].as_str().unwrap(), owner.as_str()));
}

#[tokio::test]
async fn protocol_and_device_name_rejection_do_not_consume_invitation() {
    let (app, ctx) = test_app().await;
    let ticket = ctx.qr_token_store.generate_device_ticket();
    for (name, protocol, expected) in [("Phone", 2, StatusCode::UPGRADE_REQUIRED), ("  ", 1, StatusCode::BAD_REQUEST)] {
        let response = app.clone().oneshot(json_post("/api/auth/device-pair", &serde_json::json!({"qr_token":ticket.qr_token,"device_name":name,"protocol_version":protocol}).to_string())).await.unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(ctx.qr_token_store.device_ticket_status(&ticket.ticket_id).unwrap().state, "ready");
    }
}

#[tokio::test]
async fn mobile_password_login_issues_only_individually_revocable_credentials() {
    let (app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "secure_pass123").await;
    let request = serde_json::json!({"username":"admin","password":"secure_pass123","device_name":"Manual phone","protocol_version":1}).to_string();
    let response = app.clone().oneshot(json_post("/api/auth/device-login", &request)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let response = body_json(response).await;
    let token = response["token"].as_str().unwrap();
    let id = response["device"]["device_id"].as_str().unwrap();
    assert_eq!(ctx.jwt_service.verify(token).unwrap().device_id.as_deref(), Some(id));
    let response = app.clone().oneshot(get_with_token("/api/connections/devices", token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app.clone().oneshot(Request::builder().method("DELETE").uri("/api/connections/device").header("authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(ctx.jwt_service.verify(token).is_err());
    let wrong = serde_json::json!({"username":"admin","password":"wrong","device_name":"Manual phone","protocol_version":1}).to_string();
    assert_eq!(app.oneshot(json_post("/api/auth/device-login", &wrong)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mobile_first_setup_retains_the_atomic_single_claim_and_device_identity() {
    let (app, ctx) = test_app().await;
    let wrong_version = serde_json::json!({"username":"mobile_admin","password":"secure_pass123","device_name":"First phone","protocol_version":2}).to_string();
    assert_eq!(app.clone().oneshot(json_post("/api/auth/device-setup", &wrong_version)).await.unwrap().status(), StatusCode::UPGRADE_REQUIRED);
    assert!(!ctx.user_repo.has_users().await.unwrap());
    let request = serde_json::json!({"username":"mobile_admin","password":"secure_pass123","device_name":"First phone","protocol_version":1}).to_string();
    let (a, b) = tokio::join!(app.clone().oneshot(json_post("/api/auth/device-setup", &request)), app.clone().oneshot(json_post("/api/auth/device-setup", &request)));
    let a = a.unwrap(); let b = b.unwrap();
    let response = match (a.status(), b.status()) {
        (StatusCode::OK, StatusCode::CONFLICT) => a,
        (StatusCode::CONFLICT, StatusCode::OK) => b,
        states => panic!("setup must have exactly one winner: {states:?}"),
    };
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let response = body_json(response).await;
    let token = response["token"].as_str().unwrap();
    assert_eq!(ctx.jwt_service.verify(token).unwrap().device_id.as_deref(), response["device"]["device_id"].as_str());
    assert_eq!(response["user"]["username"], "mobile_admin");
}

#[tokio::test]
async fn desktop_setup_requires_local_trust_even_with_owner_or_device_credentials() {
    for path in ["/api/auth/setup", "/api/auth/device-setup"] {
        for (credential, origin) in [
            ("none", Some("https://attacker.example")),
            ("wrong-trust", Some("null")),
            ("owner", Some("tauri://localhost")),
            ("device", None),
        ] {
            let (app, ctx) = test_app_with_policy(AuthPolicy::TrustLocalToken).await;
            let owner = ctx.user_repo.get_system_user().await.unwrap().unwrap();
            let device = ctx.devices.register(owner.user_id.as_str(), "Existing phone").await.unwrap();
            let original_devices = ctx.devices.list(owner.user_id.as_str()).unwrap();
            let mut request = setup_request(path, "claimed");
            if let Some(origin) = origin { request.headers_mut().insert(header::ORIGIN, origin.parse().unwrap()); }
            match credential {
                "wrong-trust" => { request.headers_mut().insert(nomifun_auth::LOCAL_TRUST_HEADER, "wrong-secret".parse().unwrap()); }
                "owner" | "device" => {
                    let token = if credential == "owner" {
                        ctx.jwt_service.sign(owner.user_id.as_str(), &owner.username).unwrap()
                    } else {
                        ctx.jwt_service.sign_for_device(owner.user_id.as_str(), &owner.username, &device.device_id).unwrap()
                    };
                    request.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
                }
                _ => {}
            }
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path} / {credential}");
            assert!(response.headers().get(header::SET_COOKIE).is_none());
            let body = body_json(response).await;
            assert!(body.get("token").is_none());
            let unchanged = ctx.user_repo.get_system_user().await.unwrap().unwrap();
            assert_eq!(unchanged.username, owner.username);
            assert!(unchanged.password_hash.is_empty());
            assert_eq!(serde_json::to_value(ctx.devices.list(owner.user_id.as_str()).unwrap()).unwrap(), serde_json::to_value(original_devices).unwrap());
        }
    }
}

#[tokio::test]
async fn locally_trusted_desktop_can_initialize_each_setup_route() {
    for path in ["/api/auth/setup", "/api/auth/device-setup"] {
        let (app, ctx) = test_app_with_policy(AuthPolicy::TrustLocalToken).await;
        let mut request = setup_request(path, "chosen_owner");
        // Origin is never the authority: valid local trust also works with an
        // opaque origin or a headless caller, independently of CORS.
        request.headers_mut().insert(header::ORIGIN, "null".parse().unwrap());
        request.headers_mut().insert(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_SETUP_SECRET.parse().unwrap());
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let body = body_json(response).await;
        let claims = ctx.jwt_service.verify(body["token"].as_str().unwrap()).unwrap();
        let owner = ctx.user_repo.get_system_user().await.unwrap().unwrap();
        assert_eq!(owner.username, "chosen_owner");
        assert!(!owner.password_hash.is_empty());
        assert_eq!(claims.user_id, owner.user_id);
        assert_eq!(claims.device_id.is_some(), path.ends_with("device-setup"));
        assert_eq!(ctx.devices.list(owner.user_id.as_str()).unwrap().len(), usize::from(path.ends_with("device-setup")));
    }
}

#[tokio::test]
async fn standalone_and_explicit_no_auth_preserve_first_run_setup() {
    for policy in [AuthPolicy::Required, AuthPolicy::NoAuth] {
        for path in ["/api/auth/setup", "/api/auth/device-setup"] {
            let (app, ctx) = test_app_with_policy(policy).await;
            let response = app.oneshot(setup_request(path, "web_owner")).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{policy:?} / {path}");
            let body = body_json(response).await;
            assert!(ctx.jwt_service.verify(body["token"].as_str().unwrap()).is_ok());
            assert!(ctx.user_repo.has_users().await.unwrap());
        }
    }
}
// ===========================================================================

#[tokio::test]
async fn t4_1_login_success() {
    let (app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;

    let req = json_post("/login", r#"{"username":"admin","password":"StrongP@ss1"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Check Set-Cookie header
    let set_cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie.contains("nomifun-session="));
    assert!(set_cookie.contains("HttpOnly"));

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["message"], "Login successful");
    assert!(json["token"].is_string());
    assert_eq!(json["user"]["username"], "admin");
    assert!(json["user"]["user_id"].is_string());

    // Verify the returned token is valid
    let token = json["token"].as_str().unwrap();
    assert!(ctx.jwt_service.verify(token).is_ok());
}

#[tokio::test]
async fn t4_1_login_rejects_client_only_remember_field_before_authentication() {
    let (app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;

    // Regression witness for the browser login failure: the UI used to leak
    // its local "remember me" preference into this strict wire request. Axum
    // rejects unknown fields before the handler can verify the password.
    let req = json_post(
        "/login",
        r#"{"username":"admin","password":"StrongP@ss1","remember":false}"#,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["code"], "BAD_REQUEST");
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown field `remember`"),
        "the rejected client-only field should remain diagnosable: {json}"
    );
}

#[tokio::test]
async fn t4_2_login_nonexistent_user() {
    let (app, _ctx) = test_app().await;

    let req = json_post("/login", r#"{"username":"ghost","password":"whatever"}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn t4_3_login_wrong_password() {
    let (app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "CorrectP@ss1").await;

    let req = json_post("/login", r#"{"username":"admin","password":"WrongPass1"}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t4_4_login_missing_fields() {
    let (app, _ctx) = test_app().await;

    // Missing password
    let req = json_post("/login", r#"{"username":"admin"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Missing username
    let req = json_post("/login", r#"{"password":"test"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Empty body
    let req = json_post("/login", r#"{}"#);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn t4_5_login_empty_password_hash_returns_401() {
    // Regression: when the installation owner has an empty password_hash
    // (first-run local mode), POST /login must return 401, not 500.
    let (app, _ctx) = test_app_with_local(true).await;

    let req = json_post("/login", r#"{"username":"admin","password":"anything"}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
    assert_eq!(json["code"], "UNAUTHORIZED");
}

#[tokio::test]
async fn t4_6_login_username_too_long() {
    let (app, _ctx) = test_app().await;

    let long_name = "a".repeat(33);
    let body = format!(r#"{{"username":"{long_name}","password":"test1234"}}"#);
    let req = json_post("/login", &body);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn t4_6_login_password_too_long() {
    let (app, _ctx) = test_app().await;

    let long_pass = "a".repeat(129);
    let body = format!(r#"{{"username":"admin","password":"{long_pass}"}}"#);
    let req = json_post("/login", &body);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// T5. Logout (POST /logout)
// ===========================================================================

#[tokio::test]
async fn t5_1_logout_success() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    let req = json_post_with_token("/logout", "", &token);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Cookie should be cleared
    let set_cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie.contains("Max-Age=0"));

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["message"], "Logged out successfully");
}

#[tokio::test]
async fn t5_2_logout_token_becomes_invalid() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    // Logout
    let req = json_post_with_token("/logout", "", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Try to use the token
    let req = get_with_token("/api/auth/user", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn t5_3_logout_unauthenticated() {
    let (app, _ctx) = test_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/logout")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
// T6. Auth Status (GET /api/auth/status)
// ===========================================================================

#[tokio::test]
async fn t6_1_status_needs_setup() {
    let (app, _ctx) = test_app().await;

    let req = get_anonymous("/api/auth/status");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["needs_setup"], true);
    assert_eq!(json["is_authenticated"], false);
}

#[tokio::test]
async fn t6_2_status_has_users() {
    let (app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;

    let req = get_anonymous("/api/auth/status");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["needs_setup"], false);
}

#[tokio::test]
async fn t6_3_status_authenticated() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/auth/status", &token);
    let resp = app.oneshot(req).await.unwrap();

    let json = body_json(resp).await;
    assert_eq!(json["is_authenticated"], true);
}

#[tokio::test]
async fn t6_4_status_unauthenticated() {
    let (app, _ctx) = test_app().await;

    let req = get_anonymous("/api/auth/status");
    let resp = app.oneshot(req).await.unwrap();

    let json = body_json(resp).await;
    assert_eq!(json["is_authenticated"], false);
}

// ===========================================================================
// T7. Current User (GET /api/auth/user)
// ===========================================================================

#[tokio::test]
async fn t7_1_get_user_success() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/auth/user", &token);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["user"]["username"], "admin");
    assert!(json["user"]["user_id"].is_string());
}

#[tokio::test]
async fn t7_2_get_user_invalid_token() {
    let (app, _ctx) = test_app().await;

    let req = get_with_token("/api/auth/user", "invalid.jwt.token");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn t7_3_get_user_no_token() {
    let (app, _ctx) = test_app().await;

    let req = get_anonymous("/api/auth/user");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
// T8. Change Password (POST /api/auth/change-password)
// ===========================================================================

#[tokio::test]
async fn t8_1_change_password_success() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "OldP@ssword1").await;
    let (token, _) = login(&mut app, "admin", "OldP@ssword1").await;

    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"OldP@ssword1","new_password":"NewP@ssword2"}"#,
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["message"], "Password changed successfully");
}

#[tokio::test]
async fn t8_2_change_password_old_token_invalidated() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "OldP@ssword1").await;
    let (token, _) = login(&mut app, "admin", "OldP@ssword1").await;

    // Change password
    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"OldP@ssword1","new_password":"NewP@ssword2"}"#,
        &token,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Old token should be invalid (JWT secret rotated)
    let req = get_with_token("/api/auth/user", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn t8_3_change_password_wrong_current() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "CorrectP@ss1").await;
    let (token, _) = login(&mut app, "admin", "CorrectP@ss1").await;

    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"WrongP@ss1","new_password":"NewP@ssword2"}"#,
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t8_4_change_password_new_too_short() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "OldP@ssword1").await;
    let (token, _) = login(&mut app, "admin", "OldP@ssword1").await;

    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"OldP@ssword1","new_password":"short"}"#,
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn t8_6_change_password_weak() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "OldP@ssword1").await;
    let (token, _) = login(&mut app, "admin", "OldP@ssword1").await;

    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"OldP@ssword1","new_password":"password"}"#,
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn t8_7_change_password_missing_fields() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "OldP@ssword1").await;
    let (token, _) = login(&mut app, "admin", "OldP@ssword1").await;

    // Missing newPassword
    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"current_password":"OldP@ssword1"}"#,
        &token,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Missing currentPassword
    let req = json_post_with_token(
        "/api/auth/change-password",
        r#"{"new_password":"NewP@ssword2"}"#,
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// T9. Refresh Token (POST /api/auth/refresh)
// ===========================================================================

#[tokio::test]
async fn t9_1_refresh_token_success() {
    let (mut app, ctx) = test_app().await;
    create_test_user(&ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    let body = format!(r#"{{"token":"{token}"}}"#);
    let req = json_post("/api/auth/refresh", &body);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert!(json["token"].is_string());

    // New token should be valid
    let new_token = json["token"].as_str().unwrap();
    assert!(ctx.jwt_service.verify(new_token).is_ok());
}

#[tokio::test]
async fn t9_2_refresh_invalid_token() {
    let (app, _ctx) = test_app().await;

    let req = json_post("/api/auth/refresh", r#"{"token":"fake.jwt.token"}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t9_3_refresh_missing_token() {
    let (app, _ctx) = test_app().await;

    let req = json_post("/api/auth/refresh", r#"{}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// T10. WebSocket Token (GET /api/ws-token)
// ===========================================================================

#[tokio::test]
async fn t10_1_ws_token_success() {
    let (mut app, _ctx) = test_app().await;
    create_test_user(&_ctx, "admin", "StrongP@ss1").await;
    let (token, _) = login(&mut app, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/ws-token", &token);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert!(json["ws_token"].is_string());
    assert!(json["expires_in"].is_number());

    // expires_in should be 30 days in milliseconds
    let expires_in = json["expires_in"].as_u64().unwrap();
    assert_eq!(expires_in, 30 * 24 * 60 * 60 * 1000);
}

#[tokio::test]
async fn t10_2_ws_token_unauthenticated() {
    let (app, _ctx) = test_app().await;

    let req = get_anonymous("/api/ws-token");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
// T11. QR Login (POST /api/auth/qr-login)
// ===========================================================================

#[tokio::test]
async fn t11_1_qr_login_success() {
    let (app, ctx) = test_app().await;

    // Set up installation-owner credentials so login works
    let hash = hash_password("syspass123").unwrap();
    ctx.user_repo
        .set_system_user_credentials("sysadmin", &hash)
        .await
        .unwrap();

    // Generate QR token
    let qr_token = ctx.qr_token_store.generate();

    let body = format!(r#"{{"qr_token":"{qr_token}"}}"#);
    let req = json_post("/api/auth/qr-login", &body);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Check Set-Cookie
    let set_cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie.contains("nomifun-session="));

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert!(json["token"].is_string());
    assert_eq!(json["user"]["username"], "sysadmin");
}

#[tokio::test]
async fn t11_2_qr_login_invalid_token() {
    let (app, _ctx) = test_app().await;

    let req = json_post("/api/auth/qr-login", r#"{"qr_token":"nonexistent"}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t11_4_qr_login_already_used() {
    let (app, ctx) = test_app().await;

    let hash = hash_password("syspass123").unwrap();
    ctx.user_repo
        .set_system_user_credentials("sysadmin", &hash)
        .await
        .unwrap();

    let qr_token = ctx.qr_token_store.generate();

    // First use succeeds
    let body = format!(r#"{{"qr_token":"{qr_token}"}}"#);
    let req = json_post("/api/auth/qr-login", &body);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second use fails
    let req = json_post("/api/auth/qr-login", &body);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t11_5_qr_login_missing_token() {
    let (app, _ctx) = test_app().await;

    let req = json_post("/api/auth/qr-login", r#"{}"#);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// QR Login Page (GET /qr-login)
// ===========================================================================

#[tokio::test]
async fn qr_login_page_returns_html() {
    let (app, _ctx) = test_app().await;

    let req = get_anonymous("/qr-login");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.contains("text/html"));
}

/// Regression: the served QR-login page must POST the snake_case field that
/// `QrLoginRequest` deserializes (`qr_token`), not camelCase `qrToken`. A
/// mismatch made every phone scan fail with the serde body-rejection
/// "missing field `qr_token`".
#[tokio::test]
async fn qr_login_page_posts_snake_case_qr_token() {
    let (app, _ctx) = test_app().await;

    let resp = app.oneshot(get_anonymous("/qr-login")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(
        html.contains("qr_token"),
        "QR-login page must POST the snake_case `qr_token` field"
    );
    assert!(
        !html.contains("qrToken"),
        "QR-login page must not POST camelCase `qrToken` (serde rejects it)"
    );
}

#[tokio::test]
async fn qr_login_page_hands_success_state_to_spa_before_redirecting() {
    let (app, _ctx) = test_app().await;

    let resp = app.oneshot(get_anonymous("/qr-login")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(
        html.contains("credentials: 'same-origin'"),
        "QR-login POST must explicitly keep same-origin cookies"
    );
    assert!(
        html.contains("nomifun:qr-login-resume"),
        "QR-login page must stash the successful user for the SPA auth bridge"
    );
    assert!(
        html.contains("window.location.replace('/#/guid')"),
        "QR-login page must enter the conversation landing route explicitly after success"
    );
}

#[tokio::test]
async fn qr_login_page_checks_app_shell_before_redirecting() {
    let (app, _ctx) = test_app().await;

    let resp = app.oneshot(get_anonymous("/qr-login")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(
        html.contains("verifyAppShellThenRedirect"),
        "QR-login page must verify the SPA shell before navigating away"
    );
    assert!(
        html.contains("nomifun_spa_shell_check=1"),
        "SPA shell probe should be identifiable in server/client diagnostics"
    );
    assert!(
        html.contains("WebUI app shell is not reachable"),
        "QR-login page should report app-shell HTTP failures instead of surfacing a browser error"
    );
}

