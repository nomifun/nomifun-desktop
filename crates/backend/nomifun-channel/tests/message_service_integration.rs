use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_ai_agent::runtime_handle::{AgentRuntimeHandle, AgentRuntimeControl};
use nomifun_ai_agent::protocol::events::FinishEventData;
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{AgentSendError, AgentStreamEvent, MockAgentRuntime, AgentRuntimeRegistry};
use nomifun_api_types::WebSocketMessage;
use nomifun_channel::channel_settings::ChannelSettingsService;
use nomifun_channel::message_service::ChannelMessageService;
use nomifun_channel::types::PluginType;
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use nomifun_conversation::ConversationService;
use nomifun_conversation::skill_resolver::{ResolvedAgentSkill, SkillResolver};
use nomifun_db::models::{ChannelSessionRow, NewChannelPluginRow};
use nomifun_db::{
    CreateProviderParams, IClientPreferenceRepository, IProviderRepository, SqliteAcpSessionRepository,
    SqliteAgentMetadataRepository, SqliteChannelRepository, SqliteClientPreferenceRepository,
    SqliteConversationRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_realtime::UserEventSink;
use tokio::sync::broadcast;

const DEFAULT_PROVIDER: &str = "018f1234-5678-7abc-8def-012345678940";
const COMPANION_PROVIDER: &str = "018f1234-5678-7abc-8def-012345678941";
 const SESSION_A: &str = "018f1234-5678-7abc-8def-012345678943";
const SESSION_B: &str = "018f1234-5678-7abc-8def-012345678944";
const CHANNEL_USER_ID: &str = "018f1234-5678-7abc-8def-012345678945";
const COMPANION_X: &str = "018f1234-5678-7abc-8def-012345678948";
const COMPANION_Y: &str = "018f1234-5678-7abc-8def-012345678949";
const CS_AGENT: &str = "018f1234-5678-7abc-8def-01234567894b";

struct TestBroadcaster {
    events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
}

impl TestBroadcaster {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl UserEventSink for TestBroadcaster {
    fn send_to_user(&self, _user_id: &str, event: WebSocketMessage<serde_json::Value>) {
        self.events.lock().unwrap().push(event);
    }
}

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

struct ScriptedAgent {
    conversation_id: String,
    event_tx: broadcast::Sender<AgentStreamEvent>,
}

impl ScriptedAgent {
    fn new(conversation_id: &str) -> Self {
        let (event_tx, _) = broadcast::channel(16);
        Self {
            conversation_id: conversation_id.to_owned(),
            event_tx,
        }
    }
}

#[async_trait]
impl AgentRuntimeControl for ScriptedAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }

    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    fn workspace(&self) -> &str {
        "/tmp/nomifun-channel-test"
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
        let _ = self.event_tx.send(AgentStreamEvent::Finish(FinishEventData::default()));
        Ok(())
    }

    async fn cancel(&self) -> Result<(), AppError> {
        Ok(())
    }

    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        Ok(())
    }
}

impl MockAgentRuntime for ScriptedAgent {}

struct RecordingAgentRuntimeRegistry {
    agents: Mutex<std::collections::HashMap<String, AgentRuntimeHandle>>,
}

