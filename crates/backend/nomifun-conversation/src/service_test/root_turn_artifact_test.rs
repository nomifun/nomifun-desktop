use super::*;

use std::path::Path;

use nomifun_ai_agent::artifact_store::{
    ArtifactKind, ArtifactRecoveryEnvelope, ArtifactRecoverySource, ArtifactStore,
    PersistedArtifact,
};
use nomifun_ai_agent::protocol::events::{
    AgentStatusEventData, StartEventData, ToolCallEventData, ToolCallStatus, TurnStopReason,
};
use serde_json::Value;

const ONE_PIXEL_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
const IMAGE_CALL_ID: &str = "native-image-call";

#[derive(Clone, Copy)]
enum RootTurnScript {
    ArtifactOnly,
    TextOnly,
    ProcessOnly,
    FinishOnly,
}

struct RootTurnAgent {
    conversation_id: String,
    workspace: String,
    repository: Arc<SqliteConversationRepository>,
    event_tx: broadcast::Sender<AgentStreamEvent>,
    script: RootTurnScript,
    send_count: AtomicUsize,
    root_was_durable_on_send: AtomicBool,
    wire_msg_id: Mutex<Option<String>>,
    artifact: Mutex<Option<PersistedArtifact>>,
}

impl RootTurnAgent {
    fn new(
        conversation_id: &str,
        workspace: &Path,
        repository: Arc<SqliteConversationRepository>,
        script: RootTurnScript,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(32);
        Self {
            conversation_id: conversation_id.to_owned(),
            workspace: workspace.to_string_lossy().into_owned(),
            repository,
            event_tx,
            script,
            send_count: AtomicUsize::new(0),
            root_was_durable_on_send: AtomicBool::new(false),
            wire_msg_id: Mutex::new(None),
            artifact: Mutex::new(None),
        }
    }

    fn send_count(&self) -> usize {
        self.send_count.load(Ordering::SeqCst)
    }

    fn root_was_durable_on_send(&self) -> bool {
        self.root_was_durable_on_send.load(Ordering::SeqCst)
    }

    fn wire_msg_id(&self) -> String {
        self.wire_msg_id
            .lock()
            .unwrap()
            .clone()
            .expect("agent must record the runtime-minted wire message id")
    }

    fn artifact(&self) -> PersistedArtifact {
        self.artifact
            .lock()
            .unwrap()
            .clone()
            .expect("artifact script must record its verified receipt")
    }

    fn emit(&self, event: AgentStreamEvent) {
        self.event_tx
            .send(event)
            .expect("the production relay must subscribe before Agent send_message");
    }

    async fn record_root_precondition(&self, wire_msg_id: &str) {
        let durable = self
            .repository
            .get_message(&self.conversation_id, wire_msg_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|row| {
                row.message_id == wire_msg_id
                    && row.msg_id.as_deref() == Some(wire_msg_id)
                    && row.r#type == "turn_root"
                    && row.position.as_deref() == Some("center")
                    && row.status.as_deref() == Some("finish")
                    && row.hidden
                    && serde_json::from_str::<Value>(&row.content)
                        .ok()
                        .and_then(|content| content.get("kind").cloned())
                        == Some(json!("turn_root"))
            });
        self.root_was_durable_on_send
            .store(durable, Ordering::SeqCst);
    }

    fn emit_start(&self) {
        self.emit(AgentStreamEvent::Start(StartEventData {
            session_id: Some("native-image-session".to_owned()),
        }));
    }

    fn emit_finish(&self) {
        self.emit(AgentStreamEvent::Finish(FinishEventData {
            session_id: Some("native-image-session".to_owned()),
            stop_reason: Some(TurnStopReason::EndTurn),
        }));
    }

