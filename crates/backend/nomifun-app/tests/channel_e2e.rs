//! Channel integration E2E tests.
//!
//! Covers test-plan §1-5: plugin CRUD, pairing flow, user management,
//! session management, settings sync.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, get_with_token, json_with_token, setup_and_login};

const MISSING_CHANNEL_ID: &str = "0190f5fe-7c00-7a00-8000-00000000ff01";
const MISSING_CHANNEL_USER_ID: &str = "0190f5fe-7c00-7a00-8000-00000000ff02";

/// Seed a Telegram bot channel and return its stable business UUID.
async fn seed_telegram_channel(
    repo: &std::sync::Arc<dyn nomifun_db::IChannelRepository>,
) -> String {
    use nomifun_common::now_ms;
    use nomifun_db::models::NewChannelPluginRow;
    let row = repo
        .create_plugin(&NewChannelPluginRow {
        r#type: "telegram".into(),
        name: "Test Bot".into(),
        enabled: true,
        config: "{}".into(),
        status: None,
        last_connected: None,
        companion_id: None,
        bot_key: None,
        owner_domain: "companion".into(),
        group_access_mode: "allowlist".into(),
        created_at: now_ms(),
        updated_at: now_ms(),
        })
        .await
        .unwrap();
    row.channel_plugin_id
}

/// Seed a bot with an explicit group policy so settings tests can prove that
/// one bot's update never leaks into another bot on the same platform.
async fn seed_channel_with_group_access(
    repo: &std::sync::Arc<dyn nomifun_db::IChannelRepository>,
    name: &str,
    group_access_mode: &str,
) -> String {
    use nomifun_common::now_ms;
    use nomifun_db::models::NewChannelPluginRow;

    repo.create_plugin(&NewChannelPluginRow {
        r#type: "telegram".into(),
        name: name.into(),
        enabled: true,
        config: "{}".into(),
        status: None,
        last_connected: None,
        companion_id: None,
        bot_key: None,
        owner_domain: "companion".into(),
        group_access_mode: group_access_mode.into(),
        created_at: now_ms(),
        updated_at: now_ms(),
    })
    .await
    .unwrap()
    .channel_plugin_id
}

// ===========================================================================
// §1 Plugin management
// ===========================================================================

// PS-1: Get plugins when none exist
#[tokio::test]
async fn get_plugins_empty() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/channel/plugins", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    let data = json["data"].as_array().unwrap();
    assert!(data.is_empty());
}

