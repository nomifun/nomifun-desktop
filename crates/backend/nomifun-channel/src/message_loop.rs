use std::sync::Arc;

use nomifun_db::SettleChannelInboundReceiptParams;
use nomifun_db::models::NewChannelInboundReceiptRow;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::action::{ActionExecutor, MessageResult};
use crate::error::ChannelError;
use crate::message_service::ChannelMessageService;
use crate::session::SessionManager;
use crate::stream_relay::{ChannelSender, ChannelStreamRelay, RelayConfig};
use crate::types::{
    ActionBehavior, ChannelIncoming, OutgoingMessageType, UnifiedIncomingMessage,
    UnifiedOutgoingMessage,
};

/// Reply sent when a new message arrives while the previous turn of the same
/// chat is still being processed (per-chat concurrency guard).
const BUSY_NOTICE: &str =
    "\u{23f3} Your previous message is still being processed \u{2014} please wait for it to finish.";

/// IM command clearing this chat's busy-time prompt queue (spec D1).
const CANCEL_QUEUE_COMMAND: &str = "取消排队";

/// Reply when the conversation's queue already holds the maximum number of
/// pending prompts (spec D1: 10 per conversation).
const QUEUE_FULL_NOTICE: &str = "\u{23f8} 排队已满，请稍后再发。";

/// Reply for `chat.regenerate` when there is no user message to resend.
const NOTHING_TO_REGENERATE: &str =
    "\u{2139}\u{fe0f} There is no previous message to regenerate \u{2014} send a new message first.";

#[derive(Debug, Clone)]
struct InboundOperation {
    receipt: NewChannelInboundReceiptRow,
    turn_key: String,
}

fn canonical_json(value: &serde_json::Value, output: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            output.push('{');
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("JSON object keys always serialize"),
                );
                output.push(':');
                canonical_json(&map[key], output);
            }
            output.push('}');
        }
        serde_json::Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(item, output);
            }
            output.push(']');
        }
        scalar => output.push_str(
            &serde_json::to_string(scalar).expect("JSON scalar always serializes"),
        ),
    }
}

fn build_inbound_operation(
    owner_user_id: &str,
    channel_plugin_id: &str,
    message: &UnifiedIncomingMessage,
) -> Result<InboundOperation, String> {
    if message.id.trim().is_empty() {
        return Err("provider event/message id is empty".to_owned());
    }
    if message.id.len() > 512 {
        return Err("provider event/message id exceeds 512 bytes".to_owned());
    }
    if message.chat_id.trim().is_empty() {
        return Err("provider chat id is empty".to_owned());
    }
    if message.chat_id.len() > 512 {
        return Err("provider chat id exceeds 512 bytes".to_owned());
    }

    let platform = message.platform.to_string();
    let scope = serde_json::json!([
        owner_user_id,
        channel_plugin_id,
        platform.as_str(),
        message.chat_id.as_str(),
        message.id.as_str(),
    ]);
    let scope_digest = Sha256::digest(
        serde_json::to_vec(&scope).expect("channel inbound scope always serializes"),
    );
    let scope_hex = format!("{scope_digest:x}");

    // Exclude provider `raw`, display metadata, and receive-time timestamps:
    // those may vary across transport redelivery. The business payload covers
    // exactly the fields that can alter routing or side effects.
    let payload = serde_json::json!({
        "platform": platform.as_str(),
        "chat_id": &message.chat_id,
        "user_id": &message.user.id,
        "content": &message.content,
        "reply_to_message_id": &message.reply_to_message_id,
        "action": &message.action,
    });
    let mut canonical_payload = String::new();
    canonical_json(&payload, &mut canonical_payload);
    let payload_hash = format!("{:x}", Sha256::digest(canonical_payload.as_bytes()));
    let now = nomifun_common::now_ms();

    Ok(InboundOperation {
        receipt: NewChannelInboundReceiptRow {
            operation_key: format!("channel-inbound:v1:{scope_hex}"),
            user_id: owner_user_id.to_owned(),
            channel_plugin_id: channel_plugin_id.to_owned(),
            platform: message.platform.to_string(),
            chat_id: message.chat_id.clone(),
            provider_event_id: message.id.clone(),
            payload_hash,
            created_at: now,
        },
        turn_key: format!("channel-turn:v1:{scope_hex}"),
    })
}

