//! Canonical AgentSession/model access for the robot gateway.
//!
//! `nomifun-robot` sees only its own traits; this app adapter keeps device
//! ingress on the same immutable Session/Turn authority as every other Agent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use futures_util::{FutureExt, future::{BoxFuture, Shared}};
use nomifun_api_types::SendMessageRequest;
use nomifun_db::IClientPreferenceRepository;
use nomifun_robot::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
use nomifun_robot::effect_ledger::RobotEffectLedger;
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

#[path = "robot_report_delivery.rs"]
mod robot_report_delivery;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc, watch};

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
            None,
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
    /// listener comes up, while the gateway's canonical Session owner is attached
    /// during router assembly.
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
    /// Current and replaced gateways retain their own complete task witnesses.
    gateways: Mutex<GatewayOwners>,
    bridge_workers: Arc<RobotBridgeWorkers>,
}

#[derive(Default)]
struct GatewayOwners {
    active: Option<Arc<nomifun_robot::RobotGateway>>,
    retired: Vec<Arc<nomifun_robot::RobotGateway>>,
    closing: bool,
}

type BridgeCompletion = Shared<BoxFuture<'static, Result<(), String>>>;
struct BridgeWorker {
    cancellation: tokio_util::sync::CancellationToken,
    completion: BridgeCompletion,
}
#[derive(Default)]
struct BridgeState {
    workers: HashMap<String, Arc<BridgeWorker>>,
    closing: bool,
    errors: Vec<String>,
}
#[derive(Default)]
struct RobotBridgeWorkers(Mutex<BridgeState>);

impl RobotBridgeWorkers {
    fn spawn<F, Fut>(&self, key: String, work: F) -> anyhow::Result<()>
    where F: FnOnce(tokio_util::sync::CancellationToken) -> Fut,
        Fut: std::future::Future<Output=()> + Send + 'static,
    {
        let mut state = self.0.lock().map_err(|_| anyhow::anyhow!("Robot dispatcher inventory poisoned"))?;
        reap_bridge_workers(&mut state);
        if state.closing || !state.errors.is_empty() { anyhow::bail!("Robot dispatcher admission is closed or unproven"); }
        if state.workers.len() >= 64 { anyhow::bail!("device utterance queue is full"); }
        if state.workers.contains_key(&key) { anyhow::bail!("this device utterance is already queued"); }
        let cancellation = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(work(cancellation.clone()));
        let completion = async move { task.await.map_err(|error| format!("Robot dispatcher worker ended abnormally: {error}")) }.boxed().shared();
        state.workers.insert(key, Arc::new(BridgeWorker { cancellation, completion }));
        Ok(())
    }

    fn cancel(&self, key: &str) -> anyhow::Result<()> {
        let state = self.0.lock().map_err(|_| anyhow::anyhow!("Robot dispatcher inventory poisoned"))?;
        if let Some(worker) = state.workers.get(key) { worker.cancellation.cancel(); }
        Ok(()) // Request only. The retained completion is the local join proof.
    }

    fn request_shutdown(&self) -> anyhow::Result<()> {
        let mut state = self.0.lock().map_err(|_| anyhow::anyhow!("Robot dispatcher inventory poisoned"))?;
        state.closing = true;
        for worker in state.workers.values() { worker.cancellation.cancel(); }
        Ok(())
    }

    async fn shutdown_and_wait(&self) -> anyhow::Result<()> {
        self.request_shutdown()?;
        loop {
            let workers = {
                let mut state = self.0.lock().map_err(|_| anyhow::anyhow!("Robot dispatcher inventory poisoned"))?;
                reap_bridge_workers(&mut state);
                if state.workers.is_empty() {
                    return if state.errors.is_empty() { Ok(()) } else { Err(anyhow::anyhow!(state.errors.join("; "))) };
                }
                state.workers.values().cloned().collect::<Vec<_>>()
            };
            for worker in workers { let _ = worker.completion.clone().await; }
        }
    }
}

fn reap_bridge_workers(state: &mut BridgeState) {
    let BridgeState { workers, errors, .. } = state;
    workers.retain(|_, worker| match worker.completion.clone().now_or_never() {
        None => true,
        Some(Ok(())) => false,
        Some(Err(error)) => { errors.push(error); false }
    });
}

