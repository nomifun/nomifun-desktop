//! Concrete conversation/model access for the robot gateway.
//!
//! Lives here because only this crate holds a `ConversationService`, the agent
//! runtime registry, the companion registry, the provider catalog and the
//! installation owner id at once. `nomifun-robot` sees only its own traits, so
//! the dependency direction stays one-way.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use nomifun_api_types::{AgentErrorOwnership, SendMessageRequest};
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_ai_agent::protocol::events::{AgentStreamEvent, TurnStopReason};
use nomifun_conversation::ConversationService;
use nomifun_db::IClientPreferenceRepository;
use nomifun_robot::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
use nomifun_robot::effect_ledger::RobotEffectLedger;
use nomifun_robot::mcp_proxy::RobotMcpProxyServer;
use nomifun_robot::registry::RobotRegistry;
use nomifun_robot::services::{SpeechServices, TurnEvent};
use nomifun_robot::status::RobotStatusRegistry;
use nomifun_robot::tool_registry::RobotToolRegistry;
use nomifun_robot::vad::VadTuning;
use nomifun_robot::vision::RobotVisionObservationRegistry;
use nomifun_robot::wiring::{
    CompanionSlotReader, PreferenceReader, RobotSpeech, VisionCompletionExecutor,
    VisionCompletionRequest,
};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
#[cfg(test)]
use tokio::sync::broadcast;

const ROBOT_VISION_MAX_TOKENS: u32 = 512;

/// Production bridge from the robot's one-shot image request to the shared
/// Agent Chat resolver. Protocol, endpoint and auth come exclusively from the
/// selected model's persisted Chat capability.
struct AgentRobotVisionExecutor {
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    workspace: PathBuf,
}

#[async_trait::async_trait]
impl VisionCompletionExecutor for AgentRobotVisionExecutor {
    async fn complete(&self, request: VisionCompletionRequest) -> anyhow::Result<String> {
        let config = nomifun_ai_agent::resolve_provider_config(
            self.model_invoke.as_ref(),
            &request.provider_id,
            &request.model,
            &self.workspace,
        )
        .await
        .map_err(|error| anyhow::anyhow!("视觉模型配置不可用: {error}"))?;
        let prompt = if request.question.trim().is_empty() {
            "描述这张图片。"
        } else {
            request.question.trim()
        };
        let data = base64::engine::general_purpose::STANDARD.encode(request.jpeg);
        let message = nomifun_ai_agent::nomi_types::message::Message::new(
            nomifun_ai_agent::nomi_types::message::Role::User,
            vec![
                nomifun_ai_agent::nomi_types::message::ContentBlock::Image {
                    media_type: "image/jpeg".to_owned(),
                    data,
                },
                nomifun_ai_agent::nomi_types::message::ContentBlock::Text {
                    text: prompt.to_owned(),
                },
            ],
        );
        let answer = nomifun_ai_agent::one_shot_completion(
            &config,
            "你在为一台物理机器人看图。用一到两句中文口语描述你看到的内容，直接回答问题。",
            vec![message],
            ROBOT_VISION_MAX_TOKENS,
        )
        .await
        .map_err(|error| anyhow::anyhow!("视觉模型调用失败: {error}"))?;
        if answer.trim().is_empty() {
            anyhow::bail!("视觉模型没有返回内容");
        }
        Ok(answer)
    }
}

/// Everything the host holds for the robot gateway.
///
/// Split in two phases on purpose: everything here exists before the router,
/// because the device face and the OTA response must be reachable the moment the
/// listener comes up, while the gateway itself needs a `ConversationService`
/// that only exists during router assembly.
pub struct RobotServices {
    #[cfg(test)]
    backend: std::sync::OnceLock<Arc<AppRobotBackend>>,
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub tools: Arc<RobotToolRegistry>,
    pub effect_ledger: Arc<RobotEffectLedger>,
    pub vision_observations: Arc<RobotVisionObservationRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
    pub speech: Arc<dyn SpeechServices>,
    /// Live view of the LAN listener. `desktop.rs` projects its `WebUiStatus`
    /// into this; nothing else may write it.
    pub endpoint_tx: watch::Sender<LanEndpointSnapshot>,
    /// The loopback MCP front for device tools. `None` when it failed to bind —
    /// robot tools are then simply unavailable to the model.
    pub proxy: Option<Arc<RobotMcpProxyServer>>,
    /// Set once, during router assembly.
    gateway_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RobotServices {
    /// Build everything that does not need a `ConversationService`.
    ///
    /// A failure to load the registry is fatal for the domain, not for the app:
    /// the caller degrades to "no robot support" rather than refusing to boot.
    pub async fn build(
        data_dir: &std::path::Path,
        owner_user_id: &str,
        user_events: Arc<dyn nomifun_realtime::UserEventSink>,
        invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
        companions: Arc<nomifun_companion::CompanionService>,
        preference_repo: Arc<dyn IClientPreferenceRepository>,
    ) -> anyhow::Result<Self> {
        let registry = Arc::new(RobotRegistry::load(data_dir).await?);
        let status = Arc::new(RobotStatusRegistry::new(
            nomifun_robot::events::RobotEventEmitter::new(user_events),
            owner_user_id.to_owned(),
        ));
        let tools = Arc::new(RobotToolRegistry::default());
        let effect_ledger = Arc::new(RobotEffectLedger::load(data_dir).await?);
        let vision_observations = Arc::new(RobotVisionObservationRegistry::default());
        let proxy = match RobotMcpProxyServer::spawn(tools.clone()).await {
            Ok(server) => Some(Arc::new(server)),
            Err(error) => {
                tracing::error!(%error, "robot: MCP proxy failed to bind; device tools disabled");
                None
            }
        };
        let (endpoint_tx, endpoint_rx) = watch::channel(LanEndpointSnapshot::default());
        let advertiser: Arc<dyn EndpointAdvertiser> = Arc::new(LanAdvertiser::new(endpoint_rx));

        let slots = Arc::new(AppCompanionSlots {
            companions,
            model_invoke: invoke.clone(),
        });
        let speech: Arc<dyn SpeechServices> = Arc::new(RobotSpeech::new(
            invoke.clone(),
            slots,
            Arc::new(AppPreferences {
                repo: preference_repo,
            }),
            Arc::new(AgentRobotVisionExecutor {
                model_invoke: invoke,
                workspace: data_dir.to_path_buf(),
            }),
        ));

        Ok(Self {
            #[cfg(test)]
            backend: std::sync::OnceLock::new(),
            registry,
            status,
            tools,
            effect_ledger,
            vision_observations,
            advertiser,
            speech,
            endpoint_tx,
            proxy,
            gateway_task: Mutex::new(None),
        })
    }

    /// Record the gateway's accept loop so shutdown can stop it. Replacing an
    /// existing handle aborts the old loop: two accept loops on one registry
    /// would race for the same device.
    pub fn set_gateway_task(&self, task: tokio::task::JoinHandle<()>) {
        if let Some(previous) = self
            .gateway_task
            .lock()
            .expect("robot gateway task lock poisoned")
            .replace(task)
        {
            previous.abort();
        }
    }

    /// Stop the accept loop and the loopback MCP front. Sessions are owned by
    /// their own tasks and end when their sockets close with the listener.
    pub fn shutdown(&self) {
        if let Some(task) = self
            .gateway_task
            .lock()
            .expect("robot gateway task lock poisoned")
            .take()
        {
            task.abort();
        }
        if let Some(proxy) = &self.proxy {
            proxy.stop();
        }
    }
}

// ---------------------------------------------------------------------------
// Model-layer readers
// ---------------------------------------------------------------------------

/// Companion model slots, read live off the profile so a settings change applies
/// to the next utterance rather than the next boot.
struct AppCompanionSlots {
    companions: Arc<nomifun_companion::CompanionService>,
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
}

impl AppCompanionSlots {
    async fn profile(
        &self,
        companion_id: &str,
    ) -> Option<nomifun_companion::profile::CompanionProfileConfig> {
        self.companions
            .get_companion(companion_id)
            .await
            .inspect_err(|error| {
                tracing::warn!(companion_id, %error, "robot: companion profile unavailable");
            })
            .ok()
    }