/// Runs the full channel message lifecycle.
///
/// Consumes incoming IM messages from `message_rx` and tool confirmation
/// callbacks from `confirm_rx`, driving the pipeline:
/// 1. ActionExecutor routing (auth → action/AI dispatch)
/// 2. For Dispatched: send_to_agent + spawn ChannelStreamRelay
/// 3. For Action: reply via plugin
/// 4. Forward tool confirmations to the agent
pub struct ChannelMessageLoop {
    action_executor: Arc<ActionExecutor>,
    message_service: Arc<ChannelMessageService>,
    session_manager: Arc<SessionManager>,
    sender: Arc<dyn ChannelSender>,
}

impl ChannelMessageLoop {
    pub fn new(
        action_executor: Arc<ActionExecutor>,
        message_service: Arc<ChannelMessageService>,
        session_manager: Arc<SessionManager>,
        sender: Arc<dyn ChannelSender>,
    ) -> Self {
        Self {
            action_executor,
            message_service,
            session_manager,
            sender,
        }
    }

    /// Start the message loop. Runs until both channels close.
    pub async fn run(
        self,
        mut message_rx: mpsc::Receiver<ChannelIncoming>,
        mut confirm_rx: mpsc::Receiver<(String, String)>,
    ) {
        info!("ChannelMessageLoop started");

        loop {
            tokio::select! {
                Some(incoming) = message_rx.recv() => {
                    self.handle_message(incoming).await;
                }
                Some((call_id, value)) = confirm_rx.recv() => {
                    handle_confirm(&call_id, &value);
                }
                else => break,
            }
        }

        info!("ChannelMessageLoop stopped (channels closed)");
    }

    async fn handle_message(&self, incoming: ChannelIncoming) {
        let ChannelIncoming {
            channel_plugin_id,
            message: msg,
        } = incoming;
        let platform = msg.platform;
        let chat_id = msg.chat_id.clone();
        // Outgoing routing is per channel business identity.
        let plugin_id = channel_plugin_id;
        let text = msg.content.text.clone();

        let operation = match build_inbound_operation(
            self.message_service.owner_user_id(),
            &plugin_id,
            &msg,
        ) {
            Ok(operation) => operation,
            Err(reason) => {
                error!(
                    channel_plugin_id = %plugin_id,
                    platform = %platform,
                    chat_id = %chat_id,
                    provider_event_id = %msg.id,
                    reason,
                    "channel inbound rejected before side effects: no stable event identity"
                );
                return;
            }
        };
        let claim = match self
            .session_manager
            .claim_inbound(operation.receipt.clone())
            .await
        {
            Ok(claim) => claim,
            Err(error) => {
                error!(
                    channel_plugin_id = %plugin_id,
                    platform = %platform,
                    chat_id = %chat_id,
                    provider_event_id = %msg.id,
                    error = %error,
                    "channel inbound durable claim failed closed"
                );
                return;
            }
        };
        let receipt = match claim {
            nomifun_db::ChannelInboundClaim::Replay(receipt) => {
                info!(
                    operation_key = %receipt.operation_key,
                    status = %receipt.status,
                    phase = %receipt.phase,
                    owner_generation = receipt.owner_generation,
                    "channel inbound replay absorbed"
                );
                return;
            }
            nomifun_db::ChannelInboundClaim::Owner(receipt) => receipt,
        };
        match self
            .session_manager
            .begin_inbound_effects(
                &receipt.operation_key,
                &receipt.payload_hash,
                receipt.owner_generation,
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                info!(
                    operation_key = %receipt.operation_key,
                    owner_generation = receipt.owner_generation,
                    "channel inbound owner lost effects fence; replay absorbed"
                );
                return;
            }
            Err(error) => {
                error!(
                    operation_key = %receipt.operation_key,
                    owner_generation = receipt.owner_generation,
                    error = %error,
                    "channel inbound effects fence failed closed"
                );
                return;
            }
        }

        let executor = Arc::clone(&self.action_executor);
        let msg_svc = Arc::clone(&self.message_service);
        let session_mgr = Arc::clone(&self.session_manager);
        let sender = Arc::clone(&self.sender);

