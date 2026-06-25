//! E2E tests for the Desktop Gateway MCP server (`nomifun-gateway`).
//!
//! The gateway is not part of the axum app router — it is a separate
//! localhost HTTP server started by `AppServices::from_config` and
//! deps-wired by `create_router`. These tests exercise the full stack the
//! way the stdio bridge does: authenticated `POST /tool` against the real
//! port, with real services (memory db) behind it.

mod common;

use common::build_app;
use serde_json::{Value, json};

struct Gateway {
    port: u16,
    token: String,
    client: reqwest::Client,
}

impl Gateway {
    fn from_services(services: &nomifun_app::AppServices) -> Self {
        let cfg = services
            .gateway_mcp_config
            .as_ref()
            .expect("gateway MCP server must start in tests");
        Self {
            port: cfg.port,
            token: cfg.token.clone(),
            client: reqwest::Client::new(),
        }
    }

    async fn call(&self, tool: &str, caller_conv: &str, user_id: &str, args: Value) -> Value {
        let resp = self
            .client
            .post(format!("http://127.0.0.1:{}/tool", self.port))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&json!({
                "tool": tool,
                "args": args,
                "conversation_id": caller_conv,
                "user_id": user_id,
            }))
            .send()
            .await
            .expect("gateway reachable");
        resp.json().await.expect("json body")
    }
}

/// Seed a user + one conversation directly (the gateway scopes everything by
/// user id; HTTP signup/login is irrelevant to what we exercise here).
///
/// `conv_id` is an integer: after the single-track refactor `conversations.id`
/// is an `INTEGER` autoincrement column, so the seed must insert a numeric id
/// (string ids hit `datatype mismatch`).
async fn seed_user_and_conversation(services: &nomifun_app::AppServices, user_id: &str, conv_id: i64) {
    seed_user_and_conversation_with_extra(services, user_id, conv_id, "{}").await;
}

/// Same as [`seed_user_and_conversation`] but with a caller-provided `extra`
/// JSON (e.g. a channel master-agent session carrying `companionId`).
async fn seed_user_and_conversation_with_extra(
    services: &nomifun_app::AppServices,
    user_id: &str,
    conv_id: i64,
    extra: &str,
) {
    sqlx::query("INSERT OR IGNORE INTO users (id, username, password_hash, created_at, updated_at) VALUES (?, ?, 'hash', 0, 0)")
        .bind(user_id)
        .bind(format!("user-{user_id}"))
        .execute(services.database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, user_id, name, type, extra, created_at, updated_at) \
         VALUES (?, ?, ?, 'nomi', ?, 0, 0)",
    )
    .bind(conv_id)
    .bind(user_id)
    .bind(format!("Conv {conv_id}"))
    .bind(extra)
    .execute(services.database.pool())
    .await
    .unwrap();
}

/// Seed an enabled provider so tools that resolve a default model (e.g.
/// `nomi_cron_create` auto-filling a model-less nomi conversation) can
/// complete their fallback chain.
async fn seed_provider(services: &nomifun_app::AppServices, provider_id: &str, model: &str) {
    sqlx::query(
        "INSERT OR IGNORE INTO providers \
         (id, platform, name, base_url, api_key_encrypted, models, enabled, created_at, updated_at) \
         VALUES (?, 'openai', ?, 'http://127.0.0.1:1', 'k', ?, 1, 0, 0)",
    )
    .bind(provider_id)
    .bind(format!("Provider {provider_id}"))
    .bind(format!("[\"{model}\"]"))
    .execute(services.database.pool())
    .await
    .unwrap();
}

fn result_of(body: &Value) -> &Value {
    body.get("result")
        .unwrap_or_else(|| panic!("expected result, got {body}"))
}

fn error_of(body: &Value) -> &str {
    body.get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected error, got {body}"))
}