    /// Whether a catalog row carries the vision-input trait.
    async fn model_sees_images(&self, provider_id: &str, model: &str) -> bool {
        self
            .model_invoke
            .resolve_task_config(
                &nomifun_model_invoke::ModelRef {
                    provider_id: provider_id.to_owned(),
                    model: model.to_owned(),
                },
                nomifun_api_types::ModelTask::Chat,
            )
            .await
            .is_ok_and(|resolved| {
                resolved
                    .traits
                    .contains(&nomifun_api_types::ModelTrait::VisionInput)
            })
    }
}

#[async_trait::async_trait]
impl CompanionSlotReader for AppCompanionSlots {
    async fn asr_slot(&self, companion_id: &str) -> Option<(String, String)> {
        let asr = self.profile(companion_id).await?.voice.asr?;
        Some((asr.provider_id, asr.model))
    }

    async fn tts_slot(&self, companion_id: &str) -> Option<(String, String, Option<String>)> {
        let tts = self.profile(companion_id).await?.voice.tts?;
        Some((tts.provider_id, tts.model, tts.voice))
    }

    async fn vision_slot(&self, companion_id: &str) -> Option<(String, String)> {
        let profile = self.profile(companion_id).await?;
        if let Some(vision) = profile.vision_model {
            return self
                .model_sees_images(&vision.provider_id, &vision.model)
                .await
                .then_some((vision.provider_id, vision.model));
        }
        // No dedicated slot: the main chat model may still be able to look, and
        // the catalog is the authority on that. Guessing from the model name is
        // how you end up sending a JPEG to a text-only endpoint.
        let chat = profile.model?;
        self.model_sees_images(&chat.provider_id, &chat.model)
            .await
            .then_some((chat.provider_id, chat.model))
    }
}

/// Install-wide client preferences.
struct AppPreferences {
    repo: Arc<dyn IClientPreferenceRepository>,
}

#[async_trait::async_trait]
impl PreferenceReader for AppPreferences {
    async fn get(&self, key: &str) -> Option<Value> {
        let rows = self.repo.get_by_keys(&[key]).await.ok()?;
        let row = rows.into_iter().find(|row| row.key == key)?;
        serde_json::from_str(&row.value).ok()
    }
}

// ---------------------------------------------------------------------------
// Conversation backend
// ---------------------------------------------------------------------------

/// The robot-body section appended to the companion persona.
///
/// This is the only place the model is told what a *spoken* reply must look
/// like, so it is where output discipline belongs.
///
/// **The model is required to emit plain spoken text, and nothing else.** There
/// is no marker channel in this text and no vocabulary to learn: no
/// square/full-width brackets, no stage directions or action annotations, no
/// emoji, no markdown, no parenthetical asides.
///
/// That is a deliberate replacement for a deleted design. The prompt used to ask
/// for a leading `[emotion:名]` marker out of a 21-name vocabulary so the gateway
/// could drive the OLED face; the model emitted `[winking]` — the bare name —
/// and every stripper keyed on the literal `"[emotion:"` matched nothing. The
/// marker was therefore printed in the desktop transcript AND read aloud by TTS
/// AND drove no face: broken and noisy at once. A syntax contract with an LLM is
/// not enforceable, so it is gone rather than re-syntaxed. A PROHIBITION is far
/// more enforceable than a syntax contract, which is exactly why this reads as
/// one.
///
/// A prohibition is still not a guarantee, so both readers of the model's text
/// strip stage directions as a backstop, syntax-agnostically
/// (`nomifun_common::stage_direction`): the desktop relay
/// (`stream_relay.rs`'s `spoken_output` turn policy, which owns the live stream, the
/// persisted `messages` row, search and the knowledge writeback) and the device
/// path (`nomifun-robot`'s `sanitize_for_speech` / `sanitize_for_display`, which
/// own TTS and the OLED). Those two are NOT duplicates and neither may be
/// deleted as one: they serve different consumers off independent `broadcast`
/// clones, and the crates may not depend on each other.
///
/// Facial expression still exists, but it is STATE-driven: `session.rs` sends
/// `ServerMessage::Llm { emotion: "sad" }` when the voice link is broken or a
/// turn failed. That is a fact the gateway owns, not a syntax it hopes for.
///
/// One prompt rule earns its wording carefully: **the text is read aloud**.
/// Without saying so, models write for a display — emoji, markdown, parenthetical
/// asides — none of which a TTS engine can voice, and some of which make it fail
/// outright.
fn robot_body_prompt() -> &'static str {
    "你现在通过用户绑定的物理机器人和用户说话。摄像头、屏幕和动作以本轮实际提供的设备工具为准，不假设每台机器人都有相同硬件。\n\
     - 你的回复会被语音合成念出声。所以只写能读出来的自然口语。\n\
     - 回复必须简短口语化：每句不超过 40 字，整体不超过 3 句，除非用户明确要求详细内容。\n\
     - 只输出要说出来的那句话本身。不要写任何方括号或【】里的标注（例如 [winking]、[开心]、【笑】），不要写动作描写或舞台提示，不要写旁白和括号里的补充说明。\n\
     - 不要输出 emoji、颜文字、markdown 记号（星号、井号、反引号），以及任何念不出声的符号。需要停顿就用逗号和句号。\n\
     - 需要转头、看某个方向或调音量时，用 robot_ 开头的工具。"
}

