//! T23 real App composition: an owner-requested native pause retains its
//! canonical checkpoint while an idle derived Workspace cache is discarded.
use super::*;
use nomifun_agent_execution::{
    AgentExecutionAutomationPort, AgentExecutionSessionPort, AutomationExecutionRequest,
    AutomationExecutionSource,
};
use nomifun_db::IRequirementRepository;

#[derive(Clone)]
struct PausedReadModel {
    requests: Arc<Mutex<Vec<Value>>>,
    before_read: Arc<tokio::sync::Notify>,
    release_read: Arc<tokio::sync::Semaphore>,
}

struct PausedReadServer {
    url: String,
    state: PausedReadModel,
    task: tokio::task::JoinHandle<()>,
}

impl PausedReadServer {
    async fn new() -> Self {
        let state = PausedReadModel {
            requests: Default::default(),
            before_read: Default::default(),
            release_read: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .route("/v1/models", axum::routing::get(|| async {
                axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
            }))
            .route("/v1/chat/completions", axum::routing::post(
                |axum::extract::State(state): axum::extract::State<PausedReadModel>, axum::Json(body): axum::Json<Value>| async move {
                    let call = {
                        let mut requests = state.requests.lock().unwrap();
                        requests.push(body.clone());
                        requests.len()
                    };
                    assert!(!body["messages"].as_array().unwrap().iter().any(|message|
                        message["role"] == "tool" && message["tool_call_id"] == "pause-cache-read-once"),
                        "the real native pause must keep the completed read checkpoint without another model request");
                    let available = body["tools"].as_array().unwrap().iter().any(|tool|
                        tool["function"]["name"] == "read_file");
                    let (name, id, arguments) = if available {
                        state.before_read.notify_one();
                        state.release_read.acquire().await.unwrap().forget();
                        ("read_file", "pause-cache-read-once", json!({"path":"paused-cache-proof.txt"}))
                    } else {
                        ("ToolSearch", "pause-cache-find-read", json!({"query":"workspace.files/read"}))
                    };
                    let delta = json!({"id":format!("paused-cache-{call}"),"object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                            "index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}
                        }]},"finish_reason":null}]});
                    let done = json!({"id":format!("paused-cache-{call}"),"object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                    ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],
                        format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
                },
            ))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { url, state, task }
    }

    async fn close(self) {
        self.state.release_read.add_permits(1);
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn read_api(fixture: &Fixture, path: &str) -> Value {
    let response = fixture._router.clone().oneshot(Request::builder().uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust")
        .body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let body: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert!(status.is_success(), "GET {path}: {status}: {body}");
    body["data"].clone()
}

async fn tool_counts(fixture: &Fixture, session: &str) -> (i64, i64, i64) {
    let reads = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' \
        AND json_extract(inline_json,'$.event.event')='tool_started' AND json_extract(inline_json,'$.event.action_id')='workspace.files/read' \
        AND json_extract(inline_json,'$.event.call_id')='pause-cache-read-once'")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    let all = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' \
        AND json_extract(inline_json,'$.event.event')='tool_started'")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    let starts = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.process/start'")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    (reads, all, starts)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_paused_attempt_and_exact_stop_survive_pure_workspace_cache_retirement_and_factory_failure() {
    use futures_util::FutureExt;

    let server = PausedReadServer::new().await;
    let fixture = Fixture::with_input_model("native-paused-workspace-cache-fault", &server.url).await;
    let verification = std::panic::AssertUnwindSafe(async {
        let owner = fixture.host.options.user_id.clone();
        let setup = fixture.host.options.conversation_id.clone();
        AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(), &owner, &setup,
            "pause-cache-original-setup-stop", "cleanup-retry-turn").await.unwrap();
        fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup, &fixture.runtime).await.unwrap();
        let binding = fixture.owner.canonical().store().get_live_session(&setup.into()).await.unwrap().agent_binding;
        let model = fixture.host.options.model.as_ref().unwrap();
        let created = api(&fixture._router, "/api/agent-sessions", json!({
            "preset_id":binding.preset_revision_ref.preset_id,
            "model":{"provider_id":model.provider_id,"model":model.use_model.as_deref().unwrap_or(&model.model)},
            "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
        })).await;
        let session = AgentSessionId::from(created["agent_session_id"].as_str().unwrap());
        let projection = fixture.owner.get_session(&owner, session.as_ref()).await.unwrap();
        let workspace = PathBuf::from(projection.extra["workspace"].as_str().unwrap());
        std::fs::create_dir_all(&workspace).unwrap();
        let proof = workspace.join("paused-cache-proof.txt");
        std::fs::write(&proof, "PAUSED_ORIGINAL_CACHE_PROOF: preserve the same native task and user bytes.").unwrap();
        fixture.services.requirement_service.create(nomifun_api_types::CreateRequirementRequest {
            title: "Pause the original native task".into(), content: "Read the existing proof once and retain the original task until owner control.".into(),
            tag: "paused-cache".into(), order_key: None, status: None, created_by: None, attachments: vec![],
        }).await.unwrap();
        let requirements = nomifun_db::SqliteRequirementRepository::new(fixture.pool().clone());
        let claim = requirements.claim_next_for_runner("paused-cache", Some(session.as_ref()), None,
            60_000, nomifun_common::now_ms()).await.unwrap().unwrap().row;
        let source_scope = format!("nomifun-autowork-agent-execution-v1\0{}\0{}\0{}", claim.requirement_id,
            claim.claim_generation, claim.claim_token.as_deref().unwrap());
        let admitted = fixture.voice_execution.admit_automation(&owner, AutomationExecutionRequest {
            source: AutomationExecutionSource { requirement_id: claim.requirement_id.clone(), claim_generation: claim.claim_generation,
                operation_id: format!("autowork:{}", nomifun_auth::token_sha256_hex(&source_scope)) },
            lead_session_id: session.as_ref().to_owned(), workspace: Some(workspace.to_string_lossy().into_owned()),
            goal: "Read paused-cache-proof.txt once, then preserve the original task for owner inspection.".into(),
        }).await.unwrap();
        tokio::time::timeout(Duration::from_secs(20), server.state.before_read.notified()).await.unwrap();
        let store = fixture.owner.canonical().store();
        let operation = store.head(&session).await.unwrap().active_turn_id.unwrap();
        let execution_path = format!("/api/agent-sessions/{}/execution", session.as_ref());
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner.clone() };
        let before_pause = read_api(&fixture, &execution_path).await;
        assert_eq!(before_pause["state"], "running");
        api(&fixture._router, &format!("{execution_path}/pause"), json!({"operation_id":operation,
            "idempotency_key":"pause-original-before-read-returns","reason":"Inspect the same checkpoint without resuming work."})).await;
        server.state.release_read.add_permits(1);
        let paused = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let inspection = read_api(&fixture, &execution_path).await;
                if inspection["state"] == "paused" { break inspection; }
                assert_eq!(inspection["state"], "running", "the original owner must reach a real native pause: {inspection}");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await.expect("the original Runtime must save its checkpoint and acknowledge native cleanup before pausing");
        assert_eq!(paused["operation_id"], operation);
        assert_eq!(paused["turn_state"], "running");
        assert_eq!(paused["pause"]["cleanup_proven"], true);
        assert_eq!(paused["checkpoint_retained"], true);
        assert_eq!(paused["budget"], before_pause["budget"], "owner pause does not grant a new work budget");
        let operation_id = OperationId::from(operation.clone());
        let checkpoint = store.load_native_checkpoint(&principal, &session, &operation_id).await.unwrap().unwrap();
        assert_eq!(checkpoint.operation_id, operation_id);
        assert!(checkpoint.revision > 0);
        assert!(checkpoint.state.0["model_steps"].as_u64().unwrap() > 0);
        let receipt = store.read_turn_receipt(&session, &operation_id).await.unwrap();
        let progress = store.read_turn_progress(&session, &operation_id).await.unwrap();
        let cursor = store.current_cursor(&session).await.unwrap();
        let detail = fixture.voice_execution.get(&owner, &admitted.execution_id).await.unwrap();
        assert_eq!(detail.attempts.len(), 1);
        let attempt = &detail.attempts[0];
        assert_eq!(attempt.conversation_id.as_deref(), Some(session.as_ref()));
        let attempts = serde_json::to_value(&detail.attempts).unwrap();
        let model_calls = server.state.requests.lock().unwrap().len();
        let counts = tool_counts(&fixture, session.as_ref()).await;
        // Context/instruction preparation may legitimately read additional
        // files. Identify the one model-authored call by its canonical ID;
        // retain the all-starts baseline to catch any cache-triggered work.
        let read_identities: Vec<(i64, String)> = sqlx::query_as("SELECT seq,json_extract(inline_json,'$.event.call_id') \
            FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' \
              AND json_extract(inline_json,'$.event.event')='tool_started' \
              AND json_extract(inline_json,'$.event.action_id')='workspace.files/read' ORDER BY seq")
            .bind(session.as_ref()).fetch_all(fixture.pool()).await.unwrap();
        println!("NATIVE_PAUSED_CACHE_READ_STARTS session={} calls={read_identities:?} model_calls={model_calls} all_tool_starts={}",
            session.as_ref(), counts.1);
        assert_eq!(counts.0, 1, "the exact model read must run once before the native pause: {read_identities:?}");
        assert_eq!(counts.2, 0, "the read-only fixture never starts a process");
        let inventory = read_api(&fixture, "/api/agent-activities").await;
        let target = inventory["items"].as_array().unwrap().iter().find(|row|
            row["target"]["kind"] == "auto_work_run" && row["target"]["requirement_id"] == claim.requirement_id
                && row["target"]["target_id"] == session.as_ref()).unwrap()["target"].clone();

        let host = &fixture.host.session_host;
        let cached = host.cached_workspace_host_count();
        let ticket = host.capture_completed_workspace_retirement(&session).unwrap()
            .expect("the original real read must materialize its exact derived Workspace owner");
        host.retire_captured_workspace_host(&ticket)
            .expect("only an actually idle derived owner may retire; never force a retained capability");
        assert_eq!(host.cached_workspace_host_count() + 1, cached, "the exact original cache really retired");
        let failure = host.fail_workspace_cache_for_test(&workspace).unwrap();
        assert!(host.probe_workspace_cache_for_test(&workspace).is_err(), "the original cache factory must actually fail at this cutpoint");
        for _ in 0..3 {
            let observed = read_api(&fixture, &execution_path).await;
            for field in ["state", "turn_state", "operation_id", "execution_fence", "execution_generation",
                "checkpoint_revision", "checkpoint_seq", "checkpoint_digest", "checkpoint_retained", "pause", "budget", "model_progress"] {
                assert_eq!(observed[field], paused[field], "cache retirement or factory failure changed native {field}");
            }
            let activities = read_api(&fixture, "/api/agent-activities").await;
            assert!(activities["items"].as_array().unwrap().iter().any(|row| row["target"] == target && row["can_stop"] == true),
                "host-side user Stop remains reachable while the derived cache factory fails");
        }
        assert_eq!(store.load_native_checkpoint(&principal, &session, &operation_id).await.unwrap(), Some(checkpoint));
        assert_eq!(store.read_turn_receipt(&session, &operation_id).await.unwrap(), receipt);
        assert_eq!(store.read_turn_progress(&session, &operation_id).await.unwrap(), progress);
        assert_eq!(store.current_cursor(&session).await.unwrap(), cursor);
        let current = fixture.voice_execution.get(&owner, &admitted.execution_id).await.unwrap();
        assert_eq!(serde_json::to_value(&current.attempts).unwrap(), attempts);
        assert_eq!(current.execution.event_sequence, detail.execution.event_sequence);
        assert_eq!(server.state.requests.lock().unwrap().len(), model_calls);
        assert_eq!(tool_counts(&fixture, session.as_ref()).await, counts);
        let source = requirements.get_by_requirement_id(&claim.requirement_id).await.unwrap().unwrap();
        assert_eq!(source.status, claim.status);
        assert_eq!(source.claim_generation, claim.claim_generation);
        assert_eq!(source.attempt_count, claim.attempt_count);

        let stopped = api(&fixture._router, "/api/agent-activities/stop", json!({"target":target,
            "idempotency_key":"stop-the-original-native-paused-attempt-during-cache-failure"})).await;
        assert_eq!(stopped["accepted"], true);
        let joined = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let receipt = store.read_turn_receipt(&session, &operation_id).await.unwrap();
                let carrier = fixture.services.agent_runtime_sessions.observe_runtime_carrier(session.as_ref());
                if receipt.status == nomifun_agent_session::TurnReceiptStatus::Cancelled
                    && !carrier.registered && !carrier.teardown_pending && carrier.active_generation.is_none() { break; }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await;
        assert!(joined.is_ok(), "user Stop must cancel the same paused native Turn and join its original Runtime owner; receipt={:?}, carrier={:?}",
            store.read_turn_receipt(&session, &operation_id).await.unwrap().status,
            fixture.services.agent_runtime_sessions.observe_runtime_carrier(session.as_ref()));
        let stopped = fixture.voice_execution.get(&owner, &admitted.execution_id).await.unwrap();
        assert_eq!(stopped.attempts.len(), 1);
        assert_eq!(stopped.attempts[0].attempt_id, attempt.attempt_id);
        assert_eq!(stopped.attempts[0].status, nomifun_common::ExecutionAttemptStatus::Cancelled);
        assert_eq!(server.state.requests.lock().unwrap().len(), model_calls);
        assert_eq!(tool_counts(&fixture, session.as_ref()).await, counts);
        let turns: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
            .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        assert_eq!(turns, 1, "cache faults and Stop never submit replacement input");
        assert!(std::fs::read_to_string(&proof).unwrap().contains("PAUSED_ORIGINAL_CACHE_PROOF"));
        drop(failure);
    }).catch_unwind().await;
    fixture.finish().await;
    server.close().await;
    if let Err(reason) = verification { std::panic::resume_unwind(reason); }
}
