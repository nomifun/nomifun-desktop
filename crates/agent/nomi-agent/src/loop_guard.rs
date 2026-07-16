//! Loop-stagnation guard: detects either an identical completed tool outcome or
//! consecutive turns where every tool call failed. The guard first injects a
//! corrective nudge, then aborts if the no-progress cycle continues. Successful
//! polling is excluded before the guard observes the exact-outcome signal, so
//! legitimate external waiting remains unaffected while failed polling and
//! alternating failures stay bounded.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use nomi_types::message::ContentBlock;

/// Stable JSON encoding for loop signatures. Provider/object construction
/// order is not semantic, so object keys are sorted recursively; array order
/// remains significant.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let fields = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key)
                        .expect("serializing a JSON object key cannot fail");
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{fields}}}")
        }
        scalar => serde_json::to_string(scalar)
            .expect("serializing a scalar serde_json::Value cannot fail"),
    }
}

/// Canonical signature of a turn's tool calls: each call's name + serialized
/// input, sorted to be order-independent while preserving duplicate calls. The tool
/// `id` is deliberately excluded — it changes every turn, but two turns that
/// issue the same logical call(s) with the same arguments must collide. Returns
/// `None` when there are no tool calls (a text-only turn never stagnates).
pub fn tool_calls_signature(tool_calls: &[ContentBlock]) -> Option<String> {
    let mut sigs: Vec<String> = tool_calls
        .iter()
        .filter_map(|c| match c {
            ContentBlock::ToolUse { name, input, .. } => {
                Some(format!("{name}({})", canonical_json(input)))
            }
            _ => None,
        })
        .collect();
    if sigs.is_empty() {
        None
    } else {
        sigs.sort();
        Some(sigs.join("|"))
    }
}

/// Signature of a completed tool turn. IDs pair each result to its logical call
/// but are excluded from the final signature because providers generate fresh
/// IDs every turn. Result payloads (including images) are hashed so large
/// screenshots are not retained in guard state.
pub fn tool_outcome_signature(
    tool_calls: &[ContentBlock],
    tool_results: &[ContentBlock],
) -> Option<String> {
    tool_outcome_signature_filtered(tool_calls, tool_results, |_, _, _| true)
}

/// Build an outcome signature while excluding invocations whose unchanged
/// results represent normal external waiting (for example an empty
/// `write_stdin` poll). Results are paired to tracked calls by tool-use ID so a
/// mixed turn cannot hide a repeated non-polling action behind a polling call.
pub fn tool_outcome_signature_filtered<F>(
    tool_calls: &[ContentBlock],
    tool_results: &[ContentBlock],
    mut should_track: F,
) -> Option<String>
where
    F: FnMut(&str, &str, &serde_json::Value) -> bool,
{
    let tracked_calls: Vec<(&str, String)> = tool_calls
        .iter()
        .filter_map(|call| match call {
            ContentBlock::ToolUse { id, name, input, .. }
                if should_track(id, name, input) => Some((
                    id.as_str(),
                    format!(
                        "{name}({})",
                        canonical_json(input)
                    ),
                )),
            _ => None,
        })
        .collect();
    if tracked_calls.is_empty() {
        return None;
    }

    let mut result_hashes_by_id: HashMap<&str, Vec<u64>> = HashMap::new();
    for block in tool_results {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            images,
        } = block
        {
            let mut hasher = DefaultHasher::new();
            is_error.hash(&mut hasher);
            content.hash(&mut hasher);
            for image in images {
                image.media_type.hash(&mut hasher);
                image.data.hash(&mut hasher);
            }
            result_hashes_by_id
                .entry(tool_use_id.as_str())
                .or_default()
                .push(hasher.finish());
        }
    }
    for hashes in result_hashes_by_id.values_mut() {
        hashes.sort_unstable();
    }

    let mut paired_signatures: Vec<String> = tracked_calls
        .into_iter()
        .map(|(id, call)| {
            let results = result_hashes_by_id
                .get(id)
                .map(|hashes| {
                    hashes
                        .iter()
                        .map(|hash| format!("{hash:016x}"))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_else(|| "<missing>".to_string());
            format!("{call}=>{results}")
        })
        .collect();
    paired_signatures.sort();
    Some(paired_signatures.join("|"))
}

/// Whether a completed tool turn contained at least one result and every result
/// was an error. Mixed success/error turns are progress for the consecutive-
/// failure guard and therefore return `false`.
pub(crate) fn all_tool_results_failed(tool_results: &[ContentBlock]) -> bool {
    let mut saw_result = false;
    for block in tool_results {
        if let ContentBlock::ToolResult { is_error, .. } = block {
            saw_result = true;
            if !*is_error {
                return false;
            }
        }
    }
    saw_result
}

/// The guidance injected when stagnation is detected.
pub const STAGNATION_NUDGE: &str = "Loop guard: recent tool turns are making no progress: either \
the same call(s) keep returning the same outcome, or every tool call has failed repeatedly. Stop \
repeating the same action. Either try a materially different approach (different arguments, a \
different tool, or a different sub-problem), or stop and report what you have found and what is \
blocking you.";

/// Terminal diagnostic persisted in the transcript when the model ignores the
/// corrective nudge and continues making no progress.
pub const STAGNATION_ABORT: &str = "Stopped: tool turns continued making no progress after a \
loop-guard warning. No further automatic retries were attempted.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagnationAction {
    Continue,
    Nudge,
    Abort,
}

