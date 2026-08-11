use std::sync::{Arc, Mutex};

use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role};

use crate::compact::state::CompactState;
use crate::confirm::ToolConfirmer;
use crate::output::null_sink::NullSink;

struct NullProvider;
#[async_trait::async_trait]
impl LlmProvider for NullProvider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
}

fn make_engine() -> super::AgentEngine {
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        messages: vec![],
        system_prompt: String::new(),
        model: "test-model".to_string(),
        max_tokens: 4096,
        max_turns: Some(10),
        total_usage: Default::default(),
        thinking: None,
        compat: nomi_config::compat::ProviderCompat::anthropic_defaults(),
        confirmer: Arc::new(Mutex::new(ToolConfirmer::new(true, vec![]))),
        hooks: None,
        session_manager: None,
        current_session: None,
        output: Arc::new(NullSink),
        current_msg_id: String::new(),
        approval_manager: None,
        protocol_writer: None,
        allow_list: vec![],
        current_reasoning_effort: None,
        compact_config: nomi_config::compact::CompactConfig::default(),
        compact_state: CompactState::new(),
        plan_state: Default::default(),
        plan_active_flag: None,
        cache_detector: super::CacheBreakDetector::new(),
        compaction_level: nomi_compact::CompactionLevel::default(),
        toon_enabled: false,
        max_recent_images: 3,
        commands: crate::commands::default_registry(),
        goal: None,
        stagnation_guard: crate::loop_guard::StagnationGuard::new(crate::engine::STAGNATION_THRESHOLD),
        context_contributors: Vec::new(),
        steering_inbox: None,
        system_resource_inbox: None,
        process_supervisor: None,
        editable_turn: None,
        host_context: Default::default(),
    }
}

#[tokio::test]
async fn handle_command_quit() {
    let mut engine = make_engine();
    let result = engine.handle_command("/quit").await;
    assert!(matches!(
        result,
        Some(Ok(crate::commands::CommandResult::Exit))
    ));
}

#[tokio::test]
async fn handle_command_exit_alias() {
    let mut engine = make_engine();
    let result = engine.handle_command("/exit").await;
    assert!(matches!(
        result,
        Some(Ok(crate::commands::CommandResult::Exit))
    ));
}

#[tokio::test]
async fn handle_command_unknown() {
    let mut engine = make_engine();
    let result = engine.handle_command("/nonexistent").await;
    assert!(result.is_none());
}

#[tokio::test]
async fn handle_command_clear() {
    let mut engine = make_engine();
    engine.messages.push(Message::new(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello".to_string(),
        }],
    ));
    assert_eq!(engine.messages.len(), 1);

    let result = engine.handle_command("/clear").await;
    assert!(matches!(
        result,
        Some(Ok(crate::commands::CommandResult::Continue))
    ));
    assert!(engine.messages.is_empty());
    assert_eq!(engine.compact_state.last_input_tokens, 0);
}

#[tokio::test]
async fn handle_command_with_args() {
    let mut engine = make_engine();
    let result = engine.handle_command("/help compact").await;
    assert!(matches!(
        result,
        Some(Ok(crate::commands::CommandResult::Continue))
    ));
}

#[tokio::test]
async fn handle_command_not_a_command() {
    let mut engine = make_engine();
    let result = engine.handle_command("hello world").await;
    assert!(result.is_none());
}

#[tokio::test]
async fn execute_turn_intercepts_help_returns_zero_turns() {
    let mut engine = make_engine();
    let result = engine.execute_turn("/help", "msg-1").await.unwrap();
    assert_eq!(result.turns, 0);
    assert_eq!(result.usage.input_tokens, 0);
}

#[tokio::test]
async fn execute_turn_intercepts_quit_returns_user_aborted() {
    let mut engine = make_engine();
    let err = engine.execute_turn("/quit", "msg-1").await.unwrap_err();
    assert!(matches!(err, super::AgentError::UserAborted));
}

#[test]
fn slash_command_list_returns_all() {
    let engine = make_engine();
    let list = engine.slash_command_list();
    assert!(list.len() >= 4);
    let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"help"));
    assert!(names.contains(&"compact"));
    assert!(names.contains(&"clear"));
    assert!(names.contains(&"quit"));
}
