//! Native tools for a summoned-companion work session (spec §设计 B2/B3).
//!
//! A summon loads one companion's memories **read-only** into an ordinary work
//! conversation: the session reuses `RecallMemoriesTool` (companion_tools.rs)
//! over a read-only sink. There is no write-back path — `save_memory` is never
//! registered in a summoned session, and the confirmation-style
//! `propose_companion_memory` tool retired together with the 建议 feature that
//! stored and reviewed its proposal cards.
//!
//! Backend seams follow the `companion_tools.rs` pattern: `nomifun-companion`
//! injects concrete sinks; other hosts pass `None` and none of this exists.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context_contributor::ContextContributor;

/// Backend seam for the per-turn live memory-snapshot section of a summoned
/// session. Implementations resolve the summoned `memory_ids` against the
/// live store each turn (budgeted), so edits to a memory propagate naturally.
/// `None` = nothing to inject this turn (empty selection or resolver failure —
/// implementations log their own errors; a snapshot must never fail the turn).
#[async_trait]
pub trait SummonContextSink: Send + Sync {
    async fn resolve_context(&self) -> Option<String>;
}

/// Per-turn live memory-snapshot injection for a summoned session (spec §B2:
/// memories are re-resolved from the store each turn under a budget; the
/// conversation row never copies memory content). Empty resolution → `None`
/// (engine's zero-cost no-op path, same as `CompanionSkillContributor`).
pub struct SummonContextContributor {
    sink: Arc<dyn SummonContextSink>,
}

impl SummonContextContributor {
    pub fn new(sink: Arc<dyn SummonContextSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl ContextContributor for SummonContextContributor {
    async fn pre_turn_context(&self) -> Option<String> {
        self.sink.resolve_context().await.filter(|s| !s.trim().is_empty())
    }

    fn label(&self) -> &str {
        "companion_summon"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedContextSink(Option<String>);

    #[async_trait]
    impl SummonContextSink for FixedContextSink {
        async fn resolve_context(&self) -> Option<String> {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn summon_contributor_is_noop_when_empty() {
        let c = SummonContextContributor::new(Arc::new(FixedContextSink(None)));
        assert!(c.pre_turn_context().await.is_none());
        let blank = SummonContextContributor::new(Arc::new(FixedContextSink(Some("  ".into()))));
        assert!(blank.pre_turn_context().await.is_none());
    }

    #[tokio::test]
    async fn summon_contributor_passes_snapshot_through() {
        let c = SummonContextContributor::new(Arc::new(FixedContextSink(Some(
            "## 召唤的伙伴记忆（只读参考）\n- [preference] 深烘焙".into(),
        ))));
        let out = c.pre_turn_context().await.unwrap();
        assert!(out.contains("召唤的伙伴记忆"));
        assert_eq!(c.label(), "companion_summon");
    }
}