/// Tracks consecutive identical outcomes and consecutive all-failed turns.
pub struct StagnationGuard {
    nudge_threshold: usize,
    abort_threshold: usize,
    last: Option<String>,
    repeats: usize,
    consecutive_failures: usize,
}

impl StagnationGuard {
    /// `nudge_threshold` is the number of identical outcomes that triggers the
    /// corrective message. The same cycle is aborted after one additional full
    /// threshold, giving the model a bounded opportunity to recover.
    pub fn new(nudge_threshold: usize) -> Self {
        let nudge_threshold = nudge_threshold.max(2);
        Self {
            nudge_threshold,
            abort_threshold: nudge_threshold.saturating_mul(2),
            last: None,
            repeats: 0,
            consecutive_failures: 0,
        }
    }

    /// Start a fresh progress window, for example after a new user instruction.
    pub fn reset(&mut self) {
        self.last = None;
        self.repeats = 0;
        self.consecutive_failures = 0;
    }

    /// Observe this turn's completed tool outcome. A `None` signature or a
    /// changed call/result breaks the identical-outcome streak. Separately,
    /// `all_failed` tracks consecutive turns where every call failed, so
    /// alternating errors cannot evade the guard. Any successful result must
    /// be represented by `all_failed == false`.
    pub fn observe(
        &mut self,
        signature: Option<String>,
        all_failed: bool,
    ) -> StagnationAction {
        let repeated_outcome_action = if let Some(sig) = signature {
            if self.last.as_deref() == Some(sig.as_str()) {
                self.repeats += 1;
            } else {
                self.last = Some(sig);
                self.repeats = 1;
            }
            self.action_for(self.repeats)
        } else {
            self.last = None;
            self.repeats = 0;
            StagnationAction::Continue
        };

        let failed_outcome_action = if all_failed {
            self.consecutive_failures += 1;
            self.action_for(self.consecutive_failures)
        } else {
            self.consecutive_failures = 0;
            StagnationAction::Continue
        };

        if all_failed {
            // The all-failed counter is the stronger signal for this turn and
            // owns its single nudge/abort schedule. This avoids injecting a
            // second nudge if the exact-signature counter reaches its own
            // threshold later in the same failure streak.
            failed_outcome_action
        } else {
            repeated_outcome_action
        }
    }