impl RobotServices {
    #[cfg(test)]
    pub(crate) fn conversation_backend_for_test(&self)->Arc<dyn nomifun_robot::wiring::RobotConversationBackend> {
        self.backend.get().expect("Robot backend is installed with the App router").clone()
    }
    /// Build everything that does not need the canonical Session owner.
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
            gateways: Mutex::new(GatewayOwners::default()),
            bridge_workers: Arc::new(RobotBridgeWorkers::default()),
        })
    }

    /// A replacement never discards the prior gateway's cleanup witness.
    pub fn set_gateway(&self, gateway: Arc<nomifun_robot::RobotGateway>) {
        let mut owners = self.gateways.lock().expect("robot gateway owner lock poisoned");
        if let Some(previous) = owners.active.replace(gateway.clone()) {
            previous.request_shutdown();
            owners.retired.push(previous);
        }
        if owners.closing { gateway.request_shutdown(); }
    }

    /// Request only. Storage/host shutdown must await shutdown_and_wait.
    pub fn shutdown(&self) {
        if let Err(error) = self.bridge_workers.request_shutdown() {
            tracing::error!(%error, "Robot dispatcher shutdown is unproven");
        }
        let mut owners = self.gateways.lock().expect("robot gateway owner lock poisoned");
        owners.closing = true;
        for gateway in owners.active.iter().chain(owners.retired.iter()) {
            gateway.request_shutdown();
        }
    }

    /// Join every current/replaced owner within one wait budget. A timeout or
    /// dropped waiter retains all owners and their immutable terminal result.
    pub async fn shutdown_and_wait(&self) -> anyhow::Result<()> {
        self.bridge_workers.request_shutdown()?;
        let gateways = {
            let mut owners = self.gateways.lock().map_err(|_| anyhow::anyhow!("Robot gateway owner lock poisoned"))?;
            owners.closing = true;
            owners.active.iter().chain(owners.retired.iter()).cloned().collect::<Vec<_>>()
        };
        for gateway in &gateways { gateway.request_shutdown(); }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut errors = Vec::new();
            for gateway in gateways {
                if let Err(error) = gateway.shutdown_and_wait().await { errors.push(error); }
            }
            if let Err(error) = self.bridge_workers.shutdown_and_wait().await { errors.push(error.to_string()); }
            if errors.is_empty() { Ok(()) } else { Err(anyhow::anyhow!(errors.join("; "))) }
        }).await.map_err(|_| anyhow::anyhow!("Robot gateway cleanup is still pending after 5 seconds"))?
    }

    #[cfg(test)]
    pub(crate) fn inject_bridge_panic_for_test(&self) -> anyhow::Result<()> {
        self.bridge_workers.spawn("controlled-cleanup-failure".into(), |_| async {
            panic!("controlled Robot bridge cleanup failure")
        })
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

    /// Whether the configured Chat serializer can carry image input.
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
                nomifun_chat_model_broker::chat_protocol_for_id(&resolved.protocol)
                    .is_some_and(|protocol| {
                        nomifun_chat_model_broker::protocol_features(protocol)
                            .contains(&nomifun_chat_model_broker::ChatModelFeature::ImageInput)
                    })
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
        // Without a dedicated slot, the exact configured Chat protocol decides
        // whether the main model can receive images; catalog metadata is advisory.
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

/// Bridge device speech into the Companion's canonical AgentSession.
/// Conversational gestures use discovered, explicitly granted MCP tools and
/// ordinary Session effects. Speech/display cleanup keeps stage directions out
/// of the text; it does not interpret them as physical control instructions.
/// Protocol emotion messages are reserved for gateway-owned state notices.
#[derive(Clone)]
pub(crate) struct AppRobotBackend {
    pub sessions: Arc<crate::router::nomi_core_session::NomiCoreSessionOwner>,
    pub companions: Arc<nomifun_companion::CompanionService>,
    pub owner_user_id: Arc<str>,
    pub registry: Arc<RobotRegistry>,
    bridge_workers: Arc<RobotBridgeWorkers>,
    queues: Arc<Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

fn snapshot_authorizes_companion_voice_input(
    snapshot: Option<&nomifun_api_types::AgentResolvedSnapshot>,
    owner_id: &str,
    request: &nomifun_robot::services::RobotTurnRequest,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let Some(binding_value) = snapshot.canonical_binding.as_ref() else {
        return false;
    };
    let mut companions = binding_value
        .typed_resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind == "companion");
    let Some(companion) = companions.next() else {
        return false;
    };
    if companions.next().is_some()
        || companion.owner_id != owner_id
        || companion.resource_id != request.companion_id
    {
        return false;
    }
    // The paired microphone is an input surface of this Companion's existing
    // Session. It does not need camera authority or grant device tools. Those
    // still require an explicitly selected resource and the normal Turn,
    // connection and permission fences. Do not change the Session binding here.
    let mut robot_bindings = binding_value
        .typed_resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind == "robot");
    let Some(binding) = robot_bindings.next() else {
        return true;
    };
    robot_bindings.next().is_none()
        && binding.owner_id == owner_id
        && binding.resource_id == request.robot_id
        && binding
            .typed_parameters
            .get("companion_id")
            .is_some_and(|companion| companion == &request.companion_id)
}