        tokio::spawn(async move {
            let mut settlement = SettleChannelInboundReceiptParams {
                outcome_json: Some(serde_json::json!({ "kind": "unknown" }).to_string()),
                ..Default::default()
            };
            let status = match executor.handle_incoming_message(&msg, &plugin_id).await {
                Ok(MessageResult::Action(response)) => {
                    match send_action_response(&sender, &plugin_id, &chat_id, &response).await {
                        Ok(()) => {
                            settlement.outcome_json = Some(
                                serde_json::json!({ "kind": "action", "behavior": response.behavior })
                                    .to_string(),
                            );
                            "completed"
                        }
                        Err(error) => {
                            error!(
                                channel_plugin_id = %plugin_id,
                                chat_id = %chat_id,
                                error = %error,
                                "channel action response could not be delivered"
                            );
                            settlement.error_text = Some(error.to_string());
                            settlement.outcome_json = Some(
                                serde_json::json!({
                                    "kind": "action_delivery_failed",
                                    "behavior": response.behavior
                                })
                                .to_string(),
                            );
                            "failed"
                        }
                    }
                }
                Ok(MessageResult::Dispatched {
                    session_id,
                    conversation_id,
                }) => {
                    if let Some(result) = handle_dispatched(
                        &msg_svc,
                        &session_mgr,
                        &sender,
                        &session_id,
                        conversation_id.as_deref(),
                        &text,
                        platform,
                        &plugin_id,
                        &chat_id,
                        &operation.turn_key,
                    )
                    .await
                    {
                        settlement.conversation_id = Some(result.conversation_id);
                        settlement.message_id = Some(result.message_id);
                    }
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "dispatched" }).to_string());
                    "completed"
                }
                Ok(MessageResult::DispatchedText {
                    session_id,
                    conversation_id,
                    text: synthesized,
                }) => {
                    // chat.continue: same pipeline as a typed message, with a
                    // synthesized prompt instead of the callback payload text.
                    if let Some(result) = handle_dispatched(
                        &msg_svc,
                        &session_mgr,
                        &sender,
                        &session_id,
                        conversation_id.as_deref(),
                        &synthesized,
                        platform,
                        &plugin_id,
                        &chat_id,
                        &operation.turn_key,
                    )
                    .await
                    {
                        settlement.conversation_id = Some(result.conversation_id);
                        settlement.message_id = Some(result.message_id);
                    }
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "continue" }).to_string());
                    "completed"
                }
                Ok(MessageResult::RegenerateRequested {
                    session_id,
                    conversation_id,
                }) => {
                    if let Some(result) = handle_regenerate(
                        &msg_svc,
                        &session_mgr,
                        &sender,
                        &session_id,
                        conversation_id.as_deref(),
                        platform,
                        &plugin_id,
                        &chat_id,
                        &operation.turn_key,
                    )
                    .await
                    {
                        settlement.conversation_id = Some(result.conversation_id);
                        settlement.message_id = Some(result.message_id);
                    }
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "regenerate" }).to_string());
                    "completed"
                }
                Ok(MessageResult::AlreadyProcessing) => {
                    info!(chat_id = %chat_id, "message ignored: already processing");
                    let _ = sender
                        .send_message(&plugin_id, &chat_id, plain_text_message(BUSY_NOTICE.into()))
                        .await;
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "busy" }).to_string());
                    "completed"
                }
                Err(e) => {
                    error!(error = %e, "failed to handle incoming message");
                    settlement.error_text = Some(e.to_string());
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "failed" }).to_string());
                    "failed"
                }
            };
            if let Err(error) = session_mgr
                .settle_inbound(
                    &receipt.operation_key,
                    &receipt.payload_hash,
                    receipt.owner_generation,
                    status,
                    settlement,
                )
                .await
            {
                error!(
                    operation_key = %receipt.operation_key,
                    owner_generation = receipt.owner_generation,
                    status,
                    error = %error,
                    "channel inbound settlement failed; effects remain absorbing"
                );
            }
        });
    }
}