    fn action_for(&self, count: usize) -> StagnationAction {
        if count >= self.abort_threshold {
            StagnationAction::Abort
        } else if count == self.nudge_threshold {
            StagnationAction::Nudge
        } else {
            StagnationAction::Continue
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

        let duplicate = vec![
            tool_use("Bash", json!({"command": "ls"}), "6"),
            tool_use("Bash", json!({"command": "ls"}), "7"),
        ];
        assert_ne!(
            tool_calls_signature(&a),
            tool_calls_signature(&duplicate),
            "call multiplicity must affect the signature"
        );
    }

    #[test]
    fn signature_canonicalizes_object_keys_recursively_but_preserves_array_order() {
        let left_input: serde_json::Value = serde_json::from_str(
            r#"{"outer":{"b":2,"a":1},"items":[{"y":2,"x":1},3]}"#,
        )
        .unwrap();
        let reordered_input: serde_json::Value = serde_json::from_str(
            r#"{"items":[{"x":1,"y":2},3],"outer":{"a":1,"b":2}}"#,
        )
        .unwrap();
        let reordered_array: serde_json::Value = serde_json::from_str(
            r#"{"outer":{"a":1,"b":2},"items":[3,{"x":1,"y":2}]}"#,
        )
        .unwrap();

        let left = vec![tool_use("Read", left_input, "left")];
        let same = vec![tool_use("Read", reordered_input, "right")];
        let different = vec![tool_use("Read", reordered_array, "array")];

        assert_eq!(tool_calls_signature(&left), tool_calls_signature(&same));
        assert_ne!(
            tool_calls_signature(&left),
            tool_calls_signature(&different),
            "array order remains semantically significant"
        );
    }

    #[test]
    fn text_only_turn_has_no_signature() {
        let blocks = vec![ContentBlock::Text { text: "hello".into() }];
        assert_eq!(tool_calls_signature(&blocks), None);
        assert_eq!(tool_calls_signature(&[]), None);
    }

    #[test]
    fn all_failed_requires_results_and_rejects_any_success() {
        let result = |id: &str, is_error: bool| ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: String::new(),
            is_error,
            images: Vec::new(),
        };

        assert!(!all_tool_results_failed(&[]));
        assert!(!all_tool_results_failed(&[ContentBlock::Text {
            text: "progress".into(),
        }]));
        assert!(all_tool_results_failed(&[
            result("a", true),
            result("b", true),
        ]));
        assert!(!all_tool_results_failed(&[
            result("a", true),
            result("b", false),
        ]));
    }

