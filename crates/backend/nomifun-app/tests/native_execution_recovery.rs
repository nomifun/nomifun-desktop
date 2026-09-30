//! Product-path recovery from a transactionally consistent crash image.
//! Scripted provider: this is fault-injection evidence, not a live-model SLO.
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, EventId,
    EventProducerId, IdempotencyKey, OperationId, SemanticSessionEventDraft,
    SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
    digest_payload,
};
use nomifun_agent_session::{AgentSessionStore, EffectEventRequest, EffectStrategy};
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

fn long_helper_command() -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "powershell.exe".into(),
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "[IO.File]::WriteAllText('helper.pid',[string]$PID,[Text.Encoding]::ASCII); Start-Sleep -Seconds 60".into(),
            ],
        )
    } else {
        (
            "/bin/sh".into(),
            vec!["-c".into(), "printf %s $$ > helper.pid; sleep 60".into()],
        )
    }
}

fn process_exists(pid: u32) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(sysinfo::Pid::from_u32(pid)).is_some()
}

async fn seed_pending_external_push(
    pool: &sqlx::SqlitePool,
    session_id: &str,
) -> String {
    let (turn_id, turn_started_event_id): (String, String) = sqlx::query_as(
        "SELECT operation_id,started_event_id FROM agent_turns \
         WHERE session_id=? AND state='running' ORDER BY accepted_at DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
    let agent_session_id = AgentSessionId::from(session_id.to_owned());
    let operation_id = OperationId::from("startup-pending-push-operation");
    let tool_event_id = EventId::from(format!("startup-pending-push-tool:{session_id}"));
    let tool = SessionEventAppend {
        agent_session_id: agent_session_id.clone(),
        event_id: tool_event_id.clone(),
        producer_id: EventProducerId::from("capability_host"),
        idempotency_key: IdempotencyKey::from(format!(
            "startup-pending-push-tool:{session_id}"
        )),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("tool/call-started".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId::from(operation_id.as_ref().to_owned()),
            causation_event_id: Some(EventId::from(turn_started_event_id)),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id": operation_id.as_ref(),
                "capability_id": "workspace.vcs",
                "action_id": "workspace.vcs/push",
            }))),
        },
    };
    store.append_event(&tool).await.unwrap();

    let effect_id = format!("startup-pending-push-effect:{session_id}");
    store
        .record_effect_started(EffectEventRequest {
            agent_session_id,
            effect_id: effect_id.clone(),
            turn_id: OperationId::from(turn_id),
            operation_id,
            owner_domain: "workspace".to_owned(),
            capability_module: CapabilityId::from("workspace.vcs"),
            action_id: ActionId::from("workspace.vcs/push"),
            resource_binding_id: None,
            resource_key: Some("workspace.vcs:origin:refs/heads/main".to_owned()),
            input_digest: DigestHex::from("7".repeat(64)),
            recorded_at: 1_788_000_000_010,
            event_id: EventId::from(format!("startup-pending-push-started:{session_id}")),
            producer_id: EventProducerId::from("capability_host"),
            idempotency_key: IdempotencyKey::from(format!(
                "startup-pending-push:{session_id}"
            )),
            correlation_id: CorrelationId::from(effect_id.clone()),
            strategy: EffectStrategy::ExternalUncertainEffect,
            causation_event_id: Some(tool_event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "remote": "origin",
                "refspec": "HEAD:refs/heads/main",
                "force": false,
            }))),
        })
        .await
        .unwrap();
    let state: String = sqlx::query_scalar(
        "SELECT state FROM agent_effects WHERE session_id=? AND effect_id=?",
    )
    .bind(session_id)
    .bind(&effect_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(state, "pending");
    effect_id
}