async fn send_action_response(
    sender: &Arc<dyn ChannelSender>,
    plugin_id: &str,
    chat_id: &str,
    response: &crate::types::ActionResponse,
) -> Result<(), ChannelError> {
    if let Some(text) = &response.text {
        let outgoing = UnifiedOutgoingMessage {
            message_type: OutgoingMessageType::Text,
            text: Some(text.clone()),
            parse_mode: response.parse_mode,
            buttons: response.buttons.clone(),
            keyboard: response.keyboard.clone(),
            image_url: None,
            file_url: None,
            file_name: None,
            media_actions: None,
            reply_to_message_id: None,
            silent: None,
        };

        match response.behavior {
            ActionBehavior::Edit => {
                if let Some(ref edit_id) = response.edit_message_id {
                    sender.edit_message(plugin_id, chat_id, edit_id, outgoing).await?;
                }
            }
            _ => {
                sender.send_message(plugin_id, chat_id, outgoing).await?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_dispatched(
    msg_svc: &Arc<ChannelMessageService>,
    session_mgr: &Arc<SessionManager>,
    sender: &Arc<dyn ChannelSender>,
    session_id: &str,
    conversation_id: Option<&str>,
    text: &str,
    platform: crate::types::PluginType,
    plugin_id: &str,
    chat_id: &str,
    idempotency_key: &str,
) -> Option<crate::message_service::SendResult> {
    // 客服接缝: a bot bound to a customer-service agent hands the WHOLE
    // message to the customer-service domain — no Conversation, no decision
    // interception, no per-chat busy guard (客服域自己管并发：同访客串行合并，
    // 跨访客并发)。Empty reply = the text was merged into another in-flight
    // batch → send nothing.
    if let Some(cs_agent_id) = msg_svc.cs_bound_agent(plugin_id).await {
        let session = match session_mgr.get_session_by_id(session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                warn!(session_id = %session_id, "session not found for customer-service dispatch");
                return None;
            }
            Err(e) => {
                error!(error = %e, "failed to get session for customer-service dispatch");
                return None;
            }
        };
        let reply = msg_svc
            .cs_handle_visitor_message(
                &cs_agent_id,
                plugin_id,
                &session.channel_user_id,
                chat_id,
                text,
            )
            .await;
        let outgoing = match reply {
            Ok(reply) if reply.trim().is_empty() => return None, // merged
            Ok(reply) => reply,
            Err(notice) => notice,
        };
        let _ = sender
            .send_message(plugin_id, chat_id, plain_text_message(outgoing))
            .await;
        return None;
    }

    // D1: the queue-cancel command clears this chat's pending prompts. It runs
    // right after the customer-service seam (cs-bound bots are unaffected) and
    // BEFORE decision interception / busy guards, so a chat waiting on a
    // decision or a long turn can always empty its queue.
    if text.trim() == CANCEL_QUEUE_COMMAND {
        let reply = match msg_svc.cancel_chat_queue(plugin_id, chat_id).await {
            Ok(0) => "\u{2139}\u{fe0f} 当前没有排队中的消息。".to_owned(),
            Ok(count) => format!("\u{1f9f9} 已取消排队中的 {count} 条消息。"),
            Err(error) => {
                error!(error = %error, chat_id = %chat_id, "channel queue cancel failed");
                format!("\u{274c} 取消排队失败：{error}")
            }
        };
        let _ = sender
            .send_message(plugin_id, chat_id, plain_text_message(reply))
            .await;
        return None;
    }

    // Decision interception (Bug 1, Case A): when the bound conversation is
    // waiting on a relayed numbered decision, a reply is the user's *answer*,
    // not a new prompt. Map a valid number onto an option and resolve it via
    // `confirm`; re-show the list on any other reply. Runs before the busy
    // guard because the conversation is intentionally blocked on the decision.
    if let Some(cid) = conversation_id
        && let Some(pending) = msg_svc.pending_decisions().peek(cid)
    {
        match parse_choice(text, pending.options.len()) {
            Some(idx) => match &pending.kind {
                // Channel-owned remote-stop confirmation (batch-1 handover
                // gap): "确认停止" cancels the target as owner via the same
                // safe service path as the desktop stop button; "取消" just
                // clears the entry. Never routed through `confirm` — there is
                // no agent call waiting on this decision.
                crate::pending_decision::PendingDecisionKind::StopConversation {
                    target_conversation_id,
                } => {
                    msg_svc.pending_decisions().take(cid);
                    let reply = if idx == 0 {
                        // The stop worker is detached inside the service, so a
                        // bounded wait only caps the ACK latency — dropping
                        // this future never aborts the stop itself.
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            msg_svc.stop_conversation(target_conversation_id),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                info!(
                                    target = %target_conversation_id,
                                    "channel-confirmed remote stop executed"
                                );
                                format!(
                                    "\u{2705} 已停止会话 {target_conversation_id} 的当前任务。"
                                )
                            }
                            Ok(Err(e)) => {
                                error!(error = %e, target = %target_conversation_id, "channel remote stop failed");
                                format!(
                                    "\u{274c} 停止会话 {target_conversation_id} 失败：{e}"
                                )
                            }
                            Err(_elapsed) => {
                                info!(
                                    target = %target_conversation_id,
                                    "channel remote stop is still finalizing; ack sent early"
                                );
                                format!(
                                    "\u{23f9} 停止指令已发出，会话 {target_conversation_id} 正在停止中。"
                                )
                            }
                        }
                    } else {
                        "已取消，不停止该会话。".to_owned()
                    };
                    let _ = sender
                        .send_message(plugin_id, chat_id, plain_text_message(reply))
                        .await;
                }
                crate::pending_decision::PendingDecisionKind::AgentConfirm => {
                    let option = &pending.options[idx];
                    match msg_svc.submit_decision(cid, &pending.call_id, &option.option_id).await {
                        Ok(()) => {
                            msg_svc.pending_decisions().take(cid);
                            info!(conversation_id = %cid, option_id = %option.option_id, "channel decision resolved");
                            let _ = sender
                                .send_message(
                                    plugin_id,
                                    chat_id,
                                    plain_text_message(format!("\u{2705} 已选择：{}", option.label)),
                                )
                                .await;
                        }
                        Err(e) => {
                            // The decision can no longer be submitted — most often it
                            // was already answered from the desktop UI, or the turn
                            // ended. Clear the stale entry so the user's next message
                            // dispatches normally instead of being trapped on it.
                            msg_svc.pending_decisions().take(cid);
                            error!(error = %e, conversation_id = %cid, "channel decision submit failed; cleared stale pending");
                            let _ = sender
                                .send_message(
                                    plugin_id,
                                    chat_id,
                                    plain_text_message(format!(
                                        "\u{274c} 该决策已无法提交（可能已在桌面处理）：{e}。已清除等待，请重新发送你的指令。"
                                    )),
                                )
                                .await;
                        }
                    }
                }
            },
            None => {
                // Non-numeric / out-of-range reply: re-show the numbered list
                // (do not dispatch it as a new prompt).
                let msg = ChannelMessageService::build_decision_message(&pending.prompt, &pending.options);
                let _ = sender.send_message(plugin_id, chat_id, msg).await;
            }
        }
        return None;
    }

    // Per-chat concurrency guard: when the bound conversation is already
    // working on a turn, don't race a second prompt into it (turn admission
    // would reject it with an opaque error anyway) — enqueue it instead
    // (spec D1) and tell the user its FIFO position.
    if let Some(cid) = conversation_id
        && msg_svc.is_conversation_busy(cid).await
    {
        info!(conversation_id = %cid, chat_id = %chat_id, "conversation busy: queueing prompt");
        let reply = queue_busy_prompt_reply(
            msg_svc,
            plugin_id,
            chat_id,
            session_id,
            cid,
            text,
            idempotency_key,
        )
        .await;
        let _ = sender
            .send_message(plugin_id, chat_id, plain_text_message(reply))
            .await;
        return None;
    }

    let session = match session_mgr.get_session_by_id(session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            warn!(session_id = %session_id, "session not found after dispatch");
            return None;
        }
        Err(e) => {
            error!(error = %e, "failed to get session");
            return None;
        }
    };

    let mut send_result = match msg_svc
        .send_to_agent(&session, text, platform, idempotency_key)
        .await
    {
        Ok(r) => r,
        // The (now shared) companion session is already running a turn — queue
        // the prompt against the RESOLVED conversation (spec D1). Covers the
        // first-turn race the guard above can't see (it checks the pre-bind id).
        Err(ChannelError::ConversationBusy(cid)) => {
            info!(conversation_id = %cid, chat_id = %chat_id, "companion session busy: queueing prompt");
            let reply = queue_busy_prompt_reply(
                msg_svc,
                plugin_id,
                chat_id,
                session_id,
                &cid,
                text,
                idempotency_key,
            )
            .await;
            let _ = sender
                .send_message(plugin_id, chat_id, plain_text_message(reply))
                .await;
            return None;
        }
        // Companion bound but not yet usable (no model) — relay the plain notice,
        // not the generic ❌ failure line.
        Err(e @ ChannelError::CompanionNotReady(_)) => {
            info!(chat_id = %chat_id, "message rejected: companion not ready");
            let _ = sender.send_message(plugin_id, chat_id, plain_text_message(e.to_string())).await;
            return None;
        }
        Err(e) => {
            error!(error = %e, "failed to send to agent");
            let err_msg = plain_text_message(format!("\u{274c} Failed to process: {e}"));
            let _ = sender.send_message(plugin_id, chat_id, err_msg).await;
            return None;
        }
    };

    // Bind the conversation to this per-chat session whenever the conversation
    // the turn actually ran on differs from the session's current binding: a
    // first turn (was None), or a companion turn rerouted into the companion's
    // shared single session (the per-chat session may still point at None or a
    // stale per-chat id). Keeps the per-chat pointer in sync so the busy guard
    // and decision interception operate on the shared id on subsequent turns.
    if conversation_id != Some(send_result.conversation_id.as_str())
        && let Err(e) = session_mgr
            .bind_conversation(session_id, &send_result.conversation_id)
            .await
    {
        warn!(error = %e, "failed to bind conversation to session");
    }

    // Spawn stream relay if we got a subscription
    if let Some(rx) = send_result.stream_rx.take() {
        let relay_config = RelayConfig {
            platform,
            plugin_id: plugin_id.to_owned(),
            chat_id: chat_id.to_owned(),
            throttle_ms: 500,
            conversation_id: send_result.conversation_id.clone(),
        };
        let relay = ChannelStreamRelay::new(
            relay_config,
            Arc::clone(sender),
            msg_svc.pending_decisions(),
            msg_svc.asset_resolver(),
        );
        tokio::spawn(relay.run(rx));
    } else {
        warn!(
            conversation_id = %send_result.conversation_id,
            "no Agent runtime for stream subscription"
        );
    }
    Some(send_result)
}

