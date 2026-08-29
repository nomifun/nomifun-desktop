use std::sync::Arc;

use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::skill_types::{ContextModifier, EffortLevel};

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

fn make_engine(model: &str) -> super::AgentEngine {
    super::AgentEngine {
        provider: Arc::new(NullProvider),
        tools: ToolRegistry::new(),
        workspace_root: std::path::PathBuf::from("."),
        completion_evidence_mode: super::CompletionEvidenceMode::LocalFingerprint,
        messages: vec![],
        system_prompt: String::new(),
        model: model.to_string(),
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
        compact_state: super::CompactState::new(),
        plan_state: Default::default(),
        plan_active_flag: None,
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
fn model_override_is_applied() {
    let mut engine = make_engine("original-model");
    engine.apply_context_modifiers(&[Some(ContextModifier {
        model: Some("override-model".to_string()),
        ..Default::default()
    })]);
    assert_eq!(engine.model, "override-model");
}

#[test]
fn effort_override_is_applied_for_every_level() {
    for (level, expected) in [
        (EffortLevel::Low, "low"),
        (EffortLevel::Medium, "medium"),
        (EffortLevel::High, "high"),
        (EffortLevel::Max, "max"),
    ] {
        let mut engine = make_engine("model");
        engine.apply_context_modifiers(&[Some(ContextModifier {
            effort: Some(level),
            ..Default::default()
        })]);
        assert_eq!(
            engine.current_reasoning_effort.as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn skill_tool_metadata_does_not_widen_parent_scope() {
    let mut engine = make_engine("model");
    engine.apply_context_modifiers(&[Some(ContextModifier {
        allowed_tools: vec!["Bash".to_string(), "Read".to_string()],
        ..Default::default()
    })]);
    assert_eq!(engine.model, "model");
    assert!(engine.current_reasoning_effort.is_none());
}

#[test]
fn absent_modifiers_are_noops() {
    let mut engine = make_engine("original");
    engine.apply_context_modifiers(&[None, None]);
    assert_eq!(engine.model, "original");
    assert!(engine.current_reasoning_effort.is_none());
}

#[test]
fn multiple_modifiers_apply_in_order() {
    let mut engine = make_engine("initial");
    engine.apply_context_modifiers(&[
        Some(ContextModifier {
            model: Some("model-a".to_string()),
            effort: Some(EffortLevel::Low),
            ..Default::default()
        }),
        Some(ContextModifier {
            model: Some("model-b".to_string()),
            effort: Some(EffortLevel::High),
            ..Default::default()
        }),
    ]);
    assert_eq!(engine.model, "model-b");
    assert_eq!(engine.current_reasoning_effort.as_deref(), Some("high"));
}
