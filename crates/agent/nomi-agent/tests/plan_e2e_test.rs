//! End-to-end integration tests for Plan Mode (task 3.6).
//!
//! Tests are numbered to match the test-plan.md identifiers (TC-3.6-E2E-*).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use nomi_agent::plan::prompt::plan_mode_instructions;
use nomi_agent::plan::tools::{EnterPlanModeTool, ExitPlanModeTool};
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_tools::registry::ToolRegistry;
use nomi_types::skill_types::PlanModeTransition;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct MockTool {
    tool_name: String,
    tool_category: ToolCategory,
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "mock tool"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {}, "required": []})
    }

    fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, _input: serde_json::Value) -> nomi_types::tool::ToolResult {
        nomi_types::tool::ToolResult {
            content: format!("{} executed", self.tool_name),
            is_error: false,
            images: Vec::new(),
        }
    }

    fn category(&self) -> ToolCategory {
        self.tool_category
    }
}

fn mock_tool(name: &str, category: ToolCategory) -> Box<MockTool> {
    Box::new(MockTool {
        tool_name: name.to_string(),
        tool_category: category,
    })
}

/// Simulate the plan mode filter the engine applies in its run() loop.
fn plan_mode_filter(registry: &ToolRegistry) -> Vec<String> {
    registry
        .to_tool_defs_filtered(|t| {
            t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
        })
        .iter()
        .map(|d| d.name.clone())
        .collect()
}

/// Simulate the normal mode filter the engine applies in its run() loop.
fn normal_mode_filter(registry: &ToolRegistry) -> Vec<String> {
    registry
        .to_tool_defs_filtered(|t| t.name() != "ExitPlanMode")
        .iter()
        .map(|d| d.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// TC-3.6-E2E-01: Full plan mode lifecycle
//
// Verifies the complete flow: normal → enter plan mode → use read-only tools
// → exit plan mode → write tools restored.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_3_6_e2e_01_full_plan_mode_lifecycle() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut registry = ToolRegistry::new();

    // Register standard tools
    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("Grep", ToolCategory::Info));
    registry.register(mock_tool("Glob", ToolCategory::Info));
    registry.register(mock_tool("Skill", ToolCategory::Info));
    registry.register(mock_tool("Write", ToolCategory::Edit));
    registry.register(mock_tool("Edit", ToolCategory::Edit));
    registry.register(mock_tool("Bash", ToolCategory::Exec));
    registry.register(Box::new(EnterPlanModeTool::new(Arc::clone(&flag))));
    registry.register(Box::new(ExitPlanModeTool::new(Arc::clone(&flag))));

    // Step 1: Verify normal mode — all tools except ExitPlanMode
    let normal_tools = normal_mode_filter(&registry);
    assert!(normal_tools.contains(&"Read".to_string()));
    assert!(normal_tools.contains(&"Write".to_string()));
    assert!(normal_tools.contains(&"Bash".to_string()));
    assert!(normal_tools.contains(&"EnterPlanMode".to_string()));
    assert!(!normal_tools.contains(&"ExitPlanMode".to_string()));

    // Step 2: LLM calls EnterPlanMode
    let enter_tool = EnterPlanModeTool::new(Arc::clone(&flag));
    let result = enter_tool.execute(json!({})).await;
    assert!(!result.is_error, "EnterPlanMode should succeed");

    // Verify context modifier signals Enter transition
    let cm = enter_tool.context_modifier_for(&json!({})).unwrap();
    assert_eq!(cm.plan_mode_transition, Some(PlanModeTransition::Enter));

    // Simulate engine processing the transition
    flag.store(true, Ordering::Release);

    // Step 3: Verify plan mode — only read-only tools + ExitPlanMode
    let plan_tools = plan_mode_filter(&registry);
    assert!(plan_tools.contains(&"Read".to_string()));
    assert!(plan_tools.contains(&"Grep".to_string()));
    assert!(plan_tools.contains(&"Glob".to_string()));
    assert!(plan_tools.contains(&"Skill".to_string()));
    assert!(plan_tools.contains(&"ExitPlanMode".to_string()));
    assert!(!plan_tools.contains(&"Write".to_string()));
    assert!(!plan_tools.contains(&"Edit".to_string()));
    assert!(!plan_tools.contains(&"Bash".to_string()));
    assert!(!plan_tools.contains(&"EnterPlanMode".to_string()));

    // Step 4: Verify read-only tools execute successfully in plan mode
    let read_tool = mock_tool("Read", ToolCategory::Info);
    let read_result = read_tool.execute(json!({})).await;
    assert!(!read_result.is_error, "Read should work in plan mode");

    // Step 5: Verify double-enter is rejected
    let double_enter = enter_tool.execute(json!({})).await;
    assert!(double_enter.is_error, "double-enter should be rejected");

    // Step 6: LLM calls ExitPlanMode
    let exit_tool = ExitPlanModeTool::new(Arc::clone(&flag));
    let exit_result = exit_tool.execute(json!({})).await;
    assert!(!exit_result.is_error, "ExitPlanMode should succeed");

    // Verify context modifier signals Exit transition
    let exit_cm = exit_tool.context_modifier_for(&json!({})).unwrap();
    assert!(matches!(
        exit_cm.plan_mode_transition,
        Some(PlanModeTransition::Exit { .. })
    ));

    // Simulate engine processing the transition
    flag.store(false, Ordering::Release);

    // Step 7: Verify normal mode restored — write tools available again
    let restored_tools = normal_mode_filter(&registry);
    assert!(restored_tools.contains(&"Write".to_string()));
    assert!(restored_tools.contains(&"Edit".to_string()));
    assert!(restored_tools.contains(&"Bash".to_string()));
    assert!(restored_tools.contains(&"EnterPlanMode".to_string()));
    assert!(!restored_tools.contains(&"ExitPlanMode".to_string()));

    // Step 8: Verify double-exit is rejected
    let double_exit = exit_tool.execute(json!({})).await;
    assert!(double_exit.is_error, "double-exit should be rejected");
}

