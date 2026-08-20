//! Resumable-round state: what the engine carries from a pass the provider cut
//! off at its output ceiling into the next attempt at the same requirement.
//!
//! # Why this exists
//!
//! A provider that stops at `StopReason::MaxTokens` did not answer the request.
//! Continuing its half-written draft is not recoverable — the draft may end
//! mid-token, mid-JSON, or mid-sentence, and asking a model to continue such a
//! string reliably produces a fresh restatement rather than a completion. The
//! only sound recovery is to drop the draft and re-attempt the ORIGINAL
//! requirement, telling the model what it already accomplished so it does not
//! redo work or re-declare a plan.
//!
//! [`RoundLedger`] is that "what you already accomplished", and every field in
//! it is a machine observation:
//!
//! * plan steps come from the model's own accepted `update_plan` snapshot,
//! * effects come from dispatched tool results the engine itself executed,
//! * cutoffs come from provider events for calls the ceiling truncated.
//!
//! Nothing here is scraped from transcript prose or produced by a summarization
//! pass. A ledger that guessed would put false claims into a system prompt.
//!
//! # Lifetime
//!
//! [`RoundState`] is deliberately a stack local of the engine's turn function
//! and is **not** persisted. The restart happens entirely inside one
//! `execute_turn_inner` call, and the ledger's only consumer is the system
//! section rendered for the next pass of that same call. Persisting it would
//! buy nothing today while adding a `host_context` key that outlives
//! conversation resets, needs a staleness fence, and interacts with the
//! editable-turn rewind. When a user-initiated resume needs to reuse a ledger
//! across processes, that consumer can define the fence it actually requires.

use nomi_protocol::events::ToolCategory;
use nomi_tools::update_plan::StepStatus;
use nomi_types::message::ContentBlock;

/// Total attempts at one accepted requirement, including the first.
///
/// 3 preserves the envelope the deleted host-side auto-continue had
/// (`MAX_TRUNCATION_AUTO_CONTINUES = 2` → 3 provider passes) while making all
/// three passes useful instead of spending two of them re-generating prose
/// after an English "continue where you left off" instruction.
pub const MAX_ROUND_ATTEMPTS: usize = 3;

/// How many entries the rendered system section lists per block, newest kept.
///
/// Bounds the PROMPT only. Counts come from the monotonic totals on
/// [`RoundLedger`], never from the length of a window — a turn with 30 effects
/// must not report zero progress because the window dropped the successful ones.
///
/// Applies to the plan and cutoff blocks for the same reason: neither has a
/// trusted upper bound. The plan is the model's own snapshot (it can declare 500
/// steps) and the cutoff list is the provider's (it can truncate many parallel
/// calls). One truncated round must not crowd out the system prompt it is
/// attached to.
const MAX_RENDERED_LINES: usize = 24;

/// Byte budget for one rendered effect label. Load-bearing: `Tool::describe`'s
/// default implementation dumps the entire input JSON, so an unbounded label
/// from an MCP tool could dwarf the rest of the prompt.
const EFFECT_LABEL_BYTES: usize = 160;

/// Byte budget for one rendered plan step. Model-authored text, so unbounded.
const STEP_TEXT_BYTES: usize = 200;

/// Whether a successful call of this category is evidence that the turn changed
/// state.
///
/// `ToolCategory::Mcp` is deliberately absent: it is dead in production (the
/// MCP proxy classifies by annotation into `Info` or `Exec`), so matching it
/// would only add a branch that never fires.
///
/// This is NOT "the tool had no side effect" — in this codebase `Info` means
/// "no approval gate", and several `Info` tools (companion memory writes,
/// knowledge and skill tools) persist data. It is specifically "the kind of
/// effect a coding turn is asked to produce".
pub(crate) fn is_state_changing(category: ToolCategory) -> bool {
    matches!(
        category,
        ToolCategory::Edit | ToolCategory::Exec | ToolCategory::Irreversible
    )
}

/// One step of the model's own most recent accepted plan snapshot.
#[derive(Debug, Clone)]
pub struct LedgerStep {
    pub step: String,
    pub status: StepStatus,
}

/// One dispatched state-changing tool result.
#[derive(Debug, Clone)]
pub struct LedgerEffect {
    pub tool: String,
    pub label: String,
    pub ok: bool,
}

