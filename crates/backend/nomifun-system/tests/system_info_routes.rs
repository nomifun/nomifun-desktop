//! Black-box integration tests for system info and version check routes.
//!
//! System info tests verify the GET /api/system/info endpoint returns
//! correct platform/arch values and non-empty directory paths.
//!
//! Version check tests use `wiremock` to mock the GitHub Releases API
//! and verify the POST /api/system/check-update endpoint.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nomifun_db::init_database_memory;
use nomifun_system::{SystemRouterState, VersionCheckService, system_routes};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const TEST_KEY: [u8; 32] = [0x42; 32];

fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn build_state(db: &nomifun_db::Database, version_check_service: VersionCheckService) -> SystemRouterState {
    let http_client = test_http_client();
    common::build_system_state(
        db,
        TEST_KEY,
        http_client,
        version_check_service,
        None,
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

async fn setup() -> axum::Router {
    let db = init_database_memory().await.unwrap();
    let http_client = test_http_client();
    let vcs = VersionCheckService::new(http_client, "1.0.0".to_owned());
    let state = build_state(&db, vcs);
    system_routes(state)
}

async fn setup_with_mock(current_version: &str, mock_server: &MockServer) -> axum::Router {
    let db = init_database_memory().await.unwrap();
    let http_client = test_http_client();
    let vcs = VersionCheckService::with_api_base(http_client, current_version.to_owned(), mock_server.uri());
    let state = build_state(&db, vcs);
    system_routes(state)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

fn json_request(method_str: &str, uri: &str, mut body: serde_json::Value) -> Request<Body> {
    // Version requests must not depend on the runner's repository override.
    if let Some(object) = body.as_object_mut() {
        object.entry("repo").or_insert_with(|| json!("nomifun/nomifun-app"));
    }
    Request::builder()
        .method(method_str)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn make_github_release(tag: &str, draft: bool, prerelease: bool, assets: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "tag_name": tag,
        "name": format!("Release {tag}"),
        "body": "Release notes",
        "html_url": format!("https://github.com/nomifun/nomifun-app/releases/tag/{tag}"),
        "published_at": "2026-04-01T00:00:00Z",
        "prerelease": prerelease,
        "draft": draft,
        "assets": assets,
    })
}

fn make_github_asset(name: &str, size: u64) -> serde_json::Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/download/{name}"),
        "size": size,
        "content_type": "application/octet-stream",
    })
}

// ===========================================================================
// GET /api/system/info
// ===========================================================================