// ── auth ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn gw_unauthenticated_tool_call_is_rejected() {
    let (_app, services) = build_app().await;
    let cfg = services.gateway_mcp_config.as_ref().unwrap();
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/tool", cfg.port))
        .json(&json!({"tool": "nomi_list_conversations", "args": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn gw_unknown_tool_returns_error() {
    let (_app, services) = build_app().await;
    let gw = Gateway::from_services(&services);
    let body = gw.call("nomi_explode_desktop", "", "u", json!({})).await;
    assert!(error_of(&body).contains("Unknown tool"));
}

// ── conversations ────────────────────────────────────────────────────

#[tokio::test]
async fn gw_list_conversations_returns_rows_with_runtime_state() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_gw", 1).await;
    seed_user_and_conversation(&services, "user_gw", 2).await;
    let gw = Gateway::from_services(&services);

    let body = gw.call("nomi_list_conversations", "1", "user_gw", json!({})).await;
    let result = result_of(&body);
    assert_eq!(result["total"], json!(2));
    let convs = result["conversations"].as_array().unwrap();
    assert_eq!(convs.len(), 2);
    for conv in convs {
        assert_eq!(conv["runtime_state"], json!("idle"));
    }
    let self_marked: Vec<bool> = convs.iter().map(|c| c["is_self"].as_bool().unwrap()).collect();
    assert!(self_marked.contains(&true), "caller conversation flagged is_self");
}

#[tokio::test]
async fn gw_conversation_tools_require_user_identity() {
    let (_app, services) = build_app().await;
    let gw = Gateway::from_services(&services);
    let body = gw.call("nomi_list_conversations", "1", "", json!({})).await;
    assert!(error_of(&body).contains("user identity"));
}

#[tokio::test]
async fn gw_list_conversations_excludes_companion_sessions() {
    let (_app, services) = build_app().await;
    // A companion (work-partner) single session…
    seed_user_and_conversation_with_extra(
        &services,
        "user_companion",
        1,
        r#"{"companionSession":true,"companionId":"companion_42"}"#,
    )
    .await;
    // …and a plain session with no binding.
    seed_user_and_conversation(&services, "user_companion", 2).await;
    let gw = Gateway::from_services(&services);

    let body = gw.call("nomi_list_conversations", "", "user_companion", json!({})).await;
    let result = result_of(&body);
    let convs = result["conversations"].as_array().unwrap().clone();

    // Product rule: a companion's own work-partner session is not part of the
    // session list — it is filtered out of BOTH the page and the total.
    assert_eq!(convs.len(), 1, "companion session excluded, got {convs:?}");
    assert_eq!(result["total"], json!(1));
    assert!(
        convs.iter().all(|c| c["id"] != json!(1)),
        "companion-bound conversation must be absent"
    );

    // The surviving entry is the plain session, with no companion binding.
    // `conversations.id` is an INTEGER now → the `id` field is a JSON number.
    let plain = &convs[0];
    assert_eq!(plain["id"], json!(2));
    assert_eq!(plain["companion_id"], json!(null), "no binding → null, got {plain}");
    assert_eq!(plain["is_companion_companion"], json!(false));
}

#[tokio::test]
async fn gw_send_to_own_conversation_is_refused() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_gw", 1).await;
    let gw = Gateway::from_services(&services);
    let body = gw
        .call(
            "nomi_send_to_conversation",
            "1",
            "user_gw",
            json!({"conversation_id": 1, "content": "hi me"}),
        )
        .await;
    assert!(error_of(&body).contains("self_injection_forbidden"));
}

#[tokio::test]
async fn gw_delete_own_conversation_is_refused_but_other_succeeds() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_gw", 1).await;
    seed_user_and_conversation(&services, "user_gw", 2).await;
    let gw = Gateway::from_services(&services);

    let body = gw
        .call(
            "nomi_delete_conversation",
            "1",
            "user_gw",
            json!({"conversation_id": 1, "confirm": true}),
        )
        .await;
    assert!(error_of(&body).contains("self_deletion_forbidden"));

    let body = gw
        .call(
            "nomi_delete_conversation",
            "1",
            "user_gw",
            json!({"conversation_id": 2, "confirm": true}),
        )
        .await;
    // `delete` echoes the stringified i64 id it parsed.
    assert_eq!(result_of(&body)["deleted"], json!("2"));

    let body = gw.call("nomi_list_conversations", "1", "user_gw", json!({})).await;
    assert_eq!(result_of(&body)["total"], json!(1));
}

#[tokio::test]
async fn gw_conversation_status_reports_idle_and_messages() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_gw", 1).await;
    let gw = Gateway::from_services(&services);
    let body = gw
        .call(
            "nomi_conversation_status",
            "",
            "user_gw",
            json!({"conversation_id": 1}),
        )
        .await;
    let result = result_of(&body);
    assert_eq!(result["id"], json!(1));
    assert_eq!(result["runtime"]["state"], json!("idle"));
    assert!(result.get("recent_messages").is_some());
}

#[tokio::test]
async fn gw_user_isolation_hides_other_users_conversations() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_a", 1).await;
    seed_user_and_conversation(&services, "user_b", 2).await;
    let gw = Gateway::from_services(&services);

    // user_b cannot read or delete user_a's conversation.
    let body = gw
        .call(
            "nomi_conversation_status",
            "",
            "user_b",
            json!({"conversation_id": 1}),
        )
        .await;
    assert!(error_of(&body).contains("not found"), "got {body}");
    let body = gw
        .call(
            "nomi_delete_conversation",
            "",
            "user_b",
            json!({"conversation_id": 1, "confirm": true}),
        )
        .await;
    assert!(error_of(&body).contains("not found"), "got {body}");
}

