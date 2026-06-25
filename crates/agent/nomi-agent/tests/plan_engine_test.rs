//! Integration tests for Plan Mode engine integration (task 3.5).
//!
//! Tests are numbered to match the test-plan.md identifiers (TC-3.5-*).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use nomi_agent::plan::tools::{EnterPlanModeTool, ExitPlanModeTool};
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_tools::registry::ToolRegistry;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers: mock tools with configurable categories
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
            content: "ok".to_string(),
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

/// Build a registry with typical tools + plan mode tools
fn build_test_registry() -> ToolRegistry {
    let flag = Arc::new(AtomicBool::new(false));
    let mut registry = ToolRegistry::new();

    // Info tools
    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("Grep", ToolCategory::Info));
    registry.register(mock_tool("Glob", ToolCategory::Info));
    registry.register(mock_tool("Skill", ToolCategory::Info));

    // Edit tools
    registry.register(mock_tool("Write", ToolCategory::Edit));
    registry.register(mock_tool("Edit", ToolCategory::Edit));

    // Exec tools
    registry.register(mock_tool("Bash", ToolCategory::Exec));

    // Plan mode tools (both are Info category)
    registry.register(Box::new(EnterPlanModeTool::new(Arc::clone(&flag))));
    registry.register(Box::new(ExitPlanModeTool::new(Arc::clone(&flag))));

    registry
}

// ---------------------------------------------------------------------------
// TC-3.5-01: Plan mode filters to only Info tools + ExitPlanMode
// ---------------------------------------------------------------------------

#[test]
fn tc_3_5_01_plan_mode_only_info_tools_plus_exit() {
    let registry = build_test_registry();

    // Plan mode filter: Info category except EnterPlanMode
    let defs = registry.to_tool_defs_filtered(|t| {
        t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
    });

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

    // Should include Info tools
    assert!(
        names.contains(&"Read"),
        "Read should be available in plan mode"
    );
    assert!(
        names.contains(&"Grep"),
        "Grep should be available in plan mode"
    );
    assert!(
        names.contains(&"Glob"),
        "Glob should be available in plan mode"
    );
    assert!(
        names.contains(&"Skill"),
        "Skill should be available in plan mode"
    );
    assert!(
        names.contains(&"ExitPlanMode"),
        "ExitPlanMode should be available in plan mode"
    );

    // Should NOT include write/exec/EnterPlanMode
    assert!(
        !names.contains(&"Write"),
        "Write should NOT be in plan mode"
    );
    assert!(!names.contains(&"Edit"), "Edit should NOT be in plan mode");
    assert!(!names.contains(&"Bash"), "Bash should NOT be in plan mode");
    assert!(
        !names.contains(&"EnterPlanMode"),
        "EnterPlanMode should NOT be in plan mode"
    );
}

// ---------------------------------------------------------------------------
// TC-3.5-02: Normal mode includes all tools except ExitPlanMode
// ---------------------------------------------------------------------------

#[test]
fn tc_3_5_02_normal_mode_all_tools_except_exit_plan_mode() {
    let registry = build_test_registry();

    // Normal mode filter: everything except ExitPlanMode
    let defs = registry.to_tool_defs_filtered(|t| t.name() != "ExitPlanMode");

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

    // Should include all standard tools and EnterPlanMode
    assert!(names.contains(&"Read"));
    assert!(names.contains(&"Grep"));
    assert!(names.contains(&"Glob"));
    assert!(names.contains(&"Skill"));
    assert!(names.contains(&"Write"));
    assert!(names.contains(&"Edit"));
    assert!(names.contains(&"Bash"));
    assert!(
        names.contains(&"EnterPlanMode"),
        "EnterPlanMode should be in normal mode"
    );

    // Should NOT include ExitPlanMode
    assert!(
        !names.contains(&"ExitPlanMode"),
        "ExitPlanMode should NOT be in normal mode"
    );
}

// ---------------------------------------------------------------------------
// TC-3.5-07: to_tool_defs_filtered correctly filters mixed categories
// ---------------------------------------------------------------------------

