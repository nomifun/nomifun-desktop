//! Product-path recovery from a transactionally consistent crash image.
//! Scripted provider: this is fault-injection evidence, not a live-model SLO.
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use axum::{Router, body::Body, http::{Request, StatusCode}};
use serde_json::{Value, json};
use tower::ServiceExt;
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;

const TRUST: &str = "native-recovery-fixture";

async fn call(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let (status, body) = response(router, method, path, body).await;
    assert!(matches!(status, StatusCode::OK | StatusCode::CREATED), "{status}: {body}");
    body["data"].clone()
}

async fn response(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, body)
}

fn stream(tool: Option<(&str, &str, Value)>, text: &str) -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
        .set_body_string(stream_body(tool,text))
}

fn stream_body(tool: Option<(&str, &str, Value)>, text: &str) -> String {
    let (delta, reason) = match tool {
        Some((id, name, args)) => (json!({"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}), "tool_calls"),
        None => (json!({"content":text}), "stop"),
    };
    let data = json!({"id":"recovery-model", "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let done = json!({"id":"recovery-model", "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
    format!("data: {data}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn owner_pause_resume_keeps_one_turn_and_one_write_across_generations() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let handler_requests = requests.clone(); let handler_entered = entered.clone(); let handler_release = release.clone();
    let provider = Router::new().route("/v1/chat/completions", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
        let requests = handler_requests.clone(); let entered = handler_entered.clone(); let release = handler_release.clone();
        async move {
            let round = requests.fetch_add(1,Ordering::SeqCst);
            let data = match round {
                0 => {
                    entered.notify_one();
                    release.notified().await;
                    stream_body(Some(("write-once","write_file",json!({"path":"answer.txt","content":"PAUSE_RESUME_OK"}))),"")
                },
                1 => {
                    assert!(body.to_string().contains("write-once"),"resume lost completed write");
                    stream_body(Some(("replan","update_plan",json!({"explanation":"Confirm current workspace after pause","plan":[{"step":"Verify saved file","status":"in_progress"}]}))),"")
                },
                2 => stream_body(Some(("verify-current","read_file",json!({"path":"answer.txt"}))),""),
                3 => stream_body(Some(("close-plan","update_plan",json!({"explanation":"Fresh read confirms file","plan":[{"step":"Verify saved file","status":"completed"}]}))),""),
                4 => stream_body(Some(("report","report_completion",json!({"summary":"Verified current file","criteria":[{"step":"Verify saved file","disposition":"supported","evidence_call_ids":["verify-current"],"requirement_ids":["input_0"],"rationale":"Fresh read confirms all requested content"}]}))),""),
                5 => stream_body(None,"PAUSE_RESUME_COMPLETE"),
                _ => panic!("unexpected resumed model request {round}"),
            };
            ([(axum::http::header::CONTENT_TYPE,"text/event-stream")], data)
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener,provider).await.unwrap(); });
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let app = AppServices::from_config(db,&config).await.unwrap();
    let router = create_router(&app).await;
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"scripted pause resume", "base_url":format!("http://{address}/v1"),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Pause fixture","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let execution = format!("/api/agent-sessions/{id}/execution");
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"one-task","input":{"content":"Create answer.txt containing PAUSE_RESUME_OK, verify it, and report the result."}})).await;
    tokio::time::timeout(Duration::from_secs(30),entered.notified()).await.unwrap();
    let running = call(&router,"GET",&execution,Value::Null).await;
    let pause = json!({"operation_id":running["operation_id"],"idempotency_key":"pause-once","reason":"inspect work"});
    let ack = call(&router,"POST",&format!("{execution}/pause"),pause.clone()).await;
    assert_eq!(call(&router,"POST",&format!("{execution}/pause"),pause).await,ack);
    release.notify_one();
    let paused = tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "paused" { break state; }
            assert_eq!(state["state"],"running","unexpected stop: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(paused["turn_state"],"running");
    assert_eq!(paused["pause"]["cleanup_proven"],true);
    assert_eq!(paused["checkpoint_retained"],true);
    let projection = call(&router,"GET",&format!("/api/agent-sessions/{id}/projection"),Value::Null).await;
    assert_eq!(projection["status"],"running");
    assert_eq!(projection["extra"]["execution_phase"],"paused");
    assert_eq!(projection["runtime"]["can_send_message"],false);
    assert_eq!(projection["runtime"]["is_processing"],false);
    assert!(projection["runtime"]["active_turn_id"].is_string());
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(),"PAUSE_RESUME_OK");
    assert_eq!(requests.load(Ordering::SeqCst),1);
    let (status,_) = response(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"must-not-start","input":{"content":"Replace the task"}})).await;
    assert!(status.is_client_error());
    let resume = json!({"operation_id":paused["operation_id"],"idempotency_key":"resume-once",
        "expected_pause_revision":paused["pause"]["revision"],"expected_checkpoint_revision":paused["checkpoint_revision"],
        "expected_checkpoint_digest":paused["checkpoint_digest"],"budget":{}});
    let mut stale = resume.clone(); stale["expected_checkpoint_digest"] = json!("f".repeat(64));
    assert!(response(&router,"POST",&format!("{execution}/resume"),stale).await.0.is_client_error());
    let resume_path = format!("{execution}/resume");
    let (authorized,duplicate) = tokio::join!(call(&router,"POST",&resume_path,resume.clone()),call(&router,"POST",&resume_path,resume));
    assert_ne!(authorized["duplicate"],duplicate["duplicate"]);
    assert_eq!(authorized["authorization_event_id"],duplicate["authorization_event_id"]);
    let completed = tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "completed" { break state; }
            assert_eq!(state["state"],"running","resume did not continue: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(completed["operation_id"],paused["operation_id"]);
    assert!(completed["execution_generation"].as_u64().unwrap() > paused["execution_generation"].as_u64().unwrap());
    for (kind,count) in [("turn/started",1i64),("turn/paused",1),("turn/resume-authorized",1),("turn/completed",1),("turn/failed",0)] {
        let actual: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind=?").bind(id).bind(kind).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(actual,count,"{kind}");
    }
    let writes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_started' AND json_extract(inline_json,'$.event.action_id')='workspace.files/write'")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    assert_eq!(writes,1);
    assert_eq!(requests.load(Ordering::SeqCst),6);
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(),"PAUSE_RESUME_OK");
    drop(router);
    app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
    server.abort(); let _ = server.await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn startup_resumes_crash_image_without_repeating_the_completed_write() {
    startup_recovery_scenario(false).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn startup_quarantines_a_reconciliation_head_without_starting_a_model() {
    startup_recovery_scenario(true).await;
}

async fn startup_recovery_scenario(reconciliation_required: bool) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let mode = Arc::new(AtomicUsize::new(0));
    let pending = Arc::new(AtomicBool::new(false));
    let resumed = Arc::new(AtomicUsize::new(0));
    let upstream = wiremock::MockServer::start().await;
    let handler_mode = mode.clone(); let handler_pending = pending.clone(); let handler_resumed = resumed.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if handler_mode.load(Ordering::SeqCst) == 0 {
                let has_write_result = body["messages"].as_array().unwrap().iter().any(|message| message["role"] == "tool");
                if !has_write_result {
                    return stream(Some(("create-once", "write_file", json!({"path":"answer.txt","content":"CHECKPOINT_RECOVERY_OK"}))), "");
                }
                handler_pending.store(true, Ordering::SeqCst);
                return stream(None, "old response must not be used").set_delay(Duration::from_secs(60));
            }
            let round = handler_resumed.fetch_add(1, Ordering::SeqCst);
            assert!(body.to_string().contains("create-once"), "recovery lost the completed operation");
            match round {
                0 => stream(Some(("replan", "update_plan", json!({"explanation":"Verify restored progress","plan":[{"step":"Verify saved file","status":"in_progress"}]}))), ""),
                1 => stream(Some(("verify-restored", "read_file", json!({"path":"answer.txt"}))), ""),
                2 => stream(Some(("close-plan", "update_plan", json!({"explanation":"Current file verified","plan":[{"step":"Verify saved file","status":"completed"}]}))), ""),
                3 => stream(Some(("report", "report_completion", json!({"summary":"Verified restored file","criteria":[{"step":"Verify saved file","disposition":"supported","evidence_call_ids":["verify-restored"],"requirement_ids":["input_0"],"rationale":"Fresh read confirms the saved content"}]}))), ""),
                4 => stream(None, "RECOVERED_TASK_OK"),
                _ => panic!("unexpected model round after recovery: {round}"),
            }
        }).mount(&upstream).await;
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let first = AppServices::from_config(db, &config).await.unwrap();
    let router = create_router(&first).await;
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"scripted recovery", "base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Recovery fixture","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap().to_owned();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"recovery-turn","input":{"content":"Create answer.txt containing CHECKPOINT_RECOVERY_OK, verify it, then report the result."}})).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        while !pending.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap();
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(), "CHECKPOINT_RECOVERY_OK");
    let inspection = call(&router, "GET", &format!("/api/agent-sessions/{id}/execution"), Value::Null).await;
    assert_eq!(inspection["state"], "running");
    assert_eq!(inspection["checkpoint_retained"], true);
    assert_eq!(inspection["automatic_replay_authorized"], false);
    let snapshot = root.path().join("crash-image.db");
    first.database.snapshot_into(&snapshot).await.unwrap();
    // Graceful cleanup affects only the original DB. The snapshot preserves
    // the running Turn and is the authoritative crash image used below.
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({"idempotency_key":"clean-original"})).await;
    first.shutdown_browser_platform().await.unwrap();
    first.database.close().await;
    drop(router); drop(first);
    mode.store(1, Ordering::SeqCst);
    let recovered_db = nomifun_db::init_database(&snapshot).await.unwrap();
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE state='running'").execute(recovered_db.pool()).await.unwrap();
    if reconciliation_required {
        // This isolated crash image models the head preserved by migration
        // 007 or effect uncertainty. No user database is modified.
        sqlx::query("UPDATE agent_session_heads SET status='reconciliation' WHERE session_id=?")
            .bind(&id).execute(recovered_db.pool()).await.unwrap();
    }
    let second = AppServices::from_config(recovered_db, &config).await.unwrap();
    let restored_router = create_router(&second).await;
    if reconciliation_required {
        let inspection = tokio::time::timeout(Duration::from_secs(30),async {
            loop {
                let state = call(&restored_router,"GET",&format!("/api/agent-sessions/{id}/execution"),Value::Null).await;
                if state["state"] == "paused" { break state; }
                assert_eq!(state["turn_state"],"running");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        assert_eq!(inspection["recovery_blocked"],true);
        assert_eq!(inspection["pause"]["cleanup_proven"],false);
        assert_eq!(inspection["checkpoint_retained"],true);
        assert_eq!(inspection["turn_state"],"running");
        assert_eq!(resumed.load(Ordering::SeqCst),0);
        assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(),"CHECKPOINT_RECOVERY_OK");
        drop(restored_router);
        second.shutdown_browser_platform().await.unwrap(); second.database.close().await;
        return;
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state: String = sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=? ORDER BY accepted_at DESC LIMIT 1")
                .bind(&id).fetch_one(second.database.pool()).await.unwrap();
            if state != "running" {
                assert_eq!(state, "completed", "recovery did not complete"); break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.unwrap();
    let writes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_started' AND json_extract(inline_json,'$.event.action_id')='workspace.files/write'")
        .bind(&id).fetch_one(second.database.pool()).await.unwrap();
    let resumes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='execution_resumed'")
        .bind(&id).fetch_one(second.database.pool()).await.unwrap();
    assert_eq!(writes, 1); assert_eq!(resumes, 1); assert_eq!(resumed.load(Ordering::SeqCst), 5);
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(), "CHECKPOINT_RECOVERY_OK");
    let inspection = call(&restored_router, "GET", &format!("/api/agent-sessions/{id}/execution"), Value::Null).await;
    assert_eq!(inspection["state"], "completed");
    assert_eq!(inspection["checkpoint_retained"], false);
    assert_eq!(inspection["execution_fence"], 1);
    drop(restored_router);
    second.shutdown_browser_platform().await.unwrap(); second.database.close().await;
}
