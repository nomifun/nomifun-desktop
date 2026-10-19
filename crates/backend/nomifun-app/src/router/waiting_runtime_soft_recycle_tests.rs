//! T20: real App retirement replaces only the Runtime transport. A live Bun
//! process keeps its original Session owner and accepts explicit later input.
#![cfg(windows)]

use super::*;
use futures_util::FutureExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore};

#[derive(Clone)]
struct RecycleModel {
    requests: Arc<Mutex<Vec<Value>>>,
    discoveries: Arc<AtomicUsize>,
    launch: Arc<OnceLock<Value>>,
    started: Arc<OnceLock<Value>>,
    input: Arc<OnceLock<Value>>,
    polled: Arc<OnceLock<Value>>,
    initial_ready: Arc<Notify>,
    followup_ready: Arc<Notify>,
    initial_finish: Arc<Semaphore>,
    followup_finish: Arc<Semaphore>,
}

struct RecycleServer {
    url: String,
    state: RecycleModel,
    task: tokio::task::JoinHandle<()>,
}

fn recycle_tool(state: &RecycleModel, body: &Value, name: &str, id: &str, action: &str, input: Value) -> Value {
    if writer_tool_available(body, name) {
        writer_tool(name, id, input)
    } else {
        state.discoveries.fetch_add(1, Ordering::SeqCst);
        writer_tool("ToolSearch", &format!("find-{id}"), json!({"query":action}))
    }
}

fn recycle_result(body: &Value, id: &str) -> Option<Value> {
    writer_result(body, id).map(|result| {
        let content = result["content"].as_str().unwrap();
        assert!(!content.starts_with("[tool error]"),
            "the exact process call {id} was rejected; do not substitute a native result or replay it: {content}");
        serde_json::from_str(content).unwrap_or_else(|error|
            panic!("the original process call {id} did not return its expected native JSON: {error}; {content}"))
    })
}