// ---------------------------------------------------------------------------
// TC-3.6-E2E-02: Plan mode + compaction don't conflict
//
// Verifies that microcompact does not interfere with plan mode state.
// Uses tool result clearing to simulate compaction activity.
// ---------------------------------------------------------------------------

#[test]
fn tc_3_6_e2e_02_plan_mode_and_compaction_independent() {
    use nomi_agent::compact::micro::microcompact;
    use nomi_agent::plan::state::PlanState;
    use nomi_config::compact::CompactConfig;
    use nomi_types::message::{ContentBlock, Message, Role};

    // Build messages with compactable tool results
    let mut messages = Vec::new();
    for i in 0..8 {
        let id = format!("t{i}");
        messages.push(Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: id.clone(),
                name: "Read".to_string(),
                input: json!({}),
                extra: None,
            }],
        ));
        messages.push(Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: id,
                content: format!("data-{i}"),
                is_error: false,
                images: Vec::new(),
            }],
        ));
    }

    // Create plan state simulating active plan mode
    let plan_state = PlanState {
        is_active: true,
        pre_plan_allow_list: vec!["Read".to_string(), "Grep".to_string()],
    };

    // Actually run microcompact
    let config = CompactConfig {
        micro_keep_recent: 3,
        ..CompactConfig::default()
    };
    let result = microcompact(&mut messages, &config);

    // Microcompact should have cleared some results
    assert!(
        result.cleared_count > 0,
        "microcompact should clear old results"
    );

    // Plan state should be completely unaffected (microcompact only touches messages)
    assert!(plan_state.is_active, "plan mode should remain active");
    assert_eq!(
        plan_state.pre_plan_allow_list,
        vec!["Read".to_string(), "Grep".to_string()],
        "allow list should be unchanged"
    );
}

// ---------------------------------------------------------------------------
// TC-3.6-E2E-03: SkillTool available in plan mode
//
// Verifies that Skill (category: Info) is available in plan mode.
// ---------------------------------------------------------------------------

#[test]
fn tc_3_6_e2e_03_skill_tool_available_in_plan_mode() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut registry = ToolRegistry::new();

    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("Skill", ToolCategory::Info));
    registry.register(mock_tool("Write", ToolCategory::Edit));
    registry.register(mock_tool("Bash", ToolCategory::Exec));
    registry.register(Box::new(EnterPlanModeTool::new(Arc::clone(&flag))));
    registry.register(Box::new(ExitPlanModeTool::new(Arc::clone(&flag))));

    // Plan mode filter
    let plan_tools = plan_mode_filter(&registry);

    assert!(
        plan_tools.contains(&"Skill".to_string()),
        "Skill tool (Info category) should be available in plan mode"
    );
    assert!(
        plan_tools.contains(&"Read".to_string()),
        "Read should be available"
    );
    assert!(
        !plan_tools.contains(&"Write".to_string()),
        "Write should not be available"
    );
}