// ── cron ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn gw_cron_create_list_update_delete_roundtrip() {
    let (_app, services) = build_app().await;
    seed_user_and_conversation(&services, "user_gw", 1).await;
    // The conversation is seeded without a model: cron creation must resolve
    // one via the fallback chain (companion profile → first enabled provider), so a
    // provider has to exist — a model-less desktop is refused with guidance.
    seed_provider(&services, "prov_e2e", "test-model").await;
    let gw = Gateway::from_services(&services);

    let body = gw
        .call(
            "nomi_cron_create",
            "1",
            "user_gw",
            json!({"name": "晨报", "cron": "0 9 * * *", "description": "每天 9 点", "message": "写晨报"}),
        )
        .await;
    // The create result became an object when the duplicate-guard and the
    // model-fallback note were introduced: {"message": ..., "model_note": ...}.
    let created = result_of(&body);
    let created_msg = created["message"].as_str().unwrap().to_owned();
    assert!(created_msg.contains("晨报"), "got {created_msg}");
    // The seeded conversation had no model — the fallback chain must have
    // auto-selected the only enabled provider and said so.
    let model_note = created["model_note"].as_str().unwrap_or_default();
    assert!(model_note.contains("prov_e2e") || model_note.contains("test-model"), "got model_note {model_note}");

    let body = gw.call("nomi_cron_list", "1", "user_gw", json!({})).await;
    let jobs = result_of(&body).as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    let job_id = jobs[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(jobs[0]["name"], json!("晨报"));

    let body = gw
        .call(
            "nomi_cron_update",
            "1",
            "user_gw",
            json!({"job_id": job_id, "name": "晚报", "cron": "0 21 * * *", "message": "写晚报"}),
        )
        .await;
    assert!(result_of(&body).as_str().unwrap().contains("晚报"));

    let body = gw
        .call("nomi_cron_delete", "1", "user_gw", json!({"job_id": job_id, "confirm": true}))
        .await;
    assert!(result_of(&body).as_str().unwrap().contains("Deleted"));

    let body = gw.call("nomi_cron_list", "1", "user_gw", json!({})).await;
    assert!(result_of(&body).as_array().unwrap().is_empty());
}

// ── global memory ────────────────────────────────────────────────────

#[tokio::test]
async fn gw_memory_save_list_update_delete_roundtrip() {
    let (_app, services) = build_app().await;
    let gw = Gateway::from_services(&services);

    let body = gw
        .call(
            "nomi_memory_save",
            "",
            "user_gw",
            json!({"content": "主人喜欢深色主题", "kind": "preference", "tags": ["ui"]}),
        )
        .await;
    let memory_id = result_of(&body)["id"].as_str().unwrap().to_owned();

    let body = gw
        .call("nomi_memory_list", "", "user_gw", json!({"query": "深色主题"}))
        .await;
    let memories = result_of(&body).as_array().unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["kind"], json!("preference"));

    let body = gw
        .call("nomi_memory_update", "", "user_gw", json!({"id": memory_id, "pinned": true}))
        .await;
    assert!(result_of(&body).as_str().unwrap().contains("updated"));

    let body = gw
        .call("nomi_memory_delete", "", "user_gw", json!({"id": memory_id, "confirm": true}))
        .await;
    assert!(result_of(&body).as_str().unwrap().contains("deleted"));

    let body = gw
        .call("nomi_memory_list", "", "user_gw", json!({"query": "深色主题"}))
        .await;
    assert!(result_of(&body).as_array().unwrap().is_empty());
}

#[tokio::test]
async fn gw_memory_update_with_no_fields_is_rejected() {
    let (_app, services) = build_app().await;
    let gw = Gateway::from_services(&services);
    let body = gw
        .call("nomi_memory_update", "", "user_gw", json!({"id": "mem_x"}))
        .await;
    assert!(error_of(&body).contains("nothing to update"));
}

// ── requirements ─────────────────────────────────────────────────────

#[tokio::test]
async fn gw_requirement_create_list_update_delete_roundtrip() {
    let (_app, services) = build_app().await;
    let gw = Gateway::from_services(&services);

    let body = gw
        .call(
            "nomi_requirement_create",
            "",
            "user_gw",
            json!({"title": "修复登录页样式", "content": "按钮溢出", "tag": "前端"}),
        )
        .await;
    let req_id = result_of(&body)["id"].as_i64().unwrap();
    assert_eq!(result_of(&body)["created_by"], json!("agent"));

    let body = gw
        .call("nomi_requirement_list", "", "user_gw", json!({"tag": "前端"}))
        .await;
    let items = result_of(&body)["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 1);

    let body = gw
        .call(
            "nomi_requirement_update",
            "",
            "user_gw",
            json!({"id": req_id, "title": "修复登录页按钮溢出"}),
        )
        .await;
    assert_eq!(result_of(&body)["title"], json!("修复登录页按钮溢出"));

    let body = gw
        .call("nomi_requirement_delete", "", "user_gw", json!({"id": req_id, "confirm": true}))
        .await;
    assert!(result_of(&body).as_str().unwrap().contains("deleted"));
}