impl RecycleServer {
    async fn new() -> Self {
        let state = RecycleModel {
            requests: Default::default(), discoveries: Default::default(), launch: Default::default(), started: Default::default(),
            input: Default::default(), polled: Default::default(),
            initial_ready: Default::default(), followup_ready: Default::default(),
            initial_finish: Arc::new(Semaphore::new(0)), followup_finish: Arc::new(Semaphore::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let routes = axum::Router::new()
            .route("/v1/models", axum::routing::get(|| async {
                axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
            }))
            .route("/v1/chat/completions", axum::routing::post(
                |axum::extract::State(state): axum::extract::State<RecycleModel>, axum::Json(body): axum::Json<Value>| async move {
                    let call = { let mut calls = state.requests.lock().unwrap(); calls.push(body.clone()); calls.len() };
                    let followup = body["messages"].as_array().unwrap().iter()
                        .any(|message| message["role"] == "user" && message["content"].to_string().contains("RECYCLE_FOLLOWUP"));
                    let question = if followup { "recycle-next-question" } else { "recycle-initial-question" };
                    assert!(writer_result(&body, question).is_none(),
                        "a typed user-input handoff must close this Turn without another model request");
                    let (delta, reason) = if followup {
                        let original = state.started.get().expect("the later user request follows the original launch");
                        let process = original["process_id"].as_str().unwrap();
                        if let Some(polled) = recycle_result(&body, "recycle-poll") {
                            assert_eq!(polled["process_id"], process);
                            assert_eq!(polled["pid"], original["pid"]);
                            assert_eq!(polled["state"], "running");
                            assert!(polled["output"]["text"].as_str().unwrap().contains("RECYCLE_PING"),
                                "the original process must consume input after the carrier is replaced: {polled}");
                            state.polled.set(polled).unwrap();
                            state.followup_ready.notify_one();
                            state.followup_finish.acquire().await.unwrap().forget();
                            assert!(writer_tool_available(&body, "request_user_input"));
                            (writer_tool("request_user_input", question, json!({
                                "question":"The original process consumed RECYCLE_PING. What should it receive next?",
                                "remaining_work":"Keep the same background process and its original owner alive while awaiting the user's next input."
                            })), "tool_calls")
                        } else if let Some(input) = recycle_result(&body, "recycle-input") {
                            assert_eq!(input["state"], "running");
                            assert_eq!(input["process_id"], process);
                            assert_eq!(input["pid"], original["pid"]);
                            let _ = state.input.set(input);
                            (recycle_tool(&state, &body, "poll_process", "recycle-poll", "workspace.process/poll",
                                json!({"process_id":process,"cursor":0,"wait_ms":1000})), "tool_calls")
                        } else {
                            (recycle_tool(&state, &body, "write_process_stdin", "recycle-input", "workspace.process/input",
                                json!({"process_id":process,"input":"RECYCLE_PING\n"})), "tool_calls")
                        }
                    } else if let Some(started) = recycle_result(&body, "recycle-start") {
                        assert_eq!(started["state"], "running");
                        assert!(started["pid"].as_u64().unwrap() > 0);
                        state.started.set(started).unwrap();
                        state.initial_ready.notify_one();
                        state.initial_finish.acquire().await.unwrap().forget();
                        assert!(writer_tool_available(&body, "request_user_input"));
                        (writer_tool("request_user_input", question, json!({
                            "question":"The background process is running. What input should it receive?",
                            "remaining_work":"Keep the original background process alive and control that same process when the user supplies the next input."
                        })), "tool_calls")
                    } else {
                        (recycle_tool(&state, &body, "start_process", "recycle-start", "workspace.process/start",
                            state.launch.get().unwrap().clone()), "tool_calls")
                    };
                    let chunk = json!({"id":format!("recycle-{call}"),"object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                    let end = json!({"id":format!("recycle-{call}"),"object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                    ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],
                        format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
                },
            )).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap(); });
        Self { url, state, task }
    }

    async fn close(self) { self.task.abort(); let _ = self.task.await; }
}

/// One observation-only native handle is retained across both carriers. This
/// proves continuity of the original process object, independent of PID reuse.
struct OriginalProcess(windows_sys::Win32::Foundation::HANDLE);

impl OriginalProcess {
    fn open(pid: u32) -> Self {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE};
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null(), "open exact fixture process: {}", std::io::Error::last_os_error());
        Self(handle)
    }
    fn assert_alive(&self, pid: u32) {
        use windows_sys::Win32::{Foundation::{STILL_ACTIVE, WAIT_TIMEOUT}, System::Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject}};
        assert_eq!(unsafe { GetProcessId(self.0) }, pid);
        let mut status = 0;
        assert_ne!(unsafe { GetExitCodeProcess(self.0, &mut status) }, 0);
        assert_eq!(status, STILL_ACTIVE as u32);
        assert_eq!(unsafe { WaitForSingleObject(self.0, 0) }, WAIT_TIMEOUT,
            "the original native process object must survive transport retirement");
    }
    async fn wait_exited(&self) {
        use windows_sys::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
        tokio::time::timeout(Duration::from_secs(10), async {
            while unsafe { WaitForSingleObject(self.0, 0) } != WAIT_OBJECT_0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("exact user Stop must reap the original process object");
    }
}

impl Drop for OriginalProcess {
    fn drop(&mut self) { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); } }
}

fn captured_host(session: &str) -> Weak<ConversationRuntimeHost> {
    HOSTS.get().unwrap().lock().unwrap().get(session).unwrap().clone()
}

async fn retired_transport(fixture: &Fixture, session: &str, operation: &str, host: &Weak<ConversationRuntimeHost>) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let progress = fixture.owner.canonical().store().read_turn_progress(&session.into(), &operation.into()).await.unwrap();
            let carrier = fixture.services.agent_runtime_sessions.observe_runtime_carrier(session);
            if progress.status == nomifun_agent_session::TurnReceiptStatus::Completed
                && !carrier.registered && !carrier.teardown_pending && carrier.instance_id.is_none()
                && carrier.active_generation.is_none() && carrier.constructing_generation.is_none() && host.strong_count() == 0 { break; }
            assert!(!matches!(progress.status, nomifun_agent_session::TurnReceiptStatus::Failed | nomifun_agent_session::TurnReceiptStatus::Cancelled),
                "the actual Turn must complete normally: {progress:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("the actual completed carrier and Host must retire automatically while its Session process stays alive");
}