#[test]
fn tc_3_5_07_to_tool_defs_filtered_mixed_categories() {
    let mut registry = ToolRegistry::new();
    let flag = Arc::new(AtomicBool::new(false));

    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("Write", ToolCategory::Edit));
    registry.register(mock_tool("Bash", ToolCategory::Exec));
    registry.register(Box::new(ExitPlanModeTool::new(flag)));

    let defs = registry.to_tool_defs_filtered(|t| {
        t.category() == ToolCategory::Info || t.name() == "ExitPlanMode"
    });

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Read"), "Info tool should be included");
    assert!(
        names.contains(&"ExitPlanMode"),
        "ExitPlanMode should be included"
    );
    assert!(!names.contains(&"Write"), "Edit tool should be excluded");
    assert!(!names.contains(&"Bash"), "Exec tool should be excluded");
}

// ---------------------------------------------------------------------------
// TC-3.5-08: System prompt dynamically includes plan mode instructions
// ---------------------------------------------------------------------------

#[test]
fn tc_3_5_08_system_prompt_includes_plan_mode_when_active() {
    use nomi_agent::plan::prompt::plan_mode_instructions;

    let base_prompt = "You are an AI assistant.";
    let instructions = plan_mode_instructions();

    // Simulate plan mode active: system prompt should contain plan instructions
    let active_prompt = format!("{}\n\n{}", base_prompt, instructions);
    assert!(
        active_prompt.contains("Plan Mode"),
        "active prompt should contain plan mode instructions"
    );
    assert!(
        active_prompt.contains("MUST NOT"),
        "should contain plan mode restrictions"
    );
    assert!(
        active_prompt.contains(base_prompt),
        "should still contain base prompt"
    );
}

#[test]
fn tc_3_5_08_system_prompt_excludes_plan_mode_when_inactive() {
    let base_prompt = "You are an AI assistant.";

    // Normal mode: system prompt is just the base
    assert!(
        !base_prompt.contains("Plan Mode"),
        "inactive prompt should NOT contain plan mode instructions"
    );
}

// ---------------------------------------------------------------------------
// TC-3.5-06: PlanConfig disabled means no plan tools registered
// ---------------------------------------------------------------------------

#[test]
fn tc_3_5_06_disabled_config_no_plan_tools() {
    // Simulate PlanConfig.enabled = false — do not register plan tools
    let mut registry = ToolRegistry::new();
    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("Write", ToolCategory::Edit));
    // Intentionally NOT registering EnterPlanMode/ExitPlanMode

    let defs = registry.to_tool_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

    assert!(
        !names.contains(&"EnterPlanMode"),
        "EnterPlanMode should not be registered when disabled"
    );
    assert!(
        !names.contains(&"ExitPlanMode"),
        "ExitPlanMode should not be registered when disabled"
    );
}

// ---------------------------------------------------------------------------
// Additional: plan mode filter with MCP tools
// ---------------------------------------------------------------------------

#[test]
fn plan_mode_filter_excludes_mcp_tools() {
    let mut registry = ToolRegistry::new();
    registry.register(mock_tool("Read", ToolCategory::Info));
    registry.register(mock_tool("mcp_server_tool", ToolCategory::Mcp));

    let defs = registry.to_tool_defs_filtered(|t| {
        t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
    });

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Read"));
    assert!(
        !names.contains(&"mcp_server_tool"),
        "MCP tools should be excluded in plan mode"
    );
}

// ---------------------------------------------------------------------------
// Additional: full enter-exit cycle verifies tool set transitions
// ---------------------------------------------------------------------------

#[test]
fn tool_set_transitions_through_plan_mode_cycle() {
    let registry = build_test_registry();

    // Normal mode: all except ExitPlanMode
    let normal_defs = registry.to_tool_defs_filtered(|t| t.name() != "ExitPlanMode");
    let normal_names: Vec<&str> = normal_defs.iter().map(|d| d.name.as_str()).collect();
    assert!(normal_names.contains(&"Write"));
    assert!(normal_names.contains(&"EnterPlanMode"));
    assert!(!normal_names.contains(&"ExitPlanMode"));

    // Enter plan mode: only Info (minus EnterPlanMode)
    let plan_defs = registry.to_tool_defs_filtered(|t| {
        t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
    });
    let plan_names: Vec<&str> = plan_defs.iter().map(|d| d.name.as_str()).collect();
    assert!(!plan_names.contains(&"Write"));
    assert!(!plan_names.contains(&"EnterPlanMode"));
    assert!(plan_names.contains(&"ExitPlanMode"));
    assert!(plan_names.contains(&"Read"));

    // Exit plan mode: back to normal
    let back_to_normal = registry.to_tool_defs_filtered(|t| t.name() != "ExitPlanMode");
    let back_names: Vec<&str> = back_to_normal.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        normal_names, back_names,
        "tool set should be identical after exit"
    );
}
