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
use nomifun_api_types::{
    AgentErrorOwnership, CreateConversationRequest, SendMessageRequest, SessionMcpServer,
    SessionMcpTransport, UpdateConversationRequest,
};
use nomifun_common::{AgentType, ProviderWithModel};
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_ai_agent::protocol::events::AgentStreamEvent;
use nomifun_conversation::ConversationService;
use nomifun_db::IClientPreferenceRepository;
use nomifun_robot::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
use nomifun_robot::mcp_proxy::{MCP_PROXY_SERVER_NAME, RobotMcpProxyServer};
use nomifun_robot::registry::RobotRegistry;
use nomifun_robot::services::{SpeechServices, TurnEvent};
use nomifun_robot::status::RobotStatusRegistry;
use nomifun_robot::tool_registry::RobotToolRegistry;
use nomifun_robot::vad::VadTuning;
use nomifun_robot::wiring::{
    CompanionSlotReader, PreferenceReader, RobotSpeech, VisionCompletionExecutor,
    VisionCompletionRequest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};

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
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub tools: Arc<RobotToolRegistry>,
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
            registry,
            status,
            tools,
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
/// (`stream_relay.rs`'s `robot_session` gate, which owns the live stream, the
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
    "你现在通过一台物理机器人和用户说话。它有一块 OLED 表情屏、一个可以转动的头（云台）、扬声器和麦克风。\n\
     - 你写的每一个字都会被语音合成念出声，同时显示在一块 128x64 的小屏上。所以只写能读出来的自然口语。\n\
     - 回复必须简短口语化：每句不超过 40 字，整体不超过 3 句，除非用户明确要求详细内容。\n\
     - 只输出要说出来的那句话本身。不要写任何方括号或【】里的标注（例如 [winking]、[开心]、【笑】），不要写动作描写或舞台提示，不要写旁白和括号里的补充说明。\n\
     - 不要输出 emoji、颜文字、markdown 记号（星号、井号、反引号），以及任何念不出声的符号。需要停顿就用逗号和句号。\n\
     - 需要转头、看某个方向或调音量时，用 robot_ 开头的工具。"
}

/// A stable, valid UUIDv7 naming this robot's session MCP registration.
///
/// `McpServerId` accepts nothing but a UUIDv7, and a fresh id on every boot
/// would rewrite the conversation's `extra` for no reason, so the id is derived:
/// a fixed timestamp plus ten bytes of the robot id's digest. Hashing (rather
/// than slicing the MAC) matters because every MAC on one board shares a prefix.
fn robot_mcp_server_id(robot_id: &str) -> String {
    let digest = Sha256::digest(robot_id.as_bytes());
    let mut random_bytes = [0u8; 10];
    random_bytes.copy_from_slice(&digest[..10]);
    uuid::Builder::from_unix_timestamp_millis(0, &random_bytes)
        .into_uuid()
        .to_string()
}

/// Backfill a robot thread created before its companion had a chat model.
///
/// An existing selection may be the fallback chosen after a provider failure,
/// so only a genuinely empty conversation inherits the companion model.
fn missing_robot_thread_model(
    current: Option<&ProviderWithModel>,
    configured: Option<&ProviderWithModel>,
) -> Option<ProviderWithModel> {
    current.is_none().then(|| configured.cloned()).flatten()
}

/// Real conversation access for one installation.
pub struct AppRobotBackend {
    pub conversations: ConversationService,
    pub runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    pub companions: Arc<nomifun_companion::CompanionService>,
    pub owner_user_id: Arc<str>,
    pub data_dir: PathBuf,
    /// Live robot MCP proxy URL + headers, so a reused thread is refreshed
    /// instead of pointing at last boot's port.
    pub mcp_proxy: Option<Arc<RobotMcpProxyServer>>,
}

