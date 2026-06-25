//! In-memory goal state for a single engine run. P0: not persisted, no SQLite.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    Active,
    Complete,
    Blocked,
}

/// Per-session goal state. Lives in engine memory for the lifetime of one
/// `engine.run()`; lost on restart (degrades to a plain session, no data loss).
#[derive(Debug, Clone)]
pub struct GoalState {
    /// The objective text (provided at session start).
    pub objective: String,
    /// Current status. Only `Active` continues.
    pub status: GoalStatus,
    /// Cap on automatic continuations (anti-runaway). Default 8.
    pub max_auto_continuations: usize,
    /// How many automatic continuations have fired so far.
    pub auto_continuations: usize,
    /// Threshold (in goal turns) before `blocked` is allowed. Rendered into the
    /// continuation prompt; P0 only constrains the model via the prompt.
    pub blocked_threshold: usize,
}

impl GoalState {
    pub fn new(objective: String, max_auto_continuations: usize) -> Self {
        Self {
            objective,
            status: GoalStatus::Active,
            max_auto_continuations,
            auto_continuations: 0,
            blocked_threshold: 3,
        }
    }

    /// Whether continuation should still fire: Active and under the cap.
    pub fn should_continue(&self) -> bool {
        self.status == GoalStatus::Active && self.auto_continuations < self.max_auto_continuations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_under_cap_continues() {
        let g = GoalState::new("do X".into(), 8);
        assert!(g.should_continue());
    }

    #[test]
    fn completed_does_not_continue() {
        let mut g = GoalState::new("do X".into(), 8);
        g.status = GoalStatus::Complete;
        assert!(!g.should_continue());
    }

    #[test]
    fn blocked_does_not_continue() {
        let mut g = GoalState::new("do X".into(), 8);
        g.status = GoalStatus::Blocked;
        assert!(!g.should_continue());
    }

    #[test]
    fn cap_stops_continuation() {
        let mut g = GoalState::new("do X".into(), 2);
        g.auto_continuations = 2;
        assert!(!g.should_continue());
    }
}