/// Production device ingress shares the Companion's authoritative Conversation.
#[derive(Clone)]
pub struct AppRobotBackend {
    pub conversations: ConversationService,
    pub runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    pub companions: Arc<nomifun_companion::CompanionService>,
    pub owner_user_id: Arc<str>,
    pub registry: Arc<RobotRegistry>,
    pub vision_observations: Arc<RobotVisionObservationRegistry>,
    pub mcp_proxy: Option<Arc<RobotMcpProxyServer>>,
    pending: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    queues: Arc<Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

#[derive(Clone)]
struct DeviceTurnAuthority {
    from_desktop: bool,
    registry: Arc<RobotRegistry>,
    request: nomifun_robot::services::RobotTurnRequest,
    revision: u64,
    capabilities: std::collections::BTreeSet<String>,
    cancelled: tokio_util::sync::CancellationToken,
    conversations: ConversationService,
    owner_user_id: Arc<str>,
    agent_revision: Option<(String, i64)>,
}

#[async_trait::async_trait]
impl nomifun_robot::vision::RobotVisionRecorder for DeviceTurnAuthority {
    fn robot_id(&self) -> &str { &self.request.robot_id }
    fn source(&self) -> nomifun_robot::vision::RobotVisionSource {
        nomifun_robot::vision::RobotVisionSource {
            companion_id: self.request.companion_id.clone(), conversation_id: self.request.conversation_id.clone(),
            connection_id: self.request.connection_id.clone(), request_id: self.request.request_id.clone(),
        }
    }
    async fn authorize(&self) -> Result<(), String> {
        nomifun_robot::mcp_proxy::RobotToolAuthority::validate(self, "robot.vision").await?;
        let mut context = device_turn_context(&self.request);
        context.from_desktop = self.from_desktop;
        context.agent_revision = self.agent_revision.clone();
        if !self.conversations.companion_device_turn_is_active(&self.owner_user_id, &self.request.conversation_id, &context) {
            return Err("camera upload does not belong to the active Companion turn".to_owned());
        }
        Ok(())
    }
    async fn record(&self, question: &str, answer: &str, jpeg: &[u8]) -> Result<(), String> {
        self.authorize().await?;
        let mut context = device_turn_context(&self.request);
        context.from_desktop = self.from_desktop;
        context.agent_revision = self.agent_revision.clone();
        self.conversations.record_companion_device_observation(&self.owner_user_id,
            &self.request.conversation_id, &context, question, answer,
            &base64::engine::general_purpose::STANDARD.encode(jpeg),
        ).await.map_err(|error| error.to_string())
    }
}

impl DeviceTurnAuthority {
    async fn validate_connection(&self) -> Result<nomifun_robot::registry::RobotRecord, String> {
        if self.cancelled.is_cancelled() { return Err("device utterance was cancelled".to_owned()); }
        let record = self.registry.get(&self.request.robot_id).await.ok_or("robot was removed")?;
        if record.companion_id.as_deref() != Some(self.request.companion_id.as_str())
            || record.authorization_revision != self.revision
            || !self.registry.connection_matches(&self.request.robot_id, &self.request.connection_id).await
        { return Err("robot binding, permissions or connection changed".to_owned()); }
        Ok(record)
    }
}

#[async_trait::async_trait]
impl nomifun_conversation::service::BackgroundTurnPreSendHook for DeviceTurnAuthority {
    async fn prepare(&self) -> Result<(), nomifun_common::AppError> {
        self.validate_connection().await.map(|_| ()).map_err(nomifun_common::AppError::Forbidden)
    }
}

#[async_trait::async_trait]
impl nomifun_robot::mcp_proxy::RobotToolAuthority for DeviceTurnAuthority {
    fn robot_id(&self) -> &str { &self.request.robot_id }
    fn connection_id(&self) -> &str { &self.request.connection_id }
    async fn validate(&self, capability: &str) -> Result<(), String> {
        let record = self.validate_connection().await?;
        if !self.capabilities.contains(capability) || !record.permissions.allows(capability) {
            return Err(format!("{capability} is outside the Companion and device capability ceiling"));
        }
        Ok(())
    }
    async fn validate_tool(&self, device_name: &str) -> Result<(), String> {
        self.validate(nomifun_robot::tool_registry::tool_capability(device_name).capability_id()).await?;
        if device_name.starts_with("self.audio") || device_name.starts_with("audio") {
            self.validate("robot.audio").await?;
        }
        let record = self.validate_connection().await?;
        if !record.permissions.allows_tool(device_name) { return Err("continuous observation is not allowed".to_owned()); }
        let mut context = device_turn_context(&self.request);
        context.from_desktop = self.from_desktop;
        if !self.conversations.companion_device_turn_is_active(&self.owner_user_id, &self.request.conversation_id, &context) {
            return Err("device tool no longer belongs to an active turn".to_owned());
        }
        Ok(())
    }
}

fn device_turn_context(request: &nomifun_robot::services::RobotTurnRequest)
    -> nomifun_conversation::companion_interaction::CompanionDeviceTurn
{
    nomifun_conversation::companion_interaction::CompanionDeviceTurn {
        from_desktop: false, resources: None,
        companion_id: request.companion_id.clone(), robot_id: request.robot_id.clone(),
        connection_id: request.connection_id.clone(), request_id: request.request_id.clone(),
        agent_revision: None,
        model: None, fallback_model: None, mcp_servers: vec![], system_prompt: String::new(),
    }
}

impl AppRobotBackend {
    async fn run_device_turn(
        &self, request: &nomifun_robot::services::RobotTurnRequest,
        cancelled: tokio_util::sync::CancellationToken, tx: mpsc::Sender<TurnEvent>,
    ) -> anyhow::Result<()> {
        let queue = {
            let mut queues = self.queues.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            queues.retain(|_, queue| queue.strong_count() > 0);
            if let Some(queue) = queues.get(&request.conversation_id).and_then(std::sync::Weak::upgrade) { queue }
            else {
                let queue = Arc::new(tokio::sync::Mutex::new(()));
                queues.insert(request.conversation_id.clone(), Arc::downgrade(&queue));
                queue
            }
        };
        let _queue_guard = tokio::select! {
            guard = queue.lock_owned() => guard,
            _ = cancelled.cancelled() => return Ok(()),
        };
        let queued_at = tokio::time::Instant::now();
        loop {
            if cancelled.is_cancelled() { return Ok(()); }
            if queued_at.elapsed() > std::time::Duration::from_secs(300) {
                anyhow::bail!("伙伴仍在处理其他消息，本次排队已超时，请重试");
            }
            if !self.registry.connection_matches(&request.robot_id, &request.connection_id).await {
                anyhow::bail!("robot connection is no longer active");
            }
            if self.conversations.runtime_summary_for(&request.conversation_id).await.is_processing {
                tokio::select! {
                    _ = cancelled.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
                }
                continue;
            }
            let profile = self.companions.get_companion(&request.companion_id).await?;
            let record = self.registry.get(&request.robot_id).await.ok_or_else(|| anyhow::anyhow!("robot was removed"))?;
            let conversation = match self.conversations.refresh_product_agent_for_existing(
                &self.owner_user_id, &request.conversation_id).await {
                Ok(conversation) => conversation,
                Err(nomifun_common::AppError::Conflict(_)) if self.conversations
                    .runtime_summary_for(&request.conversation_id).await.is_processing => continue,
                Err(error) => return Err(error.into()),
            };
            let capabilities = conversation.agent_snapshot.as_ref()
                .map(|snapshot| snapshot.enabled_capabilities.iter().cloned().collect())
                .unwrap_or_default();
            let authority = Arc::new(DeviceTurnAuthority {
                from_desktop: false,
                registry: self.registry.clone(), request: request.clone(),
                revision: record.authorization_revision, capabilities, cancelled: cancelled.clone(),
                conversations: self.conversations.clone(), owner_user_id: self.owner_user_id.clone(),
                agent_revision: conversation.preset_id.clone().zip(conversation.preset_revision),
            });
            authority.validate_connection().await.map_err(anyhow::Error::msg)?;
            let mut context = device_turn_context(request);
            context.agent_revision = conversation.preset_id.clone().zip(conversation.preset_revision);
            context.model = profile.model;
            context.fallback_model = profile.fallback_model;
            context.system_prompt = nomifun_ai_agent::CompanionPromptProvider::build_system_prompt(
                &*self.companions, Some(&request.companion_id), None).await.unwrap_or_default();
            context.system_prompt.push_str("\n\n");
            context.system_prompt.push_str(robot_body_prompt());
            let tool_lease = if authority.capabilities.contains("robot.link") {
                match &self.mcp_proxy { Some(proxy) => Some(proxy.issue(authority.clone()).await?), None => None }
            } else { None };
            if let Some(lease) = tool_lease.as_ref() { context.mcp_servers.push(lease.registration()); }
            let resources = Arc::new(DeviceTurnResources { _tool_lease: tool_lease, vision_lease: Mutex::new(None) });
            context.resources = Some(resources.clone());
            let prepare_hook = Arc::new(DeviceTurnPreparation { authority, resources, observations: self.vision_observations.clone() });
            let message = SendMessageRequest {
                content: request.text.clone(), files: vec![], inject_skills: vec![],
                hidden: false, origin: None, channel_platform: Some("robot".to_owned()),
            };
            let observed = tokio::select! {
                result = self.conversations.send_companion_device_message(
                    &self.owner_user_id, &request.conversation_id, message, context.clone(), prepare_hook, &self.runtime_registry,
                ) => result,
                _ = cancelled.cancelled() => {
                    self.conversations.cancel_companion_device_message(&self.owner_user_id,
                        &request.conversation_id, &context, &self.runtime_registry).await?;
                    return Ok(());
                }
            };
            let mut stream = match observed {
                Ok(observed) => observed.events,
                Err(error) => {
                let receipt = self.conversations.companion_device_delivery_result(
                    &self.owner_user_id, &request.conversation_id, &context).await?;
                if receipt.is_none() {
                    if matches!(error, nomifun_common::AppError::Conflict(_))
                        && self.conversations.runtime_summary_for(&request.conversation_id).await.is_processing {
                        continue;
                    }
                    return Err(error.into());
                }
                None
                }
            };
            let mut reducer = SpokenReplyReducer::default();
            let mut spoken_answer: Option<String> = None;
            // The durable result includes automatic continuations and final
            // persistence. Never subscribe to whichever cached runtime happens
            // to be current, and never speak an intermediate tool-pass Finish.
            loop {
                if let Some(mut receipt) = self.conversations.companion_device_delivery_result(
                    &self.owner_user_id, &request.conversation_id, &context).await?
                    && receipt.completed {
                    // A fast producer can commit its receipt while this
                    // observer still has buffered tool/text events. Drain
                    // those before selecting the final spoken segment.
                    if let Some(receiver) = stream.as_mut() {
                        'drain: while let Ok(event) = receiver.try_recv() {
                            for reduced in reducer.push(event) {
                                match reduced {
                                    TurnEvent::Text(text) => spoken_answer = Some(text),
                                    TurnEvent::Done => break 'drain,
                                    TurnEvent::Failed { .. } => spoken_answer = None,
                                }
                            }
                        }
                    }
                    if receipt.result_ok == Some(true) && let Some(answer) = spoken_answer.take() {
                        receipt.result_text = Some(answer);
                    }
                    for event in completed_robot_delivery_events(&receipt) {
                        if tx.send(event).await.is_err() { break; }
                    }
                    return Ok(());
                }
                tokio::select! {
                    event = async {
                        match stream.as_mut() {
                            Some(receiver) => receiver.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match event {
                            Ok(event) => {
                                for reduced in reducer.push(event) {
                                    match reduced {
                                        TurnEvent::Text(text) => { spoken_answer = Some(text); }
                                        TurnEvent::Done => { stream = None; }
                                        TurnEvent::Failed { .. } => { spoken_answer = None; }
                                    }
                                }
                            }
                            Err(_) => { stream = None; spoken_answer = None; }
                        }
                    }
                    _ = cancelled.cancelled() => {
                        self.conversations.cancel_companion_device_message(&self.owner_user_id,
                            &request.conversation_id, &context, &self.runtime_registry).await?;
                        return Ok(());
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(30)) => {},
                }
            }
        }
    }
}

struct DeviceTurnResources {
    _tool_lease: Option<Arc<nomifun_robot::mcp_proxy::RobotMcpLease>>,
    vision_lease: Mutex<Option<nomifun_robot::vision::RobotVisionTurnLease>>,
}

struct DeviceTurnPreparation {
    authority: Arc<DeviceTurnAuthority>,
    resources: Arc<DeviceTurnResources>,
    observations: Arc<RobotVisionObservationRegistry>,
}

#[async_trait::async_trait]
impl nomifun_conversation::service::BackgroundTurnPreSendHook for DeviceTurnPreparation {
    async fn prepare(&self) -> Result<(), nomifun_common::AppError> {
        self.authority.validate_connection().await.map_err(nomifun_common::AppError::Forbidden)?;
        *self.resources.vision_lease.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(self.observations.register_turn(self.authority.clone()));
        Ok(())
    }
}

#[async_trait::async_trait]
impl nomifun_robot::services::RobotPlaybackSource for AppRobotBackend {
    async fn response_text(&self, robot_id: &str, conversation_id: &str) -> Result<String, String> {
        let device = self.registry.get(robot_id).await.ok_or("device not found")?;
        let conversation = self.conversations.get(&self.owner_user_id, conversation_id).await.map_err(|e| e.to_string())?;
        if device.companion_id.is_none() || conversation.extra.get("companion_id").and_then(Value::as_str) != device.companion_id.as_deref() {
            return Err("conversation belongs to another Companion".to_owned());
        }
        if self.conversations.runtime_summary_for(conversation_id).await.is_processing {
            return Err("wait for the current reply to finish".to_owned());
        }
        let messages = self.conversations.list_messages(&self.owner_user_id, conversation_id,
            serde_json::from_value(serde_json::json!({"page":1,"page_size":50,"order":"desc"})).map_err(|e| e.to_string())?,
        ).await.map_err(|e| e.to_string())?;
        messages.items.iter().filter(|message| !message.hidden
            && message.position == Some(nomifun_common::MessagePosition::Left)
            && message.r#type == nomifun_common::MessageType::Text)
            .filter_map(|message| message.content.get("content").and_then(Value::as_str))
            .find(|text| !text.trim().is_empty()).map(str::to_owned)
            .ok_or_else(|| "no reply is available to play".to_owned())
    }
}

#[async_trait::async_trait]
impl nomifun_conversation::companion_interaction::CompanionDesktopTurnProvider for AppRobotBackend {
    async fn prepare(&self, owner_id: &str, conversation_id: &str, request_id: &str)
        -> Result<Option<nomifun_conversation::companion_interaction::PreparedDesktopDeviceTurn>, nomifun_common::AppError>
    {
        use nomifun_common::AppError;
        if owner_id != self.owner_user_id.as_ref() { return Ok(None); }
        let conversation = self.conversations.get(owner_id, conversation_id).await?;
        let Some(companion_id) = conversation.extra.get("companion_id").and_then(Value::as_str) else { return Ok(None); };
        if conversation.extra.get("companion_session").and_then(Value::as_bool) != Some(true) { return Ok(None); }
        let profile = self.companions.get_companion(companion_id).await?;
        let devices = self.registry.list().await.into_iter()
            .filter(|device| device.companion_id.as_deref() == Some(companion_id)).collect::<Vec<_>>();
        let selected = match profile.control_robot_id.as_deref() {
            Some(id) => devices.iter().find(|device| device.robot_id == id),
            None if devices.len() == 1 => devices.first(),
            None => None,
        };
        let Some(device) = selected else { return Ok(None); };
        let Some(connection_id) = self.registry.current_connection(&device.robot_id).await else { return Ok(None); };
        let capabilities: std::collections::BTreeSet<String> = conversation.agent_snapshot.as_ref()
            .map(|snapshot| snapshot.enabled_capabilities.iter().cloned().collect()).unwrap_or_default();
        if !capabilities.contains("robot.link") { return Ok(None); }
        let Some(proxy) = self.mcp_proxy.as_ref() else { return Ok(None); };
        let request = nomifun_robot::services::RobotTurnRequest {
            robot_id: device.robot_id.clone(), companion_id: companion_id.to_owned(),
            conversation_id: conversation_id.to_owned(), connection_id, request_id: request_id.to_owned(), text: String::new(),
        };
        let authority = Arc::new(DeviceTurnAuthority {
            from_desktop: true, registry: self.registry.clone(), request: request.clone(),
            revision: device.authorization_revision, capabilities,
            cancelled: tokio_util::sync::CancellationToken::new(), conversations: self.conversations.clone(),
            owner_user_id: self.owner_user_id.clone(), agent_revision: conversation.preset_id.zip(conversation.preset_revision),
        });
        let tool_lease = proxy.issue(authority.clone()).await.map_err(|error| AppError::Forbidden(error.to_string()))?;
        let mut context = device_turn_context(&request);
        context.from_desktop = true;
        context.agent_revision = authority.agent_revision.clone();
        context.model = profile.model;
        context.fallback_model = profile.fallback_model;
        context.system_prompt = nomifun_ai_agent::CompanionPromptProvider::build_system_prompt(
            &*self.companions, Some(companion_id), None).await.unwrap_or_default();
        context.system_prompt.push_str("\n当前输入来自桌面，正常使用完整文字、Markdown 等形式回答；回复不会自动播报。已连接的物理机器人可通过本轮提供的 robot_ 工具操作，只操作用户当前选定的设备。\n");
        context.mcp_servers.push(tool_lease.registration());
        let resources = Arc::new(DeviceTurnResources { _tool_lease: Some(tool_lease), vision_lease: Mutex::new(None) });
        context.resources = Some(resources.clone());
        let hook = Arc::new(DeviceTurnPreparation { authority, resources, observations: self.vision_observations.clone() });
        Ok(Some(nomifun_conversation::companion_interaction::PreparedDesktopDeviceTurn { context, pre_send_hook: hook }))
    }
}

#[async_trait::async_trait]
impl nomifun_robot::wiring::RobotConversationBackend for AppRobotBackend {
    async fn ensure_companion_session(&self, companion_id: &str) -> anyhow::Result<String> {
        Ok(self.companions.create_companion_thread(companion_id, None).await?.conversation_id)
    }