async fn assert_input_handoff(fixture: &Fixture, session: &AgentSessionId, operation: &str, request_id: &str) {
    let store = fixture.owner.canonical().store();
    let operation = OperationId::from(operation);
    let receipt = store.read_turn_receipt(session, &operation).await.unwrap();
    assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Completed);
    let facts = store.turn_output_facts(session, &operation).await.unwrap();
    let delivery = nomifun_agent_execution::canonical_turn_delivery(&facts, &receipt, true).unwrap();
    assert_eq!(delivery.user_input_request.as_ref().unwrap().request_id.as_ref(), request_id,
        "the original native owner must commit a real input handoff instead of accepting a plain text final while a process remains live");
    let corrections: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events event LEFT JOIN agent_payloads payload \
        ON payload.session_id=event.session_id AND payload.payload_id=event.payload_id \
        WHERE event.session_id=? AND event.kind='runtime/progress-recorded' \
        AND (COALESCE(json_extract(event.inline_json,'$.event.event'),json_extract(CAST(payload.body AS TEXT),'$.value.event.event')) \
            IN ('end_intent_review','model_response_rejected') \
          OR COALESCE(json_extract(event.inline_json,'$.event.runtime_hold'),json_extract(CAST(payload.body AS TEXT),'$.value.event.runtime_hold'))='arguments_rejected')")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(corrections, 0, "the fixture uses valid native arguments and formal input handoff directly, without protocol repair");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_native_bun_process_survives_actual_runtime_retirement_and_explicit_successor_control() {
    let server = RecycleServer::new().await;
    let fixture = Fixture::with_input_model("native-process-real-runtime-soft-recycle", &server.url).await;
    let verification = std::panic::AssertUnwindSafe(async {
        let owner = fixture.host.options.user_id.clone();
        let setup = fixture.host.options.conversation_id.clone();
        nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(), &owner,
            &setup, "recycle-setup-stop", "cleanup-retry-turn").await.unwrap();
        fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup, &fixture.runtime).await.unwrap();
        let selected = fixture.host.options.model.as_ref().unwrap();
        let model = json!({"provider_id":selected.provider_id,"model":selected.use_model.as_deref().unwrap_or(&selected.model)});
        let preset = api(&fixture._router, "/api/agent-presets/from-template/coding.codex",
            json!({"reuse_existing":false,"display_name":"Original process soft recycle","model":model})).await;
        let created = api(&fixture._router, "/api/agent-sessions", json!({"preset_id":preset["preset"]["preset_id"],"model":model,
            "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await;
        let session = created["agent_session_id"].as_str().unwrap().to_owned();
        let id = AgentSessionId::from(session.clone());
        let principal = nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(), principal_id:owner.clone() };
        let projection = fixture.owner.get_session(&owner, &session).await.unwrap();
        let snapshot = projection.agent_snapshot.as_ref().unwrap();
        let process_actions = snapshot.enabled_capability_actions.get("workspace.process").unwrap();
        for action in ["workspace.process/start", "workspace.process/input", "workspace.process/poll"] {
            assert!(process_actions.iter().any(|actual| actual == action), "use the original official template grants: {action}");
        }
        let workspace = PathBuf::from(projection.extra["workspace"].as_str().unwrap());
        let script = workspace.join("original-process.mjs");
        let launches = workspace.join("original-launches.jsonl");
        let token = nomifun_common::generate_id();
        std::fs::write(&script, "import {appendFileSync} from 'node:fs';const token=process.argv[3];appendFileSync(process.argv[2],JSON.stringify({pid:process.pid,token})+'\\n');process.stdout.write('READY:'+token+':'+process.pid+'\\n');process.stdin.setEncoding('utf8');process.stdin.on('data',input=>process.stdout.write('ACK:'+token+':'+process.pid+':'+input));process.stdin.resume();\n").unwrap();
        server.state.launch.set(json!({"command":fixture_bun(),"args":[script,launches,token],
            "execution_identity":"current_user","tty":false,"wait_ms":1000})).unwrap();
        let initial = api(&fixture._router, &format!("/api/agent-sessions/{session}/turns"),
            json!({"idempotency_key":"original-native-launch","input":{"content":"Start this existing fixture program once in the background. Keep it alive and ask me which input to send next; do not declare the live process finished or stop it."}})).await;
        let operation = initial["operation_id"].as_str().unwrap().to_owned();
        tokio::time::timeout(Duration::from_secs(25), server.state.initial_ready.notified()).await.unwrap();
        let first_carrier = fixture.services.agent_runtime_sessions.observe_runtime_carrier(&session);
        assert!(first_carrier.registered && first_carrier.active_generation.is_some());
        let first_nonce = first_carrier.instance_id.clone().unwrap();
        let first_host = captured_host(&session);
        assert!(first_host.strong_count() > 0);
        let start = server.state.started.get().unwrap();
        let process_id = start["process_id"].as_str().unwrap().to_owned();
        let pid = u32::try_from(start["pid"].as_u64().unwrap()).unwrap();
        let native = OriginalProcess::open(pid);
        native.assert_alive(pid);
        let original = fixture.host.session_host.process_resource_instances(&owner, &id).unwrap().into_iter()
            .find(|process| process.process_id == process_id).unwrap();
        assert_eq!(original.creator_turn_id, operation);
        assert!(!original.stop_requested && !original.execution_stopped);
        let initial_calls = server.state.requests.lock().unwrap().len();
        let initial_discoveries = server.state.discoveries.load(Ordering::SeqCst);
        assert_eq!(initial_calls, 2 + initial_discoveries, "only one launch/result round plus formal deferred discovery");
        server.state.initial_finish.add_permits(1);
        retired_transport(&fixture, &session, &operation, &first_host).await;
        assert_input_handoff(&fixture, &id, &operation, "recycle-initial-question").await;
        assert_eq!(server.state.requests.lock().unwrap().len(), initial_calls, "retirement cannot ask the model to recreate the process");
        native.assert_alive(pid);
        let store = fixture.owner.canonical().store();
        let retention = store.session_process_owner_retention(&principal, &id, &original.owner_generation).await.unwrap().unwrap();
        assert_eq!(retention.processes.len(), 1);
        let retained = &retention.processes[0];
        assert_eq!(retained.process_id, process_id); assert_eq!(retained.pid, pid);
        assert_eq!(retained.creator_turn_id.as_ref(), original.creator_turn_id);
        assert_eq!(retained.creator_call_id.as_ref(), original.creator_call_id);
        let binding = snapshot.canonical_binding.as_ref().unwrap();
        assert_eq!(serde_json::to_value(&retained.resolved_snapshot_ref).unwrap(), serde_json::to_value(&binding.resolved_snapshot_ref).unwrap());
        let frozen_resources: nomifun_agent_contracts::TypedResourceBindings = serde_json::from_value(serde_json::to_value(
            binding.typed_resource_bindings.iter().filter(|resource| resource.resource_kind == "process_session").collect::<Vec<_>>()
        ).unwrap()).unwrap();
        let resources = frozen_resources.iter().map(|resource|
            crate::router::nomi_core_wave2::session_process_binding(workspace.to_str().unwrap(), &principal, &id, resource).unwrap()
        ).collect::<nomifun_agent_contracts::TypedResourceBindings>();
        assert_eq!(resources.len(), 1, "the original official process owner has one exact resource binding");
        assert_eq!(retained.resource_binding_digest, nomifun_agent_contracts::digest_payload(&resources).unwrap());
        let before = store.read_turn_receipt(&id, &operation.as_str().into()).await.unwrap();

        let followup = api(&fixture._router, &format!("/api/agent-sessions/{session}/turns"), json!({
            "idempotency_key":"explicit-original-process-input","input":{"content":"RECYCLE_FOLLOWUP: send RECYCLE_PING to the original background process stdin, then poll that same process. Keep it alive and ask me for its next input. Do not start another program or declare the live process finished."}})).await;
        let followup_operation = followup["operation_id"].as_str().unwrap().to_owned();
        tokio::time::timeout(Duration::from_secs(25), server.state.followup_ready.notified()).await.unwrap();
        let explicit_discoveries = server.state.discoveries.load(Ordering::SeqCst) - initial_discoveries;
        assert_eq!(server.state.requests.lock().unwrap().len(), initial_calls + 3 + explicit_discoveries,
            "only the explicit input/result/poll round and formal discovery may invoke the model after retirement");
        let successor = fixture.services.agent_runtime_sessions.observe_runtime_carrier(&session);
        assert!(successor.registered && successor.active_generation.is_some());
        assert_ne!(successor.instance_id.as_ref().unwrap(), &first_nonce);
        assert_ne!(successor.active_generation, first_carrier.active_generation);
        let successor_host = captured_host(&session);
        assert!(!Weak::ptr_eq(&first_host, &successor_host)); assert_eq!(first_host.strong_count(), 0);
        native.assert_alive(pid);
        let current = fixture.host.session_host.process_resource_instances(&owner, &id).unwrap().into_iter()
            .find(|process| process.process_id == process_id).unwrap();
        assert_eq!(current.owner_generation, original.owner_generation);
        assert_eq!(current.creator_turn_id, original.creator_turn_id); assert_eq!(current.creator_call_id, original.creator_call_id);
        assert!(!current.stop_requested && !current.execution_stopped);
        let polled = server.state.polled.get().unwrap();
        assert!(polled["output"]["text"].as_str().unwrap().contains(&format!("ACK:{token}:{pid}:RECYCLE_PING")));
        assert_eq!(store.read_turn_receipt(&id, &operation.as_str().into()).await.unwrap(), before);
        let after = store.session_process_owner_retention(&principal, &id, &original.owner_generation).await.unwrap().unwrap();
        assert_eq!(after, retention, "a successor carrier cannot change the original process's frozen creator/authority");
        server.state.followup_finish.add_permits(1);
        retired_transport(&fixture, &session, &followup_operation, &successor_host).await;
        assert_input_handoff(&fixture, &id, &followup_operation, "recycle-next-question").await;
        native.assert_alive(pid);
        let turns: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
            .bind(&session).fetch_one(fixture.pool()).await.unwrap();
        assert_eq!(turns, 2, "only the two explicit user requests create Turns");
        for action in ["workspace.process/start", "workspace.process/input", "workspace.process/poll"] {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='tool/call-started' AND json_extract(inline_json,'$.action_id')=?")
                .bind(&session).bind(action).fetch_one(fixture.pool()).await.unwrap();
            assert_eq!(count, 1, "recycling never replays original process calls: {action}");
        }
        let starts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.process/start'")
            .bind(&session).fetch_one(fixture.pool()).await.unwrap();
        assert_eq!(starts, 1, "the original mutation Effect is not replayed");
        let lines = std::fs::read_to_string(&launches).unwrap();
        assert_eq!(lines.lines().count(), 1, "the original native command executes exactly once");
        let launch: Value = serde_json::from_str(lines.trim()).unwrap();
        assert_eq!(launch["pid"], pid); assert_eq!(launch["token"], token);
        let calls = server.state.requests.lock().unwrap().len();
        let stopped = api(&fixture._router, "/api/agent-activities/stop", json!({"target":{"kind":"process_instance",
            "agent_session_id":session,"owner_generation":original.owner_generation,"process_id":process_id,
            "creator_turn_id":original.creator_turn_id,"creator_call_id":original.creator_call_id},"idempotency_key":"stop-original-after-real-recycle"})).await;
        assert_eq!(stopped["accepted"], true);
        native.wait_exited().await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !store.session_process_resource_stopped(&principal, &id, &original.owner_generation, &process_id,
                &original.creator_turn_id, &original.creator_call_id).await.unwrap() { tokio::task::yield_now().await; }
        }).await.expect("the original native Stop must also commit its exact canonical reap receipt");
        assert_eq!(server.state.requests.lock().unwrap().len(), calls);
        assert!(store.get_live_session(&id).await.is_ok()); assert!(script.is_file() && launches.is_file());
        println!("NATIVE_RUNTIME_SOFT_RECYCLE original_nonce={first_nonce} successor_nonce={} process_id={process_id} pid={pid} original_creator={} original_native_handle_alive_across_recycle=true process_starts=1 explicit_inputs=1 exact_stop=true", successor.instance_id.unwrap(), original.creator_call_id);
    }).catch_unwind().await;
    fixture.finish().await;
    server.close().await;
    if let Err(reason) = verification { std::panic::resume_unwind(reason); }
}