impl RecordingAgentRuntimeRegistry {
    fn new() -> Self {
        Self {
            agents: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl AgentRuntimeRegistry for RecordingAgentRuntimeRegistry {
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

        let agent = AgentRuntimeHandle::Mock(Arc::new(ScriptedAgent::new(conversation_id)));
        agents.insert(conversation_id.to_owned(), agent.clone());
        Ok(agent)
    }

    fn terminate(&self, conversation_id: &str, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.agents.lock().unwrap().remove(conversation_id);
        Ok(())
    }

    fn terminate_and_wait(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let _ = self.terminate(conversation_id, reason);
        Box::pin(std::future::ready(()))
    }

    fn terminate_all(&self) {
        self.agents.lock().unwrap().clear();
    }

    fn active_runtime_count(&self) -> usize {
        self.agents.lock().unwrap().len()
    }

    fn collect_idle_runtimes(&self, _idle_threshold_ms: TimestampMs) -> Vec<String> {
        Vec::new()
    }
}

/// Seed every provider id used by this integration fixture and give each
/// platform a valid default model. The database deliberately rejects dangling
/// Conversation model authorities, so tests must model a real provider catalog.
async fn seed_channel_models(pool: &nomifun_db::SqlitePool) {
    let providers = SqliteProviderRepository::new(pool.clone());
    for id in [DEFAULT_PROVIDER, COMPANION_PROVIDER] {
        providers
            .create(CreateProviderParams {
                provider_id: Some(id),
                platform: "openai",
                name: "Channel test provider",
                base_url: "https://example.invalid/v1",
                api_key_encrypted: "test-only",
                models: r#"["channel-test-model","m","pa-model-v1"]"#,
                enabled: true,
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
    }

    let preferences = SqliteClientPreferenceRepository::new(pool.clone());
    let model =
        format!(r#"{{"provider_id":"{DEFAULT_PROVIDER}","model":"channel-test-model"}}"#);
    preferences
        .upsert_batch(&[
            ("channels.telegram.defaultModel", model.as_str()),
            ("channels.lark.defaultModel", model.as_str()),
            ("channels.dingtalk.defaultModel", model.as_str()),
            ("channels.weixin.defaultModel", model.as_str()),
        ])
        .await
        .unwrap();
}

#[tokio::test]
async fn send_to_agent_warms_cold_task_before_returning_stream_subscription() {
    let db = init_database_memory().await.unwrap();
    let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let pool = db.pool().clone();
    seed_channel_models(&pool).await;

    let runtime_registry: Arc<dyn AgentRuntimeRegistry> = Arc::new(RecordingAgentRuntimeRegistry::new());
    let conversation_svc = Arc::new(ConversationService::new(
        Arc::<str>::from(installation_owner.as_str()),
        std::env::temp_dir(),
        Arc::new(TestBroadcaster::new()),
        Arc::new(NoopSkillResolver),
        Arc::clone(&runtime_registry),
        Arc::new(SqliteConversationRepository::new(pool.clone())),
        Arc::new(SqliteAgentMetadataRepository::new(pool.clone())),
        Arc::new(SqliteAcpSessionRepository::new(pool.clone())),
        Arc::new(nomifun_conversation::NoExecutionConversationBoundary),
    ));

    let settings = Arc::new(ChannelSettingsService::new(Arc::new(
        SqliteClientPreferenceRepository::new(pool.clone()),
    )));
    let message_svc = ChannelMessageService::new(
        Arc::clone(&conversation_svc),
        Arc::clone(&runtime_registry),
        settings,
        Arc::new(SqliteChannelRepository::new(pool)),
        installation_owner,
    );

    let session = ChannelSessionRow {
        channel_session_id: SESSION_A.to_owned(),
        channel_user_id: CHANNEL_USER_ID.to_owned(),
        agent_type: "nomi".to_owned(),
        conversation_id: None,
        workspace: None,
        chat_id: Some("7088048016".to_owned()),
        channel_plugin_id: None,
        created_at: 1,
        last_activity: 1,
    };

    for platform in [
        PluginType::Telegram,
        PluginType::Lark,
        PluginType::Dingtalk,
        PluginType::Weixin,
    ] {
        let idempotency_key = format!("test:cold-start:{platform}");
        let result = message_svc
            .send_to_agent(&session, "hello", platform, &idempotency_key)
            .await
            .unwrap();

        assert!(
            result.stream_rx.is_some(),
            "channel relay must have an agent stream receiver after cold start for {platform:?}"
        );
        assert!(runtime_registry.get_runtime(&result.conversation_id).is_some());
        wait_until_idle(&conversation_svc, &result.conversation_id).await;
    }
}

// ── Fix 3/4 support: last_user_text + is_conversation_busy ──────────────

struct TestStack {
    conversation_svc: Arc<ConversationService>,
    message_svc: ChannelMessageService,
    runtime: Arc<nomifun_conversation::runtime_state::ConversationRuntimeStateService>,
    channel_repo: Arc<SqliteChannelRepository>,
    installation_owner: String,
}

async fn build_stack(pool: nomifun_db::SqlitePool) -> TestStack {
    let installation_owner = nomifun_db::installation_owner_id(&pool).await.unwrap();
    seed_channel_models(&pool).await;
    let runtime_registry: Arc<dyn AgentRuntimeRegistry> = Arc::new(RecordingAgentRuntimeRegistry::new());
    let runtime = Arc::new(nomifun_conversation::runtime_state::ConversationRuntimeStateService::default());
    let conversation_svc = Arc::new(
        ConversationService::new(
            Arc::<str>::from(installation_owner.as_str()),
            std::env::temp_dir(),
            Arc::new(TestBroadcaster::new()),
            Arc::new(NoopSkillResolver),
            Arc::clone(&runtime_registry),
            Arc::new(SqliteConversationRepository::new(pool.clone())),
            Arc::new(SqliteAgentMetadataRepository::new(pool.clone())),
            Arc::new(SqliteAcpSessionRepository::new(pool.clone())),
            Arc::new(nomifun_conversation::NoExecutionConversationBoundary),
        )
        .with_runtime_state(Arc::clone(&runtime)),
    );

    let settings = Arc::new(ChannelSettingsService::new(Arc::new(
        SqliteClientPreferenceRepository::new(pool.clone()),
    )));
    let channel_repo = Arc::new(SqliteChannelRepository::new(pool));
    let message_svc = ChannelMessageService::new(
        Arc::clone(&conversation_svc),
        Arc::clone(&runtime_registry),
        settings,
        channel_repo.clone(),
        installation_owner.clone(),
    );

    TestStack {
        conversation_svc,
        message_svc,
        runtime,
        channel_repo,
        installation_owner,
    }
}

fn make_session(conversation_id: Option<String>) -> ChannelSessionRow {
    ChannelSessionRow {
        channel_session_id: SESSION_A.to_owned(),
        channel_user_id: CHANNEL_USER_ID.to_owned(),
        agent_type: "nomi".to_owned(),
        conversation_id,
        workspace: None,
        chat_id: Some("7088048016".to_owned()),
        channel_plugin_id: None,
        created_at: 1,
        last_activity: 1,
    }
}

/// Waits for the background turn spawned by `send_message` to release its
/// Agent turn handle so the next send doesn't hit the turn-conflict guard.
async fn wait_until_idle(svc: &Arc<ConversationService>, conversation_id: &str) {
    use nomifun_api_types::ConversationRuntimeStateKind;
    for _ in 0..500 {
        let summary = svc.runtime_summary_for(conversation_id).await;
        if summary.state == ConversationRuntimeStateKind::Idle {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("conversation {conversation_id} never became idle");
}

#[tokio::test]
async fn last_user_text_returns_latest_user_prompt() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    // First prompt creates the conversation; second one is the newest.
    let session = make_session(None);
    let first = stack
        .message_svc
        .send_to_agent(
            &session,
            "first prompt",
            PluginType::Telegram,
            "test:last-user:first",
        )
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &first.conversation_id).await;

    let bound_session = make_session(Some(first.conversation_id.clone()));
    stack
        .message_svc
        .send_to_agent(
            &bound_session,
            "second prompt",
            PluginType::Telegram,
            "test:last-user:second",
        )
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &first.conversation_id).await;

    let text = stack.message_svc.last_user_text(&first.conversation_id).await.unwrap();
    assert_eq!(text.as_deref(), Some("second prompt"));
}

#[tokio::test]
async fn last_user_text_none_for_unknown_conversation() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    // Unknown conversation maps to a lookup error, not a silent None.
    let missing = nomifun_common::ConversationId::new();
    let result = stack.message_svc.last_user_text(missing.as_str()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn is_conversation_busy_reflects_active_turn_handle() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let session = make_session(None);
    let sent = stack
        .message_svc
        .send_to_agent(
            &session,
            "hello",
            PluginType::Telegram,
            "test:busy-state",
        )
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;

    assert!(!stack.message_svc.is_conversation_busy(&sent.conversation_id).await);

    // Claiming the turn is exactly what send_message does while a prompt is
    // in flight → the channel guard must report busy.
    let _turn_handle = stack.runtime.try_acquire_turn(&sent.conversation_id).unwrap();
    assert!(stack.message_svc.is_conversation_busy(&sent.conversation_id).await);

    drop(_turn_handle);
    assert!(!stack.message_svc.is_conversation_busy(&sent.conversation_id).await);
}

/// A Conflict on an idle conversation is a real failure, not a concurrent
/// turn — it must surface the underlying error so the user is not trapped in
/// a "still being processed" loop (the reported WeChat bug: a knowledge
/// workspace lease clash was presented as busy forever). Reusing an
/// idempotency key with different content is a deterministic idle Conflict.
#[tokio::test]
async fn idle_conflict_surfaces_real_error_instead_of_busy() {
    use nomifun_channel::error::ChannelError;

    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let session = make_session(None);
    let sent = stack
        .message_svc
        .send_to_agent(&session, "hello", PluginType::Telegram, "test:conflict-reuse")
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;
    assert!(!stack.message_svc.is_conversation_busy(&sent.conversation_id).await);

    let bound_session = make_session(Some(sent.conversation_id.clone()));
    let error = stack
        .message_svc
        .send_to_agent(
            &bound_session,
            "different content",
            PluginType::Telegram,
            "test:conflict-reuse",
        )
        .await
        .unwrap_err();

    match error {
        ChannelError::MessageSendFailed(reason) => assert!(
            reason.contains("idempotency key was reused"),
            "the real Conflict reason must reach the user, got: {reason}"
        ),
        other => panic!(
            "an idle-conversation Conflict must not be disguised as busy, got: {other:?}"
        ),
    }
}

/// A Conflict while the conversation really is working a turn is the
/// turn-claim race — it must keep answering with the friendly busy notice.
#[tokio::test]
async fn active_turn_conflict_still_maps_to_busy() {
    use nomifun_channel::error::ChannelError;

    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let session = make_session(None);
    let sent = stack
        .message_svc
        .send_to_agent(&session, "hello", PluginType::Telegram, "test:busy-conflict-first")
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;

    // Exactly what send_message does while a prompt is in flight.
    let _turn_handle = stack.runtime.try_acquire_turn(&sent.conversation_id).unwrap();

    let bound_session = make_session(Some(sent.conversation_id.clone()));
    let error = stack
        .message_svc
        .send_to_agent(
            &bound_session,
            "second prompt",
            PluginType::Telegram,
            "test:busy-conflict-second",
        )
        .await
        .unwrap_err();

    assert!(
        matches!(error, ChannelError::ConversationBusy(ref cid) if cid == &sent.conversation_id),
        "a concurrent-turn Conflict must keep the busy signal and carry the resolved conversation, got: {error:?}"
    );
}

// ── Busy-time prompt queue (spec D1, Task 2) ─────────────────────────

/// Busy-time prompts are persisted (not dropped) with correct FIFO positions,
/// and the 「取消排队」 command clears exactly this chat's scope.
#[tokio::test]
async fn busy_prompts_enqueue_with_positions_and_cancel_clears_chat() {
    use nomifun_db::PendingPromptEnqueue;

    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;
    let channel_plugin_id = create_plain_channel(&stack.channel_repo).await;
    let conversation_id = nomifun_common::ConversationId::new().into_string();

    let first = stack
        .message_svc
        .enqueue_busy_prompt(
            &channel_plugin_id,
            "chat-1",
            SESSION_A,
            &conversation_id,
            "queued one",
            "test:queue:first",
        )
        .await
        .unwrap();
    assert!(matches!(first, PendingPromptEnqueue::Queued { position: 1, .. }));

    let second = stack
        .message_svc
        .enqueue_busy_prompt(
            &channel_plugin_id,
            "chat-1",
            SESSION_A,
            &conversation_id,
            "queued two",
            "test:queue:second",
        )
        .await
        .unwrap();
    assert!(matches!(second, PendingPromptEnqueue::Queued { position: 2, .. }));

    // Nothing was lost: the FIFO head is the first prompt.
    use nomifun_db::IChannelRepository;
    let head = stack
        .channel_repo
        .peek_next_queued(&conversation_id)
        .await
        .unwrap()
        .expect("queued rows must survive");
    assert_eq!(head.text, "queued one");

    // Cancel clears this chat's queue and reports the count.
    assert_eq!(
        stack
            .message_svc
            .cancel_chat_queue(&channel_plugin_id, "chat-1")
            .await
            .unwrap(),
        2
    );
    assert!(
        stack
            .channel_repo
            .peek_next_queued(&conversation_id)
            .await
            .unwrap()
            .is_none()
    );
}

// ── Channel companion binding resolution + single-session routing ──────────────

/// Profile stub: maps each companion id to a pre-seeded single-session
/// conversation id (what `CompanionManager.create` would return in production),
/// records every `ensure_companion_session` call, and uses `companion_y` as the
/// per-platform binding. An empty `sessions` map models a
/// companion with no chat model configured (ensure returns `None`).
struct StubProfile {
    sessions: std::collections::HashMap<String, String>,
    calls: Mutex<Vec<String>>,
}

impl StubProfile {
    fn new(sessions: std::collections::HashMap<String, String>) -> Self {
        Self {
            sessions,
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl nomifun_channel::message_service::ChannelAgentProfile for StubProfile {
    async fn companion_model(&self, _companion_id: &str) -> Option<nomifun_common::ProviderWithModel> {
        None
    }
    async fn channel_companion_id(&self, _platform: &str) -> Option<String> {
        Some(COMPANION_Y.to_owned())
    }
    async fn companion_exists(&self, _companion_id: &str) -> bool {
        true
    }
    async fn ensure_companion_session(&self, companion_id: &str) -> Option<String> {
        self.calls.lock().unwrap().push(companion_id.to_owned());
        self.sessions.get(companion_id).cloned()
    }
}

/// Seed a companion's single-session conversation (the row `CompanionManager`
/// would own), returning its canonical conversation ID.
async fn seed_companion_session(
    svc: &Arc<ConversationService>,
    installation_owner: &str,
    companion_id: &str,
) -> String {
    let req = nomifun_api_types::CreateConversationRequest {
        r#type: AgentType::Nomi,
        name: Some(format!("和 {companion_id} 聊天")),
        model: Some(nomifun_common::ProviderWithModel {
            provider_id: COMPANION_PROVIDER.to_owned(),
            model: "m".to_owned(),
            use_model: Some("m".to_owned()),
        }),
        source: None,
        channel_chat_id: None,
        preset_id: None,
        preset_overrides: None,
        delegation_policy: Default::default(),
        execution_model_pool: None,
        decision_policy: Default::default(),
        execution_template_id: None,
        extra: serde_json::json!({ "companion_session": true, "companion_id": companion_id }),
    };
    svc.create(installation_owner, req).await.unwrap().conversation_id
}

async fn bind_channel_to_companion(
    repo: &Arc<SqliteChannelRepository>,
    companion_id: &str,
) -> String {
    use nomifun_db::IChannelRepository;
    let now = nomifun_common::now_ms();
    repo.create_plugin(&NewChannelPluginRow {
        r#type: "telegram".to_owned(),
        name: "Telegram Bot".to_owned(),
        enabled: true,
        config: "enc".to_owned(),
        status: None,
        last_connected: None,
        companion_id: Some(companion_id.to_owned()),
        bot_key: Some("42".to_owned()),
        owner_domain: "companion".into(),
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap()
    .channel_plugin_id
}

/// The channel row's own companion binding wins over the profile fallback, and
/// either way the turn is routed INTO that companion's single session (not a
/// freshly-minted channel conversation).
#[tokio::test]
async fn channel_companion_turn_routes_into_companion_single_session() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let conv_x = seed_companion_session(
        &stack.conversation_svc,
        &stack.installation_owner,
        COMPANION_X,
    )
    .await;
    let conv_y = seed_companion_session(
        &stack.conversation_svc,
        &stack.installation_owner,
        COMPANION_Y,
    )
    .await;
    let sessions = std::collections::HashMap::from([
        (COMPANION_X.to_owned(), conv_x.clone()),
        (COMPANION_Y.to_owned(), conv_y.clone()),
    ]);
    let message_svc = stack
        .message_svc
        .with_channel_agent_profile(Arc::new(StubProfile::new(sessions)));

    let channel_plugin_id = bind_channel_to_companion(&stack.channel_repo, COMPANION_X).await;

    // Bound channel → channel companion (companion_x) wins; the turn runs on
    // companion_x's single session conversation, NOT a new channel conversation.
    let mut bound = make_session(None);
    bound.channel_plugin_id = Some(channel_plugin_id);
    let sent = message_svc
        .send_to_agent(
            &bound,
            "hi",
            PluginType::Telegram,
            "test:binding-precedence:bound",
        )
        .await
        .unwrap();
    assert_eq!(sent.conversation_id, conv_x);
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;

    // No channel binding → profile fallback companion (companion_y) → its session.
    let mut unbound = make_session(None);
    unbound.channel_session_id = SESSION_B.to_owned();
    unbound.chat_id = Some("other-chat".to_owned());
    let sent = message_svc
        .send_to_agent(
            &unbound,
            "hi",
            PluginType::Telegram,
            "test:binding-precedence:unbound",
        )
        .await
        .unwrap();
    assert_eq!(sent.conversation_id, conv_y);
}

/// Two different IM chats bound to the SAME companion both land in that
/// companion's ONE session — the unification guarantee. No separate channel
/// conversation is created for either.
#[tokio::test]
async fn companion_im_turns_share_one_session() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let conv_x = seed_companion_session(
        &stack.conversation_svc,
        &stack.installation_owner,
        COMPANION_X,
    )
    .await;
    let sessions =
        std::collections::HashMap::from([(COMPANION_X.to_owned(), conv_x.clone())]);
    let message_svc = stack
        .message_svc
        .with_channel_agent_profile(Arc::new(StubProfile::new(sessions)));
    let channel_plugin_id = bind_channel_to_companion(&stack.channel_repo, COMPANION_X).await;

    let mut chat_a = make_session(None);
    chat_a.channel_plugin_id = Some(channel_plugin_id.clone());
    chat_a.chat_id = Some("chat-A".to_owned());
    let a = message_svc
        .send_to_agent(
            &chat_a,
            "hi from A",
            PluginType::Telegram,
            "test:shared-companion:chat-a",
        )
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &a.conversation_id).await;

    let mut chat_b = make_session(None);
    chat_b.channel_session_id = SESSION_B.to_owned();
    chat_b.channel_plugin_id = Some(channel_plugin_id);
    chat_b.chat_id = Some("chat-B".to_owned());
    let b = message_svc
        .send_to_agent(
            &chat_b,
            "hi from B",
            PluginType::Telegram,
            "test:shared-companion:chat-b",
        )
        .await
        .unwrap();

    assert_eq!(a.conversation_id, conv_x);
    assert_eq!(b.conversation_id, conv_x, "both IM chats must share the companion's single session");
}

/// A companion with no chat model (ensure returns None) refuses the turn with a
/// distinct error instead of silently minting a separate channel conversation.
#[tokio::test]
async fn companion_without_model_refuses_turn() {
    use nomifun_channel::error::ChannelError;

    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;
    // Empty sessions map → ensure_companion_session returns None for every companion.
    let message_svc = stack
        .message_svc
        .with_channel_agent_profile(Arc::new(StubProfile::new(std::collections::HashMap::new())));
    let channel_plugin_id = bind_channel_to_companion(&stack.channel_repo, COMPANION_X).await;

    let mut bound = make_session(None);
    bound.channel_plugin_id = Some(channel_plugin_id);
    let err = message_svc
        .send_to_agent(
            &bound,
            "hi",
            PluginType::Telegram,
            "test:model-less-companion",
        )
        .await
        .expect_err("a model-less companion must refuse the turn");
    assert!(matches!(err, ChannelError::CompanionNotReady(_)));
}


/// Creates a plain (unbound) bot channel row and returns its id.
async fn create_plain_channel(repo: &Arc<SqliteChannelRepository>) -> String {
    use nomifun_db::IChannelRepository;
    let now = nomifun_common::now_ms();
    repo.create_plugin(&NewChannelPluginRow {
        r#type: "telegram".to_owned(),
        name: "Telegram Bot".to_owned(),
        enabled: true,
        config: "enc".to_owned(),
        status: None,
        last_connected: None,
        companion_id: None,
        bot_key: Some("43".to_owned()),
        owner_domain: "companion".into(),
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap()
    .channel_plugin_id
}

// ── 客服 / customer-service channel routing ────────────────────────────────

/// Recording CsRouting stub: binds exactly one plugin, records every visitor
/// message it is handed and answers with a fixed reply.
struct RecordingCsRouting {
    bound_plugin: String,
    calls: std::sync::Mutex<Vec<(String, String, String, String)>>,
}

#[async_trait]
impl nomifun_channel::message_service::CsRouting for RecordingCsRouting {
    async fn binding_for(&self, channel_plugin_id: &str) -> Option<String> {
        (channel_plugin_id == self.bound_plugin).then(|| CS_AGENT.to_owned())
    }
    async fn handle_visitor_message(
        &self,
        cs_agent_id: &str,
        channel_plugin_id: &str,
        channel_user_id: &str,
        chat_id: &str,
        text: &str,
    ) -> Result<String, String> {
        self.calls.lock().unwrap().push((
            cs_agent_id.to_owned(),
            channel_plugin_id.to_owned(),
            format!("{channel_user_id}:{chat_id}"),
            text.to_owned(),
        ));
        Ok("客服回复".to_owned())
    }
}

/// (a) A customer-service-bound bot's message is handed to the CsRouting seam
/// with the full identity, and its reply comes back verbatim — no
/// Conversation is created and the companion path is never touched.
#[tokio::test]
async fn cs_bound_bot_routes_to_seam_not_conversation() {
    use nomifun_channel::error::ChannelError;

    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;
    let channel_plugin_id = create_plain_channel(&stack.channel_repo).await;

    let routing = Arc::new(RecordingCsRouting {
        bound_plugin: channel_plugin_id.clone(),
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let message_svc = stack.message_svc.with_cs_routing(routing.clone());

    // The loop-level gate resolves the binding…
    assert_eq!(
        message_svc.cs_bound_agent(&channel_plugin_id).await.as_deref(),
        Some(CS_AGENT)
    );
    // …and the seam handles the visitor message.
    let reply = message_svc
        .cs_handle_visitor_message(CS_AGENT, &channel_plugin_id, CHANNEL_USER_ID, "chat-9", "你好")
        .await
        .unwrap();
    assert_eq!(reply, "客服回复");
    {
        let calls = routing.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, CS_AGENT);
        assert_eq!(calls[0].1, channel_plugin_id);
        assert_eq!(calls[0].2, format!("{CHANNEL_USER_ID}:chat-9"));
        assert_eq!(calls[0].3, "你好");
    }

    // Defensive: the conversation path REFUSES a cs-bound bot instead of
    // leaking a Conversation.
    let mut session = make_session(None);
    session.channel_plugin_id = Some(channel_plugin_id);
    let err = message_svc
        .send_to_agent(&session, "你好", PluginType::Telegram, "test:cs-bound:refuse")
        .await
        .expect_err("cs-bound bot must never enter the conversation path");
    assert!(matches!(err, ChannelError::MessageSendFailed(_)));
}

/// (b) With the seam wired but the bot UNBOUND, the dedicated per-chat path
/// behaves exactly as before (regression guard for the seam insertion).
#[tokio::test]
async fn unbound_bot_keeps_companion_path_with_seam_wired() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;
    let channel_plugin_id = create_plain_channel(&stack.channel_repo).await;

    let routing = Arc::new(RecordingCsRouting {
        bound_plugin: "none".to_owned(), // binds nothing
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let message_svc = stack.message_svc.with_cs_routing(routing.clone());

    assert!(message_svc.cs_bound_agent(&channel_plugin_id).await.is_none());

    let mut session = make_session(None);
    session.channel_plugin_id = Some(channel_plugin_id);
    let sent = message_svc
        .send_to_agent(&session, "hello", PluginType::Telegram, "test:cs-unbound:normal")
        .await
        .unwrap();
    wait_until_idle(&stack.conversation_svc, &sent.conversation_id).await;
    assert!(routing.calls.lock().unwrap().is_empty(), "seam must not be consulted");
    let conv = stack
        .conversation_svc
        .get(&stack.installation_owner, &sent.conversation_id)
        .await
        .unwrap();
    assert_eq!(conv.r#type, AgentType::Nomi);
}