fn push_answer_to_local_remote(project: &std::path::Path, remote_path: &std::path::Path) -> String {
    let repository = git2::Repository::init(project).unwrap();
    let mut index = repository.index().unwrap();
    index.add_path(std::path::Path::new("answer.txt")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repository.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("NomiFun test", "test@nomifun.invalid").unwrap();
    let commit = repository
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "recovery fixture push",
            &tree,
            &[],
        )
        .unwrap();
    drop(tree);
    repository.set_head("refs/heads/main").unwrap();
    git2::Repository::init_bare(remote_path).unwrap();
    repository
        .remote("origin", remote_path.to_str().unwrap())
        .unwrap();
    repository
        .find_remote("origin")
        .unwrap()
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();
    let remote_commit = git2::Repository::open_bare(remote_path)
        .unwrap()
        .find_reference("refs/heads/main")
        .unwrap()
        .target()
        .unwrap();
    assert_eq!(remote_commit, commit);
    commit.to_string()
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
    // The consumer's ordinary event page ends at 500. A durable pause after
    // that boundary must still carry its exact cleanup evidence.
    let running = call(&router,"GET",&execution,Value::Null).await;
    let store = AgentSessionStore::from_pool(app.database.pool().clone()).await.unwrap();
    let principal = nomifun_agent_contracts::PrincipalRef {
        principal_kind: "user".into(), principal_id: app.authoritative_user_id.as_ref().into(),
    };
    for index in 0..500 {
        store.request_native_pause(&principal, &id.to_owned().into(),
            &running["operation_id"].as_str().unwrap().to_owned().into(),
            &format!("pause-page-boundary:{index}"), "inspect work").await.unwrap();
    }
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
    assert_eq!(projection["extra"]["execution_pause"]["cleanup_proven"], true,
        "a pause beyond the first event page must retain its canonical cleanup proof");
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
    let (authorized,duplicate) = tokio::join!(call(&router,"POST",&resume_path,resume.clone()),call(&router,"POST",&resume_path,resume.clone()));
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
    assert_eq!(requests.load(Ordering::SeqCst),5);
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(),"PAUSE_RESUME_OK");
    let event_count:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=?")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    let replay = call(&router,"POST",&resume_path,resume.clone()).await;
    assert_eq!(replay["duplicate"],true,"an exact retry returns only its old authorization receipt");
    let mut after_terminal = resume;
    after_terminal["idempotency_key"] = json!("resume-after-completed");
    let (status,_) = response(&router,"POST",&resume_path,after_terminal).await;
    assert!(status.is_client_error(),"a new resume command cannot reopen a completed Turn");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(requests.load(Ordering::SeqCst),5,"terminal resume attempts cannot open a model request");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_events WHERE session_id=?")
        .bind(id).fetch_one(app.database.pool()).await.unwrap(),event_count,
        "terminal resume attempts cannot append canonical events");
    assert_eq!(call(&router,"GET",&execution,Value::Null).await["state"],"completed");
    drop(router);
    app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
    server.abort(); let _ = server.await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn pause_cleans_a_running_process_before_resume_and_never_restarts_it() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (helper_command, helper_args) = long_helper_command();
    let handler_requests = requests.clone(); let handler_entered = entered.clone(); let handler_release = release.clone();
    let provider = Router::new().route("/v1/chat/completions", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
        let requests = handler_requests.clone(); let entered = handler_entered.clone(); let release = handler_release.clone();
        let helper_command = helper_command.clone(); let helper_args = helper_args.clone();
        async move {
            let round = requests.fetch_add(1,Ordering::SeqCst);
            let data = match round {
                0 => stream_body(Some(("start-helper","start_process",json!({
                    "command":helper_command,"args":helper_args,"wait_ms":0
                }))),""),
                1 => {
                    entered.notify_one();
                    release.notified().await;
                    stream_body(Some(("read-before-pause","read_file",json!({"path":"helper.pid"}))),"")
                },
                2 => {
                    let encoded = body.to_string();
                    assert!(encoded.contains("start-helper") && encoded.contains("read-before-pause"),
                        "resume lost reconciled process/read history");
                    stream_body(Some(("replan","update_plan",json!({"explanation":"Verify the marker after pause cleanup",
                        "plan":[{"step":"Verify helper marker","status":"in_progress"}]}))),"")
                },
                3 => stream_body(Some(("read-marker","read_file",json!({"path":"helper.pid"}))),""),
                4 => stream_body(Some(("close-plan","update_plan",json!({"explanation":"Fresh marker read complete",
                    "plan":[{"step":"Verify helper marker","status":"completed"}]}))),""),
                5 => stream_body(Some(("report","report_completion",json!({"summary":"Helper marker verified after pause cleanup",
                    "criteria":[{"step":"Verify helper marker","disposition":"supported","evidence_call_ids":["read-marker"],
                        "requirement_ids":["input_0"],"rationale":"Fresh marker read confirms the helper started before pause"}]}))),""),
                _ => panic!("unexpected process recovery model request {round}"),
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
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"scripted process pause", "base_url":format!("http://{address}/v1"),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Process pause fixture","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let execution = format!("/api/agent-sessions/{id}/execution");
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"process-task",
        "input":{"content":"Start one helper that writes its PID to helper.pid, then verify the marker and report. Never start it twice."}})).await;
    tokio::time::timeout(Duration::from_secs(30),entered.notified()).await.unwrap();
    let marker = project.join("helper.pid");
    let pid = tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&marker)
                && let Ok(pid) = text.trim().parse::<u32>() { break pid; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(process_exists(pid),"managed helper must be live before pause cleanup");
    let running = call(&router,"GET",&execution,Value::Null).await;
    let pause = json!({"operation_id":running["operation_id"],"idempotency_key":"pause-process","reason":"inspect cleanup"});
    call(&router,"POST",&format!("{execution}/pause"),pause).await;
    release.notify_one();
    let paused = tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "paused" { break state; }
            assert_eq!(state["state"],"running","process pause failed: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(paused["pause"]["cleanup_proven"],true);
    tokio::time::timeout(Duration::from_secs(10),async {
        while process_exists(pid) { tokio::time::sleep(Duration::from_millis(50)).await; }
    }).await.expect("pause must reap the managed process tree");
    let cleanup_seq:i64 = sqlx::query_scalar("SELECT seq FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_cleanup_proven'")
        .bind(id).bind(paused["operation_id"].as_str().unwrap()).fetch_one(app.database.pool()).await.unwrap();
    let pause_seq:i64 = sqlx::query_scalar("SELECT seq FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/paused'")
        .bind(id).bind(paused["operation_id"].as_str().unwrap()).fetch_one(app.database.pool()).await.unwrap();
    assert!(cleanup_seq < pause_seq,"cleanup proof must precede the pause state");

    let resume = json!({"operation_id":paused["operation_id"],"idempotency_key":"resume-process",
        "expected_pause_revision":paused["pause"]["revision"],"expected_checkpoint_revision":paused["checkpoint_revision"],
        "expected_checkpoint_digest":paused["checkpoint_digest"],"budget":{}});
    call(&router,"POST",&format!("{execution}/resume"),resume).await;
    let completed = tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "completed" { break state; }
            assert_eq!(state["state"],"running","process resume failed: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(completed["execution_generation"].as_u64().unwrap() > paused["execution_generation"].as_u64().unwrap());
    assert!(!process_exists(pid));
    let starts:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_started' AND json_extract(inline_json,'$.event.action_id')='workspace.process/start'")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    assert_eq!(starts,1,"resume must not replay the completed process start");
    assert_eq!(requests.load(Ordering::SeqCst),6);
    for (kind,count) in [("turn/started",1i64),("turn/paused",1),("turn/resume-authorized",1),("turn/completed",1),("turn/failed",0)] {
        let actual:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind=?")
            .bind(id).bind(kind).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(actual,count,"{kind}");
    }
    drop(router);
    app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
    server.abort(); let _ = server.await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn cancel_interrupts_provider_retry_after_without_opening_another_request() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let handler_requests = requests.clone(); let handler_entered = entered.clone();
    let provider = Router::new().route("/v1/chat/completions", axum::routing::post(move || {
        let requests = handler_requests.clone(); let entered = handler_entered.clone();
        async move {
            requests.fetch_add(1,Ordering::SeqCst);
            entered.notify_one();
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER,"60")],
                axum::Json(json!({"error":{"message":"fixture cooldown","type":"rate_limit_error"}})),
            )
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener,provider).await.unwrap(); });
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let app = AppServices::from_config(db,&config).await.unwrap();
    let router = create_router(&app).await;
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"scripted retry cancellation", "base_url":format!("http://{address}/v1"),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Retry cancel fixture","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let execution = format!("/api/agent-sessions/{id}/execution");
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"retry-task",
        "input":{"content":"Reply after the provider becomes available."}})).await;
    tokio::time::timeout(Duration::from_secs(10),entered.notified()).await.unwrap();
    assert_eq!(requests.load(Ordering::SeqCst),1);
    let started = tokio::time::Instant::now();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({"idempotency_key":"cancel-retry"})).await;
    let cancelled = tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "cancelled" { break state; }
            assert_eq!(state["state"],"running","retry cancellation failed: {state}");
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(2),"cancel waited for provider Retry-After");
    assert_eq!(cancelled["checkpoint_retained"],false);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(requests.load(Ordering::SeqCst),1,"cancellation must prevent the retry attempt");
    for (kind,count) in [("turn/cancelled",1i64),("turn/completed",0),("turn/failed",0)] {
        let actual:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind=?")
            .bind(id).bind(kind).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(actual,count,"{kind}");
    }
    drop(router);
    app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
    server.abort(); let _ = server.await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn cancelled_turn_is_never_selected_by_startup_recovery() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let handler_requests = requests.clone(); let handler_entered = entered.clone();
    let provider = Router::new().route("/v1/chat/completions", axum::routing::post(move || {
        let requests = handler_requests.clone(); let entered = handler_entered.clone();
        async move {
            requests.fetch_add(1,Ordering::SeqCst);
            entered.notify_one();
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER,"60")],
                axum::Json(json!({"error":{"message":"fixture cooldown","type":"rate_limit_error"}})),
            )
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener,provider).await.unwrap(); });
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let first = AppServices::from_config(db,&config).await.unwrap();
    let router = create_router(&first).await;
    let provider = call(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"scripted cancelled recovery", "base_url":format!("http://{address}/v1"),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Cancelled recovery fixture","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap().to_owned();
    let execution = format!("/api/agent-sessions/{id}/execution");
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"cancel-before-restart",
        "input":{"content":"Wait for the provider."}})).await;
    tokio::time::timeout(Duration::from_secs(10),entered.notified()).await.unwrap();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({"idempotency_key":"cancel-terminal"})).await;
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let state = call(&router,"GET",&execution,Value::Null).await;
            if state["state"] == "cancelled" { break; }
            assert_eq!(state["state"],"running","cancel before restart failed: {state}");
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert_eq!(requests.load(Ordering::SeqCst),1);
    drop(router);
    first.shutdown_browser_platform().await.unwrap(); first.database.close().await; drop(first);

    let reopened = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let second = AppServices::from_config(reopened,&config).await.unwrap();
    let restored_router = create_router(&second).await;
    let candidates:i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_session_heads h JOIN agent_sessions s ON s.agent_session_id=h.session_id \
         JOIN agent_turns t ON t.session_id=h.session_id AND t.operation_id=h.active_turn_id \
         WHERE s.state='live' AND h.status IN ('running','reconciliation') AND t.state='running'")
        .fetch_one(second.database.pool()).await.unwrap();
    assert_eq!(candidates,0,"cancelled Turn cannot enter the startup recovery candidate set");
    assert_eq!(requests.load(Ordering::SeqCst),1,"restart must not reopen the provider");
    let restored = call(&restored_router,"GET",&execution,Value::Null).await;
    assert_eq!(restored["state"],"cancelled");
    assert_eq!(restored["checkpoint_retained"],false);
    let resumed:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='execution_resumed'")
        .bind(&id).fetch_one(second.database.pool()).await.unwrap();
    assert_eq!(resumed,0);
    drop(restored_router);
    second.shutdown_browser_platform().await.unwrap(); second.database.close().await;
    server.abort(); let _ = server.await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn startup_resumes_crash_image_without_repeating_the_completed_write() {
    startup_recovery_scenario(false, false).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn startup_quarantines_a_pending_external_effect_without_replay() {
    startup_recovery_scenario(true, false).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn owner_reconciles_unknown_push_from_independent_remote_ref_before_resume() {
    startup_recovery_scenario(true, true).await;
}

async fn startup_recovery_scenario(
    reconciliation_required: bool,
    reconcile_external_effect: bool,
) {
    assert!(!reconcile_external_effect || reconciliation_required);
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
    let execution = format!("/api/agent-sessions/{id}/execution");
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"recovery-turn","input":{"content":"Create answer.txt containing CHECKPOINT_RECOVERY_OK, verify it, then report the result."}})).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        while !pending.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap();
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(), "CHECKPOINT_RECOVERY_OK");
    let external_push = if reconcile_external_effect {
        let remote_path = root.path().join("remote.git");
        let source_commit = push_answer_to_local_remote(&project, &remote_path);
        Some((remote_path, source_commit))
    } else { None };
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
    let pending_effect_id = if reconciliation_required {
        let effect_id = seed_pending_external_push(recovered_db.pool(), &id).await;
        // This isolated crash image models the head preserved by migration
        // 007 or effect uncertainty. No user database is modified.
        sqlx::query("UPDATE agent_session_heads SET status='reconciliation' WHERE session_id=?")
            .bind(&id).execute(recovered_db.pool()).await.unwrap();
        Some(effect_id)
    } else { None };
    let second = AppServices::from_config(recovered_db, &config).await.unwrap();
    let mut user_events = second.event_bus.subscribe_user();
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
        assert_eq!(inspection["pending_effects"],0);
        assert_eq!(inspection["unknown_effects"],1);
        assert_eq!(resumed.load(Ordering::SeqCst),0);
        assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(),"CHECKPOINT_RECOVERY_OK");
        let paused_event = tokio::time::timeout(Duration::from_secs(2),async {
            loop {
                let envelope = user_events.recv().await.unwrap();
                if envelope.event.name == "turn.paused"
                    && envelope.event.data["conversation_id"].as_str() == Some(id.as_str()) {
                    break envelope;
                }
            }
        }).await.expect("startup quarantine must notify the live owner about the canonical pause");
        assert_eq!(paused_event.user_id,second.authoritative_user_id.as_ref());
        assert_eq!(paused_event.event.data["execution_phase"],"paused");
        assert_eq!(paused_event.event.data["can_send_message"],false);
        assert!(paused_event.event.data["turn_id"].as_str().is_some_and(|value|value.len()==36));

        let effect_id = pending_effect_id.as_ref().unwrap();
        let (effect_state, strategy, action_id, terminal_event_id): (String, String, String, Option<String>) =
            sqlx::query_as("SELECT state,strategy,action_id,terminal_event_id FROM agent_effects WHERE session_id=? AND effect_id=?")
                .bind(&id).bind(effect_id).fetch_one(second.database.pool()).await.unwrap();
        assert_eq!(effect_state,"unknown");
        assert_eq!(strategy,"external_uncertain_effect");
        assert_eq!(action_id,"workspace.vcs/push");
        let terminal_event_id = terminal_event_id.expect("startup quarantine must commit an uncertainty receipt");
        let (terminal_kind, terminal_payload): (String, String) = sqlx::query_as(
            "SELECT kind,inline_json FROM agent_events WHERE session_id=? AND event_id=?",
        ).bind(&id).bind(&terminal_event_id).fetch_one(second.database.pool()).await.unwrap();
        assert_eq!(terminal_kind,"effect/uncertain");
        let terminal_payload: Value = serde_json::from_str(&terminal_payload).unwrap();
        assert_eq!(terminal_payload["outcome"],"unknown");
        assert_eq!(terminal_payload["recovery"],"process_restart_external_reconciliation_required");
        for (kind,count) in [("effect/uncertain",1i64),("effect/succeeded",0),("effect/failed",0),("effect/reconciled",0)] {
            let actual:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND correlation_id=? AND kind=?")
                .bind(&id).bind(effect_id).bind(kind).fetch_one(second.database.pool()).await.unwrap();
            assert_eq!(actual,count,"{kind}");
        }
        let blocked:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/execution-recovery-blocked'")
            .bind(&id).fetch_one(second.database.pool()).await.unwrap();
        assert_eq!(blocked,1);
        let (new_turn_status,_) = response(&restored_router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({
            "idempotency_key":"must-not-run-after-unknown-effect",
            "input":{"content":"Try the push again."}
        })).await;
        assert!(new_turn_status.is_client_error());
        assert_eq!(resumed.load(Ordering::SeqCst),0,"startup quarantine cannot replay the model or effect");

        if reconcile_external_effect {
            let operation_id = inspection["operation_id"].as_str().unwrap();
            let candidates = call(
                &restored_router,
                "GET",
                &format!("{execution}/effects?operation_id={operation_id}"),
                Value::Null,
            ).await;
            assert_eq!(candidates["items"].as_array().unwrap().len(),1);
            assert_eq!(candidates["items"][0]["effect_id"],effect_id.as_str());
            assert_eq!(candidates["items"][0]["action_id"],"workspace.vcs/push");
            assert_eq!(candidates["items"][0]["state"],"unknown");
            assert_eq!(candidates["automatic_replay_authorized"],false);

            let (remote_path, source_commit) = external_push.as_ref().unwrap();
            let observed_commit = git2::Repository::open_bare(remote_path)
                .unwrap()
                .find_reference("refs/heads/main")
                .unwrap()
                .target()
                .unwrap()
                .to_string();
            assert_eq!(&observed_commit,source_commit);
            let evidence_observation = json!({
                "remote":"origin",
                "destination_ref":"refs/heads/main",
                "remote_commit":observed_commit,
            });
            let evidence_digest = digest_payload(&evidence_observation).unwrap();
            let evidence = json!({
                "verified":true,
                "evidence_digest":evidence_digest.as_ref(),
                "reference":format!("workspace.vcs/push:origin:refs/heads/main@{source_commit}"),
            });
            let reconcile_request = json!({
                "operation_id":operation_id,
                "expected_pause_revision":inspection["pause"]["revision"],
                "idempotency_key":"verify-push-remote-ref",
                "effect_id":effect_id,
                "expected_input_digest":candidates["items"][0]["input_digest"],
                "outcome":"confirmed_succeeded",
                "evidence":evidence,
            });
            let mut wrong_digest = reconcile_request.clone();
            wrong_digest["idempotency_key"] = json!("reject-wrong-push-digest");
            wrong_digest["expected_input_digest"] = json!("8".repeat(64));
            assert!(response(&restored_router,"POST",&format!("{execution}/reconcile"),wrong_digest).await.0.is_client_error());
            let still_unknown:String = sqlx::query_scalar("SELECT state FROM agent_effects WHERE session_id=? AND effect_id=?")
                .bind(&id).bind(effect_id).fetch_one(second.database.pool()).await.unwrap();
            assert_eq!(still_unknown,"unknown");

            let reconciled = call(&restored_router,"POST",&format!("{execution}/reconcile"),reconcile_request.clone()).await;
            assert_eq!(call(&restored_router,"POST",&format!("{execution}/reconcile"),reconcile_request).await,reconciled);
            let after = call(
                &restored_router,
                "GET",
                &format!("{execution}/effects?operation_id={operation_id}"),
                Value::Null,
            ).await;
            assert!(after["items"].as_array().unwrap().is_empty());
            let (settled, reconciled_events, attestation_events):(String,i64,i64) = sqlx::query_as(
                "SELECT e.state, \
                 (SELECT COUNT(*) FROM agent_events WHERE session_id=e.session_id AND correlation_id=e.effect_id AND kind='effect/reconciled'), \
                 (SELECT COUNT(*) FROM agent_events WHERE session_id=e.session_id AND correlation_id=e.turn_id AND kind='runtime/effect-reconciliation-attested') \
                 FROM agent_effects e WHERE e.session_id=? AND e.effect_id=?",
            ).bind(&id).bind(effect_id).fetch_one(second.database.pool()).await.unwrap();
            assert_eq!(settled,"returned");
            assert_eq!(reconciled_events,1);
            assert_eq!(attestation_events,1);

            call(&restored_router,"POST",&format!("{execution}/resume"),json!({
                "operation_id":operation_id,
                "idempotency_key":"resume-after-push-reconciliation",
                "expected_pause_revision":inspection["pause"]["revision"],
                "expected_checkpoint_revision":inspection["checkpoint_revision"],
                "expected_checkpoint_digest":inspection["checkpoint_digest"],
                "budget":{},
                "cleanup_attestation":evidence,
            })).await;
            let completed = tokio::time::timeout(Duration::from_secs(30),async {
                loop {
                    let state = call(&restored_router,"GET",&execution,Value::Null).await;
                    if state["state"] == "completed" { break state; }
                    assert_eq!(state["state"],"running","owner reconciliation resume failed: {state}");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await.unwrap();
            assert_eq!(completed["checkpoint_retained"],false);
            assert_eq!(completed["pending_effects"],0);
            assert_eq!(completed["unknown_effects"],0);
            assert_eq!(resumed.load(Ordering::SeqCst),4);
            let remote_after = git2::Repository::open_bare(remote_path)
                .unwrap().find_reference("refs/heads/main").unwrap().target().unwrap().to_string();
            assert_eq!(remote_after,*source_commit,"effect reconciliation and resume cannot repeat the push");
            drop(restored_router);
            second.shutdown_browser_platform().await.unwrap(); second.database.close().await;
            return;
        }

        drop(restored_router);
        second.shutdown_browser_platform().await.unwrap(); second.database.close().await;

        let reopened_again = nomifun_db::init_database(&snapshot).await.unwrap();
        let third = AppServices::from_config(reopened_again, &config).await.unwrap();
        let third_router = create_router(&third).await;
        let persisted = call(&third_router,"GET",&format!("/api/agent-sessions/{id}/execution"),Value::Null).await;
        assert_eq!(persisted["state"],"paused");
        assert_eq!(persisted["pending_effects"],0);
        assert_eq!(persisted["unknown_effects"],1);
        let repeated:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/execution-recovery-blocked'")
            .bind(&id).fetch_one(third.database.pool()).await.unwrap();
        assert_eq!(repeated,1,"a later startup cannot quarantine the same effect twice");
        assert_eq!(resumed.load(Ordering::SeqCst),0);
        drop(third_router);
        third.shutdown_browser_platform().await.unwrap(); third.database.close().await;
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
    assert_eq!(writes, 1); assert_eq!(resumes, 1); assert_eq!(resumed.load(Ordering::SeqCst), 4);
    assert_eq!(std::fs::read_to_string(project.join("answer.txt")).unwrap(), "CHECKPOINT_RECOVERY_OK");
    let inspection = call(&restored_router, "GET", &format!("/api/agent-sessions/{id}/execution"), Value::Null).await;
    assert_eq!(inspection["state"], "completed");
    assert_eq!(inspection["checkpoint_retained"], false);
    assert_eq!(inspection["execution_fence"], 1);
    drop(restored_router);
    second.shutdown_browser_platform().await.unwrap(); second.database.close().await;
}
