//! Integration tests for the busy-time queue drain (spec D1, Task 3).
//!
//! A real in-memory database, a real `ConversationService`, and a real
//! `BroadcastEventBus` are wired exactly like production: the drain consumes
//! the SAME `turn.completed` envelopes the service broadcasts, so FIFO
//! progression after each delivered turn is exercised end-to-end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_ai_agent::protocol::events::{FinishEventData, TextEventData};
use nomifun_ai_agent::runtime_handle::{AgentRuntimeControl, AgentRuntimeHandle};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{AgentRuntimeRegistry, AgentSendError, AgentStreamEvent, MockAgentRuntime};
use nomifun_channel::channel_settings::ChannelSettingsService;
use nomifun_channel::error::ChannelError;
use nomifun_channel::message_service::ChannelMessageService;
use nomifun_channel::queue_drain::QueueDrain;
use nomifun_channel::session::SessionManager;
use nomifun_channel::stream_relay::ChannelSender;
use nomifun_channel::types::{OutgoingMedia, PluginType, UnifiedOutgoingMessage};
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use nomifun_conversation::ConversationService;
use nomifun_conversation::skill_resolver::{ResolvedAgentSkill, SkillResolver};
use nomifun_db::models::{NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow};
use nomifun_db::{
    CreateProviderParams, IChannelRepository, IProviderRepository, NewProviderModel,
    NewProviderModelCapability, SqliteAgentMetadataRepository,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteConversationRepository,
    SqliteProviderRepository, init_database_memory,
};
use nomifun_realtime::{BroadcastEventBus, UserEventSink};
use nomifun_db::sqlx;
use tokio::sync::broadcast;

const PROVIDER: &str = "018f1234-5678-7abc-8def-0123456789a0";

struct NoopSkillResolver;

#[async_trait]
impl SkillResolver for NoopSkillResolver {
    async fn auto_inject_names(&self) -> Vec<String> {
        Vec::new()
    }
    async fn resolve_skills(&self, _names: &[String]) -> Vec<ResolvedAgentSkill> {
        Vec::new()
    }
    async fn link_workspace_skills(
        &self,
        _workspace: &std::path::Path,
        _rel_dirs: &[&str],
        _skills: &[ResolvedAgentSkill],
    ) -> usize {
        0
    }
}

/// Agent that fails its first `fail_first` sends with a RETRYABLE error, then
/// succeeds. `fail_first = 0` always succeeds; a large value always fails.
struct FlakyAgent {
    conversation_id: String,
    event_tx: broadcast::Sender<AgentStreamEvent>,
    remaining_failures: Arc<Mutex<u32>>,
}

impl FlakyAgent {
    fn new(conversation_id: &str, remaining_failures: Arc<Mutex<u32>>) -> Self {
        let (event_tx, _) = broadcast::channel(16);
        Self {
            conversation_id: conversation_id.to_owned(),
            event_tx,
            remaining_failures,
        }
    }
}

#[async_trait]
impl AgentRuntimeControl for FlakyAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }
    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    fn workspace(&self) -> &str {
        "/tmp/nomifun-queue-drain-test"
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
    async fn send_message(&self, _data: SendMessageData) -> Result<(), AgentSendError> {
        {
            let mut remaining = self.remaining_failures.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                // Retryable classified provider failure (spec D4): rate limit
                // maps to result_error_code=user_llm_provider_rate_limited,
                // retryable=true. (Failover is unwired here, so the service
                // falls through to plain error surfacing.)
                return Err(AgentSendError::new(
                    "provider rate limited",
                    nomifun_api_types::AgentErrorCode::UserLlmProviderRateLimited,
                    nomifun_api_types::AgentErrorOwnership::UserLlmProvider,
                    Some("injected retryable fault".to_owned()),
                    true,
                    false,
                    None,
                ));
            }
        }
        // A real completed turn carries final text; an empty final is the D4
        // `empty_final_text` failure, which would (correctly) fail the queue
        // delivery instead of settling it delivered.
        let _ = self.event_tx.send(AgentStreamEvent::Text(TextEventData {
            content: "done".to_owned(),
        }));
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Finish(FinishEventData::default()));
        Ok(())
    }
    async fn cancel(&self) -> Result<(), AppError> {
        Ok(())
    }
    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        Ok(())
    }
}