/// A tool call the provider began streaming and the ceiling cut off. Never
/// executed; recorded so the next attempt knows what was reached for.
#[derive(Debug, Clone)]
pub struct LedgerCutoff {
    pub tool: String,
    pub argument_bytes: usize,
    /// Whether the truncated call was a state-changing one. Resolved against
    /// the registry at record time, while the advertised tool set is in scope.
    pub state_changing: bool,
}

/// Machine-observed progress within one turn, carried across restarts.
#[derive(Debug, Clone, Default)]
pub struct RoundLedger {
    /// The model's most recent accepted `update_plan` snapshot. Full-snapshot
    /// semantics: replaced, never merged, because `update_plan` is stateless
    /// and a merge would resurrect steps the model deliberately dropped.
    pub steps: Vec<LedgerStep>,
    /// Newest state-changing effects, bounded for rendering.
    pub effects: Vec<LedgerEffect>,
    /// Calls the ceiling truncated during the pass that just ended.
    pub cutoff: Vec<LedgerCutoff>,
    /// Monotonic count of dispatched state-changing results this turn.
    pub effects_total: usize,
    /// Monotonic count of those that succeeded. Never derived from
    /// `effects.len()`, which is a lossy render window.
    pub effects_ok_total: usize,
    /// Monotonic count of truncated state-changing calls across every pass.
    pub cutoff_state_changing_total: usize,
}

impl RoundLedger {
    /// Record one dispatched state-changing tool result.
    ///
    /// Totals are incremented BEFORE the render window is trimmed, so a count
    /// can never be lost to bounding.
    pub fn push_effect(&mut self, tool: String, label: String, ok: bool) {
        self.effects_total = self.effects_total.saturating_add(1);
        if ok {
            self.effects_ok_total = self.effects_ok_total.saturating_add(1);
        }
        self.effects.push(LedgerEffect { tool, label, ok });
        if self.effects.len() > MAX_RENDERED_LINES {
            let overflow = self.effects.len() - MAX_RENDERED_LINES;
            self.effects.drain(..overflow);
        }
    }

    /// Replace the plan snapshot. Called only for an accepted (non-error)
    /// `update_plan` result.
    pub fn replace_plan(&mut self, steps: Vec<LedgerStep>) {
        self.steps = steps;
    }

    /// Adopt this pass's truncated calls, superseding the previous pass's.
    pub fn set_cutoff(&mut self, cutoff: Vec<LedgerCutoff>) {
        self.cutoff_state_changing_total = self
            .cutoff_state_changing_total
            .saturating_add(cutoff.iter().filter(|c| c.state_changing).count());
        self.cutoff = cutoff;
    }
}

/// Per-turn restart bookkeeping. A stack local of the engine's turn function.
///
/// Deliberately derives no `PartialEq`: `requirement` is a `Vec<ContentBlock>`
/// and [`ContentBlock`] does not implement it. That is also why the transcript
/// anchor is "pop the assistant message this pass pushed" rather than "find the
/// message equal to the requirement".
#[derive(Debug)]
pub struct RoundState {
    /// The accepted requirement, verbatim, including any `Image` blocks. Cloned
    /// before the engine moves `user_content` into the transcript, and re-pushed
    /// unchanged on every restart so a multimodal task is never degraded to
    /// text.
    pub requirement: Vec<ContentBlock>,
    /// Attempts at `requirement` so far, including the first. 1 = no restart.
    pub attempt: usize,
    /// Whether the NEXT rendered system prompt should carry the round section.
    ///
    /// Set by a restart and consumed by exactly one pass. Without it the section
    /// would be re-appended to every remaining pass of the turn, so a model that
    /// restarted once and then worked normally would keep being told "your
    /// previous attempt was cut off, that draft has been REMOVED, your first
    /// action must be a tool call" while it was midway through a healthy tool
    /// loop — advice that is false by then and actively misleading.
    carry_section: bool,
    pub ledger: RoundLedger,
}

impl RoundState {
    pub fn new(requirement: Vec<ContentBlock>) -> Self {
        Self {
            requirement,
            attempt: 1,
            carry_section: false,
            ledger: RoundLedger::default(),
        }
    }

    /// Mark that a restart just happened, so the next pass carries the section.
    pub fn begin_attempt(&mut self) {
        self.attempt += 1;
        self.carry_section = true;
    }

