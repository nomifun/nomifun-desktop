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
}