pub(crate) fn robot_turn_idempotency_key(request: &nomifun_robot::services::RobotTurnRequest) -> String {
    let identity = serde_json::json!([
        request.companion_id,
        request.robot_id,
        request.connection_id,
        request.request_id,
    ]);
    format!(
        "companion-device:{:x}",
        Sha256::digest(identity.to_string().as_bytes())
    )
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
            if queued_at.elapsed() > std::time::Duration::from_secs(15) {
                anyhow::bail!("伙伴仍在处理其他消息，本次排队已超时，请重试");
            }
            if !self.registry.connection_matches(&request.robot_id, &request.connection_id).await {
                anyhow::bail!("robot connection is no longer active");
            }
            let conversation = self
                .sessions
                .get_session(&self.owner_user_id, &request.conversation_id)
                .await?;
            if conversation.runtime.as_ref().is_some_and(|runtime| runtime.is_processing) {
                tokio::select! {
                    _ = cancelled.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
                }
                continue;
            }
            if conversation.extra.get("companion_id").and_then(Value::as_str)
                != Some(request.companion_id.as_str())
                || conversation
                    .extra
                    .get("companion_session")
                    .and_then(Value::as_bool)
                    != Some(true)
            {
                anyhow::bail!("robot turn belongs to another Companion AgentSession");
            }
            // Keep the authenticated connection stable until canonical input
            // admission completes, including time waiting for the Session gate.
            let connection_lease = self.registry.hold_action_connection(
                &request.robot_id, &request.connection_id).await
                .ok_or_else(|| anyhow::anyhow!("robot connection changed before admission"))?;
            let record = self
                .registry
                .get(&request.robot_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("robot was removed"))?;
            if record.companion_id.as_deref() != Some(request.companion_id.as_str())
                || !self
                    .registry
                    .connection_matches(&request.robot_id, &request.connection_id)
                    .await
            {
                anyhow::bail!("robot binding or connection changed");
            }
            if !snapshot_authorizes_companion_voice_input(
                conversation.agent_snapshot.as_ref(),
                &self.owner_user_id,
                request,
            ) {
                anyhow::bail!("the AgentSession does not authorize input for this Companion and device");
            }
            let key = robot_turn_idempotency_key(request);
            let message = SendMessageRequest {
                content: request.text.clone(), files: vec![], inject_skills: vec![],
                hidden: false, origin: Some("robot".to_owned()), channel_platform: Some("robot".to_owned()),
            };
            let delivery = tokio::select! {
                biased;
                _ = cancelled.cancelled() => {
                    self.sessions.cancel_robot_session_request(&self.owner_user_id, request, &key).await?;
                    return Ok(());
                },
                result = self.sessions.send_robot_session_message_idempotent(
                    &self.owner_user_id, request, &key, message,
                ) => result,
            };
            let _delivery = match delivery {
                Ok(delivery) => delivery,
                Err(error) => return Err(error.into()),
            };
            drop(connection_lease);
            // Subscribe before the first durable read so even a very fast
            // response is found without relying on a timed terminal poll.
            let mut wake=tokio::select! {
                _=cancelled.cancelled()=>{self.sessions.cancel_robot_session_request(&self.owner_user_id,request,&key).await?;return Ok(());},
                wake=self.sessions.robot_output_wake(&self.owner_user_id,&request.conversation_id,&key)=>wake?,
            };
            let mut after_seq=0;
            let mut delivered_bytes=std::collections::BTreeMap::new();
            loop {
                if !self.registry.connection_matches(&request.robot_id,&request.connection_id).await
                    || !self.registry.get(&request.robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(request.companion_id.as_str())) {
                    anyhow::bail!("robot output connection or binding was replaced");
                }
                let public=self.sessions.robot_public_output(&self.owner_user_id,&request.conversation_id,&key,after_seq,&mut delivered_bytes).await?;
                after_seq=public.through_seq;
                for (source,text) in public.segments {
                    tracing::info!(agent_session_id=%request.conversation_id,request_id=%request.request_id,
                        source_event_id=%source.message_id,through_seq=source.through_seq,"robot: canonical public source queued for delivery");
                    if tx.send(TurnEvent::PublicSpeech {source,text}).await.is_err() {return Ok(());}
                }
                match self
                    .sessions
                    .session_turn_delivery_state(
                        &self.owner_user_id,
                        &request.conversation_id,
                        &key,
                    )
                    .await?
                {
                    nomifun_conversation::PublicTurnDeliveryState::Completed(receipt) => {
                        let events=completed_robot_public_delivery_events(&self.sessions,&self.owner_user_id,request,&key,
                            &receipt,after_seq,&mut delivered_bytes).await?;
                        for event in events {
                            if tx.send(event).await.is_err() { break; }
                        }
                        return Ok(());
                    }
                    nomifun_conversation::PublicTurnDeliveryState::Accepted { .. } => {}
                    nomifun_conversation::PublicTurnDeliveryState::InputUnavailable {cause,..} => {
                        let _=tx.send(unavailable_robot_input_event(&cause)).await;
                        return Ok(());
                    }
                    nomifun_conversation::PublicTurnDeliveryState::Missing => {
                        anyhow::bail!("canonical robot Turn receipt disappeared");
                    }
                }
                tokio::select! {
                    _ = cancelled.cancelled() => {
                        self.sessions.cancel_robot_session_request(&self.owner_user_id, request, &key).await?;
                        return Ok(());
                    }
                    event=async {match wake.as_mut() {Some(receiver)=>receiver.recv().await.map(|_|()),None=>std::future::pending().await}}=>{
                        if matches!(event,Err(broadcast::error::RecvError::Closed)) {wake=None;}
                    },
                    // Recovery safety net only: successful canonical output
                    // and terminal events normally wake the exact reader.
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl nomifun_robot::services::RobotPlaybackSource for AppRobotBackend {
    async fn response_source(&self, robot_id: &str, conversation_id: &str)
        -> Result<nomifun_robot::services::RobotSavedResponse, String> {
        let device = self.registry.get(robot_id).await.ok_or("device not found")?;
        let companion = device.companion_id.as_deref().ok_or("device is not paired")?;
        self.sessions.robot_saved_response(&self.owner_user_id, companion, robot_id, conversation_id)
            .await.map_err(|error|error.to_string())
    }
}

#[async_trait::async_trait]
impl nomifun_robot::wiring::RobotConversationBackend for AppRobotBackend {
    async fn capture_input_context(&self,companion:&str,robot:&str,session:&str,connection:&str,question:Option<String>)->anyhow::Result<Option<nomifun_robot::services::RobotInputContext>> {
        let _connection=self.registry.hold_action_connection(robot,connection).await.ok_or_else(||anyhow::anyhow!("Robot input connection was replaced"))?;
        if !self.registry.get(robot).await.is_some_and(|record|record.companion_id.as_deref()==Some(companion)) {anyhow::bail!("Robot input binding was replaced");}
        let request=nomifun_robot::services::RobotTurnRequest {robot_id:robot.into(),companion_id:companion.into(),conversation_id:session.into(),connection_id:connection.into(),
            request_id:"capture-input-context".into(),text:String::new(),presented_question_id:question,captured_context:None};
        Ok(Some(self.sessions.capture_robot_input_context(&self.owner_user_id,&request).await?))
    }
    async fn input_context_matches(&self,request:&nomifun_robot::services::RobotTurnRequest)->anyhow::Result<bool> {Ok(self.sessions.robot_output_context_matches(&self.owner_user_id,request).await?)}
    async fn acquire_output_context(&self,request:&nomifun_robot::services::RobotTurnRequest,expected_proactive_revision:Option<u64>,question:Option<&nomifun_robot::services::RobotQuestionPlaybackAuthority>)->anyhow::Result<Option<nomifun_robot::services::RobotOutputLease>> {
        if request.captured_context.is_none() {return Ok(None);}
        let Some(connection)=self.registry.hold_action_connection(&request.robot_id,&request.connection_id).await else{return Ok(None);};
        if !self.registry.get(&request.robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(request.companion_id.as_str())
            &&expected_proactive_revision.is_none_or(|revision|record.permissions.proactive_speech&&record.authorization_revision==revision)) {return Ok(None);}
        let Some(context)=self.sessions.hold_robot_output_context(&self.owner_user_id,request).await? else{return Ok(None);};
        let question=match question {
            Some(authority)=>match self.sessions.hold_execution_question_playback(&self.owner_user_id,request,authority).await? {
                Some(guard)=>Some(guard),None=>return Ok(None),
            },None=>match self.sessions.hold_robot_request_playback(&self.owner_user_id,request).await? {
                Some(guard)=>Some(guard),None=>return Ok(None),
            },
        };
        Ok(Some(nomifun_robot::services::RobotOutputLease::new((connection,context,question))))
    }
    async fn question_presented(&self,request:&nomifun_robot::services::RobotTurnRequest,source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:&str)->anyhow::Result<()> {
        let _connection=self.registry.hold_action_connection(&request.robot_id,&request.connection_id).await.ok_or_else(||anyhow::anyhow!("Robot question connection was replaced"))?;
        if presentation_id.starts_with("robot-question:") {
            self.sessions.record_robot_question_presented(&self.owner_user_id,request,source,presentation_id).await?;
        }else {
            self.sessions.record_execution_question_presented(&self.owner_user_id,request,
                &nomifun_robot::services::RobotQuestionPlaybackAuthority {source:source.clone(),presentation_id:presentation_id.into()}).await?;
        }
        Ok(())
    }
    async fn supplement(&self,request:&nomifun_robot::services::RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()> {
        let _connection=self.registry.hold_action_connection(&request.robot_id,&request.connection_id).await.ok_or_else(||anyhow::anyhow!("Robot supplement connection was replaced"))?;
        if !self.registry.get(&request.robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(request.companion_id.as_str())) {anyhow::bail!("Robot supplement binding was replaced");}
        self.sessions.supplement_robot_session_request(&self.owner_user_id,request,&robot_turn_idempotency_key(request),text,correction,input_id).await?;Ok(())
    }
    async fn cancel_background_work(&self,request:&nomifun_robot::services::RobotTurnRequest)->anyhow::Result<()> {
        let _connection=self.registry.hold_action_connection(&request.robot_id,&request.connection_id).await.ok_or_else(||anyhow::anyhow!("Robot cancellation connection was replaced"))?;
        if !self.registry.get(&request.robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(request.companion_id.as_str())) {anyhow::bail!("Robot cancellation binding was replaced");}
        self.sessions.cancel_robot_background_work(&self.owner_user_id,request,&robot_turn_idempotency_key(request)).await?;Ok(())
    }
    async fn supplement_background_work(&self,request:&nomifun_robot::services::RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()> {
        let _connection=self.registry.hold_action_connection(&request.robot_id,&request.connection_id).await.ok_or_else(||anyhow::anyhow!("Robot supplement connection was replaced"))?;
        if !self.registry.get(&request.robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(request.companion_id.as_str())) {anyhow::bail!("Robot supplement binding was replaced");}
        self.sessions.supplement_robot_background_work(&self.owner_user_id,request,&robot_turn_idempotency_key(request),text,correction,input_id).await?;Ok(())
    }
    async fn ensure_companion_session(&self, companion_id: &str, robot_id: &str) -> anyhow::Result<String> {
        Ok(self.companions.ensure_session_for_role(companion_id,
            nomifun_companion::CompanionRole::Robot, Some(robot_id)).await?.conversation_id)
    }

    async fn dispatch(&self, request: nomifun_robot::services::RobotTurnRequest) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        if request.captured_context.is_none() {anyhow::bail!("Physical Robot input is missing its recording-time canonical context");}
        let key = robot_turn_idempotency_key(&request);
        let (tx, rx) = mpsc::channel(64);
        let backend = self.clone();
        self.bridge_workers.spawn(key, move |cancelled| async move {
            if let Err(error) = backend.run_device_turn(&request, cancelled, tx.clone()).await {
                let _ = tx.send(robot_stream_failure(error.to_string())).await;
            }
            // A business/provider failure remains in the voice result; only
            // abnormal task completion makes the local shutdown proof fail.
        })?;
        Ok(rx)
    }

    async fn cancel(&self, request: &nomifun_robot::services::RobotTurnRequest) -> anyhow::Result<()> {
        let key = robot_turn_idempotency_key(request);
        self.bridge_workers.cancel(&key)
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


/// The earlier output snapshot may precede the terminal commit observed by
/// the delivery reader. Refresh only this original key/cursor before deciding
/// that a completed answer lacks a source. The reader validates canonical
/// parts and Host terminal proof; neither a receipt's text nor a new model
/// invocation can fill missing output. Delivered prefixes remain deduplicated.
pub(crate) async fn completed_robot_public_delivery_events(
    sessions:&crate::router::nomi_core_session::NomiCoreSessionOwner,owner:&str,
    request:&nomifun_robot::services::RobotTurnRequest,key:&str,
    receipt:&nomifun_conversation::IdempotentMessageDelivery,after_seq:u64,
    delivered_bytes:&mut std::collections::BTreeMap<String,usize>,
) -> anyhow::Result<Vec<TurnEvent>> {
    let public=sessions.robot_public_output(owner,&request.conversation_id,key,after_seq,delivered_bytes).await?;
    let mut events=Vec::new();
    for (source,text) in public.segments {
        tracing::info!(agent_session_id=%request.conversation_id,request_id=%request.request_id,
            source_event_id=%source.message_id,through_seq=source.through_seq,"robot: canonical public source queued for delivery");
        events.push(TurnEvent::PublicSpeech {source,text});
    }
    // A terminal question is a formal public interaction even without an
    // answer. Its existing exact presentation-source checks remain required.
    if let Some(question)=receipt.user_input_request.as_ref() {
        if let Some((source,presentation_id,text))=sessions.robot_question_output(owner,&request.conversation_id,key,question).await? {
            events.push(TurnEvent::Question {source,text,presentation_id});
        }
        events.push(TurnEvent::Done);return Ok(events);
    }
    events.extend(if delivered_bytes.is_empty() {completed_robot_delivery_events(receipt)}
        else if receipt.result_ok==Some(true) {vec![TurnEvent::Done]}
        else {vec![robot_stream_failure(receipt.result_error.as_deref().unwrap_or("The robot Turn failed"))]});
    Ok(events)
}

fn completed_robot_delivery_events(
    delivery: &nomifun_conversation::IdempotentMessageDelivery,
) -> Vec<TurnEvent> {
    if delivery.user_input_request.is_some() {
        return vec![robot_stream_failure("Robot questions require their exact canonical presentation source")];
    }
    // A typed canonical silent delivery proves that this Turn intentionally
    // has no public reply. Device receipts remain in the canonical tool/effect
    // chain; silence does not claim that accepted motion physically finished.
    if delivery.result_ok == Some(true)
        && matches!(delivery.turn_outcome, Some(nomifun_agent_contracts::AgentTurnOutcome::Silent {}))
    {
        return vec![TurnEvent::Done];
    }
    if delivery.result_ok == Some(true) && !matches!(delivery.turn_outcome,
        Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { .. })) {
        return vec![robot_stream_failure("The canonical robot turn has no public answer")];
    }
    match delivery.result_ok {
        Some(true) => vec![robot_stream_failure("The answered robot Turn has no irrevocable public source")],
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

fn unavailable_robot_input_event(cause:&nomifun_conversation::UnavailableInputCause)->TurnEvent {
    match cause {
        nomifun_conversation::UnavailableInputCause::CancelledByOwner
        |nomifun_conversation::UnavailableInputCause::ContextCleared=>TurnEvent::Done,
        nomifun_conversation::UnavailableInputCause::PreparationBlocked {reason}=>
            robot_stream_failure(format!("Robot input preparation is blocked: {reason}")),
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
/// where the canonical AgentSession owner is fully assembled.
pub(crate) fn mount(
    robot: &Arc<RobotServices>,
    sessions: Arc<crate::router::nomi_core_session::NomiCoreSessionOwner>,
    companions: Arc<nomifun_companion::CompanionService>,
    owner_user_id: Arc<str>,
    _data_dir: PathBuf,
) -> RobotFaces {
    let backend = Arc::new(AppRobotBackend {
        sessions,
        companions,
        owner_user_id,
        registry: robot.registry.clone(),
        bridge_workers: robot.bridge_workers.clone(),
        queues: Arc::new(Mutex::new(HashMap::new())),
    });
    let delivery: Arc<dyn crate::router::nomi_core_session::ExecutionReportDeliveryPort> = backend.clone();
    backend.sessions.install_execution_report_delivery(&delivery)
        .expect("install Robot directed-report delivery owner");
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
    robot.set_gateway(gateway.clone());
    gateway.start(vec![source]);

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
    fn robot_delivery(outcome: nomifun_agent_contracts::AgentTurnOutcome) -> nomifun_conversation::IdempotentMessageDelivery {
        nomifun_conversation::IdempotentMessageDelivery {
            message_id: "robot-turn-root".into(), replayed: false, completed: true,
            turn_outcome: Some(outcome), result_ok: Some(true), result_text: None,
            result_error: None, result_error_code: None, result_error_retryable: None,
            user_input_request: None,
        }
    }

    #[test]
    fn canonical_silent_robot_delivery_finishes_without_speech_or_failure() {
        let mut delivery = robot_delivery(nomifun_agent_contracts::AgentTurnOutcome::Silent {});
        for replayed in [false, true] {
            delivery.replayed = replayed;
            assert_eq!(completed_robot_delivery_events(&delivery), vec![TurnEvent::Done]);
        }
        delivery.result_text = Some("stale optional display text must not become speech".into());
        assert_eq!(completed_robot_delivery_events(&delivery), vec![TurnEvent::Done], "typed delivery, not incidental text, selects silence");
    }

    #[test]
    fn unclaimed_input_lifecycle_ends_robot_observation_without_provider_retry() {
        use nomifun_conversation::UnavailableInputCause;
        for cause in [UnavailableInputCause::CancelledByOwner,UnavailableInputCause::ContextCleared] {
            assert_eq!(unavailable_robot_input_event(&cause),TurnEvent::Done);
        }
        assert_eq!(unavailable_robot_input_event(&UnavailableInputCause::PreparationBlocked {
            reason:"COMPONENT_PREPARATION_REQUIRED: speech binding unavailable".into(),
        }),TurnEvent::Failed {
            message:"Robot input preparation is blocked: COMPONENT_PREPARATION_REQUIRED: speech binding unavailable".into(),
            provider_fault:false,
        });
    }

    #[test]
    fn robot_delivery_keeps_questions_answers_and_failures_distinct_from_silence() {
        let mut delivery = robot_delivery(nomifun_agent_contracts::AgentTurnOutcome::Answered {
            message_ids: vec!["answer".into()],
        });
        delivery.result_text = Some("北京今天有小雨。".into());
        assert!(matches!(completed_robot_delivery_events(&delivery).as_slice(),[TurnEvent::Failed {provider_fault:false,..}]),"an aggregate text receipt cannot authorize playback without its committed source");
        delivery.turn_outcome = Some(nomifun_agent_contracts::AgentTurnOutcome::Unavailable);
        assert!(matches!(completed_robot_delivery_events(&delivery).as_slice(), [TurnEvent::Failed { provider_fault: false, .. }]));
        delivery.turn_outcome = Some(nomifun_agent_contracts::AgentTurnOutcome::Silent {});
        delivery.result_ok = Some(false);
        delivery.result_error = Some("device connection lost".into());
        assert_eq!(completed_robot_delivery_events(&delivery), vec![TurnEvent::Failed {
            message: "device connection lost".into(), provider_fault: false,
        }], "a typed silent intent cannot hide a failed canonical Turn");
    }

    fn companion_voice_request() -> nomifun_robot::services::RobotTurnRequest {
        nomifun_robot::services::RobotTurnRequest {
            robot_id: "robot-1".into(),
            companion_id: "companion-1".into(),
            conversation_id: "session-1".into(),
            connection_id: "connection-1".into(),
            request_id: "utterance-1".into(),
            text: "你好".into(),
            presented_question_id: None,
            captured_context:None,
        }
    }

    fn companion_voice_snapshot() -> nomifun_api_types::AgentResolvedSnapshot {
        serde_json::from_value(serde_json::json!({
            "preset_id": "0190f5fe-7c00-7a00-8000-0000000000aa",
            "preset_revision": 1,
            "preset_name": "Companion",
            "canonical_binding": {
                "preset_revision_ref": {
                    "preset_id": "0190f5fe-7c00-7a00-8000-0000000000aa",
                    "revision": 1, "revision_digest": "revision"
                },
                "resolved_snapshot_ref": {"snapshot_id": "snapshot", "snapshot_digest": "digest"},
                "typed_resource_bindings": [{
                    "binding_id": "companion", "resource_kind": "companion",
                    "resource_id": "companion-1", "owner_id": "owner-1", "operations": []
                }],
                "binding_version": 1
            }
        })).unwrap()
    }

    #[test]
    fn companion_voice_input_accepts_the_existing_session_without_device_or_vision_grants() {
        let snapshot = companion_voice_snapshot();
        let saved = snapshot.clone();
        assert!(snapshot.enabled_capability_actions.is_empty());
        assert!(snapshot_authorizes_companion_voice_input(
            Some(&snapshot), "owner-1", &companion_voice_request()
        ));
        assert_eq!(snapshot, saved, "accepting voice must not expand the frozen binding");
    }

    #[test]
    fn companion_voice_input_rejects_missing_foreign_and_ambiguous_companion_authority() {
        let request = companion_voice_request();
        assert!(!snapshot_authorizes_companion_voice_input(None, "owner-1", &request));
        for variant in 0..5 {
            let mut snapshot = companion_voice_snapshot();
            if variant == 0 {
                snapshot.canonical_binding = None;
            } else {
                let resources = &mut snapshot.canonical_binding.as_mut().unwrap().typed_resource_bindings;
                match variant {
                    1 => resources.clear(),
                    2 => resources[0].owner_id = "other-owner".into(),
                    3 => resources[0].resource_id = "other-companion".into(),
                    4 => resources.push(resources[0].clone()),
                    _ => unreachable!(),
                }
            }
            assert!(!snapshot_authorizes_companion_voice_input(
                Some(&snapshot), "owner-1", &request
            ), "variant {variant}");
        }
    }

    #[test]
    fn companion_voice_input_preserves_the_explicit_device_target() {
        let request = companion_voice_request();
        let mut snapshot = companion_voice_snapshot();
        let resources = &mut snapshot.canonical_binding.as_mut().unwrap().typed_resource_bindings;
        resources.push(nomifun_api_types::TypedResourceBindingDto {
            binding_id: "robot".into(), resource_kind: "robot".into(),
            resource_id: request.robot_id.clone(), owner_id: "owner-1".into(),
            operations: Default::default(), connection_config_ref: None,
            typed_parameters: [("companion_id".into(), request.companion_id.clone())].into(),
        });
        assert!(snapshot_authorizes_companion_voice_input(Some(&snapshot), "owner-1", &request));
        for variant in 0..5 {
            let mut changed = snapshot.clone();
            let resources = &mut changed.canonical_binding.as_mut().unwrap().typed_resource_bindings;
            match variant {
                0 => resources[1].resource_id = "other-robot".into(),
                1 => resources[1].owner_id = "other-owner".into(),
                2 => { resources[1].typed_parameters.insert("companion_id".into(), "other-companion".into()); }
                3 => resources[1].typed_parameters.clear(),
                4 => resources.push(resources[1].clone()),
                _ => unreachable!(),
            }
            assert!(!snapshot_authorizes_companion_voice_input(
                Some(&changed), "owner-1", &request
            ), "variant {variant}");
        }
    }

    #[tokio::test]
    async fn bridge_shutdown_keeps_worker_owned_after_waiter_drop() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }
        let workers = Arc::new(RobotBridgeWorkers::default());
        let cancelled = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        workers.spawn("same-request".into(), {
            let cancelled = cancelled.clone(); let release = release.clone(); let dropped = dropped.clone();
            move |token| async move {
                let _guard = Dropped(dropped);
                token.cancelled().await;
                cancelled.notify_one();
                release.acquire().await.unwrap().forget();
            }
        }).unwrap();
        let owner = workers.clone();
        let waiter = tokio::spawn(async move { owner.shutdown_and_wait().await });
        cancelled.notified().await;
        waiter.abort(); let _ = waiter.await;
        assert!(!dropped.load(Ordering::Acquire));
        assert!(workers.spawn("no-late-admission".into(), |_| async {}).is_err());
        release.add_permits(1);
        workers.shutdown_and_wait().await.unwrap();
        assert!(dropped.load(Ordering::Acquire));
        workers.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn bridge_shutdown_retains_panic_but_not_business_failure() {
        let workers = RobotBridgeWorkers::default();
        workers.spawn("completed-failed-voice-result".into(), |_| async {
            let business_result: anyhow::Result<()> = Err(anyhow::anyhow!("controlled provider failure"));
            assert!(business_result.is_err()); // The worker nevertheless finished normally.
        }).unwrap();
        workers.shutdown_and_wait().await.unwrap();
        let workers = RobotBridgeWorkers::default();
        workers.spawn("panic".into(), |_| async { panic!("controlled Robot bridge panic") }).unwrap();
        assert!(workers.shutdown_and_wait().await.is_err());
        assert!(workers.shutdown_and_wait().await.is_err());
    }
    use super::*;


}
