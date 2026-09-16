//! Removed Session UI routes cannot consume or rewrite historical binding data.
use super::*;

#[tokio::test]
async fn retired_session_ui_routes_preserve_historical_binding_data() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    let provider = post(&router,"/api/providers",json!({
        "platform":"stepfun-plan","name":"Unused view-removal fixture","base_url":format!("{}/step_plan/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}
    })).await;
    let preset = post(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({"display_name":"Retired view binding", "reuse_existing":false,
        "model":{"provider_id":provider["provider_id"],"model":"step-3.7-flash"}}),
    )
    .await;
    let id = preset["preset"]["preset_id"].as_str().unwrap();
    let original: String =
        sqlx::query_scalar("SELECT ui_binding_json FROM nomi_agent_presets WHERE preset_id = ?")
            .bind(id)
            .fetch_one(services.database.pool())
            .await
            .unwrap();
    for (method, path, body) in [
        (
            "GET",
            "/api/agent-catalog/ui/agent-session".to_owned(),
            Value::Null,
        ),
        (
            "GET",
            format!("/api/agent-presets/{id}/ui-binding"),
            Value::Null,
        ),
        (
            "PUT",
            format!("/api/agent-presets/{id}/ui-binding"),
            json!({"selection":null,"expected_binding_version":0}),
        ),
        (
            "GET",
            format!("/api/agent-sessions/{}/ui-binding", uuid::Uuid::now_v7()),
            Value::Null,
        ),
        (
            "POST",
            "/api/plugins/drafts/from-template/agent-session-view".to_owned(),
            Value::Null,
        ),
    ] {
        let (status, _) = request(&router, method, &path, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let after: String =
        sqlx::query_scalar("SELECT ui_binding_json FROM nomi_agent_presets WHERE preset_id = ?")
            .bind(id)
            .fetch_one(services.database.pool())
            .await
            .unwrap();
    assert_eq!(after, original);
    assert!(get(&router, &format!("/api/agent-presets/{id}/editor")).await["draft"].is_object());
}
