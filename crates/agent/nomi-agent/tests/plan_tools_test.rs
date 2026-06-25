//! Integration tests for Plan Mode tools (task 3.3).
//!
//! Tests are numbered to match the test-plan.md identifiers (TC-3.3-*).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use nomi_agent::plan::state::PlanState;
use nomi_agent::plan::tools::{EnterPlanModeTool, ExitPlanModeTool};
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::skill_types::PlanModeTransition;

// ---------------------------------------------------------------------------
// TC-3.3-01  PlanState initial state
// ---------------------------------------------------------------------------

#[test]
fn tc_3_3_01_plan_state_default_is_inactive() {
    let state = PlanState::default();
    assert!(!state.is_active);
}

#[test]
fn tc_3_3_01_plan_state_default_allow_list_empty() {
    let state = PlanState::default();
    assert!(state.pre_plan_allow_list.is_empty());
}

// ---------------------------------------------------------------------------
// TC-3.3-02  EnterPlanMode normal execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_3_3_02_enter_plan_mode_succeeds_when_not_active() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);

    let result = tool.execute(json!({})).await;

    assert!(!result.is_error, "should succeed when not in plan mode");
    assert!(
        result.content.contains("plan mode"),
        "confirmation message should mention plan mode"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-03  EnterPlanMode duplicate entry rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_3_3_03_enter_plan_mode_rejects_when_already_active() {
    let flag = Arc::new(AtomicBool::new(true));
    let tool = EnterPlanModeTool::new(flag);

    let result = tool.execute(json!({})).await;

    assert!(result.is_error, "should fail when already in plan mode");
    assert!(
        result.content.contains("Already in plan mode"),
        "error message should indicate already in plan mode"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-04  ExitPlanMode normal execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_3_3_04_exit_plan_mode_succeeds_when_active() {
    let flag = Arc::new(AtomicBool::new(true));
    let tool = ExitPlanModeTool::new(flag);

    let result = tool.execute(json!({})).await;

    assert!(!result.is_error, "should succeed when in plan mode");
    assert!(
        result.content.contains("Exited plan mode"),
        "confirmation message should mention exiting"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-05  ExitPlanMode when not in plan mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_3_3_05_exit_plan_mode_rejects_when_not_active() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = ExitPlanModeTool::new(flag);

    let result = tool.execute(json!({})).await;

    assert!(result.is_error, "should fail when not in plan mode");
    assert!(
        result.content.contains("Not in plan mode"),
        "error message should indicate not in plan mode"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-06  EnterPlanMode context_modifier returns Enter
// ---------------------------------------------------------------------------

#[test]
fn tc_3_3_06_enter_context_modifier_returns_enter_transition() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);

    let modifier = tool.context_modifier_for(&json!({}));

    assert!(modifier.is_some(), "should return a context modifier");
    let cm = modifier.unwrap();
    assert_eq!(
        cm.plan_mode_transition,
        Some(PlanModeTransition::Enter),
        "transition should be Enter"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-07  ExitPlanMode context_modifier returns Exit
// ---------------------------------------------------------------------------

#[test]
fn tc_3_3_07_exit_context_modifier_returns_exit_transition() {
    let flag = Arc::new(AtomicBool::new(true));
    let tool = ExitPlanModeTool::new(flag);

    let modifier = tool.context_modifier_for(&json!({}));

    assert!(modifier.is_some(), "should return a context modifier");
    let cm = modifier.unwrap();
    assert!(
        matches!(
            cm.plan_mode_transition,
            Some(PlanModeTransition::Exit { .. })
        ),
        "transition should be Exit variant"
    );
}

// ---------------------------------------------------------------------------
// TC-3.3-08  EnterPlanMode tool metadata
// ---------------------------------------------------------------------------

#[test]
fn tc_3_3_08_enter_tool_name() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);
    assert_eq!(tool.name(), "EnterPlanMode");
}

#[test]
fn tc_3_3_08_enter_tool_category_is_info() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);
    assert!(matches!(tool.category(), ToolCategory::Info));
}

#[test]
fn tc_3_3_08_enter_tool_is_concurrency_safe() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);
    assert!(tool.is_concurrency_safe(&json!({})));
}

#[test]
fn tc_3_3_08_enter_tool_schema_no_required_params() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = EnterPlanModeTool::new(flag);
    let schema = tool.input_schema();
    let required = schema["required"]
        .as_array()
        .expect("required should be an array");
    assert!(required.is_empty(), "no required parameters expected");
}

// ---------------------------------------------------------------------------
// TC-3.3-09  ExitPlanMode tool metadata
// ---------------------------------------------------------------------------

#[test]
fn tc_3_3_09_exit_tool_name() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = ExitPlanModeTool::new(flag);
    assert_eq!(tool.name(), "ExitPlanMode");
}

#[test]
fn tc_3_3_09_exit_tool_category_is_info() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = ExitPlanModeTool::new(flag);
    assert!(matches!(tool.category(), ToolCategory::Info));
}

#[test]
fn tc_3_3_09_exit_tool_is_concurrency_safe() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = ExitPlanModeTool::new(flag);
    assert!(tool.is_concurrency_safe(&json!({})));
}

#[test]
fn tc_3_3_09_exit_tool_schema_no_required_params() {
    let flag = Arc::new(AtomicBool::new(false));
    let tool = ExitPlanModeTool::new(flag);
    let schema = tool.input_schema();
    let required = schema["required"]
        .as_array()
        .expect("required should be an array");
    assert!(required.is_empty(), "no required parameters expected");
}

// ---------------------------------------------------------------------------
// Additional: full enter-exit cycle with shared flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enter_exit_cycle_with_shared_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let enter = EnterPlanModeTool::new(Arc::clone(&flag));
    let exit = ExitPlanModeTool::new(Arc::clone(&flag));

    // Phase 1: enter should succeed
    let r = enter.execute(json!({})).await;
    assert!(!r.is_error);

    // Simulate engine applying the transition
    flag.store(true, Ordering::Release);

    // Phase 2: double-enter should fail
    let r = enter.execute(json!({})).await;
    assert!(r.is_error);

    // Phase 3: exit should succeed
    let r = exit.execute(json!({})).await;
    assert!(!r.is_error);

    // Simulate engine applying the transition
    flag.store(false, Ordering::Release);

    // Phase 4: double-exit should fail
    let r = exit.execute(json!({})).await;
    assert!(r.is_error);

    // Phase 5: re-enter should succeed
    let r = enter.execute(json!({})).await;
    assert!(!r.is_error);
}

// ---------------------------------------------------------------------------
// Additional: context_modifier fields are default except plan_mode_transition
// ---------------------------------------------------------------------------

#[test]
fn enter_context_modifier_other_fields_are_default() {
    let tool = EnterPlanModeTool::new(Arc::new(AtomicBool::new(false)));
    let cm = tool.context_modifier_for(&json!({})).unwrap();
    assert!(cm.model.is_none());
    assert!(cm.effort.is_none());
    assert!(cm.allowed_tools.is_empty());
}

#[test]
fn exit_context_modifier_other_fields_are_default() {
    let tool = ExitPlanModeTool::new(Arc::new(AtomicBool::new(false)));
    let cm = tool.context_modifier_for(&json!({})).unwrap();
    assert!(cm.model.is_none());
    assert!(cm.effort.is_none());
    assert!(cm.allowed_tools.is_empty());
}
