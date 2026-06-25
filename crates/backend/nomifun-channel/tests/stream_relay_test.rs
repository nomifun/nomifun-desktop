use std::sync::Arc;

use nomifun_ai_agent::AgentStreamEvent;
use nomifun_ai_agent::protocol::events::{
    AcpPermissionEventData, AcpPermissionOptionData, AcpPermissionOptionKind, AcpPermissionRequestData,
    AcpPermissionToolCall, ErrorEventData, FinishEventData, TextEventData, ToolCallEventData, ToolCallStatus,
};
use nomifun_channel::pending_decision::PendingDecisionStore;
use nomifun_channel::stream_relay::{ChannelSender, ChannelStreamRelay, MessageRecorder, RelayConfig};
use nomifun_channel::types::{ParseMode, PluginType};
use tokio::sync::broadcast;

/// Builds a relay with a fresh (unshared) pending-decision store. Tests that
/// need to inspect the store pass their own via [`relay_with_store`].
fn relay(config: RelayConfig, sender: Arc<dyn ChannelSender>) -> ChannelStreamRelay {
    ChannelStreamRelay::new(config, sender, PendingDecisionStore::new())
}

/// Builds a relay sharing the caller's pending-decision store.
fn relay_with_store(
    config: RelayConfig,
    sender: Arc<dyn ChannelSender>,
    store: Arc<PendingDecisionStore>,
) -> ChannelStreamRelay {
    ChannelStreamRelay::new(config, sender, store)
}

// ── RelayConfig construction ─────────────────────────────────────

#[test]
fn relay_config_fields() {
    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "123".into(),
        throttle_ms: 500,
        conversation_id: "conv-1".into(),
    };
    assert_eq!(config.throttle_ms, 500);
    assert_eq!(config.plugin_id, "telegram");
    assert_eq!(config.conversation_id, "conv-1");
}

// ── Full relay run with mock ChannelSender ───────────────────────

#[tokio::test]
async fn relay_sends_thinking_then_final_message() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());

    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "Hello".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: " World".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None }))
        .unwrap();

    relay.run(rx).await;

    let sends = recorder.take_sends();
    assert!(!sends.is_empty());
    assert!(sends[0].text.as_deref().unwrap().contains("Thinking"));

    let edits = recorder.take_edits();
    let last = edits.last().unwrap();
    assert!(last.text.as_deref().unwrap().contains("Hello World"));
    assert!(last.buttons.is_some());
}

#[tokio::test]
async fn relay_handles_error_event() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Error(ErrorEventData::legacy("timeout", None)))
        .unwrap();

    relay.run(rx).await;

    let edits = recorder.take_edits();
    let last = edits.last().unwrap();
    assert!(last.text.as_deref().unwrap().contains("timeout"));
}

#[tokio::test]
async fn weixin_flushes_pending_text_before_tool_call() {
    // Port of Nomi TS fix `406a62665` to the backend relay layer. On
    // WeChat, in-place editing is not supported, so a tool-status update
    // would otherwise overwrite any assistant text the user hasn't yet
    // seen. The relay should flush buffered text as an independent
    // send_message before rendering the tool-call indicator, matching the
    // TS WeixinPlugin.sendTextNow draft-flush behaviour.
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Weixin,
        plugin_id: "weixin".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000, // large throttle so the mid-stream edit doesn't fire
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "Here is the plan:".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None }))
        .unwrap();

    relay.run(rx).await;

    let sends = recorder.take_sends();
    // WeChat relay does NOT send a "Thinking..." placeholder. The first
    // send_message should be the flushed assistant text triggered by the
    // ToolCall event.
    assert!(!sends.is_empty(), "expected flush send_message, got {:?}", sends);
    let flushed = &sends[0];
    assert!(
        flushed.text.as_deref().unwrap().contains("Here is the plan"),
        "expected flushed text, got {:?}",
        flushed.text
    );
}

#[tokio::test]
async fn telegram_does_not_flush_text_before_tool_call() {
    // Non-WeChat platforms support edit_message, so the TS flush rule does
    // not apply — the relay should continue to edit the placeholder in
    // place without issuing a new send_message for the buffered text.
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "Here is the plan:".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None }))
        .unwrap();

    relay.run(rx).await;

    let sends = recorder.take_sends();
    // Only the "Thinking..." placeholder is sent — no flush on non-WeChat.
    assert_eq!(sends.len(), 1, "unexpected extra sends: {:?}", sends);
}

#[tokio::test]
async fn weixin_skips_flush_when_buffer_is_empty() {
    // Tool call before any assistant text should not trigger a blank flush.
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Weixin,
        plugin_id: "weixin".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData { session_id: None, stop_reason: None }))
        .unwrap();

    relay.run(rx).await;

    let sends = recorder.take_sends();
    // WeChat relay does NOT send Thinking placeholder, and with no buffered
    // text there should be zero sends (no flush needed).
    assert_eq!(sends.len(), 0, "no sends expected for empty buffer: {:?}", sends);
}

#[tokio::test]
async fn relay_handles_channel_closed() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "partial".into(),
        }))
        .unwrap();
    drop(event_tx);

    relay.run(rx).await;

    let edits = recorder.take_edits();
    assert!(!edits.is_empty());
    assert!(edits.last().unwrap().text.as_deref().unwrap().contains("partial"));
}

// ── Telegram parse mode (HTML formatter output must be declared) ─────

