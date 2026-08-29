/// Runtime state for Plan Mode.
///
/// Tracks whether the agent is currently in plan mode.
#[derive(Debug, Clone, Default)]
pub struct PlanState {
    /// Whether plan mode is currently active.
    pub is_active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_inactive() {
        let state = PlanState::default();
        assert!(!state.is_active);
    }

    #[test]
    fn can_set_active() {
        let state = PlanState { is_active: true };
        assert!(state.is_active);
    }

    #[test]
    fn clone_produces_independent_copy() {
        let original = PlanState { is_active: true };
        let mut cloned = original.clone();
        cloned.is_active = false;

        // Original unchanged
        assert!(original.is_active);
        assert!(!cloned.is_active);
    }
}
