// ---------------------------------------------------------------------------
// Phase 6 tests — apply_context_modifiers()
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::skill_types::{ContextModifier, EffortLevel};

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

fn make_engine(model: &str, allow_list: Vec<String>) -> super::AgentEngine {
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        messages: vec![],
        system_prompt: String::new(),
        model: model.to_string(),
        max_tokens: 4096,
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
        compact_state: super::CompactState::new(),
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

#[test]
fn tc_6_21_model_override_applied() {
    let mut engine = make_engine("original-model", vec![]);
    let modifiers = vec![Some(ContextModifier {
        model: Some("override-model".to_string()),
        ..Default::default()
    })];
    engine.apply_context_modifiers(&modifiers);
    assert_eq!(engine.model, "override-model");
}

#[test]
fn tc_6_22_effort_override_applied() {
    let mut engine = make_engine("m", vec![]);
    let modifiers = vec![Some(ContextModifier {
        effort: Some(EffortLevel::High),
        ..Default::default()
    })];
    engine.apply_context_modifiers(&modifiers);
    assert_eq!(engine.current_reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn tc_6_22b_effort_all_variants() {
    for (level, expected) in [
        (EffortLevel::Low, "low"),
        (EffortLevel::Medium, "medium"),
        (EffortLevel::High, "high"),
        (EffortLevel::Max, "max"),
    ] {
        let mut engine = make_engine("m", vec![]);
        engine.apply_context_modifiers(&[Some(ContextModifier {
            effort: Some(level),
            ..Default::default()
        })]);
        assert_eq!(
            engine.current_reasoning_effort.as_deref(),
            Some(expected),
            "EffortLevel::{level:?} should map to {expected:?}"
        );
    }
}

#[test]
fn tc_6_23_allowed_tools_no_duplicates() {
    let mut engine = make_engine("m", vec!["Bash".to_string()]);
    let modifiers = vec![Some(ContextModifier {
        allowed_tools: vec!["Bash".to_string(), "Read".to_string()],
        ..Default::default()
    })];
    engine.apply_context_modifiers(&modifiers);
    let bash_count = engine
        .allow_list
        .iter()
        .filter(|t| t.as_str() == "Bash")
        .count();
    assert_eq!(bash_count, 1, "Bash should appear exactly once");
    assert!(engine.allow_list.contains(&"Read".to_string()));
}

#[test]
fn tc_6_24_none_modifiers_skipped() {
    let mut engine = make_engine("original", vec![]);
    engine.apply_context_modifiers(&[None, None]);
    assert_eq!(engine.model, "original");
    assert!(engine.current_reasoning_effort.is_none());
}

#[test]
fn tc_6_25_empty_modifiers_no_change() {
    let mut engine = make_engine("current-model", vec![]);
    engine.apply_context_modifiers(&[]);
    assert_eq!(engine.model, "current-model");
    assert!(engine.allow_list.is_empty());
}

#[test]
fn tc_6_26_none_model_does_not_overwrite() {
    let mut engine = make_engine("current-model", vec![]);
    engine.apply_context_modifiers(&[Some(ContextModifier {
        allowed_tools: vec!["Bash".to_string()],
        ..Default::default()
    })]);
    assert_eq!(engine.model, "current-model");
    assert!(engine.allow_list.contains(&"Bash".to_string()));
}

#[test]
fn tc_6_27_multiple_modifiers_stacked() {
    let mut engine = make_engine("initial", vec![]);
    let modifiers = vec![
        Some(ContextModifier {
            model: Some("model-a".to_string()),
            allowed_tools: vec!["Bash".to_string()],
            ..Default::default()
        }),
        Some(ContextModifier {
            model: Some("model-b".to_string()),
            allowed_tools: vec!["Read".to_string()],
            ..Default::default()
        }),
    ];
    engine.apply_context_modifiers(&modifiers);
    assert_eq!(engine.model, "model-b", "last model wins");
    assert!(engine.allow_list.contains(&"Bash".to_string()));
    assert!(engine.allow_list.contains(&"Read".to_string()));
}

#[test]
fn tc_6_28_modifier_applied_after_tool_execution_not_during() {
    let mut engine = make_engine("original", vec![]);
    let model_before = engine.model.clone();
    let modifiers = vec![Some(ContextModifier {
        model: Some("new-model".to_string()),
        ..Default::default()
    })];
    assert_eq!(engine.model, model_before);
    engine.apply_context_modifiers(&modifiers);
    assert_eq!(engine.model, "new-model");
    assert_eq!(model_before, "original");
}