/// The formatter emits HTML for Telegram; streaming edits and the final
/// message must carry `parse_mode: HTML` or the tags render literally.
#[tokio::test]
async fn telegram_streaming_and_final_messages_use_html_parse_mode() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 0, // edit on every chunk so the streaming path is exercised
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "**bold** & <raw>".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: None,
        }))
        .unwrap();

    relay.run(rx).await;

    // The "Thinking..." placeholder is plain text — no parse mode.
    let sends = recorder.take_sends();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].parse_mode, None);

    let edits = recorder.take_edits();
    assert!(!edits.is_empty());
    for edit in &edits {
        assert_eq!(
            edit.parse_mode,
            Some(ParseMode::HTML),
            "telegram edit must declare HTML parse mode: {edit:?}"
        );
    }
    // Formatter output: markdown converted to tags, source &/< escaped.
    let final_text = edits.last().unwrap().text.as_deref().unwrap();
    assert!(final_text.contains("<b>bold</b>"), "got: {final_text}");
    assert!(final_text.contains("&amp;"), "got: {final_text}");
    assert!(final_text.contains("&lt;raw&gt;"), "got: {final_text}");
}

/// Tool-status edits show raw (unescaped) agent output, so they must stay
/// plain text even on Telegram.
#[tokio::test]
async fn telegram_tool_call_edit_stays_plain_text() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "call-1".into(),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: None,
        }))
        .unwrap();

    relay.run(rx).await;

    let edits = recorder.take_edits();
    let tool_edit = edits
        .iter()
        .find(|e| e.text.as_deref().is_some_and(|t| t.contains("read_file")))
        .expect("tool-status edit");
    assert_eq!(tool_edit.parse_mode, None);
}

/// Non-Telegram platforms receive markdown/plain text — parse mode stays
/// unset for them.
#[tokio::test]
async fn lark_messages_have_no_parse_mode() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());

    let config = RelayConfig {
        platform: PluginType::Lark,
        plugin_id: "lark".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 0,
        conversation_id: "conv-test".into(),
    };
    let relay = relay(config, recorder.clone());
    let rx = event_tx.subscribe();

    event_tx
        .send(AgentStreamEvent::Text(TextEventData {
            content: "**bold** text".into(),
        }))
        .unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: None,
        }))
        .unwrap();

    relay.run(rx).await;

    let edits = recorder.take_edits();
    assert!(!edits.is_empty());
    for edit in &edits {
        assert_eq!(edit.parse_mode, None, "lark edits must not set parse mode: {edit:?}");
    }
}

// ── Decision relay (Bug 1, Case A) ───────────────────────────────────

/// Builds an ACP permission-request event with two options.
fn acp_decision_event(call_id: &str, title: &str) -> AgentStreamEvent {
    AgentStreamEvent::AcpPermission(AcpPermissionEventData::Request(AcpPermissionRequestData {
        session_id: "s1".into(),
        tool_call: AcpPermissionToolCall {
            tool_call_id: call_id.into(),
            status: None,
            title: Some(title.into()),
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
    }))
}

/// A relayed decision is recorded in the shared store and forwarded as a
/// numbered text message (a new send, not an edit of the thinking card).
#[tokio::test]
async fn relay_forwards_decision_and_records_pending() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());
    let store = PendingDecisionStore::new();

    let config = RelayConfig {
        platform: PluginType::Telegram,
        plugin_id: "telegram".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000,
        conversation_id: "conv-dec".into(),
    };
    let relay = relay_with_store(config, recorder.clone(), Arc::clone(&store));
    let rx = event_tx.subscribe();

    event_tx.send(acp_decision_event("call-42", "Run rm -rf?")).unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: None,
        }))
        .unwrap();

    relay.run(rx).await;

    // A numbered decision message was sent as a new message.
    let sends = recorder.take_sends();
    let decision = sends
        .iter()
        .find(|m| m.text.as_deref().is_some_and(|t| t.contains("需要你的决策")))
        .expect("a numbered decision message must be sent");
    let text = decision.text.as_deref().unwrap();
    assert!(text.contains("Run rm -rf?"), "prompt present: {text}");
    assert!(text.contains("1. Allow once"), "first option numbered: {text}");
    assert!(text.contains("2. Reject"), "second option numbered: {text}");
    assert!(decision.buttons.is_none(), "decision is plain text, no buttons");

    // The pending decision is recorded against the conversation.
    let pending = store.peek("conv-dec").expect("decision recorded in store");
    assert_eq!(pending.call_id, "call-42");
    assert_eq!(pending.options.len(), 2);
    assert_eq!(pending.options[0].option_id, "allow");
    assert_eq!(pending.options[1].option_id, "reject");
}

/// WeChat (no edit support) also forwards the decision as a send_message.
#[tokio::test]
async fn weixin_relay_forwards_decision() {
    let (event_tx, _) = broadcast::channel::<AgentStreamEvent>(64);
    let recorder = Arc::new(MessageRecorder::new());
    let store = PendingDecisionStore::new();

    let config = RelayConfig {
        platform: PluginType::Weixin,
        plugin_id: "weixin".into(),
        chat_id: "chat_1".into(),
        throttle_ms: 10_000,
        conversation_id: "conv-wx".into(),
    };
    let relay = relay_with_store(config, recorder.clone(), Arc::clone(&store));
    let rx = event_tx.subscribe();

    event_tx.send(acp_decision_event("call-wx", "Proceed?")).unwrap();
    event_tx
        .send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: None,
        }))
        .unwrap();

    relay.run(rx).await;

    let sends = recorder.take_sends();
    assert!(
        sends
            .iter()
            .any(|m| m.text.as_deref().is_some_and(|t| t.contains("需要你的决策"))),
        "weixin relay must forward the decision: {sends:?}"
    );
    assert!(store.peek("conv-wx").is_some(), "decision recorded for weixin");
}
