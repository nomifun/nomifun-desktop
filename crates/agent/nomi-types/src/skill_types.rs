/// Effort level for a skill invocation or reasoning model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Max,
}

/// Signals a transition into or out of Plan Mode.
///
/// Returned via `ContextModifier::plan_mode_transition` from
/// the EnterPlanMode / ExitPlanMode tools.  The engine reads this
/// to toggle the plan-mode state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanModeTransition {
    /// Enter plan mode — restrict to read-only tools.
    Enter,
    /// Exit plan mode — optionally carrying the plan text.
    Exit { plan_content: Option<String> },
}

/// Convert EffortLevel to the string value expected by LlmRequest.reasoning_effort.
pub fn effort_to_string(level: EffortLevel) -> String {
    match level {
        EffortLevel::Low => "low".to_string(),
        EffortLevel::Medium => "medium".to_string(),
        EffortLevel::High => "high".to_string(),
        EffortLevel::Max => "max".to_string(),
    }
}

/// Overrides that a skill execution can apply to subsequent turns.
#[derive(Debug, Clone, Default)]
pub struct ContextModifier {
    /// Override model ID for subsequent LLM requests.
    /// None = no override.
    pub model: Option<String>,

    /// Override reasoning effort for subsequent LLM requests.
    pub effort: Option<EffortLevel>,

    /// Legacy metadata projection. Runtime engines must ignore this field; Skill
    /// tool lists may only narrow a forked invocation's exact tool set.
    pub allowed_tools: Vec<String>,

    /// Signal a plan-mode state transition (enter or exit).
    /// None = no transition.
    pub plan_mode_transition: Option<PlanModeTransition>,
}

impl ContextModifier {
    /// Returns true if this modifier carries no actual overrides.
    pub fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.effort.is_none()
            && self.allowed_tools.is_empty()
            && self.plan_mode_transition.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_modifier_default_has_no_plan_transition() {
        let cm = ContextModifier::default();
        assert!(cm.plan_mode_transition.is_none());
        assert!(cm.is_empty());
    }

    #[test]
    fn context_modifier_with_plan_transition_is_not_empty() {
        let cm = ContextModifier {
            plan_mode_transition: Some(PlanModeTransition::Enter),
            ..Default::default()
        };
        assert!(!cm.is_empty());
    }

    // PlanModeTransition construction/Debug/equality are covered by the
    // public integration tests in tests/plan_mode_transition_test.rs.

    #[test]
    fn context_modifier_existing_fields_unaffected() {
        // Verify that adding plan_mode_transition doesn't break existing usage
        let cm = ContextModifier {
            model: Some("test-model".into()),
            effort: Some(EffortLevel::High),
            allowed_tools: vec!["Bash".into()],
            plan_mode_transition: None,
        };
        assert!(!cm.is_empty());
        assert_eq!(cm.model.as_deref(), Some("test-model"));
        assert_eq!(cm.effort, Some(EffortLevel::High));
        assert_eq!(cm.allowed_tools, vec!["Bash".to_string()]);
        assert!(cm.plan_mode_transition.is_none());
    }
}