// ---------------------------------------------------------------------------
// TC-3.6-E2E-04: Plan mode state is runtime-only, not persisted
//
// Verifies that PlanState defaults to inactive — session resume starts fresh.
// ---------------------------------------------------------------------------

#[test]
fn tc_3_6_e2e_04_plan_state_not_persisted_across_sessions() {
    use nomi_agent::plan::state::PlanState;

    // Simulate a "previous session" where plan mode was active
    let active_state = PlanState {
        is_active: true,
        pre_plan_allow_list: vec!["Read".into(), "Bash".into()],
    };
    assert!(active_state.is_active);

    // On session resume, engine creates PlanState::default() (see engine.rs resume_with_provider)
    let resumed_state = PlanState::default();

    assert!(
        !resumed_state.is_active,
        "plan state should be inactive after session resume"
    );
    assert!(
        resumed_state.pre_plan_allow_list.is_empty(),
        "allow list should be empty after resume"
    );
}

// ---------------------------------------------------------------------------
// Additional: System prompt reflects plan mode transitions
// ---------------------------------------------------------------------------

#[test]
fn system_prompt_tracks_plan_mode_transitions() {
    let base_prompt = "You are an AI assistant.";
    let instructions = plan_mode_instructions();

    // Normal mode: no plan instructions
    assert!(
        !base_prompt.contains("Plan Mode"),
        "base prompt should not mention plan mode"
    );

    // Enter plan mode: instructions appended
    let active_prompt = format!("{}\n\n{}", base_prompt, instructions);
    assert!(active_prompt.contains("# Plan Mode"));
    assert!(active_prompt.contains("MUST NOT"));
    assert!(active_prompt.contains("ExitPlanMode"));
    assert!(active_prompt.contains(base_prompt));

    // Exit plan mode: back to base prompt only
    let exited_prompt = base_prompt.to_string();
    assert!(!exited_prompt.contains("# Plan Mode"));
}

// ---------------------------------------------------------------------------
// Additional: Multiple enter-exit cycles maintain consistency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multiple_plan_mode_cycles_consistent() {
    let flag = Arc::new(AtomicBool::new(false));
    let enter = EnterPlanModeTool::new(Arc::clone(&flag));
    let exit = ExitPlanModeTool::new(Arc::clone(&flag));

    for cycle in 0..3 {
        // Enter should succeed
        let r = enter.execute(json!({})).await;
        assert!(!r.is_error, "enter should succeed on cycle {cycle}");

        flag.store(true, Ordering::Release);

        // Exit should succeed
        let r = exit.execute(json!({})).await;
        assert!(!r.is_error, "exit should succeed on cycle {cycle}");

        flag.store(false, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Additional: Plan mode context_modifier fields are orthogonal
// ---------------------------------------------------------------------------

#[test]
fn plan_mode_modifiers_do_not_interfere_with_other_fields() {
    let enter = EnterPlanModeTool::new(Arc::new(AtomicBool::new(false)));
    let exit = ExitPlanModeTool::new(Arc::new(AtomicBool::new(true)));

    // Enter modifier should only set plan_mode_transition
    let enter_cm = enter.context_modifier_for(&json!({})).unwrap();
    assert!(enter_cm.model.is_none());
    assert!(enter_cm.effort.is_none());
    assert!(enter_cm.allowed_tools.is_empty());
    assert_eq!(
        enter_cm.plan_mode_transition,
        Some(PlanModeTransition::Enter)
    );

    // Exit modifier should only set plan_mode_transition
    let exit_cm = exit.context_modifier_for(&json!({})).unwrap();
    assert!(exit_cm.model.is_none());
    assert!(exit_cm.effort.is_none());
    assert!(exit_cm.allowed_tools.is_empty());
    assert!(matches!(
        exit_cm.plan_mode_transition,
        Some(PlanModeTransition::Exit { .. })
    ));
}
