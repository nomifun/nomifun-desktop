//! Opt-in canonical Session smoke using the explicitly selected Agnes model.
//! Supply the key on stdin; it is never a command argument or a source fixture.
//! All provider state is encrypted in an isolated in-memory database.
use std::io::BufRead as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use axum::body::Body;
use axum::http::{Method, Request};
use http_body_util::BodyExt;
use nomifun_app::compatibility::{AppServices, build_module_states, create_router_with_states};
use nomifun_app::{AppConfig, AuthPolicy};
use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability, SqliteProviderRepository};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zeroize::Zeroizing;

const TRUST: &str = "isolated-live-context-clear";
const MODEL: &str = "agnes-3.0-flash";

async fn call(router: &axum::Router, path: &str, body: Value) -> Value {
    let response = tokio::time::timeout(Duration::from_secs(30), router.clone().oneshot(Request::builder()
        .method(Method::POST).uri(path).header("x-nomi-local-trust", TRUST)
        .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()))
        .await.expect("local API deadline").expect("local API response");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("JSON response");
    assert!(status.is_success(), "local request failed: status={status}, code={}", value["code"]);
    value["data"].clone()
}

async fn send(router: &axum::Router, pool: &nomifun_db::SqlitePool, session: &str, content: &str) -> String {
    let floor: i64 = nomifun_db::sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=?")
        .bind(session).fetch_one(pool).await.unwrap();
    let accepted = call(router, &format!("/api/agent-sessions/{session}/turns"),
        json!({"idempotency_key":Uuid::now_v7().to_string(),"input":{"content":content}})).await;
    let operation = accepted["operation_id"].as_str().expect("accepted canonical Turn");
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let state: String = nomifun_db::sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session).bind(operation).fetch_one(pool).await.unwrap();
        if state == "completed" { break; }
        assert!(!matches!(state.as_str(), "failed" | "cancelled" | "paused"), "real model Turn ended in {state}");
        assert!(Instant::now() < deadline, "real model Turn exceeded deadline");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // This is a display assertion on the exact new history window, never a
    // source used to construct Runtime context or prove tool effects.
    let rows: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT projection_json FROM agent_messages WHERE session_id=? AND first_seq>? AND presentation_intent='message' ORDER BY first_seq",
    ).bind(session).bind(floor).fetch_all(pool).await.unwrap();
    rows.iter().filter_map(|row| serde_json::from_str::<Value>(row).ok())
        .filter(|row| row["state"] != "accepted")
        .filter_map(|row| row["content"].as_str().map(str::to_owned)).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
#[ignore = "billable Agnes model calls; requires the user-provided credential on stdin"]
async fn agnes_3_flash_forgets_cleared_context_and_keeps_visible_history() {
    let mut key = Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut key).expect("credential on stdin");
    assert!(!key.trim().is_empty(), "credential required on stdin");
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    std::fs::create_dir_all(root_path.join("data")).unwrap();
    std::fs::create_dir_all(root_path.join("work")).unwrap();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &AppConfig {
        data_dir:root_path.join("data"), work_dir:root_path.join("work"),
        auth_policy:AuthPolicy::TrustLocalToken, local_trust_secret:Some(Arc::from(TRUST)), ..Default::default()
    }).await.unwrap();
    let pool = services.database.pool().clone();
    let provider_id = Uuid::now_v7().to_string();
    let credential = Zeroizing::new(json!({"api_keys":[key.trim()]}).to_string());
    let encrypted = nomifun_common::encrypt_string(&credential, &services.encryption_key).unwrap();
    SqliteProviderRepository::new(pool.clone()).create(CreateProviderParams {
        provider_id:Some(&provider_id), platform:"agnes", name:"Isolated context clear smoke",
        base_url:"https://apihub.agnes-ai.com/v1", auth_scheme:"bearer", credentials_encrypted:&encrypted,
        enabled:true, bedrock_config:None, sort_order:None,
    }, &NewProviderModel {
        model:MODEL, enabled:true, sort_order:0, description:None,
        capabilities:&[NewProviderModelCapability {
            task:"chat", traits:"[]", protocol:"openai.chat_text", connection_role:"default",
            provider_params:"{}", ..Default::default()
        }],
    }, &[]).await.unwrap();
    let (states, _components) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let model = json!({"provider_id":provider_id,"model":MODEL});
    let preset = call(&router,"/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Context clear smoke","model":model
    })).await;
    let opened = call(&router,"/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model})).await;
    let session = opened["agent_session_id"].as_str().unwrap();
    let marker = format!("MEMORY_{}",Uuid::now_v7().simple());
    let saved = send(&router,&pool,session,&format!("Remember this random marker for this conversation: {marker}. Reply only SAVED.")).await;
    assert!(saved.contains("SAVED"), "model did not acknowledge the input");
    let recalled = send(&router,&pool,session,"What is the random marker from the previous user message? Reply only with the exact marker.").await;
    assert!(recalled.contains(&marker), "ordinary continuation did not retain the marker before clear");
    let history_before: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages WHERE session_id=?")
        .bind(session).fetch_one(&pool).await.unwrap();
    let action_key = Uuid::now_v7().to_string();
    let clear_path = format!("/api/agent-sessions/{session}/clear-context");
    call(&router,&clear_path,json!({"idempotency_key":action_key})).await;
    let after = send(&router,&pool,session,"If an earlier message in the current conversation gave you a random marker, reply with that exact marker. If there is no earlier marker, reply only NO_PREVIOUS_MARKER. Do not invent a marker.").await;
    assert!(after.contains("NO_PREVIOUS_MARKER") && !after.contains(&marker), "cleared context leaked into the new model Turn");
    // Replaying the original clear after a successor has completed is a no-op.
    call(&router,&clear_path,json!({"idempotency_key":action_key})).await;
    let clears: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session).fetch_one(&pool).await.unwrap();
    assert_eq!(clears,1);
    let history_after: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages WHERE session_id=?")
        .bind(session).fetch_one(&pool).await.unwrap();
    assert!(history_after > history_before, "visible history must be retained across clear");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
    eprintln!("AGNES_SESSION_CONTEXT_CLEAR_PASS model={MODEL} history_retained=true continued_before_clear=true cleared_before_next_turn=true clear_replay_count={clears}");
}
