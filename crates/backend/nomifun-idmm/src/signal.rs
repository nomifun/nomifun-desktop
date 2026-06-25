//! Normalized supervision signals, stall classes, and wake actions — the
//! vocabulary the detector emits and the policy/supervisor consume. Independent
//! of how signals are sourced (agent events vs PTY bytes).

use nomifun_api_types::AgentErrorCode;

/// Where a detected decision prompt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSource {
    /// Parsed from terminal output bytes.
    TerminalScan,
    /// Parsed from a chat conversation's assistant turn text (a "方案 1/2/3、
    /// 请回复编号" style prompt the agent ended its turn on). Plain-desktop
    /// conversations only — channel/companion conversations route such menus to
    /// a remote human and must NOT be auto-answered (see `ConversationProbe`).
    TextScan,
    /// An agent `Permission`/`AcpPermission` event.
    Permission,
}

/// Whether a detected decision is a discrete option/permission choice or an
/// open-ended question with no enumerable options (纯问答, D6). `Options` is the
/// default (back-compat with the existing numbered-choice / permission path);
/// `OpenQuestion` marks an interrogative end-of-turn that has NO selectable
/// options, which only the model tier may answer (rule tier never guesses an
/// open answer — spec §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionKind {
    /// A discrete option / permission decision (numbered choice, y/n, tool
    /// permission). The existing auto-pick / confirm path handles it.
    #[default]
    Options,
    /// An open-ended question with no enumerable options. Answered only by the
    /// decision watch's model tier (free-text), never by the rule tier.
    OpenQuestion,
}

/// A parsed decision prompt awaiting a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionPrompt {
    /// The raw (ANSI-stripped) prompt text.
    pub text: String,
    /// Parsed selectable options in order, if any (e.g. `["1) yes", "2) no"]`).
    pub options: Vec<String>,
    /// The option the CLI marks recommended/default, if detectable.
    pub recommended: Option<String>,
    pub source: DecisionSource,
    /// Whether this is a discrete-options decision (default) or an open-ended
    /// question (D6). An `OpenQuestion` carries no `options`/`permission` and is
    /// answered with free text only by the model tier.
    pub kind: DecisionKind,
    /// Set when this is a STRUCTURED tool-permission decision
    /// (`Permission`/`AcpPermission`): it is answered by resolving the agent's
    /// pending approval via `ConversationService::confirm(call_id, …)`, NOT by
    /// injecting a chat message. `None` for text/terminal numbered-choice
    /// prompts (answered with their option text). See [`PermissionConfirm`].
    pub permission: Option<PermissionConfirm>,
}

/// Structured data needed to resolve a tool-permission decision via the agent's
/// confirmation channel (instead of a free-text chat reply, which never clears
/// the pending approval).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionConfirm {
    /// Tool-call id the approval is keyed by (`ConversationService::confirm`).
    pub call_id: String,
    /// `(label, submit-value)` per option, in order. The submit-value is the
    /// per-backend token (`option_id` for ACP, `proceed_once`/`cancel`/… for
    /// nomi) — IDMM submits it as both `option_id` and `value` so either backend
    /// resolves it.
    pub options: Vec<(String, String)>,
    /// The conservatively-safe "approve once" option's submit-value, set ONLY
    /// when it is safe to auto-approve WITHOUT a model (read-only / benign tool).
    /// `None` for risky tools (edit/execute): the rule tier must escalate to the
    /// sidecar (model judges with the tool details) or halt — never blanket
    /// auto-approve a write/exec.
    pub safe_value: Option<String>,
}

/// A normalized signal emitted by a `SessionProbe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSignal {
    /// Activity observed; resets the idle timer.
    Working,
    /// Provider fault. `retryable` mirrors `AgentStreamErrorData.retryable` when known.
    ProviderError {
        code: Option<AgentErrorCode>,
        retryable: Option<bool>,
        message: String,
    },
    /// Non-provider agent error.
    AgentError { retryable: Option<bool>, message: String },
    /// Quiescent beyond the idle threshold.
    Idle,
    /// A decision prompt is awaiting input.
    Decision(DecisionPrompt),
    /// The turn finished normally.
    Done,
    /// The turn was deliberately cancelled by the user (engines emit
    /// `Finish(stop_reason=Cancelled)` only on the user-stop path). NOT a
    /// stall: the supervisor must stand down instead of "recovering" work the
    /// user just stopped — nudging here was the "I paused it and it started
    /// running again" bug.
    Cancelled,
    /// The session/PTY ended.
    Exited,
}

/// Stall classification (drives the ladder + `InterventionRecord.stall_class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallClass {
    ProviderError,
    Idle,
    Decision,
    /// 纯问答(open-ended question, no enumerable options) — D6.
    OpenQuestion,
}

impl StallClass {
    pub fn as_str(self) -> &'static str {
        match self {
            StallClass::ProviderError => "provider_error",
            StallClass::Idle => "idle",
            StallClass::Decision => "decision",
            StallClass::OpenQuestion => "open_question",
        }
    }
}

/// The concrete action injected into a session to unblock it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeAction {
    /// Re-submit / "continue" the turn (backoff already applied by the policy).
    Retry,
    /// Send a free-text nudge or instruction.
    SendText(String),
    /// Answer a decision prompt (option text / "y" / a value).
    AnswerChoice(String),
    /// Resolve a STRUCTURED tool-permission approval via the agent's confirm
    /// channel (`call_id` + the chosen option's submit-`value`). Distinct from
    /// `AnswerChoice` (a chat-text reply) — a permission is a structured oneshot
    /// that a chat message would never clear.
    Confirm {
        call_id: String,
        value: String,
        always_allow: bool,
    },
    /// Switch to the next model in the failover queue and re-drive the turn
    /// (D6). Resolved by the conversation probe via the conversation service's
    /// shared failover helper — the SAME implementation the send-loop uses
    /// (`ConversationService::perform_model_failover`), so there is one source
    /// of truth for the swap. Terminal/ACP sessions self-manage their model and
    /// do NOT support this (the terminal probe degrades it to Retry; see D7).
    Failover,
    /// Back off for a duration before re-evaluating.
    Wait(std::time::Duration),
    /// Give up; surface to the user. Carries a reason.
    Stop(String),
}

impl WakeAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            WakeAction::Retry => "retry",
            WakeAction::SendText(_) => "send_text",
            WakeAction::AnswerChoice(_) => "answer_choice",
            WakeAction::Confirm { .. } => "confirm",
            WakeAction::Failover => "failover",
            WakeAction::Wait(_) => "wait",
            WakeAction::Stop(_) => "stop",
        }
    }
}
