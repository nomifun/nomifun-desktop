mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{
    body_json, build_app, build_app_with_mock_agents, delete_with_token, get_with_token, json_with_token,
    setup_and_login,
};

fn two_agent_body() -> serde_json::Value {
    json!({
        "name": "Alpha",
        "agents": [
            { "name": "Lead", "role": "lead", "backend": "acp", "model": "claude" },
            { "name": "Worker", "role": "teammate", "backend": "acp", "model": "claude" }
        ]
    })
}

async fn create_team(app: &mut axum::Router, token: &str, csrf: &str) -> serde_json::Value {
    let req = json_with_token("POST", "/api/teams", two_agent_body(), token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    json["data"].clone()
}

// ===========================================================================
// §1 Team CRUD (TC-*, TL-*, TG-*, TD-*, TR-*)
// ===========================================================================

// TC-1: Create team with multiple agents
#[tokio::test]
async fn tc1_create_team_with_multiple_agents() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    assert_eq!(data["name"], "Alpha");
    assert_eq!(data["agents"].as_array().unwrap().len(), 2);
    assert_eq!(data["agents"][0]["role"], "lead");
    assert_eq!(data["agents"][1]["role"], "teammate");
    assert!(data["lead_agent_id"].is_string());
    assert_eq!(data["lead_agent_id"], data["agents"][0]["slot_id"]);
}

// TC-2: Create single agent team
#[tokio::test]
async fn tc2_create_single_agent_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "Solo",
        "agents": [{ "name": "Lead", "role": "lead", "backend": "acp", "model": "claude" }]
    });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["agents"].as_array().unwrap().len(), 1);
}

// TC-3: Each agent has a conversation
#[tokio::test]
async fn tc3_each_agent_has_conversation_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    for agent in data["agents"].as_array().unwrap() {
        assert!(agent["conversation_id"].is_i64());
        assert!(agent["conversation_id"].as_i64().unwrap() > 0);
    }
    assert_ne!(
        data["agents"][0]["conversation_id"],
        data["agents"][1]["conversation_id"]
    );
}

// TC-4: First agent defaults to lead
#[tokio::test]
async fn tc4_first_agent_is_lead() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "T",
        "agents": [
            { "name": "A", "role": "teammate", "backend": "acp", "model": "claude" },
            { "name": "B", "role": "teammate", "backend": "acp", "model": "claude" }
        ]
    });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["agents"][0]["role"], "lead");
    assert_eq!(json["data"]["lead_agent_id"], json["data"]["agents"][0]["slot_id"]);
}

// TC-5: Empty agents returns 400
#[tokio::test]
async fn tc5_empty_agents_returns_error() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({ "name": "Empty", "agents": [] });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// TC-6: Missing name returns 400
#[tokio::test]
async fn tc6_missing_name_returns_error() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({ "agents": [{ "name": "L", "role": "lead", "backend": "acp", "model": "c" }] });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tc6b_workspace_with_edge_whitespace_segment_returns_specific_code() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "Alpha",
        "workspace": "/Users/developer/Documents/Archive ",
        "agents": [{ "name": "Lead", "role": "lead", "backend": "acp", "model": "claude" }]
    });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert_eq!(json["code"], "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("begins or ends with whitespace")
    );
}

// TC-7: Unauthenticated returns 403
#[tokio::test]
async fn tc7_unauthenticated_returns_403() {
    let (app, _services) = build_app().await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/teams")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// TL-1: Empty team list
#[tokio::test]
async fn tl1_empty_team_list() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/teams", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());
}