    #[test]
    fn nudges_then_aborts_consecutive_identical_outcomes() {
        let mut guard = StagnationGuard::new(3);
        let sig = Some("Bash(ls)".to_string());
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Nudge);
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(sig.clone(), false), StagnationAction::Abort);
    }

    #[test]
    fn alternating_all_failed_outcomes_nudge_then_abort() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("create({})=>error".to_string());
        let b = Some("update({})=>error".to_string());

        let actions = [a.clone(), b.clone(), a.clone(), b.clone(), a, b]
            .into_iter()
            .map(|signature| guard.observe(signature, true))
            .collect::<Vec<_>>();

        assert_eq!(actions[2], StagnationAction::Nudge);
        assert_eq!(actions[5], StagnationAction::Abort);
    }

    #[test]
    fn a_successful_outcome_resets_the_all_failed_streak() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("create({})=>error".to_string());
        let b = Some("update({})=>error".to_string());
        let success = Some("status({})=>success".to_string());

        assert_eq!(guard.observe(a.clone(), true), StagnationAction::Continue);
        assert_eq!(guard.observe(b.clone(), true), StagnationAction::Continue);
        // `false` represents a turn with at least one successful result. The
        // exact-outcome guard remains independent and still catches unchanged
        // successful non-polling cycles.
        assert_eq!(guard.observe(success, false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), true), StagnationAction::Continue);
        assert_eq!(guard.observe(b.clone(), true), StagnationAction::Continue);
        assert_eq!(guard.observe(a, true), StagnationAction::Nudge);
    }

    #[test]
    fn successful_explicit_polling_never_advances_either_streak() {
        let mut guard = StagnationGuard::new(3);
        for _ in 0..12 {
            assert_eq!(guard.observe(None, false), StagnationAction::Continue);
        }
    }

    #[test]
    fn a_different_turn_breaks_the_streak() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("Bash(ls)".to_string());
        let b = Some("Read(a)".to_string());
        guard.observe(a.clone(), false);
        guard.observe(a.clone(), false);
        guard.observe(b.clone(), false); // breaks the streak
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Nudge);
    }

    #[test]
    fn text_turn_between_identical_calls_breaks_streak() {
        let mut guard = StagnationGuard::new(3);
        let a = Some("Bash(ls)".to_string());
        guard.observe(a.clone(), false);
        guard.observe(a.clone(), false);
        assert_eq!(guard.observe(None, false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(a.clone(), false), StagnationAction::Nudge);
    }

    #[test]
    fn changing_tool_result_breaks_the_streak() {
        let first_calls = vec![tool_use("status", json!({}), "call-1")];
        let second_calls = vec![tool_use("status", json!({}), "call-2")];
        let result = |content: &str, id: &str| {
            vec![ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: content.to_string(),
                is_error: false,
                images: Vec::new(),
            }]
        };
        let first = tool_outcome_signature(&first_calls, &result("pending", "call-1"));
        let same_with_new_ids =
            tool_outcome_signature(&second_calls, &result("pending", "call-2"));
        assert_eq!(first, same_with_new_ids, "paired IDs must not affect the signature");

        let second = tool_outcome_signature(&second_calls, &result("complete", "call-2"));
        assert_ne!(first, second);

        let mut guard = StagnationGuard::new(3);
        assert_eq!(guard.observe(first.clone(), false), StagnationAction::Continue);
        assert_eq!(guard.observe(first, false), StagnationAction::Continue);
        assert_eq!(guard.observe(second, false), StagnationAction::Continue);
    }

    #[test]
    fn polling_calls_are_excluded_without_masking_other_calls() {
        let calls = vec![
            tool_use("write_stdin", json!({"session_id": 7}), "poll"),
            tool_use("update", json!({"id": 1}), "mutation"),
        ];
        let results = vec![
            ContentBlock::ToolResult {
                tool_use_id: "poll".into(),
                content: "still running".into(),
                is_error: false,
                images: Vec::new(),
            },
            ContentBlock::ToolResult {
                tool_use_id: "mutation".into(),
                content: "updated".into(),
                is_error: false,
                images: Vec::new(),
            },
        ];

        let mixed = tool_outcome_signature_filtered(&calls, &results, |_, name, _| {
            name != "write_stdin"
        })
        .expect("the non-polling mutation remains tracked");
        assert!(mixed.contains("update"));
        assert!(!mixed.contains("write_stdin"));

        let poll_only =
            tool_outcome_signature_filtered(&calls[..1], &results[..1], |_, name, _| {
                name != "write_stdin"
            });
        assert_eq!(poll_only, None);
    }

    #[test]
    fn successful_non_polling_cycles_nudge_then_abort() {
        let mut guard = StagnationGuard::new(3);
        let sig = Some("status({})".to_string());
        let actions: Vec<StagnationAction> =
            (0..6).map(|_| guard.observe(sig.clone(), false)).collect();
        assert_eq!(actions[2], StagnationAction::Nudge);
        assert_eq!(actions[5], StagnationAction::Abort);
    }

    #[test]
    fn swapped_results_are_not_the_same_outcome() {
        let calls = vec![
            tool_use("first", json!({}), "call-a"),
            tool_use("second", json!({}), "call-b"),
        ];
        let result = |id: &str, content: &str| ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            content: content.to_string(),
            is_error: false,
            images: Vec::new(),
        };
        let normal = vec![result("call-a", "A"), result("call-b", "B")];
        let swapped = vec![result("call-a", "B"), result("call-b", "A")];
        assert_ne!(
            tool_outcome_signature(&calls, &normal),
            tool_outcome_signature(&calls, &swapped)
        );
    }
}