    /// The system-channel section describing this round, consumed once.
    ///
    /// Returns `None` on the first attempt and on every pass that is not the one
    /// immediately following a restart: the section's header states that the
    /// previous attempt was cut off and its draft removed, which is only true
    /// for exactly one pass.
    ///
    /// Lives on `RoundState` rather than `RoundLedger` because the header needs
    /// `attempt`, which is turn bookkeeping and not an observation.
    ///
    /// Deliberately does NOT restate the requirement: that is the tail user
    /// message, and duplicating it here would both waste context and teach the
    /// model that the system channel carries user instructions.
    pub fn take_section(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.carry_section) {
            return None;
        }
        let mut out = format!(
            "[resumable round {}/{}] Your previous attempt was cut off by the provider's output \
             token ceiling. That draft has been REMOVED from your context and cannot be continued. \
             The original request is restated as the last user message below.",
            self.attempt, MAX_ROUND_ATTEMPTS
        );

        if !self.ledger.steps.is_empty() {
            out.push_str("\n\nALREADY DECLARED (your own plan):");
            for step in self.ledger.steps.iter().take(MAX_RENDERED_LINES) {
                let mark = match step.status {
                    StepStatus::Completed => "x",
                    StepStatus::InProgress => ">",
                    StepStatus::Pending => " ",
                };
                out.push_str(&format!(
                    "\n  [{mark}] {}",
                    one_line(&step.step, STEP_TEXT_BYTES)
                ));
            }
            append_elision(&mut out, self.ledger.steps.len());
        }

        if !self.ledger.effects.is_empty() {
            out.push_str("\n\nALREADY DONE (observed tool effects):");
            for effect in self.ledger.effects.iter().take(MAX_RENDERED_LINES) {
                let mark = if effect.ok { "ok  " } else { "FAIL" };
                out.push_str(&format!("\n  {mark}  {}: {}", effect.tool, effect.label));
            }
            append_elision(&mut out, self.ledger.effects.len());
        }

        if !self.ledger.cutoff.is_empty() {
            out.push_str("\n\nWHAT WAS CUT OFF:");
            for cut in self.ledger.cutoff.iter().take(MAX_RENDERED_LINES) {
                out.push_str(&format!(
                    "\n  {} ({} bytes of arguments streamed, NOT executed)",
                    cut.tool, cut.argument_bytes
                ));
            }
            append_elision(&mut out, self.ledger.cutoff.len());
        }

        out.push_str(
            "\n\nRULES FOR THIS ATTEMPT:\n\
             - Your first action must be a tool call. Do not restate the plan in prose.\n\
             - Split any large file: write a small complete version first, then edit or append.",
        );
        Some(out)
    }
}

/// Bound one effect label for the prompt.
///
/// Collapsed to a single line: the rendered section is one entry per line, and
/// `truncate_middle`'s elision marker itself contains newlines, so a long
/// `describe` output would otherwise break the list structure the model reads.
pub(crate) fn effect_label(description: &str) -> String {
    one_line(description, EFFECT_LABEL_BYTES)
}

/// Bound `text` to `budget` bytes on a char boundary and collapse every run of
/// whitespace to a single space, so it occupies exactly one rendered line.
fn one_line(text: &str, budget: usize) -> String {
    let bounded = nomi_tools::truncate_middle(text, nomi_tools::TruncationBudget::Bytes(budget));
    let collapsed = bounded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "(no description)".to_owned()
    } else {
        collapsed
    }
}

