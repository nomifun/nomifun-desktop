use std::sync::{Arc, Mutex};

use nomi_types::message::{ContentBlock, Message, Role};

use crate::goal::state::GoalState;

const CONTINUATION_TEMPLATE: &str = include_str!("templates/continuation.md");

/// What a caller supplies to start a goal-driven session.
#[derive(Debug, Clone)]
pub struct GoalSpec {
    pub objective: String,
    pub max_auto_continuations: usize,
}

impl GoalSpec {
    pub fn new(objective: impl Into<String>, max_auto_continuations: usize) -> Self {
        Self {
            objective: objective.into(),
            max_auto_continuations,
        }
    }
}

/// Engine-side goal runtime: holds the shared state (also held by
/// `UpdateGoalTool`) and renders the continuation prompt.
pub struct GoalRuntime {
    state: Arc<Mutex<GoalState>>,
}

impl GoalRuntime {
    pub fn new(objective: String, max_auto_continuations: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(GoalState::new(objective, max_auto_continuations))),
        }
    }

    /// Clone the shared handle for injection into `UpdateGoalTool`.
    pub fn shared_state(&self) -> Arc<Mutex<GoalState>> {
        Arc::clone(&self.state)
    }

    /// Capture the shared goal state for accepted-turn rollback.
    pub fn snapshot_state(&self) -> GoalState {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Restore a captured goal state through the shared handle.
    ///
    /// The `Arc` is deliberately not replaced: `UpdateGoalTool` already holds a
    /// clone of it, so a rejected turn must write through the same allocation
    /// rather than leave the tool pointing at orphaned state.
    pub fn restore_state(&self, state: GoalState) {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
    }

    /// Called at the engine's natural-termination point. Returns `Some(message)`
    /// to inject a continuation and run another turn, or `None` to stop
    /// (goal reached a terminal state, or the auto-continuation cap was hit).
    pub fn maybe_continuation(&self) -> Option<Message> {
        let mut g = self.state.lock().unwrap();
        if !g.should_continue() {
            return None;
        }
        g.auto_continuations += 1;
        let prompt = render_continuation(&g.objective, g.blocked_threshold);
        Some(Message::now(
            Role::User,
            vec![ContentBlock::Text { text: prompt }],
        ))
    }
}

fn render_continuation(objective: &str, blocked_threshold: usize) -> String {
    CONTINUATION_TEMPLATE
        .replace("{{objective}}", objective)
        .replace("{{blocked_threshold}}", &blocked_threshold.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::state::GoalStatus;

    #[test]
    fn continuation_injects_until_cap() {
        let rt = GoalRuntime::new("ship the feature".into(), 2);
        // First two fire, third stops at cap.
        assert!(rt.maybe_continuation().is_some());
        assert!(rt.maybe_continuation().is_some());
        assert!(rt.maybe_continuation().is_none());
    }

    #[test]
    fn continuation_stops_when_completed() {
        let rt = GoalRuntime::new("ship the feature".into(), 8);
        rt.shared_state().lock().unwrap().status = GoalStatus::Complete;
        assert!(rt.maybe_continuation().is_none());
    }

    #[test]
    fn continuation_renders_objective_and_threshold() {
        let rt = GoalRuntime::new("migrate the database".into(), 8);
        let msg = rt.maybe_continuation().unwrap();
        let text = match &msg.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("migrate the database"));
        assert!(text.contains("连续 3 个目标轮次"));
        assert!(!text.contains("{{")); // all placeholders substituted
    }

    // A rejected turn must be able to undo the goal progress it claimed, and the
    // tool's shared handle has to observe the restored value.
    #[test]
    fn restore_state_writes_through_the_tool_handle() {
        let rt = GoalRuntime::new("ship the feature".into(), 8);
        let tool_handle = rt.shared_state();
        let root = rt.snapshot_state();

        assert!(rt.maybe_continuation().is_some());
        tool_handle.lock().unwrap().status = GoalStatus::Complete;

        rt.restore_state(root);

        let observed = tool_handle.lock().unwrap();
        assert_eq!(observed.status, GoalStatus::Active);
        assert_eq!(observed.auto_continuations, 0);
    }

    // The snapshot must be a value copy, not a view of the live state.
    #[test]
    fn snapshot_state_is_independent_of_later_progress() {
        let rt = GoalRuntime::new("ship the feature".into(), 8);
        let root = rt.snapshot_state();
        rt.shared_state().lock().unwrap().auto_continuations = 5;
        assert_eq!(root.auto_continuations, 0);
    }
}