/// Handles `chat.regenerate`: look up the conversation's last user message
/// and resend it through the regular dispatch path (streaming reply
/// included). Falls back to a notice when there is nothing to resend.
#[allow(clippy::too_many_arguments)]
async fn handle_regenerate(
    msg_svc: &Arc<ChannelMessageService>,
    session_mgr: &Arc<SessionManager>,
    sender: &Arc<dyn ChannelSender>,
    session_id: &str,
    conversation_id: Option<&str>,
    platform: crate::types::PluginType,
    plugin_id: &str,
    chat_id: &str,
    idempotency_key: &str,
) -> Option<crate::message_service::SendResult> {
    let Some(conversation_id) = conversation_id else {
        // Session has no backing conversation yet — nothing was ever asked.
        let _ = sender
            .send_message(plugin_id, chat_id, plain_text_message(NOTHING_TO_REGENERATE.into()))
            .await;
        return None;
    };

    match msg_svc.last_user_text(conversation_id).await {
        Ok(Some(text)) => {
            handle_dispatched(
                msg_svc,
                session_mgr,
                sender,
                session_id,
                Some(conversation_id),
                &text,
                platform,
                plugin_id,
                chat_id,
                idempotency_key,
            )
            .await
        }
        Ok(None) => {
            let _ = sender
                .send_message(plugin_id, chat_id, plain_text_message(NOTHING_TO_REGENERATE.into()))
                .await;
            None
        }
        Err(e) => {
            error!(error = %e, conversation_id = %conversation_id, "failed to load last user message for regenerate");
            let _ = sender
                .send_message(
                    plugin_id,
                    chat_id,
                    plain_text_message(format!("\u{274c} Failed to process: {e}")),
                )
                .await;
            None
        }
    }
}

