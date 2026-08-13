//! Busy-time prompt queue drain (spec D1).
//!
//! Watches turn completions and delivers each conversation's queued prompts
//! strictly FIFO through the full `send_to_agent` receipt chain, with bounded
//! automatic retries for structured-retryable failures (spec D4 fields) and a
//! 30-minute expiry sweep.
//!
//! Signal source selection (plan Task 3 exploration): completions are consumed
//! from the in-process realtime bus — `BroadcastEventBus::subscribe_user()`
//! observes every `turn.completed` envelope that
//! `StreamRelay::broadcast_turn_completed_with_context` publishes, so no new
//! event plumbing is needed. A periodic sweep tick backstops both queue expiry
//! and any envelopes lost to broadcast lag.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_db::models::ChannelPendingPromptRow;
use nomifun_db::{IChannelRepository, PENDING_PROMPT_EXPIRY_MS};
use nomifun_realtime::UserEventEnvelope;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::group_policy::GroupPolicyFence;
use crate::message_service::ChannelMessageService;
use crate::session::SessionManager;
use crate::stream_relay::{ChannelSender, ChannelStreamRelay, RelayConfig};
use crate::types::{OutgoingMessageType, PluginType, UnifiedOutgoingMessage};

/// Maximum automatic retries per queued prompt (spec: ≤2, backoff 30s/120s).
const MAX_PROMPT_RETRIES: i64 = 2;

/// Spec backoff schedule: first retry after 30s, second after 120s.
const DEFAULT_RETRY_BACKOFF: [Duration; 2] =
    [Duration::from_secs(30), Duration::from_secs(120)];

/// Backstop sweep period (expiry + lost-signal recovery).
const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Delivers queued busy-time prompts once their conversation finishes a turn.
pub struct QueueDrain {
    repo: Arc<dyn IChannelRepository>,
    message_service: Arc<ChannelMessageService>,
    session_manager: Arc<SessionManager>,
    sender: Arc<dyn ChannelSender>,
    group_policy_fence: Arc<GroupPolicyFence>,
    retry_backoff: [Duration; 2],
    sweep_interval: Duration,
    /// In-memory retry gate: prompt_id → earliest next attempt. Deliberately
    /// not persisted — after a restart a due retry simply runs immediately.
    retry_not_before: Mutex<HashMap<String, tokio::time::Instant>>,
    /// Conversations currently inside `drain_conversation`. A concurrent
    /// signal for the same conversation is dropped (the in-flight pass or the
    /// sweep backstop covers it) so one queued prompt can never be dispatched
    /// twice concurrently with two relays racing on the same chat.
    draining: Mutex<HashSet<String>>,
}

impl QueueDrain {
    pub fn new(
        repo: Arc<dyn IChannelRepository>,
        message_service: Arc<ChannelMessageService>,
        session_manager: Arc<SessionManager>,
        sender: Arc<dyn ChannelSender>,
    ) -> Self {
        Self {
            repo,
            message_service,
            session_manager,
            sender,
            group_policy_fence: Arc::new(GroupPolicyFence::default()),
            retry_backoff: DEFAULT_RETRY_BACKOFF,
            sweep_interval: DEFAULT_SWEEP_INTERVAL,
            retry_not_before: Mutex::new(HashMap::new()),
            draining: Mutex::new(HashSet::new()),
        }
    }

    /// Share the manager-owned group-policy fence with queued delivery.
    pub fn with_group_policy_fence(mut self, fence: Arc<GroupPolicyFence>) -> Self {
        self.group_policy_fence = fence;
        self
    }

    /// Test-only knob: shrink the spec 30s/120s backoff and the sweep period
    /// so bounded-retry behaviour is observable in integration tests.
    pub fn with_timing(mut self, retry_backoff: [Duration; 2], sweep_interval: Duration) -> Self {
        self.retry_backoff = retry_backoff;
        self.sweep_interval = sweep_interval;
        self
    }

    /// Runs until the event bus closes. Call via `tokio::spawn`.
    ///
    /// `events` is a `BroadcastEventBus::subscribe_user()` receiver; only
    /// `turn.completed` envelopes are consumed.
    pub async fn run(self, mut events: broadcast::Receiver<UserEventEnvelope>) {
        let drain = Arc::new(self);
        info!("channel queue drain started");

        // Startup recovery: expire stale rows (with chat notices), then try to
        // move every conversation that still has queued prompts — their target
        // may already be idle, in which case no completion event will come.
        drain.expire_and_notify().await;
        drain.sweep().await;

        let mut tick = tokio::time::interval(drain.sweep_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                envelope = events.recv() => match envelope {
                    Ok(envelope) if envelope.event.name == "turn.completed" => {
                        if let Some(conversation_id) = envelope
                            .event
                            .data
                            .get("conversation_id")
                            .and_then(serde_json::Value::as_str)
                        {
                            drain.drain_conversation(conversation_id).await;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "queue drain lagged behind the event bus; sweeping all queued conversations");
                        drain.sweep().await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    drain.expire_and_notify().await;
                    drain.sweep().await;
                }
            }
        }
        info!("channel queue drain stopped (event bus closed)");
    }