    fn emit_artifact_turn(&self, wire_msg_id: &str) {
        let store = ArtifactStore::new(&self.workspace);
        let artifact = store
            .persist_inline_and_existing_batch_recoverable(
                [(ArtifactKind::Image, "image/png", ONE_PIXEL_PNG)],
                std::iter::empty::<&Path>(),
                &ArtifactRecoverySource {
                    conversation_id: self.conversation_id.clone(),
                    wire_msg_id: wire_msg_id.to_owned(),
                },
            )
            .expect("persist a verified image receipt")
            .pop()
            .expect("one image receipt");

        let args = json!({
            "prompt": "a friendly illustrated fox reading beneath a lantern",
            "size": "1024x1024",
            "aspect_ratio": "1:1",
            "style": "storybook"
        });
        let running = ToolCallEventData {
            call_id: IMAGE_CALL_ID.to_owned(),
            name: "image_gen".to_owned(),
            args: args.clone(),
            status: ToolCallStatus::Running,
            input: Some(args.clone()),
            output: None,
            description: Some("Generating image".to_owned()),
            retry: None,
            artifacts: Vec::new(),
        };
        let completed = ToolCallEventData {
            status: ToolCallStatus::Completed,
            output: Some("Image generated".to_owned()),
            artifacts: vec![artifact.clone()],
            ..running.clone()
        };
        store
            .prepare_recovery_receipts(
                std::slice::from_ref(&artifact),
                &ArtifactRecoveryEnvelope {
                    conversation_id: self.conversation_id.clone(),
                    wire_msg_id: wire_msg_id.to_owned(),
                    event_kind: "tool_call".to_owned(),
                    event_json: serde_json::to_string(&completed)
                        .expect("serialize the recovery terminal event"),
                },
            )
            .expect("prepare the exact recovery envelope before broadcast");
        *self.artifact.lock().unwrap() = Some(artifact);

        self.emit_start();
        self.emit(AgentStreamEvent::AgentStatus(AgentStatusEventData {
            backend: "nomi".to_owned(),
            status: "preparing".to_owned(),
            agent_name: Some("Nomi".to_owned()),
            session_id: Some("native-image-session".to_owned()),
        }));
        self.emit(AgentStreamEvent::ToolCall(running));
        self.emit(AgentStreamEvent::ToolCall(completed));
        self.emit_finish();
    }
}

#[async_trait::async_trait]
impl AgentRuntimeControl for RootTurnAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }

    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    fn workspace(&self) -> &str {
        &self.workspace
    }

    fn status(&self) -> Option<ConversationStatus> {
        Some(ConversationStatus::Finished)
    }

    fn is_transport_healthy(&self) -> bool {
        true
    }

    fn last_activity_at(&self) -> TimestampMs {
        0
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.event_tx.subscribe()
    }

    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        self.send_count.fetch_add(1, Ordering::SeqCst);
        *self.wire_msg_id.lock().unwrap() = Some(data.msg_id.clone());
        self.record_root_precondition(&data.msg_id).await;

        match self.script {
            RootTurnScript::ArtifactOnly => self.emit_artifact_turn(&data.msg_id),
            RootTurnScript::TextOnly => {
                self.emit_start();
                self.emit(AgentStreamEvent::Text(TextEventData {
                    content: "The lantern is ready.".to_owned(),
                }));
                self.emit_finish();
            }
            RootTurnScript::ProcessOnly => {
                self.emit_start();
                self.emit(AgentStreamEvent::AgentStatus(AgentStatusEventData {
                    backend: "nomi".to_owned(),
                    status: "preparing".to_owned(),
                    agent_name: Some("Nomi".to_owned()),
                    session_id: Some("process-only-session".to_owned()),
                }));
                self.emit(AgentStreamEvent::Thinking(ThinkingEventData {
                    content: "Inspecting the requested composition".to_owned(),
                    subject: None,
                    duration: None,
                    status: None,
                }));
                self.emit_finish();
            }
            RootTurnScript::FinishOnly => {
                self.emit_start();
                self.emit_finish();
            }
        }
        Ok(())
    }

    async fn cancel(&self) -> Result<(), AppError> {
        Ok(())
    }

    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        Ok(())
    }
}

impl MockAgentRuntime for RootTurnAgent {}

struct RootTurnFixture {
    service: ConversationService,
    broadcaster: Arc<MockBroadcaster>,
    repository: Arc<SqliteConversationRepository>,
    registry: Arc<MockAgentRuntimeRegistry>,
    database: nomifun_db::Database,
    conversation_id: String,
    workspace: PathBuf,
    agent: Arc<RootTurnAgent>,
}