impl MockAgentRuntime for FlakyAgent {}

struct FlakyRegistry {
    agents: Mutex<std::collections::HashMap<String, AgentRuntimeHandle>>,
    remaining_failures: Arc<Mutex<u32>>,
}

impl FlakyRegistry {
    fn new(fail_first: u32) -> Self {
        Self {
            agents: Mutex::new(std::collections::HashMap::new()),
            remaining_failures: Arc::new(Mutex::new(fail_first)),
        }
    }
}

#[async_trait]
impl AgentRuntimeRegistry for FlakyRegistry {
    fn get_runtime(&self, conversation_id: &str) -> Option<AgentRuntimeHandle> {
        self.agents.lock().unwrap().get(conversation_id).cloned()
    }
    async fn get_or_create_runtime(
        &self,
        conversation_id: &str,
        _options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(agent) = agents.get(conversation_id) {
            return Ok(agent.clone());
        }
        let agent = AgentRuntimeHandle::Mock(Arc::new(FlakyAgent::new(
            conversation_id,
            Arc::clone(&self.remaining_failures),
        )));
        agents.insert(conversation_id.to_owned(), agent.clone());
        Ok(agent)
    }
    fn terminate(&self, conversation_id: &str, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.agents.lock().unwrap().remove(conversation_id);
        Ok(())
    }
    fn terminate_all(&self) {
        self.agents.lock().unwrap().clear();
    }
    fn active_runtime_count(&self) -> usize {
        self.agents.lock().unwrap().len()
    }
}

/// Records every outbound chat message.
struct MessageRecorder {
    sends: Mutex<Vec<(String, String)>>,
}

impl MessageRecorder {
    fn new() -> Self {
        Self {
            sends: Mutex::new(Vec::new()),
        }
    }
    fn texts(&self) -> Vec<String> {
        self.sends.lock().unwrap().iter().map(|(_, t)| t.clone()).collect()
    }
}

#[async_trait]
impl ChannelSender for MessageRecorder {
    async fn send_message(
        &self,
        _plugin_id: &str,
        chat_id: &str,
        message: UnifiedOutgoingMessage,
    ) -> Result<String, ChannelError> {
        self.sends
            .lock()
            .unwrap()
            .push((chat_id.to_owned(), message.text.unwrap_or_default()));
        Ok("mid".to_owned())
    }
    async fn edit_message(
        &self,
        _plugin_id: &str,
        _chat_id: &str,
        _message_id: &str,
        _message: UnifiedOutgoingMessage,
    ) -> Result<(), ChannelError> {
        Ok(())
    }
    async fn send_media(
        &self,
        _plugin_id: &str,
        _chat_id: &str,
        _media: OutgoingMedia,
        _caption: Option<&str>,
    ) -> Result<String, ChannelError> {
        Ok("mid".to_owned())
    }
}

struct Stack {
    conversation_svc: Arc<ConversationService>,
    message_svc: Arc<ChannelMessageService>,
    session_manager: Arc<SessionManager>,
    channel_repo: Arc<SqliteChannelRepository>,
    event_bus: Arc<BroadcastEventBus>,
    runtime: Arc<nomifun_conversation::runtime_state::ConversationRuntimeStateService>,
    recorder: Arc<MessageRecorder>,
    installation_owner: String,
    plugin_id: String,
    session_id: String,
    pool: nomifun_db::SqlitePool,
}

