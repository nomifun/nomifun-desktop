//! T09/T23 real App composition: a native background writer survives a closed
//! InputRequired Turn and a controlled derived-cache fault. The HTTP model is
//! scripted; native process, canonical facts, scheduler and controls are real.
use super::*;
use nomifun_agent_execution::{
    AgentExecutionAutomationPort, AutomationExecutionRequest, AutomationExecutionSource,
};
use nomifun_db::IRequirementRepository;

#[path = "waiting_runtime_soft_recycle_tests.rs"]
mod soft_recycle_tests;

#[derive(Clone)]
struct WaitingWriterModel {
    requests: Arc<Mutex<Vec<Value>>>,
    launch: Arc<OnceLock<Value>>,
    process: Arc<OnceLock<String>>,
}
struct WaitingWriterServer {
    url: String,
    state: WaitingWriterModel,
    task: tokio::task::JoinHandle<()>,
}

fn writer_tool(name: &str, id: &str, arguments: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]})
}
fn writer_tool_available(body: &Value, name: &str) -> bool {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["function"]["name"] == name)
}
fn writer_result<'a>(body: &'a Value, id: &str) -> Option<&'a Value> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
}
impl WaitingWriterServer {
    async fn new() -> Self {
        let state = WaitingWriterModel {
            requests: Default::default(),
            launch: Default::default(),
            process: Default::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let router=axum::Router::new().route("/v1/models",axum::routing::get(||async {axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))}))
            .route("/v1/chat/completions",axum::routing::post(|axum::extract::State(state):axum::extract::State<WaitingWriterModel>,axum::Json(body):axum::Json<Value>|async move {
                let call={let mut requests=state.requests.lock().unwrap();requests.push(body.clone());requests.len()};
                assert!(writer_result(&body,"waiting-writer-question").is_none(),"InputRequired must close before another model step");
                let delta=if let Some(result)=writer_result(&body,"waiting-writer-start") {
                    let started:Value=serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
                    let process=started["process_id"].as_str().expect("original native process identity must reach the model");
                    if let Some(original)=state.process.get(){assert_eq!(original,process);}else{state.process.set(process.into()).unwrap();}
                    writer_tool("request_user_input","waiting-writer-question",json!({"question":"Which report should the existing writer produce next?","remaining_work":"Use the original background writer after the user answers."}))
                } else if let Some(result)=writer_result(&body,"waiting-writer-read") {
                    assert!(result["content"].as_str().unwrap().contains("setInterval"),"the real file read must materialize the original derived workspace owner");
                    if writer_tool_available(&body,"start_process") {writer_tool("start_process","waiting-writer-start",state.launch.get().unwrap().clone())}
                    else{writer_tool("ToolSearch","waiting-find-process",json!({"query":"workspace.process/start"}))}
                } else if writer_tool_available(&body,"read_file") {writer_tool("read_file","waiting-writer-read",json!({"path":"waiting-writer.mjs"}))}
                else{writer_tool("ToolSearch","waiting-find-read",json!({"query":"workspace.files/read"}))};
                let chunk=json!({"id":format!("waiting-writer-{call}"),"object":"chat.completion.chunk","created":1,"model":"cleanup-fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end=json!({"id":format!("waiting-writer-{call}"),"object":"chat.completion.chunk","created":1,"model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
            })).with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { url, state, task }
    }
    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

fn fixture_bun() -> PathBuf {
    let supplied = std::env::var_os("NOMIFUN_BUN_PATH").map(PathBuf::from);
    #[cfg(windows)]
    let name = "bun.exe";
    #[cfg(not(windows))]
    let name = "bun";
    let path=supplied.or_else(||std::env::var_os("PATH").and_then(|paths|std::env::split_paths(&paths).map(|path|path.join(name)).find(|path|path.is_file())))
        .expect("this native writer fixture requires existing Bun; supply NOMIFUN_BUN_PATH, never install it in a test");
    assert!(path.is_file());
    std::fs::canonicalize(path).unwrap()
}
async fn writer_grows(path: &std::path::Path, after: u64) -> u64 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            if size > after {
                return size;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the original native writer must continue producing real file bytes")
}

async fn waiting_writer_survives(cache_fault: bool) {
    use futures_util::FutureExt;
    let server = WaitingWriterServer::new().await;
    let fixture = Fixture::with_process(
        if cache_fault {
            "waiting-cache-fault-native-writer"
        } else {
            "waiting-native-writer-reservation"
        },
        &server.url,
    )
    .await;
    let verification=std::panic::AssertUnwindSafe(async {
        let owner=fixture.host.options.user_id.clone();let setup=fixture.host.options.conversation_id.clone();
        nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(),&owner,&setup,"waiting-setup-stop","cleanup-retry-turn").await.unwrap();
        fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup,&fixture.runtime).await.unwrap();
        let binding=fixture.owner.canonical().store().get_live_session(&setup.clone().into()).await.unwrap().agent_binding;
        let model=fixture.host.options.model.as_ref().unwrap();
        let lead=api(&fixture._router,"/api/agent-sessions",json!({"preset_id":binding.preset_revision_ref.preset_id,
            "model":{"provider_id":model.provider_id,"model":model.use_model.as_deref().unwrap_or(&model.model)},
            "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await["agent_session_id"].as_str().unwrap().to_owned();
        let projection=fixture.owner.get_session(&owner,&lead).await.unwrap();let workspace=PathBuf::from(projection.extra["workspace"].as_str().unwrap());
        std::fs::create_dir_all(&workspace).unwrap();let script=workspace.join("waiting-writer.mjs");let output=workspace.join("waiting-writer.log");
        std::fs::write(&script,"import {appendFileSync} from 'node:fs';const target=process.argv[2];appendFileSync(target,'ready\\n');setInterval(()=>appendFileSync(target,'tick\\n'),25);\n").unwrap();
        server.state.launch.set(json!({"command":fixture_bun(),"args":[script,output],"execution_identity":"current_user","tty":false,"wait_ms":0})).unwrap();
        fixture.services.requirement_service.create(nomifun_api_types::CreateRequirementRequest {title:"Existing writer waits for the user".into(),content:"Start the original writer once, then ask the user.".into(),
            tag:"waiting-writer".into(),order_key:None,status:None,created_by:None,attachments:vec![]}).await.unwrap();
        let requirements=nomifun_db::SqliteRequirementRepository::new(fixture.pool().clone());
        let claim=requirements.claim_next_for_runner("waiting-writer",Some(&lead),None,30_000,nomifun_common::now_ms()).await.unwrap().unwrap().row;
        let source=AutomationExecutionSource {requirement_id:claim.requirement_id.clone(),claim_generation:claim.claim_generation,
            operation_id:format!("autowork:{}",nomifun_auth::token_sha256_hex(&format!("nomifun-autowork-agent-execution-v1\0{}\0{}\0{}",claim.requirement_id,claim.claim_generation,claim.claim_token.as_ref().unwrap())))};
        let admitted=fixture.voice_execution.admit_automation(&owner,AutomationExecutionRequest {source:source.clone(),lead_session_id:lead.clone(),goal:"Read the writer script, start it once, then ask the user for the next report.".into(),workspace:Some(workspace.to_string_lossy().into())}).await.unwrap();
        let waiting=tokio::time::timeout(Duration::from_secs(25),async {loop {
            let detail=fixture.voice_execution.get(&owner,&admitted.execution_id).await.unwrap();
            if detail.execution.status==nomifun_common::AgentExecutionStatus::WaitingInput {break detail;}
            assert!(!detail.execution.status.is_terminal(),"the real native handoff unexpectedly failed: {detail:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.expect("real process start plus request_user_input must reach Attempt WaitingInput");
        assert_eq!(waiting.attempts.len(),1);let attempt=&waiting.attempts[0];assert_eq!(attempt.status,nomifun_common::ExecutionAttemptStatus::WaitingInput);
        let session:AgentSessionId=lead.clone().into();let operation:OperationId=format!("turn:user:{owner}:{lead}:{}:initial-turn",attempt.attempt_id).into();
        let store=fixture.owner.canonical().store();let receipt=store.read_turn_receipt(&session,&operation).await.unwrap();
        assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);let terminal=receipt.terminal_event.as_ref().unwrap();
        let facts=store.turn_output_facts(&session,&operation).await.unwrap();let delivery=nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,true).unwrap();
        assert_eq!(delivery.user_input_request.as_ref().unwrap().request_id.as_ref(),"waiting-writer-question");
        let mut tx=fixture.pool().begin().await.unwrap();let reservations=nomifun_db::execution_reservations_tx(&mut tx,&admitted.execution_id).await.unwrap();tx.commit().await.unwrap();
        assert_eq!(reservations.len(),1);assert_eq!(reservations[0].attempt_id,attempt.attempt_id);
        assert!(reservations[0].workspace_held&&reservations[0].workspace_mutator,"Full policy retains the legitimate writer's workspace responsibility");
        assert!(!reservations[0].capacity_held,"a typed closed InputRequired handoff releases only model capacity");
        let first=writer_grows(&output,0).await;let host=fixture.host.session_host.clone();
        let process=host.process_resource_instances(&owner,&session).unwrap().into_iter().find(|process|process.process_id==*server.state.process.get().unwrap()).unwrap();
        assert!(!process.execution_stopped&&!process.stop_requested);assert_eq!(process.creator_turn_id,operation.as_ref());
        let model_calls=server.state.requests.lock().unwrap().len();
        // Observe dozens of actual writes across normal watcher/scheduler
        // ticks, instead of stopping immediately after the initial spawn ACK.
        let first=writer_grows(&output,first+256).await;
        assert_eq!(server.state.requests.lock().unwrap().len(),model_calls,"background file changes cannot resume an unanswered handoff");
        let stable=fixture.voice_execution.get(&owner,&admitted.execution_id).await.unwrap();
        assert_eq!(stable.attempts.len(),1);assert_eq!(stable.attempts[0].attempt_id,attempt.attempt_id);
        let before=store.read_turn_progress(&session,&operation).await.unwrap();
        let mut failure=None;
        if cache_fault {
            if let Some(ticket)=host.capture_completed_workspace_retirement(&session).unwrap(){host.retire_captured_workspace_host(&ticket).unwrap();}
            failure=Some(host.fail_workspace_cache_for_test(&workspace).unwrap());
            assert!(host.probe_workspace_cache_for_test(&workspace).is_err(),"the actual derived-cache factory must be unavailable in this cutpoint");
            let response=fixture._router.clone().oneshot(Request::builder().uri("/api/agent-activities").header("x-nomi-local-trust","cleanup-fixture-local-trust").body(Body::empty()).unwrap()).await.unwrap();
            assert!(response.status().is_success(),"read-only work discovery/control cannot depend on rehydrating the failed derived cache");
        }
        let second=writer_grows(&output,first+256).await;assert!(second>first);
        assert_eq!(server.state.requests.lock().unwrap().len(),model_calls,"waiting, refresh and cache faults cannot make another model request");
        assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap(),receipt,"the original InputRequired terminal/cursor stays immutable");
        assert_eq!(store.read_turn_progress(&session,&operation).await.unwrap(),before,"the pure derived cache cannot write a Session cursor");
        let current=fixture.voice_execution.get(&owner,&admitted.execution_id).await.unwrap();assert_eq!(current.attempts.len(),1);assert_eq!(current.attempts[0].attempt_id,attempt.attempt_id);
        assert_eq!(current.attempts[0].status,nomifun_common::ExecutionAttemptStatus::WaitingInput);assert_eq!(current.execution.event_sequence,stable.execution.event_sequence);
        let starts:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.process/start'").bind(&lead).fetch_one(fixture.pool()).await.unwrap();
        assert_eq!(starts,1,"the same writer is not restarted to compensate for a cache miss");
        let stopped=api(&fixture._router,"/api/agent-activities/stop",json!({"target":{"kind":"process_instance","agent_session_id":lead,"owner_generation":process.owner_generation,
            "process_id":process.process_id,"creator_turn_id":process.creator_turn_id,"creator_call_id":process.creator_call_id},"idempotency_key":"stop-original-waiting-writer"})).await;
        assert_eq!(stopped["accepted"],true,"host-side exact Stop remains reachable while the derived cache is unavailable");
        tokio::time::timeout(Duration::from_secs(10),async {loop {if store.session_process_resource_stopped(&nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:owner.clone()},&session,
            &process.owner_generation,&process.process_id,&process.creator_turn_id,&process.creator_call_id).await.unwrap(){break;}tokio::task::yield_now().await;}}).await.expect("original native process Stop must settle its canonical receipt");
        assert!(output.is_file()&&script.is_file(),"resource Stop preserves real workspace data");
        let stopped=api(&fixture._router,"/api/agent-activities/stop",json!({"target":{"kind":"auto_work_run","target_kind":"conversation","target_id":lead,"requirement_id":claim.requirement_id,"claim_generation":claim.claim_generation},"idempotency_key":"stop-original-waiting-claim"})).await;
        assert_eq!(stopped["accepted"],true);drop(failure);
        let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(&lead).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,1);
        assert_eq!(server.state.requests.lock().unwrap().len(),model_calls);
        println!("WAITING_NATIVE_WRITER cache_fault={cache_fault} workspace_held=true capacity_held=false attempts=1 turns=1 process_starts=1 original_terminal_seq={} model_calls={model_calls} exact_stop=true",terminal.seq);
    }).catch_unwind().await;
    fixture.finish().await;
    server.close().await;
    if let Err(reason) = verification {
        std::panic::resume_unwind(reason);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn input_required_retains_real_background_writer_and_workspace_but_yields_model_capacity() {
    waiting_writer_survives(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waiting_writer_and_exact_stop_survive_derived_cache_retirement_and_load_failure() {
    waiting_writer_survives(true).await;
}
