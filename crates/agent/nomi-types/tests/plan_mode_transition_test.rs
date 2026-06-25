//! Black-box integration tests for PlanModeTransition and ContextModifier
//! extensions (TC-3.2-04, TC-3.2-05).

use nomi_types::skill_types::{ContextModifier, PlanModeTransition};

/// TC-3.2-04: ContextModifier without plan_mode_transition is backward-compatible.
/// Existing code that does not set plan_mode_transition should see None.
#[test]
fn tc_3_2_04_context_modifier_backward_compatible() {
    let cm = ContextModifier::default();
    assert!(cm.plan_mode_transition.is_none());
    assert!(cm.is_empty());

    // Existing fields still work
    let cm = ContextModifier {
        model: Some("test".to_string()),
        ..Default::default()
    };
    assert!(!cm.is_empty());
    assert!(cm.plan_mode_transition.is_none());
}

/// TC-3.2-05: PlanModeTransition variants construct correctly and Debug output
/// contains variant names.
#[test]
fn tc_3_2_05_plan_mode_transition_variants() {
    // Enter variant
    let enter = PlanModeTransition::Enter;
    let dbg = format!("{:?}", enter);
    assert!(dbg.contains("Enter"), "Debug should contain 'Enter'");

    // Exit with plan content
    let exit_with = PlanModeTransition::Exit {
        plan_content: Some("# My Plan\n1. Step one".to_string()),
    };
    let dbg = format!("{:?}", exit_with);
    assert!(dbg.contains("Exit"), "Debug should contain 'Exit'");
    assert!(dbg.contains("My Plan"), "Debug should contain plan text");

    // Exit without plan content
    let exit_without = PlanModeTransition::Exit { plan_content: None };
    let dbg = format!("{:?}", exit_without);
    assert!(dbg.contains("Exit"), "Debug should contain 'Exit'");
    assert!(dbg.contains("None"), "Debug should contain 'None'");
}

/// PlanModeTransition equality works correctly.
#[test]
fn plan_mode_transition_equality() {
    assert_eq!(PlanModeTransition::Enter, PlanModeTransition::Enter);

    assert_ne!(
        PlanModeTransition::Enter,
        PlanModeTransition::Exit { plan_content: None }
    );

    assert_eq!(
        PlanModeTransition::Exit {
            plan_content: Some("x".into())
        },
        PlanModeTransition::Exit {
            plan_content: Some("x".into())
        }
    );

    assert_ne!(
        PlanModeTransition::Exit {
            plan_content: Some("a".into())
        },
        PlanModeTransition::Exit {
            plan_content: Some("b".into())
        }
    );
}

/// ContextModifier with plan_mode_transition set is not empty.
#[test]
fn context_modifier_with_transition_not_empty() {
    let cm = ContextModifier {
        plan_mode_transition: Some(PlanModeTransition::Enter),
        ..Default::default()
    };
    assert!(!cm.is_empty());
}

/// ContextModifier with plan_mode_transition does not affect other fields.
#[test]
fn context_modifier_transition_independent_of_other_fields() {
    let cm = ContextModifier {
        model: Some("model-x".into()),
        plan_mode_transition: Some(PlanModeTransition::Exit {
            plan_content: Some("plan".into()),
        }),
        ..Default::default()
    };
    assert_eq!(cm.model.as_deref(), Some("model-x"));
    assert!(cm.effort.is_none());
    assert!(cm.allowed_tools.is_empty());
    assert!(cm.plan_mode_transition.is_some());
}