    /// Expire every queued prompt older than 30 minutes and tell its chat.
    async fn expire_and_notify(&self) {
        let cutoff = nomifun_common::now_ms() - PENDING_PROMPT_EXPIRY_MS;
        let expired = match self.repo.expire_stale(cutoff, nomifun_common::now_ms()).await {
            Ok(rows) => rows,
            Err(error) => {
                error!(error = %error, "queue drain expiry sweep failed");
                return;
            }
        };
        for row in expired {
            info!(prompt_id = %row.prompt_id, conversation_id = %row.conversation_id, "queued prompt expired");
            self.notify_chat(
                &row,
                format!(
                    "\u{231b} 排队消息已放弃（等待超过 30 分钟）：{}",
                    snippet(&row.text)
                ),
            )
            .await;
        }
    }

    /// Attempt to drain every conversation that still has queued prompts.
    async fn sweep(&self) {
        let conversations = match self.repo.list_queued_conversations().await {
            Ok(list) => list,
            Err(error) => {
                error!(error = %error, "queue drain sweep listing failed");
                return;
            }
        };
        for conversation_id in conversations {
            self.drain_conversation(&conversation_id).await;
        }
    }

    /// Move one conversation's queue forward as far as currently possible.
    ///
    /// Settles heads whose outcome is already durable (delivered / final
    /// failure / expiry), then dispatches at most ONE fresh delivery — its own
    /// `turn.completed` re-enters this method for the next FIFO element.
    async fn drain_conversation(&self, conversation_id: &str) {
        if !self
            .draining
            .lock()
            .unwrap()
            .insert(conversation_id.to_owned())
        {
            return; // another pass is already working this conversation
        }
        self.drain_conversation_inner(conversation_id).await;
        self.draining.lock().unwrap().remove(conversation_id);
    }

    async fn drain_conversation_inner(&self, conversation_id: &str) {
        loop {
            let candidate = match self.repo.peek_next_queued(conversation_id).await {
                Ok(Some(row)) => row,
                Ok(None) => return,
                Err(error) => {
                    error!(error = %error, conversation_id, "queue drain peek failed");
                    return;
                }
            };

            // The first peek is only a candidate used to select the bot gate.
            // Once inside the read-side critical section, re-peek before any
            // delivery side effect: a policy writer may have cancelled this
            // row and deleted its session while we were waiting for the gate.
            let _policy_permit = self
                .group_policy_fence
                .read(&candidate.channel_plugin_id)
                .await;
            let head = match self.repo.peek_next_queued(conversation_id).await {
                Ok(Some(row)) if row.prompt_id == candidate.prompt_id => row,
                Ok(Some(_)) => continue,
                Ok(None) => return,
                Err(error) => {
                    error!(error = %error, conversation_id, "queue drain fenced re-peek failed");
                    return;
                }
            };

            // Expiry is judged at delivery time too, not only by the sweep.
            if nomifun_common::now_ms() - head.queued_at >= PENDING_PROMPT_EXPIRY_MS {
                if self.settle(&head, "expired").await {
                    self.notify_chat(
                        &head,
                        format!(
                            "\u{231b} 排队消息已放弃（等待超过 30 分钟）：{}",
                            snippet(&head.text)
                        ),
                    )
                    .await;
                }
                continue;
            }

            let attempt_key = attempt_idempotency_key(&head);

            // The durable receipt of the current attempt decides what happens
            // next; this makes the drain restart-safe (a crash between send
            // and settle replays into the absorbed receipt, never re-executes).
            match self
                .message_service
                .turn_outcome(conversation_id, &attempt_key)
                .await
            {
                Ok(nomifun_conversation::PublicTurnDeliveryState::Missing) => {}
                Ok(nomifun_conversation::PublicTurnDeliveryState::Accepted { .. }) => {
                    // In flight (or quarantined after a crash) — judged by its
                    // completion event / a later sweep.
                    return;
                }
                Ok(nomifun_conversation::PublicTurnDeliveryState::Completed(delivery)) => {
                    if delivery.result_ok == Some(true) {
                        self.settle(&head, "delivered").await;
                        continue;
                    }
                    let retryable = delivery.result_error_retryable.unwrap_or(false);
                    if retryable && head.attempts < MAX_PROMPT_RETRIES {
                        match self.repo.increment_prompt_attempts(&head.prompt_id).await {
                            Ok(attempts) => {
                                let backoff = self.retry_backoff
                                    [usize::try_from(attempts - 1).unwrap_or(0).min(1)];
                                self.retry_not_before.lock().unwrap().insert(
                                    head.prompt_id.clone(),
                                    tokio::time::Instant::now() + backoff,
                                );
                                info!(
                                    prompt_id = %head.prompt_id,
                                    attempts,
                                    backoff_secs = backoff.as_secs(),
                                    "queued prompt failed retryably; scheduling retry"
                                );
                            }
                            Err(error) => {
                                error!(error = %error, prompt_id = %head.prompt_id, "retry accounting failed");
                            }
                        }
                        return;
                    }
                    // Final failure: real error text back to the chat.
                    if self.settle(&head, "failed").await {
                        let reason = delivery
                            .result_error
                            .or(delivery.result_error_code)
                            .unwrap_or_else(|| "unknown error".to_owned());
                        self.notify_chat(
                            &head,
                            format!(
                                "\u{274c} 排队消息「{}」处理失败：{reason}",
                                snippet(&head.text)
                            ),
                        )
                        .await;
                    }
                    continue;
                }
                Err(error) => {
                    warn!(error = %error, conversation_id, "queue drain outcome probe failed");
                    return;
                }
            }

            // Backoff gate for a scheduled retry: the sweep tick re-enters
            // once the wait is over.
            if let Some(due) = self
                .retry_not_before
                .lock()
                .unwrap()
                .get(&head.prompt_id)
                .copied()
                && tokio::time::Instant::now() < due
            {
                return;
            }
            self.retry_not_before.lock().unwrap().remove(&head.prompt_id);

            if self.message_service.is_conversation_busy(conversation_id).await {
                return;
            }
            if !self.dispatch(&head, &attempt_key).await {
                continue;
            }
            return;
        }
    }

