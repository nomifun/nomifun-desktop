//! `ContextContributor` — the host-agnostic seam (design §3.5) that lets the
//! backend inject dynamic, per-turn context into the system prompt without the
//! engine hard-coding each source. The engine holds a list of contributors
//! (empty by default → behaviour byte-for-byte unchanged) and, at the start of
//! each turn, appends whatever they contribute to the system prompt.
//!
//! This is the foundation for turning "passive" platform features into "active"
//! injection (knowledge auto-RAG, inline memory, etc.) as registered
//! contributors rather than bespoke call-sites. It is purely additive: with no
//! contributors registered, `merge_pre_turn_context` returns the system prompt
//! unchanged.

use async_trait::async_trait;

/// A source of dynamic per-turn context. Implementations live in the backend
/// (host) and are registered onto the engine; the engine stays host-agnostic.
#[async_trait]
pub trait ContextContributor: Send + Sync {
    /// Context to add to the system prompt for the upcoming turn, or `None` to
    /// contribute nothing this turn. Called once per turn before the model call.
    async fn pre_turn_context(&self) -> Option<String>;

    /// A short stable label for diagnostics/telemetry.
    fn label(&self) -> &str {
        "context_contributor"
    }
}

/// Append non-empty contributor contributions to `system`, each under a blank
/// line, in registration order. Empty / all-`None` → `system` returned
/// unchanged (the zero-contributor fast path the engine relies on). Pure so the
/// merge rule is unit-testable without an engine.
pub fn merge_pre_turn_context(system: String, contributions: Vec<String>) -> String {
    let mut out = system;
    for c in contributions {
        let trimmed = c.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_contributions_returns_system_unchanged() {
        let sys = "SYSTEM PROMPT".to_string();
        assert_eq!(merge_pre_turn_context(sys.clone(), vec![]), sys);
        // All-empty contributions are also a no-op.
        assert_eq!(
            merge_pre_turn_context(sys.clone(), vec!["".into(), "   ".into()]),
            sys
        );
    }

    #[test]
    fn appends_non_empty_contributions_in_order() {
        let out = merge_pre_turn_context(
            "BASE".to_string(),
            vec!["[KB] hit".into(), "".into(), "[memory] fact".into()],
        );
        assert_eq!(out, "BASE\n\n[KB] hit\n\n[memory] fact");
    }

    #[test]
    fn empty_system_with_one_contribution_has_no_leading_blank() {
        let out = merge_pre_turn_context(String::new(), vec!["only".into()]);
        assert_eq!(out, "only");
    }

    #[tokio::test]
    async fn trait_object_contributes_through_merge() {
        struct Fixed(&'static str);
        #[async_trait]
        impl ContextContributor for Fixed {
            async fn pre_turn_context(&self) -> Option<String> {
                Some(self.0.to_string())
            }
        }
        let contributors: Vec<Box<dyn ContextContributor>> =
            vec![Box::new(Fixed("alpha")), Box::new(Fixed("beta"))];
        let mut contributions = Vec::new();
        for c in &contributors {
            if let Some(s) = c.pre_turn_context().await {
                contributions.push(s);
            }
        }
        assert_eq!(merge_pre_turn_context("S".into(), contributions), "S\n\nalpha\n\nbeta");
    }
}
