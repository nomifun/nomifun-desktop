//! Concrete conversation/model access for the robot gateway.
//!
//! Lives here because only this crate holds a `ConversationService`, the agent
//! runtime registry, the companion registry, the provider catalog and the
//! installation owner id at once. `nomifun-robot` sees only its own traits, so
//! the dependency direction stays one-way.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nomifun_api_types::{
    AgentErrorOwnership, CreateConversationRequest, SendMessageRequest, SessionMcpServer,
    SessionMcpTransport, UpdateConversationRequest,
};
use nomifun_common::AgentType;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_ai_agent::protocol::events::AgentStreamEvent;
use nomifun_conversation::ConversationService;
use nomifun_db::{IClientPreferenceRepository, IProviderModelRepository, IProviderRepository};
use nomifun_robot::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
use nomifun_robot::mcp_proxy::{MCP_PROXY_SERVER_NAME, RobotMcpProxyServer};
use nomifun_robot::registry::RobotRegistry;
use nomifun_robot::services::{SpeechServices, TurnEvent};
use nomifun_robot::status::RobotStatusRegistry;
use nomifun_robot::tool_registry::RobotToolRegistry;
use nomifun_robot::vad::VadTuning;
use nomifun_robot::wiring::{
    CompanionSlotReader, PreferenceReader, ProviderCredentials, ProviderRowReader, RobotSpeech,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};

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
        provider_repo: Arc<dyn IProviderRepository>,
        provider_model_repo: Arc<dyn IProviderModelRepository>,
        preference_repo: Arc<dyn IClientPreferenceRepository>,
        encryption_key: [u8; 32],
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
            provider_model_repo,
        });
        let speech: Arc<dyn SpeechServices> = Arc::new(RobotSpeech::new(
            invoke,
            slots,
            Arc::new(AppPreferences {
                repo: preference_repo,
            }),
            Arc::new(AppProviderRows {
                repo: provider_repo,
                encryption_key,
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
    provider_model_repo: Arc<dyn IProviderModelRepository>,
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
        let Ok(Some(row)) = self.provider_model_repo.get(provider_id, model).await else {
            return false;
        };
        serde_json::from_str::<Vec<nomifun_api_types::ModelTrait>>(&row.traits)
            .is_ok_and(|traits| traits.contains(&nomifun_api_types::ModelTrait::VisionInput))
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
            return Some((vision.provider_id, vision.model));
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

/// Provider rows, decrypted for the one direct (non-invoke) call the robot
/// makes: one-shot vision.
struct AppProviderRows {
    repo: Arc<dyn IProviderRepository>,
    encryption_key: [u8; 32],
}

#[async_trait::async_trait]
impl ProviderRowReader for AppProviderRows {
    async fn credentials(&self, provider_id: &str) -> Option<ProviderCredentials> {
        let provider = self.repo.find_by_id(provider_id).await.ok()??;
        if !provider.enabled {
            tracing::warn!(provider_id, "robot: vision provider is disabled");
            return None;
        }
        let decrypted =
            nomifun_common::decrypt_string(&provider.api_key_encrypted, &self.encryption_key)
                .inspect_err(|error| {
                    tracing::warn!(provider_id, %error, "robot: provider key decrypt failed");
                })
                .ok()?;
        // Stored keys are a comma/newline separated rotation list; the invoke
        // layer takes the first non-empty entry and so does this.
        let api_key = decrypted
            .split([',', '\n'])
            .map(str::trim)
            .find(|key| !key.is_empty())?
            .to_owned();
        Some(ProviderCredentials {
            api_key,
            base_url: provider.base_url,
            platform: provider.platform,
        })
    }
}

// ---------------------------------------------------------------------------
// Conversation backend
// ---------------------------------------------------------------------------

/// The physical-body section appended to the companion persona.
fn robot_body_prompt() -> &'static str {
    "你现在通过一台物理机器人和用户说话。它有一块 OLED 表情屏、一个可以转动的头（云台）、扬声器和麦克风。\n\
     - 回复必须简短口语化：每句不超过 40 字，整体不超过 3 句，除非用户明确要求详细内容。\n\
     - 每句话可以用 [emotion:名] 开头来驱动表情和头部动作，可用的名字只有：neutral, happy, laughing, funny, sad, angry, crying, loving, embarrassed, surprised, shocked, thinking, winking, cool, relaxed, delicious, kissy, confident, sleepy, silly, confused。\n\
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
            loop {
                match stream.recv().await {
                    Ok(event) => match reduce_event(event) {
                        Some(TurnEvent::Done) => {
                            let _ = tx.send(TurnEvent::Done).await;
                            break;
                        }
                        Some(reduced) => {
                            let terminal = matches!(reduced, TurnEvent::Failed { .. });
                            if tx.send(reduced).await.is_err() || terminal {
                                break;
                            }
                        }
                        None => {}
                    },
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

/// Reduce the agent's rich event stream to the three things the downlink needs.
///
/// `provider_fault` decides whether a fallback-model retry makes sense, and the
/// platform already classifies that for us: `UserLlmProvider` and
/// `UnknownUpstream` are upstream problems, everything else is ours or the
/// user's and would fail identically on the fallback model.
fn reduce_event(event: AgentStreamEvent) -> Option<TurnEvent> {
    match event {
        AgentStreamEvent::Text(data) => Some(TurnEvent::Text(data.content)),
        AgentStreamEvent::Finish(_) => Some(TurnEvent::Done),
        AgentStreamEvent::Error(data) => {
            let provider_fault = matches!(
                data.ownership,
                Some(AgentErrorOwnership::UserLlmProvider) | Some(AgentErrorOwnership::UnknownUpstream)
            );
            Some(TurnEvent::Failed {
                message: data.message,
                provider_fault,
            })
        }
        // Tool cards, thinking, plans, tips: visible in the desktop UI, not
        // something a speaker can convey.
        _ => None,
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
            reduce_event(error(Some(AgentErrorOwnership::UserLlmProvider))),
            Some(TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true
            })
        );
        assert_eq!(
            reduce_event(error(Some(AgentErrorOwnership::UnknownUpstream))),
            Some(TurnEvent::Failed {
                message: "boom".to_owned(),
                provider_fault: true
            })
        );
        for ours in [
            Some(AgentErrorOwnership::Nomifun),
            Some(AgentErrorOwnership::UserAgent),
            None,
        ] {
            assert_eq!(
                reduce_event(error(ours)),
                Some(TurnEvent::Failed {
                    message: "boom".to_owned(),
                    provider_fault: false
                }),
                "{ours:?} would fail the same way on the fallback model"
            );
        }
    }

    #[test]
    fn only_text_and_terminal_events_reach_the_speaker() {
        assert_eq!(
            reduce_event(AgentStreamEvent::Text(
                nomifun_ai_agent::protocol::events::TextEventData {
                    content: "在呢".to_owned(),
                }
            )),
            Some(TurnEvent::Text("在呢".to_owned()))
        );
        assert_eq!(
            reduce_event(AgentStreamEvent::Finish(Default::default())),
            Some(TurnEvent::Done)
        );
        assert_eq!(
            reduce_event(AgentStreamEvent::Thinking(
                nomifun_ai_agent::protocol::events::ThinkingEventData {
                    content: "hmm".to_owned(),
                    subject: None,
                    duration: None,
                    status: None,
                }
            )),
            None,
            "a speaker cannot convey a thinking card"
        );
    }
}