#[tokio::test]
async fn test_system_info_returns_all_fields() {
    let app = setup().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);

    let data = &json["data"];
    assert!(data["cache_dir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["work_dir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["log_dir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(
        data["storage_generation"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(data["platform"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["arch"].as_str().is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn test_system_info_platform_is_known() {
    let app = setup().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    let json = body_json(resp).await;
    let platform = json["data"]["platform"].as_str().unwrap();
    assert!(
        ["darwin", "win32", "linux"].contains(&platform),
        "unexpected platform: {platform}"
    );
}

#[tokio::test]
async fn test_system_info_arch_is_known() {
    let app = setup().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    let json = body_json(resp).await;
    let arch = json["data"]["arch"].as_str().unwrap();
    assert!(["x64", "arm64"].contains(&arch), "unexpected arch: {arch}");
}

#[tokio::test]
async fn test_system_info_snake_case_keys() {
    let app = setup().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    let json = body_json(resp).await;
    let data = &json["data"];
    assert!(data.get("cache_dir").is_some());
    assert!(data.get("work_dir").is_some());
    assert!(data.get("log_dir").is_some());
    assert!(data.get("storage_generation").is_some());
    assert!(data.get("cacheDir").is_none());
    assert!(data.get("workDir").is_none());
    assert!(data.get("logDir").is_none());
}

// ===========================================================================
// POST /api/system/check-update — with wiremock
// ===========================================================================

#[tokio::test]
async fn test_check_update_has_new_version() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release(
                "v2.0.0",
                false,
                false,
                vec![
                    make_github_asset("app-2.0.0-darwin-arm64.dmg", 80_000_000),
                    make_github_asset("app-2.0.0-linux-x64.deb", 60_000_000),
                ]
            ),
            make_github_release("v1.5.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["current_version"], "1.0.0");
    assert_eq!(json["data"]["update_available"], true);

    let latest = &json["data"]["latest"];
    assert_eq!(latest["tag_name"], "v2.0.0");
    assert_eq!(latest["version"], "2.0.0");
    assert!(!latest["assets"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_check_update_no_update_available() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v1.0.0", false, false, vec![]),
            make_github_release("v0.9.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["update_available"], false);
    assert!(json["data"].get("latest").is_none());
}

#[tokio::test]
async fn test_check_update_skips_draft() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v5.0.0", true, false, vec![]), // draft — skip
            make_github_release("v2.0.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["update_available"], true);
    assert_eq!(json["data"]["latest"]["version"], "2.0.0");
}

#[tokio::test]
async fn test_check_update_skips_prerelease_by_default() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v3.0.0-beta.1", false, true, vec![]),
            make_github_release("v4.0.0-beta.1", false, false, vec![]),
            make_github_release("v2.0.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request(
            "POST",
            "/api/system/check-update",
            json!({"include_prerelease": false}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["latest"]["version"], "2.0.0");
}

#[tokio::test]
async fn test_check_update_includes_prerelease_when_requested() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v3.0.0-beta.1", false, true, vec![]),
            make_github_release("v2.0.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request(
            "POST",
            "/api/system/check-update",
            json!({"include_prerelease": true}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["latest"]["version"], "3.0.0-beta.1");
}

#[tokio::test]
async fn test_check_update_recommended_asset_matches_platform() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([make_github_release(
            "v2.0.0",
            false,
            false,
            vec![
                make_github_asset("app-2.0.0-darwin-x64.dmg", 80_000_000),
                make_github_asset("app-2.0.0-win-x64.exe.sig", 100),
                make_github_asset("app-2.0.0-win-x64.exe", 50_000_000),
                make_github_asset("app-2.0.0-darwin-arm64.dmg", 80_000_000),
                make_github_asset("app-2.0.0-linux-amd64.deb", 60_000_000),
                make_github_asset("app-2.0.0-win-arm64.exe", 50_000_000),
                make_github_asset("app-2.0.0-linux-arm64.deb", 60_000_000),
            ]
        ),])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let recommended = &json["data"]["latest"]["recommended_asset"];
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Some("app-2.0.0-darwin-x64.dmg"),
        ("macos", "aarch64") => Some("app-2.0.0-darwin-arm64.dmg"),
        ("windows", "x86_64") => Some("app-2.0.0-win-x64.exe"),
        ("windows", "aarch64") => Some("app-2.0.0-win-arm64.exe"),
        ("linux", "x86_64") => Some("app-2.0.0-linux-amd64.deb"),
        ("linux", "aarch64") => Some("app-2.0.0-linux-arm64.deb"),
        _ => None,
    };
    assert_eq!(recommended["name"].as_str(), expected);
}

#[tokio::test]
async fn test_check_update_github_api_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(500).set_body_string("SYNTHETIC_UPSTREAM_PRIVATE_BODY"))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("500"));
    assert!(!json.to_string().contains("SYNTHETIC_UPSTREAM_PRIVATE_BODY"));
}

#[tokio::test]
async fn test_check_update_empty_releases() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["update_available"], false);
}

#[tokio::test]
async fn test_check_update_custom_repo() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/custom-org/custom-repo/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([make_github_release(
            "v3.0.0",
            false,
            false,
            vec![]
        ),])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request(
            "POST",
            "/api/system/check-update",
            json!({"repo": "custom-org/custom-repo"}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["update_available"], true);
    assert_eq!(json["data"]["latest"]["version"], "3.0.0");
}

#[tokio::test]
async fn test_check_update_invalid_tag_ignored() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("not-semver", false, false, vec![]),
            make_github_release("v2.0.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["latest"]["version"], "2.0.0");
}

#[tokio::test]
async fn test_check_update_response_format() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([make_github_release(
            "v2.0.0",
            false,
            false,
            vec![make_github_asset("app.dmg", 100_000),]
        ),])))
        .mount(&mock_server)
        .await;

    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app
        .oneshot(json_request("POST", "/api/system/check-update", json!({})))
        .await
        .unwrap();

    let json = body_json(resp).await;
    let latest = &json["data"]["latest"];

    // Verify snake_case serialization
    assert!(latest.get("tag_name").is_some());
    assert!(latest.get("html_url").is_some());
    assert!(latest.get("published_at").is_some());
    // Verify camelCase is NOT used
    assert!(latest.get("tagName").is_none());
    assert!(latest.get("htmlUrl").is_none());
}