/// Note the entries the render cap dropped, so a bound never reads as coverage.
fn append_elision(out: &mut String, total: usize) {
    if total > MAX_RENDERED_LINES {
        out.push_str(&format!(
            "\n  … {} more not shown",
            total - MAX_RENDERED_LINES
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(text: &str, status: StepStatus) -> LedgerStep {
        LedgerStep {
            step: text.to_owned(),
            status,
        }
    }

    #[test]
    fn a_first_attempt_renders_no_section() {
        let mut state = RoundState::new(vec![ContentBlock::Text {
            text: "build it".to_owned(),
        }]);
        assert!(state.take_section().is_none());
    }

    #[test]
    fn an_empty_ledger_on_a_restart_still_states_the_draft_was_dropped() {
        let mut state = RoundState::new(Vec::new());
        state.begin_attempt();
        let section = state.take_section().expect("restart renders a section");
        assert!(section.contains("[resumable round 2/3]"));
        assert!(section.contains("REMOVED from your context"));
        assert!(!section.contains("ALREADY DECLARED"));
        assert!(!section.contains("ALREADY DONE"));
        assert!(!section.contains("WHAT WAS CUT OFF"));
    }

    #[test]
    fn a_rendered_section_carries_plan_effects_and_cutoff() {
        let mut state = RoundState::new(Vec::new());
        state.begin_attempt();
        state.ledger.replace_plan(vec![
            step("scaffold the layout", StepStatus::Completed),
            step("write miniapp.html", StepStatus::InProgress),
            step("verify it opens", StepStatus::Pending),
        ]);
        state
            .ledger
            .push_effect("Bash".to_owned(), "mkdir -p toolbox".to_owned(), true);
        state.ledger.set_cutoff(vec![LedgerCutoff {
            tool: "Write".to_owned(),
            argument_bytes: 6142,
            state_changing: true,
        }]);

        let section = state.take_section().expect("restart renders a section");
        assert!(section.contains("[x] scaffold the layout"));
        assert!(section.contains("[>] write miniapp.html"));
        assert!(section.contains("[ ] verify it opens"));
        assert!(section.contains("ok    Bash: mkdir -p toolbox"));
        assert!(section.contains("Write (6142 bytes of arguments streamed, NOT executed)"));
        // The requirement is the tail user message, never repeated here.
        assert!(!section.contains("build it"));
    }

    /// The section's header asserts that the PREVIOUS attempt was cut off and its
    /// draft removed, and orders the model to open with a tool call. That is true
    /// for exactly one pass. Left un-consumed it would be re-appended to the
    /// system prompt of every remaining pass, so a model that restarted once and
    /// then worked normally would keep being told its draft was just discarded
    /// while it was midway through a healthy tool loop.
    #[test]
    fn the_round_section_is_consumed_by_exactly_one_pass() {
        let mut state = RoundState::new(Vec::new());
        assert!(state.take_section().is_none(), "no restart, no section");

        state.begin_attempt();
        assert!(state.take_section().is_some(), "the pass after a restart");
        assert!(
            state.take_section().is_none(),
            "every later pass of the same round"
        );
        assert!(state.take_section().is_none());

        // A second restart re-arms it, and the header counts up.
        state.begin_attempt();
        let section = state.take_section().expect("the pass after the 2nd restart");
        assert!(section.contains("[resumable round 3/3]"), "{section}");
        assert!(state.take_section().is_none());
    }

    #[test]
    fn a_replaced_plan_does_not_merge_dropped_steps() {
        let mut ledger = RoundLedger::default();
        ledger.replace_plan(vec![
            step("keep", StepStatus::Pending),
            step("drop", StepStatus::Pending),
        ]);
        ledger.replace_plan(vec![step("keep", StepStatus::Completed)]);
        assert_eq!(ledger.steps.len(), 1);
        assert_eq!(ledger.steps[0].step, "keep");
        assert_eq!(ledger.steps[0].status, StepStatus::Completed);
    }

    /// A count must survive the render bound: a turn with 30 effects whose last
    /// 24 all failed still really did succeed 6 times.
    #[test]
    fn effect_totals_are_monotonic_across_the_render_window() {
        let mut ledger = RoundLedger::default();
        for i in 0..MAX_RENDERED_LINES + 6 {
            ledger.push_effect("Write".to_owned(), format!("file{i}.txt"), i < 3);
        }
        assert_eq!(ledger.effects.len(), MAX_RENDERED_LINES);
        assert_eq!(ledger.effects_total, MAX_RENDERED_LINES + 6);
        // The three successes were the OLDEST, so the window no longer holds
        // them; the totals must still report them.
        assert_eq!(ledger.effects_ok_total, 3);
        assert!(ledger.effects.iter().all(|e| !e.ok));
    }

    #[test]
    fn the_render_window_keeps_the_newest_effects() {
        let mut ledger = RoundLedger::default();
        for i in 0..MAX_RENDERED_LINES + 2 {
            ledger.push_effect("Bash".to_owned(), format!("step{i}"), true);
        }
        assert_eq!(ledger.effects.first().expect("bounded").label, "step2");
        assert_eq!(
            ledger.effects.last().expect("bounded").label,
            format!("step{}", MAX_RENDERED_LINES + 1)
        );
    }

    #[test]
    fn cutoff_state_changing_accumulates_across_passes() {
        let mut ledger = RoundLedger::default();
        ledger.set_cutoff(vec![
            LedgerCutoff {
                tool: "Write".to_owned(),
                argument_bytes: 10,
                state_changing: true,
            },
            LedgerCutoff {
                tool: "Read".to_owned(),
                argument_bytes: 5,
                state_changing: false,
            },
        ]);
        assert_eq!(ledger.cutoff_state_changing_total, 1);
        ledger.set_cutoff(vec![LedgerCutoff {
            tool: "Edit".to_owned(),
            argument_bytes: 20,
            state_changing: true,
        }]);
        // The window shows only the newest pass; the total remembers both.
        assert_eq!(ledger.cutoff.len(), 1);
        assert_eq!(ledger.cutoff_state_changing_total, 2);
    }

    #[test]
    fn mcp_is_not_treated_as_state_changing_because_it_is_dead_in_production() {
        assert!(is_state_changing(ToolCategory::Edit));
        assert!(is_state_changing(ToolCategory::Exec));
        assert!(is_state_changing(ToolCategory::Irreversible));
        assert!(!is_state_changing(ToolCategory::Info));
        assert!(!is_state_changing(ToolCategory::Mcp));
    }

    #[test]
    fn an_effect_label_is_bounded() {
        let long = "a".repeat(4096);
        let label = effect_label(&long);
        assert!(label.len() <= EFFECT_LABEL_BYTES + 64, "got {}", label.len());
        assert!(label.len() < long.len());
    }

    #[test]
    fn a_short_effect_label_is_untouched() {
        assert_eq!(effect_label("mkdir -p toolbox"), "mkdir -p toolbox");
    }

    /// `truncate_middle`'s elision marker embeds newlines, and a tool's
    /// `describe` output can be multi-line JSON. Either would break the
    /// one-entry-per-line list the model reads.
    #[test]
    fn an_effect_label_never_contains_a_newline() {
        let multiline = format!("{{\n  \"content\": \"{}\"\n}}", "x".repeat(4096));
        let label = effect_label(&multiline);
        assert!(!label.contains('\n'), "got {label:?}");
        assert!(!label.contains('\r'));

        let short_multiline = effect_label("line one\nline two");
        assert_eq!(short_multiline, "line one line two");
    }

    /// Multi-byte input must not panic on a byte-budget boundary.
    #[test]
    fn an_effect_label_respects_utf8_boundaries() {
        let cjk = "综合小工具箱".repeat(200);
        let label = effect_label(&cjk);
        assert!(!label.is_empty());
        assert!(label.len() < cjk.len());
        // Round-trips as valid UTF-8 by construction; assert it is still CJK.
        assert!(label.starts_with('综'));
    }

    #[test]
    fn a_whitespace_only_label_is_still_readable() {
        assert_eq!(effect_label("   \n\t "), "(no description)");
        assert_eq!(effect_label(""), "(no description)");
    }

    /// A model can declare an arbitrarily long plan and a provider can truncate
    /// many parallel calls. Neither may crowd out the system prompt, and the
    /// elision must be stated so a bound never reads as coverage.
    #[test]
    fn the_rendered_section_bounds_plan_steps_and_cutoffs_and_says_so() {
        let mut state = RoundState::new(Vec::new());
        state.begin_attempt();
        state.ledger.replace_plan(
            (0..MAX_RENDERED_LINES + 5)
                .map(|i| step(&format!("step {i}"), StepStatus::Pending))
                .collect(),
        );
        state.ledger.set_cutoff(
            (0..MAX_RENDERED_LINES + 3)
                .map(|i| LedgerCutoff {
                    tool: format!("Tool{i}"),
                    argument_bytes: i,
                    state_changing: true,
                })
                .collect(),
        );

        let section = state.take_section().expect("restart renders a section");
        assert_eq!(section.matches("[ ] step").count(), MAX_RENDERED_LINES);
        assert!(section.contains("… 5 more not shown"));
        assert_eq!(
            section.matches("bytes of arguments streamed").count(),
            MAX_RENDERED_LINES
        );
        assert!(section.contains("… 3 more not shown"));
        // The bound is a render bound only; the count is still exact.
        assert_eq!(
            state.ledger.cutoff_state_changing_total,
            MAX_RENDERED_LINES + 3
        );
    }

    #[test]
    fn a_long_plan_step_is_collapsed_to_one_line() {
        let mut state = RoundState::new(Vec::new());
        state.begin_attempt();
        state
            .ledger
            .replace_plan(vec![step(&format!("do\n{}", "y".repeat(4096)), StepStatus::Pending)]);
        let section = state.take_section().expect("restart renders a section");
        let plan_lines = section
            .lines()
            .filter(|l| l.trim_start().starts_with("[ ]"))
            .count();
        assert_eq!(plan_lines, 1, "section:\n{section}");
    }
}