async fn build_stack(pool: nomifun_db::SqlitePool, fail_first: u32) -> Stack {
    let stack_pool = pool.clone();
    let installation_owner = nomifun_db::installation_owner_id(&pool).await.unwrap();

    // Provider + default telegram model so channel conversations can be created.
    let providers = SqliteProviderRepository::new(pool.clone());
    let chat = [NewProviderModelCapability {
        task: "chat",
        traits: "[]",
        protocol: "openai.chat_text",
        connection_role: "default",
        provider_params: "{}",
        ..Default::default()
    }];
    let initial_model = NewProviderModel {
        model: "drain-model",
        enabled: true,
        sort_order: 0,
        description: None,
        capabilities: &chat,
    };
    let credentials_encrypted = nomifun_common::encrypt_string(
        r#"{"api_keys":["test-only"]}"#,
        &[0x42; 32],
    )
    .unwrap();
    providers
        .create(CreateProviderParams {
            provider_id: Some(PROVIDER),
            platform: "openai",
            name: "Queue drain provider",
            base_url: "https://example.invalid",
            auth_scheme: "bearer",
            credentials_encrypted: &credentials_encrypted,
            enabled: true,
            bedrock_config: None,
            sort_order: None,
        }, &initial_model, &[])
        .await
        .unwrap();
    let prefs = SqliteClientPreferenceRepository::new(pool.clone());
    let model = format!(r#"{{"provider_id":"{PROVIDER}","model":"drain-model"}}"#);
    nomifun_db::IClientPreferenceRepository::upsert_batch(
        &prefs,
        &[("channels.telegram.defaultModel", model.as_str())],
    )
    .await
    .unwrap();

    let runtime_registry: Arc<dyn AgentRuntimeRegistry> = Arc::new(FlakyRegistry::new(fail_first));
    let runtime =
        Arc::new(nomifun_conversation::runtime_state::ConversationRuntimeStateService::default());
    let event_bus = Arc::new(BroadcastEventBus::new(64));
    let conversation_svc = Arc::new(
        ConversationService::new(
            Arc::<str>::from(installation_owner.as_str()),
            std::env::temp_dir(),
            Arc::clone(&event_bus) as Arc<dyn nomifun_realtime::UserEventSink>,
            Arc::new(NoopSkillResolver),
            Arc::clone(&runtime_registry),
            Arc::new(SqliteConversationRepository::new(pool.clone())),
            Arc::new(SqliteAgentMetadataRepository::new(pool.clone())),
            Arc::new(nomifun_conversation::NoExecutionConversationBoundary),
        )
        .with_runtime_state(Arc::clone(&runtime)),
    );

    let settings = Arc::new(ChannelSettingsService::new(Arc::new(
        SqliteClientPreferenceRepository::new(pool.clone()),
    )));
    let channel_repo = Arc::new(SqliteChannelRepository::new(pool));
    let message_svc = Arc::new(ChannelMessageService::new(
        nomifun_channel::conversation_channel_session_port(
            Arc::clone(&conversation_svc),
            Arc::clone(&runtime_registry),
        ),
        settings,
        channel_repo.clone(),
        installation_owner.clone(),
    ));
    let session_manager = Arc::new(SessionManager::new(
        channel_repo.clone() as Arc<dyn IChannelRepository>,
    ));

    // One telegram bot + authorized user + per-chat session.
    let now = nomifun_common::now_ms();
    let plugin = channel_repo
        .create_plugin(&NewChannelPluginRow {
            r#type: "telegram".to_owned(),
            name: "Drain bot".to_owned(),
            enabled: true,
            config: "enc".to_owned(),
            status: None,
            last_connected: None,
            companion_id: None,
            bot_key: Some("drain".to_owned()),
            owner_domain: "companion".into(),
            group_access_mode: nomifun_db::models::CHANNEL_GROUP_ACCESS_MODE_ALLOWLIST.to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let user = channel_repo
        .create_user(&NewChannelUserRow {
            platform_user_id: "tg_drain".to_owned(),
            platform_type: "telegram".to_owned(),
            channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
            display_name: Some("Drain".to_owned()),
            authorization_kind: nomifun_db::models::CHANNEL_USER_AUTHORIZATION_APPROVED.to_owned(),
            authorized_at: now,
            last_active: None,
        })
        .await
        .unwrap();
    let session = channel_repo
        .get_or_create_session(
            &user.channel_user_id,
            "chat-drain",
            &plugin.channel_plugin_id,
            &NewChannelSessionRow {
                channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
                channel_user_id: user.channel_user_id.clone(),
                agent_type: "nomi".to_owned(),
                conversation_id: None,
                workspace: None,
                chat_id: Some("chat-drain".to_owned()),
                channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
                chat_kind: nomifun_db::models::CHANNEL_CHAT_KIND_DIRECT.to_owned(),
                created_at: now,
                last_activity: now,
            },
        )
        .await
        .unwrap();

    Stack {
        conversation_svc,
        message_svc,
        session_manager,
        channel_repo,
        event_bus,
        runtime,
        recorder: Arc::new(MessageRecorder::new()),
        installation_owner,
        plugin_id: plugin.channel_plugin_id,
        session_id: session.channel_session_id,
        pool: stack_pool,
    }
}

fn spawn_drain(stack: &Stack) {
    let drain = QueueDrain::new(
        stack.channel_repo.clone() as Arc<dyn IChannelRepository>,
        Arc::clone(&stack.message_svc),
        Arc::clone(&stack.session_manager),
        Arc::clone(&stack.recorder) as Arc<dyn ChannelSender>,
    )
    .with_timing(
        [Duration::from_millis(50), Duration::from_millis(50)],
        Duration::from_millis(100),
    );
    tokio::spawn(drain.run(stack.event_bus.subscribe_user()));
}

async fn wait_until_idle(svc: &Arc<ConversationService>, conversation_id: &str) {
    use nomifun_api_types::ConversationRuntimeStateKind;
    let mut last = None;
    for _ in 0..3000 {
        let summary = svc.runtime_summary_for(conversation_id).await;
        if summary.state == ConversationRuntimeStateKind::Idle {
            return;
        }
        last = Some(summary.state);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("conversation {conversation_id} never became idle; last state: {last:?}");
}

/// Creates the session's conversation with a live first turn and binds it.
async fn seed_conversation(stack: &Stack) -> String {
    let session = stack
        .session_manager
        .get_session_by_id(&stack.session_id)
        .await
        .unwrap()
        .unwrap();
    let sent = stack
        .message_svc
        .send_to_agent(&session, "first", PluginType::Telegram, "drain:first")
        .await
        .unwrap();
    stack
        .session_manager
        .bind_conversation(&stack.session_id, &sent.conversation_id)
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;
    sent.conversation_id
}

async fn enqueue(stack: &Stack, conversation_id: &str, text: &str, key: &str) {
    let outcome = stack
        .message_svc
        .enqueue_busy_prompt(
            &stack.plugin_id,
            "chat-drain",
            &stack.session_id,
            conversation_id,
            text,
            key,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, nomifun_db::PendingPromptEnqueue::Queued { .. }));
}

async fn wait_for_queue_state(
    stack: &Stack,
    conversation_id: &str,
    predicate: impl Fn(&[nomifun_db::models::ChannelPendingPromptRow]) -> bool,
    what: &str,
) -> Vec<nomifun_db::models::ChannelPendingPromptRow> {
    let mut last: Vec<nomifun_db::models::ChannelPendingPromptRow> = Vec::new();
    for _ in 0..600 {
        let rows = sqlx::query_as::<_, nomifun_db::models::ChannelPendingPromptRow>(
            "SELECT prompt_id, channel_plugin_id, chat_id, channel_session_id, \
                    conversation_id, text, idempotency_key, state, attempts, \
                    queued_at, settled_at \
             FROM channel_pending_prompts WHERE conversation_id = ? ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&stack.pool)
        .await
        .unwrap();
        if predicate(&rows) {
            return rows;
        }
        last = rows;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "queue never reached expected state: {what}; last rows: {:?}; chat sends: {:?}",
        last.iter().map(|r| (r.text.clone(), r.state.clone(), r.attempts)).collect::<Vec<_>>(),
        stack.recorder.texts(),
    );
}

#[tokio::test]
async fn turn_completion_drains_queue_fifo() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone(), 0).await;
    let conversation_id = seed_conversation(&stack).await;

    // Busy conversation: two prompts get queued while a turn is in flight.
    let turn_handle = stack.runtime.try_acquire_turn(&conversation_id).unwrap();
    enqueue(&stack, &conversation_id, "queued one", "drain:q1").await;
    enqueue(&stack, &conversation_id, "queued two", "drain:q2").await;

    spawn_drain(&stack);

    // Release the turn and announce its completion exactly like the service
    // does; from here every follow-up completion event is the REAL one the
    // drained deliveries broadcast through the shared bus.
    drop(turn_handle);
    stack.event_bus.send_to_user(
        &stack.installation_owner,
        nomifun_api_types::WebSocketMessage::new(
            "turn.completed",
            serde_json::json!({ "conversation_id": conversation_id }),
        ),
    );

    let rows = wait_for_queue_state(
        &stack,
        &conversation_id,
        |rows| rows.len() == 2 && rows.iter().all(|r| r.state == "delivered"),
        "both prompts delivered",
    )
    .await;
    assert_eq!(rows[0].text, "queued one");
    assert_eq!(rows[1].text, "queued two");

    // FIFO order is visible in the conversation transcript.
    let query = nomifun_api_types::ListMessagesQuery {
        page: Some(1),
        page_size: Some(50),
        order: Some("ASC".into()),
        content_mode: None,
        cursor: None,
        day: None,
    };
    let messages = stack
        .conversation_svc
        .list_messages(&stack.installation_owner, &conversation_id, query)
        .await
        .unwrap();
    let user_texts: Vec<String> = messages
        .items
        .iter()
        .filter(|m| m.position == Some(nomifun_common::MessagePosition::Right))
        .filter_map(|m| m.content.get("content").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    assert_eq!(user_texts, vec!["first", "queued one", "queued two"]);
}

#[tokio::test]
async fn retryable_failures_retry_with_backoff_then_fail_with_real_error() {
    let db = init_database_memory().await.unwrap();
    // Every send fails with a retryable classified error: the drain must try
    // 1 initial + 2 bounded retries, then settle failed with the real reason.
    let stack = build_stack(db.pool().clone(), u32::MAX).await;

    // Seeding must not consume a failure: use a separate conversation? The
    // flaky registry fails EVERY send, including the seed — so seed by
    // creating the conversation via a failing first turn, which still binds.
    let session = stack
        .session_manager
        .get_session_by_id(&stack.session_id)
        .await
        .unwrap()
        .unwrap();
    let sent = stack
        .message_svc
        .send_to_agent(&session, "first", PluginType::Telegram, "drain:first")
        .await
        .unwrap();
    stack
        .session_manager
        .bind_conversation(&stack.session_id, &sent.conversation_id)
        .await
        .unwrap();
    let conversation_id = sent.conversation_id;
    wait_until_idle(&stack.conversation_svc, &conversation_id).await;

    enqueue(&stack, &conversation_id, "doomed prompt", "drain:doomed").await;
    spawn_drain(&stack);

    let rows = wait_for_queue_state(
        &stack,
        &conversation_id,
        |rows| rows.iter().any(|r| r.text == "doomed prompt" && r.state == "failed"),
        "prompt settled failed after bounded retries",
    )
    .await;
    let doomed = rows.iter().find(|r| r.text == "doomed prompt").unwrap();
    assert_eq!(doomed.attempts, 2, "exactly two bounded retries are spent");

    // The chat received the real error text, not a generic busy line.
    let texts = stack.recorder.texts();
    assert!(
        texts.iter().any(|t| t.contains("处理失败")),
        "failure notice must reach the chat: {texts:?}"
    );
}

#[tokio::test]
async fn startup_expires_stale_prompts_and_notifies_chat() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone(), 0).await;
    let conversation_id = seed_conversation(&stack).await;

    // A prompt queued 31 minutes ago (enqueue accepts the caller clock).
    let stale_at = nomifun_common::now_ms() - 31 * 60 * 1000;
    let outcome = stack
        .channel_repo
        .enqueue_pending_prompt(
            &nomifun_db::models::NewChannelPendingPromptRow {
                channel_plugin_id: stack.plugin_id.clone(),
                chat_id: "chat-drain".to_owned(),
                channel_session_id: stack.session_id.clone(),
                conversation_id: conversation_id.clone(),
                text: "too old".to_owned(),
                idempotency_key: "drain:stale".to_owned(),
            },
            stale_at,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, nomifun_db::PendingPromptEnqueue::Queued { .. }));

    spawn_drain(&stack);

    wait_for_queue_state(
        &stack,
        &conversation_id,
        |rows| rows.iter().any(|r| r.text == "too old" && r.state == "expired"),
        "stale prompt expired on startup",
    )
    .await;
    for _ in 0..200 {
        if stack.recorder.texts().iter().any(|t| t.contains("已放弃")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("expiry notice never reached the chat: {:?}", stack.recorder.texts());
}