    async fn dispatch(&self, request: nomifun_robot::services::RobotTurnRequest) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        let key = device_turn_context(&request).idempotency_key();
        let cancelled = tokio_util::sync::CancellationToken::new();
        {
            let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.len() >= 64 { anyhow::bail!("device utterance queue is full"); }
            if pending.contains_key(&key) { anyhow::bail!("this device utterance is already queued"); }
            pending.insert(key.clone(), cancelled.clone());
        }
        let (tx, rx) = mpsc::channel(64);
        let backend = self.clone();
        tokio::spawn(async move {
            if let Err(error) = backend.run_device_turn(&request, cancelled, tx.clone()).await {
                let _ = tx.send(robot_stream_failure(error.to_string())).await;
            }
            backend.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&key);
        });
        Ok(rx)
    }

    async fn cancel(&self, request: &nomifun_robot::services::RobotTurnRequest) -> anyhow::Result<()> {
        let key = device_turn_context(request).idempotency_key();
        if let Some(token) = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            token.cancel();
        }
        Ok(())
    }

    async fn vad_tuning(&self, companion_id: &str) -> VadTuning {
        match self.companions.get_companion(companion_id).await {
            Ok(profile) => VadTuning::from_profile(
                &profile.voice.vad.engine,
                profile.voice.vad.effective_sensitivity(),
                profile.voice.vad.effective_min_silence_ms(),
            ),
            Err(_) => VadTuning::default(),
        }
    }

    async fn vad_engine(&self, companion_id: &str) -> String {
        match self.companions.get_companion(companion_id).await {
            Ok(profile) => profile.voice.vad.engine,
            // A profile we cannot read is not a reason to pick a different
            // endpointer than the default one every new profile gets.
            Err(_) => nomifun_robot::vad::DEFAULT_VAD_ENGINE.to_owned(),
        }
    }

}