// PS-3: Unauthenticated request returns 403
#[tokio::test]
async fn get_plugins_unauthenticated() {
    let (app, _services) = build_app().await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/channel/plugins")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// EP-3: Enable without any addressing info fails.
// `plugin_id` is optional since the per-companion multi-bot refactor (absent id +
// `plugin_type` is the create path), so the request now deserializes and the
// failure surfaces as success=false from the manager instead of HTTP 400.
#[tokio::test]
async fn enable_plugin_missing_plugin_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/enable",
        json!({ "config": {} }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let data = &json["data"];
    assert!(!data["success"].as_bool().unwrap());
    assert!(data["error"].as_str().unwrap().contains("plugin_type is required"));
}

// EP-4: Enable missing config fails
#[tokio::test]
async fn enable_plugin_missing_config() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/enable",
        json!({ "plugin_type": "telegram" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// EP-5: Enable invalid plugin type returns error in response body
#[tokio::test]
async fn enable_plugin_invalid_type() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/enable",
        json!({
            "plugin_type": "nonexistent",
            "config": { "credentials": { "token": "x" } }
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let data = &json["data"];
    assert!(!data["success"].as_bool().unwrap());
    assert!(data["error"].as_str().unwrap().contains("Invalid plugin type"));
}

// DP-3: Disable missing pluginId fails
#[tokio::test]
async fn disable_plugin_missing_plugin_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/channel/plugins/disable", json!({}), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// DP-2: Disable non-existent plugin returns success=false (not registered)
#[tokio::test]
async fn disable_plugin_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/disable",
        json!({ "plugin_id": MISSING_CHANNEL_ID }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // Plugin was never enabled, so disable returns success=false with error
    assert!(!json["data"]["success"].as_bool().unwrap());
    assert!(json["data"]["error"].as_str().is_some());
}

// TP-4: Test plugin missing pluginId fails
#[tokio::test]
async fn test_plugin_missing_plugin_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/test",
        json!({ "token": "xxx" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// TP-5: Test plugin missing token fails
#[tokio::test]
async fn test_plugin_missing_token() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/plugins/test",
        json!({ "plugin_type": "telegram" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// §2 Pairing management
// ===========================================================================

// PP-1: No pending pairings
#[tokio::test]
async fn get_pairings_empty() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/channel/pairings", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"].as_array().unwrap().is_empty());
}

// AP-6: Approve missing code fails
#[tokio::test]
async fn approve_pairing_missing_code() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/channel/pairings/approve", json!({}), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// AP-3: Approve non-existent code returns 404
#[tokio::test]
async fn approve_pairing_not_found() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/pairings/approve",
        json!({ "code": "000000" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// RP-3: Reject non-existent code returns 404
#[tokio::test]
async fn reject_pairing_not_found() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/pairings/reject",
        json!({ "code": "000000" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// §3 User management
// ===========================================================================

// GU-1: No authorized users
#[tokio::test]
async fn get_users_empty() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/channel/users", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"].as_array().unwrap().is_empty());
}

// RU-5: Revoke missing userId fails
#[tokio::test]
async fn revoke_user_missing_user_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/channel/users/revoke", json!({}), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// RU-4: Revoke non-existent user returns 404
#[tokio::test]
async fn revoke_user_not_found() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/users/revoke",
        json!({ "channel_user_id": MISSING_CHANNEL_USER_ID }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// §4 Session management
// ===========================================================================

// GS-1: No active sessions
#[tokio::test]
async fn get_sessions_empty() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/channel/sessions", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"].as_array().unwrap().is_empty());
}

// ===========================================================================
// §5 Settings sync
// ===========================================================================

// SS-1: Sync valid platform clears sessions
#[tokio::test]
async fn sync_settings_valid() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/settings/sync",
        json!({ "platform": "telegram" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"]["success"].as_bool().unwrap());
}

// SS-2: Sync missing platform fails deserialization
#[tokio::test]
async fn sync_settings_missing_platform() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/channel/settings/sync", json!({}), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// SS-3: Sync invalid platform fails validation
#[tokio::test]
async fn sync_settings_invalid_platform() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/settings/sync",
        json!({ "platform": "invalid" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// Full pairing → user → session lifecycle
// ===========================================================================

// GA-1/2/3: all three policies are accepted for a canonical bot id, projected
// by plugin status, and isolated from another bot on the same platform.
#[tokio::test]
async fn set_group_access_projects_each_mode_and_isolates_bots() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let repo: std::sync::Arc<dyn nomifun_db::IChannelRepository> = std::sync::Arc::new(
        nomifun_db::SqliteChannelRepository::new(services.database.pool().clone()),
    );
    let target_id = seed_channel_with_group_access(&repo, "Target Bot", "allowlist").await;
    let isolated_id = seed_channel_with_group_access(&repo, "Isolated Bot", "disabled").await;

    assert!(nomifun_common::validate_uuidv7(&target_id).is_ok());
    assert!(nomifun_common::validate_uuidv7(&isolated_id).is_ok());

    for mode in ["all_members", "allowlist", "disabled"] {
        let req = json_with_token(
            "POST",
            "/api/channel/settings/group-access",
            json!({
                "plugin_id": target_id,
                "group_access_mode": mode,
            }),
            &token,
            &csrf,
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "mode={mode}");
        let body = body_json(resp).await;
        assert_eq!(body["data"]["success"], true, "mode={mode}");

        let resp = app
            .clone()
            .oneshot(get_with_token("/api/channel/plugins", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let plugins = body["data"].as_array().unwrap();
        let target = plugins
            .iter()
            .find(|plugin| plugin["plugin_id"] == target_id)
            .expect("target bot must remain in status projection");
        let isolated = plugins
            .iter()
            .find(|plugin| plugin["plugin_id"] == isolated_id)
            .expect("isolated bot must remain in status projection");
        assert_eq!(target["group_access_mode"], mode, "mode={mode}");
        assert_eq!(isolated["group_access_mode"], "disabled", "mode={mode}");
    }
}

// GA-4: the settings endpoint is bot-addressed; implementation aliases such
// as a platform key are never accepted in place of the canonical UUIDv7 id.
#[tokio::test]
async fn set_group_access_rejects_noncanonical_plugin_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/channel/settings/group-access",
        json!({
            "plugin_id": "telegram",
            "group_access_mode": "all_members",
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Test the complete pairing flow using direct DB access for the parts
/// that normally come from IM platform (pairing request).
#[tokio::test]
async fn pairing_approve_creates_user() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create a pairing request directly via the pairing service
    let pool = services.database.pool().clone();
    let repo: std::sync::Arc<dyn nomifun_db::IChannelRepository> =
        std::sync::Arc::new(nomifun_db::SqliteChannelRepository::new(pool));
    let pairing_svc = nomifun_channel::pairing::PairingService::new(
        repo.clone(),
        services.event_bus.clone(),
        services.authoritative_user_id.as_ref(),
    );

    // The pairing request uses the persisted channel row's logical ID.
    let telegram_channel_id = seed_telegram_channel(&repo).await;

    let code = pairing_svc
        .request_pairing(
            "tg_user_42",
            "telegram",
            &telegram_channel_id,
            Some("Alice"),
        )
        .await
        .unwrap();

    // Verify pairing appears in pending list
    let req = get_with_token("/api/channel/pairings", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let pairings = json["data"].as_array().unwrap();
    assert_eq!(pairings.len(), 1);
    assert_eq!(pairings[0]["code"], code);
    assert_eq!(pairings[0]["platform_user_id"], "tg_user_42");
    assert_eq!(pairings[0]["platform_type"], "telegram");
    assert_eq!(pairings[0]["display_name"], "Alice");

    // Approve the pairing
    let req = json_with_token(
        "POST",
        "/api/channel/pairings/approve",
        json!({ "code": code }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["success"].as_bool().unwrap());

    // Verify user appears in authorized users
    let req = get_with_token("/api/channel/users", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let users = json["data"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["platform_user_id"], "tg_user_42");
    assert_eq!(users[0]["platform_type"], "telegram");
    assert_eq!(users[0]["display_name"], "Alice");
    let channel_user_id = users[0]["channel_user_id"]
        .as_str()
        .expect("authorized user exposes a stable business UUID");

    // Verify double-approve fails
    let req = json_with_token(
        "POST",
        "/api/channel/pairings/approve",
        json!({ "code": code }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Pairing should no longer appear in pending list
    let req = get_with_token("/api/channel/pairings", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());

    // Revoke the user
    let req = json_with_token(
        "POST",
        "/api/channel/users/revoke",
        json!({ "channel_user_id": channel_user_id }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["success"].as_bool().unwrap());

    // Verify user no longer in list
    let req = get_with_token("/api/channel/users", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());
}

/// Test pairing rejection flow.
#[tokio::test]
async fn pairing_reject_removes_from_pending() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create a pairing request
    let pool = services.database.pool().clone();
    let repo: std::sync::Arc<dyn nomifun_db::IChannelRepository> =
        std::sync::Arc::new(nomifun_db::SqliteChannelRepository::new(pool));
    let pairing_svc = nomifun_channel::pairing::PairingService::new(
        repo.clone(),
        services.event_bus.clone(),
        services.authoritative_user_id.as_ref(),
    );

    // Seed the bot channel first and use its stable business UUID.
    let telegram_channel_id = seed_telegram_channel(&repo).await;

    let code = pairing_svc
        .request_pairing("tg_user_99", "telegram", &telegram_channel_id, None)
        .await
        .unwrap();

    // Reject the pairing
    let req = json_with_token(
        "POST",
        "/api/channel/pairings/reject",
        json!({ "code": code }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["success"].as_bool().unwrap());

    // Verify pairing no longer in pending list
    let req = get_with_token("/api/channel/pairings", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());

    // Verify no user was created
    let req = get_with_token("/api/channel/users", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());

    // Verify reject same code again fails (already processed)
    let req = json_with_token(
        "POST",
        "/api/channel/pairings/reject",
        json!({ "code": code }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// Plugin enable/disable with real telegram factory
// ===========================================================================

/// Enable a Telegram plugin with mock-friendly config, verify status
/// appears in the plugin list, then disable it.
#[tokio::test]
async fn enable_disable_plugin_lifecycle() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Enable Telegram plugin (will fail connecting to real API, but
    // the error is captured in response, not an HTTP error)
    let req = json_with_token(
        "POST",
        "/api/channel/plugins/enable",
        json!({
            "plugin_type": "telegram",
            "config": {
                "credentials": { "token": "000000000:FAKE_TOKEN" },
                "config": { "mode": "polling" }
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The result may be success or failure depending on network —
    // either way, the plugin should appear in the list
    let req = get_with_token("/api/channel/plugins", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let plugins = json["data"].as_array().unwrap();
    assert_eq!(plugins.len(), 1);
    let telegram = plugins
        .iter()
        .find(|plugin| plugin["type"] == "telegram")
        .expect("telegram plugin should be present");
    let channel_id = telegram["plugin_id"]
        .as_str()
        .expect("persisted plugin exposes its stable business UUID");
    assert_eq!(telegram["type"], "telegram");
    assert_eq!(telegram["name"], "Telegram Bot");
    assert_eq!(telegram["enabled"], true);

    // Disable the plugin
    let req = json_with_token(
        "POST",
        "/api/channel/plugins/disable",
        json!({ "plugin_id": channel_id }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["success"].as_bool().unwrap());

    // Verify plugin is now disabled
    let req = get_with_token("/api/channel/plugins", &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let plugins = json["data"].as_array().unwrap();
    assert_eq!(plugins.len(), 1);
    let telegram = plugins
        .iter()
        .find(|plugin| plugin["type"] == "telegram")
        .expect("telegram plugin should remain listed after disable");
    assert!(!telegram["enabled"].as_bool().unwrap());
}
