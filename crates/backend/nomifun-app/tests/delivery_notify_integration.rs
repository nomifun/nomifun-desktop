//! Integration tests for the spec D2 delivery-notify loop (Task 4).
//!
//! Real in-memory database + real `ConversationService` + the real
//! `DeliveryNotifyObserver`: a keyed turn on the target conversation with a
//! registration delivers exactly one receipt message into the requester
//! session, whose reply is relayed to its bound IM chat; duplicate
//! completions deliver nothing twice; a delivery-notify origin turn cannot
//! register further receipts (loop guard).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_ai_agent::protocol::events::{FinishEventData, TextEventData};
use nomifun_ai_agent::runtime_handle::{AgentRuntimeControl, AgentRuntimeHandle};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{AgentRuntimeRegistry, AgentSendError, AgentStreamEvent, MockAgentRuntime};
use nomifun_api_types::{SendMessageRequest, WebSocketMessage};
use nomifun_app::delivery_notify::DeliveryNotifyObserver;
use nomifun_channel::error::ChannelError;
use nomifun_channel::pending_decision::PendingDecisionStore;
use nomifun_channel::stream_relay::ChannelSender;
use nomifun_channel::types::{OutgoingMedia, UnifiedOutgoingMessage};
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use nomifun_conversation::{
    ConversationService, DeliveryNotifyRegistration, TurnCompletionObserver,
};
use nomifun_conversation::skill_resolver::{ResolvedAgentSkill, SkillResolver};
use nomifun_db::models::{NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow};
use nomifun_db::{
    CreateProviderParams, IChannelRepository, IConversationRepository, IProviderRepository,
    SqliteAcpSessionRepository, SqliteAgentMetadataRepository, SqliteChannelRepository,
    SqliteConversationRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_realtime::UserEventSink;
use tokio::sync::broadcast;

const PROVIDER: &str = "018f1234-5678-7abc-8def-0123456789b0";

struct NullSink;

impl UserEventSink for NullSink {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
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
        "/tmp/nomifun-delivery-notify-test"
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
        let _ = self.event_tx.send(AgentStreamEvent::Text(TextEventData {
            content: "target finished the delegated task".to_owned(),
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

impl MockAgentRuntime for ScriptedAgent {}

struct ScriptedRegistry {
    agents: Mutex<std::collections::HashMap<String, AgentRuntimeHandle>>,
}

impl ScriptedRegistry {
    fn new() -> Self {
        Self {
            agents: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl AgentRuntimeRegistry for ScriptedRegistry {
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

struct MessageRecorder {
    sends: Mutex<Vec<String>>,
}

impl MessageRecorder {
    fn new() -> Self {
        Self {
            sends: Mutex::new(Vec::new()),
        }
    }
    fn texts(&self) -> Vec<String> {
        self.sends.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelSender for MessageRecorder {
    async fn send_message(
        &self,
        _plugin_id: &str,
        _chat_id: &str,
        message: UnifiedOutgoingMessage,
    ) -> Result<String, ChannelError> {
        self.sends
            .lock()
            .unwrap()
            .push(message.text.unwrap_or_default());
        Ok("mid".to_owned())
    }
    async fn edit_message(
        &self,
        _plugin_id: &str,
        _chat_id: &str,
        _message_id: &str,
        message: UnifiedOutgoingMessage,
    ) -> Result<(), ChannelError> {
        if let Some(text) = message.text {
            self.sends.lock().unwrap().push(text);
        }
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
    conversation_svc: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    conv_repo: Arc<SqliteConversationRepository>,
    channel_repo: Arc<SqliteChannelRepository>,
    recorder: Arc<MessageRecorder>,
    observer: Arc<DeliveryNotifyObserver>,
    owner: String,
}

async fn build_stack(pool: nomifun_db::SqlitePool) -> Stack {
    let owner = nomifun_db::installation_owner_id(&pool).await.unwrap();

    let providers = SqliteProviderRepository::new(pool.clone());
    providers
        .create(CreateProviderParams {
            provider_id: Some(PROVIDER),
            platform: "openai",
            name: "Delivery notify provider",
            base_url: "https://example.invalid/v1",
            api_key_encrypted: "test-only",
            models: r#"["m"]"#,
            enabled: true,
            capabilities: "[]",
            model_context_limits: None,
            model_protocols: None,
            model_descriptions: None,
            model_enabled: None,
            model_health: None,
            bedrock_config: None,
            is_full_url: false,
            sort_order: None,
        })
        .await
        .unwrap();

    let runtime_registry: Arc<dyn AgentRuntimeRegistry> = Arc::new(ScriptedRegistry::new());
    let conv_repo = Arc::new(SqliteConversationRepository::new(pool.clone()));
    let conversation_svc = ConversationService::new(
        Arc::<str>::from(owner.as_str()),
        std::env::temp_dir(),
        Arc::new(NullSink),
        Arc::new(NoopSkillResolver),
        Arc::clone(&runtime_registry),
        conv_repo.clone(),
        Arc::new(SqliteAgentMetadataRepository::new(pool.clone())),
        Arc::new(SqliteAcpSessionRepository::new(pool.clone())),
        Arc::new(nomifun_conversation::NoExecutionConversationBoundary),
    );

    let channel_repo = Arc::new(SqliteChannelRepository::new(pool));
    let recorder = Arc::new(MessageRecorder::new());
    let observer = Arc::new(DeliveryNotifyObserver::new(
        conversation_svc.clone(),
        Arc::clone(&runtime_registry),
        Arc::<str>::from(owner.as_str()),
        channel_repo.clone() as Arc<dyn IChannelRepository>,
        Arc::clone(&recorder) as Arc<dyn ChannelSender>,
        PendingDecisionStore::new(),
        None,
    ));
    conversation_svc.with_turn_completion_observer(
        Arc::clone(&observer) as Arc<dyn TurnCompletionObserver>
    );

    Stack {
        conversation_svc,
        runtime_registry,
        conv_repo,
        channel_repo,
        recorder,
        observer,
        owner,
    }
}

async fn create_conversation(stack: &Stack, name: &str) -> String {
    let req = nomifun_api_types::CreateConversationRequest {
        r#type: AgentType::Nomi,
        name: Some(name.to_owned()),
        model: Some(nomifun_common::ProviderWithModel {
            provider_id: PROVIDER.to_owned(),
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
        extra: serde_json::json!({}),
    };
    stack
        .conversation_svc
        .create(&stack.owner, req)
        .await
        .unwrap()
        .conversation_id
}

/// Bind an IM chat to `conversation_id` so the observer relays into it.
async fn bind_channel_session(stack: &Stack, conversation_id: &str) {
    let now = nomifun_common::now_ms();
    let plugin = stack
        .channel_repo
        .create_plugin(&NewChannelPluginRow {
            r#type: "telegram".to_owned(),
            name: "Notify bot".to_owned(),
            enabled: true,
            config: "enc".to_owned(),
            status: None,
            last_connected: None,
            companion_id: None,
            bot_key: Some("notify".to_owned()),
            owner_domain: "companion".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let user = stack
        .channel_repo
        .create_user(&NewChannelUserRow {
            platform_user_id: "tg_notify".to_owned(),
            platform_type: "telegram".to_owned(),
            channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
            display_name: Some("Notify".to_owned()),
            authorized_at: now,
            last_active: None,
        })
        .await
        .unwrap();
    stack
        .channel_repo
        .get_or_create_session(
            &user.channel_user_id,
            "chat-notify",
            &plugin.channel_plugin_id,
            &NewChannelSessionRow {
                channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
                channel_user_id: user.channel_user_id.clone(),
                agent_type: "nomi".to_owned(),
                conversation_id: Some(conversation_id.to_owned()),
                workspace: None,
                chat_id: Some("chat-notify".to_owned()),
                channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
                created_at: now,
                last_activity: now,
            },
        )
        .await
        .unwrap();
}

fn send_request(content: &str) -> SendMessageRequest {
    SendMessageRequest {
        content: content.to_owned(),
        files: vec![],
        inject_skills: vec![],
        hidden: false,
        origin: Some("companion".to_owned()),
        channel_platform: None,
    }
}

async fn user_texts(stack: &Stack, conversation_id: &str) -> Vec<String> {
    let query = nomifun_api_types::ListMessagesQuery {
        page: Some(1),
        page_size: Some(50),
        order: Some("ASC".into()),
        content_mode: None,
        cursor: None,
    };
    stack
        .conversation_svc
        .list_messages(&stack.owner, conversation_id, query)
        .await
        .unwrap()
        .items
        .iter()
        .filter(|m| m.position == Some(nomifun_common::MessagePosition::Right))
        .filter_map(|m| m.content.get("content").and_then(|v| v.as_str()).map(str::to_owned))
        .collect()
}

async fn wait_for<'a, F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool> + 'a,
{
    for _ in 0..800 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition never became true: {what}");
}

#[tokio::test]
async fn notify_back_delivers_one_receipt_and_relays_to_bound_chat() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let requester = create_conversation(&stack, "requester").await;
    let target = create_conversation(&stack, "target").await;
    bind_channel_session(&stack, &requester).await;

    // Gateway order: registration BEFORE the keyed send.
    let registration = stack
        .conversation_svc
        .register_delivery_notify(&stack.owner, &target, "op-key-1", &requester)
        .await
        .unwrap();
    assert_eq!(registration, DeliveryNotifyRegistration::Registered);

    stack
        .conversation_svc
        .send_message_with_idempotency_key(
            &stack.owner,
            &target,
            "op-key-1",
            send_request("please do the delegated task"),
            &stack.runtime_registry,
        )
        .await
        .unwrap();

    // Target completes → observer takes the registration → receipt lands in
    // the requester transcript with the delivery-notify framing + target's
    // final text.
    wait_for("requester received the receipt message", || {
        let stack = &stack;
        let requester = requester.clone();
        async move {
            user_texts(stack, &requester)
                .await
                .iter()
                .any(|t| t.contains("任务回执") && t.contains("target finished the delegated task"))
        }
    })
    .await;

    // The bound IM chat sees the requester companion's reply via the relay.
    wait_for("bound chat received the relayed companion reply", || {
        let stack = &stack;
        async move {
            stack
                .recorder
                .texts()
                .iter()
                .any(|t| t.contains("target finished the delegated task"))
        }
    })
    .await;

    // Idempotency: replaying the SAME completion must not deliver twice —
    // the registration claim is single-winner.
    let operation_id = format!("public-turn:v1:{}:{}:op-key-1", stack.owner, target);
    stack
        .observer
        .on_turn_completed(&target, &operation_id, true, Some("replayed text"), None)
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let receipts = user_texts(&stack, &requester)
        .await
        .iter()
        .filter(|t| t.contains("任务回执"))
        .count();
    assert_eq!(receipts, 1, "duplicate completion must not deliver a second receipt");
}

#[tokio::test]
async fn delivery_notify_origin_turn_cannot_register_notify_back() {
    let db = init_database_memory().await.unwrap();
    let stack = build_stack(db.pool().clone()).await;

    let requester = create_conversation(&stack, "requester").await;
    let target = create_conversation(&stack, "target").await;

    // Put the requester into an ACTIVE turn whose durable request payload
    // carries origin=delivery-notify — exactly the state its receipt turn is
    // in while the companion processes the receipt and calls tools.
    let admission = stack
        .conv_repo
        .get_turn_admission_state(&stack.owner, &requester)
        .await
        .unwrap();
    let payload = serde_json::json!({
        "content": "receipt prompt",
        "files": [],
        "inject_skills": [],
        "hidden": false,
        "origin": "delivery-notify",
        "channel_platform": null,
    })
    .to_string();
    stack
        .conv_repo
        .claim_turn_delivery_receipt_and_admit_with_candidate(
            &stack.owner,
            &requester,
            "public-turn:v1:test:receipt-turn",
            &nomifun_common::MessageId::new().into_string(),
            &payload,
            admission.epoch,
            nomifun_common::now_ms(),
        )
        .await
        .unwrap();

    // The loop guard refuses the registration…
    let refused = stack
        .conversation_svc
        .register_delivery_notify(&stack.owner, &target, "op-key-loop", &requester)
        .await
        .unwrap();
    assert_eq!(
        refused,
        DeliveryNotifyRegistration::RefusedDeliveryNotifyOrigin
    );

    // …and nothing was persisted for that operation.
    let taken = stack
        .conversation_svc
        .take_pending_delivery_notify(&format!(
            "public-turn:v1:{}:{}:op-key-loop",
            stack.owner, target
        ))
        .await
        .unwrap();
    assert!(taken.is_none(), "a refused registration must not exist");

    // Control: a plain companion-origin active turn registers fine.
    let requester2 = create_conversation(&stack, "requester2").await;
    let admission = stack
        .conv_repo
        .get_turn_admission_state(&stack.owner, &requester2)
        .await
        .unwrap();
    let payload = serde_json::json!({
        "content": "ordinary prompt",
        "files": [],
        "inject_skills": [],
        "hidden": false,
        "origin": "companion",
        "channel_platform": null,
    })
    .to_string();
    stack
        .conv_repo
        .claim_turn_delivery_receipt_and_admit_with_candidate(
            &stack.owner,
            &requester2,
            "public-turn:v1:test:ordinary-turn",
            &nomifun_common::MessageId::new().into_string(),
            &payload,
            admission.epoch,
            nomifun_common::now_ms(),
        )
        .await
        .unwrap();
    assert_eq!(
        stack
            .conversation_svc
            .register_delivery_notify(&stack.owner, &target, "op-key-ok", &requester2)
            .await
            .unwrap(),
        DeliveryNotifyRegistration::Registered
    );
}
