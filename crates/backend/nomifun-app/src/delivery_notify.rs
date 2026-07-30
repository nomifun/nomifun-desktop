//! Spec D2 delivery-notify observer: pushes a completion receipt message to
//! the requester conversation once a watched keyed turn finishes.
//!
//! `nomifun-conversation` defines the [`TurnCompletionObserver`] seam and
//! invokes it (detached) after the terminal turn receipt is durably
//! persisted; this module implements it over the conversation service +
//! channel domain so neither of those crates gains a dependency on the other:
//!
//! 1. `take_pending_delivery_notify(operation_id)` claims the registration
//!    (single winner — duplicate completions deliver nothing twice).
//! 2. A receipt message (`origin = "delivery-notify"`, idempotency key
//!    derived from the operation id) is injected into the requester session
//!    through the same at-most-once keyed send every other producer uses.
//! 3. When the requester session is bound to an IM chat, the standard
//!    `ChannelStreamRelay` is spawned on the receipt turn so the companion's
//!    generated summary rides the existing stream-relay path back to the IM.

use std::sync::Arc;
use std::time::Duration;

use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::SendMessageRequest;
use nomifun_channel::message_service::AssetResolver;
use nomifun_channel::pending_decision::PendingDecisionStore;
use nomifun_channel::stream_relay::{ChannelSender, ChannelStreamRelay, RelayConfig};
use nomifun_channel::types::PluginType;
use nomifun_common::AppError;
use nomifun_conversation::{ConversationService, DELIVERY_NOTIFY_ORIGIN, TurnCompletionObserver};
use nomifun_db::IChannelRepository;
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};

/// Spec: success receipts embed at most this many characters of result text.
const RESULT_TEXT_LIMIT: usize = 1500;

/// Bounded busy-retry schedule for injecting the receipt into a requester
/// session that is momentarily running its own turn. The idempotency key is
/// fixed, so every attempt is an at-most-once replay-or-claim.
const BUSY_RETRY_ATTEMPTS: u32 = 8;
const BUSY_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(2);
const BUSY_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

pub struct DeliveryNotifyObserver {
    conversation_service: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    owner_user_id: Arc<str>,
    channel_repo: Arc<dyn IChannelRepository>,
    channel_sender: Arc<dyn ChannelSender>,
    pending_decisions: Arc<PendingDecisionStore>,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
}

impl DeliveryNotifyObserver {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_service: ConversationService,
        runtime_registry: Arc<dyn AgentRuntimeRegistry>,
        owner_user_id: Arc<str>,
        channel_repo: Arc<dyn IChannelRepository>,
        channel_sender: Arc<dyn ChannelSender>,
        pending_decisions: Arc<PendingDecisionStore>,
        asset_resolver: Option<Arc<dyn AssetResolver>>,
    ) -> Self {
        Self {
            conversation_service,
            runtime_registry,
            owner_user_id,
            channel_repo,
            channel_sender,
            pending_decisions,
            asset_resolver,
        }
    }

    /// Inject the receipt into the requester session (bounded busy retries),
    /// then relay the receipt turn to the requester's bound IM chat, if any.
    async fn deliver(
        &self,
        requester_conversation_id: &str,
        operation_id: &str,
        content: String,
    ) -> Result<(), AppError> {
        let idempotency_key = delivery_notify_idempotency_key(operation_id);
        let mut delay = BUSY_RETRY_INITIAL_DELAY;
        for attempt in 1..=BUSY_RETRY_ATTEMPTS {
            let req = SendMessageRequest {
                content: content.clone(),
                files: vec![],
                inject_skills: vec![],
                hidden: false,
                origin: Some(DELIVERY_NOTIFY_ORIGIN.to_owned()),
                channel_platform: None,
            };
            match self
                .conversation_service
                .send_message_with_idempotency_key(
                    &self.owner_user_id,
                    requester_conversation_id,
                    &idempotency_key,
                    req,
                    &self.runtime_registry,
                )
                .await
            {
                Ok(delivery) => {
                    info!(
                        requester_conversation_id,
                        operation_id,
                        replayed = delivery.replayed,
                        "delivery-notify receipt injected"
                    );
                    // A fresh (non-replayed, still running) receipt turn is
                    // relayed to the bound IM chat through the existing
                    // stream-relay path.
                    if !delivery.completed {
                        self.relay_to_bound_chat(requester_conversation_id).await;
                    }
                    return Ok(());
                }
                // The requester session is momentarily busy — the receipt is
                // worth a short bounded wait (the fixed key keeps this safe).
                Err(AppError::Conflict(_)) if attempt < BUSY_RETRY_ATTEMPTS => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(BUSY_RETRY_MAX_DELAY);
                }
                Err(e) => return Err(e),
            }
        }
        Err(AppError::Conflict(
            "requester conversation stayed busy for every receipt delivery attempt".to_owned(),
        ))
    }

    /// Spawn the standard channel relay on the receipt turn when the
    /// requester conversation is bound to an IM chat (most recently active
    /// binding wins). Desktop-only sessions need nothing: the turn already
    /// reaches the UI through the realtime bus.
    async fn relay_to_bound_chat(&self, requester_conversation_id: &str) {
        let sessions = match self.channel_repo.get_all_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                warn!(error = %error, "delivery-notify channel binding lookup failed");
                return;
            }
        };
        let Some(session) = sessions
            .into_iter()
            .filter(|s| s.conversation_id.as_deref() == Some(requester_conversation_id))
            .max_by_key(|s| s.last_activity)
        else {
            return; // not channel-bound
        };
        let (Some(plugin_id), Some(chat_id)) = (session.channel_plugin_id, session.chat_id) else {
            return;
        };
        let platform = match self.channel_repo.get_plugin(&plugin_id).await {
            Ok(Some(plugin)) => match PluginType::from_str_opt(&plugin.r#type) {
                Some(platform) => platform,
                None => return,
            },
            _ => return,
        };

        // Runtime registration precedes prompt dispatch, so polling briefly
        // right after the keyed send catches the receipt turn's stream.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let rx = loop {
            if let Some(handle) = self.runtime_registry.get_runtime(requester_conversation_id) {
                break handle.subscribe();
            }
            if tokio::time::Instant::now() >= deadline {
                warn!(
                    requester_conversation_id,
                    "delivery-notify relay could not attach to the receipt turn's runtime"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let relay = ChannelStreamRelay::new(
            RelayConfig {
                platform,
                plugin_id,
                chat_id,
                throttle_ms: 500,
                conversation_id: requester_conversation_id.to_owned(),
            },
            Arc::clone(&self.channel_sender),
            Arc::clone(&self.pending_decisions),
            self.asset_resolver.clone(),
        );
        tokio::spawn(relay.run(rx));
    }
}

