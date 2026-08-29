use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::skill_types::{ContextModifier, PlanModeTransition};

use crate::compact::state::CompactState;
use crate::output::null_sink::NullSink;
use crate::plan::state::PlanState;

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

fn make_plan_engine() -> super::AgentEngine {
    let flag = Arc::new(AtomicBool::new(false));
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        workspace_root: std::path::PathBuf::from("."),
        completion_evidence_mode: super::CompletionEvidenceMode::LocalFingerprint,
        messages: vec![],
        system_prompt: String::new(),
        model: "test-model".to_string(),
        output_max_tokens: Some(4096),
        max_turns: Some(10),
        total_usage: Default::default(),
        thinking: None,
        compat: nomi_config::compat::ProviderCompat::anthropic_defaults(),
        hooks: None,
        session_manager: None,
        current_session: None,
        output: Arc::new(NullSink),
        current_msg_id: String::new(),
        protocol_writer: None,
        current_reasoning_effort: None,
        compact_config: nomi_config::compact::CompactConfig::default(),
        compact_state: CompactState::new(),
        plan_state: PlanState::default(),
        plan_active_flag: Some(flag),
        cache_detector: super::CacheBreakDetector::new(),
        compaction_level: nomi_compact::CompactionLevel::default(),
        toon_enabled: false,
        max_recent_images: 3,
        commands: crate::commands::default_registry(),
        goal: None,
        stagnation_guard: crate::loop_guard::StagnationGuard::new(
            crate::engine::STAGNATION_THRESHOLD,
        ),
        context_contributors: Vec::new(),
        steering_inbox: None,
        system_resource_inbox: None,
        process_supervisor: None,
        editable_turn: None,
        host_context: Default::default(),
    }
}

#[test]
fn enter_transition_activates_plan_mode_and_shared_flag() {
    let mut engine = make_plan_engine();
    let flag = engine.plan_active_flag.clone().unwrap();
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);
    assert!(engine.plan_state.is_active);
    assert!(flag.load(Ordering::Acquire));
}

#[test]
fn exit_transition_deactivates_plan_mode_and_shared_flag() {
    let mut engine = make_plan_engine();
    let flag = engine.plan_active_flag.clone().unwrap();
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Exit { plan_content: None }),
        ..Default::default()
    })]);
    assert!(!engine.plan_state.is_active);
    assert!(!flag.load(Ordering::Acquire));
}

#[test]
fn no_transition_preserves_plan_state_and_applies_other_modifiers() {
    let mut engine = make_plan_engine();
    engine.apply_context_modifiers(&[Some(ContextModifier {
        model: Some("new-model".into()),
        plan_mode_transition: None,
        ..Default::default()
    })]);
    assert_eq!(engine.model, "new-model");
    assert!(!engine.plan_state.is_active);
}

#[test]
fn transition_without_shared_flag_does_not_panic() {
    let mut engine = make_plan_engine();
    engine.plan_active_flag = None;
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);
    assert!(engine.plan_state.is_active);
}