// TL-2: List multiple teams
#[tokio::test]
async fn tl2_list_multiple_teams() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    create_team(&mut app, &token, &csrf).await;

    let body = json!({
        "name": "Beta",
        "agents": [{ "name": "Lead", "role": "lead", "backend": "acp", "model": "claude" }]
    });
    let req = json_with_token("POST", "/api/teams", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = get_with_token("/api/teams", &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
}

// TL-3: Each team contains full agents info
#[tokio::test]
async fn tl3_teams_contain_full_agent_info() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    create_team(&mut app, &token, &csrf).await;

    let req = get_with_token("/api/teams", &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let teams = json["data"].as_array().unwrap();
    let agent = &teams[0]["agents"][0];
    assert!(agent["slot_id"].is_string());
    assert!(agent["name"].is_string());
    assert!(agent["role"].is_string());
    assert!(agent["conversation_id"].is_i64());
    assert!(agent["backend"].is_string());
    assert!(agent["model"].is_string());
}

// TG-1: Get existing team
#[tokio::test]
async fn tg1_get_existing_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["id"], team_id);
    assert_eq!(json["data"]["name"], "Alpha");
}

// TG-2: Get nonexistent team returns 404
#[tokio::test]
async fn tg2_get_nonexistent_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/teams/nonexistent", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// TD-1: Delete existing team
#[tokio::test]
async fn td1_delete_existing_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// TD-2: Delete then list confirms removal
#[tokio::test]
async fn td2_delete_then_list_empty() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token("/api/teams", &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());
}