#[async_trait::async_trait]
impl TurnCompletionObserver for DeliveryNotifyObserver {
    async fn on_turn_completed(
        &self,
        conversation_id: &str,
        operation_id: &str,
        result_ok: bool,
        result_text: Option<&str>,
        result_error_code: Option<&str>,
    ) {
        // Single-winner claim: a duplicate completion (replayed finalization)
        // observes None and delivers nothing twice.
        let registration = match self
            .conversation_service
            .take_pending_delivery_notify(operation_id)
            .await
        {
            Ok(Some(registration)) => registration,
            Ok(None) => return,
            Err(error) => {
                error!(error = %error, operation_id, "delivery-notify take failed");
                return;
            }
        };
        let content = build_delivery_notify_text(
            conversation_id,
            result_ok,
            result_text,
            result_error_code,
        );
        if let Err(error) = self
            .deliver(&registration.requester_conversation_id, operation_id, content)
            .await
        {
            error!(
                error = %error,
                operation_id,
                requester = %registration.requester_conversation_id,
                "delivery-notify receipt could not be injected"
            );
            if let Err(mark_error) = self
                .conversation_service
                .mark_delivery_notify_failed(operation_id)
                .await
            {
                warn!(error = %mark_error, operation_id, "delivery-notify failure marking failed");
            }
        }
    }
}

/// Idempotency key of one receipt injection.
///
/// The plan derives it as `"delivery-notify:" + operation_id`, but a raw
/// public-turn operation id can exceed the 128-byte public idempotency key
/// bound, so the operation identity is folded through SHA-256 (same
/// determinism, bounded length).
pub fn delivery_notify_idempotency_key(operation_id: &str) -> String {
    format!(
        "delivery-notify:{:x}",
        Sha256::digest(operation_id.as_bytes())
    )
}

/// User-facing receipt text for the requester session.
///
/// Success embeds the target's final text (truncated to 1500 chars); failure
/// carries the structured code plus whatever partial text exists. The framing
/// asks the companion to summarize back — the receipt turn's reply is what a
/// bound IM chat ultimately sees.
pub fn build_delivery_notify_text(
    conversation_id: &str,
    result_ok: bool,
    result_text: Option<&str>,
    result_error_code: Option<&str>,
) -> String {
    if result_ok {
        let body = match result_text.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => truncate_chars(text, RESULT_TEXT_LIMIT),
            None => "（无文本结果）".to_owned(),
        };
        format!(
            "📬 任务回执：你派发给会话 {conversation_id} 的任务已完成。结果如下，请向用户简要转述：\n{body}"
        )
    } else {
        let code = result_error_code.unwrap_or("unknown_error");
        let detail = result_text
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| format!("\n{}", truncate_chars(text, RESULT_TEXT_LIMIT)))
            .unwrap_or_default();
        format!(
            "📬 任务回执：你派发给会话 {conversation_id} 的任务失败（{code}）。请把失败情况告知用户。{detail}"
        )
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let head: String = text.chars().take(limit).collect();
        format!("{head}…[已截断]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_is_deterministic_and_bounded() {
        let long_operation = format!(
            "public-turn:v1:{}:{}:{}",
            "0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-7a00-8000-000000000002",
            "k".repeat(128),
        );
        let key = delivery_notify_idempotency_key(&long_operation);
        assert_eq!(key, delivery_notify_idempotency_key(&long_operation));
        assert!(key.starts_with("delivery-notify:"));
        assert!(
            key.len() <= 128,
            "must satisfy the public idempotency key bound, got {}",
            key.len()
        );
        assert_ne!(key, delivery_notify_idempotency_key("other-op"));
    }

    #[test]
    fn success_receipt_embeds_truncated_result_text() {
        let long = "结".repeat(2000);
        let text = build_delivery_notify_text("conv-1", true, Some(&long), None);
        assert!(text.contains("已完成"));
        assert!(text.contains("[已截断]"));
        assert!(text.chars().count() < 1700);

        let empty = build_delivery_notify_text("conv-1", true, None, None);
        assert!(empty.contains("（无文本结果）"));
    }

    #[test]
    fn failure_receipt_carries_the_structured_code() {
        let text = build_delivery_notify_text(
            "conv-1",
            false,
            Some("partial output"),
            Some("user_llm_provider_rate_limited"),
        );
        assert!(text.contains("失败"));
        assert!(text.contains("user_llm_provider_rate_limited"));
        assert!(text.contains("partial output"));

        let bare = build_delivery_notify_text("conv-1", false, None, None);
        assert!(bare.contains("unknown_error"));
    }
}
