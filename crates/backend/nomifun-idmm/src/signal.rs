//! Normalized supervision signals and wake actions.

use nomifun_api_types::AgentErrorCode;

/// Where a detected decision prompt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSource {
    TerminalScan,
    TextScan,
}

/// Whether a decision has discrete options or requires a model-generated answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionKind {
    #[default]
    Options,
    OpenQuestion,
}

/// A parsed decision prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionPrompt {
    pub text: String,
    pub options: Vec<String>,
    pub recommended: Option<String>,
    pub source: DecisionSource,
    pub kind: DecisionKind,
}

/// A normalized signal emitted by a supervised session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSignal {
    Working,
    ProviderError {
        code: Option<AgentErrorCode>,
        retryable: Option<bool>,
        message: String,
    },
    AgentError {
        retryable: Option<bool>,
        message: String,
    },
    Idle,
    Decision(DecisionPrompt),
    Done,
    Cancelled,
    Exited,
}

/// Stall classification used by intervention policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallClass {
    ProviderError,
    Idle,
    Decision,
    OpenQuestion,
}

impl StallClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::Idle => "idle",
            Self::Decision => "decision",
            Self::OpenQuestion => "open_question",
        }
    }
}

/// The concrete action injected into a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeAction {
    Retry,
    SendText(String),
    AnswerChoice(String),
    Failover,
    Wait(std::time::Duration),
    Stop(String),
}

impl WakeAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::SendText(_) => "send_text",
            Self::AnswerChoice(_) => "answer_choice",
            Self::Failover => "failover",
            Self::Wait(_) => "wait",
            Self::Stop(_) => "stop",
        }
    }
}
