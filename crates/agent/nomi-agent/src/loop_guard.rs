//! Loop-stagnation guard: detects when the agent repeats the *identical* set of
//! tool calls turn after turn — a degenerate loop where it keeps doing the same
//! thing expecting a different result (a common "stuck" failure mode). When the
//! repeat streak crosses a threshold the engine injects a one-time nudge asking
//! the model to change approach. It never aborts (legitimate polling exists);
//! the hard `max_turns` cap remains the real backstop.

use std::collections::BTreeSet;

use nomi_types::message::ContentBlock;

/// Canonical signature of a turn's tool calls: each call's name + serialized
/// input, deduplicated and order-independent (a `BTreeSet`), joined. The tool
/// `id` is deliberately excluded — it changes every turn, but two turns that
/// issue the same logical call(s) with the same arguments must collide. Returns
/// `None` when there are no tool calls (a text-only turn never stagnates).
pub fn tool_calls_signature(tool_calls: &[ContentBlock]) -> Option<String> {
    let sigs: BTreeSet<String> = tool_calls
        .iter()
        .filter_map(|c| match c {
            ContentBlock::ToolUse { name, input, .. } => {
                Some(format!("{name}({})", serde_json::to_string(input).unwrap_or_default()))
            }
            _ => None,
        })
        .collect();
    if sigs.is_empty() {
        None
    } else {
        Some(sigs.into_iter().collect::<Vec<_>>().join("|"))
    }
}

/// The guidance injected when stagnation is detected.
pub const STAGNATION_NUDGE: &str = "Loop guard: you have issued the identical tool call(s) with the \
same arguments several turns in a row and the results are not changing. Stop repeating the same \
action. Either try a materially different approach (different arguments, a different tool, or a \
different sub-problem), or stop and report what you have found and what is blocking you.";

/// Tracks consecutive identical tool-call signatures and decides when to nudge.
pub struct StagnationGuard {
    threshold: usize,
    last: Option<String>,
    repeats: usize,
}

impl StagnationGuard {
    /// `threshold` = how many consecutive identical-signature turns trigger a
    /// nudge (e.g. 3: the 3rd identical turn fires).
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold: threshold.max(2),
            last: None,
            repeats: 0,
        }
    }

    /// Observe this turn's tool-call signature. Returns `true` exactly when the
    /// repeat streak reaches the threshold, at which point the streak resets so
    /// the next nudge requires a fresh streak (no nudge-every-turn spam). A
    /// `None` signature (text-only turn) breaks any streak.
    pub fn observe(&mut self, signature: Option<String>) -> bool {
        let Some(sig) = signature else {
            self.last = None;
            self.repeats = 0;
            return false;
        };
        if self.last.as_deref() == Some(sig.as_str()) {
            self.repeats += 1;
        } else {
            self.last = Some(sig);
            self.repeats = 1;
        }
        if self.repeats >= self.threshold {
            self.last = None;
            self.repeats = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_use(name: &str, input: serde_json::Value, id: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input,
            extra: None,
        }
    }

    #[test]
    fn signature_ignores_id_and_order_but_not_args() {
        let a = vec![tool_use("Bash", json!({"command": "ls"}), "id-1")];
        let b = vec![tool_use("Bash", json!({"command": "ls"}), "id-2-different")];
        assert_eq!(tool_calls_signature(&a), tool_calls_signature(&b), "id must not affect signature");

        let two_ab = vec![
            tool_use("Read", json!({"p": "a"}), "1"),
            tool_use("Grep", json!({"q": "x"}), "2"),
        ];
        let two_ba = vec![
            tool_use("Grep", json!({"q": "x"}), "3"),
            tool_use("Read", json!({"p": "a"}), "4"),
        ];
        assert_eq!(tool_calls_signature(&two_ab), tool_calls_signature(&two_ba), "order must not matter");

        let diff_args = vec![tool_use("Bash", json!({"command": "pwd"}), "5")];
        assert_ne!(tool_calls_signature(&a), tool_calls_signature(&diff_args), "different args must differ");
    }

    #[test]
    fn text_only_turn_has_no_signature() {
        let blocks = vec![ContentBlock::Text { text: "hello".into() }];
        assert_eq!(tool_calls_signature(&blocks), None);
        assert_eq!(tool_calls_signature(&[]), None);
    }

    #[test]
    fn fires_on_threshold_consecutive_identical_turns() {
        let mut guard = StagnationGuard::new(3);
        let sig = Some("Bash(ls)".to_string());
        assert!(!guard.observe(sig.clone()), "1st identical turn must not fire");
        assert!(!guard.observe(sig.clone()), "2nd must not fire");
        assert!(guard.observe(sig.clone()), "3rd identical turn fires the nudge");
    }

    #[test]
    fn resets_after_firing_so_no_per_turn_spam() {
        let mut guard = StagnationGuard::new(3);
        let sig = Some("Bash(ls)".to_string());
        guard.observe(sig.clone());
        guard.observe(sig.clone());
        assert!(guard.observe(sig.clone()), "fires on 3rd");
        // After firing, the streak resets — the 4th identical turn must NOT fire.
        assert!(!guard.observe(sig.clone()), "no nudge again immediately after firing");
        assert!(!guard.observe(sig.clone()));
        assert!(guard.observe(sig.clone()), "fires again only after a fresh full streak");
    }

    #[test]
    fn a_different_turn_breaks_the_streak() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("Bash(ls)".to_string());
        let b = Some("Read(a)".to_string());
        guard.observe(a.clone());
        guard.observe(a.clone());
        guard.observe(b.clone()); // breaks the streak
        assert!(!guard.observe(a.clone()), "streak restarted, 1st identical again");
        assert!(!guard.observe(a.clone()));
        assert!(guard.observe(a.clone()), "fires only after 3 fresh consecutive");
    }

    #[test]
    fn text_turn_between_identical_calls_breaks_streak() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("Bash(ls)".to_string());
        guard.observe(a.clone());
        guard.observe(a.clone());
        assert!(!guard.observe(None), "a text-only turn breaks the streak and never fires");
        assert!(!guard.observe(a.clone()));
        assert!(!guard.observe(a.clone()));
        assert!(guard.observe(a.clone()));
    }
}
