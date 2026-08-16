use std::sync::Arc;

use nomifun_ai_agent::{AgentStreamEvent, AgentRuntimeRegistry};
use nomifun_api_types::{
    ConversationRuntimeStateKind, CreateConversationRequest, ListMessagesQuery, MessageResponse, SendMessageRequest,
};
use nomifun_common::{AgentType, ConversationSource, MessagePosition, MessageType};
use nomifun_conversation::ConversationService;
use nomifun_db::IChannelRepository;
use nomifun_db::models::{
    CHANNEL_CHAT_KIND_DIRECT, CHANNEL_CHAT_KIND_GROUP, CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE,
    CHANNEL_USER_AUTHORIZATION_AUTO_GROUP, ChannelSessionRow,
};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::channel_settings::{ChannelSettingsService, resolved_model_to_provider};
use crate::error::ChannelError;
use crate::types::{OutgoingMessageType, PluginType, UnifiedOutgoingMessage};

/// 客服域接缝 (customer-service routing seam) — the channel layer's ONLY
/// contact point with the customer-service domain. Implemented in
/// `nomifun-app` over the customer-service crate; the channel crate stays
/// domain-agnostic.
///
/// `handle_visitor_message` contract: `Ok(reply)` is the final reply text to
/// send to the visitor — an EMPTY string means the message was merged into
/// another in-flight batch (同访客串行合并) and the caller must send nothing.
/// `Err(notice)` is a visitor-facing failure notice to send verbatim.
#[async_trait::async_trait] pub trait CsRouting: Send + Sync {
    async fn binding_for(&self, channel_plugin_id: &str) -> Option<String /*cs_agent_id*/>;
    async fn handle_visitor_message(&self, cs_agent_id: &str, channel_plugin_id: &str,
        channel_user_id: &str, chat_id: &str, text: &str) -> Result<String /*回复文本*/, String /*给访客的失败提示*/>;
}

/// Agent profile used by channel conversations. The channel layer
/// resolves which companion greets a session via the channel row's own `companion_id`
/// binding first, falling back to this profile (`channel_companion_id`: the
/// per-platform binding only — no default-companion fallback). `companion_model` is the **primary** model
/// source for a channel Nomi session bound to a companion (唯一事实源
/// `profile.model`); the platform `defaultModel` is only a fallback
/// when the companion has no model.
/// Implemented in `nomifun-app` over `CompanionService` + `ChannelSettingsService`
/// so the channel crate stays companion-agnostic.
#[async_trait::async_trait]
pub trait ChannelAgentProfile: Send + Sync {
    /// The configured model of `companion_id`, `None` when the companion does not
    /// exist or its model is not configured.
    async fn companion_model(&self, companion_id: &str) -> Option<nomifun_common::ProviderWithModel>;
    /// The per-platform fallback companion for `platform` (e.g. "telegram"):
    /// the platform binding when set (and alive), else `None`. There is **no
    /// default-companion fallback** — an unbound channel is hosted by no companion.
    async fn channel_companion_id(&self, platform: &str) -> Option<String>;
    /// Whether `companion_id` names a live companion. Used to validate companion-binding
    /// writes and to skip dead channel bindings.
    async fn companion_exists(&self, companion_id: &str) -> bool;

    /// Display name of `companion_id`, `None` when it does not exist. Used only to
    /// render a friendlier "already bound to companion …" error (name over raw id).
    /// Default `None` keeps companion-only / test impls unaffected.
    async fn companion_name(&self, _companion_id: &str) -> Option<String> {
        None
    }

    /// Idempotently resolve (create-or-get) the companion's ONE persistent
    /// session conversation id. This is what unifies a companion's IM-channel
    /// turns into the SAME session the desktop bubble and chat tab use, instead
    /// of minting a separate per-chat channel conversation. Returns
    /// `None` when the companion cannot host a session yet (e.g. its chat model
    /// is not configured) — the caller then refuses the turn with a notice
    /// rather than leaking an unintended per-chat conversation.
    async fn ensure_companion_session(&self, companion_id: &str) -> Option<String>;
}

/// Resolves a workshop asset UUIDv7 to raw bytes for outbound media.
///
/// Defined here (not in `nomifun-workshop`) so `nomifun-channel` stays free of
/// a workshop dependency; the concrete impl lives in `nomifun-app`
/// (`channel_asset_resolver.rs`), mirroring [`ChannelAgentProfile`].
#[async_trait::async_trait]
pub trait AssetResolver: Send + Sync {
    /// Load `asset_id` as bytes + mime + a suggested filename. `None` when the
    /// asset can't be found or read (the relay then simply skips that image).
    async fn resolve(&self, asset_id: &str) -> Option<crate::types::OutgoingMedia>;
}

/// Bridges channel messages to the conversation + AI agent layer.
///
/// Responsibilities:
/// - Creating conversations for channel sessions
/// - Sending user messages to the AI agent
/// - Receiving stream events and converting them to outgoing messages
/// - Throttling editMessage calls for streaming responses
/// - Handling tool confirmation with timeout
pub struct ChannelMessageService {
    conversation_svc: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    settings: Arc<ChannelSettingsService>,
    repo: Arc<dyn IChannelRepository>,
    owner_user_id: String,
    channel_agent_profile: Option<Arc<dyn ChannelAgentProfile>>,
    /// 客服域接缝: when wired, a bot bound to a customer-service agent has its
    /// inbound turns handed to the customer-service domain instead of any
    /// Conversation path. `None` (default / tests) means no customer-service
    /// domain is available and every bot follows the companion path.
    cs_routing: Option<Arc<dyn CsRouting>>,
    /// Per-conversation store of the decision currently awaiting a numbered
    /// reply. Shared with each `ChannelStreamRelay` (writer) so the inbound
    /// reply can be resolved against the right `call_id`/option.
    pending_decisions: Arc<crate::pending_decision::PendingDecisionStore>,
    /// Optional resolver turning workshop asset UUIDv7 ids into raw bytes for
    /// outbound media.
    /// `None` (default / tests) disables channel image sending gracefully.
    asset_resolver: Option<Arc<dyn AssetResolver>>,
}

impl ChannelMessageService {
    pub fn new(
        conversation_svc: Arc<ConversationService>,
        runtime_registry: Arc<dyn AgentRuntimeRegistry>,
        settings: Arc<ChannelSettingsService>,
        repo: Arc<dyn IChannelRepository>,
        owner_user_id: String,
    ) -> Self {
        Self {
            conversation_svc,
            runtime_registry,
            settings,
            repo,
            owner_user_id,
            channel_agent_profile: None,
            cs_routing: None,
            pending_decisions: crate::pending_decision::PendingDecisionStore::new(),
            asset_resolver: None,
        }
    }

    /// Wire the customer-service routing seam. Without it, no bot is treated
    /// as customer-service bound.
    pub fn with_cs_routing(mut self, routing: Arc<dyn CsRouting>) -> Self {
        self.cs_routing = Some(routing);
        self
    }

    /// The customer-service agent bound to `channel_plugin_id`, if the seam is
    /// wired, the bot belongs to the customer-service domain, and a binding
    /// exists. A stray binding on a companion bot is ignored. The message loop
    /// uses this to skip the conversation-based busy guard (客服域自己管并发).
    pub async fn cs_bound_agent(&self, channel_plugin_id: &str) -> Option<String> {
        let plugin = self.repo.get_plugin(channel_plugin_id).await.ok()??;
        if plugin.owner_domain != CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE {
            return None;
        }
        let routing = self.cs_routing.as_ref()?;
        routing.binding_for(channel_plugin_id).await
    }

    /// Hand one visitor message to the customer-service domain and return the
    /// reply text (`Ok("")` = merged into another in-flight batch → send
    /// nothing) or a visitor-facing failure notice.
    pub async fn cs_handle_visitor_message(
        &self,
        cs_agent_id: &str,
        channel_plugin_id: &str,
        channel_user_id: &str,
        chat_id: &str,
        text: &str,
    ) -> Result<String, String> {
        if self.cs_bound_agent(channel_plugin_id).await.as_deref() != Some(cs_agent_id) {
            return Err("customer-service bot binding is unavailable".to_owned());
        }
        let Some(routing) = self.cs_routing.as_ref() else {
            return Err("customer-service routing not configured".to_owned());
        };
        routing
            .handle_visitor_message(cs_agent_id, channel_plugin_id, channel_user_id, chat_id, text)
            .await
    }

    /// Wire the profile that resolves a channel's companion owner.
    /// Without it, channel conversations still receive their base Agent context,
    /// but owner-specific model and persona resolution are unavailable.
    pub fn with_channel_agent_profile(mut self, profile: Arc<dyn ChannelAgentProfile>) -> Self {
        self.channel_agent_profile = Some(profile);
        self
    }

    /// Wire the asset resolver so channel replies can send AI-generated images.
    /// Without it, image sending is disabled (text-only behaviour, unchanged).
    pub fn with_asset_resolver(mut self, resolver: Arc<dyn AssetResolver>) -> Self {
        self.asset_resolver = Some(resolver);
        self
    }

