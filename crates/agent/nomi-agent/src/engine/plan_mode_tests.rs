// ---------------------------------------------------------------------------
// Phase 3 tests — plan mode integration in apply_context_modifiers()
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::skill_types::{ContextModifier, PlanModeTransition};

use crate::compact::state::CompactState;
use crate::confirm::ToolConfirmer;
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

fn make_plan_engine(allow_list: Vec<String>) -> super::AgentEngine {
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
        confirmer: Arc::new(Mutex::new(ToolConfirmer::new(true, allow_list.clone()))),
        hooks: None,
        session_manager: None,
        current_session: None,
        output: Arc::new(NullSink),
        current_msg_id: String::new(),
        approval_manager: None,
        protocol_writer: None,
        allow_list,
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
        stagnation_guard: crate::loop_guard::StagnationGuard::new(crate::engine::STAGNATION_THRESHOLD),
        context_contributors: Vec::new(),
        steering_inbox: None,
        system_resource_inbox: None,
        process_supervisor: None,
        editable_turn: None,
        host_context: Default::default(),
    }
}

// --- TC-3.5-03: Enter transition activates plan mode ---

#[test]
fn enter_transition_activates_plan_mode() {
    let mut engine = make_plan_engine(vec!["Read".into(), "Bash".into()]);
    let modifiers = vec![Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })];

    engine.apply_context_modifiers(&modifiers);

    assert!(engine.plan_state.is_active, "plan mode should be active");
    assert_eq!(
        engine.plan_state.pre_plan_allow_list,
        vec!["Read".to_string(), "Bash".to_string()],
        "pre_plan_allow_list should capture original allow_list"
    );
}

// --- TC-3.5-03 supplement: shared flag updated on enter ---

#[test]
fn enter_transition_updates_shared_flag() {
    let mut engine = make_plan_engine(vec![]);
    let flag = engine.plan_active_flag.clone().unwrap();
    assert!(!flag.load(Ordering::Acquire));

    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);

    assert!(flag.load(Ordering::Acquire), "shared flag should be true");
}

// --- TC-3.5-04: Exit transition deactivates plan mode and restores allow_list ---

#[test]
fn exit_transition_deactivates_and_restores() {
    let mut engine = make_plan_engine(vec!["Read".into(), "Bash".into()]);

    // Enter plan mode first
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);
    assert!(engine.plan_state.is_active);

    // Modify allow_list while in plan mode (simulating a skill adding tools)
    engine.allow_list.push("NewTool".into());

    // Exit plan mode
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Exit { plan_content: None }),
        ..Default::default()
    })]);

    assert!(!engine.plan_state.is_active, "plan mode should be inactive");
    assert_eq!(
        engine.allow_list,
        vec!["Read".to_string(), "Bash".to_string()],
        "allow_list should be restored to pre-plan state"
    );
}

// --- TC-3.5-04 supplement: shared flag updated on exit ---

#[test]
fn exit_transition_updates_shared_flag() {
    let mut engine = make_plan_engine(vec![]);
    let flag = engine.plan_active_flag.clone().unwrap();

    // Enter
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);
    assert!(flag.load(Ordering::Acquire));

    // Exit
    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Exit { plan_content: None }),
        ..Default::default()
    })]);
    assert!(
        !flag.load(Ordering::Acquire),
        "shared flag should be false after exit"
    );
}

// --- TC-3.5-05: No transition does not affect plan state ---

#[test]
fn no_transition_does_not_affect_plan_state() {
    let mut engine = make_plan_engine(vec![]);

    engine.apply_context_modifiers(&[Some(ContextModifier {
        model: Some("new-model".into()),
        plan_mode_transition: None,
        ..Default::default()
    })]);

    assert_eq!(engine.model, "new-model");
    assert!(
        !engine.plan_state.is_active,
        "plan state should remain inactive"
    );
}

// --- Enter + other modifiers applied together ---

#[test]
fn enter_with_model_override_both_applied() {
    let mut engine = make_plan_engine(vec![]);

    engine.apply_context_modifiers(&[Some(ContextModifier {
        model: Some("planning-model".into()),
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);

    assert!(engine.plan_state.is_active);
    assert_eq!(engine.model, "planning-model");
}

// --- No plan_active_flag set does not panic ---

#[test]
fn enter_without_flag_does_not_panic() {
    let mut engine = make_plan_engine(vec![]);
    engine.plan_active_flag = None;

    engine.apply_context_modifiers(&[Some(ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    })]);

    assert!(engine.plan_state.is_active);
}