// TD-6: Delete nonexistent team returns 404
#[tokio::test]
async fn td6_delete_nonexistent_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = delete_with_token("/api/teams/nonexistent", &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// TR-1: Rename existing team
#[tokio::test]
async fn tr1_rename_existing_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/name"),
        json!({ "name": "New Name" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// TR-2: Rename then get confirms new name
#[tokio::test]
async fn tr2_rename_then_get_confirms_new_name() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/name"),
        json!({ "name": "New Name" }),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "New Name");
}

// TR-4: Rename nonexistent team returns 404
#[tokio::test]
async fn tr4_rename_nonexistent_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PATCH",
        "/api/teams/nonexistent/name",
        json!({ "name": "X" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// §2 Agent Management (AA-*, AR-*, AN-*)
// ===========================================================================

// AA-1: Add agent to team
#[tokio::test]
async fn aa1_add_agent_to_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let body = json!({
        "name": "New Agent",
        "role": "teammate",
        "backend": "acp",
        "model": "claude"
    });
    let req = json_with_token("POST", &format!("/api/teams/{team_id}/agents"), body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "New Agent");
    assert!(json["data"]["conversation_id"].is_i64());
}

// AA-2: After adding, agent count increases
#[tokio::test]
async fn aa2_add_agent_increases_count() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let body = json!({ "name": "X", "role": "teammate", "backend": "acp", "model": "claude" });
    let req = json_with_token("POST", &format!("/api/teams/{team_id}/agents"), body, &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["agents"].as_array().unwrap().len(), 3);
}

// AA-4: Add agent to nonexistent team returns 404
#[tokio::test]
async fn aa4_add_agent_nonexistent_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({ "name": "X", "role": "teammate", "backend": "acp", "model": "claude" });
    let req = json_with_token("POST", "/api/teams/nonexistent/agents", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// AA-5: Missing required fields returns 400
#[tokio::test]
async fn aa5_add_agent_missing_fields() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let body = json!({ "role": "teammate", "backend": "acp" });
    let req = json_with_token("POST", &format!("/api/teams/{team_id}/agents"), body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// AR-1: Remove agent from team
#[tokio::test]
async fn ar1_remove_agent_from_team() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    let slot_id = data["agents"][1]["slot_id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}/agents/{slot_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// AR-2: After removal, agent not in team
#[tokio::test]
async fn ar2_after_removal_agent_gone() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    let slot_id = data["agents"][1]["slot_id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}/agents/{slot_id}"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let agents = json["data"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert!(agents.iter().all(|a| a["slot_id"] != slot_id));
}

// AR-4: Remove nonexistent agent returns 404
#[tokio::test]
async fn ar4_remove_nonexistent_agent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}/agents/nonexistent"), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// AN-1: Rename agent
#[tokio::test]
async fn an1_rename_agent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    let slot_id = data["agents"][1]["slot_id"].as_str().unwrap();

    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/agents/{slot_id}/name"),
        json!({ "name": "Senior Worker" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// AN-2: Rename then get confirms new name
#[tokio::test]
async fn an2_rename_then_get_confirms_name() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    let slot_id = data["agents"][1]["slot_id"].as_str().unwrap();

    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/agents/{slot_id}/name"),
        json!({ "name": "Senior Worker" }),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let agents = json["data"]["agents"].as_array().unwrap();
    let agent = agents.iter().find(|a| a["slot_id"] == slot_id).unwrap();
    assert_eq!(agent["name"], "Senior Worker");
}

// AN-3: Rename nonexistent agent returns 404
#[tokio::test]
async fn an3_rename_nonexistent_agent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/agents/nonexistent/name"),
        json!({ "name": "X" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// §3 Session Management (ES-*, SS-*)
// ===========================================================================

// ES-1: Ensure session
#[tokio::test]
async fn es1_ensure_session() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ES-2: Ensure session is idempotent
#[tokio::test]
async fn es2_ensure_session_idempotent() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ES-3: Ensure session for nonexistent team returns 404
#[tokio::test]
async fn es3_ensure_session_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/teams/nonexistent/session", json!({}), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// SS-1: Stop session
#[tokio::test]
async fn ss1_stop_session() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}/session"), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// SS-3: Stop session without active is noop
#[tokio::test]
async fn ss3_stop_session_noop() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = delete_with_token(&format!("/api/teams/{team_id}/session"), &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ===========================================================================
// §4 Message sending (SM-*, SA-*)
// ===========================================================================

// SM-1: Send message with active session
#[tokio::test]
async fn sm1_send_message_with_session() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    // Start session first
    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/messages"),
        json!({ "content": "Hello team" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// SM-4: Send message without session returns 404
#[tokio::test]
async fn sm4_send_message_no_session() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/teams/nonexistent/messages",
        json!({ "content": "Hello" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// SM-5: Missing content returns 400
#[tokio::test]
async fn sm5_send_message_missing_content() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/messages"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// SA-1: Send message to specific agent
#[tokio::test]
async fn sa1_send_message_to_agent() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    let slot_id = data["agents"][1]["slot_id"].as_str().unwrap();

    // Start session first
    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/agents/{slot_id}/messages"),
        json!({ "content": "Do this" }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ===========================================================================
// §5 Full lifecycle
// ===========================================================================

// Full CRUD lifecycle
#[tokio::test]
async fn full_team_lifecycle() {
    let (mut app, services) = build_app_with_mock_agents().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create
    let data = create_team(&mut app, &token, &csrf).await;
    let team_id = data["id"].as_str().unwrap();
    assert_eq!(data["agents"].as_array().unwrap().len(), 2);

    // Add agent
    let body = json!({ "name": "Helper", "role": "teammate", "backend": "acp", "model": "claude" });
    let req = json_with_token("POST", &format!("/api/teams/{team_id}/agents"), body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let added = body_json(resp).await;
    let new_slot = added["data"]["slot_id"].as_str().unwrap().to_owned();

    // Verify 3 agents
    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["agents"].as_array().unwrap().len(), 3);

    // Rename team
    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/name"),
        json!({ "name": "Renamed" }),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    // Rename agent
    let req = json_with_token(
        "PATCH",
        &format!("/api/teams/{team_id}/agents/{new_slot}/name"),
        json!({ "name": "Senior Helper" }),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    // Ensure session
    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/session"),
        json!({}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    // Send message
    let req = json_with_token(
        "POST",
        &format!("/api/teams/{team_id}/messages"),
        json!({ "content": "Hello" }),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    // Stop session
    let req = delete_with_token(&format!("/api/teams/{team_id}/session"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    // Remove added agent
    let req = delete_with_token(&format!("/api/teams/{team_id}/agents/{new_slot}"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    // Verify 2 agents remain
    let req = get_with_token(&format!("/api/teams/{team_id}"), &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["agents"].as_array().unwrap().len(), 2);
    assert_eq!(json["data"]["name"], "Renamed");

    // Delete team
    let req = delete_with_token(&format!("/api/teams/{team_id}"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    // Verify empty
    let req = get_with_token("/api/teams", &token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().is_empty());
}