impl AppRobotBackend {
    /// The session MCP registration for this robot's toolset, or `None` when the
    /// loopback proxy is not running (the thread then simply has no robot tools).
    fn session_mcp_servers(&self, robot_id: &str) -> Option<Vec<SessionMcpServer>> {
        let proxy = self.mcp_proxy.as_ref()?;
        let mcp_server_id = nomifun_api_types::McpServerId::parse(robot_mcp_server_id(robot_id))
            .expect("a derived v7 uuid is a valid McpServerId");
        Some(vec![SessionMcpServer {
            mcp_server_id,
            name: MCP_PROXY_SERVER_NAME.to_owned(),
            transport: SessionMcpTransport::StreamableHttp {
                url: proxy.url_for(robot_id),
                headers: proxy
                    .headers()
                    .into_iter()
                    .collect::<HashMap<String, String>>(),
            },
        }])
    }

    /// The companion's own persona with the robot body section appended.
    ///
    /// Built here rather than left to the agent factory because the factory only
    /// builds a persona when `extra.system_prompt` is absent, and the body
    /// section must sit on top of the persona — the same companion has to sound
    /// like itself on the desktop and in the room.
    async fn robot_system_prompt(&self, companion_id: &str) -> String {
        let persona = nomifun_ai_agent::CompanionPromptProvider::build_system_prompt(
            &*self.companions,
            Some(companion_id),
            None,
        )
        .await
        .unwrap_or_default();
        if persona.trim().is_empty() {
            robot_body_prompt().to_owned()
        } else {
            format!("{persona}\n\n{}", robot_body_prompt())
        }
    }

    /// `{data_dir}/robot/threads.json` — `"{robot_id}|{companion_id}" -> conversation_id`.
    ///
    /// A separate file from `robots.json` on purpose: rebinding a robot must not
    /// be able to corrupt the device registry, and a lost thread map costs one
    /// new conversation, not a re-pairing.
    fn threads_path(&self) -> PathBuf {
        self.data_dir
            .join(nomifun_robot::registry::ROBOT_REL_DIR)
            .join("threads.json")
    }