/// Enqueue a busy-time prompt (spec D1) and render the user-facing reply.
///
/// Falls back to the plain busy notice when the queue write itself fails —
/// the user is still told the conversation is working, and the error is
/// logged for diagnostics.
#[allow(clippy::too_many_arguments)]
async fn queue_busy_prompt_reply(
    msg_svc: &Arc<ChannelMessageService>,
    plugin_id: &str,
    chat_id: &str,
    channel_session_id: &str,
    conversation_id: &str,
    text: &str,
    idempotency_key: &str,
) -> String {
    match msg_svc
        .enqueue_busy_prompt(
            plugin_id,
            chat_id,
            channel_session_id,
            conversation_id,
            text,
            idempotency_key,
        )
        .await
    {
        Ok(outcome) => busy_queue_reply(&outcome),
        Err(error) => {
            error!(
                error = %error,
                conversation_id = %conversation_id,
                chat_id = %chat_id,
                "busy prompt could not be queued; falling back to plain busy notice"
            );
            BUSY_NOTICE.to_owned()
        }
    }
}

/// The reply text for one enqueue outcome (spec D1 wording).
fn busy_queue_reply(outcome: &nomifun_db::PendingPromptEnqueue) -> String {
    match outcome {
        nomifun_db::PendingPromptEnqueue::Queued { position, .. } => format!(
            "\u{23f3} 会话正忙，已排队（第 {position} 位），完成后自动处理。回复「取消排队」可清空。"
        ),
        nomifun_db::PendingPromptEnqueue::QueueFull => QUEUE_FULL_NOTICE.to_owned(),
    }
}