fn completed_robot_delivery_events(
    delivery: &nomifun_conversation::IdempotentMessageDelivery,
) -> Vec<TurnEvent> {
    match delivery.result_ok {
        Some(true) => {
            let mut events = Vec::with_capacity(2);
            if let Some(text) = delivery
                .result_text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                events.push(TurnEvent::Text(text.to_owned()));
            }
            events.push(TurnEvent::Done);
            events
        }
        Some(false) => vec![robot_stream_failure(
            delivery
                .result_error
                .as_deref()
                .unwrap_or("The admitted robot turn failed without an error message"),
        )],
        None => vec![robot_stream_failure(
            "The completed robot turn had no authoritative result",
        )],
    }
}

fn robot_stream_failure(message: impl Into<String>) -> TurnEvent {
    TurnEvent::Failed {
        message: message.into(),
        // An absent/lagged stream cannot prove provider ownership and must not
        // automatically replay a possibly side-effecting turn on a fallback.
        provider_fault: false,
    }
}

#[cfg(test)]
async fn relay_optional_robot_turn_stream(
    stream: Option<broadcast::Receiver<AgentStreamEvent>>,
    tx: mpsc::Sender<TurnEvent>,
) {
    let Some(stream) = stream else {
        let _ = tx
            .send(robot_stream_failure(
                "Robot could not attach to the admitted agent turn",
            ))
            .await;
        return;
    };
    relay_robot_turn_stream(stream, tx).await;
}

