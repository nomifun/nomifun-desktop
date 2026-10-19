//! Real SDK completion while post-answer Knowledge work remains independently owned.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use nomifun_knowledge::KnowledgeCompleter;

const ORIGINAL: &str = "cleanup-retry-turn";

struct ModelServer {
    url: String,
    calls: Arc<AtomicUsize>,
    gate: Arc<Semaphore>,
    task: tokio::task::JoinHandle<()>,
}

impl ModelServer {
    async fn new(block_call: usize) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let counted = calls.clone();
        let waiting = gate.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let read = socket.read(&mut chunk).await.unwrap();
                    if read == 0 { break None; }
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") { break Some(index + 4); }
                };
                let Some(header_end) = header_end else { continue; };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = headers.lines().filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap()).unwrap_or(0);
                while bytes.len() < header_end + length {
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0); bytes.extend_from_slice(&chunk[..read]);
                }
                if headers.starts_with("GET ") {
                    let body = r#"{"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}"#;
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                    continue;
                }
                let call = counted.fetch_add(1, Ordering::SeqCst) + 1;
                if call == block_call { waiting.acquire().await.unwrap().forget(); }
                let answer = format!("A durable answer from model step {call}.");
                let delta = json!({"id":format!("knowledge-main-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":{"role":"assistant","content":answer},"finish_reason":null}]});
                let finish = json!({"id":format!("knowledge-main-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                let body = format!("data: {delta}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self { url, calls, gate, task }
    }

    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

struct ControlledCompleter {
    base: nomifun_common::KnowledgeBaseId,
    block_first: bool,
    fail_first: bool,
    entered: Semaphore,
    release: Semaphore,
    calls: AtomicUsize,
    inputs: Mutex<Vec<(String, String, String)>>,
}

impl ControlledCompleter {
    fn install(fixture: &Fixture, block_first: bool, fail_first: bool) -> Arc<Self> {
        let owner = Arc::new(Self {
            base:fixture.knowledge_base.as_ref().unwrap().0.clone(), block_first, fail_first,
            entered:Semaphore::new(0), release:Semaphore::new(0), calls:AtomicUsize::new(0), inputs:Default::default(),
        });
        fixture.services.knowledge_service.set_completer(owner.clone());
        owner
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(8), self.entered.acquire()).await.unwrap().unwrap().forget();
    }
}

#[async_trait]
impl KnowledgeCompleter for ControlledCompleter {
    async fn complete(&self, _: &str, _: &str) -> Result<String, AppError> {
        panic!("the original admitted model must reach complete_with");
    }

    async fn complete_with(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, AppError> {
        panic!("write-back must use its bounded completion contract");
    }

    async fn complete_writeback(
        &self, request: nomifun_knowledge::KnowledgeWritebackCompletionRequest<'_>,
    ) -> Result<nomifun_knowledge::KnowledgeWritebackCompletion, nomifun_knowledge::KnowledgeWritebackCompletionError> {
        let started = std::time::Instant::now();
        let model = request.model.expect("the original admitted model must reach extraction");
        let provider = &model.provider_id;
        let selected = model.use_model.as_deref().unwrap_or(&model.model);
        assert_eq!(request.system, nomifun_knowledge::turn_writeback::TURN_WRITEBACK_SYSTEM);
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.inputs.lock().unwrap().push((provider.into(), selected.into(), request.user.into()));
        self.entered.add_permits(1);
        let mut diagnostic = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Stream);
        diagnostic.output_token_limit = Some(request.limits.output_max_tokens);
        diagnostic.received_bytes = Some(0);
        diagnostic.received_frames = Some(0);
        diagnostic.output_bytes = Some(0);
        if call == 1 && self.block_first {
            let token = request.cancellation.expect("owned extraction must be cancellable");
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    diagnostic.stage = KnowledgeWritebackStage::Cancelled;
                    diagnostic.code = Some("NOMIFUN_TASK_INCOMPLETE".into());
                    diagnostic.elapsed_ms = Some(started.elapsed().as_millis() as u64);
                    return Err(nomifun_knowledge::KnowledgeWritebackCompletionError { diagnostic });
                }
                ready = self.release.acquire() => ready.unwrap().forget(),
            }
        }
        diagnostic.elapsed_ms = Some(started.elapsed().as_millis() as u64);
        if call == 1 && self.fail_first {
            diagnostic.code = Some("USER_LLM_PROVIDER_NETWORK_ERROR".into());
            diagnostic.detail = Some("fixture extraction unavailable: 知识提炼无法取得响应".into());
            return Err(nomifun_knowledge::KnowledgeWritebackCompletionError { diagnostic });
        }
        let text = if call == 1 {
            json!({"candidates":[{"kb_id":self.base,"rel_path":"notes/sdk-writeback.md","content":"# Durable knowledge\n\nThe original completed Turn owns this note."}]}).to_string()
        } else { json!({"candidates":[]}).to_string() };
        diagnostic.output_bytes = Some(text.len() as u64);
        Ok(nomifun_knowledge::KnowledgeWritebackCompletion { text, diagnostic })
    }

}