    async fn read_threads(&self) -> std::collections::BTreeMap<String, String> {
        match tokio::fs::read(self.threads_path()).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Default::default(),
        }
    }

    async fn lookup_thread(
        &self,
        robot_id: &str,
        companion_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let key = format!("{robot_id}|{companion_id}");
        let Some(conversation_id) = self.read_threads().await.get(&key).cloned() else {
            return Ok(None);
        };
        // A conversation the user deleted must not be resurrected as a ghost id.
        match self
            .conversations
            .get(&self.owner_user_id, &conversation_id)
            .await
        {
            Ok(_) => Ok(Some(conversation_id)),
            Err(_) => Ok(None),
        }
    }

    async fn record_thread(
        &self,
        robot_id: &str,
        companion_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<()> {
        let mut threads = self.read_threads().await;
        threads.insert(
            format!("{robot_id}|{companion_id}"),
            conversation_id.to_owned(),
        );
        let path = self.threads_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(&threads)?).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Point the conversation at the companion's fallback chat model for the
    /// retry. Changing the model kills and rebuilds the runtime, which is
    /// exactly what a retry after a provider outage wants: a fresh client
    /// against a different provider. The model is deliberately **left** on the
    /// fallback — silently flipping back would send the next turn straight into
    /// the same outage, and the UI shows the robot thread's model, so the state
    /// stays visible.
    async fn apply_fallback_model(&self, conversation_id: &str) -> anyhow::Result<()> {
        let companion_id = self
            .companion_of(conversation_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("conversation is not a robot thread"))?;
        let profile = self.companions.get_companion(&companion_id).await?;
        let fallback = profile
            .fallback_model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no fallback model configured"))?;
        tracing::warn!(
            conversation_id,
            provider_id = %fallback.provider_id,
            model = %fallback.model,
            "robot: switching this thread to the fallback model after a provider fault"
        );
        let request = UpdateConversationRequest {
            name: None,
            pinned: None,
            model: Some(fallback),
            delegation_policy: None,
            execution_model_pool: None,
            decision_policy: None,
            execution_template_id: None,
            extra: None,
        };
        self.conversations
            .update(
                &self.owner_user_id,
                conversation_id,
                request,
                &self.runtime_registry,
            )
            .await?;
        Ok(())
    }

    /// The companion this robot thread belongs to, read from the conversation's
    /// own `extra` so it survives a restart with no in-memory state.
    async fn companion_of(&self, conversation_id: &str) -> Option<String> {
        let conversation = self
            .conversations
            .get(&self.owner_user_id, conversation_id)
            .await
            .ok()?;
        conversation
            .extra
            .get("companion_id")?
            .as_str()
            .map(str::to_owned)
    }

    /// A robot can connect before the user finishes configuring its companion.
    /// The thread is durable, so creation-time model copying alone leaves that
    /// thread permanently unconfigured. Heal it from the live companion profile
    /// while preserving any existing selection (including a fallback model).
    async fn backfill_companion_model_if_missing(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<()> {
        let conversation = self
            .conversations
            .get(&self.owner_user_id, conversation_id)
            .await?;
        if conversation.model.is_some() {
            return Ok(());
        }

        let companion_id = conversation
            .extra
            .get("companion_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("conversation is not a robot thread"))?;
        let profile = self.companions.get_companion(companion_id).await?;
        let Some(model) = missing_robot_thread_model(None, profile.model.as_ref()) else {
            return Ok(());
        };

        tracing::info!(
            conversation_id,
            companion_id,
            provider_id = %model.provider_id,
            model = %model.model,
            "robot: backfilling the companion chat model onto an unconfigured thread"
        );
        self.conversations
            .update(
                &self.owner_user_id,
                conversation_id,
                UpdateConversationRequest {
                    name: None,
                    pinned: None,
                    model: Some(model),
                    delegation_policy: None,
                    execution_model_pool: None,
                    decision_policy: None,
                    execution_template_id: None,
                    extra: None,
                },
                &self.runtime_registry,
            )
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl nomifun_robot::wiring::RobotConversationBackend for AppRobotBackend {
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String> {
        let session_mcp = self.session_mcp_servers(robot_id);
        let system_prompt = self.robot_system_prompt(companion_id).await;

        // Reuse the thread recorded for this pair, refreshing both per-boot
        // facts: the proxy URL (the port is per-boot) and the persona (its
        // embedded memory snapshot would otherwise be frozen at creation).
        // `update_extra` merges, and it writes the already-resolved
        // `session_mcp_servers` key that the agent build reads.
        if let Some(existing) = self.lookup_thread(robot_id, companion_id).await? {
            let mut patch = json!({ "system_prompt": system_prompt });
            if let Some(servers) = &session_mcp {
                patch["session_mcp_servers"] = serde_json::to_value(servers)?;
            }
            self.conversations.update_extra(&existing, patch).await?;
            self.backfill_companion_model_if_missing(&existing).await?;
            return Ok(existing);
        }

        // `companion_session` + `companion_id` mark this as a companion-owned
        // thread (memory tools, gateway authority). The persona is supplied
        // explicitly rather than left to the factory, because the robot body
        // section has to sit on top of it and the factory only builds a persona
        // when `system_prompt` is absent.
        let mut extra = json!({
            "robot_session": true,
            "robot_id": robot_id,
            "companion_session": true,
            "companion_id": companion_id,
            "system_prompt": system_prompt,
            // The companion persona already carries the frozen preset
            // instructions; without this the generic path appends them twice.
            "preset_instructions_embedded": true,
            // No approval UI exists on a robot: a tool call under the default
            // mode would park forever and the device would wait in silence.
            "session_mode": "yolo",
        });
        if let Some(servers) = &session_mcp {
            extra["selected_session_mcp_servers"] = serde_json::to_value(servers)?;
        }

        let model = self
            .companions
            .get_companion(companion_id)
            .await
            .ok()
            .and_then(|profile| profile.model);
        let request = CreateConversationRequest {
            r#type: AgentType::Nomi,
            name: Some(format!("机器人 · {robot_id}")),
            model,
            source: None,
            channel_chat_id: None,
            preset_id: None,
            preset_overrides: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        };
        // Keyed create: a crash between `create` and `record_thread` must not
        // leave a second thread behind on the next handshake.
        let creation_key = format!("robot-thread:v1:{robot_id}|{companion_id}");
        let conversation = self
            .conversations
            .create_idempotent(&self.owner_user_id, request, &creation_key)
            .await?;
        self.record_thread(robot_id, companion_id, &conversation.conversation_id)
            .await?;
        Ok(conversation.conversation_id)
    }

    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        if use_fallback_model {
            self.apply_fallback_model(conversation_id).await?;
        } else {
            // The companion may have been configured after this durable robot
            // thread was first created. Resolve that race on the next utterance,
            // even when the device stayed connected and never re-ran handshake.
            self.backfill_companion_model_if_missing(conversation_id)
                .await?;
        }
        let request = SendMessageRequest {
            content: text.to_owned(),
            files: vec![],
            inject_skills: vec![],
            hidden: false,
            origin: None,
            channel_platform: Some("robot".to_owned()),
        };
        let delivery = self
            .conversations
            .send_message_with_idempotency_key(
                &self.owner_user_id,
                conversation_id,
                &uuid::Uuid::now_v7().to_string(),
                request,
                &self.runtime_registry,
            )
            .await?;

        let (tx, rx) = mpsc::channel(64);
        // The keyed send admits synchronously but builds a cold runtime in its
        // own background task, so the stream is attached after admission.
        let stream = if delivery.completed {
            None
        } else {
            wait_for_runtime_subscription(&self.runtime_registry, conversation_id).await
        };
        tokio::spawn(async move {
            let Some(mut stream) = stream else {
                let _ = tx.send(TurnEvent::Done).await;
                return;
            };
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
                    // A lagged or closed broadcast ends the turn rather than
                    // leaving the device listening to nothing forever.
                    Err(_) => {
                        let _ = tx.send(TurnEvent::Done).await;
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }

    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
        // Public `cancel` only: `cancel_with_origin` is crate-private, and
        // `runtime_registry.terminate` would kill the runtime, not the turn.
        self.conversations
            .cancel(&self.owner_user_id, conversation_id, &self.runtime_registry)
            .await?;
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

    async fn has_fallback_model(&self, companion_id: &str) -> bool {
        self.companions
            .get_companion(companion_id)
            .await
            .map(|profile| profile.fallback_model.is_some())
            .unwrap_or(false)
    }
}

/// Attach to a conversation's runtime stream once the registry publishes it.
///
/// A copy of the channel domain's own helper (`nomifun-channel`'s
/// `wait_for_runtime_subscription`, which is private to that crate): registration
/// happens before prompt dispatch, so polling until it appears is what makes a
/// cold-start turn observable. A timeout degrades streaming only and never
/// retries the model turn.
async fn wait_for_runtime_subscription(
    runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
    conversation_id: &str,
) -> Option<tokio::sync::broadcast::Receiver<AgentStreamEvent>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(handle) = runtime_registry.get_runtime(conversation_id) {
            return Some(handle.subscribe());
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                conversation_id,
                "robot: runtime did not register before the subscription timeout"
            );
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
}

impl SpokenReplyReducer {
    fn push(&mut self, event: AgentStreamEvent) -> Vec<TurnEvent> {
        match event {
            AgentStreamEvent::Start(_) => {
                self.candidate.clear();
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
                }
                Vec::new()
            }
            AgentStreamEvent::Plan(_)
            | AgentStreamEvent::ToolCall(_)
            | AgentStreamEvent::ToolGroup(_)
            | AgentStreamEvent::Permission(_) => {
                self.candidate.clear();
                Vec::new()
            }
            AgentStreamEvent::Finish(_) => {
                let answer = std::mem::take(&mut self.candidate);
                let mut reduced = Vec::with_capacity(2);
                if !answer.trim().is_empty() {
                    reduced.push(TurnEvent::Text(answer));
                }
                reduced.push(TurnEvent::Done);
                reduced
            }
            AgentStreamEvent::Error(data) => {
                self.candidate.clear();
                // `provider_fault` decides whether a fallback-model retry makes
                // sense. The platform already classifies upstream ownership.
                let provider_fault = matches!(
                    data.ownership,
                    Some(AgentErrorOwnership::UserLlmProvider)
                        | Some(AgentErrorOwnership::UnknownUpstream)
                );
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
    data_dir: PathBuf,
) -> RobotFaces {
    let backend = Arc::new(AppRobotBackend {
        conversations,
        runtime_registry,
        companions,
        owner_user_id,
        data_dir,
        mcp_proxy: robot.proxy.clone(),
    });
    let dispatcher = Arc::new(nomifun_robot::wiring::RobotDispatcher::new(backend));
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
        }),
        admin: nomifun_robot::routes::admin_router(nomifun_robot::routes::RobotAdminState {
            registry: robot.registry.clone(),
            status: robot.status.clone(),
            advertiser: robot.advertiser.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider_id: &str, name: &str) -> ProviderWithModel {
        ProviderWithModel {
            provider_id: provider_id.to_owned(),
            model: name.to_owned(),
            use_model: None,
        }
    }

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

    #[test]
    fn a_missing_robot_thread_model_inherits_the_companion_model() {
        let configured = model("provider-a", "chat-primary");
        assert_eq!(
            missing_robot_thread_model(None, Some(&configured)),
            Some(configured)
        );
    }

    #[test]
    fn an_existing_robot_thread_model_is_never_overwritten() {
        let fallback = model("provider-b", "chat-fallback");
        let configured = model("provider-a", "chat-primary");
        assert_eq!(
            missing_robot_thread_model(Some(&fallback), Some(&configured)),
            None,
            "the current selection may be a deliberately persistent fallback"
        );
    }

    #[test]
    fn an_unconfigured_companion_cannot_backfill_a_robot_thread() {
        assert_eq!(missing_robot_thread_model(None, None), None);
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
    fn a_robots_mcp_server_id_is_a_stable_uuidv7() {
        let first = robot_mcp_server_id("aa:bb:cc:dd:ee:ff");
        assert_eq!(first, robot_mcp_server_id("aa:bb:cc:dd:ee:ff"));
        assert!(
            nomifun_api_types::McpServerId::parse(first.clone()).is_ok(),
            "{first} must satisfy the McpServerId contract"
        );
        assert_ne!(
            first,
            robot_mcp_server_id("aa:bb:cc:dd:ee:f0"),
            "two robots on one board differ only in the last byte"
        );
    }

    #[test]
    fn only_upstream_faults_are_worth_a_fallback_retry() {
        let error = |ownership| {
            AgentStreamEvent::Error(nomifun_api_types::AgentStreamErrorData {
                message: "boom".to_owned(),
                code: None,
                ownership,
                detail: None,
                workspace_path: None,
                retryable: None,
                feedback_recommended: None,
                resolution: None,
            })
        };
        assert_eq!(
            SpokenReplyReducer::default()
                .push(error(Some(AgentErrorOwnership::UserLlmProvider))),
            vec![TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true
            }]
        );
        assert_eq!(
            SpokenReplyReducer::default()
                .push(error(Some(AgentErrorOwnership::UnknownUpstream))),
            vec![TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true
            }]
        );
        for ours in [
            Some(AgentErrorOwnership::Nomifun),
            Some(AgentErrorOwnership::UserAgent),
            None,
        ] {
            assert_eq!(
                SpokenReplyReducer::default().push(error(ours)),
                vec![TurnEvent::Failed {
                    message: "boom".to_owned(),
                    provider_fault: false
                }],
                "{ours:?} would fail the same way on the fallback model"
            );
        }
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