#[tokio::test]
async fn test_check_update_rejects_invalid_local_inputs_before_network() {
    let mock_server = MockServer::start().await;
    let app = setup_with_mock("1.0.0", &mock_server).await;
    for repo in ["org", "/repo", "org/", "org/..", "org/repo/extra", "org/repo?x=1", "org/repo#fragment", "org/%2e%2e", r"org/repo\extra", "https://example.invalid/repo"] {
        let resp = app.clone().oneshot(json_request(
            "POST", "/api/system/check-update", json!({"repo": repo}),
        )).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{repo}");
    }
    let app = setup_with_mock("invalid-version", &mock_server).await;
    let resp = app.oneshot(json_request("POST", "/api/system/check-update", json!({}))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(mock_server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_check_update_follows_short_pages_but_stops_at_five() {
    let mock_server = MockServer::start().await;
    for page in 1..=5 {
        Mock::given(method("GET"))
            .and(path("/repos/nomifun/nomifun-app/releases"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", page.to_string()))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("Link", "<http://127.0.0.1:9/not-followed>; rel=\"next\"")
                .set_body_json(json!([make_github_release(&format!("v{}.0.0", page + 1), false, false, vec![])])))
            .expect(1)
            .mount(&mock_server)
            .await;
    }
    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app.oneshot(json_request("POST", "/api/system/check-update", json!({}))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["data"]["latest"]["version"], "6.0.0");
    assert_eq!(mock_server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn test_check_update_malformed_upstream_json_is_bad_gateway() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("not-json", "application/json"))
        .expect(1)
        .mount(&mock_server)
        .await;
    let app = setup_with_mock("1.0.0", &mock_server).await;
    let resp = app.oneshot(json_request("POST", "/api/system/check-update", json!({}))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(body_json(resp).await["code"], "BAD_GATEWAY");
}

#[tokio::test]
async fn test_check_update_preserves_shorter_client_timeout() {
    use std::time::Duration;

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/nomifun/nomifun-app/releases"))
        .respond_with(ResponseTemplate::new(200)
            .set_delay(Duration::from_millis(500))
            .set_body_json(json!([])))
        .mount(&mock_server)
        .await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let service = VersionCheckService::with_api_base(client, "1.0.0".to_owned(), mock_server.uri());
    let db = init_database_memory().await.unwrap();
    let app = system_routes(build_state(&db, service));
    let resp = tokio::time::timeout(
        Duration::from_secs(2),
        app.oneshot(json_request("POST", "/api/system/check-update", json!({}))),
    ).await.expect("client timeout must remain effective").unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let json = body_json(resp).await;
    assert_eq!(json["code"], "BAD_GATEWAY");
    assert!(!json["error"].as_str().unwrap().contains(&mock_server.uri()));
}