async fn wait_finish(mut events: tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent>) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match events.recv().await.unwrap() {
                nomifun_ai_agent::AgentStreamEvent::Finish(_) => break,
                nomifun_ai_agent::AgentStreamEvent::Error(error) => panic!("main SDK Turn failed: {error:?}"),
                _ => {},
            }
        }
    }).await.expect("the main SDK Turn must finish independently of extraction");
}

async fn send_original(fixture: &Fixture) {
    let events = fixture.runtime.subscribe();
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    wait_finish(events).await;
    assert_eq!(fixture.runtime.status(), Some(nomifun_common::ConversationStatus::Finished));
    assert!(fixture.host.active.lock().await.is_none());
}

async fn successor(fixture: &Fixture, key: &str) -> (Value, tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent>) {
    let events = fixture.runtime.subscribe();
    let response = api(&fixture._router, &format!("/api/agent-sessions/{}/turns", fixture.host.options.conversation_id),
        json!({"idempotency_key":key,"input":{"content":"Complete the successor without borrowing the original write-back."}})).await;
    (response, events)
}

async fn summary(fixture: &Fixture, operation: &str) -> Value {
    fixture.owner.canonical().store().turn_history_for_operation(
        &PrincipalRef {principal_kind:"user".into(),principal_id:fixture.host.options.user_id.clone()},
        &fixture.host.options.conversation_id.clone().into(), &operation.into(),
    ).await.unwrap().unwrap().projection
}

async fn wait_writeback(fixture: &Fixture, operation: &str, expected: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let view = summary(fixture, operation).await;
            if view.pointer("/knowledge_writeback/status").and_then(Value::as_str) == Some(expected) { return view; }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.unwrap_or_else(|_| panic!("write-back did not reach {expected} for {operation}"))
}

fn note_path(fixture: &Fixture) -> PathBuf {
    fixture.knowledge_base.as_ref().unwrap().1.join("notes/sdk-writeback.md")
}