impl RootTurnFixture {
    async fn new(label: &str, script: RootTurnScript) -> Self {
        let database = init_database_memory().await.expect("initialize real SQLite fixture");
        seed_openai_chat_model(
            database.pool(),
            PROVIDER_ID_1,
            "root-turn-test",
            "m1",
            0,
        )
        .await;
        let repository = Arc::new(SqliteConversationRepository::new(database.pool().clone()));
        let broadcaster = Arc::new(MockBroadcaster::new());
        let registry = Arc::new(MockAgentRuntimeRegistry::new());
        let runtime_registry: Arc<dyn AgentRuntimeRegistry> = registry.clone();
        let workspace = isolated_test_workspace(label);
        let service = ConversationService::new(
            Arc::<str>::from(SQLITE_TEST_OWNER),
            std::env::temp_dir(),
            broadcaster.clone(),
            Arc::new(FixedSkillResolver { names: vec![] }),
            runtime_registry,
            repository.clone(),
            Arc::new(StubAgentMetadataRepo),
            Arc::new(crate::NoExecutionConversationBoundary),
        );
        let conversation = service
            .create(
                SQLITE_TEST_OWNER,
                serde_json::from_value(json!({
                    "type": "nomi",
                    "model": { "provider_id": PROVIDER_ID_1, "model": "m1" },
                    "extra": { "workspace": workspace }
                }))
                .expect("valid Nomi conversation request"),
            )
            .await
            .expect("create SQLite-backed Nomi conversation");
        let agent = Arc::new(RootTurnAgent::new(
            &conversation.conversation_id,
            &workspace,
            repository.clone(),
            script,
        ));
        registry.insert_agent(
            &conversation.conversation_id,
            AgentRuntimeHandle::Mock(agent.clone()),
        );
        broadcaster.take_events();

        Self {
            service,
            broadcaster,
            repository,
            registry,
            database,
            conversation_id: conversation.conversation_id,
            workspace,
            agent,
        }
    }

    fn runtime_registry(&self) -> Arc<dyn AgentRuntimeRegistry> {
        self.registry.clone()
    }

    async fn send_and_wait(&self, operation_id: &str) -> String {
        let runtime_registry = self.runtime_registry();
        let accepted_message_id = send_message_with_test_key(
            &self.service,
            SQLITE_TEST_OWNER,
            &self.conversation_id,
            operation_id,
            serde_json::from_value(json!({ "content": "Generate the illustration" }))
                .expect("valid send request"),
            &runtime_registry,
        )
        .await
        .expect("turn admission must succeed");
        wait_for_turn_released(&self.service, &self.conversation_id).await;
        accepted_message_id
    }

    async fn rows(&self) -> Vec<MessageRow> {
        nomifun_db::sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages WHERE conversation_id = ? \
             ORDER BY created_at ASC, message_id ASC",
        )
            .bind(&self.conversation_id)
            .fetch_all(self.database.pool())
            .await
            .expect("load raw SQLite message history, including hidden turn roots")
    }
}

fn content(row: &MessageRow) -> Value {
    serde_json::from_str(&row.content).expect("message content must be JSON")
}

fn event_index<F>(events: &[WebSocketMessage<Value>], predicate: F) -> usize
where
    F: Fn(&WebSocketMessage<Value>) -> bool,
{
    events
        .iter()
        .position(predicate)
        .expect("expected realtime event")
}