    /// The wired asset resolver, if any. The message loop hands this to each
    /// `ChannelStreamRelay` it spawns.
    pub fn asset_resolver(&self) -> Option<Arc<dyn AssetResolver>> {
        self.asset_resolver.clone()
    }

    /// The shared pending-decision store. The message loop hands this to each
    /// `ChannelStreamRelay` it spawns and reads it back when intercepting a
    /// numeric reply, so the relay (writer) and the message loop (reader) act
    /// on the same store.
    pub fn pending_decisions(&self) -> Arc<crate::pending_decision::PendingDecisionStore> {
        Arc::clone(&self.pending_decisions)
    }

    /// Installation owner used to namespace server-derived channel operation
    /// identities. This value never comes from the provider payload.
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    /// Whether the conversation's agent is currently working on a turn.
    ///
    /// Used by the message loop as a per-chat concurrency guard: a new
    /// channel message for a busy conversation is answered with a "still
    /// processing" notice instead of being queued as a second prompt.
    pub async fn is_conversation_busy(&self, conversation_id: &str) -> bool {
        let summary = self.conversation_svc.runtime_summary_for(conversation_id).await;
        matches!(
            summary.state,
            ConversationRuntimeStateKind::Starting | ConversationRuntimeStateKind::Running
        )
    }

    // ── Busy-time pending prompt queue (spec D1) ─────────────────────

    /// Persist a prompt that arrived while its conversation was busy. The
    /// queue drain delivers it FIFO once the running turn completes.
    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue_busy_prompt(
        &self,
        channel_plugin_id: &str,
        chat_id: &str,
        channel_session_id: &str,
        conversation_id: &str,
        text: &str,
        idempotency_key: &str,
    ) -> Result<nomifun_db::PendingPromptEnqueue, ChannelError> {
        let row = nomifun_db::models::NewChannelPendingPromptRow {
            channel_plugin_id: channel_plugin_id.to_owned(),
            chat_id: chat_id.to_owned(),
            channel_session_id: channel_session_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            text: text.to_owned(),
            idempotency_key: idempotency_key.to_owned(),
        };
        Ok(self
            .repo
            .enqueue_pending_prompt(&row, nomifun_common::now_ms())
            .await?)
    }

    /// Clear every queued prompt of one `(plugin, chat)` scope (the IM
    /// 「取消排队」 command). Returns how many prompts were cancelled.
    pub async fn cancel_chat_queue(
        &self,
        channel_plugin_id: &str,
        chat_id: &str,
    ) -> Result<u64, ChannelError> {
        Ok(self
            .repo
            .cancel_chat_queue(channel_plugin_id, chat_id, nomifun_common::now_ms())
            .await?)
    }