    /// Send the head prompt through the full delivery chain. `true` means the
    /// attempt is in flight (stop draining); `false` means the head was
    /// settled and the caller may continue with the next element.
    async fn dispatch(&self, head: &ChannelPendingPromptRow, attempt_key: &str) -> bool {
        let session = match self
            .session_manager
            .get_session_by_id(&head.channel_session_id)
            .await
        {
            Ok(Some(session)) => session,
            Ok(None) => {
                warn!(prompt_id = %head.prompt_id, "queued prompt session no longer exists");
                self.settle(head, "failed").await;
                return false;
            }
            Err(error) => {
                error!(error = %error, prompt_id = %head.prompt_id, "queued prompt session lookup failed");
                return true; // transient: retry on the next signal
            }
        };
        if session.channel_plugin_id.as_deref() != Some(head.channel_plugin_id.as_str()) {
            warn!(
                prompt_id = %head.prompt_id,
                session_id = %session.channel_session_id,
                "queued prompt/session channel scope mismatch; cancelling without dispatch"
            );
            self.settle(head, "cancelled").await;
            return false;
        }
        // Unknown is not a weaker form of Direct. Old/malformed queued rows
        // with an unproven chat scope are permanently retired and can never be
        // handed to the agent, even when no policy update happens concurrently.
        if !is_dispatchable_session_chat_kind(&session.chat_kind) {
            warn!(
                prompt_id = %head.prompt_id,
                chat_kind = %session.chat_kind,
                "queued prompt has unsafe chat scope; cancelling without dispatch"
            );
            self.settle(head, "cancelled").await;
            return false;
        }
        let platform = match self.repo.get_plugin(&head.channel_plugin_id).await {
            Ok(Some(plugin)) => match PluginType::from_str_opt(&plugin.r#type) {
                Some(platform) => platform,
                None => {
                    warn!(prompt_id = %head.prompt_id, plugin_type = %plugin.r#type, "queued prompt has unknown platform");
                    self.settle(head, "failed").await;
                    return false;
                }
            },
            Ok(None) => {
                warn!(prompt_id = %head.prompt_id, "queued prompt bot channel no longer exists");
                self.settle(head, "failed").await;
                return false;
            }
            Err(error) => {
                error!(error = %error, prompt_id = %head.prompt_id, "queued prompt plugin lookup failed");
                return true;
            }
        };

        match self
            .message_service
            .send_to_agent(&session, &head.text, platform, attempt_key)
            .await
        {
            Ok(mut send_result) => {
                info!(
                    prompt_id = %head.prompt_id,
                    conversation_id = %send_result.conversation_id,
                    attempts = head.attempts,
                    "queued prompt dispatched"
                );
                // Relay the reply to the originating chat exactly like a live
                // inbound message would.
                if let Some(rx) = send_result.stream_rx.take() {
                    let relay = ChannelStreamRelay::new(
                        RelayConfig {
                            platform,
                            plugin_id: head.channel_plugin_id.clone(),
                            chat_id: head.chat_id.clone(),
                            throttle_ms: 500,
                            conversation_id: send_result.conversation_id.clone(),
                        },
                        Arc::clone(&self.sender),
                        self.message_service.pending_decisions(),
                        self.message_service.asset_resolver(),
                    );
                    tokio::spawn(relay.run(rx));
                }
                // Deliberately NOT settled here: the durable receipt outcome
                // (observed on the turn's completion) settles delivered /
                // schedules a retry / fails with the real error text.
                true
            }
            Err(crate::error::ChannelError::ConversationBusy(_)) => true,
            Err(error) => {
                // Admission-time failure: there is no durable receipt to
                // judge later, so this attempt is final.
                error!(error = %error, prompt_id = %head.prompt_id, "queued prompt dispatch failed");
                if self.settle(head, "failed").await {
                    self.notify_chat(
                        head,
                        format!(
                            "\u{274c} 排队消息「{}」处理失败：{error}",
                            snippet(&head.text)
                        ),
                    )
                    .await;
                }
                false
            }
        }
    }

    /// Settle a prompt; `true` when this caller performed the transition.
    async fn settle(&self, row: &ChannelPendingPromptRow, state: &str) -> bool {
        self.retry_not_before.lock().unwrap().remove(&row.prompt_id);
        match self
            .repo
            .settle_prompt(&row.prompt_id, state, nomifun_common::now_ms())
            .await
        {
            Ok(()) => true,
            Err(error) => {
                warn!(error = %error, prompt_id = %row.prompt_id, state, "queued prompt settlement rejected");
                false
            }
        }
    }

    async fn notify_chat(&self, row: &ChannelPendingPromptRow, text: String) {
        let message = UnifiedOutgoingMessage {
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
        };
        if let Err(error) = self
            .sender
            .send_message(&row.channel_plugin_id, &row.chat_id, message)
            .await
        {
            warn!(error = %error, prompt_id = %row.prompt_id, "queue drain chat notice failed");
        }
    }
}

fn is_dispatchable_session_chat_kind(chat_kind: &str) -> bool {
    matches!(chat_kind, "direct" | "group")
}

/// Idempotency key of the CURRENT delivery attempt.
///
/// Attempt 0 reuses the key minted at enqueue time (the exact key an
/// immediate dispatch would have used); each bounded retry derives a fresh
/// key because completed receipts are absorbing and would otherwise replay
/// the failed outcome forever.
fn attempt_idempotency_key(row: &ChannelPendingPromptRow) -> String {
    if row.attempts == 0 {
        row.idempotency_key.clone()
    } else {
        format!("{}:retry{}", row.idempotency_key, row.attempts)
    }
}

/// Short user-facing extract of a queued prompt.
fn snippet(text: &str) -> String {
    const MAX: usize = 40;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        trimmed.to_owned()
    } else {
        let head: String = trimmed.chars().take(MAX).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_delivery_permanently_rejects_unknown_chat_scope() {
        assert!(is_dispatchable_session_chat_kind("direct"));
        assert!(is_dispatchable_session_chat_kind("group"));
        assert!(!is_dispatchable_session_chat_kind("unknown"));
        assert!(!is_dispatchable_session_chat_kind(""));
        assert!(!is_dispatchable_session_chat_kind("future_kind"));
    }

    fn row(attempts: i64) -> ChannelPendingPromptRow {
        ChannelPendingPromptRow {
            prompt_id: nomifun_common::ChannelPendingPromptId::new().into_string(),
            channel_plugin_id: nomifun_common::ChannelPluginId::new().into_string(),
            chat_id: "chat-1".into(),
            channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
            conversation_id: nomifun_common::ConversationId::new().into_string(),
            text: "hello".into(),
            idempotency_key: "channel-turn:v1:abc".into(),
            state: "queued".into(),
            attempts,
            queued_at: 1,
            settled_at: None,
        }
    }

    #[test]
    fn first_attempt_reuses_the_enqueue_key() {
        assert_eq!(attempt_idempotency_key(&row(0)), "channel-turn:v1:abc");
    }

    #[test]
    fn retries_derive_fresh_absorbing_keys() {
        assert_eq!(attempt_idempotency_key(&row(1)), "channel-turn:v1:abc:retry1");
        assert_eq!(attempt_idempotency_key(&row(2)), "channel-turn:v1:abc:retry2");
    }

    #[test]
    fn snippet_truncates_long_prompts() {
        assert_eq!(snippet("  hi  "), "hi");
        let long = "x".repeat(80);
        let short = snippet(&long);
        assert!(short.chars().count() <= 41);
        assert!(short.ends_with('…'));
    }
}
