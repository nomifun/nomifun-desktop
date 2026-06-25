// Acceptance tests for Plan Mode tool filtering and prompt injection (Task 6.4).
//
// These tests are LOCAL (no LLM required) and verify that:
// - Tool registry filtering produces the correct tool sets for normal vs plan mode
// - System prompt correctly includes/excludes plan mode instructions

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use nomi_agent::context::{SystemPromptCache, build_system_prompt};
use nomi_agent::plan::tools::{EnterPlanModeTool, ExitPlanModeTool};
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_tools::registry::ToolRegistry;
use nomi_types::tool::ToolResult;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers: mock tool with configurable category
// ---------------------------------------------------------------------------

struct CategoryMockTool {
    tool_name: String,
    cat: ToolCategory,
}

impl CategoryMockTool {
    fn new(name: &str, cat: ToolCategory) -> Self {
        Self {
            tool_name: name.to_string(),
            cat,
        }
    }
}

#[async_trait]
impl Tool for CategoryMockTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "mock"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn category(&self) -> ToolCategory {
        self.cat
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult {
            content: String::new(),
            is_error: false,
            images: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TC-A3-01: Plan Mode tool filtering (LOCAL, no LLM)
// ---------------------------------------------------------------------------

#[test]
fn tc_a3_01_plan_mode_tool_filtering() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut registry = ToolRegistry::new();

    // Info category tools
    registry.register(Box::new(CategoryMockTool::new("Read", ToolCategory::Info)));
    registry.register(Box::new(CategoryMockTool::new("Grep", ToolCategory::Info)));
    registry.register(Box::new(EnterPlanModeTool::new(Arc::clone(&flag))));
    registry.register(Box::new(ExitPlanModeTool::new(Arc::clone(&flag))));

    // Edit category tool
    registry.register(Box::new(CategoryMockTool::new("Write", ToolCategory::Edit)));

    // Exec category tool
    registry.register(Box::new(CategoryMockTool::new("Bash", ToolCategory::Exec)));

    // --- Normal mode: all tools except ExitPlanMode ---
    let normal_defs = registry.to_tool_defs_filtered(|t| t.name() != "ExitPlanMode");
    let normal_names: Vec<&str> = normal_defs.iter().map(|d| d.name.as_str()).collect();

    assert!(
        !normal_names.contains(&"ExitPlanMode"),
        "ExitPlanMode should be excluded in normal mode"
    );
    assert!(
        normal_names.contains(&"Read"),
        "Read should be present in normal mode"
    );
    assert!(
        normal_names.contains(&"Grep"),
        "Grep should be present in normal mode"
    );
    assert!(
        normal_names.contains(&"Write"),
        "Write should be present in normal mode"
    );
    assert!(
        normal_names.contains(&"Bash"),
        "Bash should be present in normal mode"
    );
    assert!(
        normal_names.contains(&"EnterPlanMode"),
        "EnterPlanMode should be present in normal mode"
    );

    // --- Plan mode: only Info tools, excluding EnterPlanMode ---
    let plan_defs = registry.to_tool_defs_filtered(|t| {
        t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
    });
    let plan_names: Vec<&str> = plan_defs.iter().map(|d| d.name.as_str()).collect();

    // Info tools should be present
    assert!(
        plan_names.contains(&"Read"),
        "Read (Info) should be available in plan mode"
    );
    assert!(
        plan_names.contains(&"Grep"),
        "Grep (Info) should be available in plan mode"
    );
    assert!(
        plan_names.contains(&"ExitPlanMode"),
        "ExitPlanMode (Info) should be available in plan mode"
    );

    // EnterPlanMode should be excluded
    assert!(
        !plan_names.contains(&"EnterPlanMode"),
        "EnterPlanMode should be excluded in plan mode"
    );

    // Edit and Exec tools should be excluded
    assert!(
        !plan_names.contains(&"Write"),
        "Write (Edit) should be excluded in plan mode"
    );
    assert!(
        !plan_names.contains(&"Bash"),
        "Bash (Exec) should be excluded in plan mode"
    );
}

// ---------------------------------------------------------------------------
// TC-A3-02: Plan Mode system prompt injection (LOCAL, no LLM)
// ---------------------------------------------------------------------------

#[test]
fn tc_a3_02_plan_mode_system_prompt_injection() {
    // --- Plan mode active: prompt should contain plan mode instructions ---
    let active_prompt = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        None,
        true,
        false,
        false,
    );

    assert!(
        active_prompt.contains("# Plan Mode"),
        "active prompt should contain plan mode heading"
    );
    assert!(
        active_prompt.contains("Understand"),
        "active prompt should mention Phase 1: Understand"
    );
    assert!(
        active_prompt.contains("Design"),
        "active prompt should mention Phase 2: Design"
    );
    assert!(
        active_prompt.contains("Write the plan"),
        "active prompt should mention Phase 3: Write the plan"
    );
    assert!(
        active_prompt.contains("Submit for review"),
        "active prompt should mention Phase 4: Submit for review"
    );
    assert!(
        active_prompt.contains("Forbidden"),
        "active prompt should mention forbidden actions"
    );
    assert!(
        active_prompt.contains("ExitPlanMode"),
        "active prompt should reference ExitPlanMode tool"
    );

    // --- Plan mode inactive: prompt should NOT contain plan mode instructions ---
    let inactive_prompt = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        None,
        false,
        false,
        false,
    );

    assert!(
        !inactive_prompt.contains("# Plan Mode"),
        "inactive prompt should NOT contain plan mode heading"
    );
    assert!(
        !inactive_prompt.contains("Forbidden actions"),
        "inactive prompt should NOT contain forbidden actions section"
    );
}