/// Builds a plain text outgoing message (no parse mode, no buttons).
fn plain_text_message(text: String) -> UnifiedOutgoingMessage {
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

/// Forward a tool confirmation callback to the active agent.
fn handle_confirm(call_id: &str, value: &str) {
    // Channel conversations use yoloMode which auto-approves everything,
    // so this path is rarely hit. When needed, we can add a
    // call_id→conversation_id lookup via AgentRuntimeRegistry.
    info!(call_id = %call_id, value = %value, "forwarding tool confirmation");
}

/// Parses a channel user's numbered-decision reply into a 0-based option
/// index, valid only for `1..=n` (where `n` is the option count).
///
/// Returns `None` for non-numeric, out-of-range, or empty replies so the
/// caller can re-show the numbered list instead of dispatching the text.
fn parse_choice(text: &str, n: usize) -> Option<usize> {
    let choice: usize = text.trim().parse().ok()?;
    if choice >= 1 && choice <= n {
        Some(choice - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ActionResponse, MessageContentType, OutgoingMedia, PluginType,
        UnifiedMessageContent, UnifiedUser,
    };

    struct FailingActionSender;

    #[async_trait::async_trait]
    impl ChannelSender for FailingActionSender {
        async fn send_message(
            &self,
            _plugin_id: &str,
            _chat_id: &str,
            _message: UnifiedOutgoingMessage,
        ) -> Result<String, ChannelError> {
            Err(ChannelError::MessageSendFailed("test delivery failure".into()))
        }

        async fn edit_message(
            &self,
            _plugin_id: &str,
            _chat_id: &str,
            _message_id: &str,
            _message: UnifiedOutgoingMessage,
        ) -> Result<(), ChannelError> {
            Err(ChannelError::MessageSendFailed("test delivery failure".into()))
        }

        async fn send_media(
            &self,
            _plugin_id: &str,
            _chat_id: &str,
            _media: OutgoingMedia,
            _caption: Option<&str>,
        ) -> Result<String, ChannelError> {
            Err(ChannelError::MessageSendFailed("test delivery failure".into()))
        }
    }

    fn sample_inbound(id: &str, chat_id: &str, text: &str) -> UnifiedIncomingMessage {
        UnifiedIncomingMessage {
            id: id.into(),
            platform: PluginType::Telegram,
            chat_id: chat_id.into(),
            user: UnifiedUser {
                id: "provider-user".into(),
                username: Some("alice".into()),
                display_name: "Alice".into(),
                avatar_url: Some("https://example.invalid/alice.png".into()),
            },
            content: UnifiedMessageContent {
                content_type: MessageContentType::Text,
                text: text.into(),
                attachments: None,
            },
            timestamp: 1,
            reply_to_message_id: None,
            action: None,
            raw: Some(serde_json::json!({ "transport_attempt": 1 })),
        }
    }

    #[test]
    fn inbound_identity_scopes_same_provider_id_by_chat() {
        let owner = nomifun_common::UserId::new();
        let plugin = nomifun_common::ChannelPluginId::new();
        let first = build_inbound_operation(
            owner.as_str(),
            plugin.as_str(),
            &sample_inbound("provider-event-1", "chat-a", "hello"),
        )
        .unwrap();
        let second = build_inbound_operation(
            owner.as_str(),
            plugin.as_str(),
            &sample_inbound("provider-event-1", "chat-b", "hello"),
        )
        .unwrap();

        assert_ne!(first.receipt.operation_key, second.receipt.operation_key);
        assert_ne!(first.turn_key, second.turn_key);
    }

    #[test]
    fn inbound_payload_hash_ignores_redelivery_metadata_but_covers_business_payload() {
        let owner = nomifun_common::UserId::new();
        let plugin = nomifun_common::ChannelPluginId::new();
        let original = sample_inbound("provider-event-1", "chat-a", "hello");
        let mut redelivery = original.clone();
        redelivery.timestamp = 99_999;
        redelivery.user.display_name = "Renamed".into();
        redelivery.user.username = None;
        redelivery.user.avatar_url = None;
        redelivery.raw = Some(serde_json::json!({ "transport_attempt": 2 }));
        let mut changed_text = redelivery.clone();
        changed_text.content.text = "different".into();
        let mut changed_reply = redelivery.clone();
        changed_reply.reply_to_message_id = Some("reply-target".into());

        let original =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &original).unwrap();
        let redelivery =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &redelivery).unwrap();
        let changed_text =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_text).unwrap();
        let changed_reply =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_reply).unwrap();

        assert_eq!(original.receipt.operation_key, redelivery.receipt.operation_key);
        assert_eq!(original.receipt.payload_hash, redelivery.receipt.payload_hash);
        assert_eq!(original.receipt.operation_key, changed_text.receipt.operation_key);
        assert_ne!(original.receipt.payload_hash, changed_text.receipt.payload_hash);
        assert_ne!(original.receipt.payload_hash, changed_reply.receipt.payload_hash);
    }

    #[test]
    fn inbound_without_stable_provider_identity_fails_closed() {
        let owner = nomifun_common::UserId::new();
        let plugin = nomifun_common::ChannelPluginId::new();
        assert!(
            build_inbound_operation(
                owner.as_str(),
                plugin.as_str(),
                &sample_inbound("   ", "chat-a", "hello"),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn action_response_delivery_failure_is_propagated() {
        let sender: Arc<dyn ChannelSender> = Arc::new(FailingActionSender);
        let response = ActionResponse {
            text: Some("pairing code".into()),
            parse_mode: None,
            buttons: None,
            keyboard: None,
            behavior: ActionBehavior::Send,
            toast: None,
            edit_message_id: None,
        };

        let error = send_action_response(&sender, "plugin", "chat", &response)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("test delivery failure"));
    }

    #[test]
    fn parse_choice_valid_indices() {
        assert_eq!(parse_choice("1", 2), Some(0));
        assert_eq!(parse_choice("2", 2), Some(1));
        // Surrounding whitespace is tolerated.
        assert_eq!(parse_choice("  2  ", 3), Some(1));
        assert_eq!(parse_choice("\n1\t", 3), Some(0));
    }

    #[test]
    fn parse_choice_out_of_range() {
        assert_eq!(parse_choice("0", 2), None, "1-based: 0 is invalid");
        assert_eq!(parse_choice("3", 2), None, "beyond option count");
        assert_eq!(parse_choice("1", 0), None, "no options at all");
    }

    #[test]
    fn parse_choice_non_numeric() {
        assert_eq!(parse_choice("hello", 2), None);
        assert_eq!(parse_choice("", 2), None);
        assert_eq!(parse_choice("1.5", 2), None);
        assert_eq!(parse_choice("-1", 2), None);
        assert_eq!(parse_choice("1 2", 2), None, "two numbers is not a single choice");
    }

    // ── busy queue replies (spec D1) ───────────────────────────────────

    #[test]
    fn busy_queue_reply_reports_fifo_position_and_cancel_hint() {
        let row = nomifun_db::models::ChannelPendingPromptRow {
            prompt_id: nomifun_common::ChannelPendingPromptId::new().into_string(),
            channel_plugin_id: nomifun_common::ChannelPluginId::new().into_string(),
            chat_id: "chat-1".into(),
            channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
            conversation_id: nomifun_common::ConversationId::new().into_string(),
            text: "hello".into(),
            idempotency_key: "key".into(),
            state: "queued".into(),
            attempts: 0,
            queued_at: 1,
            settled_at: None,
        };
        let reply = busy_queue_reply(&nomifun_db::PendingPromptEnqueue::Queued {
            row,
            position: 3,
        });
        assert!(reply.contains("已排队"), "{reply}");
        assert!(reply.contains("第 3 位"), "{reply}");
        assert!(reply.contains("取消排队"), "cancel hint present: {reply}");
    }

    #[test]
    fn busy_queue_reply_full_queue_asks_to_retry_later() {
        let reply = busy_queue_reply(&nomifun_db::PendingPromptEnqueue::QueueFull);
        assert!(reply.contains("排队已满"), "{reply}");
    }
}