#[cfg(test)]
async fn relay_robot_turn_stream(
    mut stream: broadcast::Receiver<AgentStreamEvent>,
    tx: mpsc::Sender<TurnEvent>,
) {
    let mut reducer = SpokenReplyReducer::default();
    'stream: loop {
        match stream.recv().await {
            Ok(event) => {
                for reduced in reducer.push(event) {
                    let terminal = matches!(reduced, TurnEvent::Done | TurnEvent::Failed { .. });
                    if tx.send(reduced).await.is_err() || terminal {
                        break 'stream;
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                let _ = tx
                    .send(robot_stream_failure(format!(
                        "Robot missed {skipped} agent stream event(s)"
                    )))
                    .await;
                break;
            }
            Err(broadcast::error::RecvError::Closed) => {
                let _ = tx
                    .send(robot_stream_failure(
                        "Agent stream closed without a terminal event",
                    ))
                    .await;
                break;
            }
        }
    }
}

/// Reduce the agent's rich event stream to the final spoken reply.
///
/// Nomi can emit visible narration before each tool call ("let me check"), as
/// well as typed `Thinking` events. Both are execution progress, not the answer
/// the robot should speak. A tool/thinking boundary invalidates text collected
/// before it; only text produced after the last such boundary is released when
/// the whole turn finishes.
#[derive(Default)]
struct SpokenReplyReducer {
    candidate: String,
    output_checkpoint: Option<usize>,
}

impl SpokenReplyReducer {
    fn push(&mut self, event: AgentStreamEvent) -> Vec<TurnEvent> {
        match event {
            AgentStreamEvent::Start(_) => {
                self.output_checkpoint = Some(self.candidate.len());
                Vec::new()
            }
            AgentStreamEvent::OutputDiscarded(data) => {
                let Some(checkpoint) = self.output_checkpoint else {
                    self.candidate.clear();
                    return vec![TurnEvent::Failed {
                        message: format!(
                            "Attempt {} discarded output without a robot stream checkpoint",
                            data.restart_attempt
                        ),
                        provider_fault: false,
                    }];
                };
                if checkpoint > self.candidate.len()
                    || !self.candidate.is_char_boundary(checkpoint)
                {
                    self.candidate.clear();
                    self.output_checkpoint = None;
                    return vec![TurnEvent::Failed {
                        message: "Discarded output checkpoint no longer matched the robot reply"
                            .to_owned(),
                        provider_fault: false,
                    }];
                }
                self.candidate.truncate(checkpoint);
                self.output_checkpoint = Some(self.candidate.len());
                Vec::new()
            }
            AgentStreamEvent::Text(data) => {
                self.candidate.push_str(&data.content);
                Vec::new()
            }
            AgentStreamEvent::Thinking(data) => {
                // A completion marker only closes the existing thinking card;
                // it is not the start of a new model pass.
                if data.status.as_deref() != Some("done") {
                    self.candidate.clear();
                    if self.output_checkpoint.is_some() {
                        self.output_checkpoint = Some(0);
                    }
                }
                Vec::new()
            }
            AgentStreamEvent::Plan(_)
            | AgentStreamEvent::ToolCall(_)
            | AgentStreamEvent::ToolGroup(_) => {
                self.candidate.clear();
                if self.output_checkpoint.is_some() {
                    self.output_checkpoint = Some(0);
                }
                Vec::new()
            }
            AgentStreamEvent::Finish(data)
                if matches!(data.stop_reason, None | Some(TurnStopReason::EndTurn)) =>
            {
                let answer = std::mem::take(&mut self.candidate);
                self.output_checkpoint = None;
                let mut reduced = Vec::with_capacity(2);
                if !answer.trim().is_empty() {
                    reduced.push(TurnEvent::Text(answer));
                }
                reduced.push(TurnEvent::Done);
                reduced
            }
            AgentStreamEvent::Finish(data) => {
                self.candidate.clear();
                self.output_checkpoint = None;
                let reason = match data.stop_reason {
                    Some(TurnStopReason::MaxTokens) => "maximum output tokens reached",
                    Some(TurnStopReason::MaxTurnRequests) => "maximum tool requests reached",
                    Some(TurnStopReason::Refusal) => "the model refused the request",
                    Some(TurnStopReason::Cancelled) => "the turn was cancelled",
                    Some(TurnStopReason::EndTurn) | None => {
                        unreachable!("normal finish was handled above")
                    }
                };
                vec![TurnEvent::Failed {
                    message: format!(
                        "The turn ended before its requested output was completed: {reason}"
                    ),
                    provider_fault: false,
                }]
            }
            AgentStreamEvent::Error(data) => {
                self.candidate.clear();
                self.output_checkpoint = None;
                // `provider_fault` decides whether a fallback-model retry makes
                // sense. A replay is permitted only when the platform proved
                // retryability and classified the failure as provider-owned or
                // explicitly unknown-upstream. Non-retryable provider-quality
                // failures must not duplicate a possibly side-effecting turn.
                let provider_fault = matches!(
                    data.ownership,
                    Some(AgentErrorOwnership::UserLlmProvider)
                        | Some(AgentErrorOwnership::UnknownUpstream)
                ) && data.retryable == Some(true);
                vec![TurnEvent::Failed {
                    message: data.message,
                    provider_fault,
                }]
            }
            // Metrics, status and UI-only metadata do not invalidate an answer
            // that has already been produced immediately before Finish.
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP faces + gateway start
// ---------------------------------------------------------------------------

/// The two routers the host mounts, kept apart because they belong in different
/// middleware groups.
pub struct RobotFaces {
    /// `/robot/*` — devices. Bearer token, never a cookie, so it must NOT ride
    /// the cookie-CSRF group.
    pub device: axum::Router,
    /// `/api/robots*` — the desktop UI. Owner-gated by the caller.
    pub admin: axum::Router,
}

/// Build both HTTP faces and start the gateway's accept loop.
///
/// Called during router assembly rather than in `AppServices`, because that is
/// where the `ConversationService` the sessions dispatch through comes into
/// existence.
pub fn mount(
    robot: &Arc<RobotServices>,
    conversations: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    companions: Arc<nomifun_companion::CompanionService>,
    owner_user_id: Arc<str>,
    _data_dir: PathBuf,
) -> RobotFaces {
    let backend = Arc::new(AppRobotBackend {
        conversations,
        runtime_registry,
        companions,
        owner_user_id,
        registry: robot.registry.clone(),
        vision_observations: robot.vision_observations.clone(),
        pending: Arc::new(Mutex::new(HashMap::new())),
        queues: Arc::new(Mutex::new(HashMap::new())),
        mcp_proxy: robot.proxy.clone(),
    });
    backend.conversations.with_companion_desktop_provider(backend.clone());
    #[cfg(test)]
    let _ = robot.backend.set(backend.clone());
    let dispatcher = Arc::new(nomifun_robot::wiring::RobotDispatcher::new(backend.clone()));
    let (source, acceptor) = nomifun_robot::lan_source::LanWsSource::new();

    let gateway = Arc::new(nomifun_robot::RobotGateway::new(
        nomifun_robot::session::SessionDeps {
            registry: robot.registry.clone(),
            status: robot.status.clone(),
            speech: robot.speech.clone(),
            dispatcher,
            tools: robot.tools.clone(),
        },
    ));
    robot.set_gateway_task(tokio::spawn(gateway.serve(vec![source])));

    RobotFaces {
        device: nomifun_robot::routes::device_router(nomifun_robot::routes::RobotDeviceState {
            registry: robot.registry.clone(),
            advertiser: robot.advertiser.clone(),
            acceptor,
            speech: robot.speech.clone(),
            vision_observations: robot.vision_observations.clone(),
        }),
        admin: nomifun_robot::routes::admin_router(nomifun_robot::routes::RobotAdminState {
            playback_source: Some(backend),
            tools: robot.tools.clone(),
            registry: robot.registry.clone(),
            status: robot.status.clone(),
            advertiser: robot.advertiser.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    fn running_tool_call() -> AgentStreamEvent {
        use nomifun_ai_agent::protocol::events::tool_call::{
            ToolCallEventData, ToolCallStatus,
        };

        AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tool-1".to_owned(),
            name: "Browser".to_owned(),
            args: serde_json::json!({"action": "observe"}),
            status: ToolCallStatus::Running,
            input: None,
            output: None,
            description: None,
            retry: None,
            artifacts: Vec::new(),
        })
    }




    /// The prompt must PROHIBIT, not specify. It once specified a marker syntax
    /// (`[emotion:名]` plus a 21-name vocabulary) and the model emitted
    /// `[winking]` instead — a syntax contract with an LLM is not enforceable, so
    /// the contract is deleted and only the prohibition remains. This test is the
    /// guard against re-introducing one: no marker syntax, no vocabulary, and the
    /// spoken-aloud rule and the bracket ban both intact.
    #[test]
    fn the_prompt_bans_brackets_and_offers_no_marker_syntax() {
        let prompt = robot_body_prompt();

        assert!(
            !prompt.contains("[emotion:") && !prompt.contains("emotion:名"),
            "the deleted marker syntax must not come back: {prompt}"
        );
        for name in ["neutral", "laughing", "embarrassed", "kissy", "confused"] {
            assert!(
                !prompt.contains(name),
                "{name} is part of a vocabulary the model was told to emit; there is no vocabulary now"
            );
        }

        assert!(
            prompt.contains("方括号") && prompt.contains("【】"),
            "both bracket shapes must be forbidden by name"
        );
        assert!(
            prompt.contains("[winking]"),
            "the ban names the exact form the model actually emitted, which is what makes it land"
        );
        assert!(
            prompt.contains("动作描写") || prompt.contains("舞台提示"),
            "stage directions and action annotations must be forbidden"
        );
        assert!(
            prompt.contains("旁白") || prompt.contains("括号里的补充说明"),
            "parenthetical asides must be forbidden"
        );
        assert!(prompt.contains("emoji"), "nothing tells the model to skip emoji");
        assert!(prompt.contains("markdown"), "markdown must be forbidden");
        assert!(
            prompt.contains("念出声") || prompt.contains("念出来"),
            "the model is no longer told the text is spoken aloud"
        );
        // The short-reply rules and the tool guidance are the parts that were
        // working and are deliberately kept.
        assert!(prompt.contains("不超过 40 字") && prompt.contains("不超过 3 句"));
        assert!(prompt.contains("robot_"), "the tool guidance is still needed");
    }


    #[test]
    fn only_explicitly_retryable_upstream_faults_are_worth_a_fallback_retry() {
        let error = |ownership, retryable| {
            AgentStreamEvent::Error(nomifun_api_types::AgentStreamErrorData {
                message: "boom".to_owned(),
                code: None,
                ownership,
                detail: None,
                workspace_path: None,
                retryable,
                feedback_recommended: None,
                resolution: None,
            })
        };
        assert_eq!(
            SpokenReplyReducer::default()
                .push(error(
                    Some(AgentErrorOwnership::UserLlmProvider),
                    Some(true),
                )),
            vec![TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true
            }]
        );
        for (ownership, retryable) in [
            (Some(AgentErrorOwnership::UserLlmProvider), Some(false)),
            (Some(AgentErrorOwnership::UserLlmProvider), None),
            (Some(AgentErrorOwnership::Nomifun), Some(true)),
            (Some(AgentErrorOwnership::UserAgent), Some(true)),
            (None, Some(true)),
        ] {
            assert_eq!(
                SpokenReplyReducer::default().push(error(ownership, retryable)),
                vec![TurnEvent::Failed {
                    message: "boom".to_owned(),
                    provider_fault: false
                }],
                "{ownership:?} with retryable={retryable:?} must not replay on the fallback model"
            );
        }
        assert_eq!(
            SpokenReplyReducer::default().push(error(
                Some(AgentErrorOwnership::UnknownUpstream),
                Some(true),
            )),
            vec![TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true,
            }],
            "an explicitly retryable unknown-upstream failure keeps its existing fallback path"
        );
    }

    #[test]
    fn non_success_finish_never_releases_a_truncated_spoken_reply() {
        use nomifun_ai_agent::protocol::events::{FinishEventData, TextEventData};

        for stop_reason in [
            TurnStopReason::MaxTokens,
            TurnStopReason::MaxTurnRequests,
            TurnStopReason::Refusal,
            TurnStopReason::Cancelled,
        ] {
            let mut reducer = SpokenReplyReducer::default();
            assert!(
                reducer
                    .push(AgentStreamEvent::Text(TextEventData {
                        content: "unfinished draft".to_owned(),
                    }))
                    .is_empty()
            );

            let reduced = reducer.push(AgentStreamEvent::Finish(FinishEventData {
                session_id: None,
                stop_reason: Some(stop_reason),
            }));
            assert!(
                matches!(
                    reduced.as_slice(),
                    [TurnEvent::Failed {
                        provider_fault: false,
                        ..
                    }]
                ),
                "{stop_reason:?} must not be spoken or reported as Done: {reduced:?}"
            );
        }
    }

    #[tokio::test]
    async fn missing_robot_stream_fails_closed_instead_of_reporting_done() {
        let (tx, mut rx) = mpsc::channel(1);
        relay_optional_robot_turn_stream(None, tx).await;

        assert!(matches!(
            rx.recv().await,
            Some(TurnEvent::Failed {
                provider_fault: false,
                ..
            })
        ));
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn lagged_robot_stream_fails_closed_instead_of_reporting_done() {
        use nomifun_ai_agent::protocol::events::StartEventData;

        let (source, stream) = broadcast::channel(1);
        source
            .send(AgentStreamEvent::Start(StartEventData::default()))
            .unwrap();
        source
            .send(AgentStreamEvent::Start(StartEventData::default()))
            .unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        relay_robot_turn_stream(stream, tx).await;

        assert!(matches!(
            rx.recv().await,
            Some(TurnEvent::Failed {
                provider_fault: false,
                ..
            })
        ));
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn closed_robot_stream_without_terminal_fails_closed() {
        let (source, stream) = broadcast::channel(1);
        drop(source);
        let (tx, mut rx) = mpsc::channel(1);
        relay_robot_turn_stream(stream, tx).await;

        assert!(matches!(
            rx.recv().await,
            Some(TurnEvent::Failed {
                provider_fault: false,
                ..
            })
        ));
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn thinking_is_never_spoken_and_text_waits_for_finish() {
        use nomifun_ai_agent::protocol::events::{TextEventData, ThinkingEventData};

        let mut reducer = SpokenReplyReducer::default();
        assert_eq!(
            reducer.push(AgentStreamEvent::Thinking(ThinkingEventData {
                content: "internal reasoning".to_owned(),
                subject: None,
                duration: None,
                status: None,
            })),
            vec![]
        );
        assert_eq!(
            reducer.push(AgentStreamEvent::Text(TextEventData {
                content: "在".to_owned(),
            })),
            vec![],
            "an unconfirmed text delta must not reach TTS"
        );
        assert_eq!(
            reducer.push(AgentStreamEvent::Text(TextEventData {
                content: "呢".to_owned(),
            })),
            vec![]
        );
        assert_eq!(
            reducer.push(AgentStreamEvent::Finish(Default::default())),
            vec![TurnEvent::Text("在呢".to_owned()), TurnEvent::Done]
        );
    }

    #[test]
    fn discarded_attempt_keeps_the_latest_start_prefix_out_of_tts_until_finish() {
        use nomifun_ai_agent::protocol::events::{
            OutputDiscardedEventData, StartEventData, TextEventData,
        };

        let mut reducer = SpokenReplyReducer::default();
        assert!(
            reducer
                .push(AgentStreamEvent::Start(StartEventData::default()))
                .is_empty()
        );
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "前缀".to_owned(),
                }))
                .is_empty()
        );
        assert!(
            reducer
                .push(AgentStreamEvent::Start(StartEventData::default()))
                .is_empty()
        );
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "废弃草稿".to_owned(),
                }))
                .is_empty()
        );
        assert!(
            reducer
                .push(AgentStreamEvent::OutputDiscarded(
                    OutputDiscardedEventData { restart_attempt: 2 },
                ))
                .is_empty()
        );
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "答案".to_owned(),
                }))
                .is_empty()
        );
        assert_eq!(
            reducer.push(AgentStreamEvent::Finish(Default::default())),
            vec![TurnEvent::Text("前缀答案".to_owned()), TurnEvent::Done]
        );
    }

    #[test]
    fn tool_progress_text_is_discarded_and_only_the_final_answer_is_spoken() {
        use nomifun_ai_agent::protocol::events::TextEventData;

        let mut reducer = SpokenReplyReducer::default();
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "我先搜索一下。".to_owned(),
                }))
                .is_empty()
        );
        assert!(reducer.push(running_tool_call()).is_empty());
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "已经为你打开视频。".to_owned(),
                }))
                .is_empty()
        );
        assert_eq!(
            reducer.push(AgentStreamEvent::Finish(Default::default())),
            vec![
                TurnEvent::Text("已经为你打开视频。".to_owned()),
                TurnEvent::Done
            ],
            "the narration before the tool call must never become TTS input"
        );
    }

    #[test]
    fn tool_progress_without_a_final_answer_stays_silent() {
        use nomifun_ai_agent::protocol::events::TextEventData;

        let mut reducer = SpokenReplyReducer::default();
        assert!(
            reducer
                .push(AgentStreamEvent::Text(TextEventData {
                    content: "我正在处理。".to_owned(),
                }))
                .is_empty()
        );
        assert!(reducer.push(running_tool_call()).is_empty());
        assert_eq!(
            reducer.push(AgentStreamEvent::Finish(Default::default())),
            vec![TurnEvent::Done],
            "stale progress narration must not be spoken when no final text follows the tool"
        );
    }
}

#[cfg(test)]
#[path = "robot_wiring/unified_tests.rs"]
mod unified_tests;