#[tokio::test]
async fn root_turn_artifact_only_success_uses_durable_hidden_parent_and_receipt_success() {
    const OPERATION_ID: &str = "root-turn-artifact-only-success";
    let fixture = RootTurnFixture::new("root-artifact-only", RootTurnScript::ArtifactOnly).await;
    let accepted_message_id = fixture.send_and_wait(OPERATION_ID).await;

    assert_eq!(fixture.agent.send_count(), 1);
    assert!(
        fixture.agent.root_was_durable_on_send(),
        "the structural root must be committed before Agent send_message starts"
    );
    let root_id = fixture.agent.wire_msg_id();
    assert_ne!(accepted_message_id, root_id);

    let rows = fixture.rows().await;
    let root = rows
        .iter()
        .find(|row| row.message_id == root_id)
        .expect("hidden structural root");
    assert_eq!(root.msg_id.as_deref(), Some(root_id.as_str()));
    assert_eq!(root.r#type, "turn_root");
    assert_eq!(root.position.as_deref(), Some("center"));
    assert_eq!(root.status.as_deref(), Some("finish"));
    assert!(root.hidden);
    assert_eq!(content(root)["kind"], "turn_root");

    let public_root = fixture
        .service
        .get_message(SQLITE_TEST_OWNER, &fixture.conversation_id, &root_id)
        .await
        .expect_err("the hidden structural root is not a public message");
    assert!(matches!(public_root, AppError::NotFound(_)));

    let status = rows
        .iter()
        .find(|row| row.r#type == "agent_status")
        .expect("durable agent status child");
    assert_ne!(status.message_id, root_id);
    assert_eq!(status.msg_id.as_deref(), Some(root_id.as_str()));
    assert_eq!(status.status.as_deref(), Some("finish"));
    assert_eq!(content(status)["status"], "prepared");

    let tool = rows
        .iter()
        .find(|row| row.r#type == "tool_call")
        .expect("durable image tool child");
    assert_ne!(tool.message_id, root_id);
    assert_ne!(tool.message_id, status.message_id);
    assert_eq!(tool.msg_id.as_deref(), Some(root_id.as_str()));
    assert_eq!(tool.status.as_deref(), Some("finish"));
    let tool_content = content(tool);
    assert_eq!(tool_content["call_id"], IMAGE_CALL_ID);
    assert_eq!(tool_content["name"], "image_gen");
    assert_eq!(tool_content["status"], "completed");
    assert_eq!(tool_content["turn_id"], root_id);
    assert_eq!(tool_content["artifact_delivery_committed"], true);
    assert_eq!(
        tool_content["artifacts"].as_array().map(Vec::len),
        Some(1)
    );
    assert!(
        rows.iter().all(|row| row.status.as_deref() != Some("work")),
        "no provisional message may survive the terminal commit"
    );
    assert!(
        rows.iter().all(|row| row.r#type != "tips"),
        "an artifact-only Finish must not be downgraded to an error tip"
    );
    assert!(
        rows.iter().all(|row| {
            row.position.as_deref() != Some("left")
                || !matches!(row.r#type.as_str(), "text" | "thinking")
        }),
        "the native image turn intentionally contains no assistant Text/Thinking"
    );

    let artifact = fixture.agent.artifact();
    ArtifactStore::new(&fixture.workspace)
        .reverify_receipt(&artifact)
        .expect("committed image bytes must still match their receipt");
    assert!(
        ArtifactStore::new(&fixture.workspace)
            .recovery_records()
            .expect("read recovery journal")
            .is_empty(),
        "the durable commit must finalize its recovery journal"
    );

    let durable_operation_id = ConversationService::public_turn_operation_id(
        SQLITE_TEST_OWNER,
        &fixture.conversation_id,
        OPERATION_ID,
    );
    let receipt = fixture
        .repository
        .get_delivery_receipt(
            SQLITE_TEST_OWNER,
            &fixture.conversation_id,
            &durable_operation_id,
        )
        .await
        .expect("load turn delivery receipt")
        .expect("turn delivery receipt");
    assert_eq!(receipt.status, "completed");
    assert_eq!(receipt.result_ok, Some(true));
    assert_eq!(receipt.result_text, None);
    assert_eq!(receipt.result_error, None);
    assert_eq!(receipt.result_error_code, None);

    let events = fixture.broadcaster.take_events();
    let started = event_index(&events, |event| event.name == "turn.started");
    assert_eq!(events[started].data["turn_id"], root_id);
    let stream_start = event_index(&events, |event| {
        event.name == "message.stream" && event.data["type"] == "start"
    });
    let preparing = event_index(&events, |event| {
        event.name == "message.stream"
            && event.data["type"] == "agent_status"
            && event.data["data"]["status"] == "preparing"
    });
    let running = event_index(&events, |event| {
        event.name == "message.stream"
            && event.data["type"] == "tool_call"
            && event.data["data"]["status"] == "running"
    });
    let completed = event_index(&events, |event| {
        event.name == "message.stream"
            && event.data["type"] == "tool_call"
            && event.data["data"]["status"] == "completed"
    });
    let finish = event_index(&events, |event| {
        event.name == "message.stream" && event.data["type"] == "finish"
    });
    let turn_completed = event_index(&events, |event| event.name == "turn.completed");
    assert!(started < stream_start);
    assert!(stream_start < preparing);
    assert!(preparing < running);
    assert!(running < completed);
    assert!(completed < finish);
    assert!(finish < turn_completed);
    assert_eq!(events[completed].data["data"]["artifacts"][0]["id"], artifact.id);
    assert!(events.iter().all(|event| {
        event.name != "message.stream"
            || !matches!(
                event.data["type"].as_str(),
                Some("content" | "thinking" | "error")
            )
    }));
}

#[tokio::test]
async fn root_turn_first_visible_text_uses_a_child_id_instead_of_hidden_root_id() {
    const OPERATION_ID: &str = "root-turn-first-visible-text";
    let fixture = RootTurnFixture::new("root-first-text", RootTurnScript::TextOnly).await;
    fixture.send_and_wait(OPERATION_ID).await;

    assert_eq!(fixture.agent.send_count(), 1);
    assert!(fixture.agent.root_was_durable_on_send());
    let root_id = fixture.agent.wire_msg_id();
    let rows = fixture.rows().await;
    let root = rows
        .iter()
        .find(|row| row.message_id == root_id)
        .expect("hidden structural root");
    assert_eq!(root.r#type, "turn_root");
    assert!(root.hidden);
    assert_eq!(content(root)["kind"], "turn_root");

    let visible_text = rows
        .iter()
        .filter(|row| {
            row.r#type == "text"
                && row.position.as_deref() == Some("left")
                && !row.hidden
        })
        .collect::<Vec<_>>();
    assert_eq!(visible_text.len(), 1);
    let visible_text = visible_text[0];
    assert_ne!(visible_text.message_id, root_id);
    assert_eq!(
        visible_text.msg_id.as_deref(),
        Some(visible_text.message_id.as_str())
    );
    assert_eq!(visible_text.status.as_deref(), Some("finish"));
    assert_eq!(content(visible_text)["turn_id"], root_id);
    assert_eq!(content(visible_text)["content"], "The lantern is ready.");

    let events = fixture.broadcaster.take_events();
    let text_event = events
        .iter()
        .find(|event| {
            event.name == "message.stream"
                && event.data["type"] == "content"
                && event.data["data"]["content"] == "The lantern is ready."
        })
        .expect("visible Text stream event");
    assert_eq!(text_event.data["msg_id"], visible_text.message_id);
    assert_ne!(text_event.data["msg_id"], root_id);
    assert_eq!(text_event.data["turn_id"], root_id);
}

#[tokio::test]
async fn root_turn_thinking_and_agent_status_persist_the_explicit_turn_id() {
    let fixture = RootTurnFixture::new("root-process-turn-id", RootTurnScript::ProcessOnly).await;
    fixture.send_and_wait("root-process-turn-id").await;
    let root_id = fixture.agent.wire_msg_id();
    let rows = fixture.rows().await;
    for row_type in ["agent_status", "thinking"] {
        let row = rows
            .iter()
            .find(|row| {
                row.r#type == row_type
                    && content(row)["turn_id"].as_str() == Some(root_id.as_str())
            })
            .unwrap_or_else(|| panic!("{row_type} must retain its explicit root turn id"));
        assert_ne!(row.message_id, root_id);
        assert_eq!(content(row)["turn_id"].as_str(), Some(root_id.as_str()));
    }
}

#[tokio::test]
async fn root_turn_preflight_failure_never_invokes_agent_send() {
    let fixture = RootTurnFixture::new("root-preflight-failure", RootTurnScript::FinishOnly).await;
    nomifun_db::sqlx::query(
        r#"CREATE TRIGGER reject_test_turn_root
           BEFORE INSERT ON messages
           WHEN NEW.type = 'turn_root'
             AND NEW.hidden = 1
             AND NEW.content = '{"kind":"turn_root"}'
           BEGIN
             SELECT RAISE(ABORT, 'injected turn root preflight failure');
           END"#,
    )
    .execute(fixture.database.pool())
    .await
    .expect("install root-only failure trigger");

    fixture
        .send_and_wait("root-turn-preflight-failure")
        .await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(
        fixture.agent.send_count(),
        0,
        "billable Agent work must not start when the structural root cannot be persisted"
    );
}