    /// Read-only durable outcome of one keyed public turn — the drain's
    /// retryable-failure classifier (spec D4 fields from batch 1).
    pub async fn turn_outcome(
        &self,
        conversation_id: &str,
        idempotency_key: &str,
    ) -> Result<nomifun_conversation::PublicTurnDeliveryState, ChannelError> {
        self.conversation_svc
            .public_turn_delivery_state(&self.owner_user_id, conversation_id, idempotency_key)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(e.to_string()))
    }

    /// Remote unlock (batch-1 handover gap): stop a conversation's current
    /// turn as the installation owner after the channel user confirmed the
    /// numbered stop decision. Same safe service path as the desktop stop
    /// button (`POST /api/conversations/{id}/cancel`); deliberately NOT the
    /// gateway matrix, which denies Destructive on the Channel surface.
    pub async fn stop_conversation(&self, conversation_id: &str) -> Result<(), ChannelError> {
        self.conversation_svc
            .cancel(&self.owner_user_id, conversation_id, &self.runtime_registry)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(e.to_string()))
    }

    /// Submits a numbered-decision choice back through the confirm chain.
    ///
    /// `option_id` is sent as the bare `data` string accepted by
    /// `ConversationService::confirm` for ACP (`msg_id` is ignored there).
    /// `always_allow` is `false` — a numbered reply approves this one decision
    /// only, never a standing grant.
    pub async fn submit_decision(
        &self,
        conversation_id: &str,
        call_id: &str,
        option_id: &str,
    ) -> Result<(), ChannelError> {
        let req = nomifun_api_types::ConfirmRequest {
            msg_id: String::new(),
            data: serde_json::Value::String(option_id.to_owned()),
            always_allow: false,
        };
        self.conversation_svc
            .confirm(&self.owner_user_id, conversation_id, call_id, req, &self.runtime_registry)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(e.to_string()))
    }

    /// Returns the most recent visible user message text of a conversation,
    /// used by `chat.regenerate` to resend the last prompt.
    ///
    /// Reads a single newest-first page; user turns alternate with assistant
    /// output, so 50 rows is far more than enough to reach the latest one.
    pub async fn last_user_text(&self, conversation_id: &str) -> Result<Option<String>, ChannelError> {
        let query = ListMessagesQuery {
            page: Some(1),
            page_size: Some(50),
            order: Some("DESC".into()),
            content_mode: None,
            cursor: None,
            day: None,
        };
        let result = self
            .conversation_svc
            .list_messages(&self.owner_user_id, conversation_id, query)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(e.to_string()))?;
        Ok(extract_last_user_text(&result.items))
    }

    /// Sends a text message from a channel user to the AI agent.
    ///
    /// 1. Ensures the session has a backing conversation (creates one if needed)
    /// 2. Warms up the backing Agent runtime so stream subscription is available
    /// 3. Sends the message via ConversationService
    /// 4. Returns the conversation_id and stream receiver for relay
    ///
    /// The caller is responsible for subscribing to stream events and
    /// relaying them to the IM platform.
    pub async fn send_to_agent(
        &self,
        session: &ChannelSessionRow,
        text: &str,
        platform: PluginType,
        idempotency_key: &str,
    ) -> Result<SendResult, ChannelError> {
        // 客服接缝 (defensive): a bot bound to a customer-service agent is
        // routed in the message loop BEFORE this method — its turns must never
        // create or touch a Conversation. If a caller still lands here with a
        // bound bot, refuse instead of leaking a conversation.
        if let Some(channel_plugin_id) = session.channel_plugin_id.as_deref() {
            let plugin = self
                .repo
                .get_plugin(channel_plugin_id)
                .await?
                .ok_or_else(|| ChannelError::PluginNotFound(channel_plugin_id.to_owned()))?;
            if plugin.owner_domain == CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE {
                return Err(ChannelError::MessageSendFailed(
                    "customer-service bot must not enter the conversation path".into(),
                ));
            }
        }

        // Admission normally rejects ambiguous provider events before a
        // session can dispatch. Keep the same fail-closed boundary here for
        // stale queue rows and internal callers that bypass admission: an
        // unknown scope must never be treated as direct or gain a fallback
        // dedicated conversation. Plugin identity is validated first above so
        // every scoped session also fails closed when its bot row is missing.
        if session.chat_kind != CHANNEL_CHAT_KIND_DIRECT
            && session.chat_kind != CHANNEL_CHAT_KIND_GROUP
        {
            return Err(ChannelError::UserNotAuthorized(
                "channel chat kind is unknown; dispatch refused".into(),
            ));
        }

        // Resolve the target conversation. A DIRECT Nomi turn bound to a
        // companion uses that companion's one private persistent session.
        // Every GROUP turn is forced into a dedicated channel conversation so
        // group members can never read or pollute the owner's private transcript.
        // Non-companion / ACP / unbound channels are dedicated as before.
        let agent_type = parse_agent_type(&session.agent_type)?;
        let is_direct = session.chat_kind == CHANNEL_CHAT_KIND_DIRECT;
        let is_group = session.chat_kind == CHANNEL_CHAT_KIND_GROUP;
        let auto_group_guest = if is_group {
            let user = self
                .repo
                .get_user(&session.channel_user_id)
                .await?
                .ok_or_else(|| ChannelError::UserNotFound(session.channel_user_id.clone()))?;
            user.authorization_kind == CHANNEL_USER_AUTHORIZATION_AUTO_GROUP
        } else {
            false
        };
        if auto_group_guest && agent_type != AgentType::Nomi {
            return Err(ChannelError::UserNotAuthorized(
                "open-group guests may only use the restricted Nomi agent".into(),
            ));
        }
        let companion_id = if agent_type == AgentType::Nomi {
            self.resolve_session_companion(session, platform).await
        } else {
            None
        };
        let uses_shared_companion_session = is_direct && companion_id.is_some();
        let conversation_id = if uses_shared_companion_session {
            let cid = companion_id
                .as_deref()
                .expect("shared companion session requires a companion id");
            match self.channel_agent_profile.as_ref() {
                Some(profile) => match profile.ensure_companion_session(cid).await {
                    Some(id) => id,
                    // Companion bound but no chat model → can't open its single
                    // session. Refuse with a notice instead of silently minting a
                    // leaking an unintended channel conversation (reintroducing the bug).
                    None => {
                        return Err(ChannelError::CompanionNotReady(
                            "这个伙伴还没有配置对话模型，请先在桌面端为它选择模型后再聊天。".into(),
                        ));
                    }
                },
                None => {
                    return Err(ChannelError::MessageSendFailed(
                        "channel agent profile not configured".into(),
                    ));
                }
            }
        } else {
            match self
                .reusable_session_conversation(session, auto_group_guest)
                .await?
            {
                Some(cid) => cid,
                None => {
                    self.create_conversation_for_session(session, platform, auto_group_guest)
                        .await?
                }
            }
        };

        // Tag this turn with its origin platform ONLY when it rides a
        // companion's shared single session (companion_id resolved): that
        // conversation row carries no `channel_platform`, so the per-turn marker
        // is what lets the floating window render it as a remote IM turn.
        // Dedicated per-chat channel conversations keep their extra-derived marker
        // (marker None → send_message falls back to extra).
        let channel_platform = uses_shared_companion_session.then(|| platform.to_string());
        self.dispatch_to_conversation(
            &session.channel_session_id,
            conversation_id,
            text,
            channel_platform,
            idempotency_key,
        )
            .await
    }

    /// Warms the conversation's agent, subscribes to its stream, and sends the
    /// user turn. Shared by the companion and per-chat paths — the only
    /// per-path difference is the `channel_platform` per-turn marker
    /// (companion shared session ⇒ platform; per-chat ⇒ None, the marker
    /// rides the conversation extra).
    async fn dispatch_to_conversation(
        &self,
        session_id: &str,
        conversation_id: String,
        text: &str,
        channel_platform: Option<String>,
        idempotency_key: &str,
    ) -> Result<SendResult, ChannelError> {
        // `msg_id` is server-generated inside the service; channel plugins that
        // need to correlate the user message back to the conversation should use
        // `conversation_id` + stream events instead of a client-provided id.
        let req = SendMessageRequest {
            content: text.to_owned(),
            files: vec![],
            inject_skills: vec![],
            hidden: false,
            origin: None,
            channel_platform,
        };

        let user_id = &self.owner_user_id;
        // The keyed send claims its durable Conversation receipt and turn
        // admission under the shared preparation gate before the background
        // runtime build. That gate is also the only safe place to recover a
        // pre-admission edit reservation; an observer-only precheck here used
        // to make that crash cutpoint permanently unrecoverable.
        let delivery = match self
            .conversation_svc
            .send_message_with_idempotency_key(
                user_id,
                &conversation_id,
                idempotency_key,
                req,
                &self.runtime_registry,
            )
            .await
        {
            Ok(delivery) => delivery,
            // A Conflict is only "please wait" when the (now shared) session is
            // actually working a turn — the turn-claim race the per-chat busy
            // guard can't see (it checks the pre-bind session id). A Conflict on
            // an IDLE conversation is a real failure (e.g. a knowledge workspace
            // lease clash or an idempotency-key reuse) and must reach the user;
            // disguising it as busy traps the chat in a permanent "still being
            // processed" loop.
            Err(error @ nomifun_common::AppError::Conflict(_)) => {
                if self.is_conversation_busy(&conversation_id).await {
                    return Err(ChannelError::ConversationBusy(conversation_id));
                }
                return Err(ChannelError::MessageSendFailed(error.to_string()));
            }
            Err(other) => return Err(ChannelError::MessageSendFailed(other.to_string())),
        };
        let message_id = delivery.message_id;

        // `send_message_with_idempotency_key` admits synchronously but builds a
        // cold runtime in its owned background task. Attach as soon as the
        // registered runtime appears; registration happens before prompt
        // dispatch. A timeout degrades streaming only and never retries the
        // model turn.
        let stream_rx = if delivery.completed {
            None
        } else {
            wait_for_runtime_subscription(
                &self.runtime_registry,
                &conversation_id,
            )
            .await
        };

        info!(
            conversation_id = %conversation_id,
            session_id = %session_id,
            has_stream = stream_rx.is_some(),
            "message sent to agent"
        );

        Ok(SendResult {
            conversation_id,
            message_id,
            stream_rx,
        })
    }

    /// A group session may only reuse a conversation created for that exact
    /// group chat and authorization tier. This is a defensive backstop for
    /// legacy/stale bindings: a row that still points at the companion owner's
    /// private conversation, or an unrestricted conversation attached to an
    /// `auto_group` identity, is never dispatched into.
    async fn reusable_session_conversation(
        &self,
        session: &ChannelSessionRow,
        auto_group_guest: bool,
    ) -> Result<Option<String>, ChannelError> {
        let Some(conversation_id) = session.conversation_id.as_deref() else {
            return Ok(None);
        };
        if session.chat_kind == CHANNEL_CHAT_KIND_DIRECT {
            return Ok(Some(conversation_id.to_owned()));
        }
        if session.chat_kind != CHANNEL_CHAT_KIND_GROUP {
            warn!(
                channel_session_id = %session.channel_session_id,
                conversation_id,
                "discarding conversation binding for unknown chat kind"
            );
            return Ok(None);
        }

        let conversation = match self
            .conversation_svc
            .get(&self.owner_user_id, conversation_id)
            .await
        {
            Ok(conversation) => conversation,
            Err(nomifun_common::AppError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(ChannelError::MessageSendFailed(error.to_string())),
        };
        let belongs_to_group = session.chat_id.is_some()
            && conversation.channel_chat_id.as_deref() == session.chat_id.as_deref();
        let is_group_guest = conversation
            .extra
            .get("channel_group_guest")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !belongs_to_group || is_group_guest != auto_group_guest {
            warn!(
                channel_session_id = %session.channel_session_id,
                conversation_id,
                belongs_to_group,
                expected_group_guest = auto_group_guest,
                actual_group_guest = is_group_guest,
                "discarding unsafe or stale group conversation binding"
            );
            return Ok(None);
        }

        Ok(Some(conversation_id.to_owned()))
    }

    /// Creates a new conversation for a channel session.
    ///
    /// Sets `source` to the appropriate platform and `channel_chat_id`
    /// for per-chat isolation.
    async fn create_conversation_for_session(
        &self,
        session: &ChannelSessionRow,
        platform: PluginType,
        auto_group_guest: bool,
    ) -> Result<String, ChannelError> {
        let source = platform_to_source(platform);
        let agent_type = parse_agent_type(&session.agent_type)?;

        let agent_config = self.settings.get_agent_config(platform).await?;
        let model_config = self.settings.get_model_config(platform).await?;

        // The companion greeting this session. Resolution order: the channel
        // row's own companion binding (per-bot, the multi-bot path) > the
        // per-platform binding. NO default-companion fallback — an unbound channel
        // is hosted by no companion. Recorded in extra.companion_id so the
        // persona/memory layers and gateway tools know which companion owns the
        // session. Companion persona and memory context apply to Nomi only; every
        // channel session receives the base channel Agent context.
        let channel_companion_id = if agent_type == AgentType::Nomi {
            self.resolve_session_companion(session, platform).await
        } else {
            None
        };

        // Model resolution for a companion-owned Nomi conversation prefers
        // the companion profile, then the platform default. ACP CLIs own their
        // model configuration and only receive a platform model when present.
        let mut model = if agent_type == AgentType::Nomi
            && let Some(profile) = self.channel_agent_profile.as_ref()
            && let Some(companion_id) = channel_companion_id.as_deref()
            && let Some(companion_model) = profile.companion_model(companion_id).await
        {
            Some(companion_model)
        } else {
            resolved_model_to_provider(model_config.as_ref())
        };

        if model.is_none() {
            model = resolved_model_to_provider(model_config.as_ref());
        }

        let mut extra = Self::build_channel_extra(agent_config.backend.as_deref());
        if auto_group_guest {
            extra["channel_group_guest"] = serde_json::Value::Bool(true);
        }
        apply_channel_agent_context(&mut extra, agent_type, platform, channel_companion_id.as_deref());
        let name = channel_conversation_name(
            platform,
            &session.agent_type,
            agent_config.backend.as_deref(),
            session.chat_id.as_deref(),
        );

        // Top-level `model` is only accepted for nomi; other types pass via `extra`.
        let top_level_model = if agent_type == AgentType::Nomi {
            model
        } else {
            if let Some(model) = model {
                extra["model"] = serde_json::to_value(model).map_err(|error| {
                    ChannelError::MessageSendFailed(format!(
                        "failed to serialize channel model configuration: {error}"
                    ))
                })?;
            }
            None
        };

        let req = CreateConversationRequest {
            r#type: agent_type,
            name: Some(name),
            model: top_level_model,
            source: Some(source),
            channel_chat_id: session.chat_id.clone(),
            preset_id: None,
            preset_overrides: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        };

        let creation_scope = if session.chat_kind == CHANNEL_CHAT_KIND_GROUP {
            if auto_group_guest {
                "dedicated-group-guest"
            } else {
                "dedicated-group-approved"
            }
        } else {
            "dedicated"
        };
        let creation_key = channel_creation_key(&self.owner_user_id, session, creation_scope);
        let response = self
            .conversation_svc
            .create_idempotent(&self.owner_user_id, req, &creation_key)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(e.to_string()))?;

        debug!(
            conversation_id = %response.conversation_id,
            channel_session_id = %session.channel_session_id,
            "conversation created for channel session"
        );

        Ok(response.conversation_id)
    }

    /// Resolves which companion greets a channel session.
    ///
    /// The channel row's `companion_id` wins (each bot is bound to its own companion);
    /// a dead binding degrades to the profile fallback instead of pinning
    /// sessions to a ghost. Without a channel binding, the profile resolves
    /// the per-platform binding only (no default-companion fallback).
    async fn resolve_session_companion(&self, session: &ChannelSessionRow, platform: PluginType) -> Option<String> {
        let profile = self.channel_agent_profile.as_ref()?;

        if let Some(channel_plugin_id) = session.channel_plugin_id.as_deref() {
            match self.repo.get_plugin(channel_plugin_id).await {
                Ok(Some(row)) => {
                    if let Some(companion_id) = row.companion_id.filter(|p| !p.trim().is_empty()) {
                        if profile.companion_exists(&companion_id).await {
                            return Some(companion_id);
                        }
                        warn!(
                            channel_plugin_id,
                            companion_id = %companion_id,
                            "channel companion binding names a missing companion; falling back"
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => warn!(
                    channel_plugin_id,
                    error = %e,
                    "failed to load channel row for companion resolution"
                ),
            }
        }

        profile.channel_companion_id(&platform.to_string()).await
    }

    /// Processes a stream event from the AI agent and converts it to
    /// an optional outgoing message for the IM platform.
    ///
    /// Returns `None` for events that don't need to be sent to the user
    /// (e.g., internal status updates, thinking traces).
    pub fn process_stream_event(event: &AgentStreamEvent) -> Option<StreamAction> {
        match event {
            AgentStreamEvent::Text(data) => Some(StreamAction::AppendText(data.content.clone())),
            AgentStreamEvent::Finish(data)
                if matches!(
                    data.stop_reason,
                    None
                        | Some(
                            nomifun_ai_agent::protocol::events::TurnStopReason::EndTurn
                        )
                ) =>
            {
                Some(StreamAction::Finish)
            }
            AgentStreamEvent::Finish(data) => Some(StreamAction::Error(format!(
                "The turn ended before its requested output was completed: {}",
                match data.stop_reason {
                    Some(nomifun_ai_agent::protocol::events::TurnStopReason::MaxTokens) =>
                        "maximum output tokens reached",
                    Some(
                        nomifun_ai_agent::protocol::events::TurnStopReason::MaxTurnRequests
                    ) => "maximum tool requests reached",
                    Some(nomifun_ai_agent::protocol::events::TurnStopReason::Refusal) =>
                        "the model refused the request",
                    Some(nomifun_ai_agent::protocol::events::TurnStopReason::Cancelled) =>
                        "the turn was cancelled",
                    Some(nomifun_ai_agent::protocol::events::TurnStopReason::EndTurn) | None =>
                        unreachable!("normal finish was handled above"),
                }
            ))),
            AgentStreamEvent::Error(data) => Some(StreamAction::Error(data.message.clone())),
            AgentStreamEvent::Thinking(data) => Some(StreamAction::Thinking(data.content.clone())),
            AgentStreamEvent::ToolCall(data) => {
                // Remote unlock (batch-1 handover gap): the gateway matrix
                // denies nomi_stop_conversation on the Channel surface, so the
                // channel takes over with its own numbered confirmation.
                if let Some(target) = stop_denied_target(data) {
                    return Some(StreamAction::StopDenied {
                        target_conversation_id: target,
                    });
                }
                // Verified Nomi/ACP artifacts are already durable workspace
                // files. Preserve their receipts so the relay can upload the
                // bytes (or explicitly report the path if upload/read fails).
                if matches!(
                    data.status,
                    nomifun_ai_agent::protocol::events::ToolCallStatus::Completed
                ) && !data.artifacts.is_empty()
                {
                    return Some(StreamAction::ArtifactsProduced(data.artifacts.clone()));
                }
                // A completed tool call may carry produced workshop asset ids in
                // its output JSON (nomi_workshop_get_task/generate `result_asset_ids`).
                // Surface those as MediaProduced so the relay can send the picture;
                // otherwise keep the cosmetic {name,status} progress update.
                if matches!(
                    data.status,
                    nomifun_ai_agent::protocol::events::ToolCallStatus::Completed
                ) && let Some(output) = data.output.as_deref()
                {
                    let ids = crate::media_refs::asset_ids_from_tool_output(output);
                    if !ids.is_empty() {
                        return Some(StreamAction::MediaProduced(ids));
                    }
                }
                Some(StreamAction::ToolCall {
                    name: data.name.clone(),
                    status: format!("{:?}", data.status),
                })
            }
            AgentStreamEvent::AcpToolCall(data) => {
                if data.update.status
                    != Some(nomifun_ai_agent::protocol::events::AcpToolCallStatus::Completed)
                {
                    return None;
                }
                let artifacts = data
                    .update
                    .content
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .filter_map(|item| match item {
                        nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                            artifact,
                            ..
                        } => Some(artifact.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                (!artifacts.is_empty()).then_some(StreamAction::ArtifactsProduced(artifacts))
            }
            // Blocking decisions: forward as a numbered text choice. A decision
            // with no options is unanswerable, so it is dropped (None).
            AgentStreamEvent::AcpPermission(data) => match data {
                nomifun_ai_agent::protocol::events::AcpPermissionEventData::Request(req) => {
                    let options: Vec<crate::types::DecisionOption> = req
                        .options
                        .iter()
                        .map(|o| crate::types::DecisionOption {
                            option_id: o.option_id.clone(),
                            label: o.name.clone(),
                        })
                        .collect();
                    if options.is_empty() {
                        return None;
                    }
                    Some(StreamAction::Decision {
                        call_id: req.tool_call.tool_call_id.clone(),
                        prompt: req
                            .tool_call
                            .title
                            .clone()
                            .unwrap_or_else(|| "请选择".to_owned()),
                        options,
                    })
                }
                nomifun_ai_agent::protocol::events::AcpPermissionEventData::Confirmation(conf) => {
                    confirmation_to_decision(conf)
                }
            },
            AgentStreamEvent::Permission(value) => serde_json::from_value::<nomifun_common::Confirmation>(value.clone())
                .ok()
                .and_then(|conf| confirmation_to_decision(&conf)),
            // Events that don't produce user-facing messages
            AgentStreamEvent::Start(_)
            | AgentStreamEvent::Tips(_)
            | AgentStreamEvent::ToolGroup(_)
            | AgentStreamEvent::AgentStatus(_)
            | AgentStreamEvent::Plan(_)
            | AgentStreamEvent::AvailableCommands(_)
            | AgentStreamEvent::SkillSuggest(_)
            | AgentStreamEvent::CronTrigger(_)
            | AgentStreamEvent::AcpModelInfo(_)
            | AgentStreamEvent::AcpModeInfo(_)
            | AgentStreamEvent::AcpConfigOption(_)
            | AgentStreamEvent::AcpSessionInfo(_)
            | AgentStreamEvent::AcpContextUsage(_)
            | AgentStreamEvent::TurnCompleted(_)
            | AgentStreamEvent::System(_)
            | AgentStreamEvent::RequestTrace(_)
            | AgentStreamEvent::SlashCommandsUpdated(_)
            | AgentStreamEvent::SessionAssigned(_) => None,
        }
    }

    /// Builds the "thinking" placeholder message sent immediately after
    /// receiving a user message, before the AI starts streaming.
    pub fn build_thinking_message() -> UnifiedOutgoingMessage {
        UnifiedOutgoingMessage {
            message_type: OutgoingMessageType::Text,
            text: Some("\u{23f3} Thinking...".into()),
            parse_mode: None,
            buttons: None,
            keyboard: None,
            image_url: None,
            file_url: None,
            file_name: None,
            media_actions: None,
            reply_to_message_id: None,
            silent: None,
        }
    }

    /// Builds the final message after streaming completes.
    ///
    /// The `Buttons` message type is retained purely as the "final turn" marker
    /// that channels key off to finalize a streaming card (e.g. DingTalk flips
    /// its AI Card to FINISHED on this type). No action buttons are attached:
    /// the Regenerate / Continue / New Session affordances were removed because
    /// they cluttered the reply and hurt readability across IM channels.
    pub fn build_final_message(text: &str) -> UnifiedOutgoingMessage {
        UnifiedOutgoingMessage {
            message_type: OutgoingMessageType::Buttons,
            text: Some(text.to_owned()),
            parse_mode: None,
            buttons: None,
            keyboard: None,
            image_url: None,
            file_url: None,
            file_name: None,
            media_actions: None,
            reply_to_message_id: None,
            silent: None,
        }
    }

    /// Builds an intermediate streaming message (for editMessage calls).
    pub fn build_streaming_message(text: &str) -> UnifiedOutgoingMessage {
        UnifiedOutgoingMessage {
            message_type: OutgoingMessageType::Text,
            text: Some(text.to_owned()),
            parse_mode: None,
            buttons: None,
            keyboard: None,
            image_url: None,
            file_url: None,
            file_name: None,
            media_actions: None,
            reply_to_message_id: None,
            silent: None,
        }
    }

    /// Builds the numbered-text rendering of a blocking decision.
    ///
    /// Portable across channels (no card-button dependency): the prompt, a
    /// numbered list of option labels, and an instruction to reply with the
    /// number. Plain `Text` with no buttons.
    pub fn build_decision_message(prompt: &str, options: &[crate::types::DecisionOption]) -> UnifiedOutgoingMessage {
        let mut text = format!("\u{26a0}\u{fe0f} 需要你的决策：\n{prompt}\n");
        for (idx, option) in options.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", idx + 1, option.label));
        }
        text.push_str("回复编号选择（如 1）");

        UnifiedOutgoingMessage {
            message_type: OutgoingMessageType::Text,
            text: Some(text),
            parse_mode: None,
            buttons: None,
            keyboard: None,
            image_url: None,
            file_url: None,
            file_name: None,
            media_actions: None,
            reply_to_message_id: None,
            silent: None,
        }
    }

    /// Build the `extra` JSON for channel conversations.
    ///
    /// Sets `session_mode` to `"yolo"` so the agent auto-approves tool calls —
    /// channel users have no interactive UI for confirmations.
    pub fn build_channel_extra(backend: Option<&str>) -> serde_json::Value {
        let mut extra = serde_json::json!({
            "session_mode": "yolo",
        });
        if let Some(b) = backend {
            extra["backend"] = serde_json::Value::String(b.to_owned());
        }
        extra
    }

}

fn channel_creation_key(
    owner_user_id: &str,
    session: &ChannelSessionRow,
    purpose: &str,
) -> String {
    let scope = serde_json::json!([
        owner_user_id,
        session.channel_plugin_id.as_deref().unwrap_or(""),
        session.channel_session_id.as_str(),
        session.agent_type.as_str(),
        purpose,
    ]);
    let digest = Sha256::digest(
        serde_json::to_vec(&scope).expect("channel creation scope always serializes"),
    );
    format!("channel-session:v1:{digest:x}")
}

async fn wait_for_runtime_subscription(
    runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
    conversation_id: &str,
) -> Option<broadcast::Receiver<AgentStreamEvent>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(handle) = runtime_registry.get_runtime(conversation_id) {
            return Some(handle.subscribe());
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                conversation_id,
                "runtime did not register before channel relay subscription timeout"
            );
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Result of sending a message to the agent.
#[derive(Debug)]
pub struct SendResult {
    pub conversation_id: String,
    /// Canonical user message ID owned by the Conversation delivery receipt.
    pub message_id: String,
    /// Agent event stream for the ChannelStreamRelay.
    /// `None` when the Agent runtime could not be found after sending
    /// (should not happen in normal flow).
    pub stream_rx: Option<broadcast::Receiver<AgentStreamEvent>>,
}

/// Actions derived from agent stream events.
#[derive(Debug, Clone)]
pub enum StreamAction {
    /// Append text content to the current response.
    AppendText(String),
    /// Streaming finished.
    Finish,
    /// An error occurred.
    Error(String),
    /// Agent is thinking/reasoning.
    Thinking(String),
    /// Tool call status update.
    ToolCall { name: String, status: String },
    /// A blocking decision (permission / confirmation) the channel user must
    /// answer. Carried so the relay can forward a numbered list and the
    /// message loop can map a numeric reply back to `confirm`.
    Decision {
        call_id: String,
        prompt: String,
        options: Vec<crate::types::DecisionOption>,
    },
    /// The companion's `nomi_stop_conversation` was denied by the gateway
    /// matrix on the Channel surface (batch-1 handover gap). The relay
    /// forwards the channel-owned numbered stop confirmation; on "确认" the
    /// message loop cancels the target as owner.
    StopDenied {
        target_conversation_id: String,
    },
    /// One or more workshop asset ids produced by a *completed* tool call
    /// (e.g. `nomi_workshop_get_task` `result_asset_ids`). The relay resolves
    /// each to bytes and sends it as media after the final text.
    MediaProduced(Vec<String>),
    /// Verified workspace artifacts attached directly to a completed tool
    /// result. Unlike workshop ids these need no resolver: the receipt carries
    /// the canonical path, MIME and integrity metadata.
    ArtifactsProduced(Vec<nomifun_ai_agent::artifact_store::PersistedArtifact>),
}

/// Decorate a channel conversation with its persisted Agent context. Gateway
/// authorization is deliberately absent here: the factory injects its
/// process-owned capability only after validating runtime ownership. On the Nomi
/// engine this function adds companion semantics — persona system prompt (built fresh per agent build by the factory's
/// `CompanionPromptProvider`) + memory tools (`extra.companion_session`), with the
/// platform recorded for the persona's remote-context framing and the bound
/// companion pinned in `extra.companion_id` (per-bot binding > platform binding;
/// key omitted when no companion is bound — the session then has no companion persona).
///
/// This context is unconditional for channel sessions; it is not a separate
/// product mode or runtime type.
fn apply_channel_agent_context(
    extra: &mut serde_json::Value,
    agent_type: AgentType,
    platform: PluginType,
    companion_id: Option<&str>,
) {
    if agent_type == AgentType::Nomi {
        extra["companion_session"] = serde_json::Value::Bool(true);
        extra["channel_platform"] = serde_json::Value::String(platform.to_string());
        if let Some(pid) = companion_id.map(str::trim).filter(|s| !s.is_empty()) {
            extra["companion_id"] = serde_json::Value::String(pid.to_owned());
        }
    }
}

/// Detects a gateway-denied `nomi_stop_conversation` tool call and extracts
/// its target conversation id.
///
/// On the Channel surface the gateway matrix denies Destructive capabilities,
/// answering either `'…' is not permitted on the Channel surface` (dispatch
/// deny) or `session_capability_denied` (visibility filter). Both shapes ride
/// the tool-call OUTPUT of a terminal event; the target id comes from the
/// model-provided arguments (`args`, falling back to the raw `input` JSON).
fn stop_denied_target(
    data: &nomifun_ai_agent::protocol::events::ToolCallEventData,
) -> Option<String> {
    use nomifun_ai_agent::protocol::events::ToolCallStatus;
    if !matches!(data.status, ToolCallStatus::Completed | ToolCallStatus::Error) {
        return None;
    }
    if !data.name.ends_with("nomi_stop_conversation") {
        return None;
    }
    let output = data.output.as_deref()?;
    if !(output.contains("not permitted on the") || output.contains("session_capability_denied")) {
        return None;
    }
    let from_args = data
        .args
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    from_args.or_else(|| {
        data.input
            .as_ref()
            .and_then(|value| value.get("conversation_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

/// Maps a `nomifun_common::Confirmation` to a `Decision` action.
///
/// Option values become option ids (ACP `confirm` accepts a bare option-id
/// string). A confirmation with no options is unanswerable and yields `None`.
fn confirmation_to_decision(conf: &nomifun_common::Confirmation) -> Option<StreamAction> {
    let options: Vec<crate::types::DecisionOption> = conf
        .options
        .iter()
        .map(|o| crate::types::DecisionOption {
            option_id: option_value_to_string(&o.value),
            label: o.label.clone(),
        })
        .collect();
    if options.is_empty() {
        return None;
    }
    Some(StreamAction::Decision {
        call_id: conf.call_id.clone(),
        prompt: conf.title.clone().unwrap_or_else(|| conf.description.clone()),
        options,
    })
}

/// Renders a confirmation option value as the option id string to submit
/// back through `confirm`. String values pass through verbatim; other JSON
/// values fall back to their compact serialization.
fn option_value_to_string(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// Picks the newest visible user-authored text from a newest-first message
/// page. User messages are persisted as `type: "text"`, `position: "right"`
/// with content `{"content": "..."}` (see `ConversationService::send_message`),
/// so this is the inverse of that write path.
fn extract_last_user_text(items: &[MessageResponse]) -> Option<String> {
    items
        .iter()
        .filter(|m| !m.hidden && m.r#type == MessageType::Text && m.position == Some(MessagePosition::Right))
        .find_map(|m| {
            m.content
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
}

/// Maps a PluginType to the corresponding ConversationSource.
fn platform_to_source(platform: PluginType) -> ConversationSource {
    match platform {
        PluginType::Telegram => ConversationSource::Telegram,
        PluginType::Lark => ConversationSource::Lark,
        PluginType::Dingtalk => ConversationSource::Dingtalk,
        PluginType::Weixin => ConversationSource::Weixin,
        // Reserved / new outbound channels default to Nomi source until a
        // dedicated ConversationSource variant is added per channel phase.
        PluginType::Slack
        | PluginType::Discord
        | PluginType::Matrix
        | PluginType::Mattermost
        | PluginType::Twitch
        | PluginType::Nostr
        | PluginType::Wecom
        | PluginType::Qqbot => ConversationSource::Nomifun,
    }
}

/// Parses an `agent_type` string from a persisted channel session.
///
/// Rejects anything that is not a live engine. This column is free-form TEXT,
/// so a session bound to a retired engine is still readable — coercing it to a
/// surviving engine would resurrect it with the wrong runtime and the wrong
/// `extra` shape, failing much later and far from the cause.
fn parse_agent_type(s: &str) -> Result<AgentType, ChannelError> {
    match s {
        "acp" => Ok(AgentType::Acp),
        "openclaw-gateway" => Ok(AgentType::OpenclawGateway),
        "remote" => Ok(AgentType::Remote),
        "nomi" => Ok(AgentType::Nomi),
        _ => Err(ChannelError::InvalidConfig(format!(
            "channel session names agent type '{s}', which no longer exists in this build"
        ))),
    }
}

fn channel_conversation_name(
    platform: PluginType,
    agent_type: &str,
    backend: Option<&str>,
    chat_id: Option<&str>,
) -> String {
    let short = match platform {
        PluginType::Telegram => "tg",
        PluginType::Lark => "lark",
        PluginType::Dingtalk => "ding",
        PluginType::Weixin => "wx",
        PluginType::Wecom => "wecom",
        PluginType::Slack => "slack",
        PluginType::Discord => "discord",
        PluginType::Matrix => "matrix",
        PluginType::Mattermost => "mm",
        PluginType::Twitch => "twitch",
        PluginType::Nostr => "nostr",
        PluginType::Qqbot => "qq",
    };

    let mut parts = vec![short.to_owned()];
    if !agent_type.is_empty() {
        parts.push(agent_type.to_owned());
    }
    if agent_type == "acp"
        && let Some(b) = backend
    {
        parts.push(b.to_owned());
    }
    if let Some(cid) = chat_id {
        let end = cid.len().min(8);
        parts.push(cid[..end].to_owned());
    }
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_ai_agent::protocol::events::{
        AcpToolCallContentItem, AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
        AcpToolCallUpdateData, ErrorEventData, FinishEventData, StartEventData, TextEventData,
        ThinkingEventData, ToolCallEventData, ToolCallStatus,
    };
    use nomifun_common::{PersistedArtifactId, ProviderWithModel};

    // ── extract_last_user_text ────────────────────────────────────────

    fn make_history_message(
        id: &str,
        msg_type: MessageType,
        position: Option<MessagePosition>,
        content: serde_json::Value,
        hidden: bool,
    ) -> MessageResponse {
        MessageResponse {
            message_id: id.into(),
            conversation_id: nomifun_common::ConversationId::new().into_string(),
            msg_id: Some(id.into()),
            r#type: msg_type,
            content,
            position,
            status: None,
            hidden,
            created_at: 0,
        }
    }

    #[test]
    fn extract_picks_newest_user_text_from_desc_page() {
        // Newest-first page: assistant reply, then the user prompt that
        // produced it, then an older user prompt.
        let items = vec![
            make_history_message(
                "m3",
                MessageType::Text,
                Some(MessagePosition::Left),
                serde_json::json!({ "content": "assistant says hi" }),
                false,
            ),
            make_history_message(
                "m2",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "newest user prompt" }),
                false,
            ),
            make_history_message(
                "m1",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "older user prompt" }),
                false,
            ),
        ];
        assert_eq!(extract_last_user_text(&items).as_deref(), Some("newest user prompt"));
    }

    #[test]
    fn extract_skips_hidden_and_non_text_messages() {
        let items = vec![
            make_history_message(
                "m3",
                MessageType::ToolCall,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "tool payload" }),
                false,
            ),
            make_history_message(
                "m2",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "hidden prompt" }),
                true,
            ),
            make_history_message(
                "m1",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "visible prompt" }),
                false,
            ),
        ];
        assert_eq!(extract_last_user_text(&items).as_deref(), Some("visible prompt"));
    }

    #[test]
    fn extract_returns_none_without_user_messages() {
        let items = vec![make_history_message(
            "m1",
            MessageType::Text,
            Some(MessagePosition::Left),
            serde_json::json!({ "content": "assistant only" }),
            false,
        )];
        assert_eq!(extract_last_user_text(&items), None);
        assert_eq!(extract_last_user_text(&[]), None);
    }

    #[test]
    fn extract_skips_blank_content() {
        let items = vec![
            make_history_message(
                "m2",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "   " }),
                false,
            ),
            make_history_message(
                "m1",
                MessageType::Text,
                Some(MessagePosition::Right),
                serde_json::json!({ "content": "real prompt" }),
                false,
            ),
        ];
        assert_eq!(extract_last_user_text(&items).as_deref(), Some("real prompt"));
    }

    // ── platform_to_source ─────────────────────────────────────────────

    #[test]
    fn platform_to_source_telegram() {
        assert_eq!(platform_to_source(PluginType::Telegram), ConversationSource::Telegram);
    }

    // ── apply_channel_agent_context ────────────────────────────────────

    #[test]
    fn channel_context_nomi_applies_companion_context() {
        let mut extra = ChannelMessageService::build_channel_extra(None);
        apply_channel_agent_context(&mut extra, AgentType::Nomi, PluginType::Telegram, Some("companion_1"));
        assert_eq!(extra["companion_session"], serde_json::json!(true));
        assert_eq!(extra["channel_platform"], serde_json::json!("telegram"));
        assert_eq!(extra["companion_id"], serde_json::json!("companion_1"));
        // Existing channel semantics survive.
        assert_eq!(extra["session_mode"], serde_json::json!("yolo"));
    }

    #[test]
    fn channel_context_nomi_without_companion_omits_companion_id_key() {
        let mut extra = ChannelMessageService::build_channel_extra(None);
        apply_channel_agent_context(&mut extra, AgentType::Nomi, PluginType::Telegram, None);
        assert_eq!(extra["companion_session"], serde_json::json!(true));
        assert!(extra.get("companion_id").is_none(), "no companion → no companion_id key");

        // Blank companion id is treated the same as no companion.
        let mut extra = ChannelMessageService::build_channel_extra(None);
        apply_channel_agent_context(&mut extra, AgentType::Nomi, PluginType::Telegram, Some("  "));
        assert!(extra.get("companion_id").is_none());
    }

    #[test]
    fn channel_context_acp_preserves_backend_without_nomi_context() {
        let mut extra = ChannelMessageService::build_channel_extra(Some("claude"));
        apply_channel_agent_context(&mut extra, AgentType::Acp, PluginType::Lark, Some("companion_1"));
        assert!(extra.get("companion_session").is_none());
        assert!(extra.get("channel_platform").is_none());
        assert!(extra.get("companion_id").is_none());
        assert_eq!(extra["backend"], serde_json::json!("claude"));
    }

    #[test]
    fn platform_to_source_lark() {
        assert_eq!(platform_to_source(PluginType::Lark), ConversationSource::Lark);
    }

    #[test]
    fn platform_to_source_dingtalk() {
        assert_eq!(platform_to_source(PluginType::Dingtalk), ConversationSource::Dingtalk);
    }

    #[test]
    fn platform_to_source_weixin() {
        assert_eq!(platform_to_source(PluginType::Weixin), ConversationSource::Weixin);
    }

    #[test]
    fn platform_to_source_reserved_defaults_to_nomifun() {
        assert_eq!(platform_to_source(PluginType::Slack), ConversationSource::Nomifun);
        assert_eq!(platform_to_source(PluginType::Discord), ConversationSource::Nomifun);
    }

    // ── parse_agent_type ───────────────────────────────────────────────

    #[test]
    fn parse_known_agent_types() {
        assert_eq!(parse_agent_type("acp").unwrap(), AgentType::Acp);
        assert_eq!(
            parse_agent_type("openclaw-gateway").unwrap(),
            AgentType::OpenclawGateway
        );
        assert_eq!(parse_agent_type("remote").unwrap(), AgentType::Remote);
        assert_eq!(parse_agent_type("nomi").unwrap(), AgentType::Nomi);
    }

    #[test]
    fn parse_unknown_agent_type_is_rejected() {
        assert!(parse_agent_type("unknown").is_err());
        assert!(parse_agent_type("nanobot").is_err());
        assert!(parse_agent_type("").is_err());
    }

    // ── process_stream_event ───────────────────────────────────────────

    #[test]
    fn text_event_produces_append() {
        let event = AgentStreamEvent::Text(TextEventData {
            content: "Hello".into(),
        });
        let action = ChannelMessageService::process_stream_event(&event);
        match action {
            Some(StreamAction::AppendText(text)) => assert_eq!(text, "Hello"),
            _ => panic!("Expected AppendText"),
        }
    }

    #[test]
    fn finish_event_produces_finish() {
        let event = AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None });
        let action = ChannelMessageService::process_stream_event(&event);
        assert!(matches!(action, Some(StreamAction::Finish)));
    }

    #[test]
    fn truncated_or_cancelled_finish_is_an_error_not_a_success_boundary() {
        use nomifun_ai_agent::protocol::events::TurnStopReason;

        for stop_reason in [
            TurnStopReason::MaxTokens,
            TurnStopReason::MaxTurnRequests,
            TurnStopReason::Refusal,
            TurnStopReason::Cancelled,
        ] {
            let event = AgentStreamEvent::Finish(FinishEventData {
                session_id: None,
                stop_reason: Some(stop_reason),
            });
            assert!(
                matches!(
                    ChannelMessageService::process_stream_event(&event),
                    Some(StreamAction::Error(_))
                ),
                "{stop_reason:?} must not release queued artifacts"
            );
        }

        let normal = AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: Some(TurnStopReason::EndTurn),
        });
        assert!(matches!(
            ChannelMessageService::process_stream_event(&normal),
            Some(StreamAction::Finish)
        ));
    }

    #[test]
    fn error_event_produces_error() {
        let event = AgentStreamEvent::Error(ErrorEventData::legacy("timeout", None));
        let action = ChannelMessageService::process_stream_event(&event);
        match action {
            Some(StreamAction::Error(msg)) => assert_eq!(msg, "timeout"),
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn thinking_event_produces_thinking() {
        let event = AgentStreamEvent::Thinking(ThinkingEventData {
            content: "Analyzing...".into(),
            subject: None,
            duration: None,
            status: None,
        });
        let action = ChannelMessageService::process_stream_event(&event);
        match action {
            Some(StreamAction::Thinking(text)) => assert_eq!(text, "Analyzing..."),
            _ => panic!("Expected Thinking"),
        }
    }

    #[test]
    fn tool_call_event_produces_tool_call() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
            artifacts: Vec::new(),
            retry: None,
        });
        let action = ChannelMessageService::process_stream_event(&event);
        match action {
            Some(StreamAction::ToolCall { name, status }) => {
                assert_eq!(name, "read_file");
                assert_eq!(status, "Running");
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn start_event_produces_none() {
        let event = AgentStreamEvent::Start(StartEventData { session_id: None });
        assert!(ChannelMessageService::process_stream_event(&event).is_none());
    }

    #[test]
    fn completed_workshop_tool_call_produces_media() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "nomi_workshop_get_task".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some(
                r#"{"status":"succeeded","result_asset_ids":["0190f5fe-7c00-7a00-8000-000000000086"]}"#
                    .into(),
            ),
            artifacts: Vec::new(),
            retry: None,
        });
        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::MediaProduced(ids)) => {
                assert_eq!(ids, vec!["0190f5fe-7c00-7a00-8000-000000000086"])
            }
            other => panic!("expected MediaProduced, got {other:?}"),
        }
    }

    #[test]
    fn completed_tool_call_preserves_verified_artifact_receipts() {
        let artifact = nomifun_ai_agent::artifact_store::PersistedArtifact {
            id: PersistedArtifactId::new().into_string(),
            kind: nomifun_ai_agent::artifact_store::ArtifactKind::File,
            mime_type: "application/pdf".into(),
            path: "/workspace/nomifun-artifacts/artifact-1.pdf".into(),
            relative_path: "nomifun-artifacts/artifact-1.pdf".into(),
            size_bytes: 10,
            sha256: "abc".into(),
        };
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "mcp__reports__export".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some("done".into()),
            artifacts: vec![artifact.clone()],
            retry: None,
        });

        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::ArtifactsProduced(artifacts)) => {
                assert_eq!(artifacts, vec![artifact]);
            }
            other => panic!("expected ArtifactsProduced, got {other:?}"),
        }
    }

    #[test]
    fn completed_acp_tool_call_preserves_verified_artifact_receipts() {
        let artifact = nomifun_ai_agent::artifact_store::PersistedArtifact {
            id: PersistedArtifactId::new().into_string(),
            kind: nomifun_ai_agent::artifact_store::ArtifactKind::Image,
            mime_type: "image/png".into(),
            path: "/workspace/nomifun-artifacts/artifact-acp-1.png".into(),
            relative_path: "nomifun-artifacts/artifact-acp-1.png".into(),
            size_bytes: 10,
            sha256: "abc".into(),
        };
        let event = AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "tool-1".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: None,
                kind: None,
                raw_input: None,
                raw_output: None,
                content: Some(vec![AcpToolCallContentItem::Artifact {
                    artifact: artifact.clone(),
                    source_uri: None,
                }]),
                locations: None,
            },
            meta: None,
        });

        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::ArtifactsProduced(artifacts)) => {
                assert_eq!(artifacts, vec![artifact]);
            }
            other => panic!("expected ACP ArtifactsProduced, got {other:?}"),
        }
    }

    #[test]
    fn failed_acp_tool_call_never_uploads_artifact_receipts() {
        let artifact = nomifun_ai_agent::artifact_store::PersistedArtifact {
            id: PersistedArtifactId::new().into_string(),
            kind: nomifun_ai_agent::artifact_store::ArtifactKind::Image,
            mime_type: "image/png".into(),
            path: "/workspace/nomifun-artifacts/artifact-acp-failed.png".into(),
            relative_path: "nomifun-artifacts/artifact-acp-failed.png".into(),
            size_bytes: 10,
            sha256: "abc".into(),
        };
        let event = AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "tool-1".into(),
                status: Some(AcpToolCallStatus::Failed),
                title: None,
                kind: None,
                raw_input: None,
                raw_output: None,
                content: Some(vec![AcpToolCallContentItem::Artifact {
                    artifact,
                    source_uri: None,
                }]),
                locations: None,
            },
            meta: None,
        });

        assert!(ChannelMessageService::process_stream_event(&event).is_none());
    }

    #[test]
    fn running_tool_call_still_produces_tool_call_status() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "nomi_workshop_generate".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
            artifacts: Vec::new(),
            retry: None,
        });
        assert!(matches!(
            ChannelMessageService::process_stream_event(&event),
            Some(StreamAction::ToolCall { .. })
        ));
    }

    #[test]
    fn completed_tool_call_without_asset_ids_stays_tool_call() {
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "Read".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some("just some text output".into()),
            artifacts: Vec::new(),
            retry: None,
        });
        assert!(matches!(
            ChannelMessageService::process_stream_event(&event),
            Some(StreamAction::ToolCall { .. })
        ));
    }

    // ── process_stream_event → Decision ────────────────────────────────

    #[test]
    fn denied_stop_tool_call_produces_stop_denied_with_target() {
        let target = "0190f5fe-7c00-7a00-8abc-012345678901";
        for output in [
            "{\"error\":\"'nomi_stop_conversation' is not permitted on the Channel surface\"}",
            "{\"error\":\"session_capability_denied\",\"tool\":\"nomi_stop_conversation\"}",
        ] {
            let event = AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: "c1".into(),
                name: "mcp__nomi__nomi_stop_conversation".into(),
                args: serde_json::json!({ "conversation_id": target }),
                status: ToolCallStatus::Completed,
                description: None,
                input: None,
                output: Some(output.into()),
                artifacts: Vec::new(),
                retry: None,
            });
            match ChannelMessageService::process_stream_event(&event) {
                Some(StreamAction::StopDenied { target_conversation_id }) => {
                    assert_eq!(target_conversation_id, target);
                }
                other => panic!("expected StopDenied for output {output:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn denied_stop_target_falls_back_to_raw_input_json() {
        let target = "0190f5fe-7c00-7a00-8abc-012345678902";
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "nomi_stop_conversation".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Error,
            description: None,
            input: Some(serde_json::json!({ "conversation_id": target })),
            output: Some("'nomi_stop_conversation' is not permitted on the Channel surface".into()),
            artifacts: Vec::new(),
            retry: None,
        });
        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::StopDenied { target_conversation_id }) => {
                assert_eq!(target_conversation_id, target);
            }
            other => panic!("expected StopDenied, got {other:?}"),
        }
    }

    #[test]
    fn successful_or_unrelated_tool_calls_never_produce_stop_denied() {
        // A SUCCESSFUL stop (allowed surface) keeps the plain tool-call action.
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c1".into(),
            name: "nomi_stop_conversation".into(),
            args: serde_json::json!({ "conversation_id": "0190f5fe-7c00-7a00-8abc-012345678903" }),
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some("{\"result\":{\"stopped\":true}}".into()),
            artifacts: Vec::new(),
            retry: None,
        });
        assert!(matches!(
            ChannelMessageService::process_stream_event(&event),
            Some(StreamAction::ToolCall { .. })
        ));

        // Another denied tool must not be misread as a stop confirmation.
        let event = AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "c2".into(),
            name: "nomi_delete_conversation".into(),
            args: serde_json::json!({ "conversation_id": "0190f5fe-7c00-7a00-8abc-012345678903" }),
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some("'nomi_delete_conversation' is not permitted on the Channel surface".into()),
            artifacts: Vec::new(),
            retry: None,
        });
        assert!(matches!(
            ChannelMessageService::process_stream_event(&event),
            Some(StreamAction::ToolCall { .. })
        ));
    }

    #[test]
    fn acp_permission_request_produces_decision() {
        use nomifun_ai_agent::protocol::events::{
            AcpPermissionEventData, AcpPermissionOptionData, AcpPermissionOptionKind, AcpPermissionRequestData,
            AcpPermissionToolCall,
        };

        let event = AgentStreamEvent::AcpPermission(AcpPermissionEventData::Request(AcpPermissionRequestData {
            session_id: "s1".into(),
            tool_call: AcpPermissionToolCall {
                tool_call_id: "call-7".into(),
                status: None,
                title: Some("Run rm -rf?".into()),
                kind: None,
                raw_input: None,
                raw_output: None,
                content: None,
                locations: None,
                meta: None,
            },
            options: vec![
                AcpPermissionOptionData {
                    option_id: "allow".into(),
                    name: "Allow once".into(),
                    kind: AcpPermissionOptionKind::AllowOnce,
                    meta: None,
                },
                AcpPermissionOptionData {
                    option_id: "reject".into(),
                    name: "Reject".into(),
                    kind: AcpPermissionOptionKind::RejectOnce,
                    meta: None,
                },
            ],
            meta: None,
        }));

        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::Decision { call_id, prompt, options }) => {
                assert_eq!(call_id, "call-7");
                assert_eq!(prompt, "Run rm -rf?");
                assert_eq!(
                    options,
                    vec![
                        crate::types::DecisionOption {
                            option_id: "allow".into(),
                            label: "Allow once".into()
                        },
                        crate::types::DecisionOption {
                            option_id: "reject".into(),
                            label: "Reject".into()
                        },
                    ]
                );
            }
            other => panic!("expected Decision, got {other:?}"),
        }
    }

    #[test]
    fn permission_value_confirmation_produces_decision() {
        // Legacy untyped Permission carrying a serialized `Confirmation`.
        let value = serde_json::json!({
            "id": "conf-1",
            "call_id": "call-9",
            "title": "Edit file?",
            "action": null,
            "description": "edits main.rs",
            "command_type": "edit",
            "options": [
                { "label": "Yes", "value": "yes" },
                { "label": "No", "value": "no" },
            ],
        });
        let event = AgentStreamEvent::Permission(value);

        match ChannelMessageService::process_stream_event(&event) {
            Some(StreamAction::Decision { call_id, prompt, options }) => {
                assert_eq!(call_id, "call-9");
                assert_eq!(prompt, "Edit file?");
                assert_eq!(
                    options,
                    vec![
                        crate::types::DecisionOption {
                            option_id: "yes".into(),
                            label: "Yes".into()
                        },
                        crate::types::DecisionOption {
                            option_id: "no".into(),
                            label: "No".into()
                        },
                    ]
                );
            }
            other => panic!("expected Decision, got {other:?}"),
        }
    }

    #[test]
    fn permission_with_empty_options_produces_none() {
        let value = serde_json::json!({
            "id": "conf-2",
            "call_id": "call-10",
            "title": "No choices",
            "description": "",
            "options": [],
        });
        let event = AgentStreamEvent::Permission(value);
        assert!(
            ChannelMessageService::process_stream_event(&event).is_none(),
            "an unanswerable decision (no options) must not surface"
        );
    }


    // ── build_thinking_message ─────────────────────────────────────────

    #[test]
    fn thinking_message_has_text() {
        let msg = ChannelMessageService::build_thinking_message();
        assert_eq!(msg.message_type, OutgoingMessageType::Text);
        let text = msg.text.unwrap();
        assert!(text.contains("Thinking"));
    }

    // ── build_final_message ────────────────────────────────────────────

    #[test]
    fn final_message_is_marked_final_without_buttons() {
        let msg = ChannelMessageService::build_final_message("Response text");
        // `Buttons` type is kept as the "final turn" marker (channels key off it
        // to finalize a streaming card), but no action buttons are attached.
        assert_eq!(msg.message_type, OutgoingMessageType::Buttons);
        assert_eq!(msg.text.as_deref(), Some("Response text"));
        assert!(msg.buttons.is_none(), "final reply must carry no action buttons");
    }

    // ── build_streaming_message ────────────────────────────────────────

    #[test]
    fn streaming_message_is_plain_text() {
        let msg = ChannelMessageService::build_streaming_message("partial...");
        assert_eq!(msg.message_type, OutgoingMessageType::Text);
        assert_eq!(msg.text.as_deref(), Some("partial..."));
        assert!(msg.buttons.is_none());
    }

    // ── build_decision_message ─────────────────────────────────────────

    #[test]
    fn decision_message_is_numbered_plain_text() {
        let options = vec![
            crate::types::DecisionOption {
                option_id: "a".into(),
                label: "Allow".into(),
            },
            crate::types::DecisionOption {
                option_id: "b".into(),
                label: "Deny".into(),
            },
        ];
        let msg = ChannelMessageService::build_decision_message("Proceed?", &options);

        assert_eq!(msg.message_type, OutgoingMessageType::Text);
        assert!(msg.buttons.is_none(), "decision is plain text, no buttons");
        let text = msg.text.expect("decision message must carry text");
        assert!(text.contains("Proceed?"), "prompt rendered: {text}");
        assert!(text.contains("1. Allow"), "first option numbered: {text}");
        assert!(text.contains("2. Deny"), "second option numbered: {text}");
        assert!(text.contains("回复编号"), "reply-by-number hint present: {text}");
    }

    // ── build_channel_extra ───────────────────────────────────────────

    #[test]
    fn yolo_extra_contains_session_mode() {
        let extra = ChannelMessageService::build_channel_extra(None);
        assert_eq!(extra["session_mode"], "yolo");
        assert!(extra.get("backend").is_none());
    }

    #[test]
    fn yolo_extra_with_backend() {
        let extra = ChannelMessageService::build_channel_extra(Some("claude"));
        assert_eq!(extra["session_mode"], "yolo");
        assert_eq!(extra["backend"], "claude");
    }

    // ── model placement by agent_type (regression: non-nomi must not
    //    use top-level model) ──────────────────────────────────────────

    #[test]
    fn acp_model_goes_into_extra_not_top_level() {
        let agent_type = AgentType::Acp;
        let model = ProviderWithModel {
            provider_id: "prov1".into(),
            model: "claude-sonnet".into(),
            use_model: Some("global.anthropic.claude-sonnet-4-6".into()),
        };
        let mut extra = ChannelMessageService::build_channel_extra(Some("codex"));

        let top_level_model = if agent_type == AgentType::Nomi {
            Some(model.clone())
        } else {
            extra["model"] = serde_json::to_value(&model).unwrap();
            None
        };

        assert!(top_level_model.is_none(), "acp must not have top-level model");
        assert_eq!(extra["model"]["provider_id"], "prov1");
        assert_eq!(extra["model"]["use_model"], "global.anthropic.claude-sonnet-4-6");
    }

    #[test]
    fn nomi_model_stays_at_top_level() {
        let agent_type = AgentType::Nomi;
        let model = ProviderWithModel {
            provider_id: "prov2".into(),
            model: "gpt-4o".into(),
            use_model: None,
        };
        let mut extra = ChannelMessageService::build_channel_extra(None);

        let top_level_model = if agent_type == AgentType::Nomi {
            Some(model.clone())
        } else {
            extra["model"] = serde_json::to_value(&model).unwrap();
            None
        };

        assert!(top_level_model.is_some(), "nomi must use top-level model");
        assert!(extra.get("model").is_none() || extra["model"].is_null());
    }

    // ── channel_conversation_name ─────────────────────────────────────

    #[test]
    fn conv_name_telegram_acp_with_backend() {
        let name = channel_conversation_name(PluginType::Telegram, "acp", Some("claude"), Some("70880480"));
        assert_eq!(name, "tg-acp-claude-70880480");
    }

    #[test]
    fn conv_name_telegram_nomi() {
        let name = channel_conversation_name(PluginType::Telegram, "nomi", None, Some("70880480"));
        assert_eq!(name, "tg-nomi-70880480");
    }

    #[test]
    fn conv_name_lark_acp_no_backend() {
        let name = channel_conversation_name(PluginType::Lark, "acp", None, Some("abcdef12"));
        assert_eq!(name, "lark-acp-abcdef12");
    }

    #[test]
    fn conv_name_dingtalk_truncates_long_chat_id() {
        let name = channel_conversation_name(PluginType::Dingtalk, "acp", Some("vertex"), Some("123456789abcdef"));
        assert_eq!(name, "ding-acp-vertex-12345678");
    }

    #[test]
    fn conv_name_weixin_no_chat_id() {
        let name = channel_conversation_name(PluginType::Weixin, "acp", Some("gemini"), None);
        assert_eq!(name, "wx-acp-gemini");
    }

    #[test]
    fn conv_name_non_acp_ignores_backend() {
        let name = channel_conversation_name(PluginType::Telegram, "nomi", Some("claude"), Some("70880480"));
        assert_eq!(name, "tg-nomi-70880480");
    }
}
