//! Provider health-check route auth and validation tests.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, json_with_token, setup_and_login};

#[tokio::test]
async fn provider_health_check_unauthenticated_is_rejected() {
    let (app, _services) = build_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/agents/provider-health-check")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"provider_id": "0190f5fe-7c00-7a00-8000-000000000010", "model": "gpt-4o"})).unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "expected auth rejection, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn provider_health_check_bearer_auth_bypasses_cookie_csrf() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/agents/provider-health-check")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&json!({"provider_id": "0190f5fe-7c00-7a00-8000-000000000010", "model": "gpt-4o"})).unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "Bearer-authenticated requests must reach provider validation without cookie CSRF"
    );
}

#[tokio::test]
async fn provider_health_check_cookie_auth_requires_csrf() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/agents/provider-health-check")
        .header("content-type", "application/json")
        .header("cookie", format!("nomifun-session={token}"))
        .body(Body::from(
            serde_json::to_vec(&json!({"provider_id": "0190f5fe-7c00-7a00-8000-000000000010", "model": "gpt-4o"})).unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn provider_health_check_validates_required_fields() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/agents/provider-health-check",
        json!({"provider_id": "", "model": "gpt-4o"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["code"], "BAD_REQUEST");
    assert!(
        json["error"]
            .as_str()
            .is_some_and(|message| {
                message.contains("provider_id")
                    && (message.contains("canonical lowercase hyphenated UUID")
                        || message.contains("invalid provider_id"))
            }),
        "expected canonical UUIDv7 provider_id contract error, got {json}"
    );
}

#[tokio::test]
async fn startup_repairs_saved_agnes_v20_before_catalog_health_and_real_invocation() {
    use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderModelCapabilityRepository, IProviderRepository,
        NewProviderModel, NewProviderModelCapability, SqliteProviderModelCapabilityRepository,
        SqliteProviderRepository,
    };
    use nomifun_model_invoke::{ModelRef, TaskOutcome, TaskRequest, VideoGenRequest};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{body_partial_json, header, method, path}};

    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/videos"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_partial_json(json!({"model":"agnes-video-v2.0", "width":1920,
            "height":1080, "frame_rate":16, "num_frames":161})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"video_id":"v20-native-job"})))
        .expect(2).mount(&server).await;
    let root = tempfile::tempdir().unwrap();
    let config = AppConfig { data_dir:root.path().join("data"), work_dir:root.path().join("work"),
        ..AppConfig::default() };
    let key = nomifun_app::load_or_create_data_encryption_key(&config.data_dir, "fixture-key-source").unwrap();
    let credentials = encrypt_string(&json!({"api_keys":["sk-test"]}).to_string(), &key).unwrap();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let base = format!("{}/v1", server.uri());
    // The old release persisted a Video model under the Image task/protocol.
    // Seed that exact state before startup, bypassing today's save validation.
    let legacy = [NewProviderModelCapability { task:"image_generation", traits:"[]",
        protocol:"agnes.images", connection_role:"default", endpoint:Some("/images/generations"),
        provider_params:r#"{"width":1920,"height":1080,"frame_rate":16,"num_frames":161,"n":1,"quality":"standard","response_format":"b64_json"}"#,
        ..Default::default() }];
    let (provider, _) = SqliteProviderRepository::new(db.pool().clone()).create(CreateProviderParams {
        provider_id:None, platform:"agnes", name:"Agnes", base_url:&base, auth_scheme:"bearer",
        credentials_encrypted:&credentials, enabled:true, bedrock_config:None, sort_order:None,
    }, &NewProviderModel { model:"agnes-video-v2.0", enabled:true, capabilities:&legacy,
        ..Default::default() }, &[]).await.unwrap();
    let services = AppServices::from_config(db, &config).await.unwrap();
    let mut app = create_router(&services).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let response = app.clone().oneshot(json_with_token("GET", "/api/providers", json!(null),
        &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let catalog = body_json(response).await;
    let capability = &catalog["data"][0]["models"][0]["capabilities"][0];
    assert_eq!(capability["task"], "video_generation");
    assert_eq!(capability["protocol"], "agnes.video_jobs");
    assert_eq!(capability["poll_endpoint"], format!("{}/agnesapi?video_id={{id}}", server.uri()));
    assert!(capability["health"].is_null());

    let response = app.oneshot(json_with_token("POST", "/api/agents/provider-health-check",
        json!({"provider_id":provider.provider_id, "model":"agnes-video-v2.0", "task":"video_generation"}),
        &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let health = body_json(response).await;
    assert_eq!(health["data"]["status"], "healthy", "{health}");
    assert_eq!(health["data"]["task"], "video_generation");
    assert_eq!(health["data"]["attempted_url"], format!("{}/v1/videos", server.uri()));
    let outcome = services.model_invoke_service.invoke(&ModelRef {
        provider_id:provider.provider_id.clone(), model:"agnes-video-v2.0".into(),
    }, TaskRequest::VideoGeneration(VideoGenRequest {
        prompt:"waves".into(), seconds:None, size:None, resolution:None,
        inputs:vec![], extra:json!({}),
    })).await.unwrap();
    let TaskOutcome::Pending(job) = outcome else { panic!("expected native video job") };
    assert_eq!(job.adapter_id, "agnes.video_jobs");
    assert_eq!(job.remote_id, "v20-native-job");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "no request may reach /images/generations");
    for request in requests {
        let body:serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        for image_or_v25_key in ["n", "quality", "response_format", "size", "seconds", "mode"] {
            assert!(body.get(image_or_v25_key).is_none(), "unexpected wire key {image_or_v25_key}: {body}");
        }
    }
    let capabilities = SqliteProviderModelCapabilityRepository::new(services.database.pool().clone());
    assert!(capabilities.get(&provider.provider_id, "agnes-video-v2.0", "image_generation").await.unwrap().is_none());
    let stored = capabilities.get(&provider.provider_id, "agnes-video-v2.0", "video_generation").await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(stored.health.as_deref().unwrap()).unwrap()["status"], "healthy");
    services.shutdown_browser_platform().await.unwrap();
    services.companion_service.close_storage().await;
    services.database.close().await;
}
