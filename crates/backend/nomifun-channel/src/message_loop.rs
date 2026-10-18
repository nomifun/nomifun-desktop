use std::sync::Arc;

use nomifun_db::SettleChannelInboundReceiptParams;
use nomifun_db::models::NewChannelInboundReceiptRow;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
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
    if message.user.id.trim().is_empty() {
        return Err("provider sender id is empty".to_owned());
    }
    if message.user.id.len() > 512 {
        return Err("provider sender id exceeds 512 bytes".to_owned());
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
        "chat_kind": message.chat_kind,
        "mention_state": message.mention_state,
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
/// Consumes incoming IM messages from `message_rx`, driving the pipeline:
/// 1. ActionExecutor routing (auth → action/AI dispatch)
/// 2. For Dispatched: send_to_agent + spawn ChannelStreamRelay
/// 3. For Action: reply via plugin
pub struct ChannelMessageLoop {
    action_executor: Arc<ActionExecutor>,
    message_service: Arc<ChannelMessageService>,
    session_manager: Arc<SessionManager>,
    sender: Arc<dyn ChannelSender>,
    shutdown: CancellationToken,
    child_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

type ChildTaskRegistry = Arc<Mutex<Vec<JoinHandle<()>>>>;

async fn track_child_task(registry: &ChildTaskRegistry, task: JoinHandle<()>) {
    let mut tasks = registry.lock().await;
    tasks.retain(|task| !task.is_finished());
    tasks.push(task);
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
            shutdown: CancellationToken::new(),
            child_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start the message loop. Runs until the message channel closes.
    pub async fn run(self, mut message_rx: mpsc::Receiver<ChannelIncoming>) {
        self.run_with_shutdown(&mut message_rx, CancellationToken::new())
            .await;
    }

    /// Start the message loop with an explicit process-lifetime cancellation
    /// boundary.  The ordinary `run` method remains available to standalone
    /// users/tests; production hosts use this method so all child handlers and
    /// stream relays are joined before the database closes.
    pub async fn run_with_shutdown(
        mut self,
        message_rx: &mut mpsc::Receiver<ChannelIncoming>,
        shutdown: CancellationToken,
    ) {
        self.shutdown = shutdown;
        info!("ChannelMessageLoop started");

        loop {
            let incoming = tokio::select! {
                _ = self.shutdown.cancelled() => break,
                incoming = message_rx.recv() => incoming,
            };
            let Some(incoming) = incoming else {
                break;
            };
            if self.shutdown.is_cancelled() {
                break;
            }
            self.handle_message(incoming).await;
        }

        self.shutdown.cancel();
        self.join_child_tasks().await;
        info!("ChannelMessageLoop stopped (channel closed)");
    }

    async fn join_child_tasks(&self) {
        let tasks = std::mem::take(&mut *self.child_tasks.lock().await);
        for mut task in tasks {
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if !error.is_cancelled() => {
                    warn!(error = %error, "channel message child task failed during shutdown");
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    warn!("channel message child task exceeded shutdown deadline and was aborted");
                }
            }
        }
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
        let group_policy_fence = executor.group_policy_fence();
        let msg_svc = Arc::clone(&self.message_service);
        let session_mgr = Arc::clone(&self.session_manager);
        let sender = Arc::clone(&self.sender);
        let shutdown = self.shutdown.clone();
        let child_tasks = Arc::clone(&self.child_tasks);

        let task = tokio::spawn(async move {
            let mut settlement = SettleChannelInboundReceiptParams {
                outcome_json: Some(serde_json::json!({ "kind": "unknown" }).to_string()),
                ..Default::default()
            };
            // Hold admission's read permit across the complete durable handoff,
            // not merely ActionExecutor's policy read/session creation. This
            // also covers busy-queue insertion and send_to_agent admission.
            let policy_permit = group_policy_fence.read(&plugin_id).await;
            let status = match executor
                .handle_incoming_message_with_policy_permit(&msg, &plugin_id, &policy_permit)
                .await
            {
                Ok(MessageResult::Ignored { reason }) => {
                    info!(channel_plugin_id = %plugin_id, chat_id = %chat_id, reason, "channel inbound ignored by admission policy");
                    settlement.outcome_json =
                        Some(serde_json::json!({ "kind": "ignored", "reason": reason }).to_string());
                    "completed"
                }
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
                        &shutdown,
                        &child_tasks,
                        &session_id,
                        conversation_id.as_deref(),
                        &text,
                        msg.reply_to_message_id.as_deref(),
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
                        &shutdown,
                        &child_tasks,
                        &session_id,
                        conversation_id.as_deref(),
                        &synthesized,
                        None,
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
                        &shutdown,
                        &child_tasks,
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
            // Relays and model completion are intentionally outside the
            // critical section: send_to_agent has already made the durable
            // turn admission decision. The setter waits only for that handoff.
            drop(policy_permit);
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
        track_child_task(&self.child_tasks, task).await;
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
    shutdown: &CancellationToken,
    child_tasks: &ChildTaskRegistry,
    session_id: &str,
    conversation_id: Option<&str>,
    text: &str,
    reply_to_message_id: Option<&str>,
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

    // Channel-owned stop interception: when the bound conversation is waiting
    // on a numbered stop decision, a reply is the user's answer rather than a
    // new prompt. Re-show the list on any invalid reply. This runs before the
    // busy guard so a running conversation can still be stopped remotely.
    if let Some(cid) = conversation_id
        && let Some(pending) = msg_svc.stop_confirmations().peek(cid)
    {
        match parse_choice(text, pending.options.len()) {
            Some(idx) => match &pending.kind {
                // Channel-owned remote-stop confirmation (batch-1 handover
                // gap): "确认停止" cancels the target as owner via the same
                // safe service path as the desktop stop button; "取消" just
                // clears the entry. Never routed through `confirm` — there is
                // no agent call waiting on this decision.
                crate::pending_decision::ChannelStopConfirmationKind::StopConversation {
                    target_conversation_id,
                } => {
                    msg_svc.stop_confirmations().take(cid);
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
            },
            None => {
                // Non-numeric / out-of-range reply: re-show the numbered list
                // (do not dispatch it as a new prompt).
                let msg = ChannelMessageService::build_stop_confirmation_message(&pending.prompt, &pending.options);
                let _ = sender.send_message(plugin_id, chat_id, msg).await;
            }
        }
        return None;
    }

    let session = match session_mgr.get_session_by_id(session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => { warn!(session_id, "session not found after dispatch"); return None; }
        Err(error) => { error!(%error, "failed to get dispatched Channel session"); return None; }
    };
    let quoted = if let Some(cid) = conversation_id {
        match msg_svc.try_reply_to_question(&session,cid,text,reply_to_message_id,platform,idempotency_key).await {
            Ok(result) => result,
            Err(error) => {
                let _ = sender.send_message(plugin_id,chat_id,plain_text_message(format!("❌ 消息未保存：{error}"))).await;
                return None;
            }
        }
    } else { None };
    // Guest sessions keep their existing constrained domain dispatch. Approved
    // ordinary messages are always saved by the canonical intake, even while
    // execution is busy; no frontend busy FIFO chooses their next Turn.
    let approved = match msg_svc.is_approved_session(&session).await {
        Ok(approved) => approved,
        Err(error) => { warn!(%error,"could not verify Channel input principal");return None; }
    };
    if !approved && let Some(cid)=conversation_id && msg_svc.is_conversation_busy(cid).await {
        let reply=queue_busy_prompt_reply(msg_svc,plugin_id,chat_id,session_id,cid,text,idempotency_key).await;
        let _=sender.send_message(plugin_id,chat_id,plain_text_message(reply)).await;
        return None;
    }
    let mut send_result = if let Some(result)=quoted { result } else { match msg_svc
        .send_to_agent(&session, text, platform, idempotency_key)
        .await
    {
        Ok(r) => r,
        // The (now shared) companion session is already running a turn — queue
        // the prompt against the RESOLVED conversation (spec D1). Covers the
        // first-turn race the guard above can't see (it checks the pre-bind id).
        Err(ChannelError::ConversationBusy(cid)) if !approved => {
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
    }};

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

    let relay_config = RelayConfig {platform,plugin_id:plugin_id.into(),chat_id:chat_id.into(),
        throttle_ms:500,conversation_id:send_result.conversation_id.clone()};
    if let Some(key)=send_result.observer_input_id.as_deref() {
        if let Some(task)=crate::input_observer::register_input_observer(msg_svc.clone(),relay_config,
            sender.clone(),key.to_owned(),shutdown.clone()) {
            track_child_task(child_tasks,task).await;
        }
    } else if let Some(rx)=send_result.stream_rx.take() {
        let relay=ChannelStreamRelay::new(relay_config,sender.clone(),msg_svc.stop_confirmations(),msg_svc.asset_resolver())
            .with_input_question_delivery(msg_svc.clone());
        let shutdown=shutdown.clone();
        track_child_task(child_tasks,tokio::spawn(async move {tokio::select! {
            _=shutdown.cancelled()=>{},_=relay.run(rx)=>{},
        }})).await;
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
    shutdown: &CancellationToken,
    child_tasks: &ChildTaskRegistry,
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
                shutdown,
                child_tasks,
                session_id,
                Some(conversation_id),
                &text,
                None,
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
mod input_ingress_tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_api_types::{AgentQuestionReplyTargetDto,ConversationResponse,CreateConversationRequest,ListMessagesQuery,MessageListResponse,SendMessageRequest};
    use nomifun_common::AppError;
    use nomifun_db::{IChannelRepository,SqliteChannelRepository,init_database_memory};
    use nomifun_db::models::{NewChannelPluginRow,NewChannelUserRow,CHANNEL_USER_AUTHORIZATION_APPROVED,CHANNEL_USER_AUTHORIZATION_AUTO_GROUP};
    use crate::session_port::{ChannelSessionPort,ChannelTurnDelivery,ChannelTurnDeliveryReceipt,ChannelTurnReceiptState,ChannelQuestionDelivery,ChannelInputObservation};

    struct InputPort {
        reject:bool,
        replies:std::sync::Mutex<Vec<(String,String,AgentQuestionReplyTargetDto)>>,
        inputs:std::sync::Mutex<Vec<(String,String)>>,
        deliveries:std::sync::Mutex<Vec<ChannelQuestionDelivery>>,
    }
    fn accepted()->ChannelTurnDelivery {
        ChannelTurnDelivery {delivery:ChannelTurnDeliveryReceipt {message_id:"0190f5fe-7c00-7a00-8000-000000000904".into(),
            replayed:false,completed:false,turn_outcome:None,result_ok:None,result_text:None,result_error:None,result_error_code:None,result_error_retryable:None},events:None}
    }
    #[async_trait]
    impl ChannelSessionPort for InputPort {
        async fn accept_input(&self,owner:&str,session:&str,key:&str,request:SendMessageRequest,
            _source:nomifun_agent_contracts::ChannelInputSource,reply:Option<AgentQuestionReplyTargetDto>)->Result<ChannelTurnDelivery,AppError> {
            match reply {Some(target)=>self.reply_to_question(owner,session,key,request,target).await,None=>self.send_turn(owner,session,key,request).await}
        }
        async fn is_busy(&self,_:&str)->bool {true}
        async fn turn_outcome(&self,_:&str,_:&str,_:&str)->Result<ChannelTurnReceiptState,AppError> {Ok(ChannelTurnReceiptState::Missing)}
        async fn cancel(&self,_:&str,_:&str)->Result<(),AppError> {Ok(())}
        async fn list_messages(&self,_:&str,_:&str,_:ListMessagesQuery)->Result<MessageListResponse,AppError> {panic!("input delivery must not infer a target from transcript")}
        async fn send_turn(&self,_:&str,_:&str,key:&str,request:SendMessageRequest)->Result<ChannelTurnDelivery,AppError> {
            self.inputs.lock().unwrap().push((key.into(),request.content));Ok(accepted())
        }
        async fn reply_to_question(&self,_:&str,_:&str,key:&str,request:SendMessageRequest,target:AgentQuestionReplyTargetDto)->Result<ChannelTurnDelivery,AppError> {
            self.replies.lock().unwrap().push((key.into(),request.content,target));
            if self.reject {Err(AppError::Conflict("original question association was refused".into()))} else {Ok(accepted())}
        }
        async fn observe_input(&self,_:&str,_:&str,_:&str)->Result<ChannelInputObservation,AppError> {Ok(ChannelInputObservation::Pending)}
        async fn record_question_delivery(&self,_:&str,_:&str,delivery:ChannelQuestionDelivery)->Result<(),AppError> {
            self.deliveries.lock().unwrap().push(delivery);Ok(())
        }
        async fn resolve_question_delivery(&self,_:&str,_:&str,platform:&str,plugin:&str,chat:&str,message:&str)->Result<Option<AgentQuestionReplyTargetDto>,AppError> {
            Ok(self.deliveries.lock().unwrap().iter().find(|row|row.platform==platform && row.channel_plugin_id==plugin && row.chat_id==chat && row.provider_message_id==message)
                .map(|row|row.target.clone()))
        }
        async fn get(&self,_:&str,session:&str)->Result<ConversationResponse,AppError> {
            Ok(serde_json::from_value(serde_json::json!({"conversation_id":session,"name":"Original","type":"nomi","status":"running","pinned":false,
                "delegation_policy":"automatic","decision_policy":"automatic","created_at":1,"modified_at":1,"extra":{}})).unwrap())
        }
        async fn create_idempotent(&self,_:&str,_:CreateConversationRequest,_:&str)->Result<ConversationResponse,AppError> {panic!("an existing input cannot create another Session")}
    }
    #[tokio::test]
    async fn quoted_closed_question_preserves_its_source_while_ordinary_busy_input_bypasses_fifo() {
        for(quoted,reject,guest)in [(true,false,false),(true,true,false),(false,false,false),(true,false,true)] {
            let db=init_database_memory().await.unwrap();
            let repo:Arc<dyn IChannelRepository>=Arc::new(SqliteChannelRepository::new(db.pool().clone()));
            let plugin=repo.create_plugin(&NewChannelPluginRow {r#type:"telegram".into(),name:"Input test".into(),enabled:true,config:"{}".into(),status:None,
                last_connected:None,companion_id:None,bot_key:None,owner_domain:"companion".into(),group_access_mode:"allowlist".into(),created_at:1,updated_at:1}).await.unwrap();
            let row=NewChannelUserRow {platform_user_id:"human".into(),platform_type:"telegram".into(),channel_plugin_id:Some(plugin.channel_plugin_id.clone()),
                display_name:None,authorization_kind:if guest {CHANNEL_USER_AUTHORIZATION_AUTO_GROUP}else{CHANNEL_USER_AUTHORIZATION_APPROVED}.into(),authorized_at:1,last_active:None};
            let user=if guest {repo.ensure_auto_group_user(&row).await}else{repo.create_user(&row).await}.unwrap();
            let manager=Arc::new(SessionManager::new(repo.clone()));
            let session=manager.get_or_create_session_for_chat(&user.channel_user_id,"chat",&plugin.channel_plugin_id,"nomi",
                if guest {crate::types::ChatKind::Group}else{crate::types::ChatKind::Direct},None).await.unwrap();
            let conversation="0190f5fe-7c00-7a00-8000-000000000901";
            let owner=nomifun_db::installation_owner_id(db.pool()).await.unwrap();
            nomifun_db::sqlx::query("INSERT INTO agent_sessions (agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
                VALUES (?,json_object('principal_kind','user','principal_id',?),'live','Channel reference fixture',0,0,'{}',1,1)")
                .bind(conversation).bind(&owner).execute(db.pool()).await.unwrap();
            manager.bind_conversation(&session.channel_session_id,conversation).await.unwrap();
            let original=AgentQuestionReplyTargetDto {turn_id:"0190f5fe-7c00-7a00-8000-000000000902".into(),question_seq:40};
            let port=Arc::new(InputPort {reject,replies:Default::default(),inputs:Default::default(),deliveries:Default::default()});
            let preferences:Arc<dyn nomifun_db::IClientPreferenceRepository>=Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(db.pool().clone()));
            let service=Arc::new(ChannelMessageService::new(port.clone(),Arc::new(crate::channel_settings::ChannelSettingsService::new(preferences)),repo.clone(),owner));
            let recorder=Arc::new(crate::stream_relay::MessageRecorder::new());let sender:Arc<dyn ChannelSender>=recorder.clone();
            if quoted && !guest {
                // A has already reached finite handoff. Its actual delivered
                // provider identity remains usable even if another Turn owns B.
                let relay=ChannelStreamRelay::new(RelayConfig {platform:crate::types::PluginType::Telegram,plugin_id:plugin.channel_plugin_id.clone(),
                    chat_id:"chat".into(),throttle_ms:0,conversation_id:conversation.into()},sender.clone(),service.stop_confirmations(),None)
                    .with_input_question_delivery(service.clone());
                let(tx,rx)=tokio::sync::broadcast::channel(8);
                tx.send(nomifun_ai_agent::AgentStreamEvent::InputQuestion(nomifun_ai_agent::protocol::events::InputQuestionEventData {
                    request:nomifun_agent_contracts::AgentUserInputRequest {request_id:"question-a".into(),question:"Complete authorization.".into(),remaining_work:"Verify result.".into()},
                    target:original.clone()})).unwrap();
                tx.send(nomifun_ai_agent::AgentStreamEvent::Finish(nomifun_ai_agent::protocol::events::FinishEventData {
                    stop_reason:Some(nomifun_ai_agent::protocol::events::TurnStopReason::InputRequired),..Default::default()})).unwrap();
                drop(tx);relay.run(rx).await;
                assert_eq!(port.deliveries.lock().unwrap()[0].provider_message_id,"msg-1");
            }
            let children=Arc::new(Mutex::new(Vec::new()));let shutdown=CancellationToken::new();
            let result=handle_dispatched(&service,&manager,&sender,&shutdown,&children,&session.channel_session_id,Some(conversation),"Authorization complete",
                quoted.then_some("msg-1"),crate::types::PluginType::Telegram,&plugin.channel_plugin_id,"chat","channel:original-inbound").await;
            let replies=port.replies.lock().unwrap().clone();
            if guest {
                assert!(replies.is_empty());assert!(result.is_none());assert!(repo.peek_next_queued(conversation).await.unwrap().is_some());
            } else if quoted {
                assert_eq!(replies,vec![("channel:original-inbound".into(),"Authorization complete".into(),original)]);
                assert!(port.inputs.lock().unwrap().is_empty(),"a rejected association never falls through to ordinary input");
                assert_eq!(result.is_some(),!reject);
            } else {
                assert!(result.is_some());assert_eq!(port.inputs.lock().unwrap().as_slice(),&[("channel:original-inbound".into(),"Authorization complete".into())]);
            }
            if !guest {assert!(repo.peek_next_queued(conversation).await.unwrap().is_none(),"ordinary intake has no channel busy FIFO");}
            shutdown.cancel();
            for task in children.lock().await.drain(..) {task.await.unwrap();}
        }
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
            chat_kind: crate::types::ChatKind::Unknown,
            mention_state: crate::types::MentionState::Unknown,
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
        let mut changed_chat_kind = redelivery.clone();
        changed_chat_kind.chat_kind = crate::types::ChatKind::Group;
        let mut changed_mention_state = redelivery.clone();
        changed_mention_state.mention_state = crate::types::MentionState::Mentioned;

        let original =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &original).unwrap();
        let redelivery =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &redelivery).unwrap();
        let changed_text =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_text).unwrap();
        let changed_reply =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_reply).unwrap();
        let changed_chat_kind =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_chat_kind).unwrap();
        let changed_mention_state =
            build_inbound_operation(owner.as_str(), plugin.as_str(), &changed_mention_state).unwrap();

        assert_eq!(original.receipt.operation_key, redelivery.receipt.operation_key);
        assert_eq!(original.receipt.payload_hash, redelivery.receipt.payload_hash);
        assert_eq!(original.receipt.operation_key, changed_text.receipt.operation_key);
        assert_ne!(original.receipt.payload_hash, changed_text.receipt.payload_hash);
        assert_ne!(original.receipt.payload_hash, changed_reply.receipt.payload_hash);
        assert_eq!(original.receipt.operation_key, changed_chat_kind.receipt.operation_key);
        assert_ne!(original.receipt.payload_hash, changed_chat_kind.receipt.payload_hash);
        assert_eq!(
            original.receipt.operation_key,
            changed_mention_state.receipt.operation_key
        );
        assert_ne!(
            original.receipt.payload_hash,
            changed_mention_state.receipt.payload_hash
        );
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
        let mut empty_sender = sample_inbound("provider-event-1", "chat-a", "hello");
        empty_sender.user.id = "   ".into();
        assert!(build_inbound_operation(owner.as_str(), plugin.as_str(), &empty_sender).is_err());

        let mut oversized_sender = sample_inbound("provider-event-1", "chat-a", "hello");
        oversized_sender.user.id = "x".repeat(513);
        assert!(build_inbound_operation(owner.as_str(), plugin.as_str(), &oversized_sender).is_err());
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