#[tokio::test]
async fn sdk_main_completes_and_a_successor_runs_while_the_original_extractor_is_blocked() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-sdk-successor", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    let before = wait_writeback(&fixture, ORIGINAL, "running").await;
    assert_eq!(before["state"], "completed");
    assert!(!note_path(&fixture).exists());
    let (next, events) = successor(&fixture, "knowledge-successor").await;
    wait_finish(events).await;
    let next_operation = next["operation_id"].as_str().unwrap();
    wait_writeback(&fixture, next_operation, "no_candidate").await;
    assert_eq!(summary(&fixture, next_operation).await["state"], "completed");
    assert_eq!(wait_writeback(&fixture, ORIGINAL, "running").await["finished_seq"], before["finished_seq"]);
    completer.release.add_permits(1);
    let after = wait_writeback(&fixture, ORIGINAL, "written").await;
    assert_eq!(after["source_message_id"], fixture.message.source_message_id.as_ref().unwrap().as_str());
    assert_eq!(after["finished_at_ms"], before["finished_at_ms"]);
    assert_eq!(after["finished_seq"], before["finished_seq"]);
    assert_eq!(after["knowledge_writeback"]["written"][0]["kb_id"], completer.base.as_str());
    assert_eq!(after["knowledge_writeback"]["written"][0]["rel_path"], "notes/sdk-writeback.md");
    assert!(std::fs::read_to_string(note_path(&fixture)).unwrap().contains("original completed Turn"));
    let model = fixture.host.options.model.as_ref().unwrap();
    let inputs = completer.inputs.lock().unwrap();
    assert_eq!(inputs[0].0, model.provider_id);
    assert_eq!(inputs[0].1, model.model);
    assert!(inputs[0].2.contains("A durable answer from model step 1."));
    assert!(!inputs[0].2.contains("A durable answer from model step 2."));
    drop(inputs);
    let started: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='turn/knowledge-writeback-started' AND correlation_id=?")
        .bind(&fixture.host.options.conversation_id).bind(ORIGINAL).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&started).unwrap()["resolved_snapshot_ref"], json!(fixture.host.snapshot_ref));
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn delayed_exact_cancel_stops_only_the_original_writeback_while_the_successor_model_runs() {
    let server = ModelServer::new(2).await;
    let fixture = Fixture::with_knowledge("knowledge-exact-cancel-successor", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    wait_writeback(&fixture, ORIGINAL, "running").await;
    let (next, events) = successor(&fixture, "knowledge-cancel-successor").await;
    tokio::time::timeout(Duration::from_secs(8), async {
        while server.calls.load(Ordering::SeqCst) < 2 { tokio::time::sleep(Duration::from_millis(2)).await; }
    }).await.unwrap();
    let cancelled = api(&fixture._router, &format!("/api/agent-sessions/{}/turns/cancel", fixture.host.options.conversation_id),
        json!({"idempotency_key":"late-original-writeback-stop","expected_turn_id":fixture.message.source_message_id})).await;
    assert_eq!(cancelled["target_operation_id"], ORIGINAL);
    wait_writeback(&fixture, ORIGINAL, "cancelled").await;
    let next_operation = next["operation_id"].as_str().unwrap();
    assert_eq!(summary(&fixture, next_operation).await["state"], "running");
    assert_eq!(fixture.runtime.status(), Some(nomifun_common::ConversationStatus::Running));
    server.gate.add_permits(1);
    wait_finish(events).await;
    wait_writeback(&fixture, next_operation, "no_candidate").await;
    assert!(!note_path(&fixture).exists());
    assert_eq!(summary(&fixture, ORIGINAL).await["state"], "completed");
    completer.release.add_permits(1);
    fixture.finish().await;
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    server.close().await;
}

#[tokio::test]
async fn soft_runtime_retirement_waits_for_the_original_writeback_without_cancelling_success() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-soft-runtime-retirement", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    let before = wait_writeback(&fixture, ORIGINAL, "running").await;
    let mut retiring = fixture.runtime.kill_and_wait(Some(nomifun_common::AgentKillReason::TurnBoundaryRecycle));
    assert!(futures_util::poll!(retiring.as_mut()).is_pending());
    assert_eq!(wait_writeback(&fixture, ORIGINAL, "running").await["finished_seq"], before["finished_seq"]);
    completer.release.add_permits(1);
    retiring.await.unwrap();
    let after = wait_writeback(&fixture, ORIGINAL, "written").await;
    assert_eq!(after["state"], "completed");
    assert_eq!(after["finished_at_ms"], before["finished_at_ms"]);
    assert!(fixture.host.resources.teardown_proven().unwrap());
    assert!(std::fs::read_to_string(note_path(&fixture)).unwrap().contains("original completed Turn"));
    assert_eq!(completer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn completed_attempt_cleanup_retires_the_actual_runtime_after_writeback_without_cancelling_it() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-completed-attempt-cleanup", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    let before = wait_writeback(&fixture, ORIGINAL, "running").await;
    let session = fixture.host.options.conversation_id.clone();
    let owner = fixture.host.options.user_id.clone();
    let attempt = nomifun_common::generate_id();
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_attempt_for_execution(
        fixture.owner.as_ref(), &owner, &session, &attempt, ORIGINAL,
    ).await.unwrap();
    let session_id = session.clone().into();
    let operation = ORIGINAL.into();
    let mut retirement = Box::pin(fixture.owner.retire_completed_runtime(&owner, &session_id,
        &operation, &fixture.runtime));
    assert!(futures_util::poll!(&mut retirement).is_pending());
    let still_running = wait_writeback(&fixture, ORIGINAL, "running").await;
    assert_eq!(still_running["finished_at_ms"], before["finished_at_ms"]);
    assert!(fixture.services.agent_runtime_sessions.get_runtime(&session).is_some(),
        "a pending native cleanup must retain its original registry witness");
    completer.release.add_permits(1);
    retirement.await.unwrap();
    let settled = wait_writeback(&fixture, ORIGINAL, "written").await;
    assert_eq!(settled["state"], "completed");
    assert_eq!(settled["finished_at_ms"], before["finished_at_ms"]);
    assert!(fixture.services.agent_runtime_sessions.get_runtime(&session).is_none(),
        "cleanup must retire the actual Runtime, not just acknowledge an inactive Turn");
    assert!(fixture.host.resources.teardown_proven().unwrap());
    assert!(std::fs::read_to_string(note_path(&fixture)).unwrap().contains("original completed Turn"));
    assert_eq!(completer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn completed_attempt_cleanup_preserves_a_successor_and_the_original_pending_writeback() {
    let server = ModelServer::new(2).await;
    let fixture = Fixture::with_knowledge("knowledge-completed-cleanup-successor", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    let (next, events) = successor(&fixture, "completed-cleanup-successor").await;
    let next_operation = next["operation_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while server.calls.load(Ordering::SeqCst) < 2 { tokio::task::yield_now().await; }
    }).await.unwrap();
    let session = fixture.host.options.conversation_id.clone();
    let generation = fixture.services.agent_runtime_sessions.active_turn_generation(&session);
    assert!(generation.is_some());
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_attempt_for_execution(
        fixture.owner.as_ref(), &fixture.host.options.user_id, &session,
        &nomifun_common::generate_id(), ORIGINAL,
    ).await.unwrap();
    assert_eq!(fixture.services.agent_runtime_sessions.active_turn_generation(&session), generation);
    assert_eq!(fixture.owner.canonical().store().head(&session.clone().into()).await.unwrap().active_turn_id.as_deref(), Some(next_operation));
    assert_eq!(wait_writeback(&fixture, ORIGINAL, "running").await["state"], "completed");
    let session_id = session.clone().into();
    let operation = ORIGINAL.into();
    let mut retirement = Box::pin(fixture.owner.retire_completed_runtime(&fixture.host.options.user_id,
        &session_id, &operation, &fixture.runtime));
    assert!(futures_util::poll!(&mut retirement).is_pending());
    completer.release.add_permits(1);
    assert_eq!(wait_writeback(&fixture, ORIGINAL, "written").await["state"], "completed");
    retirement.await.unwrap();
    assert_eq!(fixture.services.agent_runtime_sessions.active_turn_generation(&session), generation);
    server.gate.add_permits(1);
    wait_finish(events).await;
    wait_writeback(&fixture, next_operation, "no_candidate").await;
    assert_eq!(completer.calls.load(Ordering::SeqCst), 2);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn session_teardown_joins_the_cancelled_extractor_and_prevents_a_late_write() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-session-teardown", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    wait_writeback(&fixture, ORIGINAL, "running").await;
    tokio::time::timeout(Duration::from_secs(8), fixture.runtime.kill_and_wait(None)).await.unwrap().unwrap();
    let result = wait_writeback(&fixture, ORIGINAL, "cancelled").await;
    assert!(fixture.host.resources.teardown_proven().unwrap());
    assert_eq!(result["state"], "completed");
    assert!(!note_path(&fixture).exists());
    completer.release.add_permits(1);
    tokio::task::yield_now().await;
    assert!(!note_path(&fixture).exists());
    assert_eq!(summary(&fixture, ORIGINAL).await, result);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn context_cleanup_joins_writeback_and_keeps_the_same_resource_owner_usable() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-context-cleanup", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    wait_writeback(&fixture, ORIGINAL, "running").await;
    fixture.host.session_host.cancel_and_join_knowledge_writebacks(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    wait_writeback(&fixture, ORIGINAL, "cancelled").await;
    completer.release.add_permits(1);
    let (next, events) = successor(&fixture, "knowledge-after-context-cleanup").await;
    wait_finish(events).await;
    wait_writeback(&fixture, next["operation_id"].as_str().unwrap(), "no_candidate").await;
    assert!(!note_path(&fixture).exists());
    assert!(!fixture.host.resources.teardown_proven().unwrap());
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn a_failed_result_commit_retries_the_original_outcome_without_repeating_extraction_or_writes() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-settlement-retry", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, true, false);
    send_original(&fixture).await;
    completer.wait_entered().await;
    let before = wait_writeback(&fixture, ORIGINAL, "running").await;
    sqlx::query("CREATE TRIGGER reject_knowledge_settlement BEFORE INSERT ON agent_events WHEN NEW.kind='turn/knowledge-writeback-settled' BEGIN SELECT RAISE(FAIL,'fixture Knowledge settlement failure'); END")
        .execute(fixture.pool()).await.unwrap();
    completer.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(8), async {
        while !note_path(&fixture).exists() { tokio::time::sleep(Duration::from_millis(2)).await; }
    }).await.unwrap();
    let body = std::fs::read_to_string(note_path(&fixture)).unwrap();
    let error = fixture.host.resources.cancel_and_join_knowledge_writebacks().await.unwrap_err();
    assert!(error.to_string().contains("fixture Knowledge settlement failure"));
    assert_eq!(summary(&fixture, ORIGINAL).await["state"], "completed");
    assert_eq!(completer.calls.load(Ordering::SeqCst), 1);
    sqlx::query("DROP TRIGGER reject_knowledge_settlement").execute(fixture.pool()).await.unwrap();
    fixture.host.resources.cancel_and_join_knowledge_writebacks().await.unwrap();
    let after = summary(&fixture, ORIGINAL).await;
    assert!(matches!(after["knowledge_writeback"]["status"].as_str(), Some("written" | "cancelled")),
        "a cancellation after publication must retain the actual file outcome: {after}");
    assert_eq!(after["knowledge_writeback"]["written"][0]["rel_path"], "notes/sdk-writeback.md");
    assert_eq!(before["finished_at_ms"], after["finished_at_ms"]);
    assert_eq!(std::fs::read_to_string(note_path(&fixture)).unwrap(), body);
    assert_eq!(completer.calls.load(Ordering::SeqCst), 1);
    let settlements: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/knowledge-writeback-settled'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(settlements, 1);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    fixture.finish().await;
    server.close().await;
}

#[tokio::test]
async fn extraction_failure_is_a_writeback_result_and_does_not_fail_the_completed_answer() {
    let server = ModelServer::new(0).await;
    let fixture = Fixture::with_knowledge("knowledge-extractor-failure", &server.url).await;
    let completer = ControlledCompleter::install(&fixture, false, true);
    send_original(&fixture).await;
    let result = wait_writeback(&fixture, ORIGINAL, "failed").await;
    assert_eq!(result["state"], "completed");
    assert!(result["knowledge_writeback"]["diagnostic"]["detail"].as_str().unwrap().contains("fixture extraction unavailable"));
    assert!(result["knowledge_writeback"]["diagnostic"]["detail"].as_str().unwrap().len() <= 2048);
    assert!(!note_path(&fixture).exists());
    let (next, events) = successor(&fixture, "knowledge-after-extraction-failure").await;
    wait_finish(events).await;
    wait_writeback(&fixture, next["operation_id"].as_str().unwrap(), "no_candidate").await;
    assert_eq!(completer.calls.load(Ordering::SeqCst), 2);
    fixture.finish().await;
    server.close().await;
}
