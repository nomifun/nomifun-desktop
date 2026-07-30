//! Phase-3 model-failover config route tests (review #6/#12): GET defaults to
//! disabled, PUT round-trips the queue, and the path matches the frontend
//! `agentModelFailover` (`/api/agent/model-failover`).

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, get_with_token, json_with_token, setup_and_login};

/// The client-preference layer validates every provider referenced by
/// `agent.model_failover` against the providers table inside one writer
/// transaction (dangling references are a 409 Conflict). Queue fixtures must
/// therefore reference real rows.
async fn seed_provider(services: &nomifun_app::AppServices, provider_id: &str, model: &str) {
    nomifun_db::sqlx::query(
        "INSERT INTO providers \
         (provider_id, platform, name, base_url, api_key_encrypted, enabled, \
          capabilities, created_at, updated_at) \
         VALUES (?, 'openai', ?, 'https://example.invalid', 'encrypted', 1, '[]', 1, 1)",
    )
    .bind(provider_id)
    .bind(format!("Provider {provider_id}"))
    .execute(services.database.pool())
    .await
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO provider_models \
         (provider_id, model, enabled, sort_order, tasks, traits, params, source, created_at, updated_at) \
         VALUES (?, ?, 1, 0, '[]', '[]', '{}', 'inferred', 1, 1)",
    )
    .bind(provider_id)
    .bind(model)
    .execute(services.database.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn model_failover_get_defaults_to_disabled_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/agent/model-failover", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    // Unset pref → ModelFailoverConfig::default() = disabled.
    assert_eq!(json["data"]["enabled"], false);
    assert_eq!(json["data"]["queue"], json!([]));
}

#[tokio::test]
async fn model_failover_put_then_get_roundtrips_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_provider(&services, "0190f5fe-7c00-7a00-8000-000000000010", "m1").await;
    seed_provider(&services, "0190f5fe-7c00-7a00-8000-000000000011", "m2").await;

    let cfg = json!({
        "enabled": true,
        "queue": [
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000010", "model": "m1"},
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000011", "model": "m2"}
        ],
        "max_switches": 3,
        "stamp_unhealthy": false
    });

    let req = json_with_token("PUT", "/api/agent/model-failover", cfg.clone(), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // PUT echoes the saved config back.
    let json = body_json(resp).await;
    assert_eq!(json["data"]["enabled"], true);
    assert_eq!(json["data"]["max_switches"], 3);
    assert_eq!(json["data"]["queue"][1]["provider_id"], "0190f5fe-7c00-7a00-8000-000000000011");

    let resp = app
        .oneshot(get_with_token("/api/agent/model-failover", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["enabled"], true);
    assert_eq!(json["data"]["stamp_unhealthy"], false);
    assert_eq!(json["data"]["queue"][0]["model"], "m1");
    assert_eq!(json["data"]["queue"][1]["model"], "m2");
}
