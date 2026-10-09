//! Shared test helpers for nomifun-app E2E tests.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;
use wiremock::MockServer;

use nomifun_app::AppConfig;
use nomifun_app::compatibility::{
    AppServices, build_module_states, create_router, create_router_with_states,
};
use nomifun_auth::AuthPolicy;
use nomifun_file::FileService;
use nomifun_system::VersionCheckService;

fn isolated_config(prefix: &str) -> AppConfig {
    let root = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .unwrap()
        .keep();
    AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        ..AppConfig::default()
    }
}

pub async fn build_app() -> (axum::Router, AppServices) {
    let root = tempfile::Builder::new()
        .prefix("nomifun-app-e2e-")
        .tempdir()
        .unwrap()
        .keep();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(
        db,
        &AppConfig {
            data_dir: root.join("data"),
            work_dir: root.join("work"),
            ..AppConfig::default()
        },
    )
    .await
    .unwrap();
    let router = create_router(&services).await;
    (router, services)
}

pub async fn build_local_trust_app(secret: &str) -> (axum::Router, AppServices) {
    let root = tempfile::Builder::new()
        .prefix("nomifun-app-local-trust-e2e-")
        .tempdir()
        .unwrap()
        .keep();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(
        db,
        &AppConfig {
            data_dir: root.join("data"),
            work_dir: root.join("work"),
            auth_policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(std::sync::Arc::from(secret)),
            ..AppConfig::default()
        },
    )
    .await
    .unwrap();
    let router = create_router(&services).await;
    (router, services)
}

pub async fn materialize_builtin_skills_for_fixture(services: &AppServices) {
    // This fixture skips BootstrapContext, so perform the same real builtin
    // corpus materialization before frozen Session skills are resolved.
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(
        &services.data_dir, nomifun_skill_library::builtin_skills_corpus(), &corpus_version,
    ).await.expect("materialize the production builtin skill corpus for the isolated fixture");
}

/// Produce real encrypted-at-rest fixture credentials whose plaintext follows
/// the canonical typed credential-object contract.
pub fn encrypted_bearer_credentials() -> String {
    nomifun_common::encrypt_string(r#"{"api_keys":["test-only"]}"#, &[0x42; 32]).unwrap()
}

/// Idempotently seed one enabled model with an exact Chat capability.
///
/// The protocol is explicit so App E2E fixtures exercise the same normalized,
/// task-scoped authority as production.
pub async fn seed_openai_chat_model(pool: &nomifun_db::SqlitePool, provider_id: &str, model: &str) {
    nomifun_db::sqlx::query(
        "INSERT OR IGNORE INTO provider_models \
         (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
         VALUES (?, ?, 1, 0, NULL, 1, 1)",
    )
    .bind(provider_id)
    .bind(model)
    .execute(pool)
    .await
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT OR IGNORE INTO provider_model_capabilities \
         (provider_id, model, task, traits, protocol, connection_role, \
          allow_cross_origin_credentials, provider_params, created_at, updated_at) \
         VALUES (?, ?, 'chat', '[]', 'openai.chat_text', 'default', 0, '{}', 1, 1)",
    )
    .bind(provider_id)
    .bind(model)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn build_app_with_noop_opener() -> (axum::Router, AppServices) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &isolated_config("nomifun-noop-opener-e2e-"))
        .await
        .unwrap();
    let (mut states, _) = build_module_states(&services).await;
    states.shell.shell_service = std::sync::Arc::new(nomifun_shell::ShellService::new(
        std::sync::Arc::new(nomifun_shell::NoopSystemOpener),
    ));
    let router = create_router_with_states(&services, states);
    (router, services)
}

pub async fn build_app_with_file_roots(
    allowed_roots: Vec<std::path::PathBuf>,
) -> (axum::Router, AppServices) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &isolated_config("nomifun-file-roots-e2e-"))
        .await
        .unwrap();
    let (mut states, _) = build_module_states(&services).await;
    states.file.file_service =
        std::sync::Arc::new(FileService::with_inventory_cache(services.event_bus.clone(), allowed_roots, services.file_inventory.clone()));
    let router = create_router_with_states(&services, states);
    (router, services)
}

pub async fn build_app_with_mock_version(
    current_version: &str,
    mock_server: &MockServer,
) -> (axum::Router, AppServices) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &isolated_config("nomifun-version-e2e-"))
        .await
        .unwrap();
    let (mut states, _) = build_module_states(&services).await;
    let http_client = reqwest::Client::builder().no_proxy().build().unwrap();
    states.system.version_check_service = VersionCheckService::with_api_base(
        http_client,
        current_version.to_owned(),
        mock_server.uri(),
    );
    let router = create_router_with_states(&services, states);
    (router, services)
}

pub async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn extract_csrf_token(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("nomifun-csrf-token="))
        .map(|s| {
            s.strip_prefix("nomifun-csrf-token=")
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        })
}

pub fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

pub fn get_with_token(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn is_public_conversation_send(method: &str, uri: &str) -> bool {
    if !method.eq_ignore_ascii_case("POST") {
        return false;
    }
    let Ok(uri) = uri.parse::<axum::http::Uri>() else {
        return false;
    };
    if uri.query().is_some() {
        return false;
    }
    let Some(conversation_id) = uri
        .path()
        .strip_prefix("/api/conversations/")
        .and_then(|path| path.strip_suffix("/messages"))
    else {
        return false;
    };

    !conversation_id.is_empty() && !conversation_id.contains('/')
}

pub fn json_with_token(
    method_str: &str,
    uri: &str,
    body: serde_json::Value,
    token: &str,
    csrf: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method_str)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", csrf)
        .header("cookie", format!("nomifun-csrf-token={csrf}"));
    if is_public_conversation_send(method_str, uri) {
        builder = builder.header("idempotency-key", nomifun_common::generate_id());
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

pub fn delete_with_token(uri: &str, token: &str, csrf: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", csrf)
        .header("cookie", format!("nomifun-csrf-token={csrf}"))
        .body(Body::empty())
        .unwrap()
}

/// Set up a user and login, returning (session_token, csrf_token).
///
/// The canonical installation owner already uses `username = "admin"`; if
/// the test asks for that username, overwrite the owner row's empty credentials
/// in place instead of trying to INSERT a duplicate.
pub async fn setup_and_login(
    app: &mut axum::Router,
    services: &AppServices,
    username: &str,
    password: &str,
) -> (String, String) {
    let hash = nomifun_auth::hash_password(password).unwrap();
    if username == "admin" {
        services
            .user_repo
            .set_system_user_credentials(username, &hash)
            .await
            .unwrap();
    } else {
        services
            .user_repo
            .create_user(username, &hash)
            .await
            .unwrap();
    }

    let resp = app
        .clone()
        .oneshot(get_request("/api/auth/status"))
        .await
        .unwrap();
    let csrf = extract_csrf_token(&resp).expect("CSRF cookie should be set");

    let body = format!(r#"{{"username":"{username}","password":"{password}"}}"#);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");

    let json = body_json(resp).await;
    let token = json["token"].as_str().unwrap().to_owned();

    (token, csrf)
}
